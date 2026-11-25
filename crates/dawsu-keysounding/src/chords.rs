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

/// Detects every osu!standard hit object where 2+ unmuted notes on a track land within `tolerance_ms`.
/// Runs in $O(N \log M)$ via sorted binary search over object timestamps.
pub fn detect_chords(doc: &KeysoundDocument, osu: &OsuFile, tolerance_ms: f64) -> Vec<ChordConflict> {
    let tol = tolerance_ms.max(0.0);
    let mut object_times: Vec<(usize, f64)> = osu
        .hit_objects()
        .iter()
        .enumerate()
        .map(|(i, obj)| (i, obj.time()))
        .collect();
    let raw_times: Vec<f64> = object_times.iter().map(|&(_, t)| t).collect();
    object_times.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    let mut conflicts = Vec::new();
    for track in &doc.tracks {
        if track.muted {
            continue;
        }
        let mut by_object: BTreeMap<usize, Vec<(usize, u8)>> = BTreeMap::new();
        for (note_idx, note) in track.notes.iter().enumerate() {
            if note.muted {
                continue;
            }
            let lo = object_times.partition_point(|&(_, t)| t < note.start_ms - tol);
            let mut best: Option<(usize, f64)> = None;
            for &(obj_idx, obj_time) in &object_times[lo..] {
                if obj_time > note.start_ms + tol {
                    break;
                }
                let dist = (obj_time - note.start_ms).abs();
                if best.is_none_or(|(_, best_dist)| dist < best_dist) {
                    best = Some((obj_idx, dist));
                }
            }
            if let Some((obj_idx, _)) = best {
                by_object.entry(obj_idx).or_default().push((note_idx, note.pitch));
            }
        }
        for (obj_idx, group) in by_object {
            if group.len() < 2 {
                continue;
            }
            let time_ms = raw_times[obj_idx];
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
}
