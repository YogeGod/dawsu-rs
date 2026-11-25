//! osu!standard Chord & Polyphony Resolver (TDD Non-Goal: NO storyboard `.osb` samples).
//!
//! osu!standard plays one custom sample per hit object. When two or more unmuted keysound notes
//! land within `tolerance_ms` of the same hit object, this module detects the chord conflict and
//! resolves it deterministically via either:
//! - [`ChordResolution::FlattenPolyphonic`] (recommended): groups the chord into one deduplicated
//!   polyphonic sample.
//! - [`ChordResolution::TopNoteOnly`]: keeps the highest-pitched melody note and mutes lower voices.

use crate::model::KeysoundDocument;
use dawsu_osu_format::OsuFile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Deterministic chord resolution policy for osu!standard hit circles.
/// Intentionally omits any storyboard `.osb` policy (`tdd-keysounding.md:42`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChordResolution {
    #[default]
    FlattenPolyphonic,
    TopNoteOnly,
}

/// One detected polyphonic chord landing on a single osu!standard hit object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChordConflict {
    pub time_ms: f64,
    pub object_index: usize,
    pub track_id: u64,
    pub note_indices: Vec<usize>,
    pub pitches: Vec<u8>,
    pub suggested_name: String,
}

fn pitch_name(pitch: u8) -> String {
    const NAMES: [&str; 12] = ["c", "cs", "d", "ds", "e", "f", "fs", "g", "gs", "a", "as", "b"];
    let octave = (pitch as i32 / 12) - 1;
    format!("{}{}", NAMES[(pitch % 12) as usize], octave)
}

/// Formats a deterministic voicing key for chord deduplication (e.g. `"chord_c5_e5_g5"`).
pub fn chord_voicing_key(pitches: &[u8]) -> String {
    let mut sorted = pitches.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let parts: Vec<String> = sorted.into_iter().map(pitch_name).collect();
    format!("chord_{}", parts.join("_"))
}

/// Detects every beatmap slot where more unmuted notes land within `tolerance_ms` than there are
/// simultaneous playable hit-object slots available (e.g. 2+ voices on one osu!standard hit circle,
/// or 4 voices on a 2-column osu!mania chord — never flagging playable multi-column mania chords).
pub fn detect_chords(doc: &KeysoundDocument, osu: &OsuFile, tolerance_ms: f64) -> Vec<ChordConflict> {
    let tol = tolerance_ms.max(0.0);
    let slots = crate::generate::extract_map_slots(osu);
    if slots.is_empty() {
        return Vec::new();
    }
    let mut sorted_slots: Vec<(usize, f64)> = slots
        .iter()
        .enumerate()
        .map(|(i, slot)| (i, slot.time_ms))
        .collect();
    sorted_slots.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    let mut conflicts = Vec::new();
    for track in &doc.tracks {
        if track.muted {
            continue;
        }
        // Match unmuted notes (sorted by time, then highest pitch first) against free slots,
        // mirroring `generate::preview_sync`.
        let mut notes: Vec<(usize, f64, u8)> = track
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.muted)
            .map(|(idx, n)| (idx, n.start_ms, n.pitch))
            .collect();
        notes.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.2.cmp(&a.2)).then_with(|| a.0.cmp(&b.0)));

        let mut used_slots = vec![false; sorted_slots.len()];
        // Map each claimed sorted-slot position to the notes assigned to it (primary + overflow).
        let mut slot_groups: BTreeMap<usize, Vec<(usize, u8)>> = BTreeMap::new();

        for (note_idx, start_ms, pitch) in notes {
            let lo = sorted_slots.partition_point(|&(_, t)| t < start_ms - tol);
            let mut best_free: Option<(usize, f64)> = None;
            let mut best_any: Option<(usize, f64)> = None;
            for pos in lo..sorted_slots.len() {
                let (_, slot_time) = sorted_slots[pos];
                if slot_time > start_ms + tol {
                    break;
                }
                let dist = (slot_time - start_ms).abs();
                if best_any.is_none_or(|(_, d)| dist < d) {
                    best_any = Some((pos, dist));
                }
                if !used_slots[pos] && best_free.is_none_or(|(_, d)| dist < d) {
                    best_free = Some((pos, dist));
                }
            }
            if let Some((pos, _)) = best_free {
                used_slots[pos] = true;
                slot_groups.entry(pos).or_default().push((note_idx, pitch));
            } else if let Some((pos, _)) = best_any {
                // No free slot remains within tolerance: attach overflow voice to the last claimed
                // slot in this timestamp window so earlier playable mania columns remain untouched.
                let mut overflow_pos = pos;
                for cand in lo..sorted_slots.len() {
                    let (_, slot_time) = sorted_slots[cand];
                    if slot_time > start_ms + tol {
                        break;
                    }
                    if used_slots[cand] && (slot_time - sorted_slots[pos].1).abs() <= tol {
                        overflow_pos = cand;
                    }
                }
                slot_groups.entry(overflow_pos).or_default().push((note_idx, pitch));
            }
        }

        for (pos, group) in slot_groups {
            if group.len() < 2 {
                continue;
            }
            let (obj_idx, time_ms) = sorted_slots[pos];
            let note_indices: Vec<usize> = group.iter().map(|&(i, _)| i).collect();
            let mut pitches: Vec<u8> = group.iter().map(|&(_, p)| p).collect();
            pitches.sort_unstable();
            let suggested_name = chord_voicing_key(&pitches);
            conflicts.push(ChordConflict {
                time_ms,
                object_index: obj_idx,
                track_id: track.id,
                note_indices,
                pitches,
                suggested_name,
            });
        }
    }
    conflicts.sort_by(|a, b| {
        a.time_ms
            .total_cmp(&b.time_ms)
            .then_with(|| a.track_id.cmp(&b.track_id))
    });
    conflicts
}

/// Applies `resolution` to one detected chord conflict.
pub fn resolve_chord(
    doc: &mut KeysoundDocument,
    conflict: &ChordConflict,
    resolution: ChordResolution,
) -> bool {
    let Some(track) = doc.tracks.iter_mut().find(|t| t.id == conflict.track_id) else {
        return false;
    };
    if conflict.note_indices.len() < 2 {
        return false;
    }
    match resolution {
        ChordResolution::TopNoteOnly => {
            let top_idx = conflict
                .note_indices
                .iter()
                .copied()
                .filter_map(|idx| track.notes.get(idx).map(|n| (idx, n.pitch)))
                .max_by_key(|&(_, pitch)| pitch)
                .map(|(idx, _)| idx);
            let Some(keep_idx) = top_idx else { return false };
            for &idx in &conflict.note_indices {
                if idx != keep_idx
                    && let Some(note) = track.notes.get_mut(idx)
                {
                    note.muted = true;
                }
            }
            true
        }
        ChordResolution::FlattenPolyphonic => {
            // Deterministic hash of voicing key so identical chords share one split_group ID.
            let mut hash: u64 = 14695981039346656037;
            for b in conflict.suggested_name.as_bytes() {
                hash ^= u64::from(*b);
                hash = hash.wrapping_mul(1099511628211);
            }
            let group_id = hash.max(1);
            let top_idx = conflict
                .note_indices
                .iter()
                .copied()
                .filter_map(|idx| track.notes.get(idx).map(|n| (idx, n.pitch)))
                .max_by_key(|&(_, pitch)| pitch)
                .map(|(idx, _)| idx);
            for &idx in &conflict.note_indices {
                if let Some(note) = track.notes.get_mut(idx) {
                    note.split_group = Some(group_id);
                    // Non-anchor voices stay muted for single-slot osu!standard generation while
                    // sharing `split_group` for polyphonic bounce rendering.
                    if Some(idx) != top_idx {
                        note.muted = true;
                    }
                }
            }
            true
        }
    }
}

/// Detects and resolves all chord conflicts in `doc` against `osu`, returning count resolved.
pub fn resolve_all_chords(
    doc: &mut KeysoundDocument,
    osu: &OsuFile,
    tolerance_ms: f64,
    resolution: ChordResolution,
) -> usize {
    let conflicts = detect_chords(doc, osu, tolerance_ms);
    let mut resolved = 0;
    for conflict in &conflicts {
        if resolve_chord(doc, conflict, resolution) {
            resolved += 1;
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeysoundNote, KeysoundTrack};

    #[test]
    fn detects_and_resolves_chords_without_storyboard() {
        let osu = OsuFile::read(
            b"osu file format v14\n[General]\nMode:0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n".to_vec(),
        )
        .file;
        let mut doc = KeysoundDocument::default();
        let mut track = KeysoundTrack::new(1, "Piano", 0, false);
        track.notes.push(KeysoundNote::new(1000.0, 1200.0, 60, 100));
        track.notes.push(KeysoundNote::new(1000.0, 1200.0, 64, 100));
        track.notes.push(KeysoundNote::new(1000.0, 1200.0, 67, 100));
        track.notes.push(KeysoundNote::new(2000.0, 2200.0, 60, 100));
        track.notes.push(KeysoundNote::new(2000.0, 2200.0, 64, 100));
        track.notes.push(KeysoundNote::new(2000.0, 2200.0, 67, 100));
        doc.tracks.push(track);

        let conflicts = detect_chords(&doc, &osu, 10.0);
        assert_eq!(conflicts.len(), 2);
        assert_eq!(conflicts[0].suggested_name, "chord_c4_e4_g4");
        assert_eq!(conflicts[0].suggested_name, conflicts[1].suggested_name);

        let count = resolve_all_chords(&mut doc, &osu, 10.0, ChordResolution::FlattenPolyphonic);
        assert_eq!(count, 2);
        assert_eq!(detect_chords(&doc, &osu, 10.0).len(), 0);
        // Both chords share the same deduplicated split_group ID.
        assert_eq!(doc.tracks[0].notes[0].split_group, doc.tracks[0].notes[3].split_group);
    }

    #[test]
    fn mania_simultaneous_columns_are_not_flagged_or_muted() {
        // 4K osu!mania beatmap with 3 simultaneous objects at 1000 ms and 2 at 2000 ms.
        let osu = OsuFile::read(
            b"osu file format v14\n[General]\nMode:3\n[Difficulty]\nCircleSize:4\n[HitObjects]\n\
              64,192,1000,1,0,0:0:0:0:\n\
              192,192,1000,1,0,0:0:0:0:\n\
              320,192,1000,1,0,0:0:0:0:\n\
              64,192,2000,1,0,0:0:0:0:\n\
              192,192,2000,1,0,0:0:0:0:\n"
                .to_vec(),
        )
        .file;
        let mut doc = KeysoundDocument::default();
        let mut track = KeysoundTrack::new(1, "Piano", 0, false);
        // 3 voices for 3 simultaneous mania columns -> 0 conflicts!
        track.notes.push(KeysoundNote::new(1000.0, 1200.0, 60, 100));
        track.notes.push(KeysoundNote::new(1000.0, 1200.0, 64, 100));
        track.notes.push(KeysoundNote::new(1000.0, 1200.0, 67, 100));
        // 4 voices for 2 simultaneous mania columns -> top voice keeps col 1, other 3 share col 2.
        track.notes.push(KeysoundNote::new(2000.0, 2200.0, 60, 100));
        track.notes.push(KeysoundNote::new(2000.0, 2200.0, 64, 100));
        track.notes.push(KeysoundNote::new(2000.0, 2200.0, 67, 100));
        track.notes.push(KeysoundNote::new(2000.0, 2200.0, 72, 100));
        doc.tracks.push(track);

        let conflicts = detect_chords(&doc, &osu, 10.0);
        assert_eq!(conflicts.len(), 1, "only the 2000 ms overflow should conflict");
        assert_eq!(conflicts[0].time_ms, 2000.0);
        assert!(!conflicts[0].pitches.contains(&72), "top playable mania voice stays independent");

        let resolved = resolve_all_chords(&mut doc, &osu, 10.0, ChordResolution::TopNoteOnly);
        assert_eq!(resolved, 1);
        // All 3 notes at 1000 ms remain unmuted; 2 playable voices remain unmuted at 2000 ms (72 and 67).
        let unmuted_at_1000: Vec<u8> = doc.tracks[0].notes.iter().filter(|n| n.start_ms == 1000.0 && !n.muted).map(|n| n.pitch).collect();
        let unmuted_at_2000: Vec<u8> = doc.tracks[0].notes.iter().filter(|n| n.start_ms == 2000.0 && !n.muted).map(|n| n.pitch).collect();
        assert_eq!(unmuted_at_1000, vec![60, 64, 67]);
        assert_eq!(unmuted_at_2000, vec![67, 72]);
    }
}
