//! Native `.osu` green-line slider tick hitsound rendering (`TickPolicy::Render`) and rollback history.
//!
//! Writes a short green line at each matched slider tick timestamp pointing at a custom
//! `slidertick` sample index (`body_sample_first_index`, default 100+) and reverts to the
//! prior green-line state `+5 ms` later. Records every written tick index and slider-body edit
//! in [`SliderEditHistory`] so repeat exports undo only dawsu!'s previous writes while keeping
//! manual edits made in osu!.

use crate::lazer::control_points::{ControlPoints, offset_for};
use dawsu_foundation::SampleSet;
use dawsu_osu_format::{OsuFile, TimingPoint};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Default first custom sample index reserved for generated `slidertick` / `sliderslide` files.
pub const DEFAULT_BODY_SAMPLE_FIRST_INDEX: u32 = 100;
/// Delay in milliseconds after a tick green line before restoring the prior timing-point state.
pub const TICK_REVERT_MS: f64 = 5.0;

/// What to do when hitsound events land on a slider tick instead of a clickable edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TickPolicy {
    /// Leave slider ticks untouched (default).
    #[default]
    Ignore,
    /// Write a short green line + custom `slidertick` index at each matched tick, reverting at `+5 ms`.
    Render,
}

/// One slider tick sound to render into a difficulty via a short green line.
#[derive(Debug, Clone, PartialEq)]
pub struct TickWrite {
    /// Song timestamp of the slider tick in ms.
    pub time_ms: f64,
    /// Sample set for `<set>-slidertick<index>`.
    pub sample_set: SampleSet,
    /// Custom sample index (>= 1, typically >= 100).
    pub custom_index: u32,
    /// Green-line volume clamped to `5..=100`.
    pub volume: i32,
}

/// History of tick indices and slider-body edits previously written by dawsu! so repeat direct
/// exports can cleanly restore only dawsu!'s own edits without clobbering mapper changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SliderEditHistory {
    #[serde(default)]
    pub tick_indices: BTreeSet<u32>,
    /// Map of `<difficulty filename>` -> list of previously injected tick green-line timestamps (ms).
    #[serde(default)]
    pub tick_times_by_diff: BTreeMap<String, Vec<i64>>,
}

impl SliderEditHistory {
    /// Removes green lines injected by a previous dawsu! tick-render pass on `diff_file`,
    /// preserving every timing point edited or added manually in osu!.
    pub fn restore_ticks(&self, diff_file: &str, file: &OsuFile) -> (OsuFile, usize) {
        if self.tick_indices.is_empty() {
            return (file.clone(), 0);
        }
        let known_times: BTreeSet<i64> = self
            .tick_times_by_diff
            .get(diff_file)
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let mut out = file.clone();
        let offset = offset_for(file.format_version());
        let tps: Vec<TimingPoint> = out.timing_points();
        let mut to_remove = Vec::new();
        for (i, tp) in tps.iter().enumerate() {
            if tp.uninherited() {
                continue;
            }
            let idx = tp.sample_index().unwrap_or(0).max(0) as u32;
            let t_ms = (tp.time() + offset).round() as i64;
            if self.tick_indices.contains(&idx)
                && (known_times.is_empty() || known_times.contains(&t_ms))
            {
                to_remove.push(tp.line());
                // Also remove the matching revert green line placed within +5..+6 ms if it
                // carries the same slider velocity and kiai flag as the previous timing point.
                if let Some(next) = tps.get(i + 1)
                    && !next.uninherited()
                    && (next.time() - tp.time() - TICK_REVERT_MS).abs() <= 1.5
                    && !self.tick_indices.contains(&(next.sample_index().unwrap_or(0).max(0) as u32))
                {
                    to_remove.push(next.line());
                }
            }
        }
        let removed = to_remove.len();
        for line in to_remove {
            let _ = out.remove_green_line(line);
        }
        (out, removed)
    }

    pub fn record_ticks(&mut self, diff_file: &str, writes: &[TickWrite]) {
        let mut times = Vec::with_capacity(writes.len());
        for w in writes {
            if w.custom_index > 0 {
                self.tick_indices.insert(w.custom_index);
                times.push(w.time_ms.round() as i64);
            }
        }
        times.sort_unstable();
        times.dedup();
        if times.is_empty() {
            self.tick_times_by_diff.remove(diff_file);
        } else {
            self.tick_times_by_diff.insert(diff_file.to_owned(), times);
        }
    }
}

/// Applies `writes` to `file` by inserting a green line at each tick timestamp with its custom
/// `slidertick` sample set, index, and volume, followed by a revert green line `+5 ms` later
/// restoring the prior sample set, index, and volume (unless another timing point already sits
/// inside that 5 ms window).
///
/// Returns `(patched_file, applied_tick_count)`.
pub fn apply_tick_greenlines(file: &OsuFile, writes: &[TickWrite]) -> (OsuFile, usize) {
    if writes.is_empty() {
        return (file.clone(), 0);
    }
    let mut out = file.clone();
    let offset = offset_for(file.format_version());
    let cp = ControlPoints::from_file(file);
    let existing_tps: Vec<TimingPoint> = out.timing_points();
    let existing_times: Vec<f64> = existing_tps.iter().map(|tp| tp.time() + offset).collect();

    let mut ordered = writes.to_vec();
    ordered.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));

    let mut applied = 0usize;
    for (idx, tick) in ordered.iter().enumerate() {
        if tick.custom_index == 0 {
            continue;
        }
        let raw_time = (tick.time_ms - offset).round();
        let dp = cp.difficulty_point_at(tick.time_ms);
        let beat_length = if dp.slider_velocity > 0.0 {
            -100.0 / dp.slider_velocity
        } else {
            -100.0
        };
        let effects = existing_tps
            .iter()
            .rev()
            .find(|tp| tp.time() <= raw_time)
            .and_then(TimingPoint::effects)
            .unwrap_or(0);
        let set_code = sample_set_to_osu(tick.sample_set);
        let vol = tick.volume.clamp(5, 100);

        let tick_line = format!(
            "{raw_time:.0},{beat_length:.6},4,{set_code},{},{vol},0,{effects}",
            tick.custom_index
        );
        out.insert_green_line(&tick_line);
        applied += 1;

        // Only emit a revert line at +5 ms if neither an existing timing point nor the next tick
        // already resets state before then.
        let revert_song_ms = tick.time_ms + TICK_REVERT_MS;
        let blocked_by_existing = existing_times
            .iter()
            .any(|&t| t > tick.time_ms + 0.5 && t <= revert_song_ms + 0.5);
        let blocked_by_next_tick = ordered
            .get(idx + 1)
            .is_some_and(|next| next.time_ms <= revert_song_ms + 0.5);
        if !blocked_by_existing && !blocked_by_next_tick {
            let prior = cp.sample_point_at(tick.time_ms);
            let prior_set = prior.map_or(0, |sp| bank_str_to_osu(&sp.bank));
            let prior_idx = prior.map_or(0, |sp| sp.custom_sample_bank.max(0));
            let prior_vol = prior.map_or(100, |sp| sp.volume.clamp(5, 100));
            let revert_raw = (revert_song_ms - offset).round();
            let revert_line = format!(
                "{revert_raw:.0},{beat_length:.6},4,{prior_set},{prior_idx},{prior_vol},0,{effects}"
            );
            out.insert_green_line(&revert_line);
        }
    }

    (out, applied)
}

fn sample_set_to_osu(set: SampleSet) -> i32 {
    match set {
        SampleSet::Normal => 1,
        SampleSet::Soft => 2,
        SampleSet::Drum => 3,
    }
}

fn bank_str_to_osu(bank: &str) -> i32 {
    match bank {
        "normal" => 1,
        "soft" => 2,
        "drum" => 3,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_greenlines_write_and_revert_at_plus_5ms_and_rollback_cleanly() {
        let raw = concat!(
            "osu file format v14\r\n\r\n",
            "[General]\r\nMode: 0\r\n\r\n",
            "[TimingPoints]\r\n0,500,4,2,1,80,1,0\r\n\r\n",
            "[HitObjects]\r\n",
            "100,100,1000,2,0,L|300:100,1,200,0|0,0:0|0:0,0:0:0:0:\r\n",
        );
        let file = OsuFile::read(raw.as_bytes().to_vec()).file;
        let writes = vec![TickWrite {
            time_ms: 1250.0,
            sample_set: SampleSet::Drum,
            custom_index: 100,
            volume: 90,
        }];
        let (patched, count) = apply_tick_greenlines(&file, &writes);
        assert_eq!(count, 1);
        let tps = patched.timing_points();
        assert_eq!(tps.len(), 3, "red line + tick line + revert line");
        assert_eq!(tps[1].time(), 1250.0);
        assert_eq!(tps[1].sample_index(), Some(100));
        assert_eq!(tps[1].volume(), Some(90));
        assert_eq!(tps[2].time(), 1255.0);
        assert_eq!(tps[2].sample_index(), Some(1));
        assert_eq!(tps[2].volume(), Some(80));

        let mut history = SliderEditHistory::default();
        history.record_ticks("Hard.osu", &writes);
        let (restored, removed) = history.restore_ticks("Hard.osu", &patched);
        assert_eq!(removed, 2);
        assert_eq!(restored.timing_points().len(), 1);
        assert_eq!(restored.to_bytes(), file.to_bytes());
    }
}
