//! Save writes everything it can (F052; Yoge, 2026-11-19). A part whose drawn hits compile into a
//! save problem is left out of the `.osu`: it goes back to its state at the last write, then to the
//! difficulty as opened, and the rest is compiled again. After four rounds a failing part goes
//! straight back to the difficulty as opened. What was left out stays in dawsu!'s project.

use super::{Pass, compile_pass};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::hs::hs_group_time;
use dawsu_hitsounding::{BodyState, Hitsounds, Moment, MomentId, ObjectId, Place, ProblemKind, SaveProblem};
use dawsu_osu_format::{LineId, OsuFile};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Rounds that try the last written state before a part goes straight back to the file as opened.
pub(crate) const MAX_ROUNDS: usize = 4;

/// A part Save can leave out of the `.osu`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Part {
    /// A circle, a spinner's end or a slider node.
    Moment(Place),
    /// A slider body.
    Body(ObjectId),
    /// The loose hits HS mode gives one circle, by that circle's file time.
    Group(i64),
}

/// A moment's hits, each at its own volume.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct MomentState {
    pub time_bits: u64,
    pub volume: i32,
    pub hits: Vec<(SampleIdentity, i32)>,
}

impl MomentState {
    pub(crate) fn of(moment: &Moment) -> Self {
        Self {
            time_bits: moment.time().ms().to_bits(),
            volume: moment.volume(),
            hits: moment.hits().map(|lane| (lane.clone(), moment.hit_volume(lane))).collect(),
        }
    }
}

/// What a part holds.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Held {
    Moment(MomentState),
    Body(BodyState),
    Group(Vec<MomentState>),
}

/// A part Save left out, with the problem that stopped it and what it held as drawn.
#[derive(Clone, Debug)]
pub(crate) struct LeftOut {
    pub problem: SaveProblem,
    pub part: Part,
    pub held: Held,
    pub hs: bool,
}

impl LeftOut {
    /// True while the part still holds what Save left out, in the HS mode it was saved in.
    pub(crate) fn still_held(&self, hits: &Hitsounds, base: &OsuFile, hs: bool) -> bool {
        self.hs == hs && held(hits, base, self.part).as_ref() == Some(&self.held)
    }
}

/// What `part` holds in `hits`. `base` is the difficulty compile starts from, for HS group times.
pub(crate) fn held(hits: &Hitsounds, base: &OsuFile, part: Part) -> Option<Held> {
    match part {
        Part::Moment(place) => hits.moments().iter().find(|m| m.place() == place).map(|m| Held::Moment(MomentState::of(m))),
        Part::Body(object) => hits.bodies().iter().find(|b| b.object() == object).map(|b| Held::Body(b.state())),
        Part::Group(time) => {
            let mut states: Vec<_> = group(hits, base, time).map(|id| MomentState::of(hits.moment(id))).collect();
            states.sort();
            Some(Held::Group(states))
        }
    }
}

/// The loose moments with hits that HS mode gives the circle at `time`.
pub(crate) fn group<'a>(hits: &'a Hitsounds, base: &'a OsuFile, time: i64) -> impl Iterator<Item = MomentId> + 'a {
    hits.moments().iter().enumerate()
        .filter(move |(_, m)| m.place() == Place::NoObject && m.hits().next().is_some() && hs_group_time(base, m.time()) == time)
        .map(|(index, _)| MomentId(index))
}

/// What Save compiles: the file as opened, the drawn hits and the hits as last written.
pub(crate) struct Inputs<'a> {
    pub base: &'a OsuFile,
    pub drawn: &'a Hitsounds,
    pub written: &'a Hitsounds,
    pub hs: bool,
    pub owned: &'a BTreeSet<LineId>,
    pub loose_in_project: bool,
    /// Off without a project: nothing could keep a left-out part, so Save refuses instead.
    pub roll_back: bool,
}

/// The result of Save's compile with roll-back.
pub(crate) struct Rolled {
    pub out: OsuFile,
    /// The final compile input: the drawn hits with every left-out part rolled back.
    pub effective: Hitsounds,
    pub left_out: Vec<LeftOut>,
    pub owned: BTreeSet<LineId>,
    /// The circle HS mode made for each group, by the group's file time, in `out`.
    pub circles: BTreeMap<i64, LineId>,
    /// With roll-back off: the problems that stop Save.
    pub refused: Vec<SaveProblem>,
    /// How many compiles it took; the tests check it stays within [`MAX_ROUNDS`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub rounds: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Written,
    Base,
}

/// What one roll-back step changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Unit {
    Moment(Place),
    /// Only the body's volume; the head keeps what it holds.
    BodyVolume(ObjectId),
    Body(ObjectId),
    Group(i64),
}

impl Unit {
    fn part(self) -> Part {
        match self {
            Unit::Moment(place) => Part::Moment(place),
            Unit::BodyVolume(object) | Unit::Body(object) => Part::Body(object),
            Unit::Group(time) => Part::Group(time),
        }
    }
}

/// Compiles the drawn hits, rolling back each part that comes back as a problem. `mixable`
/// decides which problems a sample mix will write instead; `finish` gets each clean pass and its
/// mixable problems and returns the final file, or the problems that still stop it.
pub(crate) fn compile_with_roll_back(
    inputs: &Inputs,
    mixable: &dyn Fn(&SaveProblem) -> bool,
    finish: &mut dyn FnMut(&Pass, &Hitsounds, Vec<SaveProblem>) -> Result<Result<OsuFile, Vec<SaveProblem>>, String>,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Rolled, String> {
    let mut effective = inputs.drawn.clone();
    let mut levels: HashMap<Unit, Level> = HashMap::new();
    let mut left_out: Vec<LeftOut> = Vec::new();
    let mut round = 0;
    loop {
        if cancel() {
            return Err("Save cancelled".into());
        }
        round += 1;
        let pass = compile_pass(inputs.base, &effective, inputs.hs, inputs.owned, inputs.loose_in_project, cancel, progress)?;
        let (mixes, rest): (Vec<_>, Vec<_>) = pass.problems.iter().cloned().partition(|problem| mixable(problem));
        let failing = if rest.is_empty() {
            match finish(&pass, &effective, mixes)? {
                Ok(out) => {
                    left_out.sort_by(|a, b| a.problem.time.ms().total_cmp(&b.problem.time.ms()));
                    return Ok(Rolled {
                        out,
                        effective,
                        left_out,
                        owned: pass.owned,
                        circles: pass.circles.into_iter().map(|(line, time)| (time, line)).collect(),
                        refused: Vec::new(),
                        rounds: round,
                    });
                }
                Err(problems) => problems,
            }
        } else if !inputs.roll_back {
            pass.problems.clone()
        } else {
            rest
        };
        if !inputs.roll_back {
            return Ok(Rolled {
                out: pass.out,
                effective,
                left_out,
                owned: pass.owned,
                circles: BTreeMap::new(),
                refused: failing,
                rounds: round,
            });
        }
        let to_base = round >= MAX_ROUNDS;
        let mut moved = false;
        let mut seen = Vec::new();
        for problem in &failing {
            if seen.contains(&problem.place) {
                continue;
            }
            seen.push(problem.place);
            for (units, level) in steps(problem, &failing, &effective, &pass.circles) {
                if to_base && level != Level::Base {
                    continue;
                }
                let mut changed = Vec::new();
                for unit in units {
                    if levels.get(&unit).is_some_and(|at| *at >= level) {
                        continue;
                    }
                    let before = held(&effective, inputs.base, unit.part());
                    apply(&mut effective, inputs.written, inputs.base, unit, level);
                    levels.insert(unit, level);
                    if let Unit::Body(object) = unit {
                        let volume = levels.entry(Unit::BodyVolume(object)).or_insert(level);
                        *volume = (*volume).max(level);
                    }
                    if held(&effective, inputs.base, unit.part()) != before {
                        changed.push(unit.part());
                    }
                }
                if changed.is_empty() {
                    continue;
                }
                moved = true;
                for part in changed {
                    if left_out.iter().any(|entry| entry.part == part) {
                        continue;
                    }
                    let held = held(inputs.drawn, inputs.base, part)
                        .ok_or_else(|| format!("Save could not find the moment at {:.0} ms to leave it out.", problem.time.ms()))?;
                    left_out.push(LeftOut { problem: problem.clone(), part, held, hs: inputs.hs });
                }
                break;
            }
        }
        if !moved {
            let problem = &failing[0];
            return Err(format!(
                "Save could not leave out the moment at {:.0} ms: {}",
                problem.time.ms(),
                super::problem_description(&problem.kind)
            ));
        }
    }
}

/// The roll-back steps for one problem, first to last; each applies its units at its level.
fn steps(problem: &SaveProblem, failing: &[SaveProblem], effective: &Hitsounds, circles: &BTreeMap<LineId, i64>) -> Vec<(Vec<Unit>, Level)> {
    let simple = |unit: Unit| vec![(vec![unit], Level::Written), (vec![unit], Level::Base)];
    match problem.place {
        Place::Circle(id) if circles.contains_key(&id.line()) => simple(Unit::Group(circles[&id.line()])),
        Place::Circle(_) | Place::SpinnerEnd(_) => simple(Unit::Moment(problem.place)),
        Place::SliderNode { object, node } => {
            let body = effective.bodies().iter().find(|b| b.object() == object);
            let moments: Vec<Unit> = effective.moments().iter()
                .filter(|m| m.is_changed() && matches!(m.place(), Place::SliderNode { object: o, .. } if o == object))
                .map(|m| Unit::Moment(m.place()))
                .collect();
            let refused = |place: Place| failing.iter().any(|p| p.place == place && p.kind == ProblemKind::LineRefused);
            let whole_object = problem.kind == ProblemKind::LineRefused
                && moments.iter().all(|unit| matches!(unit, Unit::Moment(place) if refused(*place)))
                && body.filter(|b| b.is_changed()).is_none_or(|b| {
                    failing.iter().any(|p| p.place == Place::SliderNode { object, node: 0 } && p.kind == ProblemKind::LineRefused && p.time == b.start())
                });
            if whole_object {
                // The object's line could not be patched: the body first, then every moment on it.
                let mut steps = Vec::new();
                let mut all = moments.clone();
                if body.is_some_and(|b| b.is_changed()) {
                    steps.push((vec![Unit::Body(object)], Level::Written));
                    all.push(Unit::Body(object));
                }
                steps.push((moments, Level::Written));
                steps.push((all, Level::Base));
                steps
            } else if node == 0
                && !matches!(problem.kind, ProblemKind::NeedsSampleMix(_))
                && body.is_some_and(|b| b.volume() != b.decompiled_state().volume)
            {
                // A body's volume goes with its head's (one green line both read): the body's volume
                // first, then the head moment.
                let head = Unit::Moment(problem.place);
                vec![
                    (vec![Unit::BodyVolume(object)], Level::Written),
                    (vec![head], Level::Written),
                    (vec![Unit::BodyVolume(object), head], Level::Base),
                ]
            } else {
                simple(Unit::Moment(problem.place))
            }
        }
        Place::NoObject => Vec::new(),
    }
}

/// Puts `unit` in `effective` back to its state in `written`, or as opened.
fn apply(effective: &mut Hitsounds, written: &Hitsounds, base: &OsuFile, unit: Unit, level: Level) {
    match unit {
        Unit::Moment(place) => {
            let Some(index) = effective.moments().iter().position(|m| m.place() == place) else { return };
            match written.moments().iter().find(|m| m.place() == place).filter(|_| level == Level::Written) {
                Some(w) => {
                    let hits: Vec<_> = w.hits().map(|lane| (lane.clone(), w.hit_volume(lane))).collect();
                    effective.set_moment_state(MomentId(index), w.volume(), hits);
                }
                None => effective.reset_moment(MomentId(index)),
            }
        }
        Unit::BodyVolume(object) | Unit::Body(object) => {
            let Some(index) = effective.bodies().iter().position(|b| b.object() == object) else { return };
            let target = match written.bodies().iter().find(|b| b.object() == object).filter(|_| level == Level::Written) {
                Some(w) => w.state(),
                None => effective.bodies()[index].decompiled_state(),
            };
            let state = match unit {
                Unit::BodyVolume(_) => BodyState { volume: target.volume, ..effective.bodies()[index].state() },
                _ => target,
            };
            effective.set_body_state(index, state);
        }
        Unit::Group(time) => {
            let members: Vec<_> = group(effective, base, time).collect();
            for id in members {
                effective.reset_moment(id);
            }
            if level == Level::Written {
                let copies: Vec<_> = group(written, base, time).map(|id| written.moment(id).clone()).collect();
                for w in copies {
                    let id = effective.add_moment(w.time(), w.volume());
                    let hits: Vec<_> = w.hits().map(|lane| (lane.clone(), w.hit_volume(lane))).collect();
                    effective.set_moment_state(id, w.volume(), hits);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile_for_save;
    use dawsu_foundation::{CustomIndex, Hitsound, SampleSet, SongTime, Sound};
    use dawsu_hitsounding::{Decompiled, decompile};

    const MAP: &str = "osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n";
    /// Under a timing line with custom index 2, a circle cannot ask for the skin's index 0 on its
    /// own line: that needs a green line, which HS mode does not add.
    const INDEXED: &str = "osu file format v14\r\n\r\n[General]\r\nSampleSet: Soft\r\n\r\n[TimingPoints]\r\n0,500,4,2,2,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";

    fn read(text: &str) -> OsuFile {
        OsuFile::read(text.as_bytes().to_vec()).file
    }

    fn dec(file: &OsuFile) -> Hitsounds {
        match decompile(file, &["voice.wav".to_string()], &|| false, &mut |_, _| {}) {
            Decompiled::Done(hits) => hits,
            _ => panic!("decompile"),
        }
    }

    fn skin(set: SampleSet, sound: Hitsound) -> SampleIdentity {
        SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(0) }
    }

    fn node(hits: &Hitsounds, node: usize) -> MomentId {
        MomentId(hits.moments().iter().position(|m| matches!(m.place(), Place::SliderNode { node: n, .. } if n == node)).unwrap())
    }

    fn roll(base: &OsuFile, drawn: &Hitsounds, written: &Hitsounds, hs: bool) -> Rolled {
        let inputs = Inputs { base, drawn, written, hs, owned: &BTreeSet::new(), loose_in_project: true, roll_back: true };
        let rolled = compile_with_roll_back(
            &inputs, &|_| false, &mut |pass, _, _| Ok(Ok(pass.out.clone())), &|| false, &mut |_, _| {},
        ).unwrap();
        // C3: the final compile input reproduces the written bytes on its own.
        let (again, problems, _) = compile_for_save(base, &rolled.effective, hs, &BTreeSet::new(), true, &|| false, &mut |_, _| {}).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(again.to_bytes(), rolled.out.to_bytes());
        rolled
    }

    fn slider_line(file: &OsuFile) -> String {
        file.lines().map(|line| String::from_utf8_lossy(line.content()).into_owned()).find(|line| line.starts_with("100,100,2000,")).unwrap()
    }

    #[test]
    fn an_unwritable_slider_node_is_left_out_and_the_rest_is_written() {
        let base = read(MAP);
        let opened = dec(&base);
        let mut drawn = opened.clone();
        let tail = node(&drawn, 1);
        drawn.add_hit(tail, skin(SampleSet::Soft, Hitsound::Clap));
        drawn.add_hit(tail, skin(SampleSet::Drum, Hitsound::Clap));
        drawn.add_hit(MomentId(0), skin(SampleSet::Normal, Hitsound::Finish));
        let rolled = roll(&base, &drawn, &opened, true);
        assert_eq!(rolled.rounds, 2);
        assert_eq!(slider_line(&rolled.out), slider_line(&base));
        assert!(rolled.out.to_bytes() != base.to_bytes(), "the circle's finish is written");
        let [left] = &rolled.left_out[..] else { panic!("{:?}", rolled.left_out) };
        assert_eq!(left.part, Part::Moment(drawn.moment(tail).place()));
        assert!(matches!(left.problem.kind, ProblemKind::NeedsSampleMix(_)));
        assert_eq!(held(&drawn, &base, left.part), Some(left.held.clone()));
        assert!(!rolled.effective.moment(tail).is_changed());
    }

    #[test]
    fn a_node_written_before_keeps_its_last_written_fields() {
        let base = read(MAP);
        let mut written = dec(&base);
        let tail = node(&written, 1);
        written.add_hit(tail, skin(SampleSet::Soft, Hitsound::Clap));
        let first = roll(&base, &written, &dec(&base), true);
        assert!(first.left_out.is_empty());
        let mut drawn = written.clone();
        drawn.add_hit(tail, skin(SampleSet::Drum, Hitsound::Clap));
        let second = roll(&base, &drawn, &first.effective, true);
        assert_eq!(second.left_out.len(), 1);
        assert_eq!(slider_line(&second.out), slider_line(&first.out));
        assert_ne!(slider_line(&second.out), slider_line(&base));
    }

    #[test]
    fn an_hs_group_never_written_makes_no_circle() {
        let base = read(INDEXED);
        let opened = dec(&base);
        let mut drawn = opened.clone();
        let loose = drawn.add_moment(SongTime::from_ms(3000.0), 80);
        drawn.add_hit(loose, skin(SampleSet::Soft, Hitsound::Normal));
        drawn.add_hit(loose, skin(SampleSet::Soft, Hitsound::Clap));
        let rolled = roll(&base, &drawn, &opened, true);
        assert_eq!(rolled.out.to_bytes(), base.to_bytes(), "no 256,192 placeholder circle");
        let [left] = &rolled.left_out[..] else { panic!("{:?}", rolled.left_out) };
        assert_eq!(left.part, Part::Group(3000));
        assert_eq!(left.problem.kind, ProblemKind::NeedsGreenLine);
        assert!(rolled.circles.is_empty());
    }

    #[test]
    fn an_hs_group_written_before_keeps_its_one_circle() {
        let base = read(INDEXED);
        let opened = dec(&base);
        let mut written = opened.clone();
        let loose = written.add_moment(SongTime::from_ms(3000.0), 80);
        written.add_hit(loose, SampleIdentity::File("voice.wav".into()));
        let first = roll(&base, &written, &opened, true);
        assert!(first.left_out.is_empty());
        assert_eq!(first.out.hit_objects().len(), 2);
        let mut drawn = written.clone();
        drawn.add_hit(loose, skin(SampleSet::Soft, Hitsound::Clap));
        let second = roll(&base, &drawn, &first.effective, true);
        assert_eq!(second.out.to_bytes(), first.out.to_bytes());
        assert_eq!(second.circles.keys().copied().collect::<Vec<_>>(), [3000]);
        let [left] = &second.left_out[..] else { panic!("{:?}", second.left_out) };
        assert_eq!(left.part, Part::Group(3000));
    }

    #[test]
    fn a_body_volume_in_hs_mode_rolls_back_the_body_then_its_head() {
        let base = read(MAP);
        let opened = dec(&base);
        let mut drawn = opened.clone();
        drawn.set_body_volume(0, 40);
        let rolled = roll(&base, &drawn, &opened, true);
        assert_eq!(rolled.out.to_bytes(), base.to_bytes());
        assert_eq!(rolled.rounds, 3);
        let parts: Vec<_> = rolled.left_out.iter().map(|entry| entry.part).collect();
        let head = drawn.moment(node(&drawn, 0)).place();
        assert!(parts.contains(&Part::Body(drawn.bodies()[0].object())) && parts.contains(&Part::Moment(head)), "{parts:?}");
    }

    #[test]
    fn without_roll_back_the_problems_refuse_and_nothing_changes() {
        let base = read(MAP);
        let opened = dec(&base);
        let mut drawn = opened.clone();
        let tail = node(&drawn, 1);
        drawn.add_hit(tail, skin(SampleSet::Soft, Hitsound::Clap));
        drawn.add_hit(tail, skin(SampleSet::Drum, Hitsound::Clap));
        let inputs = Inputs { base: &base, drawn: &drawn, written: &opened, hs: true, owned: &BTreeSet::new(), loose_in_project: false, roll_back: false };
        let rolled = compile_with_roll_back(&inputs, &|_| false, &mut |pass, _, _| Ok(Ok(pass.out.clone())), &|| false, &mut |_, _| {}).unwrap();
        assert_eq!(rolled.refused.len(), 1);
        assert!(rolled.left_out.is_empty());
        assert_eq!(rolled.effective, drawn);
    }
}
