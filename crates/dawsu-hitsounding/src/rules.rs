//! The hitsound rules (ADR 0036): one answer for what plays at every part of every object,
//! and the small answers beside it (played volume, encoding limits, the saved-volume floor).
//! Every rule follows lazer at the pin; the ported pieces are in `crate::lazer`.
//! The rules never read the disk: they are given the folder's sample file names as a list.
//! Follows osu!lazer (ppy/osu) at 131fe72a15581049401a588a13684938d4d0e0c7; copyright (c) ppy Pty Ltd <contact@ppy.sh>, MIT Licence (ADR 0027, THIRD-PARTY-NOTICES.md).

use crate::lazer::control_points::{offset_for, ControlPoints, SamplePointData, CONTROL_POINT_LENIENCY};
use crate::lazer::path_approximator::almost_equals_f64;
use crate::lazer::samples::{convert_sound_type, LegacySample, SampleBankInfo, HIT_NORMAL, HIT_WHISTLE, WHISTLE};
use crate::lazer::slider_events::{self, SliderEventType};
use crate::lazer::slider_path::{self, FIRST_LAZER_VERSION};
use crate::lazer::vector::Vector2;
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, Sound};
use dawsu_osu_format::number::{parse_double, parse_int, try_parse_int, MAX_PARSE_VALUE};
use dawsu_osu_format::{check_map_path, find_case_insensitive, FolderIndex, LineId, ObjectKind, OsuFile};

/// `LegacyDecoder.LATEST_VERSION`: a file with no version header counts as this.
const LATEST_VERSION: u32 = 14;
/// `OsuHitObject.BASE_SCORING_DISTANCE`.
const BASE_SCORING_DISTANCE: f64 = 100.0;

/// R5: played volume is never below 5% or above 100%.
pub const MINIMUM_PLAYED_VOLUME: i32 = 5;
/// R15 (a dawsu! policy): a moment a save changes is written at 5% or more.
pub const SAVED_VOLUME_FLOOR: i32 = 5;

/// What compile may write on one object (Responsibility 6): one hitnormal plus up to three
/// additions that share one addition set, one index and one volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodingLimits {
    pub max_additions: usize,
}

pub const ENCODING_LIMITS: EncodingLimits = EncodingLimits { max_additions: 3 };

/// The played volume for a stored volume (R5).
pub fn played_volume(volume: i32) -> i32 {
    volume.clamp(MINIMUM_PLAYED_VOLUME, 100)
}

/// The volume a save writes for a moment it changes (R15).
pub fn saved_volume(volume: i32) -> i32 {
    volume.max(SAVED_VOLUME_FLOOR)
}

/// Which sound plays for a sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plays {
    /// A file from the map folder, as the folder's listing names it.
    BeatmapFile(String),
    /// The skin's sample, looked up by these names (an osu!stable-style skin ignores the index).
    Skin { lookup_names: Vec<String> },
}

/// One sample of one part, as lazer resolves it, and what plays for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub lazer: LegacySample,
    pub lookup_names: Vec<String>,
    pub plays: Plays,
    pub played_volume: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    Head,
    Tick,
    Repeat,
    Tail,
}

/// A slider's head, tick, repeat or tail.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub kind: PartKind,
    pub time: f64,
    pub samples: Vec<Sample>,
}

/// A slider's timing values and its parts, as the osu! ruleset works them out.
#[derive(Debug, Clone, PartialEq)]
pub struct SliderAnswer {
    pub beat_length: f64,
    pub slider_velocity_multiplier: f64,
    pub velocity: f64,
    pub path_distance: f64,
    pub span_duration: f64,
    pub tick_distance: f64,
    pub tick_distance_multiplier: f64,
    pub generate_ticks: bool,
    pub repeat_count: usize,
    /// Head, ticks, repeats and tail, in time order. The tail plays no sample of its own.
    pub parts: Vec<Part>,
    /// The samples the slider plays at its end.
    pub tail_samples: Vec<Sample>,
    /// The body's sounds: the slider slide and, with a whistle, the slider whistle.
    pub sliding_samples: Vec<Sample>,
    /// The slider whistle the body plays while its whistle is on: the slider's own hitwhistle,
    /// from its addition bank, renamed (`Slider.UpdateNestedSamples`), whether or not it is on now.
    pub whistle_sample: Sample,
    /// When each node's samples were looked up: the decoder's own slider timing plus 5 ms.
    pub node_lookup_times: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Circle,
    Slider,
    Spinner,
}

/// What plays for one hit object.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectAnswer {
    pub line: LineId,
    pub kind: Kind,
    pub start_time: f64,
    pub end_time: Option<f64>,
    /// When the object's own samples were looked up (Responsibility 2).
    pub lookup_time: f64,
    /// The timing point those samples took their set, index and volume from.
    pub sample_point: Option<SamplePointData>,
    pub samples: Vec<Sample>,
    pub slider: Option<SliderAnswer>,
}

/// The one answer (Responsibility 1a).
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// Every object in lazer's order: sorted by start time, ties in file order.
    Objects(Vec<ObjectAnswer>),
    /// A taiko, catch or mania difficulty (Responsibility 1c).
    UnsupportedMode { mode: i32 },
}

/// What plays at every part of every object of `file`, given the names of the sample files in
/// its folder (as `dawsu_osu_format::list_folder` gives them).
pub fn answer(file: &OsuFile, folder_files: &[String]) -> Answer {
    let mode = file.general("Mode").and_then(|m| parse_int(&m).ok()).unwrap_or(0);
    if mode != 0 {
        return Answer::UnsupportedMode { mode };
    }
    let version = file.format_version().unwrap_or(LATEST_VERSION);
    let offset = offset_for(file.format_version());
    let cp = ControlPoints::from_file(file);
    let slider_multiplier = difficulty(file, "SliderMultiplier", 1.4).clamp(0.4, 3.6);
    let tick_rate = difficulty(file, "SliderTickRate", 1.0).clamp(0.5, 8.0);
    // Every sample of every object is looked up in the folder: index the listing once.
    let folder = FolderIndex::new(folder_files);

    let mut objects: Vec<ObjectAnswer> = file
        .hit_objects()
        .iter()
        .filter_map(|o| object_answer(o, version, offset, &cp, slider_multiplier, tick_rate, &folder))
        .collect();
    // LegacyBeatmapDecoder sorts with a stable OrderBy on start time.
    objects.sort_by(|a, b| a.start_time.total_cmp(&b.start_time));
    Answer::Objects(objects)
}

fn difficulty(file: &OsuFile, key: &str, default: f64) -> f64 {
    file.value("Difficulty", key).and_then(|v| parse_double(&v, MAX_PARSE_VALUE, false).ok()).unwrap_or(default)
}

fn resolve(samples: &[LegacySample], folder: &FolderIndex) -> Vec<Sample> {
    samples.iter().map(|s| resolve_in(s, folder)).collect()
}

/// Which file plays (R11 to R13): from index 1, the map folder by the beatmap skin's lookup
/// names, each with R12's extensions; otherwise, or when nothing matches, the skin.
/// One sample alone searches the listing as it is; `resolve_in` takes an index for many.
pub(crate) fn resolve_one(sample: &LegacySample, folder_files: &[String]) -> Sample {
    resolve_with(sample, |name| find_case_insensitive(folder_files, name))
}

fn resolve_in(sample: &LegacySample, folder: &FolderIndex) -> Sample {
    resolve_with(sample, |name| folder.find(name))
}

fn resolve_with<'a>(sample: &LegacySample, find: impl Fn(&str) -> Option<&'a str>) -> Sample {
    let plays = beatmap_file(sample, find)
        .map(Plays::BeatmapFile)
        .unwrap_or_else(|| Plays::Skin { lookup_names: sample.legacy_lookup_names(false) });
    Sample { lazer: sample.clone(), lookup_names: sample.lookup_names(), plays, played_volume: played_volume(sample.volume) }
}

fn beatmap_file<'a>(sample: &LegacySample, find: impl Fn(&str) -> Option<&'a str>) -> Option<String> {
    // LegacyBeatmapSkin.GetSample: beatmap samples only when the sample uses them (index 1 or more).
    if !sample.use_beatmap_samples {
        return None;
    }
    for name in sample.legacy_lookup_names(true) {
        // R12: the name as given, then with .wav, .mp3 and .ogg added.
        for ext in ["", ".wav", ".mp3", ".ogg"] {
            let candidate = format!("{name}{ext}");
            if check_map_path(&candidate).is_err() {
                continue;
            }
            if let Some(found) = find(&candidate) {
                return Some(found.to_string());
            }
        }
    }
    None
}

fn applied(samples: &[LegacySample], point: Option<&SamplePointData>) -> Vec<LegacySample> {
    samples.iter().map(|s| s.apply(point)).collect()
}

fn object_answer(o: &dawsu_osu_format::HitObject, version: u32, offset: f64, cp: &ControlPoints, slider_multiplier: f64, tick_rate: f64, folder: &FolderIndex) -> Option<ObjectAnswer> {
    let start_time = o.time() + offset;
    match o.kind() {
        ObjectKind::Circle => {
            let bank = SampleBankInfo::default().read(o.hit_sample(), false);
            let lookup_time = start_time + CONTROL_POINT_LENIENCY;
            let point = cp.sample_point_at(lookup_time);
            let samples = applied(&convert_sound_type(o.hitsound(), &bank), point);
            Some(ObjectAnswer { line: o.line(), kind: Kind::Circle, start_time, end_time: None, lookup_time, sample_point: point.cloned(), samples: resolve(&samples, folder), slider: None })
        }
        ObjectKind::Spinner { end } | ObjectKind::Hold { end } => {
            // A hold in an osu! map converts to a spinner, as anything with a duration does.
            let duration = (end + offset - start_time).max(0.0);
            let end_time = start_time + duration;
            let bank = SampleBankInfo::default().read(o.hit_sample(), false);
            let lookup_time = end_time + CONTROL_POINT_LENIENCY;
            let point = cp.sample_point_at(lookup_time);
            let samples = applied(&convert_sound_type(o.hitsound(), &bank), point);
            Some(ObjectAnswer {
                line: o.line(),
                kind: Kind::Spinner,
                start_time,
                end_time: Some(end_time),
                lookup_time,
                sample_point: point.cloned(),
                samples: resolve(&samples, folder),
                slider: None,
            })
        }
        ObjectKind::Slider(fields) => slider_answer(o, fields, version, start_time, cp, slider_multiplier, tick_rate, folder),
    }
}

/// `LegacyRulesetExtensions.GetPrecisionAdjustedBeatLength` for the osu! ruleset.
fn precision_adjusted_beat_length(beat_length: f64, slider_velocity_multiplier: f64) -> f64 {
    let as_beat_length = -100.0 / slider_velocity_multiplier;
    let multiplier = if as_beat_length < 0.0 { f64::from((-as_beat_length as f32).clamp(10.0, 1000.0)) / 100.0 } else { 1.0 };
    beat_length * multiplier
}

#[allow(clippy::too_many_arguments)]
fn slider_answer(
    o: &dawsu_osu_format::HitObject,
    fields: &dawsu_osu_format::SliderFields,
    version: u32,
    start_time: f64,
    cp: &ControlPoints,
    slider_multiplier: f64,
    tick_rate: f64,
    folder: &FolderIndex,
) -> Option<ObjectAnswer> {
    // ConvertHitObjectParser: the position is clamped to 0-512 and, before v128, truncated.
    let (x, y) = (o.x().clamp(0.0, 512.0), o.y().clamp(0.0, 512.0));
    let position = if version >= FIRST_LAZER_VERSION { Vector2::new(x, y) } else { Vector2::new(x.trunc(), y.trunc()) };
    let control_points = slider_path::parse_curve(&fields.curve, position, version).ok()?;

    let mut repeat_count = (fields.slides - 1).max(0) as usize;
    let length = fields.length.map(|l| l.max(0.0)).filter(|l| *l != 0.0);

    let bank = SampleBankInfo::default().read(o.hit_sample(), true);
    let nodes = repeat_count + 2;
    let mut node_banks = vec![bank.clone(); nodes];
    if let Some(sets) = &fields.edge_sets {
        // readCustomSampleBanks(sets[i], nodeBankInfos[i]) reads the whole entry, not only its banks.
        for (i, set) in sets.iter().enumerate().take(nodes) {
            if set.is_some() {
                node_banks[i] = node_banks[i].clone().read(set.as_ref(), false);
            }
        }
    }
    let mut node_sounds = vec![o.hitsound(); nodes];
    if let Some(sounds) = &fields.edge_sounds {
        for (i, s) in sounds.iter().enumerate().take(nodes) {
            node_sounds[i] = try_parse_int(s);
        }
    }
    let mut node_samples: Vec<Vec<LegacySample>> = (0..nodes).map(|i| convert_sound_type(node_sounds[i], &node_banks[i])).collect();
    let own_samples = convert_sound_type(o.hitsound(), &bank);

    // The decoder's own slider (ConvertSlider): no Catmull optimisation, plain beat length.
    let convert_distance = slider_path::path_length(&control_points, length, false).distance;
    if almost_equals_f64(convert_distance, 0.0) {
        repeat_count = 0;
        node_samples = vec![node_samples[0].clone(), node_samples[node_samples.len() - 1].clone()];
    }
    let span_count = repeat_count + 1;
    let dp = cp.difficulty_point_at(start_time);
    let timing = cp.timing_point_at(start_time);
    let convert_velocity = BASE_SCORING_DISTANCE * slider_multiplier * dp.slider_velocity / timing.beat_length;
    let convert_duration = span_count as f64 * convert_distance / convert_velocity;

    // LegacyBeatmapDecoder.applySamples: the slider's own samples at start + 6 ms, each node at its time + 5 ms.
    let lookup_time = start_time + CONTROL_POINT_LENIENCY + 1.0;
    let point = cp.sample_point_at(lookup_time);
    let samples = applied(&own_samples, point);
    let mut node_lookup_times = Vec::with_capacity(node_samples.len());
    for (i, ns) in node_samples.iter_mut().enumerate() {
        let t = start_time + i as f64 * convert_duration / span_count as f64 + CONTROL_POINT_LENIENCY;
        node_lookup_times.push(t);
        *ns = applied(ns, cp.sample_point_at(t));
    }

    // The osu! ruleset's slider (Slider.ApplyDefaultsToSelf).
    let path_distance = slider_path::path_length(&control_points, length, true).distance;
    let tick_distance_multiplier = if version < 8 { 1.0 / dp.slider_velocity } else { 1.0 };
    let velocity = BASE_SCORING_DISTANCE * slider_multiplier / precision_adjusted_beat_length(timing.beat_length, dp.slider_velocity);
    let scoring_distance = velocity * timing.beat_length;
    let tick_distance = if dp.generate_ticks { scoring_distance / tick_rate * tick_distance_multiplier } else { f64::INFINITY };
    let end_time = start_time + span_count as f64 * path_distance / velocity;
    let span_duration = (end_time - start_time) / span_count as f64;

    // Slider.UpdateNestedSamples.
    while node_samples.len() < repeat_count + 2 {
        node_samples.push(samples.iter().map(LegacySample::copied).collect());
    }
    let node = |i: usize| -> Vec<LegacySample> { node_samples.get(i).cloned().unwrap_or_else(|| samples.clone()) };
    let tick_sample = samples.iter().find(|s| s.name == HIT_NORMAL).or_else(|| samples.first()).map(|s| s.renamed("slidertick"));

    let mut parts = Vec::new();
    for e in slider_events::generate(start_time, span_duration, velocity, tick_distance, path_distance, span_count) {
        match e.kind {
            SliderEventType::Head => parts.push(Part { kind: PartKind::Head, time: e.time, samples: resolve(&node(0), folder) }),
            SliderEventType::Tick => {
                parts.push(Part { kind: PartKind::Tick, time: e.time, samples: resolve(tick_sample.as_slice(), folder) });
            }
            SliderEventType::Repeat => parts.push(Part {
                kind: PartKind::Repeat,
                time: start_time + (e.span_index + 1) as f64 * span_duration,
                samples: resolve(&node(e.span_index + 1), folder),
            }),
            SliderEventType::Tail => parts.push(Part { kind: PartKind::Tail, time: e.time, samples: Vec::new() }),
        }
    }
    // HitObject.ApplyDefaults sorts nested objects by start time.
    parts.sort_by(|a, b| a.time.total_cmp(&b.time));

    let mut sliding = Vec::new();
    if let Some(n) = samples.iter().find(|s| s.name == HIT_NORMAL) {
        sliding.push(n.renamed("sliderslide"));
    }
    if let Some(w) = samples.iter().find(|s| s.name == HIT_WHISTLE) {
        sliding.push(w.renamed("sliderwhistle"));
    }
    // The same slider with its whistle flag on: the whistle sample it would play.
    let with_whistle = applied(&convert_sound_type(WHISTLE, &bank), point);
    let whistle_sample = with_whistle.iter().find(|s| s.name == HIT_WHISTLE).map(|w| w.renamed("sliderwhistle"));

    Some(ObjectAnswer {
        line: o.line(),
        kind: Kind::Slider,
        start_time,
        end_time: Some(end_time),
        lookup_time,
        sample_point: point.cloned(),
        samples: resolve(&samples, folder),
        slider: Some(SliderAnswer {
            beat_length: timing.beat_length,
            slider_velocity_multiplier: dp.slider_velocity,
            velocity,
            path_distance,
            span_duration,
            tick_distance,
            tick_distance_multiplier,
            generate_ticks: dp.generate_ticks,
            repeat_count,
            parts,
            tail_samples: resolve(&node(repeat_count + 1), folder),
            sliding_samples: resolve(&sliding, folder),
            // A slider's own hit sample is read for its banks only, so no file replaces the whistle.
            whistle_sample: resolve_in(&whistle_sample.expect("convertSoundType adds the hitwhistle for the whistle flag"), folder),
            node_lookup_times,
        }),
    })
}

// ---------------------------------------------------------------- sample file names (Responsibility 4)

pub(crate) fn set_word(set: SampleSet) -> &'static str {
    match set {
        SampleSet::Normal => "normal",
        SampleSet::Soft => "soft",
        SampleSet::Drum => "drum",
    }
}

pub(crate) fn sound_word(sound: Sound) -> &'static str {
    match sound {
        Sound::Hit(Hitsound::Normal) => "hitnormal",
        Sound::Hit(Hitsound::Whistle) => "hitwhistle",
        Sound::Hit(Hitsound::Finish) => "hitfinish",
        Sound::Hit(Hitsound::Clap) => "hitclap",
        Sound::Slider(SliderSound::Slide) => "sliderslide",
        Sound::Slider(SliderSound::Whistle) => "sliderwhistle",
        Sound::Slider(SliderSound::Tick) => "slidertick",
    }
}

/// `<set>-hit<sound>.wav` or `<set>-slider<sound>.wav` for index 1, with the index before the
/// extension for index 2 or more. Index 0 plays the skin's sound, which has no file in the map.
pub fn sample_file_name(set: SampleSet, sound: Sound, index: CustomIndex) -> Option<String> {
    match index.get() {
        0 => None,
        1 => Some(format!("{}-{}.wav", set_word(set), sound_word(sound))),
        n => Some(format!("{}-{}{n}.wav", set_word(set), sound_word(sound))),
    }
}

/// Reads a sample file name back into its identity; `None` for a name that does not follow the
/// pattern. The extension may be `.wav`, `.mp3` or `.ogg` (R12), in any case.
pub fn read_sample_file_name(name: &str) -> Option<SampleIdentity> {
    let lower = name.to_lowercase();
    let stem = [".wav", ".mp3", ".ogg"].iter().find_map(|ext| lower.strip_suffix(ext))?;
    let (set_text, rest) = stem.split_once('-')?;
    let set = match set_text {
        "normal" => SampleSet::Normal,
        "soft" => SampleSet::Soft,
        "drum" => SampleSet::Drum,
        _ => return None,
    };
    let sounds = [
        Sound::Hit(Hitsound::Normal),
        Sound::Hit(Hitsound::Whistle),
        Sound::Hit(Hitsound::Finish),
        Sound::Hit(Hitsound::Clap),
        Sound::Slider(SliderSound::Slide),
        Sound::Slider(SliderSound::Whistle),
        Sound::Slider(SliderSound::Tick),
    ];
    for sound in sounds {
        if let Some(digits) = rest.strip_prefix(sound_word(sound)) {
            let index = if digits.is_empty() {
                1
            } else {
                let n: u32 = digits.parse().ok()?;
                if n < 2 || digits.starts_with('0') {
                    return None;
                }
                n
            };
            return Some(SampleIdentity::Standard { set, sound, index: CustomIndex::new(index) });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_file_names_round_trip() {
        for (set, sound, index, name) in [
            (SampleSet::Soft, Sound::Hit(Hitsound::Clap), 1, "soft-hitclap.wav"),
            (SampleSet::Drum, Sound::Hit(Hitsound::Finish), 3, "drum-hitfinish3.wav"),
            (SampleSet::Normal, Sound::Slider(SliderSound::Tick), 2, "normal-slidertick2.wav"),
        ] {
            assert_eq!(sample_file_name(set, sound, CustomIndex::new(index)).as_deref(), Some(name));
            assert_eq!(read_sample_file_name(name), Some(SampleIdentity::Standard { set, sound, index: CustomIndex::new(index) }));
        }
        assert_eq!(sample_file_name(SampleSet::Soft, Sound::Hit(Hitsound::Clap), CustomIndex::new(0)), None);
        assert_eq!(read_sample_file_name("hitclap.wav"), None, "a bare name has no set");
        assert_eq!(read_sample_file_name("soft-hitclap1.wav"), None, "index 1 has no number");
        assert!(read_sample_file_name("SOFT-HITCLAP2.OGG").is_some());
    }

    #[test]
    fn played_and_saved_volumes() {
        assert_eq!(played_volume(0), 5);
        assert_eq!(played_volume(150), 100);
        assert_eq!(saved_volume(2), 5);
        assert_eq!(saved_volume(40), 40);
    }

    fn sample(name: &str, bank: &str, index: i32) -> LegacySample {
        LegacySample::legacy(name, Some(bank), 50, false, index, false)
    }

    #[test]
    fn index_0_plays_the_skin_even_when_the_file_exists() {
        let files = vec!["soft-hitclap.wav".to_string()];
        assert!(matches!(resolve_one(&sample("hitclap", "soft", 0), &files).plays, Plays::Skin { .. }));
    }

    #[test]
    fn index_1_finds_the_file_then_the_bare_name() {
        let files = vec!["Soft-HitClap.WAV".to_string()];
        assert_eq!(resolve_one(&sample("hitclap", "soft", 1), &files).plays, Plays::BeatmapFile("Soft-HitClap.WAV".into()));
        let bare = vec!["hitclap.ogg".to_string()];
        assert_eq!(resolve_one(&sample("hitclap", "soft", 1), &bare).plays, Plays::BeatmapFile("hitclap.ogg".into()));
    }

    #[test]
    fn index_2_never_plays_the_index_1_file() {
        let files = vec!["soft-hitclap.wav".to_string()];
        assert!(matches!(resolve_one(&sample("hitclap", "soft", 2), &files).plays, Plays::Skin { .. }));
        let files = vec!["soft-hitclap.wav".to_string(), "soft-hitclap2.mp3".to_string()];
        assert_eq!(resolve_one(&sample("hitclap", "soft", 2), &files).plays, Plays::BeatmapFile("soft-hitclap2.mp3".into()));
        let bare = vec!["hitclap.wav".to_string()];
        assert_eq!(resolve_one(&sample("hitclap", "soft", 2), &bare).plays, Plays::BeatmapFile("hitclap.wav".into()));
    }

    #[test]
    fn the_extension_order_is_wav_mp3_ogg() {
        let files = vec!["soft-hitclap.ogg".to_string(), "soft-hitclap.mp3".to_string(), "soft-hitclap.wav".to_string()];
        assert_eq!(resolve_one(&sample("hitclap", "soft", 1), &files).plays, Plays::BeatmapFile("soft-hitclap.wav".into()));
        let files = vec!["soft-hitclap.ogg".to_string(), "soft-hitclap.mp3".to_string()];
        assert_eq!(resolve_one(&sample("hitclap", "soft", 1), &files).plays, Plays::BeatmapFile("soft-hitclap.mp3".into()));
    }

    #[test]
    fn a_custom_file_plays_from_the_folder() {
        let files = vec!["dir/sub sample.ogg".to_string()];
        let s = LegacySample::file("dir/sub sample.ogg", 40);
        assert_eq!(resolve_one(&s, &files).plays, Plays::BeatmapFile("dir/sub sample.ogg".into()));
    }

    #[test]
    fn other_modes_are_unsupported() {
        let read = OsuFile::read(b"osu file format v14\r\n\r\n[General]\r\nMode: 3\r\n".to_vec());
        assert_eq!(answer(&read.file, &[]), Answer::UnsupportedMode { mode: 3 });
    }
}
