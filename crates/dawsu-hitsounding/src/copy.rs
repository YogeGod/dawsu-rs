//! Copy hitsounds (design doc "Generate and Copy hitsounds", Responsibilities 7-12; the Generate
//! and Copy plan, Task 3). Follows the old app's `HitsoundCopier`, which follows Mapping Tools 2.0's
//! Hitsound Copier (OliBomby, MIT), except nearby source hits in the leniency window merge by default
//! (`CopyCollisionPolicy::Merge`, DECISIONS.md) instead of letting the later moment win. Each target is
//! decompiled, its moments are set from the source's as the options say, and compile writes them
//! with green lines (ADR 0037, ADR 0042). Planning opens and writes no file.
//!
//! A pile-up of source moments that needs a new sample is a save problem with its mix as data.
//! Application chooses sample indexes and identifies silent mapset files.

use crate::compile::{compile, mix_problem, Compiled, Placement, SaveProblem};
use crate::decompile::{decompile, Decompiled};
use crate::index_packing::AdditionOnlyNormal;
use crate::lazer::control_points::{offset_for, ControlPoints};
use crate::model::{Hitsounds, LaneSource, Moment, MomentId, ObjectId, Place};
use crate::rules::{played_volume, MINIMUM_PLAYED_VOLUME};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, SongTime, Sound};
use dawsu_osu_format::{find_case_insensitive, OsuFile};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[path = "copy_slider.rs"]
mod slider;

/// What happens to target moments the source has nothing for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyMode {
    /// They are cleared: each plays a hitnormal in the source's sample state at its time.
    OverwriteEverything,
    /// They keep their sounds.
    OverwriteOnlyDefined,
}

/// What happens when multiple source moments at different times fall inside one target moment's leniency window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopyCollisionPolicy {
    /// Collect all source hits in the window and hand the combined layer to compile (default).
    #[default]
    Merge,
    /// A later source moment replaces earlier matches for the same target moment.
    LaterWins,
}

/// Automatic slider-end (and optional spinner-end) muting options.
///
/// Off by default (`enabled: false`) so Copy/Export leaves slider and spinner ends alone unless
/// the mapper opts in. Muted ends always respect osu!'s 5% saved volume floor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SliderEndMuteOptions {
    /// Master toggle. Default `false`.
    pub enabled: bool,
    /// Minimum slider length in beats before its end can be muted (default `0.5`).
    /// Sliders shorter than this are never muted.
    pub min_beats: f64,
    /// Also mute un-hit spinner ends (default `false`).
    pub mute_spinner_ends: bool,
    /// Only mute ends that do not already have a placed/matched hitsound (default `true`).
    pub only_unhit: bool,
    /// Beat divisors whose ends are muted (e.g. `[2, 4]`, `[3]`, `[4, 3, 8, 6, 16, 12]`).
    /// Empty means mute on any beat divisor.
    pub divisors: Vec<u32>,
}

impl Default for SliderEndMuteOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            min_beats: 0.5,
            mute_spinner_ends: false,
            only_unhit: true,
            divisors: Vec::new(),
        }
    }
}

/// Optional fallback sound applied to unmatched target gameplay object edges during Copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FallbackSound {
    #[serde(with = "sample_set_serde")]
    pub set: SampleSet,
    #[serde(default = "default_fallback_hitsound", with = "hitsound_serde")]
    pub sound: Hitsound,
    #[serde(default = "default_fallback_volume", deserialize_with = "deserialize_fallback_volume")]
    pub volume: i32,
}

impl FallbackSound {
    pub fn new(set: SampleSet, sound: Hitsound, volume: i32) -> Self {
        Self {
            set,
            sound,
            volume: volume.clamp(MINIMUM_PLAYED_VOLUME, 100),
        }
    }

    pub fn clamped_volume(self) -> i32 {
        self.volume.clamp(MINIMUM_PLAYED_VOLUME, 100)
    }
}

fn default_fallback_hitsound() -> Hitsound {
    Hitsound::Normal
}

fn default_fallback_volume() -> i32 {
    80
}

fn deserialize_fallback_volume<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <i32 as serde::Deserialize>::deserialize(deserializer)?;
    Ok(raw.clamp(MINIMUM_PLAYED_VOLUME, 100))
}

mod sample_set_serde {
    use dawsu_foundation::SampleSet;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(set: &SampleSet, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match set {
            SampleSet::Normal => "normal",
            SampleSet::Soft => "soft",
            SampleSet::Drum => "drum",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SampleSet, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.to_ascii_lowercase().as_str() {
            "normal" => Ok(SampleSet::Normal),
            "soft" => Ok(SampleSet::Soft),
            "drum" => Ok(SampleSet::Drum),
            other => Err(serde::de::Error::custom(format!("unknown sample set '{other}'"))),
        }
    }
}

mod hitsound_serde {
    use dawsu_foundation::Hitsound;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(sound: &Hitsound, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match sound {
            Hitsound::Normal => "normal",
            Hitsound::Whistle => "whistle",
            Hitsound::Finish => "finish",
            Hitsound::Clap => "clap",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Hitsound, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.to_ascii_lowercase().as_str() {
            "normal" => Ok(Hitsound::Normal),
            "whistle" => Ok(Hitsound::Whistle),
            "finish" => Ok(Hitsound::Finish),
            "clap" => Ok(Hitsound::Clap),
            other => Err(serde::de::Error::custom(format!("unknown hitsound '{other}'"))),
        }
    }
}

/// Options for Copy hitsounds (matching Mapping Tools' defaults, except Merge is the default collision policy).
#[derive(Debug, Clone, PartialEq)]
pub struct CopyOptions {
    pub mode: CopyMode,
    /// How far apart (ms, after rounding both times half-to-even) a source and a target moment may be.
    pub leniency: f64,
    /// What happens when multiple source moments at different times match one target moment.
    pub collision_policy: CopyCollisionPolicy,
    /// The first custom index for new mixed samples.
    pub start_index: u32,
    /// Keep the target's existing sound if Copy would supply a silent hitnormal.
    pub never_silent_hitnormals: bool,
    /// Copy what each moment plays.
    pub copy_hitsounds: bool,
    /// Copy a slider's body sounds from the matched source slider.
    pub copy_body_hitsounds: bool,
    /// Place unmatched source hits at nearby slider ticks as timed storyboard audio.
    pub copy_to_slider_ticks: bool,
    /// Place unmatched source hits during slider slides as timed storyboard audio.
    pub copy_to_slider_slides: bool,
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
    /// Legacy toggle: mute target slider and spinner ends using `beat_divisors`/`muted_divisors`/`min_length`.
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
    /// Structured automatic slider-end (and optional spinner-end) muting options.
    pub slider_end_mute: SliderEndMuteOptions,
    /// Optional fallback sound for target gameplay object edges that did not match any source moment.
    pub fallback_sound: Option<FallbackSound>,
    /// What hitnormal accompanies addition-only moments (whistle, finish, or clap without a normal).
    pub addition_only_normal: AdditionOnlyNormal,
    /// Pre-reserved custom index holding the 44-byte silent hitnormal when `addition_only_normal == Silent`.
    pub silent_normal_index: Option<u32>,
}

impl Default for CopyOptions {
    fn default() -> CopyOptions {
        CopyOptions {
            mode: CopyMode::OverwriteEverything,
            leniency: 5.0,
            collision_policy: CopyCollisionPolicy::Merge,
            start_index: 100,
            never_silent_hitnormals: false,
            copy_hitsounds: true,
            copy_body_hitsounds: true,
            copy_to_slider_ticks: false,
            copy_to_slider_slides: false,
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
            slider_end_mute: SliderEndMuteOptions::default(),
            fallback_sound: None,
            addition_only_normal: AdditionOnlyNormal::Difficulty,
            silent_normal_index: None,
        }
    }
}

/// Audio to play at a slider time, without changing any of its native samples.
#[derive(Debug, Clone)]
pub struct SliderCopyPlacement {
    pub time: SongTime,
    pub role: SliderSound,
    /// Volume of the storyboard event, after the layer-relative gains are rendered.
    pub volume: i32,
    pub layers: Vec<(SampleIdentity, i32)>,
}

/// A later source time displaced earlier matches for the same target moment.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyCollision {
    pub target_time: SongTime,
    pub target_place: Place,
    pub winning_source_time: SongTime,
    pub displaced_source_times: Vec<SongTime>,
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
    /// Earlier source moments replaced by a later match, in target order.
    pub collisions: Vec<CopyCollision>,
    /// Unmatched source hits routed to timed slider audio, to be staged for review.
    pub slider_placements: Vec<SliderCopyPlacement>,
    /// Source moments matched to a target moment (a later one replacing an earlier one counts too).
    pub matched: usize,
    /// Target moments given a new sound or volume.
    pub changed: usize,
    /// Target moments that kept their original sound to avoid a silent hitnormal.
    pub kept_from_silent: usize,
    /// Target moments where nearby source moments at different times were merged together.
    pub merged: usize,
    /// Slider and spinner ends muted.
    pub muted: usize,
    /// Unmatched target gameplay object edges given the optional fallback sound.
    pub fallback_applied: usize,
    /// Intended post-Copy hitsound state used for pre-write read-back verification.
    pub expected_hitsounds: Hitsounds,
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
/// (objects processed, objects across all targets).
pub fn copy_hitsounds(
    source: &Hitsounds,
    source_file: &OsuFile,
    targets: &[OsuFile],
    folder_files: &[String],
    options: &CopyOptions,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> CopyOutcome {
    copy_hitsounds_with_silence(source, source_file, targets, folder_files, options, &mut |_| false, cancel, progress)
}

/// Plans Copy using the caller's decoded mapset sample silence check.
#[allow(clippy::too_many_arguments)]
pub fn copy_hitsounds_with_silence(
    source: &Hitsounds,
    source_file: &OsuFile,
    targets: &[OsuFile],
    folder_files: &[String],
    options: &CopyOptions,
    is_silent: &mut dyn FnMut(&str) -> bool,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> CopyOutcome {
    if let Err(why) = validate(options) {
        return CopyOutcome::InvalidOptions(why);
    }
    let mut outcomes = Vec::with_capacity(targets.len());
    let mut counts = Vec::with_capacity(targets.len());
    for target in targets {
        if cancel() {
            return CopyOutcome::Cancelled;
        }
        counts.push(target.hit_objects().len());
    }
    let total = counts.iter().sum();
    let mut completed = 0;
    for (target, count) in targets.iter().zip(counts) {
        if cancel() {
            return CopyOutcome::Cancelled;
        }
        let mut report = |done: usize, _: usize| {
            if done < count {
                progress(completed + done, total);
            }
        };
        let ctx = CopyContext { source, source_file, folder_files, options };
        match plan_target(&ctx, target, is_silent, cancel, &mut report) {
            Some(outcome) => outcomes.push(outcome),
            None => return CopyOutcome::Cancelled,
        }
        completed += count;
        if count > 0 {
            progress(completed, total);
        }
    }
    CopyOutcome::Targets(outcomes)
}

fn validate(o: &CopyOptions) -> Result<(), String> {
    if !(2..=i32::MAX as u32).contains(&o.start_index) {
        return Err("The start index must be a whole number from 2 to 2147483647.".into());
    }
    if !o.leniency.is_finite() || o.leniency < 0.0 {
        return Err("the leniency must be a number of milliseconds, 0 or more".into());
    }
    if o.beat_divisors.iter().chain(&o.muted_divisors).chain(&o.slider_end_mute.divisors).any(|d| *d == 0) {
        return Err("beat divisors must be 1 or more".into());
    }
    if !o.min_length.is_finite() || o.min_length < 0.0 || !o.slider_end_mute.min_beats.is_finite() || o.slider_end_mute.min_beats < 0.0 {
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

#[derive(Default)]
struct Matches {
    retained: HashMap<usize, Vec<usize>>,
    displaced: BTreeMap<usize, Vec<usize>>,
    dropped: Vec<usize>,
}

/// Matches source moments to target moments: each source moment goes to the target moment
/// nearest in time, kept when both times rounded half-to-even differ by at most `leniency`. When
/// several target moments share that time, the k-th source moment at one source time goes to the
/// k-th of them in object order, and extra ones pile up on the last (plan decision A). Under
/// `CopyCollisionPolicy::Merge` (default), all source moments in the window are retained together;
/// under `CopyCollisionPolicy::LaterWins`, a later source moment replaces earlier ones at the same
/// target while equal-time source moments still pile up.
fn match_moments(source_times: &[f64], target_times: &[f64], leniency: f64, policy: CopyCollisionPolicy) -> Matches {
    let mut distinct: Vec<f64> = target_times.to_vec();
    distinct.dedup();
    let mut matches = Matches::default();
    // How many source moments at one source time already went to one target time.
    let mut taken: HashMap<(u64, u64), usize> = HashMap::new();
    for (s, &time) in source_times.iter().enumerate() {
        let Some(d) = nearest(&distinct, time) else {
            matches.dropped.push(s);
            continue;
        };
        let at = distinct[d];
        if (round(time) - round(at)).abs() > leniency {
            matches.dropped.push(s);
            continue;
        }
        let first = target_times.partition_point(|t| *t < at);
        let count = target_times[first..].iter().take_while(|t| **t == at).count();
        let k = taken.entry((time.to_bits(), at.to_bits())).or_insert(0);
        let target = first + (*k).min(count - 1);
        *k += 1;
        let list = matches.retained.entry(target).or_default();
        // Under LaterWins, a later source moment at another time replaces what was there.
        if policy == CopyCollisionPolicy::LaterWins
            && list.last().is_some_and(|&p| source_times[p] != time)
        {
            matches.displaced.entry(target).or_default().append(list);
        }
        list.push(s);
    }
    matches
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

/// Applies `AdditionOnlyNormal` when a matched moment carries only additions (whistle/finish/clap)
/// and no hitnormal or custom sample file.
fn apply_addition_only_normal(
    lanes: &mut Vec<SampleIdentity>,
    target_set: SampleSet,
    policy: AdditionOnlyNormal,
    silent_index: Option<u32>,
) {
    if policy == AdditionOnlyNormal::Difficulty || lanes.is_empty() {
        return;
    }
    let has_normal_or_file = lanes.iter().any(|l| {
        matches!(
            l,
            SampleIdentity::File(_)
                | SampleIdentity::Standard {
                    sound: Sound::Hit(Hitsound::Normal),
                    ..
                }
        )
    });
    if has_normal_or_file {
        return;
    }
    let first_addition = lanes.iter().find_map(|l| match l {
        SampleIdentity::Standard {
            set,
            sound: Sound::Hit(Hitsound::Whistle | Hitsound::Finish | Hitsound::Clap),
            index,
        } => Some((*set, index.get())),
        _ => None,
    });
    let Some((addition_set, add_index)) = first_addition else {
        return;
    };
    match policy.resolve_normal_set(target_set, addition_set) {
        Some(resolved_set) => {
            lanes.insert(0, hitnormal(resolved_set, add_index));
        }
        None => {
            if let Some(silent_idx) = silent_index {
                for lane in lanes.iter_mut() {
                    if let SampleIdentity::Standard { index, .. } = lane
                        && index.get() == 0
                    {
                        *index = CustomIndex::new(silent_idx);
                    }
                }
                lanes.insert(0, hitnormal(SampleSet::Normal, silent_idx));
            }
        }
    }
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

struct CopyContext<'a> {
    source: &'a Hitsounds,
    source_file: &'a OsuFile,
    folder_files: &'a [String],
    options: &'a CopyOptions,
}

/// Plans one target; `None` when cancelled.
fn plan_target(ctx: &CopyContext<'_>, target: &OsuFile, is_silent: &mut dyn FnMut(&str) -> bool, cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize, usize)) -> Option<TargetOutcome> {
    let CopyContext { source, source_file, folder_files, options } = *ctx;
    let mut to = match decompile(target, folder_files, cancel, progress) {
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
    let Matches { retained: matched, displaced, dropped } = if options.copy_hitsounds {
        match_moments(&source_times, &target_times, options.leniency, options.collision_policy)
    } else {
        Matches::default()
    };
    let matched_count = matched.values().chain(displaced.values()).map(Vec::len).sum();
    let collisions = displaced.into_iter().map(|(target, sources)| CopyCollision {
        target_time: before[target].time(),
        target_place: before[target].place(),
        winning_source_time: from[matched[&target][0]].time(),
        displaced_source_times: sources.into_iter().map(|index| from[index].time()).collect(),
    }).collect();
    let mut problems = Vec::new();
    let mut changed = 0;
    let mut changed_moments: HashSet<usize> = HashSet::new();
    let mut kept_from_silent = 0;
    let mut merged = 0;
    let mut silent_normal = |lane: &SampleIdentity| {
        if !options.never_silent_hitnormals
            || matches!(lane, SampleIdentity::Standard { sound, .. } if *sound != Sound::Hit(Hitsound::Normal))
        {
            return false;
        }
        match crate::rules::resolve_one(&crate::model::legacy_sample(lane), folder_files).plays {
            crate::rules::Plays::BeatmapFile(file) => is_silent(&file),
            _ => false,
        }
    };

    for (i, m) in before.iter().enumerate() {
        if cancel() {
            return None;
        }
        let time = target_times[i];
        let target_state = state_at(&target_cp, time + target_offset);
        let source_state = state_at(&source_cp, time + source_offset);
        let current: Vec<SampleIdentity> = m.hits().cloned().collect();
        let preserve_volume = options.copy_volumes
            && options.always_preserve_5_volume && m.volume() == MINIMUM_PLAYED_VOLUME;
        let mut overrides = None;
        let (lanes, volume) = match matched.get(&i) {
            Some(sources) if sources.len() > 1 => {
                // Multiple source moments (simultaneous or merged within the leniency window) land on this target moment.
                let same_time = sources.iter().all(|&s| source_times[s] == source_times[sources[0]]);
                let mut layers: Vec<(SampleIdentity, i32)> = sources
                    .iter()
                    .flat_map(|&s| {
                        let moment = from[s];
                        let own: Vec<SampleIdentity> = moment.hits().cloned().collect();
                        let mapped = if options.copy_sample_sets {
                            own
                        } else {
                            retarget(&own, target_state.set, target_state.index)
                        };
                        moment.hits().cloned().zip(mapped).map(move |(orig, lane)| {
                            let gain = if options.copy_volumes && !preserve_volume { moment.hit_volume(&orig) } else { m.volume() };
                            (lane, played_volume(gain))
                        })
                    })
                    .collect();
                if !layers.is_empty() && layers.iter().all(|(lane, _)| silent_normal(lane)) {
                    kept_from_silent += 1;
                    continue;
                }
                if !same_time {
                    merged += 1;
                    // Merging nearby moments (Merge/Union) collapses identical (lane, volume) entries
                    // so two ordinary circles (each carrying a hitnormal) form a plain union instead
                    // of doubling the hitnormal into a sample mix.
                    let mut seen: HashSet<(SampleIdentity, i32)> = HashSet::new();
                    layers.retain(|entry| seen.insert(entry.clone()));
                }
                let volumes: BTreeSet<i32> = layers.iter().map(|l| l.1).collect();
                let lanes: BTreeSet<SampleIdentity> = layers.iter().map(|l| l.0.clone()).collect();
                if volumes.len() > 1 || lanes.len() < layers.len() {
                    if !options.copy_sample_sets {
                        return Some(TargetOutcome::Refused("turn on copying sample sets and indexes to combine multiple source sounds on one target moment".into()));
                    }
                    // Repeated lanes and incompatible gains need a sample mix.
                    problems.push(SaveProblem { time: m.time(), place: m.place(), kind: mix_problem(&layers) });
                    continue;
                }
                let volume = if options.copy_volumes && !preserve_volume {
                    layers.first().map_or(m.volume(), |(_, volume)| *volume)
                } else {
                    m.volume()
                };
                let mut lanes_vec = lanes.into_iter().collect::<Vec<_>>();
                apply_addition_only_normal(
                    &mut lanes_vec,
                    target_state.set,
                    options.addition_only_normal,
                    options.silent_normal_index,
                );
                (lanes_vec, volume)
            }
            Some(sources) => {
                let s = from[sources[0]];
                let own: Vec<SampleIdentity> = s.hits().cloned().collect();
                let mut lanes = if options.copy_sample_sets { own } else { retarget(&own, target_state.set, target_state.index) };
                apply_addition_only_normal(
                    &mut lanes,
                    target_state.set,
                    options.addition_only_normal,
                    options.silent_normal_index,
                );
                let volume = if !options.copy_volumes || preserve_volume {
                    m.volume()
                } else if s.has_hit_volume_overrides() {
                    let gains: Vec<_> = s.hits().map(|lane| s.hit_volume(lane)).collect();
                    let first = gains.first().copied().unwrap_or(s.volume());
                    if gains.iter().all(|gain| *gain == first) {
                        first
                    } else {
                        overrides = Some(gains);
                        s.volume()
                    }
                } else {
                    s.volume()
                };
                (lanes, volume)
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
                (lanes, if options.copy_volumes && !preserve_volume { source_state.volume } else { m.volume() })
            }
            None => continue,
        };
        if !matched.get(&i).is_some_and(|sources| sources.len() > 1)
            && lanes.iter().any(&mut silent_normal)
        {
            kept_from_silent += 1;
            continue;
        }
        let id = MomentId(i);
        if let Some(gains) = overrides {
            let distinct: BTreeSet<_> = lanes.iter().collect();
            if distinct.len() < lanes.len() {
                let layers: Vec<_> = lanes.into_iter().zip(gains).collect();
                problems.push(SaveProblem { time: m.time(), place: m.place(), kind: mix_problem(&layers) });
                continue;
            }
            set_moment(&mut to, id, &lanes, volume);
            for (lane, gain) in lanes.iter().zip(gains) {
                to.set_hit_volume(id, lane, gain);
            }
            if changed_moments.insert(i) {
                changed += 1;
            }
        } else if set_moment(&mut to, id, &lanes, volume) && changed_moments.insert(i) {
            changed += 1;
        }
    }

    let (muted, muted_indices) = if options.slider_end_mute.enabled || options.mute_slider_ends {
        mute_ends(&mut to, target, &target_cp, target_offset, &matched, options, &mut changed, &mut changed_moments)
    } else {
        (0, HashSet::new())
    };
    let mut fallback_applied = 0;
    if options.copy_hitsounds
        && let Some(fb) = options.fallback_sound
    {
        let fb_volume = fb.clamped_volume();
        let fb_lanes = if fb.sound == Hitsound::Normal {
            vec![hitnormal(fb.set, 0)]
        } else {
            vec![
                hitnormal(fb.set, 0),
                SampleIdentity::Standard {
                    set: fb.set,
                    sound: Sound::Hit(fb.sound),
                    index: CustomIndex::new(0),
                },
            ]
        };
        for (i, m) in before.iter().enumerate() {
            if matched.contains_key(&i)
                || muted_indices.contains(&i)
                || matches!(m.place(), Place::NoObject)
            {
                continue;
            }
            fallback_applied += 1;
            if set_moment(&mut to, MomentId(i), &fb_lanes, fb_volume) && changed_moments.insert(i) {
                changed += 1;
            }
        }
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
    let (dropped, slider_placements) = slider::plan(dropped, &from, to.bodies(), target_offset - source_offset, options, cancel)?;
    let dropped = dropped.into_iter().map(|s| from[s].time()).collect();
    Some(TargetOutcome::Planned(Box::new(TargetPlan {
        file,
        problems,
        dropped,
        collisions,
        slider_placements,
        matched: matched_count,
        changed,
        muted,
        kept_from_silent,
        merged,
        fallback_applied,
        expected_hitsounds: to,
    })))
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

/// Automatic slider-end (and optional spinner-end) muting.
/// Returns how many were muted and their moment indices.
#[allow(clippy::too_many_arguments)]
fn mute_ends(
    to: &mut Hitsounds,
    target: &OsuFile,
    cp: &ControlPoints,
    offset: f64,
    matched: &HashMap<usize, Vec<usize>>,
    o: &CopyOptions,
    changed: &mut usize,
    changed_moments: &mut HashSet<usize>,
) -> (usize, HashSet<usize>) {
    let starts: HashMap<ObjectId, f64> = target.hit_objects().iter().map(|h| (ObjectId(h.line()), h.time() + offset)).collect();
    let mut last_node: HashMap<ObjectId, usize> = HashMap::new();
    for m in to.moments() {
        if let Place::SliderNode { object, node } = m.place() {
            let e = last_node.entry(object).or_default();
            *e = (*e).max(node);
        }
    }
    let structured = o.slider_end_mute.enabled;
    let mut muted = 0;
    let mut muted_indices = HashSet::new();
    for i in 0..to.moments().len() {
        let m = to.moments()[i].clone();
        let (object, is_spinner) = match m.place() {
            Place::SliderNode { object, node } if structured && node >= 1 && last_node.get(&object) == Some(&node) => (object, false),
            Place::SliderNode { object, node: 1 } if !structured && last_node.get(&object) == Some(&1) => (object, false),
            Place::SpinnerEnd(object) if !structured || o.slider_end_mute.mute_spinner_ends => (object, true),
            _ => continue,
        };
        // Crucial rule: never mute an end that already has a matched source hitsound.
        if matched.contains_key(&i) {
            continue;
        }
        let has_placed_hitsound = m.hits().any(|l| match l {
            SampleIdentity::Standard { sound: Sound::Hit(h), .. } => *h != Hitsound::Normal,
            _ => true,
        });
        if (!structured || o.slider_end_mute.only_unhit) && has_placed_hitsound {
            continue;
        }
        let time = m.time().ms();
        let min_beats = if structured { o.slider_end_mute.min_beats } else { o.min_length };
        if !is_spinner || !structured {
            if !cp.timing.iter().any(|t| t.time <= time - 1.0) {
                continue;
            }
            let red = cp.timing_point_at(time - 1.0);
            if red.beat_length <= 0.0 {
                continue;
            }
            if !is_spinner && time - starts.get(&object).copied().unwrap_or(time) < min_beats * red.beat_length - 1e-6 {
                continue;
            }
            if structured {
                if !o.slider_end_mute.divisors.is_empty()
                    && !matches_beat_divisor(time, red.time, red.beat_length, &o.slider_end_mute.divisors)
                {
                    continue;
                }
            } else {
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
            }
        } else if !o.slider_end_mute.divisors.is_empty() {
            if !cp.timing.iter().any(|t| t.time <= time - 1.0) {
                continue;
            }
            let red = cp.timing_point_at(time - 1.0);
            if red.beat_length <= 0.0
                || !matches_beat_divisor(time, red.time, red.beat_length, &o.slider_end_mute.divisors)
            {
                continue;
            }
        }
        let normal = m.hits().find_map(|l| match l {
            SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Normal), index } => Some((*set, index.get())),
            _ => None,
        });
        let state = state_at(cp, time);
        let set = o.muted_sample_set.or(normal.map(|n| n.0)).unwrap_or(state.set);
        let index = o.muted_index.or(normal.map(|n| n.1)).unwrap_or(state.index);
        muted += 1;
        muted_indices.insert(i);
        if set_moment(to, MomentId(i), &[hitnormal(set, index)], MINIMUM_PLAYED_VOLUME) && changed_moments.insert(i) {
            *changed += 1;
        }
    }
    (muted, muted_indices)
}

/// Returns true if `time` snaps coarsest to one of `divisors` under the active red timing point.
fn matches_beat_divisor(time: f64, red_time: f64, beat_length: f64, divisors: &[u32]) -> bool {
    let mut priority: Vec<u32> = vec![1, 2, 4, 3, 8, 6, 16, 12];
    for &d in divisors {
        if !priority.contains(&d) {
            priority.push(d);
        }
    }
    let snapped = priority
        .iter()
        .map(|&d| {
            let step = beat_length / f64::from(d);
            red_time + ((time - red_time) / step).round_ties_even() * step
        })
        .fold((f64::INFINITY, 0.0), |best, t| if (t - time).abs() < best.0 { ((t - time).abs(), t) } else { best })
        .1;
    let beats = (snapped - red_time) / beat_length;
    let on = |d: u32| {
        let scaled = beats * f64::from(d);
        (scaled - scaled.round_ties_even()).abs() < 1e-6
    };
    let Some(first) = priority.into_iter().find(|&d| on(d)) else {
        return false;
    };
    divisors.contains(&first)
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
        let matched_source = matched.get(&head).and_then(|s| {
            s.iter()
                .rev()
                .find(|&&idx| matches!(from[idx].place(), Place::SliderNode { node: 0, .. }))
                .or_else(|| s.last())
        });
        match matched_source {
            Some(&s) => {
                if o.copy_body_hitsounds
                    && let Place::SliderNode { object: source_object, node: 0 } = from[s].place()
                        && let Some(src) = source.bodies().iter().find(|x| x.object() == source_object) {
                            to.set_body_whistle(b, src.whistle());
                            if o.copy_sample_sets {
                                if let Some(set) = src.set()
                                    && to.bodies()[b].set() != Some(set) {
                                        to.set_body_set(b, set);
                                    }
                                if to.bodies()[b].whistle_set() != src.whistle_set() {
                                    to.set_body_whistle_set(b, src.whistle_set());
                                }
                            }
                        }
            }
            None if o.mode == CopyMode::OverwriteEverything => {
                if o.copy_body_hitsounds {
                    to.set_body_whistle(b, false);
                    if o.copy_sample_sets && to.bodies()[b].whistle_set().is_some() {
                        to.set_body_whistle_set(b, None);
                    }
                }
            }
            None => continue,
        }
        let head_volume = to.moments()[head].volume();
        if o.copy_volumes && !to.moments()[head].has_hit_volume_overrides()
            && head_volume != before[head].volume() && head_volume != to.bodies()[b].volume()
        {
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
        // The path field as the source writes it (old app `HitsoundCopier`), quotes and all.
        file.insert_storyboard_sample(&format!("Sample,{},{},{},{}", number(s.time), s.layer, s.raw_path, number(f64::from(s.volume))));
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
        pairs_with(source, target, CopyCollisionPolicy::Merge)
    }

    fn pairs_with(source: &[f64], target: &[f64], policy: CopyCollisionPolicy) -> (Vec<(usize, Vec<usize>)>, Vec<usize>) {
        let Matches { retained, dropped, .. } = match_moments(source, target, 5.0, policy);
        let mut m: Vec<_> = retained.into_iter().collect();
        m.sort();
        (m, dropped)
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
    fn merge_default_collects_all_nearby_source_moments_and_later_wins_replaces_earlier() {
        // Default Merge/Union collects both 999 ms and 1001 ms onto 1000 ms with no collisions.
        let merged = match_moments(&[999.0, 1001.0], &[1000.0], 5.0, CopyCollisionPolicy::Merge);
        assert_eq!(merged.retained[&0], [0, 1]);
        assert!(merged.displaced.is_empty());

        // Optional LaterWins replaces earlier times while equal times still pile up.
        assert_eq!(pairs_with(&[1000.0, 1001.0], &[1000.0], CopyCollisionPolicy::LaterWins), (vec![(0, vec![1])], vec![]));
        assert_eq!(pairs_with(&[1000.0, 1000.0], &[1000.0], CopyCollisionPolicy::LaterWins), (vec![(0, vec![0, 1])], vec![]));
    }

    #[test]
    fn collisions_keep_earlier_groups_separate_from_equal_time_layers() {
        let matches = match_moments(&[1998.0, 1999.0, 1999.0, 2002.0, 2002.0], &[2000.0], 5.0, CopyCollisionPolicy::LaterWins);
        assert_eq!(matches.retained[&0], [3, 4]);
        assert_eq!(matches.displaced[&0], [0, 1, 2]);
        assert!(matches.dropped.is_empty());

        let simultaneous = match_moments(&[1999.0, 1999.0], &[2000.0], 5.0, CopyCollisionPolicy::LaterWins);
        assert_eq!(simultaneous.retained[&0], [0, 1]);
        assert!(simultaneous.displaced.is_empty());
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

    fn test_map(objects: &str) -> OsuFile {
        let text = format!(
            "osu file format v14\r\n\r\n[General]\r\nMode:0\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,2,1,60,1,0\r\n\r\n[HitObjects]\r\n{objects}\r\n"
        );
        OsuFile::read(text.into_bytes()).file
    }

    fn run_plan(source: &OsuFile, target: &OsuFile, options: &CopyOptions) -> TargetPlan {
        let dec = match decompile(source, &[], &|| false, &mut |_, _| {}) {
            Decompiled::Done(h) => h,
            _ => panic!("decompile failed"),
        };
        match copy_hitsounds(&dec, source, std::slice::from_ref(target), &[], options, &|| false, &mut |_, _| {}) {
            CopyOutcome::Targets(mut list) => match list.remove(0) {
                TargetOutcome::Planned(plan) => *plan,
                other => panic!("unexpected target outcome: {other:?}"),
            },
            other => panic!("unexpected copy outcome: {other:?}"),
        }
    }

    fn moment_volumes(file: &OsuFile) -> Vec<(f64, Place, i32, Vec<SampleIdentity>)> {
        let parsed = OsuFile::read(file.to_bytes()).file;
        let dec = match decompile(&parsed, &[], &|| false, &mut |_, _| {}) {
            Decompiled::Done(h) => h,
            _ => panic!("decompile failed"),
        };
        dec.moments()
            .iter()
            .map(|m| (m.time().ms(), m.place(), m.volume(), m.hits().cloned().collect()))
            .collect()
    }

    #[test]
    fn slider_end_mute_default_is_disabled_and_changes_nothing() {
        let source = test_map("256,192,1000,1,0,0:0:0:0:");
        let len = 1.4 * 100.0 * 0.75; // 0.75 beats -> ends at 1375 ms
        let target = test_map(&format!("100,100,1000,2,0,L|300:100,1,{len},0|0,0:0|0:0,0:0:0:0:"));
        let options = CopyOptions::default();
        assert!(!options.slider_end_mute.enabled);
        let plan = run_plan(&source, &target, &options);
        assert_eq!(plan.muted, 0);
        let moments = moment_volumes(&plan.file);
        assert_eq!(moments.last().unwrap().2, 60, "unmuted slider end stays at 60%");
    }

    #[test]
    fn slider_end_mute_mutes_unhit_slider_ends_above_min_beats_and_preserves_matched_ends() {
        // Source has a hit at 1000 (slider 1 head), a hit at 2500 (slider 2 tail with clap), and nothing at 1375 (slider 1 tail) or 3125 (short slider 3 tail).
        let source = test_map(concat!(
            "256,192,1000,1,0,0:0:0:0:\r\n",
            "256,192,2500,1,8,0:0:0:80:",
        ));
        let len_075 = 1.4 * 100.0 * 0.75; // 0.75 beats (>= 0.5) -> 1000..1375 (un-hit)
        let len_100 = 1.4 * 100.0 * 1.0;  // 1.00 beats (>= 0.5) -> 2000..2500 (HIT by source at 2500!)
        let len_025 = 1.4 * 100.0 * 0.25; // 0.25 beats (< 0.5)  -> 3000..3125 (too short)
        let target = test_map(&format!(
            "100,100,1000,2,0,L|300:100,1,{len_075},0|0,0:0|0:0,0:0:0:0:\r\n\
             100,100,2000,2,0,L|300:100,1,{len_100},0|0,0:0|0:0,0:0:0:0:\r\n\
             100,100,3000,2,0,L|300:100,1,{len_025},0|0,0:0|0:0,0:0:0:0:"
        ));
        let options = CopyOptions {
            slider_end_mute: SliderEndMuteOptions {
                enabled: true,
                min_beats: 0.5,
                mute_spinner_ends: false,
                only_unhit: true,
                divisors: Vec::new(),
            },
            ..CopyOptions::default()
        };
        let plan = run_plan(&source, &target, &options);
        assert_eq!(plan.muted, 1, "only the un-hit slider >= 0.5 beats should be muted");
        let moments = moment_volumes(&plan.file);
        // Slider 1 tail at 1375 ms -> muted to 5% (never < 5%)
        let tail1 = moments.iter().find(|m| (m.0 - 1375.0).abs() < 1.0).unwrap();
        assert_eq!(tail1.2, MINIMUM_PLAYED_VOLUME);
        assert!(tail1.2 >= 5);
        // Slider 2 tail at 2500 ms -> matched source clap at 80%, NEVER muted!
        let tail2 = moments.iter().find(|m| (m.0 - 2500.0).abs() < 1.0).unwrap();
        assert_eq!(tail2.2, 80, "matched slider end keeps its 80% volume");
        assert!(tail2.3.iter().any(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Clap), .. })));
        // Slider 3 tail at 3125 ms -> shorter than min_beats (0.25 < 0.5), untouched at 60%
        let tail3 = moments.iter().find(|m| (m.0 - 3125.0).abs() < 1.0).unwrap();
        assert_eq!(tail3.2, 60, "slider shorter than min_beats is never muted");
    }

    #[test]
    fn slider_end_mute_divisors_filters_by_beat_snap() {
        let source = test_map("256,192,1000,1,0,0:0:0:0:");
        let len_quarter = 1.4 * 100.0 * 0.75; // 1000..1375 (1/4 beat)
        let len_whole = 1.4 * 100.0 * 1.0;    // 2000..2500 (1/1 beat)
        let target = test_map(&format!(
            "100,100,1000,2,0,L|300:100,1,{len_quarter},0|0,0:0|0:0,0:0:0:0:\r\n\
             100,100,2000,2,0,L|300:100,1,{len_whole},0|0,0:0|0:0,0:0:0:0:"
        ));
        let options = CopyOptions {
            slider_end_mute: SliderEndMuteOptions {
                enabled: true,
                min_beats: 0.5,
                mute_spinner_ends: false,
                only_unhit: true,
                divisors: vec![4],
            },
            ..CopyOptions::default()
        };
        let plan = run_plan(&source, &target, &options);
        assert_eq!(plan.muted, 1, "only the 1/4 slider end is muted; 1/1 end is untouched");
        let moments = moment_volumes(&plan.file);
        assert_eq!(moments.iter().find(|m| (m.0 - 1375.0).abs() < 1.0).unwrap().2, 5);
        assert_eq!(moments.iter().find(|m| (m.0 - 2500.0).abs() < 1.0).unwrap().2, 60);
    }

    #[test]
    fn slider_end_mute_spinner_ends_respects_toggle_and_preserves_hit_spinner_ends() {
        // Source hits spinner 2's end at 4000 ms (with finish at 90%), does NOT hit spinner 1's end at 2000 ms.
        let source = test_map("256,192,4000,1,4,0:0:0:90:");
        let target = test_map(concat!(
            "256,192,1000,8,0,2000,0:0:0:0:\r\n",
            "256,192,3000,8,0,4000,0:0:0:0:",
        ));

        // With mute_spinner_ends: false (default), neither spinner end is muted.
        let off = CopyOptions {
            slider_end_mute: SliderEndMuteOptions {
                enabled: true,
                min_beats: 0.5,
                mute_spinner_ends: false,
                only_unhit: true,
                divisors: Vec::new(),
            },
            ..CopyOptions::default()
        };
        let plan_off = run_plan(&source, &target, &off);
        assert_eq!(plan_off.muted, 0);
        let m_off = moment_volumes(&plan_off.file);
        assert_eq!(m_off.iter().find(|m| (m.0 - 2000.0).abs() < 1.0).unwrap().2, 60);

        // With mute_spinner_ends: true, un-hit spinner 1 end (2000 ms) is muted to 5%,
        // while hit spinner 2 end (4000 ms) keeps its matched finish at 90%.
        let on = CopyOptions {
            slider_end_mute: SliderEndMuteOptions {
                enabled: true,
                min_beats: 0.5,
                mute_spinner_ends: true,
                only_unhit: true,
                divisors: Vec::new(),
            },
            ..CopyOptions::default()
        };
        let plan_on = run_plan(&source, &target, &on);
        assert_eq!(plan_on.muted, 1);
        let m_on = moment_volumes(&plan_on.file);
        let spin1 = m_on.iter().find(|m| (m.0 - 2000.0).abs() < 1.0).unwrap();
        assert_eq!(spin1.2, 5);
        let spin2 = m_on.iter().find(|m| (m.0 - 4000.0).abs() < 1.0).unwrap();
        assert_eq!(spin2.2, 90);
        assert!(spin2.3.iter().any(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Finish), .. })));
    }

    #[test]
    fn fallback_sound_applies_only_to_unmatched_objects_and_clamps_volume() {
        let source_bytes = concat!(
            "osu file format v14\n",
            "[General]\nMode:0\n",
            "[TimingPoints]\n0,500,4,1,0,100,1,0\n",
            "[HitObjects]\n",
            "256,192,1000,1,8,3:3:0:85:\n",
        );
        let target_bytes = concat!(
            "osu file format v14\n",
            "[General]\nMode:0\n",
            "[TimingPoints]\n0,500,4,1,0,100,1,0\n",
            "[HitObjects]\n",
            "256,192,1000,1,0,0:0:0:90:\n",
            "256,192,2000,1,0,1:0:0:90:\n",
        );
        let source_file = OsuFile::read(source_bytes.as_bytes().to_vec()).file;
        let target_file = OsuFile::read(target_bytes.as_bytes().to_vec()).file;
        let Decompiled::Done(source_hits) = decompile(&source_file, &[], &|| false, &mut |_, _| {}) else {
            panic!("decompile source");
        };

        // With fallback_sound: Some(Soft Normal @ 60%), matched object at 1000ms keeps Drum Clap @ 85%,
        // while unmatched object at 2000ms gets Soft Normal @ 60%.
        let opts_with_fallback = CopyOptions {
            mode: CopyMode::OverwriteOnlyDefined,
            fallback_sound: Some(FallbackSound::new(SampleSet::Soft, Hitsound::Normal, 60)),
            ..CopyOptions::default()
        };
        let CopyOutcome::Targets(mut outcomes) = copy_hitsounds(
            &source_hits, &source_file, std::slice::from_ref(&target_file), &[], &opts_with_fallback, &|| false, &mut |_, _| {},
        ) else {
            panic!("copy outcome");
        };
        let TargetOutcome::Planned(plan) = outcomes.remove(0) else {
            panic!("target planned");
        };
        assert_eq!(plan.matched, 1);
        assert_eq!(plan.fallback_applied, 1);
        let Decompiled::Done(result_hits) = decompile(&plan.file, &[], &|| false, &mut |_, _| {}) else {
            panic!("decompile result");
        };
        let m0 = &result_hits.moments()[0];
        assert_eq!(m0.volume(), 85);
        assert!(m0.hits().any(|h| matches!(h, SampleIdentity::Standard { set: SampleSet::Drum, sound: Sound::Hit(Hitsound::Clap), .. })));
        let m1 = &result_hits.moments()[1];
        assert_eq!(m1.volume(), 60);
        let m1_hits: Vec<_> = m1.hits().cloned().collect();
        assert_eq!(m1_hits, vec![hitnormal(SampleSet::Soft, 0)]);

        // With fallback_sound: None, unmatched object at 2000ms stays untouched (Normal @ 90%).
        let opts_no_fallback = CopyOptions {
            mode: CopyMode::OverwriteOnlyDefined,
            fallback_sound: None,
            ..CopyOptions::default()
        };
        let CopyOutcome::Targets(mut outcomes) = copy_hitsounds(
            &source_hits, &source_file, std::slice::from_ref(&target_file), &[], &opts_no_fallback, &|| false, &mut |_, _| {},
        ) else {
            panic!("copy outcome");
        };
        let TargetOutcome::Planned(plan_none) = outcomes.remove(0) else {
            panic!("target planned");
        };
        assert_eq!(plan_none.fallback_applied, 0);
        let Decompiled::Done(result_none) = decompile(&plan_none.file, &[], &|| false, &mut |_, _| {}) else {
            panic!("decompile result_none");
        };
        let m1_none = &result_none.moments()[1];
        assert_eq!(m1_none.volume(), 90);
        let m1_none_hits: Vec<_> = m1_none.hits().cloned().collect();
        assert_eq!(m1_none_hits, vec![hitnormal(SampleSet::Normal, 0)]);

        // Volume clamps to 5..=100 (respecting dawsu!'s 5% floor) both via new() and serde.
        assert_eq!(FallbackSound::new(SampleSet::Drum, Hitsound::Normal, 0).volume, MINIMUM_PLAYED_VOLUME);
        assert_eq!(FallbackSound::new(SampleSet::Drum, Hitsound::Normal, 150).volume, 100);
        let low: FallbackSound = serde_json::from_str(r#"{"set":"soft","sound":"normal","volume":1}"#).unwrap();
        assert_eq!(low.volume, MINIMUM_PLAYED_VOLUME);
        let high: FallbackSound = serde_json::from_str(r#"{"set":"soft","sound":"normal","volume":200}"#).unwrap();
        assert_eq!(high.volume, 100);
    }
}
