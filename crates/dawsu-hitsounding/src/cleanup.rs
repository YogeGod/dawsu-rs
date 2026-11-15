//! Green-line clean-up (hitsound domain design doc Responsibility 21), after the old app's
//! `GreenLineCleanup` at c81774e: a green line goes when its values change nothing against the
//! values in force before it. The values are read as lazer reads them (`lazer::control_points`):
//! the sample bank (a missing set field takes `[General] SampleSet`), the custom index, the volume
//! (clamped to 0-100), the slider velocity (clamped to 0.1-10, with whether ticks are made) and
//! kiai. Only its own call runs it; compile never does (invariant 5).

use dawsu_osu_format::OsuFile;

/// A clean-up's plan: the cleaned copy (unedited when nothing goes) and how many lines go.
#[derive(Debug, Clone)]
pub struct Cleanup {
    pub file: OsuFile,
    pub removed: usize,
}

/// Plans green-line clean-up. Lines are walked by time, then file order, so at one time the later
/// line is the one in force; a removed line never becomes the values in force. Red lines and lines
/// lazer cannot read are never removed.
pub fn clean_up_green_lines(file: &OsuFile) -> Cleanup {
    let default_bank = crate::lazer::control_points::default_sample_set(file);
    let default_volume = file.general("SampleVolume").and_then(|v| dawsu_osu_format::number::parse_int(&v).ok()).unwrap_or(100);
    let mut points = file.timing_points();
    points.sort_by(|a, b| a.time().total_cmp(&b.time()));
    let mut in_force: Option<Values> = None;
    let mut redundant = Vec::new();
    for tp in &points {
        let beat_length = tp.beat_length();
        let speed = if beat_length < 0.0 { 100.0 / -beat_length } else { 1.0 };
        let now = Values {
            bank: crate::lazer::control_points::bank_name(tp.sample_set().unwrap_or(default_bank)),
            index: tp.sample_index().unwrap_or(0),
            volume: tp.volume().unwrap_or(default_volume).clamp(0, 100),
            velocity: speed.clamp(0.1, 10.0),
            ticks: !beat_length.is_nan(),
            kiai: tp.effects().unwrap_or(0) & 1 != 0,
        };
        if !tp.uninherited() && in_force.as_ref() == Some(&now) {
            redundant.push(tp.line());
        } else {
            in_force = Some(now);
        }
    }
    let mut out = file.clone();
    for id in &redundant {
        // Never refused: each id is a green line of this file.
        let _ = out.remove_green_line(*id);
    }
    Cleanup { file: out, removed: redundant.len() }
}

#[derive(Debug, PartialEq)]
struct Values {
    bank: String,
    index: i32,
    volume: i32,
    velocity: f64,
    ticks: bool,
    kiai: bool,
}
