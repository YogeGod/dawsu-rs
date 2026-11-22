//! Pure note-editing operations for keysounding (TDD Responsibility 6, Invariant 7).
//!
//! Every operation takes a slice of [`KeysoundNote`] and returns a new `Vec<KeysoundNote>`
//! without mutating its input.

use crate::model::KeysoundNote;
use std::collections::{BTreeMap, BTreeSet};

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
}
