//! Non-destructive green-line-only master volume envelope (shared or per-difficulty).
//!
//! Scaling green-line volumes through a piecewise Linear (ramp) or Step (hold) envelope lets a
//! mapper shape section dynamics across an entire difficulty without baking extra sample mixes.

use crate::lazer::control_points::{ControlPoints, offset_for};
use dawsu_osu_format::{OsuFile, TimingPoint};
use serde::{Deserialize, Serialize};

/// Minimum green-line volume written to `.osu` (matches osu!lazer's 5% floor).
pub const MIN_ENVELOPE_VOLUME: u32 = 5;
/// Maximum green-line volume written to `.osu`.
pub const MAX_ENVELOPE_VOLUME: u32 = 100;

/// Interpolation curve from one envelope point to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveKind {
    /// Linearly interpolate volume from this point to the next point (ramp).
    #[default]
    Linear,
    /// Hold this point's volume flat until the next point (step).
    Step,
}

/// One control node on the master volume envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumePoint {
    pub time_ms: i64,
    /// Volume percentage clamped to `5..=100`.
    pub volume: u32,
    #[serde(default)]
    pub curve: CurveKind,
}

/// Ordered list of envelope nodes. Empty means 100% everywhere (no change).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeEnvelope {
    #[serde(default)]
    pub points: Vec<VolumePoint>,
}

impl VolumeEnvelope {
    /// Builds a normalized envelope sorted by `time_ms`, keeping the last point on duplicate times
    /// and clamping volumes to `5..=100`.
    pub fn new(mut points: Vec<VolumePoint>) -> Self {
        for p in &mut points {
            p.volume = p.volume.clamp(MIN_ENVELOPE_VOLUME, MAX_ENVELOPE_VOLUME);
        }
        points.sort_by_key(|p| p.time_ms);
        let mut out: Vec<VolumePoint> = Vec::with_capacity(points.len());
        for p in points {
            match out.last_mut() {
                Some(last) if last.time_ms == p.time_ms => *last = p,
                _ => out.push(p),
            }
        }
        Self { points: out }
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Adds or updates a point at `time_ms` and keeps the envelope normalized.
    pub fn upsert(&mut self, point: VolumePoint) {
        let clamped = VolumePoint {
            time_ms: point.time_ms,
            volume: point.volume.clamp(MIN_ENVELOPE_VOLUME, MAX_ENVELOPE_VOLUME),
            curve: point.curve,
        };
        match self.points.binary_search_by_key(&clamped.time_ms, |p| p.time_ms) {
            Ok(idx) => self.points[idx] = clamped,
            Err(idx) => self.points.insert(idx, clamped),
        }
    }

    /// Removes the point at `time_ms`, returning `true` if one existed.
    pub fn remove_at(&mut self, time_ms: i64) -> bool {
        if let Ok(idx) = self.points.binary_search_by_key(&time_ms, |p| p.time_ms) {
            self.points.remove(idx);
            true
        } else {
            false
        }
    }

    /// Toggles the curve at `time_ms` between `Linear` (ramp) and `Step` (hold).
    pub fn toggle_curve_at(&mut self, time_ms: i64) -> Option<CurveKind> {
        let idx = self.points.binary_search_by_key(&time_ms, |p| p.time_ms).ok()?;
        let next = match self.points[idx].curve {
            CurveKind::Linear => CurveKind::Step,
            CurveKind::Step => CurveKind::Linear,
        };
        self.points[idx].curve = next;
        Some(next)
    }

    /// Evaluates the envelope percentage (`5.0..=100.0`) at `time_ms`.
    /// Before the first point, the first point's value holds; after the last, the last holds.
    pub fn at(&self, time_ms: f64) -> f64 {
        let Some(first) = self.points.first() else {
            return f64::from(MAX_ENVELOPE_VOLUME);
        };
        let next = self.points.partition_point(|p| (p.time_ms as f64) <= time_ms);
        if next == 0 {
            return f64::from(first.volume);
        }
        let a = self.points[next - 1];
        let Some(b) = self.points.get(next) else {
            return f64::from(a.volume);
        };
        match a.curve {
            CurveKind::Step => f64::from(a.volume),
            CurveKind::Linear => {
                let span = (b.time_ms - a.time_ms) as f64;
                if span <= 0.0 {
                    return f64::from(b.volume);
                }
                let k = ((time_ms - a.time_ms as f64) / span).clamp(0.0, 1.0);
                f64::from(a.volume) + (f64::from(b.volume) - f64::from(a.volume)) * k
            }
        }
    }

    /// Scales an osu! green-line volume (`1..=100`) by the envelope at `time_ms`, clamped to `5..=100`.
    pub fn scale(&self, base_volume: i32, time_ms: f64) -> i32 {
        if self.is_empty() {
            return base_volume.clamp(MIN_ENVELOPE_VOLUME as i32, MAX_ENVELOPE_VOLUME as i32);
        }
        let scaled = f64::from(base_volume.max(0)) * self.at(time_ms) / 100.0;
        (scaled.round() as i32).clamp(MIN_ENVELOPE_VOLUME as i32, MAX_ENVELOPE_VOLUME as i32)
    }
}

/// Applies `envelope` purely via `[TimingPoints]` green-line volume scaling without modifying
/// hit-object sample names or creating any extra sample mixes.
///
/// - Scales every existing timing point's volume by `envelope.at(tp_time)` (updating only lines
///   whose scaled volume actually changes, preserving untouched lines byte-identically).
/// - Inserts green lines at envelope nodes (and at hit object timestamps that sit on a Linear ramp
///   between two nodes whose scaled volume differs from the active green line) so ramps audition
///   smoothly across every hit object.
///
/// Returns `(patched_file, changed_timing_lines_count)`.
pub fn apply_envelope_to_osu(file: &OsuFile, envelope: &VolumeEnvelope) -> (OsuFile, usize) {
    if envelope.is_empty() {
        return (file.clone(), 0);
    }

    let mut out = file.clone();
    let offset = offset_for(file.format_version());
    let cp = ControlPoints::from_file(file);
    let mut changed = 0usize;

    // 1. Scale every existing timing point whose scaled volume differs.
    let existing_tps: Vec<TimingPoint> = out.timing_points();
    for tp in &existing_tps {
        let time_ms = tp.time() + offset;
        let cur_vol = tp.volume().unwrap_or(100);
        let scaled = envelope.scale(cur_vol, time_ms);
        if scaled != cur_vol && out.set_field(tp.line(), 5, &scaled.to_string()).is_ok() {
            changed += 1;
        }
    }

    // Collect timestamps already covered by a timing point so we don't duplicate.
    let mut tp_times: Vec<i64> = existing_tps
        .iter()
        .map(|tp| (tp.time() + offset).round() as i64)
        .collect();
    tp_times.sort_unstable();
    tp_times.dedup();

    // 2. Collect candidate timestamps where a new green line may be needed:
    //    - every envelope node itself (to establish step jumps & ramp anchors),
    //    - every hit object time that falls inside a non-flat linear ramp segment.
    let mut candidates: Vec<i64> = envelope.points.iter().map(|p| p.time_ms).collect();
    for obj in file.hit_objects() {
        let t = (obj.time() + offset).round() as i64;
        if is_on_sloped_ramp(envelope, t) {
            candidates.push(t);
        }
    }
    candidates.sort_unstable();
    candidates.dedup();

    let mut last_written_vol: Option<i32> = None;
    for t_ms in candidates {
        if tp_times.binary_search(&t_ms).is_ok() {
            // Track what volume is active after this existing timing point.
            if let Some(sp) = cp.sample_point_at(t_ms as f64) {
                last_written_vol = Some(envelope.scale(sp.volume, t_ms as f64));
            }
            continue;
        }
        let sample_pt = cp.sample_point_at(t_ms as f64);
        let base_vol = sample_pt.map_or(100, |sp| sp.volume);
        let target_vol = envelope.scale(base_vol, t_ms as f64);
        if last_written_vol == Some(target_vol) {
            continue;
        }
        let sample_set = sample_pt.map_or(0, |sp| sample_set_code(sp.bank.as_str()));
        let sample_index = sample_pt.map_or(0, |sp| sp.custom_sample_bank.max(0));
        let dp = cp.difficulty_point_at(t_ms as f64);
        let beat_length = if dp.slider_velocity > 0.0 {
            -100.0 / dp.slider_velocity
        } else {
            -100.0
        };
        let effects = active_effects(&existing_tps, t_ms as f64 - offset);
        let raw_time = (t_ms as f64 - offset).round() as i64;
        let line = format!(
            "{raw_time},{beat_length:.6},4,{sample_set},{sample_index},{target_vol},0,{effects}"
        );
        out.insert_green_line(&line);
        tp_times.push(t_ms);
        tp_times.sort_unstable();
        last_written_vol = Some(target_vol);
        changed += 1;
    }

    (out, changed)
}

fn active_effects(tps: &[TimingPoint], raw_time: f64) -> i32 {
    tps.iter()
        .rev()
        .find(|tp| tp.time() <= raw_time)
        .and_then(TimingPoint::effects)
        .unwrap_or(0)
}

fn is_on_sloped_ramp(envelope: &VolumeEnvelope, time_ms: i64) -> bool {
    let idx = envelope.points.partition_point(|p| p.time_ms <= time_ms);
    if idx == 0 || idx >= envelope.points.len() {
        return false;
    }
    let a = envelope.points[idx - 1];
    let b = envelope.points[idx];
    a.curve == CurveKind::Linear && a.volume != b.volume
}

fn sample_set_code(bank: &str) -> i32 {
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

    fn pt(time_ms: i64, volume: u32, curve: CurveKind) -> VolumePoint {
        VolumePoint { time_ms, volume, curve }
    }

    #[test]
    fn empty_envelope_leaves_volume_and_file_unchanged() {
        let env = VolumeEnvelope::default();
        assert_eq!(env.at(500.0), 100.0);
        assert_eq!(env.scale(70, 500.0), 70);
        let raw = "osu file format v14\n\n[General]\nMode: 0\n\n[TimingPoints]\n0,500,4,2,0,80,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        let file = OsuFile::read(raw.as_bytes().to_vec()).file;
        let (out, changed) = apply_envelope_to_osu(&file, &env);
        assert_eq!(changed, 0);
        assert_eq!(out.to_bytes(), file.to_bytes());
    }

    #[test]
    fn linear_ramp_and_step_hold_evaluate_and_clamp_correctly() {
        let env = VolumeEnvelope::new(vec![
            pt(1000, 100, CurveKind::Linear),
            pt(2000, 60, CurveKind::Step),
            pt(3000, 80, CurveKind::Linear),
        ]);
        assert_eq!(env.at(0.0), 100.0);
        assert!((env.at(1500.0) - 80.0).abs() < 1e-6);
        assert_eq!(env.at(2500.0), 60.0);
        assert_eq!(env.at(4000.0), 80.0);
        assert_eq!(env.scale(50, 1500.0), 40);
        assert_eq!(env.scale(1, 2500.0), 5, "clamped to 5% floor");
    }

    #[test]
    fn upsert_toggle_and_remove_maintain_sorted_order() {
        let mut env = VolumeEnvelope::default();
        env.upsert(pt(2000, 50, CurveKind::Linear));
        env.upsert(pt(1000, 90, CurveKind::Step));
        assert_eq!(env.points.len(), 2);
        assert_eq!(env.points[0].time_ms, 1000);
        assert_eq!(env.toggle_curve_at(1000), Some(CurveKind::Linear));
        assert_eq!(env.points[0].curve, CurveKind::Linear);
        assert!(env.remove_at(2000));
        assert_eq!(env.points.len(), 1);
    }

    #[test]
    fn apply_envelope_scales_green_lines_without_touching_hit_objects() {
        let raw = concat!(
            "osu file format v14\r\n\r\n",
            "[General]\r\nMode: 0\r\n\r\n",
            "[TimingPoints]\r\n0,500,4,2,1,100,1,0\r\n\r\n",
            "[HitObjects]\r\n",
            "256,192,1000,1,0,0:0:0:0:\r\n",
            "256,192,1500,1,0,0:0:0:0:\r\n",
            "256,192,2000,1,0,0:0:0:0:\r\n",
        );
        let file = OsuFile::read(raw.as_bytes().to_vec()).file;
        let env = VolumeEnvelope::new(vec![
            pt(1000, 100, CurveKind::Linear),
            pt(2000, 50, CurveKind::Step),
        ]);
        let (patched, changed) = apply_envelope_to_osu(&file, &env);
        assert!(changed >= 2, "expected ramp & endpoint green lines");
        assert_eq!(patched.hit_objects().len(), 3, "hit objects unchanged");
        let tps = patched.timing_points();
        assert!(tps.iter().any(|tp| (tp.time() - 1500.0).abs() < 1.0 && tp.volume() == Some(75)));
        assert!(tps.iter().any(|tp| (tp.time() - 2000.0).abs() < 1.0 && tp.volume() == Some(50)));
    }
}
