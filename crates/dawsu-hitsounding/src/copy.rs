//! Copy hitsounds (design doc "Generate and Copy hitsounds", Responsibilities 7-12; the Generate
//! and Copy plan, Task 3). Follows the old app's `HitsoundCopier`, which follows Mapping Tools 2.0's
//! Hitsound Copier (OliBomby, MIT): the same options with the same defaults. Each target is
//! decompiled, its moments are set from the source's as the options say, and compile writes them
//! with green lines (ADR 0037, ADR 0042). Planning opens and writes no file.
//!
//! Not built in M2 (they make or decode sample files): copying onto slider ticks and slides, the
//! start index for new samples, and never giving a silent hitnormal. A pile-up of source moments
//! that needs a new sample is a save problem with its mix as data.

use crate::compile::{compile, mix_problem, Compiled, Placement, SaveProblem};
use crate::decompile::{decompile, Decompiled};
use crate::lazer::control_points::{offset_for, ControlPoints};
use crate::model::{Hitsounds, LaneSource, Moment, MomentId, ObjectId, Place};
use crate::rules::{played_volume, MINIMUM_PLAYED_VOLUME};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_osu_format::{find_case_insensitive, OsuFile};
use std::collections::{BTreeSet, HashMap, HashSet};

/// What happens to target moments the source has nothing for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMode {
    /// They are cleared: each plays a hitnormal in the source's sample state at its time.
    OverwriteEverything,
    /// They keep their sounds.
    OverwriteOnlyDefined,
}

/// The options of the old app's Copy hitsounds, with its defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyOptions {
    pub mode: CopyMode,
    /// How far apart (ms, after rounding both times half-to-even) a source and a target moment may be.
    pub leniency: f64,
    /// Copy what each moment plays.
    pub copy_hitsounds: bool,
    /// Copy a slider's body sounds from the matched source slider.
    pub copy_body_hitsounds: bool,
    /// Copy sample sets and custom indexes (otherwise the target's are kept).
    pub copy_sample_sets: bool,
    /// Copy volumes (otherwise the target's are kept).
    pub copy_volumes: bool,
    /// With volumes copied, target moments at 5 % stay at 5 %.
    pub always_preserve_5_volume: bool,
    /// Copy the source's storyboard samples into the targets.
    pub copy_storyboarded_samples: bool,
    /// Skip a storyboard sample when a target moment within the leniency already plays its file.
    pub ignore_hitsound_satisfied_samples: bool,
    /// Skip a storyboard sample whenever a target moment is within the leniency.
    pub ignore_whenever_hitsound: bool,
    /// Mute target slider and spinner ends the source has nothing for, filtered by snap and length.
    pub mute_slider_ends: bool,
    /// Beat divisors in priority order: an end snaps to the nearest, and the first it falls on decides.
    pub beat_divisors: Vec<u32>,
    /// The divisors whose ends are muted.
    pub muted_divisors: Vec<u32>,
    /// Only sliders at least this long (in beats) get their end muted.
    pub min_length: f64,
    /// The custom index muted ends play from; `None` keeps theirs.
    pub muted_index: Option<u32>,
    /// The sample set muted ends play in; `None` keeps theirs.
    pub muted_sample_set: Option<SampleSet>,
}

impl Default for CopyOptions {
    fn default() -> CopyOptions {
        CopyOptions {
            mode: CopyMode::OverwriteEverything,
            leniency: 5.0,
            copy_hitsounds: true,
            copy_body_hitsounds: true,
            copy_sample_sets: true,
            copy_volumes: true,
            always_preserve_5_volume: true,
            copy_storyboarded_samples: false,
            ignore_hitsound_satisfied_samples: true,
            ignore_whenever_hitsound: false,
            mute_slider_ends: false,
            beat_divisors: vec![1, 4, 3, 8, 6, 16, 12],
            muted_divisors: vec![4, 3, 8, 6, 16, 12],
            min_length: 0.5,
            muted_index: None,
            muted_sample_set: None,
        }
    }
}

/// One target's plan.
#[derive(Debug, Clone)]
pub struct TargetPlan {
    /// The target with its new hitsounds: write it over the target's own file (`save_over`).
    pub file: OsuFile,
    /// Compile's save problems, and pile-ups that need a sample mix; each such moment keeps its sound.
    pub problems: Vec<SaveProblem>,
    /// Source moments with no target moment within the leniency, by their time in the source.
    pub dropped: Vec<SongTime>,
    /// Source moments matched to a target moment (a later one replacing an earlier one counts too).
    pub matched: usize,
    /// Target moments given a new sound or volume.
    pub changed: usize,
    /// Slider and spinner ends muted.
    pub muted: usize,
}

#[derive(Debug, Clone)]
pub enum TargetOutcome {
    Planned(Box<TargetPlan>),
    /// A taiko, catch or mania target.
    UnsupportedMode { mode: i32 },
    /// The target cannot be planned with these options; nothing changes in it.
    Refused(String),
}

#[derive(Debug, Clone)]
pub enum CopyOutcome {
    /// One outcome per target, in the order given.
    Targets(Vec<TargetOutcome>),
    InvalidOptions(String),
    Cancelled,
}

/// Plans Copy hitsounds from `source` (decompiled from `source_file`, edits included) into each of
/// `targets`, which are difficulties of the same mapset, so they share `folder_files`. The caller
/// keeps the source out of the targets and gives each target once (`dawsu-application`).
/// `cancel` is looked at between targets and inside decompile and compile; `progress` gets
/// (targets done, targets in total).
pub fn copy_hitsounds(
    source: &Hitsounds,
    source_file: &OsuFile,
    targets: &[OsuFile],
    folder_files: &[String],
    options: &CopyOptions,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> CopyOutcome {
    if let Err(why) = validate(options) {
        return CopyOutcome::InvalidOptions(why);
    }
    let mut outcomes = Vec::with_capacity(targets.len());
    for (done, target) in targets.iter().enumerate() {
        if cancel() {
            return CopyOutcome::Cancelled;
        }
        match plan_target(source, source_file, target, folder_files, options, cancel) {
            Some(outcome) => outcomes.push(outcome),
            None => return CopyOutcome::Cancelled,
        }
        progress(done + 1, targets.len());
    }
    CopyOutcome::Targets(outcomes)
}

fn validate(o: &CopyOptions) -> Result<(), String> {
    if !o.leniency.is_finite() || o.leniency < 0.0 {
        return Err("the leniency must be a number of milliseconds, 0 or more".into());
    }
    if o.beat_divisors.iter().chain(&o.muted_divisors).any(|d| *d == 0) {
        return Err("beat divisors must be 1 or more".into());
    }
    if !o.min_length.is_finite() || o.min_length < 0.0 {
        return Err("the minimum slider length must be 0 beats or more".into());
    }
    Ok(())
}

/// A moment's time as written in its file: song time less the format-version offset (24 ms
/// before v5). Copy matches on file times, as the old app did (plan decision D).
fn file_time(m: &Moment, offset: f64) -> f64 {
    m.time().ms() - offset
}

fn round(t: f64) -> f64 {
    // .NET's Math.Round, which the old app used, rounds half to even.
    t.round_ties_even()
}

/// The index into `times` (sorted) of the nearest time: on a tie between two different times the
/// earlier wins (old `HitsoundCopier.Nearest`).
fn nearest(times: &[f64], time: f64) -> Option<usize> {
    if times.is_empty() {
        return None;
    }
    let i = times.partition_point(|t| *t < time);
    if i < times.len() && times[i] == time {
        return Some(i);
    }
    if i == 0 {
        return Some(0);
    }
    if i == times.len() {
        return Some(times.len() - 1);
    }
    Some(if time - times[i - 1] <= times[i] - time { i - 1 } else { i })
}

/// Matches source moments to target moments: each source moment goes to the target moment
/// nearest in time, kept when both times rounded half-to-even differ by at most `leniency`. When
/// several target moments share that time, the k-th source moment at one source time goes to the
/// k-th of them in object order, and extra ones pile up on the last (plan decision A). A later
/// source moment replaces an earlier one at the same target; source moments at the same time pile
/// up. Returns, per target index, the matched source indices, and the source indices dropped.
pub(crate) fn match_moments(source_times: &[f64], target_times: &[f64], leniency: f64) -> (HashMap<usize, Vec<usize>>, Vec<usize>) {
    let mut distinct: Vec<f64> = target_times.to_vec();
    distinct.dedup();
    let mut matched: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut dropped = Vec::new();
    // How many source moments at one source time already went to one target time.
    let mut taken: HashMap<(u64, u64), usize> = HashMap::new();
    for (s, &time) in source_times.iter().enumerate() {
        let Some(d) = nearest(&distinct, time) else {
            dropped.push(s);
            continue;
        };
        let at = distinct[d];
        if (round(time) - round(at)).abs() > leniency {
            dropped.push(s);
            continue;
        }
        let first = target_times.partition_point(|t| *t < at);
        let count = target_times[first..].iter().take_while(|t| **t == at).count();
        let k = taken.entry((time.to_bits(), at.to_bits())).or_insert(0);
        let target = first + (*k).min(count - 1);
        *k += 1;
        let list = matched.entry(target).or_default();
        // A later source moment at another time replaces what was there.
        if list.last().is_some_and(|&p| source_times[p] != time) {
            list.clear();
        }
        list.push(s);
    }
    (matched, dropped)
}

/// A sample state: the set, index and volume a timing line gives.
#[derive(Debug, Clone, Copy)]
struct State {
    set: SampleSet,
    index: u32,
    volume: i32,
}

fn state_at(cp: &ControlPoints, song_time: f64) -> State {
    match cp.sample_point_at(song_time) {
        Some(p) => State {
            set: match p.bank.as_str() {
                "soft" => SampleSet::Soft,
                "drum" => SampleSet::Drum,
                _ => SampleSet::Normal,
            },
            index: p.custom_sample_bank.max(0) as u32,
            volume: p.volume,
        },
        None => State { set: SampleSet::Normal, index: 0, volume: 100 },
    }
}

fn hitnormal(set: SampleSet, index: u32) -> SampleIdentity {
    SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Normal), index: CustomIndex::new(index) }
}

/// The same sounds in another set and index, as the target's own green lines give them; additions
/// in a set of their own (not the hitnormal's) keep it, and other lanes stay (old `Retarget`).
fn retarget(lanes: &[SampleIdentity], set: SampleSet, index: u32) -> Vec<SampleIdentity> {
    let normal_set = lanes.iter().find_map(|l| match l {
        SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Normal), .. } => Some(*set),
        _ => None,
    });
    lanes
        .iter()
        .map(|l| match l {
            SampleIdentity::Standard { set: own, sound: sound @ Sound::Hit(h), .. } => {
                let s = if *h == Hitsound::Normal || Some(*own) == normal_set { set } else { *own };
                SampleIdentity::Standard { set: s, sound: *sound, index: CustomIndex::new(index) }
            }
            other => other.clone(),
        })
        .collect()
}

/// Plans one target; `None` when cancelled.
fn plan_target(source: &Hitsounds, source_file: &OsuFile, target: &OsuFile, folder_files: &[String], options: &CopyOptions, cancel: &dyn Fn() -> bool) -> Option<TargetOutcome> {
    let mut to = match decompile(target, folder_files, cancel, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        Decompiled::UnsupportedMode { mode } => return Some(TargetOutcome::UnsupportedMode { mode }),
        Decompiled::Cancelled => return None,
    };
    let (source_offset, target_offset) = (offset_for(source_file.format_version()), offset_for(target.format_version()));
    let (source_cp, target_cp) = (ControlPoints::from_file(source_file), ControlPoints::from_file(target));

    // Source moments in time order (moments added to the model come last in it).
    let mut from: Vec<&Moment> = source.moments().iter().collect();
    from.sort_by(|a, b| a.time().ms().total_cmp(&b.time().ms()));
    let source_times: Vec<f64> = from.iter().map(|m| file_time(m, source_offset)).collect();
    let before: Vec<Moment> = to.moments().to_vec();
    let target_times: Vec<f64> = before.iter().map(|m| file_time(m, target_offset)).collect();

    // Without copying hitsounds nothing is matched, and no source moment counts as dropped.
    let (matched, dropped) = if options.copy_hitsounds { match_moments(&source_times, &target_times, options.leniency) } else { (HashMap::new(), Vec::new()) };
    let matched_count = matched.values().map(Vec::len).sum();
    let mut problems = Vec::new();
    let mut changed = 0;

    for (i, m) in before.iter().enumerate() {
        let time = target_times[i];
        let target_state = state_at(&target_cp, time + target_offset);
        let source_state = state_at(&source_cp, time + source_offset);
        let current: Vec<SampleIdentity> = m.hits().cloned().collect();
        let (lanes, mut volume) = match matched.get(&i) {
            Some(sources) if sources.len() > 1 => {
                // Source moments at one time pile up on this target moment.
                if !options.copy_sample_sets {
                    return Some(TargetOutcome::Refused("turn on copying sample sets and indexes to keep simultaneous source sounds on one target moment".into()));
                }
                let layers: Vec<(SampleIdentity, i32)> = sources
                    .iter()
                    .flat_map(|&s| {
                        let (moment, volume) = (from[s], if options.copy_volumes { played_volume(from[s].volume()) } else { 100 });
                        moment.hits().map(move |h| (h.clone(), volume))
                    })
                    .collect();
                let volumes: BTreeSet<i32> = layers.iter().map(|l| l.1).collect();
                let lanes: BTreeSet<SampleIdentity> = layers.iter().map(|l| l.0.clone()).collect();
                if volumes.len() > 1 || lanes.len() < layers.len() {
                    // One moment holds one volume: this needs a sample mix, which M2 cannot make.
                    problems.push(SaveProblem { time: m.time(), place: m.place(), kind: mix_problem(&layers) });
                    continue;
                }
                let volume = if options.copy_volumes { from[sources[0]].volume() } else { m.volume() };
                (lanes.into_iter().collect::<Vec<_>>(), volume)
            }
            Some(sources) => {
                let s = from[sources[0]];
                let own: Vec<SampleIdentity> = s.hits().cloned().collect();
                let lanes = if options.copy_sample_sets { own } else { retarget(&own, target_state.set, target_state.index) };
                (lanes, if options.copy_volumes { s.volume() } else { m.volume() })
            }
            None if options.mode == CopyMode::OverwriteEverything => {
                // The target's hitsounds are cleared: a hitnormal in the source's state at this time.
                let (set, index) = if options.copy_sample_sets { (source_state.set, source_state.index) } else { (target_state.set, target_state.index) };
                let lanes = if options.copy_hitsounds {
                    vec![hitnormal(set, index)]
                } else if options.copy_sample_sets {
                    retarget(&current, set, index)
                } else {
                    current.clone()
                };
                (lanes, if options.copy_volumes { source_state.volume } else { m.volume() })
            }
            None => continue,
        };
        if options.copy_volumes && options.always_preserve_5_volume && m.volume() == MINIMUM_PLAYED_VOLUME {
            volume = MINIMUM_PLAYED_VOLUME;
        }
        if set_moment(&mut to, MomentId(i), &lanes, volume) {
            changed += 1;
        }
    }

    let mut muted = 0;
    if options.mute_slider_ends {
        muted = mute_ends(&mut to, target, &target_cp, target_offset, &matched, options, &mut changed);
    }
    if options.copy_hitsounds {
        copy_bodies(source, &mut to, &before, &matched, &from, options);
    }

    let (mut file, compile_problems) = match compile(target, &to, Placement::GreenLines, cancel, &mut |_, _| {}) {
        Compiled::Done { file, problems } => (file, problems),
        Compiled::Cancelled => return None,
    };
    problems.extend(compile_problems);
    if options.copy_storyboarded_samples {
        copy_storyboard_samples(source_file, &mut file, &to, &target_times, folder_files, options);
    }
    problems.sort_by(|a, b| a.time.ms().total_cmp(&b.time.ms()));
    let dropped = dropped.into_iter().map(|s| from[s].time()).collect();
    Some(TargetOutcome::Planned(Box::new(TargetPlan { file, problems, dropped, matched: matched_count, changed, muted })))
}

/// Sets a moment's hits and volume; false when they are already what it holds.
pub(crate) fn set_moment(h: &mut Hitsounds, id: MomentId, lanes: &[SampleIdentity], volume: i32) -> bool {
    let current: BTreeSet<SampleIdentity> = h.moment(id).hits().cloned().collect();
    let wanted: BTreeSet<SampleIdentity> = lanes.iter().cloned().collect();
    if current == wanted && h.moment(id).volume() == volume {
        return false;
    }
    for lane in current.difference(&wanted) {
        h.remove_hit(id, lane);
    }
    for lane in wanted.difference(&current) {
        h.add_hit(id, lane.clone());
    }
    h.set_volume(id, volume);
    true
}

/// Mapping Tools' slider-end filter and mute (old `IsMutable`): an unmatched end of a slider
/// without repeats or of a spinner, with no additions, at least `min_length` beats long, whose
/// time snaps first to a muted divisor, gets a hitnormal at 5 %. Returns how many were muted.
fn mute_ends(to: &mut Hitsounds, target: &OsuFile, cp: &ControlPoints, offset: f64, matched: &HashMap<usize, Vec<usize>>, o: &CopyOptions, changed: &mut usize) -> usize {
    let starts: HashMap<ObjectId, f64> = target.hit_objects().iter().map(|h| (ObjectId(h.line()), h.time() + offset)).collect();
    let mut last_node: HashMap<ObjectId, usize> = HashMap::new();
    for m in to.moments() {
        if let Place::SliderNode { object, node } = m.place() {
            let e = last_node.entry(object).or_default();
            *e = (*e).max(node);
        }
    }
    let mut muted = 0;
    for i in 0..to.moments().len() {
        let m = to.moments()[i].clone();
        let object = match m.place() {
            Place::SliderNode { object, node: 1 } if last_node.get(&object) == Some(&1) => object,
            Place::SpinnerEnd(object) => object,
            _ => continue,
        };
        if matched.contains_key(&i) || m.hits().any(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(h), .. } if *h != Hitsound::Normal)) {
            continue;
        }
        let time = m.time().ms();
        if !cp.timing.iter().any(|t| t.time <= time - 1.0) {
            continue;
        }
        let red = cp.timing_point_at(time - 1.0);
        if red.beat_length <= 0.0 || time - starts.get(&object).copied().unwrap_or(time) < o.min_length * red.beat_length - 1e-6 {
            continue;
        }
        let all: Vec<f64> = o.beat_divisors.iter().chain(&o.muted_divisors).map(|d| f64::from(*d)).collect();
        let snapped = all
            .iter()
            .map(|d| red.time + ((time - red.time) / (red.beat_length / d)).round_ties_even() * (red.beat_length / d))
            .fold((f64::INFINITY, 0.0), |best, t| if (t - time).abs() < best.0 { ((t - time).abs(), t) } else { best })
            .1;
        let beats = (snapped - red.time) / red.beat_length;
        let on = |d: f64| ((beats * d) - (beats * d).round_ties_even()).abs() < 1e-6;
        let Some(first) = all.iter().copied().find(|d| on(*d)) else { continue };
        if !o.muted_divisors.iter().any(|d| f64::from(*d) == first) {
            continue;
        }
        let normal = m.hits().find_map(|l| match l {
            SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Normal), index } => Some((*set, index.get())),
            _ => None,
        });
        let state = state_at(cp, time);
        let set = o.muted_sample_set.or(normal.map(|n| n.0)).unwrap_or(state.set);
        let index = o.muted_index.or(normal.map(|n| n.1)).unwrap_or(state.index);
        muted += 1;
        if set_moment(to, MomentId(i), &[hitnormal(set, index)], MINIMUM_PLAYED_VOLUME) {
            *changed += 1;
        }
    }
    muted
}

/// Slider bodies (plan decision B): a target slider whose head took a source slider head's sounds
/// takes that slider's body whistle and set; under Overwrite everything an unmatched slider's body
/// loses its whistle and keeps the set its timing line gives. Under Copy volumes, when this copy
/// changes the head's volume, the body follows it (ADR 0042 point 6); a body whose volume already
/// differs from its head's keeps it while the head is untouched.
fn copy_bodies(source: &Hitsounds, to: &mut Hitsounds, before: &[Moment], matched: &HashMap<usize, Vec<usize>>, from: &[&Moment], o: &CopyOptions) {
    let head_of: HashMap<ObjectId, usize> = to
        .moments()
        .iter()
        .enumerate()
        .filter_map(|(i, m)| match m.place() {
            Place::SliderNode { object, node: 0 } => Some((object, i)),
            _ => None,
        })
        .collect();
    for b in 0..to.bodies().len() {
        let object = to.bodies()[b].object();
        let Some(&head) = head_of.get(&object) else { continue };
        match matched.get(&head).and_then(|s| s.last()) {
            Some(&s) => {
                if !o.copy_body_hitsounds {
                    continue;
                }
                let Place::SliderNode { object: source_object, node: 0 } = from[s].place() else { continue };
                let Some(src) = source.bodies().iter().find(|x| x.object() == source_object) else { continue };
                to.set_body_whistle(b, src.whistle());
                if let Some(set) = src.set() {
                    if to.bodies()[b].set() != Some(set) {
                        to.set_body_set(b, set);
                    }
                }
            }
            None if o.mode == CopyMode::OverwriteEverything => to.set_body_whistle(b, false),
            None => continue,
        }
        let head_volume = to.moments()[head].volume();
        if o.copy_volumes && head_volume != before[head].volume() && head_volume != to.bodies()[b].volume() {
            to.set_body_volume(b, head_volume);
        }
    }
}

/// A sample file as osu! finds it: the folder's file with that name, or with that name and
/// another extension, lower-cased with `/` (old `Canonical`).
fn canonical(folder_files: &[String], path: &str) -> String {
    let stem = |p: &str| {
        let p = p.replace('\\', "/");
        match p.rfind('.') {
            Some(dot) if !p[dot..].contains('/') => p[..dot].to_lowercase(),
            _ => p.to_lowercase(),
        }
    };
    let found = find_case_insensitive(folder_files, path).map(str::to_string).or_else(|| folder_files.iter().find(|f| stem(f) == stem(path)).cloned());
    found.unwrap_or_else(|| path.to_string()).replace('\\', "/").to_lowercase()
}

/// Storyboard samples (old app, Mapping Tools): Overwrite everything removes the target's first;
/// a source sample is skipped when a target moment within the leniency already plays its file (or,
/// stricter, whenever there is one), and never doubled.
fn copy_storyboard_samples(source_file: &OsuFile, file: &mut OsuFile, to: &Hitsounds, target_times: &[f64], folder_files: &[String], o: &CopyOptions) {
    let clear = o.mode == CopyMode::OverwriteEverything;
    let key = |time: f64, layer: &str, path: &str, volume: f32| (time.to_bits(), layer.to_string(), path.to_lowercase(), volume.to_bits());
    let mut existing: HashSet<(u64, String, String, u32)> = HashSet::new();
    for s in file.storyboard_samples() {
        if clear {
            let _ = file.remove_storyboard_sample(s.line);
        } else {
            existing.insert(key(s.time, &s.layer, &s.path, s.volume));
        }
    }
    for s in source_file.storyboard_samples() {
        let near: Vec<usize> = (0..target_times.len()).filter(|&i| (target_times[i] - s.time).abs() <= o.leniency).collect();
        if o.ignore_whenever_hitsound && !near.is_empty() {
            continue;
        }
        if o.ignore_hitsound_satisfied_samples {
            let wanted = canonical(folder_files, &s.path);
            let playing = near.iter().any(|&i| {
                to.moments()[i].hits().any(|h| matches!(to.lane(h).map(|l| &l.source), Some(LaneSource::File(name)) if canonical(folder_files, name) == wanted))
            });
            if playing {
                continue;
            }
        }
        if !existing.insert(key(s.time, &s.layer, &s.path, s.volume)) {
            continue;
        }
        file.insert_storyboard_sample(&format!("Sample,{},{},\"{}\",{}", number(s.time), s.layer, s.path, number(f64::from(s.volume))));
    }
}

/// A number with at most three decimals and no trailing zeros (.NET `0.###`).
fn number(v: f64) -> String {
    let text = format!("{:.3}", v);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_time_takes_the_earlier_on_a_tie() {
        assert_eq!(nearest(&[1000.0, 1010.0], 1005.0), Some(0));
        assert_eq!(nearest(&[1000.0, 1010.0], 1005.1), Some(1));
        assert_eq!(nearest(&[], 1.0), None);
    }

    fn pairs(source: &[f64], target: &[f64]) -> (Vec<(usize, Vec<usize>)>, Vec<usize>) {
        let (m, d) = match_moments(source, target, 5.0);
        let mut m: Vec<_> = m.into_iter().collect();
        m.sort();
        (m, d)
    }

    #[test]
    fn a_source_moment_goes_to_the_nearest_target_within_5_ms() {
        assert_eq!(pairs(&[1000.0], &[990.0, 1004.0]), (vec![(1, vec![0])], vec![]));
        // A_hit_is_copied_only_within_the_leniency (old test, 4 and 5 ms in, 6 ms out).
        assert_eq!(pairs(&[1000.0], &[1005.0]).0.len(), 1);
        assert_eq!(pairs(&[1000.0], &[1006.0]), (vec![], vec![0]));
    }

    #[test]
    fn times_are_rounded_half_to_even_before_the_leniency_test() {
        // 1002.5 rounds to 1002 (half to even), 6 ms from 1008: dropped. Rounding half away from
        // zero would give 1003, 5 ms, and match.
        assert_eq!(pairs(&[1002.5], &[1008.0]), (vec![], vec![0]));
        assert_eq!(pairs(&[1003.5], &[1008.0]).0.len(), 1, "1003.5 rounds to 1004");
    }

    #[test]
    fn a_later_source_moment_replaces_an_earlier_one_and_equal_times_pile_up() {
        // Nearby_but_distinct_source_times_retain_the_existing_last_match_policy (old test).
        assert_eq!(pairs(&[1000.0, 1001.0], &[1000.0]), (vec![(0, vec![1])], vec![]));
        assert_eq!(pairs(&[1000.0, 1000.0], &[1000.0]), (vec![(0, vec![0, 1])], vec![]));
    }

    #[test]
    fn same_time_targets_are_taken_in_order() {
        // Plan decision A: a zero-length slider's head and tail, or stacked circles.
        assert_eq!(pairs(&[1000.0, 1000.0], &[1000.0, 1000.0]), (vec![(0, vec![0]), (1, vec![1])], vec![]));
        assert_eq!(pairs(&[1000.0, 1000.0, 1000.0], &[1000.0, 1000.0]), (vec![(0, vec![0]), (1, vec![1, 2])], vec![]), "extra ones pile up on the last");
        assert_eq!(pairs(&[1000.0], &[1000.0, 1000.0]), (vec![(0, vec![0])], vec![]));
    }

    #[test]
    fn numbers_print_like_dotnet_s_three_decimals() {
        assert_eq!((number(1500.0), number(12.25), number(0.0004), number(80.0)), ("1500".into(), "12.25".into(), "0".into(), "80".into()));
    }
}
