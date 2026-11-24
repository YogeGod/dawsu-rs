//! Read-back verification (Wave 1.2): re-parses compiled `.osu` bytes in memory and
//! decompiles them to confirm every hit moment and slider body matches the intended
//! hitsound state before any bytes are committed to disk.

use crate::decompile::{decompile, Decompiled};
use crate::model::{Hitsounds, Moment, ObjectId, Place, SliderBody};
use crate::rules::saved_volume;
use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, SliderSound, Sound};
use dawsu_osu_format::OsuFile;
use std::collections::{BTreeSet, HashMap};
use std::fmt;

const TIME_EPSILON_MS: f64 = 1e-6;

/// A mismatch detected when re-reading compiled `.osu` bytes in memory before writing to disk.
#[derive(Debug, Clone, PartialEq)]
pub struct VerificationMismatch {
    /// Timestamp of the first mismatched moment or slider body, in milliseconds.
    pub time_ms: Option<f64>,
    /// Plain-English description of expected vs actual state.
    pub detail: String,
}

impl fmt::Display for VerificationMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.time_ms {
            Some(ms) => write!(
                f,
                "Save verification stopped before writing: mismatch at {ms:.0} ms ({}).",
                self.detail
            ),
            None => write!(
                f,
                "Save verification stopped before writing: {}.",
                self.detail
            ),
        }
    }
}

impl std::error::Error for VerificationMismatch {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaceKind {
    Circle,
    SliderNode(usize),
    SpinnerEnd,
    NoObject,
}

fn place_kind(place: Place) -> PlaceKind {
    match place {
        Place::Circle(_) => PlaceKind::Circle,
        Place::SliderNode { node, .. } => PlaceKind::SliderNode(node),
        Place::SpinnerEnd(_) => PlaceKind::SpinnerEnd,
        Place::NoObject => PlaceKind::NoObject,
    }
}

fn place_label(place: Place) -> String {
    match place {
        Place::Circle(_) => "a hit circle".into(),
        Place::SliderNode { node: 0, .. } => "a slider head".into(),
        Place::SliderNode { node, .. } => format!("slider node {node}"),
        Place::SpinnerEnd(_) => "a spinner end".into(),
        Place::NoObject => "a loose hit".into(),
    }
}

fn format_sample(sample: &SampleIdentity) -> String {
    match sample {
        SampleIdentity::File(name) => name.clone(),
        SampleIdentity::Standard { set, sound, index } => {
            let set_name = match set {
                SampleSet::Normal => "normal",
                SampleSet::Soft => "soft",
                SampleSet::Drum => "drum",
            };
            let sound_name = match sound {
                Sound::Hit(Hitsound::Normal) => "hitnormal",
                Sound::Hit(Hitsound::Whistle) => "hitwhistle",
                Sound::Hit(Hitsound::Finish) => "hitfinish",
                Sound::Hit(Hitsound::Clap) => "hitclap",
                Sound::Slider(SliderSound::Slide) => "sliderslide",
                Sound::Slider(SliderSound::Whistle) => "sliderwhistle",
                Sound::Slider(SliderSound::Tick) => "slidertick",
            };
            let idx = index.get();
            if idx == 0 {
                format!("{set_name}-{sound_name}")
            } else {
                format!("{set_name}-{sound_name}{idx}")
            }
        }
    }
}

fn format_samples(samples: &BTreeSet<&SampleIdentity>) -> String {
    if samples.is_empty() {
        return "none".into();
    }
    samples.iter().map(|s| format_sample(s)).collect::<Vec<_>>().join(", ")
}

fn expected_moment_volume(moment: &Moment, body_volume_overrides: &HashMap<ObjectId, i32>) -> i32 {
    if moment.is_changed() {
        let raw = moment
            .hits()
            .next()
            .map_or_else(|| moment.volume(), |lane| moment.hit_volume(lane));
        saved_volume(raw)
    } else if let Place::SliderNode { object, node: 0 } = moment.place()
        && let Some(&vol) = body_volume_overrides.get(&object)
    {
        vol
    } else {
        moment.volume()
    }
}

fn expected_body_volume(body: &SliderBody) -> i32 {
    if body.volume_changed() {
        saved_volume(body.volume())
    } else {
        body.volume()
    }
}

/// Re-parses `compiled_bytes` in memory with `dawsu-osu-format` and decompiles it,
/// verifying that every non-loose hit moment and every slider body reproduces the
/// intended hitsounds (timestamps, sample identities, and volumes).
///
/// Accounts for osu!'s documented encoding rules (such as the 5% saved volume floor
/// for edited moments and bodies) so valid edits never false-positive.
pub fn verify_compiled_osu(
    compiled_bytes: &[u8],
    expected: &Hitsounds,
    folder_files: &[String],
) -> Result<(), VerificationMismatch> {
    let read = OsuFile::read(compiled_bytes.to_vec());
    if read.file.format_version().is_none() {
        return Err(VerificationMismatch {
            time_ms: None,
            detail: "compiled output is missing a valid osu! file format header".into(),
        });
    }
    let actual = match decompile(&read.file, folder_files, &|| false, &mut |_, _| {}) {
        Decompiled::Done(hits) => hits,
        Decompiled::UnsupportedMode { mode } => {
            return Err(VerificationMismatch {
                time_ms: None,
                detail: format!("compiled output parsed as unsupported game mode {mode}"),
            });
        }
        Decompiled::Cancelled => unreachable!("read-back verification does not cancel"),
    };

    let body_volume_overrides: HashMap<ObjectId, i32> = expected
        .bodies()
        .iter()
        .filter(|b| b.volume_changed())
        .map(|b| (b.object(), saved_volume(b.volume())))
        .collect();

    let expected_moments: Vec<&Moment> = expected
        .moments()
        .iter()
        .filter(|m| m.place() != Place::NoObject)
        .collect();
    let actual_moments = actual.moments();

    for (exp, act) in expected_moments.iter().zip(actual_moments.iter()) {
        if (exp.time().ms() - act.time().ms()).abs() > TIME_EPSILON_MS {
            return Err(VerificationMismatch {
                time_ms: Some(exp.time().ms()),
                detail: format!(
                    "expected {} at {:.0} ms, but re-reading found {} at {:.0} ms",
                    place_label(exp.place()),
                    exp.time().ms(),
                    place_label(act.place()),
                    act.time().ms(),
                ),
            });
        }
        if place_kind(exp.place()) != place_kind(act.place()) {
            return Err(VerificationMismatch {
                time_ms: Some(exp.time().ms()),
                detail: format!(
                    "expected {}, but re-reading found {}",
                    place_label(exp.place()),
                    place_label(act.place()),
                ),
            });
        }
        let exp_hits: BTreeSet<&SampleIdentity> = exp.hits().collect();
        let act_hits: BTreeSet<&SampleIdentity> = act.hits().collect();
        if exp_hits != act_hits {
            return Err(VerificationMismatch {
                time_ms: Some(exp.time().ms()),
                detail: format!(
                    "expected samples [{}], but re-reading produced [{}]",
                    format_samples(&exp_hits),
                    format_samples(&act_hits),
                ),
            });
        }
        let exp_vol = expected_moment_volume(exp, &body_volume_overrides);
        let act_vol = act.volume();
        if exp_vol != act_vol {
            return Err(VerificationMismatch {
                time_ms: Some(exp.time().ms()),
                detail: format!(
                    "expected volume {exp_vol}%, but re-reading produced {act_vol}%",
                ),
            });
        }
    }

    if expected_moments.len() != actual_moments.len() {
        let time_ms = expected_moments
            .get(actual_moments.len())
            .copied()
            .or_else(|| actual_moments.get(expected_moments.len()))
            .map(|m| m.time().ms());
        return Err(VerificationMismatch {
            time_ms,
            detail: format!(
                "expected {} hit moments, but re-reading found {}",
                expected_moments.len(),
                actual_moments.len(),
            ),
        });
    }

    let expected_bodies = expected.bodies();
    let actual_bodies = actual.bodies();
    for (exp, act) in expected_bodies.iter().zip(actual_bodies.iter()) {
        if (exp.start().ms() - act.start().ms()).abs() > TIME_EPSILON_MS
            || (exp.end().ms() - act.end().ms()).abs() > TIME_EPSILON_MS
        {
            return Err(VerificationMismatch {
                time_ms: Some(exp.start().ms()),
                detail: format!(
                    "expected slider body from {:.0} ms to {:.0} ms, but re-reading found {:.0} ms to {:.0} ms",
                    exp.start().ms(),
                    exp.end().ms(),
                    act.start().ms(),
                    act.end().ms(),
                ),
            });
        }
        if exp.slide() != act.slide() {
            return Err(VerificationMismatch {
                time_ms: Some(exp.start().ms()),
                detail: format!(
                    "expected slider slide {}, but re-reading produced {}",
                    format_sample(exp.slide()),
                    format_sample(act.slide()),
                ),
            });
        }
        if exp.whistle() != act.whistle() {
            return Err(VerificationMismatch {
                time_ms: Some(exp.start().ms()),
                detail: format!(
                    "expected slider whistle {}, but re-reading produced {}",
                    if exp.whistle() { "on" } else { "off" },
                    if act.whistle() { "on" } else { "off" },
                ),
            });
        }
        if exp.whistle_lane() != act.whistle_lane() {
            return Err(VerificationMismatch {
                time_ms: Some(exp.start().ms()),
                detail: format!(
                    "expected slider whistle sound {}, but re-reading produced {}",
                    format_sample(&exp.whistle_lane()),
                    format_sample(&act.whistle_lane()),
                ),
            });
        }
        let exp_vol = expected_body_volume(exp);
        let act_vol = act.volume();
        if exp_vol != act_vol {
            return Err(VerificationMismatch {
                time_ms: Some(exp.start().ms()),
                detail: format!(
                    "expected slider body volume {exp_vol}%, but re-reading produced {act_vol}%",
                ),
            });
        }
    }

    if expected_bodies.len() != actual_bodies.len() {
        let time_ms = expected_bodies
            .get(actual_bodies.len())
            .or_else(|| actual_bodies.get(expected_bodies.len()))
            .map(|b| b.start().ms());
        return Err(VerificationMismatch {
            time_ms,
            detail: format!(
                "expected {} slider bodies, but re-reading found {}",
                expected_bodies.len(),
                actual_bodies.len(),
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{compile, Compiled, Placement};
    use crate::model::MomentId;
    use dawsu_foundation::CustomIndex;

    const SAMPLE_MAP: &str = "\
osu file format v14\r\n\
\r\n\
[General]\r\n\
SampleSet: Normal\r\n\
\r\n\
[Difficulty]\r\n\
SliderMultiplier:1.4\r\n\
SliderTickRate:1\r\n\
\r\n\
[TimingPoints]\r\n\
0,500,4,1,0,70,1,0\r\n\
\r\n\
[HitObjects]\r\n\
256,192,1000,1,0,0:0:0:0:\r\n\
100,100,2000,2,0,L|200:100,1,140\r\n";

    fn skin(set: SampleSet, sound: Hitsound) -> SampleIdentity {
        SampleIdentity::Standard {
            set,
            sound: Sound::Hit(sound),
            index: CustomIndex::new(0),
        }
    }

    fn decompile_ok(file: &OsuFile, folder: &[String]) -> Hitsounds {
        match decompile(file, folder, &|| false, &mut |_, _| {}) {
            Decompiled::Done(h) => h,
            other => panic!("unexpected decompile outcome: {other:?}"),
        }
    }

    fn compile_ok(file: &OsuFile, hits: &Hitsounds, placement: Placement) -> OsuFile {
        match compile(file, hits, placement, &|| false, &mut |_, _| {}) {
            Compiled::Done { file, problems } => {
                assert!(problems.is_empty(), "unexpected compile problems: {problems:?}");
                file
            }
            Compiled::Cancelled => panic!("compile cancelled"),
        }
    }

    #[test]
    fn valid_compiled_osu_passes_verification() {
        let base = OsuFile::read(SAMPLE_MAP.as_bytes().to_vec()).file;
        let mut hits = decompile_ok(&base, &[]);
        hits.add_hit(MomentId(0), skin(SampleSet::Soft, Hitsound::Clap));
        hits.set_volume(MomentId(0), 55);
        hits.set_body_set(0, SampleSet::Drum);
        hits.set_body_whistle(0, true);
        let compiled = compile_ok(&base, &hits, Placement::GreenLines);
        assert_eq!(verify_compiled_osu(&compiled.to_bytes(), &hits, &[]), Ok(()));
    }

    #[test]
    fn valid_edit_below_five_percent_respects_saved_volume_floor() {
        let base = OsuFile::read(SAMPLE_MAP.as_bytes().to_vec()).file;
        let mut hits = decompile_ok(&base, &[]);
        hits.set_volume(MomentId(0), 2);
        let compiled = compile_ok(&base, &hits, Placement::GreenLines);
        assert_eq!(verify_compiled_osu(&compiled.to_bytes(), &hits, &[]), Ok(()));
    }

    #[test]
    fn tampered_hitsound_sample_is_caught_with_timestamp() {
        let base = OsuFile::read(SAMPLE_MAP.as_bytes().to_vec()).file;
        let mut hits = decompile_ok(&base, &[]);
        hits.add_hit(MomentId(0), skin(SampleSet::Normal, Hitsound::Clap));
        let compiled = compile_ok(&base, &hits, Placement::GreenLines);
        let text = String::from_utf8(compiled.to_bytes()).unwrap();
        // Tamper the circle at 1000 ms so its clap bit (8) becomes finish (4).
        let tampered = text.replace("256,192,1000,1,8,", "256,192,1000,1,4,");
        assert_ne!(text, tampered);

        let err = verify_compiled_osu(tampered.as_bytes(), &hits, &[]).unwrap_err();
        assert_eq!(err.time_ms, Some(1000.0));
        let msg = err.to_string();
        assert!(msg.contains("1000 ms"), "error should mention timestamp: {msg}");
        assert!(msg.contains("normal-hitclap"), "error should mention expected sample: {msg}");
        assert!(msg.contains("normal-hitfinish"), "error should mention actual sample: {msg}");
    }

    #[test]
    fn tampered_volume_is_caught_with_timestamp() {
        let base = OsuFile::read(SAMPLE_MAP.as_bytes().to_vec()).file;
        let mut hits = decompile_ok(&base, &[]);
        hits.set_volume(MomentId(0), 55);
        let compiled = compile_ok(&base, &hits, Placement::GreenLines);
        let text = String::from_utf8(compiled.to_bytes()).unwrap();
        let tampered = text.replace("0:0:0:55:", "0:0:0:35:");
        assert_ne!(text, tampered);

        let err = verify_compiled_osu(tampered.as_bytes(), &hits, &[]).unwrap_err();
        assert_eq!(err.time_ms, Some(1000.0));
        let msg = err.to_string();
        assert!(msg.contains("1000 ms"), "error should mention timestamp: {msg}");
        assert!(msg.contains("55%"), "error should mention expected volume: {msg}");
        assert!(msg.contains("35%"), "error should mention actual volume: {msg}");
    }

    #[test]
    fn missing_osu_header_is_caught() {
        let base = OsuFile::read(SAMPLE_MAP.as_bytes().to_vec()).file;
        let hits = decompile_ok(&base, &[]);
        let err = verify_compiled_osu(b"not an osu file", &hits, &[]).unwrap_err();
        assert_eq!(err.time_ms, None);
        assert!(err.to_string().contains("format header"));
    }
}
