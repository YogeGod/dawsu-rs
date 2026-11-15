// Ported from osu!lazer (ppy/osu) at 131fe72a15581049401a588a13684938d4d0e0c7:
//   osu.Game/Beatmaps/Formats/LegacyBeatmapDecoder.cs (handleTimingPoint, addControlPoint,
//   flushPendingPoints, the format-version offset)
//   osu.Game/Beatmaps/Legacy/LegacyControlPointInfo.cs (SamplePointAt, DifficultyPointAt, CheckAlreadyExisting)
//   osu.Game/Beatmaps/ControlPoints/ControlPointInfo.cs (TimingPointAt, BinarySearch with Rightmost)
//   osu.Game/Beatmaps/ControlPoints/{Timing,Difficulty,Sample}ControlPoint.cs and
//   LegacyDecoder.LegacySampleControlPoint (IsRedundant, clamps)
// Copyright (c) ppy Pty Ltd <contact@ppy.sh>. Licensed under the MIT Licence (ADR 0027).

use dawsu_osu_format::{OsuFile, TimingPoint};

/// `LegacyBeatmapDecoder.EARLY_VERSION_TIMING_OFFSET`, added to times in files before v5.
pub const EARLY_VERSION_TIMING_OFFSET: f64 = 24.0;
/// `LegacyBeatmapDecoder.CONTROL_POINT_LENIENCY`.
pub const CONTROL_POINT_LENIENCY: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingPointData {
    pub time: f64,
    pub beat_length: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DifficultyPointData {
    pub time: f64,
    pub slider_velocity: f64,
    pub generate_ticks: bool,
}

/// `LegacySampleControlPoint`.
#[derive(Debug, Clone, PartialEq)]
pub struct SamplePointData {
    pub time: f64,
    pub bank: String,
    pub volume: i32,
    pub custom_sample_bank: i32,
}

#[derive(Debug, Clone, PartialEq)]
enum Pending {
    Timing(TimingPointData),
    Difficulty(DifficultyPointData),
    Sample(SamplePointData),
}

impl Pending {
    fn kind(&self) -> u8 {
        match self {
            Pending::Timing(_) => 0,
            Pending::Difficulty(_) => 1,
            Pending::Sample(_) => 2,
        }
    }
}

/// The control points lazer builds from a file's timing lines.
#[derive(Debug, Clone, Default)]
pub struct ControlPoints {
    pub timing: Vec<TimingPointData>,
    pub difficulty: Vec<DifficultyPointData>,
    pub sample: Vec<SamplePointData>,
}

/// `TimingControlPoint.DEFAULT`'s beat length.
const DEFAULT_BEAT_LENGTH: f64 = 1000.0;

/// The format-version offset: 24 ms before v5, else 0. A file with no version header counts as
/// the latest version.
pub fn offset_for(format_version: Option<u32>) -> f64 {
    match format_version {
        Some(v) if v < 5 => EARLY_VERSION_TIMING_OFFSET,
        _ => 0.0,
    }
}

/// The `LegacySampleBank` name for a timing line's sample set: lazer lowercases the enum's
/// `ToString()`, which is the number itself for values outside the enum; "none" plays "normal".
fn bank_name(sample_set: i32) -> String {
    match sample_set {
        0 | 1 => "normal".into(),
        2 => "soft".into(),
        3 => "drum".into(),
        n => n.to_string(),
    }
}

/// `[General] SampleSet`, read with `Enum.Parse<LegacySampleBank>`: a name or a number.
fn default_sample_set(file: &OsuFile) -> i32 {
    match file.general("SampleSet").as_deref() {
        None => 0,
        Some(v) => match v {
            "None" => 0,
            "Normal" => 1,
            "Soft" => 2,
            "Drum" => 3,
            other => other.parse().unwrap_or(0),
        },
    }
}

impl ControlPoints {
    pub fn from_file(file: &OsuFile) -> ControlPoints {
        let offset = offset_for(file.format_version());
        let default_bank = default_sample_set(file);
        let default_volume = file.general("SampleVolume").and_then(|v| dawsu_osu_format::number::parse_int(&v).ok()).unwrap_or(100);
        let mut cp = ControlPoints::default();
        let mut pending: Vec<Pending> = Vec::new();
        let mut pending_time = 0.0f64;
        for tp in file.timing_points() {
            cp.handle_timing_point(&tp, offset, default_bank, default_volume, &mut pending, &mut pending_time);
        }
        cp.flush(&mut pending, pending_time);
        cp
    }

    fn handle_timing_point(&mut self, tp: &TimingPoint, offset: f64, default_bank: i32, default_volume: i32, pending: &mut Vec<Pending>, pending_time: &mut f64) {
        let time = tp.time() + offset;
        let beat_length = tp.beat_length();
        let speed_multiplier = if beat_length < 0.0 { 100.0 / -beat_length } else { 1.0 };
        let sample_set = tp.sample_set().unwrap_or(default_bank);
        let custom_sample_bank = tp.sample_index().unwrap_or(0);
        let volume = tp.volume().unwrap_or(default_volume);
        let timing_change = tp.uninherited();

        if timing_change {
            self.add(time, Pending::Timing(TimingPointData { time, beat_length: beat_length.clamp(6.0, 60000.0) }), true, pending, pending_time);
        }
        let sv = speed_multiplier.clamp(0.1, 10.0);
        self.add(time, Pending::Difficulty(DifficultyPointData { time, slider_velocity: sv, generate_ticks: !beat_length.is_nan() }), timing_change, pending, pending_time);
        // SampleControlPoint.SampleVolumeBindable is limited to 0-100 (R14).
        self.add(
            time,
            Pending::Sample(SamplePointData { time, bank: bank_name(sample_set), volume: volume.clamp(0, 100), custom_sample_bank }),
            timing_change,
            pending,
            pending_time,
        );
    }

    fn add(&mut self, time: f64, point: Pending, timing_change: bool, pending: &mut Vec<Pending>, pending_time: &mut f64) {
        if time != *pending_time {
            self.flush(pending, *pending_time);
        }
        if timing_change {
            pending.insert(0, point);
        } else {
            pending.push(point);
        }
        *pending_time = time;
    }

    /// Points from green lines are at the end of the list and win over those from red lines.
    fn flush(&mut self, pending: &mut Vec<Pending>, time: f64) {
        let mut seen = [false; 3];
        for point in pending.iter().rev() {
            let k = point.kind() as usize;
            if seen[k] {
                continue;
            }
            seen[k] = true;
            self.add_point(time, point.clone());
        }
        pending.clear();
    }

    fn add_point(&mut self, time: f64, point: Pending) {
        match point {
            // Timing points are never redundant.
            Pending::Timing(mut t) => {
                t.time = time;
                insert_sorted(&mut self.timing, t, |p| p.time);
            }
            Pending::Difficulty(mut d) => {
                d.time = time;
                let current = self.difficulty_point_at(time);
                if current.slider_velocity == d.slider_velocity && current.generate_ticks == d.generate_ticks {
                    return;
                }
                insert_sorted(&mut self.difficulty, d, |p| p.time);
            }
            Pending::Sample(mut s) => {
                s.time = time;
                // Intentionally not SamplePointAt: the first sample point is always considered.
                if let Some(existing) = binary_search(&self.sample, time, |p| p.time) {
                    if existing.bank == s.bank && existing.volume == s.volume && existing.custom_sample_bank == s.custom_sample_bank {
                        return;
                    }
                }
                insert_sorted(&mut self.sample, s, |p| p.time);
            }
        }
    }

    /// `TimingPointAt`: before the first red line, the first red line; with none, 1000 ms.
    pub fn timing_point_at(&self, time: f64) -> TimingPointData {
        binary_search(&self.timing, time, |p| p.time)
            .copied()
            .unwrap_or_else(|| self.timing.first().copied().unwrap_or(TimingPointData { time: 0.0, beat_length: DEFAULT_BEAT_LENGTH }))
    }

    /// `DifficultyPointAt`: before the first stored point, velocity 1.
    pub fn difficulty_point_at(&self, time: f64) -> DifficultyPointData {
        binary_search(&self.difficulty, time, |p| p.time).copied().unwrap_or(DifficultyPointData { time: 0.0, slider_velocity: 1.0, generate_ticks: true })
    }

    /// `SamplePointAt`: before the first point, the first point; with none, `None`
    /// (lazer then uses `SampleControlPoint.DEFAULT`, which is not a legacy point).
    pub fn sample_point_at(&self, time: f64) -> Option<&SamplePointData> {
        binary_search(&self.sample, time, |p| p.time).or_else(|| self.sample.first())
    }
}

/// The rightmost item at or before `time` (`EqualitySelection.Rightmost`), or `None` before the first.
fn binary_search<T>(list: &[T], time: f64, key: impl Fn(&T) -> f64) -> Option<&T> {
    let after = list.partition_point(|p| key(p) <= time);
    after.checked_sub(1).map(|i| &list[i])
}

// shortcut: lazer's SortedList inserts an item whose time equals existing ones at whatever index
// List.BinarySearch returns; this inserts after them. It only differs for a file that repeats a
// time out of order across other times. Revisit if the oracle shows such a map.
fn insert_sorted<T>(list: &mut Vec<T>, item: T, key: impl Fn(&T) -> f64) {
    let t = key(&item);
    let at = list.partition_point(|p| key(p) <= t);
    list.insert(at, item);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(text: &str) -> ControlPoints {
        let read = OsuFile::read(format!("osu file format v14\r\n\r\n[TimingPoints]\r\n{text}").into_bytes());
        ControlPoints::from_file(&read.file)
    }

    #[test]
    fn before_the_first_stored_difficulty_point_the_velocity_is_1() {
        let cp = points("1345,375,4,2,1,60,1,0\r\n1345,-166.666666666667,4,2,1,60,0,0\r\n");
        assert_eq!(cp.difficulty_point_at(1002.0).slider_velocity, 1.0);
        assert!((cp.difficulty_point_at(1345.0).slider_velocity - 0.6).abs() < 1e-9, "the green line beside the red line wins");
        assert_eq!(cp.timing_point_at(1002.0).beat_length, 375.0, "the first red line reaches back");
    }

    #[test]
    fn a_red_line_resets_the_velocity_to_1() {
        // lazer's own test: osu.Game.Tests/Resources/timingpoint-speedmultiplier-reset.osu
        let cp = points("0,-200,4,1,0,100,0,0\r\n2000,100,1,1,0,100,1,0\r\n");
        assert_eq!(cp.difficulty_point_at(0.0).slider_velocity, 0.5);
        assert_eq!(cp.difficulty_point_at(2000.0).slider_velocity, 1.0);
    }

    #[test]
    fn of_two_red_lines_at_one_time_the_earlier_wins_and_of_two_green_lines_the_later() {
        let cp = points("0,500,4,1,0,100,1,0\r\n0,250,4,1,0,100,1,0\r\n1000,-50,4,1,0,100,0,0\r\n1000,-200,4,1,0,100,0,0\r\n");
        assert_eq!(cp.timing_point_at(0.0).beat_length, 500.0);
        assert_eq!(cp.difficulty_point_at(1000.0).slider_velocity, 0.5);
    }

    #[test]
    fn sample_points_clamp_their_volume_and_name_their_bank() {
        let cp = points("0,500,4,2,3,150,1,0\r\n1000,-100,4,7,0,40,0,0\r\n");
        let first = cp.sample_point_at(-50.0).unwrap();
        assert_eq!((first.bank.as_str(), first.volume, first.custom_sample_bank), ("soft", 100, 3));
        assert_eq!(cp.sample_point_at(1000.0).unwrap().bank, "7", "an unknown sample set keeps its number as lazer's bank name");
    }

    #[test]
    fn files_before_v5_get_24_ms() {
        let read = OsuFile::read(b"osu file format v4\r\n\r\n[TimingPoints]\r\n100,500\r\n".to_vec());
        let cp = ControlPoints::from_file(&read.file);
        assert_eq!(cp.timing[0].time, 124.0);
    }
}
