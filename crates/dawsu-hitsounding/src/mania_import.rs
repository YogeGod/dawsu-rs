//! Mania (Mode 3) hitsound difficulty parser and importer.
//!
//! Many osu! mappers author hitsounds in an osu!mania difficulty where each column or hit
//! object carries a custom sample filename or a standard hitsound + volume. This module
//! parses a Mode 3 `.osu` file using osu!lazer's control-point and sample-bank rules and
//! emits a sorted, deduplicated list of [`ImportedManiaHit`] values ready to apply onto an
//! open difficulty's timeline.

use crate::lazer::control_points::{offset_for, ControlPoints, CONTROL_POINT_LENIENCY};
use crate::lazer::samples::{convert_sound_type, LegacySample, SampleBankInfo};
use crate::rules::played_volume;
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, Sound};
use dawsu_osu_format::number::parse_int;
use dawsu_osu_format::OsuFile;

/// One resolved hitsound hit imported from an osu!mania (Mode 3) difficulty.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedManiaHit {
    pub time_ms: f64,
    pub lane: SampleIdentity,
    pub volume: i32,
}

/// Parses raw `.osu` bytes as a Mode 3 (osu!mania) hitsound difficulty.
pub fn parse_mania_hitsounds_bytes(bytes: &[u8]) -> Result<Vec<ImportedManiaHit>, String> {
    let read = OsuFile::read(bytes.to_vec());
    parse_mania_hitsounds(&read.file)
}

/// Parses an [`OsuFile`] as a Mode 3 (osu!mania) hitsound difficulty and returns
/// deduplicated hits sorted by time.
///
/// Refuses non-mania difficulties with a clear plain-English error.
pub fn parse_mania_hitsounds(file: &OsuFile) -> Result<Vec<ImportedManiaHit>, String> {
    let mode = file
        .general("Mode")
        .and_then(|m| parse_int(&m).ok())
        .unwrap_or(0);
    if mode != 3 {
        let mode_name = match mode {
            0 => "osu!standard",
            1 => "osu!taiko",
            2 => "osu!catch",
            _ => "another game mode",
        };
        return Err(format!(
            "That file is an {mode_name} difficulty (Mode: {mode}), not an osu!mania (Mode: 3) hitsound difficulty."
        ));
    }

    let offset = offset_for(file.format_version());
    let cp = ControlPoints::from_file(file);

    let mut raw: Vec<ImportedManiaHit> = Vec::new();
    for obj in file.hit_objects() {
        let time_ms = obj.time() + offset;
        if !time_ms.is_finite() {
            continue;
        }
        let bank = SampleBankInfo::default().read(obj.hit_sample(), false);
        let lookup_time = time_ms + CONTROL_POINT_LENIENCY;
        let point = cp.sample_point_at(lookup_time);
        for sample in convert_sound_type(obj.hitsound(), &bank) {
            let applied = sample.apply(point);
            let lane = legacy_to_identity(&applied);
            let volume = played_volume(applied.volume);
            raw.push(ImportedManiaHit {
                time_ms,
                lane,
                volume,
            });
        }
    }

    // Sort by time, then lane identity for deterministic deduplication.
    raw.sort_by(|a, b| {
        a.time_ms
            .total_cmp(&b.time_ms)
            .then_with(|| a.lane.cmp(&b.lane))
    });

    // Deduplicate hits on the same lane at effectively the same timestamp (< 0.5 ms),
    // keeping the louder volume when multiple mania columns trigger the same sample.
    let mut deduped: Vec<ImportedManiaHit> = Vec::with_capacity(raw.len());
    for hit in raw {
        if let Some(prev) = deduped
            .iter_mut()
            .rev()
            .take_while(|p| (hit.time_ms - p.time_ms).abs() < 0.5)
            .find(|p| p.lane == hit.lane)
        {
            if hit.volume > prev.volume {
                prev.volume = hit.volume;
            }
        } else {
            deduped.push(hit);
        }
    }

    Ok(deduped)
}

fn legacy_to_identity(sample: &LegacySample) -> SampleIdentity {
    if let Some(filename) = &sample.filename {
        return SampleIdentity::File(filename.clone());
    }
    let sound = match sample.name.as_str() {
        "hitwhistle" => Sound::Hit(Hitsound::Whistle),
        "hitfinish" => Sound::Hit(Hitsound::Finish),
        "hitclap" => Sound::Hit(Hitsound::Clap),
        "sliderslide" => Sound::Slider(SliderSound::Slide),
        "sliderwhistle" => Sound::Slider(SliderSound::Whistle),
        "slidertick" => Sound::Slider(SliderSound::Tick),
        _ => Sound::Hit(Hitsound::Normal),
    };
    let set = match sample.bank.as_str() {
        "soft" => SampleSet::Soft,
        "drum" => SampleSet::Drum,
        _ => SampleSet::Normal,
    };
    SampleIdentity::Standard {
        set,
        sound,
        index: CustomIndex::new(sample.custom_sample_bank().max(0) as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_lane(set: SampleSet, hit: Hitsound, index: u32) -> SampleIdentity {
        SampleIdentity::Standard {
            set,
            sound: Sound::Hit(hit),
            index: CustomIndex::new(index),
        }
    }

    #[test]
    fn refuses_non_mania_beatmaps() {
        for (mode, expected) in [(0, "osu!standard"), (1, "osu!taiko"), (2, "osu!catch")] {
            let osu = format!(
                "osu file format v14\n\n[General]\nMode: {mode}\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n"
            );
            let err = parse_mania_hitsounds_bytes(osu.as_bytes()).unwrap_err();
            assert!(
                err.contains(expected) && err.contains("Mode: 3"),
                "unexpected error for mode {mode}: {err}"
            );
        }
    }

    #[test]
    fn parses_standard_custom_index_filenames_holds_and_volumes() {
        let osu = "\
osu file format v14

[General]
AudioFilename: audio.mp3
Mode: 3

[TimingPoints]
0,500,4,2,3,65,1,0
2000,-100,4,3,0,2,0,0

[HitObjects]
64,192,1000,1,0,0:0:0:0:
192,192,1000,1,8,0:1:4:80:
320,192,1000,1,0,0:0:0:55:kick.wav
448,192,2000,128,2,2500:0:0:0:0:
64,192,2000,1,0,0:0:0:0:snare.ogg
";
        let hits = parse_mania_hitsounds_bytes(osu.as_bytes()).expect("valid mania diff");

        // At 1000 ms:
        // - col 1: soft-hitnormal index 3 vol 65 (inherited from timing point at 0 ms)
        // - col 2: normal-hitclap index 4 vol 80 (plus layered soft-hitnormal index 4 vol 80)
        // - col 3: File("kick.wav") vol 55
        assert!(hits.contains(&ImportedManiaHit {
            time_ms: 1000.0,
            lane: std_lane(SampleSet::Soft, Hitsound::Normal, 3),
            volume: 65,
        }));
        assert!(hits.contains(&ImportedManiaHit {
            time_ms: 1000.0,
            lane: std_lane(SampleSet::Normal, Hitsound::Clap, 4),
            volume: 80,
        }));
        assert!(hits.contains(&ImportedManiaHit {
            time_ms: 1000.0,
            lane: SampleIdentity::File("kick.wav".into()),
            volume: 55,
        }));

        // At 2000 ms (timing point volume is 2% -> clamped to 5% floor):
        // - hold note with whistle flag -> drum-hitwhistle index 0 vol 5 (and layered drum-hitnormal vol 5)
        // - custom filename snare.ogg -> File("snare.ogg") vol 5
        assert!(hits.contains(&ImportedManiaHit {
            time_ms: 2000.0,
            lane: std_lane(SampleSet::Drum, Hitsound::Whistle, 0),
            volume: 5,
        }));
        assert!(hits.contains(&ImportedManiaHit {
            time_ms: 2000.0,
            lane: SampleIdentity::File("snare.ogg".into()),
            volume: 5,
        }));

        // Sorted by time.
        assert!(hits.windows(2).all(|w| w[0].time_ms <= w[1].time_ms));
    }

    #[test]
    fn deduplicates_same_lane_at_same_time_keeping_louder_volume() {
        let osu = "\
osu file format v14

[General]
Mode: 3

[TimingPoints]
0,500,4,1,0,70,1,0

[HitObjects]
64,192,1500,1,0,0:0:0:40:hihat.wav
192,192,1500,1,0,0:0:0:90:hihat.wav
320,192,500,1,0,0:0:0:60:hihat.wav
";
        let hits = parse_mania_hitsounds_bytes(osu.as_bytes()).expect("valid mania diff");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].time_ms, 500.0);
        assert_eq!(hits[0].volume, 60);
        assert_eq!(hits[1].time_ms, 1500.0);
        assert_eq!(hits[1].volume, 90);
    }
}
