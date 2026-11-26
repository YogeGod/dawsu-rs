//! Pure note-editing operations for keysounding (TDD Responsibility 6, Invariant 7).
//!
//! Every operation takes a slice of [`KeysoundNote`] and returns a new `Vec<KeysoundNote>`
//! without mutating its input.

use crate::model::KeysoundNote;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Musical scales supported for Piano Roll scale highlighting and fold-to-scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleKind {
    #[default]
    Chromatic,
    Major,
    NaturalMinor,
    HarmonicMinor,
    MelodicMinor,
    Dorian,
    Mixolydian,
    PentatonicMajor,
    PentatonicMinor,
    Blues,
}

impl ScaleKind {
    /// Parses a UI/JSON scale identifier into a [`ScaleKind`].
    pub fn from_id(id: &str) -> Self {
        match id.trim().to_ascii_lowercase().as_str() {
            "major" => Self::Major,
            "natural_minor" | "minor" => Self::NaturalMinor,
            "harmonic_minor" => Self::HarmonicMinor,
            "melodic_minor" => Self::MelodicMinor,
            "dorian" => Self::Dorian,
            "mixolydian" => Self::Mixolydian,
            "pentatonic_major" => Self::PentatonicMajor,
            "pentatonic_minor" => Self::PentatonicMinor,
            "blues" => Self::Blues,
            _ => Self::Chromatic,
        }
    }

    const fn intervals(self) -> &'static [u8] {
        match self {
            Self::Chromatic => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            Self::Major => &[0, 2, 4, 5, 7, 9, 11],
            Self::NaturalMinor => &[0, 2, 3, 5, 7, 8, 10],
            Self::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Self::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
            Self::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Self::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Self::PentatonicMajor => &[0, 2, 4, 7, 9],
            Self::PentatonicMinor => &[0, 3, 5, 7, 10],
            Self::Blues => &[0, 3, 5, 6, 7, 10],
        }
    }
}

/// Returns a 12-bit pitch-class mask (`bit pc` set iff `(root + interval) % 12 == pc`).
pub fn scale_pitch_mask(root: u8, scale: ScaleKind) -> u16 {
    let root_pc = root % 12;
    let mut mask = 0u16;
    for &interval in scale.intervals() {
        mask |= 1u16 << ((root_pc + interval) % 12);
    }
    mask
}

/// Returns `true` when MIDI `pitch` belongs to `scale` rooted at `root`.
pub fn is_pitch_in_scale(pitch: u8, root: u8, scale: ScaleKind) -> bool {
    let pc = pitch % 12;
    (scale_pitch_mask(root, scale) & (1u16 << pc)) != 0
}

/// Direction to spread simultaneous chord notes when strumming.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrumDirection {
    #[default]
    Up,
    Down,
    Alternate,
}

impl StrumDirection {
    /// Parses a UI/JSON direction identifier into a [`StrumDirection`].
    pub fn from_id(id: &str) -> Self {
        match id.trim().to_ascii_lowercase().as_str() {
            "down" => Self::Down,
            "alternate" => Self::Alternate,
            _ => Self::Up,
        }
    }
}

/// Splits the note at `index` at `split_ms` into two contiguous notes sharing a `split_group`.
///
/// If `index` is out of bounds or `split_ms` is not strictly inside `(note.start_ms, note.end_ms)`,
/// returns a clone of `notes` unchanged.
pub fn split_note_at(
    notes: &[KeysoundNote],
    index: usize,
    split_ms: f64,
    next_split_group: u64,
) -> Vec<KeysoundNote> {
    let Some(target) = notes.get(index) else {
        return notes.to_vec();
    };
    if !(split_ms > target.start_ms && split_ms < target.end_ms) {
        return notes.to_vec();
    }

    let group = target.split_group.unwrap_or(next_split_group);
    let mut left = target.clone();
    left.end_ms = split_ms;
    left.split_group = Some(group);

    let mut right = target.clone();
    right.start_ms = split_ms;
    right.split_group = Some(group);

    let mut out = Vec::with_capacity(notes.len() + 1);
    out.extend_from_slice(&notes[..index]);
    out.push(left);
    out.push(right);
    out.extend_from_slice(&notes[index + 1..]);
    out
}

/// Glues selected notes of the same pitch into a single spanning note per pitch.
///
/// For each pitch with at least two selected notes, the merged note spans from the earliest
/// `start_ms` to the latest `end_ms`, inheriting `velocity` and `muted` from the earliest note
/// and clearing `split_group`. Unselected notes and single-note selections remain unchanged.
pub fn glue_notes(notes: &[KeysoundNote], indices: &[usize]) -> Vec<KeysoundNote> {
    let selected: BTreeSet<usize> = indices.iter().copied().filter(|&i| i < notes.len()).collect();
    if selected.len() < 2 {
        return notes.to_vec();
    }

    let mut by_pitch: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for &idx in &selected {
        by_pitch.entry(notes[idx].pitch).or_default().push(idx);
    }

    let mut consumed: BTreeSet<usize> = BTreeSet::new();
    let mut merged: Vec<KeysoundNote> = Vec::new();

    for (_pitch, mut group) in by_pitch {
        if group.len() < 2 {
            continue;
        }
        group.sort_by(|&a, &b| {
            notes[a]
                .start_ms
                .total_cmp(&notes[b].start_ms)
                .then_with(|| notes[a].end_ms.total_cmp(&notes[b].end_ms))
        });
        let first = &notes[group[0]];
        let start_ms = first.start_ms;
        let end_ms = group
            .iter()
            .map(|&i| notes[i].end_ms)
            .fold(start_ms, f64::max);
        merged.push(KeysoundNote {
            start_ms,
            end_ms,
            pitch: first.pitch,
            velocity: first.velocity,
            muted: first.muted,
            split_group: None,
        });
        consumed.extend(group);
    }

    if consumed.is_empty() {
        return notes.to_vec();
    }

    let mut out: Vec<KeysoundNote> = notes
        .iter()
        .enumerate()
        .filter(|(i, _)| !consumed.contains(i))
        .map(|(_, n)| n.clone())
        .collect();
    out.extend(merged);
    sort_notes(&mut out);
    out
}

/// Quantizes the start time of selected notes (or all notes when `indices` is `None`) toward the
/// nearest grid line `origin_ms + k * grid_ms`, blended by `strength` in `0.0..=1.0`.
///
/// Each note's duration (`end_ms - start_ms`) is preserved.
pub fn quantize_notes(
    notes: &[KeysoundNote],
    indices: Option<&[usize]>,
    grid_ms: f64,
    origin_ms: f64,
    strength: f64,
) -> Vec<KeysoundNote> {
    if grid_ms <= 0.0 || !grid_ms.is_finite() {
        return notes.to_vec();
    }
    let strength = strength.clamp(0.0, 1.0);
    let selected: Option<BTreeSet<usize>> = indices.map(|s| s.iter().copied().collect());

    notes
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let apply = selected.as_ref().is_none_or(|s| s.contains(&i));
            if !apply {
                return note.clone();
            }
            let duration = note.duration_ms();
            let step = ((note.start_ms - origin_ms) / grid_ms).round();
            let snapped = origin_ms + step * grid_ms;
            let new_start = (note.start_ms + (snapped - note.start_ms) * strength).max(0.0);
            let mut updated = note.clone();
            updated.start_ms = new_start;
            updated.end_ms = new_start + duration;
            updated
        })
        .collect()
}

/// Transposes selected notes (or all notes when `indices` is `None`) by `semitones`,
/// clamping pitch to valid MIDI range `0..=127`.
pub fn transpose_notes(
    notes: &[KeysoundNote],
    indices: Option<&[usize]>,
    semitones: i8,
) -> Vec<KeysoundNote> {
    let selected: Option<BTreeSet<usize>> = indices.map(|s| s.iter().copied().collect());

    notes
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let apply = selected.as_ref().is_none_or(|s| s.contains(&i));
            if !apply {
                return note.clone();
            }
            let mut updated = note.clone();
            let shifted = (i16::from(note.pitch) + i16::from(semitones)).clamp(0, 127) as u8;
            updated.pitch = shifted;
            updated
        })
        .collect()
}

/// Trims overlapping notes of the same pitch so an earlier note ends no later than the start
/// of the next later note on the same pitch.
///
/// Notes starting at the exact same `start_ms` are left untouched (use [`deduplicate_notes`]
/// for simultaneous duplicates).
pub fn trim_overlaps(notes: &[KeysoundNote]) -> Vec<KeysoundNote> {
    let mut out = notes.to_vec();
    sort_notes(&mut out);

    let mut by_pitch: BTreeMap<u8, Vec<usize>> = BTreeMap::new();
    for (i, n) in out.iter().enumerate() {
        by_pitch.entry(n.pitch).or_default().push(i);
    }

    for indices in by_pitch.values() {
        for pair in indices.windows(2) {
            let cur = pair[0];
            let next = pair[1];
            let next_start = out[next].start_ms;
            if out[cur].start_ms < next_start && out[cur].end_ms > next_start {
                out[cur].end_ms = next_start;
            }
        }
    }

    out
}

/// Sets the instrument velocity of selected notes (or all notes when `indices` is `None`),
/// clamped to `1..=127` (TDD Responsibility 3a: modifies only note velocity).
pub fn set_velocity(
    notes: &[KeysoundNote],
    indices: Option<&[usize]>,
    velocity: u8,
) -> Vec<KeysoundNote> {
    let clamped = velocity.clamp(1, 127);
    let selected: Option<BTreeSet<usize>> = indices.map(|s| s.iter().copied().collect());

    notes
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let apply = selected.as_ref().is_none_or(|s| s.contains(&i));
            if !apply {
                return note.clone();
            }
            let mut updated = note.clone();
            updated.velocity = clamped;
            updated
        })
        .collect()
}

/// Removes duplicate notes on the same pitch whose `start_ms` are within `tolerance_ms`.
///
/// When two notes collide within `tolerance_ms`, keeps the louder note (or longer note on tie).
pub fn deduplicate_notes(notes: &[KeysoundNote], tolerance_ms: f64) -> Vec<KeysoundNote> {
    let tol = tolerance_ms.max(0.0);
    let mut sorted = notes.to_vec();
    sort_notes(&mut sorted);

    let mut kept: Vec<KeysoundNote> = Vec::with_capacity(sorted.len());
    for candidate in sorted {
        let duplicate_idx = kept.iter().rposition(|existing| {
            existing.pitch == candidate.pitch
                && (candidate.start_ms - existing.start_ms).abs() <= tol
        });
        match duplicate_idx {
            Some(idx) => {
                let existing = &kept[idx];
                let candidate_better = candidate.velocity > existing.velocity
                    || (candidate.velocity == existing.velocity
                        && candidate.duration_ms() > existing.duration_ms());
                if candidate_better {
                    kept[idx] = candidate;
                }
            }
            None => kept.push(candidate),
        }
    }

    sort_notes(&mut kept);
    kept
}

/// Strums simultaneous chord notes in `notes` (or only `indices` when `Some`), offsetting each
/// chord voice progressively by `spread_ms` while preserving note durations (`>= 20 ms`).
pub fn strum_notes(
    notes: &[KeysoundNote],
    indices: Option<&[usize]>,
    spread_ms: f64,
    direction: StrumDirection,
    chord_tolerance_ms: f64,
) -> Vec<KeysoundNote> {
    if !spread_ms.is_finite() || spread_ms <= 0.0 {
        return notes.to_vec();
    }
    let tol = chord_tolerance_ms.max(0.0);
    let selected: Option<BTreeSet<usize>> = indices.map(|s| s.iter().copied().collect());
    let mut target_indices: Vec<usize> = (0..notes.len())
        .filter(|i| selected.as_ref().is_none_or(|s| s.contains(i)))
        .collect();
    target_indices.sort_by(|&a, &b| {
        notes[a]
            .start_ms
            .total_cmp(&notes[b].start_ms)
            .then_with(|| notes[a].pitch.cmp(&notes[b].pitch))
            .then_with(|| a.cmp(&b))
    });

    let mut out = notes.to_vec();
    let mut cursor = 0usize;
    let mut chord_num = 0usize;
    while cursor < target_indices.len() {
        let anchor_start = out[target_indices[cursor]].start_ms;
        let mut end = cursor + 1;
        while end < target_indices.len()
            && (out[target_indices[end]].start_ms - anchor_start) <= tol
        {
            end += 1;
        }
        if end - cursor >= 2 {
            let mut chord: Vec<usize> = target_indices[cursor..end].to_vec();
            let descending = match direction {
                StrumDirection::Up => false,
                StrumDirection::Down => true,
                StrumDirection::Alternate => chord_num % 2 == 1,
            };
            chord.sort_by(|&a, &b| {
                if descending {
                    out[b].pitch.cmp(&out[a].pitch)
                } else {
                    out[a].pitch.cmp(&out[b].pitch)
                }
                .then_with(|| a.cmp(&b))
            });
            for (step, &idx) in chord.iter().enumerate() {
                let dur = out[idx].duration_ms().max(20.0);
                let new_start = (anchor_start + step as f64 * spread_ms).max(0.0);
                out[idx].start_ms = new_start;
                out[idx].end_ms = new_start + dur;
            }
            chord_num += 1;
        }
        cursor = end;
    }

    sort_notes(&mut out);
    out
}

/// Deterministic zero-dependency SplitMix64 step returning `[0.0, 1.0)`.
fn splitmix64_unit(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // Top 53 bits to uniform f64 in [0.0, 1.0).
    ((z >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}

/// Applies deterministic timing (`+-timing_ms`) and velocity (`+-velocity_range`) humanisation
/// to selected notes (or all notes when `indices` is `None`), preserving note durations.
pub fn humanise_notes(
    notes: &[KeysoundNote],
    indices: Option<&[usize]>,
    timing_ms: f64,
    velocity_range: u8,
    seed: u64,
) -> Vec<KeysoundNote> {
    let timing = if timing_ms.is_finite() && timing_ms > 0.0 {
        timing_ms
    } else {
        0.0
    };
    let selected: Option<BTreeSet<usize>> = indices.map(|s| s.iter().copied().collect());
    let mut rng_state = seed;
    let mut out = Vec::with_capacity(notes.len());

    for (i, note) in notes.iter().enumerate() {
        let apply = selected.as_ref().is_none_or(|s| s.contains(&i));
        if !apply {
            out.push(note.clone());
            continue;
        }
        let mut updated = note.clone();
        if timing > 0.0 {
            let u = splitmix64_unit(&mut rng_state);
            let jitter = (u * 2.0 - 1.0) * timing;
            let dur = note.duration_ms().max(20.0);
            updated.start_ms = (note.start_ms + jitter).max(0.0);
            updated.end_ms = updated.start_ms + dur;
        }
        if velocity_range > 0 {
            let u = splitmix64_unit(&mut rng_state);
            let span = i16::from(velocity_range) * 2 + 1;
            let delta = ((u * f64::from(span)).floor() as i16) - i16::from(velocity_range);
            updated.velocity = (i16::from(note.velocity) + delta).clamp(1, 127) as u8;
        }
        out.push(updated);
    }

    sort_notes(&mut out);
    out
}

/// Interpolates velocities of notes whose `start_ms` falls within `[min(start_ms, end_ms), max(start_ms, end_ms)]`
/// using an exponential ramp controlled by `curve` (`0.0` linear, `> 0` ease-in, `< 0` ease-out).
pub fn apply_velocity_ramp(
    notes: &[KeysoundNote],
    start_ms: f64,
    end_ms: f64,
    start_vel: u8,
    end_vel: u8,
    curve: f64,
) -> Vec<KeysoundNote> {
    if !start_ms.is_finite() || !end_ms.is_finite() {
        return notes.to_vec();
    }
    let (t0, t1, v0, v1) = if start_ms <= end_ms {
        (start_ms, end_ms, start_vel.clamp(1, 127), end_vel.clamp(1, 127))
    } else {
        (end_ms, start_ms, end_vel.clamp(1, 127), start_vel.clamp(1, 127))
    };
    let clamped_curve = if curve.is_finite() {
        curve.clamp(-4.0, 4.0)
    } else {
        0.0
    };
    let exponent = 2.0_f64.powf(clamped_curve);
    let span = t1 - t0;

    notes
        .iter()
        .map(|note| {
            if note.start_ms < t0 || note.start_ms > t1 {
                return note.clone();
            }
            let t = if span.abs() < 1e-6 {
                1.0
            } else {
                ((note.start_ms - t0) / span).clamp(0.0, 1.0)
            };
            let shaped = t.powf(exponent);
            let vel = (f64::from(v0) + (f64::from(v1) - f64::from(v0)) * shaped)
                .round()
                .clamp(1.0, 127.0) as u8;
            let mut updated = note.clone();
            updated.velocity = vel;
            updated
        })
        .collect()
}

fn sort_notes(notes: &mut [KeysoundNote]) {
    notes.sort_by(|a, b| {
        a.start_ms
            .total_cmp(&b.start_ms)
            .then_with(|| a.pitch.cmp(&b.pitch))
            .then_with(|| a.end_ms.total_cmp(&b.end_ms))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start_ms: f64, end_ms: f64, pitch: u8, velocity: u8) -> KeysoundNote {
        KeysoundNote::new(start_ms, end_ms, pitch, velocity)
    }

    #[test]
    fn split_note_at_creates_two_grouped_notes_and_ignores_out_of_range() {
        let notes = vec![note(100.0, 500.0, 60, 100)];
        let split = split_note_at(&notes, 0, 250.0, 42);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].start_ms, 100.0);
        assert_eq!(split[0].end_ms, 250.0);
        assert_eq!(split[0].split_group, Some(42));
        assert_eq!(split[1].start_ms, 250.0);
        assert_eq!(split[1].end_ms, 500.0);
        assert_eq!(split[1].split_group, Some(42));

        // Splitting at boundary leaves input unchanged.
        assert_eq!(split_note_at(&notes, 0, 100.0, 42), notes);
        assert_eq!(split_note_at(&notes, 0, 500.0, 42), notes);
        assert_eq!(split_note_at(&notes, 9, 250.0, 42), notes);
    }

    #[test]
    fn glue_notes_merges_same_pitch_and_leaves_other_pitches() {
        let notes = vec![
            note(0.0, 200.0, 60, 90),
            note(250.0, 500.0, 60, 70),
            note(100.0, 300.0, 64, 80),
        ];
        let glued = glue_notes(&notes, &[0, 1, 2]);
        assert_eq!(glued.len(), 2);
        assert_eq!(glued[0].pitch, 60);
        assert_eq!(glued[0].start_ms, 0.0);
        assert_eq!(glued[0].end_ms, 500.0);
        assert_eq!(glued[0].velocity, 90);
        assert_eq!(glued[1].pitch, 64);
    }

    #[test]
    fn quantize_notes_snaps_start_and_preserves_duration() {
        let notes = vec![note(112.0, 312.0, 60, 100), note(240.0, 340.0, 62, 100)];
        let full = quantize_notes(&notes, None, 125.0, 0.0, 1.0);
        assert!((full[0].start_ms - 125.0).abs() < 1e-6);
        assert!((full[0].end_ms - 325.0).abs() < 1e-6);
        assert!((full[1].start_ms - 250.0).abs() < 1e-6);
        assert!((full[1].end_ms - 350.0).abs() < 1e-6);

        // Partial strength (50%) on index 0 only.
        let half = quantize_notes(&notes, Some(&[0]), 125.0, 0.0, 0.5);
        assert!((half[0].start_ms - 118.5).abs() < 1e-6);
        assert_eq!(half[1], notes[1]);
    }

    #[test]
    fn transpose_notes_shifts_and_clamps_to_midi_bounds() {
        let notes = vec![note(0.0, 100.0, 60, 100), note(100.0, 200.0, 125, 100), note(200.0, 300.0, 2, 100)];
        let up = transpose_notes(&notes, None, 12);
        assert_eq!(up[0].pitch, 72);
        assert_eq!(up[1].pitch, 127);

        let down = transpose_notes(&notes, Some(&[2]), -12);
        assert_eq!(down[0].pitch, 60);
        assert_eq!(down[2].pitch, 0);
    }

    #[test]
    fn trim_overlaps_shortens_overlapping_same_pitch_only() {
        let notes = vec![
            note(0.0, 400.0, 60, 100),
            note(200.0, 500.0, 60, 100),
            note(100.0, 450.0, 64, 100),
        ];
        let trimmed = trim_overlaps(&notes);
        assert_eq!(trimmed.len(), 3);
        let c4_first = trimmed.iter().find(|n| n.pitch == 60 && n.start_ms == 0.0).unwrap();
        assert_eq!(c4_first.end_ms, 200.0);
        let e4 = trimmed.iter().find(|n| n.pitch == 64).unwrap();
        assert_eq!(e4.end_ms, 450.0);
    }

    #[test]
    fn set_velocity_clamps_and_targets_selection() {
        let notes = vec![note(0.0, 100.0, 60, 80), note(100.0, 200.0, 62, 80)];
        let edited = set_velocity(&notes, Some(&[0]), 0);
        assert_eq!(edited[0].velocity, 1);
        assert_eq!(edited[1].velocity, 80);

        let all_max = set_velocity(&notes, None, 200);
        assert_eq!(all_max[0].velocity, 127);
        assert_eq!(all_max[1].velocity, 127);
    }

    #[test]
    fn deduplicate_notes_keeps_louder_within_tolerance() {
        let notes = vec![
            note(100.0, 300.0, 60, 70),
            note(103.0, 250.0, 60, 110),
            note(103.0, 250.0, 64, 90),
            note(500.0, 700.0, 60, 80),
        ];
        let deduped = deduplicate_notes(&notes, 5.0);
        assert_eq!(deduped.len(), 3);
        assert_eq!(deduped[0].pitch, 60);
        assert_eq!(deduped[0].velocity, 110);
        assert_eq!(deduped[1].pitch, 64);
        assert_eq!(deduped[2].start_ms, 500.0);
    }

    #[test]
    fn scale_pitch_mask_identifies_diatonic_and_pentatonic_pitches() {
        let c_major = scale_pitch_mask(0, ScaleKind::Major);
        // C major: 7 pitch classes
        assert_eq!(c_major.count_ones(), 7);
        assert!(is_pitch_in_scale(60, 0, ScaleKind::Major)); // C4
        assert!(!is_pitch_in_scale(61, 0, ScaleKind::Major)); // C#4
        assert!(is_pitch_in_scale(64, 0, ScaleKind::Major)); // E4
        // D minor pentatonic: D(2), F(5), G(7), A(9), C(0)
        assert!(is_pitch_in_scale(62, 2, ScaleKind::PentatonicMinor));
        assert!(is_pitch_in_scale(60, 2, ScaleKind::PentatonicMinor));
        assert!(!is_pitch_in_scale(64, 2, ScaleKind::PentatonicMinor));
    }

    #[test]
    fn strum_notes_offsets_chords_and_preserves_durations() {
        let notes = vec![
            note(100.0, 300.0, 60, 100),
            note(102.0, 302.0, 64, 100),
            note(104.0, 304.0, 67, 100),
            note(500.0, 700.0, 60, 100),
            note(500.0, 700.0, 67, 100),
            note(900.0, 1000.0, 72, 100), // single note untouched
        ];
        let up = strum_notes(&notes, None, 15.0, StrumDirection::Up, 10.0);
        assert_eq!(up[0].start_ms, 100.0);
        assert_eq!(up[0].pitch, 60);
        assert_eq!(up[1].start_ms, 115.0);
        assert_eq!(up[1].pitch, 64);
        assert_eq!(up[1].duration_ms(), 200.0);
        assert_eq!(up[2].start_ms, 130.0);
        assert_eq!(up[2].pitch, 67);
        assert_eq!(up[5].start_ms, 900.0);

        let alt = strum_notes(&notes, None, 10.0, StrumDirection::Alternate, 10.0);
        // Second chord alternates to Down (pitch 67 first at 500, pitch 60 second at 510).
        let second_chord: Vec<_> = alt.iter().filter(|n| n.start_ms >= 500.0 && n.start_ms < 600.0).collect();
        assert_eq!(second_chord[0].pitch, 67);
        assert_eq!(second_chord[0].start_ms, 500.0);
        assert_eq!(second_chord[1].pitch, 60);
        assert_eq!(second_chord[1].start_ms, 510.0);
    }

    #[test]
    fn humanise_notes_is_deterministic_bounded_and_preserves_duration() {
        let notes = vec![
            note(10.0, 210.0, 60, 100),
            note(200.0, 400.0, 64, 5),
            note(400.0, 600.0, 67, 125),
        ];
        let h1 = humanise_notes(&notes, None, 20.0, 15, 42);
        let h2 = humanise_notes(&notes, None, 20.0, 15, 42);
        assert_eq!(h1, h2);
        for orig in &notes {
            let matching = h1.iter().find(|n| n.pitch == orig.pitch).unwrap();
            assert!((matching.duration_ms() - 200.0).abs() < 1e-6);
            assert!(matching.start_ms >= 0.0);
            assert!(matching.velocity >= 1 && matching.velocity <= 127);
        }
    }

    #[test]
    fn apply_velocity_ramp_interpolates_linear_curved_and_reverse_drags() {
        let notes = vec![
            note(0.0, 100.0, 60, 50),
            note(500.0, 600.0, 62, 50),
            note(1000.0, 1100.0, 64, 50),
            note(1500.0, 1600.0, 67, 99),
        ];
        let linear = apply_velocity_ramp(&notes, 0.0, 1000.0, 20, 120, 0.0);
        assert_eq!(linear[0].velocity, 20);
        assert_eq!(linear[1].velocity, 70);
        assert_eq!(linear[2].velocity, 120);
        assert_eq!(linear[3].velocity, 99, "outside range untouched");

        let ease_in = apply_velocity_ramp(&notes, 0.0, 1000.0, 20, 120, 1.0);
        assert_eq!(ease_in[1].velocity, 45); // t^2 at 0.5 -> 0.25 * 100 + 20

        let reverse = apply_velocity_ramp(&notes, 1000.0, 0.0, 120, 20, 0.0);
        assert_eq!(reverse[0].velocity, 20);
        assert_eq!(reverse[1].velocity, 70);
        assert_eq!(reverse[2].velocity, 120);
    }
}
