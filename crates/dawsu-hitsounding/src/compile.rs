//! Compile (hitsound domain Responsibilities 9-13, ADR 0037): the one encoder of hits into file
//! fields. It patches a copy of the difficulty through `dawsu-osu-format` and lists every moment it
//! cannot encode as a save problem; the rest is written and each problem moment keeps its old line.
//! Blocking, with a cancel check and a progress report (ADR 0035).
//!
//! Where a value goes follows ADR 0042 (which supersedes ADR 0041): a value the object's own line
//! can hold (sample sets, and on circles and spinners the index and volume) is written there. A
//! green line is added only for what the object cannot hold (a slider node's index or volume, or
//! index 0 under a timing line with an index), and only with green-line placement; it is followed
//! by a line restoring the earlier values 6 ms later, so no other part's lookup changes. A slider
//! body's volume change sets its head's volume too, with one line restored 7 ms later; a node
//! entry that holds its own index or volume is kept as written. On-object placement reports a
//! moment that needs a green line as a save problem instead.

use crate::lazer::control_points::{offset_for, ControlPoints, SamplePointData, CONTROL_POINT_LENIENCY};
use crate::lazer::samples::bank_of;
use crate::model::{set_name, Hitsounds, Moment, ObjectId, Place, SliderBody};
use crate::rules::{self, saved_volume, Answer, ObjectAnswer};
use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, Sound, SongTime};
use dawsu_osu_format::number::try_parse_int;
use dawsu_osu_format::{HitObject, HitSample, LineId, ObjectKind, OsuFile, SliderFields};
use std::collections::{BTreeMap, HashMap};

/// How compile places a moment (Responsibility 9a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Save and Copy hitsounds: green lines may be added.
    GreenLines,
    /// Generate hitsound diff, New hitsound diff and HS mode: values on each object only.
    OnObjects,
}

/// A moment compile could not encode (Responsibility 12). The moment keeps its old values.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveProblem {
    pub time: SongTime,
    pub place: Place,
    pub kind: ProblemKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProblemKind {
    /// The hits do not fit osu!'s native combination. The plan says which samples a mix would
    /// hold and at which volume; M2 cannot make the mix file yet (Responsibility 11, ADR 0037).
    NeedsSampleMix(Vec<(SampleIdentity, i32)>),
    /// Hits with no object: only HS mode creates objects, and M2 has no HS mode (invariant 6).
    NoObject,
    /// The moment needs a green line, and the placement writes values on objects only.
    NeedsGreenLine,
    /// Another timing line or another object's lookup sits where the green line would go.
    LookupWindowTaken,
    /// The object's line could not be patched, or is a kind compile does not write.
    LineRefused,
    /// A slider body's volume goes with its head's (one green line both read), and the head asks
    /// for another volume or index, or cannot be saved itself.
    BodyVolumeNeedsHead,
}

/// The outcome of compile.
#[derive(Debug, Clone)]
pub enum Compiled {
    /// The patched difficulty, ready for the one write, and the moments it could not encode.
    Done { file: OsuFile, problems: Vec<SaveProblem> },
    /// The cancel check asked to stop; nothing is kept.
    Cancelled,
}

const WHISTLE: i32 = 2;
const FINISH: i32 = 4;
const CLAP: i32 = 8;
const ADDITIONS: i32 = WHISTLE | FINISH | CLAP;
/// A green line's own lookup window: a node looks up at its time + 5 ms, a slider body at + 6 ms.
const WINDOW: f64 = CONTROL_POINT_LENIENCY + 1.0;

/// Compiles `hitsounds` back into a copy of `file`, the difficulty it was decompiled from.
/// `cancel` is looked at between objects; `progress` gets (objects done, objects to write).
pub fn compile(file: &OsuFile, hitsounds: &Hitsounds, placement: Placement, cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize, usize)) -> Compiled {
    let mut out = file.clone();
    let mut problems = Vec::new();
    let mut groups: BTreeMap<ObjectId, (Vec<&Moment>, Option<&SliderBody>)> = BTreeMap::new();
    for m in hitsounds.moments().iter().filter(|m| m.is_changed()) {
        match m.place() {
            Place::NoObject => problems.push(SaveProblem { time: m.time(), place: m.place(), kind: ProblemKind::NoObject }),
            Place::Circle(id) | Place::SpinnerEnd(id) | Place::SliderNode { object: id, .. } => groups.entry(id).or_default().0.push(m),
        }
    }
    for b in hitsounds.bodies().iter().filter(|b| b.is_changed()) {
        groups.entry(b.object()).or_default().1 = Some(b);
    }
    if cancel() {
        return Compiled::Cancelled;
    }
    if groups.is_empty() {
        progress(0, 0);
        return Compiled::Done { file: out, problems };
    }

    let objects: HashMap<LineId, HitObject> = file.hit_objects().into_iter().map(|o| (o.line(), o)).collect();
    let answers: HashMap<LineId, ObjectAnswer> = match rules::answer(file, &hitsounds.folder_files) {
        Answer::Objects(list) => list.into_iter().map(|a| (a.line, a)).collect(),
        Answer::UnsupportedMode { .. } => HashMap::new(),
    };
    let mut lookups: Vec<(f64, LineId)> = Vec::new();
    for a in answers.values() {
        lookups.push((a.lookup_time, a.line));
        if let Some(s) = &a.slider {
            lookups.extend(s.node_lookup_times.iter().map(|t| (*t, a.line)));
        }
    }
    let ctx = Context { offset: offset_for(file.format_version()), placement, lookups };
    let mut cp = ControlPoints::from_file(&out);

    let total = groups.len();
    for (done, (id, (moments, body))) in groups.iter().enumerate() {
        if cancel() {
            return Compiled::Cancelled;
        }
        let refuse = |problems: &mut Vec<SaveProblem>| {
            for m in moments {
                problems.push(SaveProblem { time: m.time(), place: m.place(), kind: ProblemKind::LineRefused });
            }
            if let Some(b) = body {
                problems.push(SaveProblem { time: b.start(), place: Place::SliderNode { object: *id, node: 0 }, kind: ProblemKind::LineRefused });
            }
        };
        match (objects.get(&id.0), answers.get(&id.0)) {
            (Some(obj), Some(answer)) => {
                // An object's edits touch only its own line and add green lines, so a refused edit
                // is undone by putting the line back and removing what was added.
                let (before, mark) = (out.snapshot_line(obj.line()), out.next_line_id());
                let ok = match obj.kind() {
                    ObjectKind::Slider(fields) => ctx.slider(&mut out, &cp, obj, fields, answer, moments, *body, &mut problems),
                    _ => ctx.circle_or_spinner(&mut out, &cp, obj, answer, moments[0], &mut problems),
                };
                match ok {
                    Ok(true) if out.next_line_id() != mark => cp = ControlPoints::from_file(&out),
                    Ok(true) => {}
                    // Ok(false) kept nothing before either: whatever it touched is put back.
                    _ => {
                        out.remove_lines_added_since(mark);
                        if let Some(before) = before {
                            out.restore_line(before);
                        }
                        if ok.is_err() {
                            refuse(&mut problems);
                        }
                    }
                }
            }
            _ => refuse(&mut problems),
        }
        progress(done + 1, total);
    }
    problems.sort_by(|a, b| a.time.ms().total_cmp(&b.time.ms()));
    Compiled::Done { file: out, problems }
}

struct Context {
    offset: f64,
    placement: Placement,
    lookups: Vec<(f64, LineId)>,
}

/// What the hits at a moment ask for, when they fit the native combination (Responsibility 10).
struct Native {
    slot: Slot,
    additions: i32,
    addition_set: Option<SampleSet>,
    index: Option<u32>,
    volume: i32,
}

enum Slot {
    Normal(SampleSet),
    File(String),
}

fn mix_plan(m: &Moment) -> ProblemKind {
    ProblemKind::NeedsSampleMix(m.hits().map(|h| (h.clone(), saved_volume(m.hit_volume(h)))).collect())
}

/// The save problem for layers one moment cannot hold, each at its own volume (more than one
/// volume, or a lane twice): the sample mix that would play them, as data (ADR 0037 point 4).
/// Generate and Copy call this for their groups and pile-ups, so mix planning stays here.
pub fn mix_problem(layers: &[(SampleIdentity, i32)]) -> ProblemKind {
    ProblemKind::NeedsSampleMix(layers.iter().map(|(lane, volume)| (lane.clone(), saved_volume(*volume))).collect())
}

/// One hitnormal (or one sample file) plus additions that share one set, one index and one
/// effective volume; the hitnormal's index is the object's one index too.
fn native(m: &Moment) -> Result<Native, ProblemKind> {
    let volume = m.hits().next().map_or_else(|| saved_volume(m.volume()), |lane| saved_volume(m.hit_volume(lane)));
    let mut slot = None;
    let mut slot_index = None;
    let mut additions = 0;
    let mut addition: Option<(SampleSet, u32)> = None;
    for h in m.hits() {
        if saved_volume(m.hit_volume(h)) != volume {
            return Err(mix_plan(m));
        }
        match h {
            SampleIdentity::File(f) if slot.is_none() => slot = Some(Slot::File(f.clone())),
            SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Normal), index } if slot.is_none() => {
                slot = Some(Slot::Normal(*set));
                slot_index = Some(index.get());
            }
            SampleIdentity::Standard { set, sound: Sound::Hit(s @ (Hitsound::Whistle | Hitsound::Finish | Hitsound::Clap)), index } => {
                let bit = match s {
                    Hitsound::Whistle => WHISTLE,
                    Hitsound::Finish => FINISH,
                    _ => CLAP,
                };
                if addition.is_some_and(|a| a != (*set, index.get())) {
                    return Err(mix_plan(m));
                }
                addition = Some((*set, index.get()));
                additions |= bit;
            }
            _ => return Err(mix_plan(m)),
        }
    }
    let Some(slot) = slot else { return Err(mix_plan(m)) };
    let index = match (slot_index, addition) {
        (Some(n), Some((_, a))) if n != a => return Err(mix_plan(m)),
        (Some(n), _) => Some(n),
        (None, a) => a.map(|a| a.1),
    };
    Ok(Native { slot, additions, addition_set: addition.map(|a| a.0), index, volume })
}

/// The timing point an object part reads, with lazer's defaults when there is none.
struct Point<'a>(Option<&'a SamplePointData>);

impl Point<'_> {
    fn bank(&self) -> String {
        self.0.map_or_else(|| "normal".to_string(), |p| p.bank.clone())
    }
    fn index(&self) -> i32 {
        self.0.map_or(0, |p| p.custom_sample_bank).max(0)
    }
    fn volume(&self) -> i32 {
        self.0.map_or(100, |p| p.volume)
    }
    fn bank_number(&self) -> String {
        match self.bank().as_str() {
            "normal" => "1".into(),
            "soft" => "2".into(),
            "drum" => "3".into(),
            other => other.to_string(),
        }
    }
}

fn set_number(set: SampleSet) -> i32 {
    match set {
        SampleSet::Normal => 1,
        SampleSet::Soft => 2,
        SampleSet::Drum => 3,
    }
}

/// The normal and addition set fields that make the sets play as asked, changing as little as possible.
fn encode_banks(n_raw: i32, a_raw: i32, normal: Option<SampleSet>, addition: Option<SampleSet>, point_bank: &str) -> (i32, i32) {
    let eff_normal = |n: i32| bank_of(n).unwrap_or_else(|| point_bank.to_string());
    let n = match normal {
        Some(s) if eff_normal(n_raw) != set_name(s) => set_number(s),
        _ => n_raw,
    };
    let eff_addition = bank_of(a_raw).or_else(|| bank_of(n)).unwrap_or_else(|| point_bank.to_string());
    let a = match addition {
        Some(s) if eff_addition != set_name(s) => {
            if eff_normal(n) == set_name(s) {
                0
            } else {
                set_number(s)
            }
        }
        _ => a_raw,
    };
    (n, a)
}

/// A green line to add before a part's lookup (green-line placement only).
struct Green {
    lookup: f64,
    index: i32,
    volume: i32,
    /// How long after the line the restoring line comes.
    restore_after: f64,
    /// This object's own lookups that are meant to read the line.
    readers: Vec<f64>,
}

impl Green {
    /// For one part: the line at its lookup minus 5 ms, restored 6 ms later (ADR 0042 points 2-3).
    fn part(lookup: f64, index: i32, volume: i32) -> Green {
        Green { lookup, index, volume, restore_after: WINDOW, readers: vec![lookup] }
    }

    /// For a slider's head and body together: the line at the head's lookup minus 5 ms, restored
    /// 7 ms later, just after the body's lookup (ADR 0042 point 6).
    fn head_and_body(head_lookup: f64, body_lookup: f64, index: i32, volume: i32) -> Green {
        Green { lookup: head_lookup, index, volume, restore_after: WINDOW + 1.0, readers: vec![head_lookup, body_lookup] }
    }
}

impl Context {
    fn needs_green(&self) -> Result<(), ProblemKind> {
        match self.placement {
            Placement::GreenLines => Ok(()),
            Placement::OnObjects => Err(ProblemKind::NeedsGreenLine),
        }
    }

    /// Returns Ok(true) when the trial file changed.
    fn circle_or_spinner(&self, trial: &mut OsuFile, cp: &ControlPoints, obj: &HitObject, answer: &ObjectAnswer, m: &Moment, problems: &mut Vec<SaveProblem>) -> Result<bool, ()> {
        let problem = |problems: &mut Vec<SaveProblem>, kind| problems.push(SaveProblem { time: m.time(), place: m.place(), kind });
        let want = match native(m) {
            Ok(w) => w,
            Err(kind) => {
                problem(problems, kind);
                return Ok(false);
            }
        };
        let point = Point(cp.sample_point_at(answer.lookup_time));
        let raw = obj.hit_sample();
        let (n_raw, a_raw, i_raw, v_raw) = raw.map_or((0, 0, 0, 0), |h| (h.normal_set, h.addition_set, h.index, h.volume));
        let file_raw = raw.and_then(|h| h.filename.clone()).unwrap_or_default();

        let normal = match &want.slot {
            Slot::Normal(s) => Some(*s),
            Slot::File(_) => None,
        };
        let (n, a) = encode_banks(n_raw, a_raw, normal, want.addition_set, &point.bank());
        let mut green = None;
        let i = match want.index {
            None => i_raw,
            Some(w) => {
                let w = w as i32;
                let eff = if i_raw > 0 { i_raw } else { point.index() };
                if eff == w {
                    i_raw
                } else if w > 0 || point.index() == 0 {
                    w
                } else {
                    if let Err(kind) = self.needs_green() {
                        problem(problems, kind);
                        return Ok(false);
                    }
                    green = Some(Green::part(answer.lookup_time, 0, point.volume()));
                    0
                }
            }
        };
        let eff_volume = if v_raw > 0 { v_raw } else { point.volume() };
        let v = if eff_volume == want.volume { v_raw } else { want.volume };
        let file = match &want.slot {
            Slot::File(f) => f.clone(),
            Slot::Normal(_) => String::new(),
        };
        let field4 = (obj.hitsound() & !ADDITIONS) | want.additions;

        if let Some(g) = green {
            if let Err(kind) = self.place_green(trial, cp, answer.line, &g) {
                problem(problems, kind);
                return Ok(false);
            }
        }
        let line = obj.line();
        if field4 != obj.hitsound() {
            trial.set_field(line, 4, &field4.to_string()).map_err(|_| ())?;
        }
        if (n, a, i, v, file.as_str()) != (n_raw, a_raw, i_raw, v_raw, file_raw.as_str()) {
            let sample = format!("{n}:{a}:{i}:{v}:{file}");
            match obj.kind() {
                ObjectKind::Spinner { .. } => trial.set_field(line, 6, &sample),
                // A hold note keeps its end time in front of its samples: `end:normal:addition:...`.
                ObjectKind::Hold { .. } => {
                    let text = trial.line_text(line).ok_or(())?;
                    let end = text.split(',').nth(5).and_then(|f| f.split(':').next()).ok_or(())?.to_string();
                    trial.set_field(line, 5, &format!("{end}:{sample}"))
                }
                _ => trial.set_field(line, 5, &sample),
            }
            .map_err(|_| ())?;
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn slider(
        &self,
        trial: &mut OsuFile,
        cp: &ControlPoints,
        obj: &HitObject,
        fields: &SliderFields,
        answer: &ObjectAnswer,
        moments: &[&Moment],
        body: Option<&SliderBody>,
        problems: &mut Vec<SaveProblem>,
    ) -> Result<bool, ()> {
        let Some(s) = &answer.slider else { return Err(()) };
        let nodes = (fields.slides - 1).max(0) as usize + 2;
        // A slider lazer reads as zero length keeps only its first and last node's sounds, so the
        // answer's node 1 is the file's last node.
        let collapsed = s.repeat_count + 2 != nodes;
        let file_node = |node: usize| if collapsed && node > 0 { nodes - 1 } else { node };
        let own = obj.hit_sample().map_or((0, 0), |h| (h.normal_set, h.addition_set));
        let mut sounds: Vec<i32> = (0..nodes).map(|i| fields.edge_sounds.as_ref().and_then(|e| e.get(i)).map_or(obj.hitsound(), |t| try_parse_int(t))).collect();
        let entries: Vec<Option<&HitSample>> = (0..nodes).map(|i| fields.edge_sets.as_ref().and_then(|e| e.get(i)).and_then(Option::as_ref)).collect();
        let mut sets: Vec<(i32, i32)> = entries.iter().map(|e| e.map_or(own, |h| (h.normal_set, h.addition_set))).collect();
        let (original_sounds, original_sets) = (sounds.clone(), sets.clone());
        let raw_entries: Vec<String> = trial.line_text(obj.line()).and_then(|t| t.split(',').nth(9).map(|f| f.split('|').map(String::from).collect())).unwrap_or_default();
        let mut edited_nodes = vec![false; nodes];
        let mut changed = false;

        // A body volume change sets the head's volume too, through one green line both read
        // (ADR 0042 point 6). It is decided first: if it cannot be saved, the head keeps its line.
        let head_lookup = s.node_lookup_times[0];
        let head_moment = moments.iter().find(|m| matches!(m.place(), Place::SliderNode { node: 0, .. }));
        let mut body_volume_blocked = None;
        let mut body_volume_placed = false;
        if let Some(b) = body.filter(|b| b.volume_changed()) {
            let volume = saved_volume(b.volume());
            let head_point = Point(cp.sample_point_at(head_lookup));
            let head_ok = match head_moment {
                None => true,
                Some(hm) => match native(hm) {
                    Ok(Native { slot: Slot::Normal(_), volume: v, index, .. }) => {
                        let (entry_index, entry_volume) = entries[0].map_or((0, 0), |h| (h.index, h.volume));
                        let eff_index = if entry_index > 0 { entry_index } else { head_point.index() };
                        v == volume && entry_volume == 0 && index.unwrap_or(0) as i32 == eff_index
                    }
                    _ => false,
                },
            };
            let g = Green::head_and_body(head_lookup, answer.lookup_time, head_point.index(), volume);
            body_volume_blocked = if !head_ok {
                Some(ProblemKind::BodyVolumeNeedsHead)
            } else {
                self.needs_green().and_then(|()| self.place_green(trial, cp, answer.line, &g)).err()
            };
            body_volume_placed = body_volume_blocked.is_none();
            if let Some(kind) = &body_volume_blocked {
                problems.push(SaveProblem { time: b.start(), place: Place::SliderNode { object: b.object(), node: 0 }, kind: kind.clone() });
            } else {
                changed = true;
            }
        }

        for m in moments {
            let Place::SliderNode { node, .. } = m.place() else { continue };
            let problem = |problems: &mut Vec<SaveProblem>, kind| problems.push(SaveProblem { time: m.time(), place: m.place(), kind });
            if node == 0 {
                if let Some(kind) = &body_volume_blocked {
                    problem(problems, kind.clone());
                    continue;
                }
            }
            let want = match native(m) {
                Ok(Native { slot: Slot::File(_), .. }) => {
                    problem(problems, mix_plan(m));
                    continue;
                }
                Ok(w) => w,
                Err(kind) => {
                    problem(problems, kind);
                    continue;
                }
            };
            let Slot::Normal(set) = want.slot else { continue };
            let lookup = s.node_lookup_times[node];
            let point = Point(cp.sample_point_at(lookup));
            // The head's volume and index were placed with the body's green line.
            let placed_with_body = node == 0 && body_volume_placed;
            let node = file_node(node);
            let banks = encode_banks(sets[node].0, sets[node].1, Some(set), want.addition_set, &point.bank());
            let index = want.index.unwrap_or(0) as i32;
            // A longer node entry can hold the node's own index and volume, which win over the
            // timing line's (ConvertHitObjectParser.cs:127, :231-235).
            let (entry_index, entry_volume) = entries[node].map_or((0, 0), |h| (h.index, h.volume));
            let eff_index = if entry_index > 0 { entry_index } else { point.index() };
            let eff_volume = if entry_volume > 0 { entry_volume } else { point.volume() };
            if (index != eff_index && entry_index > 0) || (want.volume != eff_volume && entry_volume > 0) {
                // A green line cannot change what the entry holds, and ADR 0042 writes a node's
                // index and volume only on green lines.
                problem(problems, ProblemKind::LineRefused);
                continue;
            }
            if !placed_with_body && (index != eff_index || want.volume != eff_volume) {
                if let Err(kind) = self.needs_green() {
                    problem(problems, kind);
                    continue;
                }
                // Otherwise the node's index and volume come from the timing line (lazer reads
                // only a slider's banks from its own hit sample).
                let g = Green::part(lookup, index, want.volume);
                if let Err(kind) = self.place_green(trial, cp, answer.line, &g) {
                    problem(problems, kind);
                    continue;
                }
            }
            sets[node] = banks;
            sounds[node] = (sounds[node] & !ADDITIONS) | want.additions;
            edited_nodes[node] = true;
            changed = true;
        }

        // The body: its whistle is the slider's own hitsound flag, its set the slider's own normal
        // bank, both shared with every node that has no edge entry of its own; its volume was
        // placed above with the head's.
        let point = Point(cp.sample_point_at(answer.lookup_time));
        let whistle = body.filter(|b| b.whistle_changed()).map(SliderBody::whistle);
        let mut own_new = own;
        if let Some(b) = body.filter(|b| b.set_changed()) {
            let whistle_set = match b.whistle_lane() {
                SampleIdentity::Standard { set, .. } => Some(set),
                SampleIdentity::File(_) => None,
            };
            own_new = encode_banks(own.0, own.1, b.set(), whistle_set, &point.bank());
        }
        if let Some(b) = body.filter(|b| b.whistle_set_changed()) {
            // Explicit and Auto must remain distinct even when the effective set is the same.
            own_new.1 = b.whistle_set.map_or(0, set_number);
        }
        let line = obj.line();
        let sounds_complete = fields.edge_sounds.as_ref().is_some_and(|e| e.len() >= nodes);
        if sounds != original_sounds || (whistle.is_some() && !sounds_complete) {
            let text = sounds.iter().map(i32::to_string).collect::<Vec<_>>().join("|");
            trial.set_field(line, 8, &text).map_err(|_| ())?;
            changed = true;
        }
        // Nodes without an edge set of their own read the slider's banks, so those are written
        // out before the slider's banks change.
        let sets_explicit = fields.edge_sets.as_ref().is_some_and(|e| e.len() >= nodes && e.iter().take(nodes).all(Option::is_some));
        if sets != original_sets || (own_new != own && !sets_explicit) {
            // An untouched entry is kept as written; an edited one keeps its parts after the banks.
            let entry_text = |i: usize| -> String {
                let (n, a) = sets[i];
                match raw_entries.get(i).filter(|r| !r.is_empty()) {
                    Some(raw) if !edited_nodes[i] => raw.clone(),
                    Some(raw) => std::iter::once(format!("{n}:{a}")).chain(raw.split(':').skip(2).map(String::from)).collect::<Vec<_>>().join(":"),
                    None => format!("{n}:{a}"),
                }
            };
            let text = (0..nodes).map(entry_text).collect::<Vec<_>>().join("|");
            trial.set_field(line, 9, &text).map_err(|_| ())?;
            changed = true;
        }
        if own_new != own {
            let raw = obj.hit_sample();
            let (i, v) = raw.map_or((0, 0), |h| (h.index, h.volume));
            let file = raw.and_then(|h| h.filename.clone()).unwrap_or_default();
            trial.set_field(line, 10, &format!("{}:{}:{i}:{v}:{file}", own_new.0, own_new.1)).map_err(|_| ())?;
            changed = true;
        }
        if let Some(w) = whistle {
            let field4 = (obj.hitsound() & !WHISTLE) | if w { WHISTLE } else { 0 };
            trial.set_field(line, 4, &field4.to_string()).map_err(|_| ())?;
            changed = true;
        }
        Ok(changed)
    }

    /// Adds a green line at `g.lookup - 5 ms` giving its readers `g.index` and `g.volume`, with the
    /// sample set, slider velocity, meter and effects already in force there, and a line
    /// `g.restore_after` later restoring what was in force before.
    fn place_green(&self, trial: &mut OsuFile, cp: &ControlPoints, own: LineId, g: &Green) -> Result<(), ProblemKind> {
        let at = g.lookup - CONTROL_POINT_LENIENCY;
        let restore_at = at + g.restore_after;
        let lines = trial.timing_points();
        if lines.iter().any(|t| {
            let time = t.time() + self.offset;
            time > at && time <= restore_at
        }) {
            return Err(ProblemKind::LookupWindowTaken);
        }
        // Any other lookup before the restoring line would read the new values, including another
        // part of the same object (a short slider's head, body or next node); one exactly at the
        // restoring line reads that line.
        let other = |t: f64, line: LineId| !(line == own && g.readers.contains(&t));
        if self.lookups.iter().any(|(t, line)| other(*t, *line) && *t >= at && *t < restore_at) {
            return Err(ProblemKind::LookupWindowTaken);
        }
        // Parts before the first timing line read the first one (SamplePointAt), so a line at or
        // before it would reach them too.
        let first = lines.iter().map(|t| t.time() + self.offset).fold(f64::INFINITY, f64::min);
        if at <= first && self.lookups.iter().any(|(t, line)| other(*t, *line) && *t < at) {
            return Err(ProblemKind::LookupWindowTaken);
        }
        let before = Point(cp.sample_point_at(g.lookup));
        if before.index() == g.index && before.volume() == g.volume {
            return Ok(());
        }
        let after = Point(cp.sample_point_at(restore_at));
        let text = |time: f64, point: &Point, index: i32, volume: i32| -> String {
            let (meter, effects) = self.meter_and_effects(trial, time);
            format!("{},{},{},{},{},{},0,{}", time_text(time - self.offset), self.beat_length_text(trial, time), meter, point.bank_number(), index, volume, effects)
        };
        let ours = text(at, &before, g.index, g.volume);
        let restore = text(restore_at, &after, after.index(), after.volume());
        trial.insert_green_line(&ours);
        trial.insert_green_line(&restore);
        Ok(())
    }

    /// The raw lines at the latest time at or before `time`, in file order.
    fn lines_at_or_before(&self, trial: &OsuFile, time: f64) -> Vec<String> {
        let points = trial.timing_points();
        let latest = points.iter().map(|t| t.time() + self.offset).filter(|t| *t <= time).fold(f64::NEG_INFINITY, f64::max);
        points.iter().filter(|t| t.time() + self.offset == latest).filter_map(|t| trial.line_text(t.line())).collect()
    }

    /// The beat length that keeps the slider velocity in force at `time`: the green line there,
    /// or -100 (velocity 1) after a red line or before any line.
    fn beat_length_text(&self, trial: &OsuFile, time: f64) -> String {
        let at_latest = self.lines_at_or_before(trial, time);
        at_latest
            .iter()
            .rev()
            .find(|l| l.split(',').nth(6).is_some_and(|u| u.trim() == "0"))
            .and_then(|l| l.split(',').nth(1).map(|b| b.trim().to_string()))
            .unwrap_or_else(|| "-100".to_string())
    }

    fn meter_and_effects(&self, trial: &OsuFile, time: f64) -> (String, String) {
        let last = self.lines_at_or_before(trial, time).pop();
        let field = |i: usize, default: &str| last.as_ref().and_then(|l| l.split(',').nth(i).map(|f| f.trim().to_string())).unwrap_or_else(|| default.to_string());
        (field(2, "4"), field(7, "0"))
    }
}

fn time_text(ms: f64) -> String {
    if ms.fract() == 0.0 && ms.abs() < 1e15 { format!("{}", ms as i64) } else { format!("{ms}") }
}
