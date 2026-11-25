//! Keysound compilation and `[Keysounds]` difficulty planning (TDD Responsibilities 8, 9, 11;
//! Invariants 3, 4, 5).
//!
//! Assigns each synced note a deterministic, bucketed custom sample filename, places it on the
//! matching osu!lazer-compatible hit object field (never using storyboard samples or custom
//! sample indices), and produces a deduplicated sample manifest for the instrument renderer.

use crate::chords::chord_voicing_key;
use crate::model::{ExportSettings, InstrumentSettings, KeysoundDocument, KeysoundNote, KeysoundTrack};
use dawsu_hitsounding::generate::file_name_for;
use dawsu_osu_format::{HitObject, HitSample, LineId, ObjectKind, OsuFile};
use std::collections::{BTreeMap, BTreeSet};

/// Default difficulty version name for generated keysound maps (TDD Responsibility 9).
pub const KEYSOUNDS_VERSION: &str = "Keysounds";

/// Where a keysound custom filename is placed in the `[Keysounds]` difficulty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyncTarget {
    Circle { line: LineId },
    /// A slider node (0 = head, 1.. = repeats and tail); custom filename goes in the longer
    /// `edgeSets` node entry (TDD Responsibility 9).
    SliderNode { line: LineId, node: usize },
    SpinnerEnd { line: LineId },
    /// An osu!mania hold note; keysound plays only at the head (TDD Responsibility 11).
    HoldHead { line: LineId, end_time_ms: i64 },
}

/// A candidate gameplay object position extracted from the reference beatmap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MapSlot {
    pub(crate) time_ms: f64,
    pub(crate) target: SyncTarget,
}

/// A single keysound note matched to a reference beatmap object.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncedNote {
    pub track_id: u64,
    pub note_index: usize,
    pub note_start_ms: f64,
    pub target_time_ms: f64,
    pub delta_ms: f64,
    pub target: SyncTarget,
    pub pitch: u8,
    pub velocity: u8,
    pub duration_ms: f64,
    pub filename: String,
}

/// A keysound note that could not be matched to any available reference beatmap object.
#[derive(Debug, Clone, PartialEq)]
pub struct UnsyncedNote {
    pub track_id: u64,
    pub note_index: usize,
    pub start_ms: f64,
    pub pitch: u8,
    pub velocity: u8,
    pub duration_ms: f64,
}

/// Preview of gameplay sync before writing the `[Keysounds]` difficulty (TDD Responsibility 8).
#[derive(Debug, Clone, PartialEq)]
pub struct SyncPreview {
    pub synced: Vec<SyncedNote>,
    pub unsynced: Vec<UnsyncedNote>,
}

/// One unique sample to be rendered by `dawsu-audio-instruments`.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleManifestEntry {
    pub filename: String,
    pub track_id: u64,
    pub instrument: InstrumentSettings,
    pub pitch: u8,
    /// All pitches rendered together for this sample (1 pitch for single notes, 2+ for flattened chords).
    pub chord_pitches: Vec<u8>,
    /// Representative quantized MIDI velocity (`1..=127`).
    pub velocity: u8,
    /// Representative quantized note duration in milliseconds.
    pub duration_ms: f64,
    /// How many synced notes reference this sample file.
    pub usage_count: usize,
}

/// The complete output of planning Generate keysounds.
#[derive(Debug, Clone)]
pub struct GeneratePlan {
    /// The new playable `[Keysounds]` difficulty (cloned and patched from the reference map;
    /// the reference map itself is never modified).
    pub file: OsuFile,
    /// Suggested `.osu` filename in the mapset folder.
    pub file_name: String,
    /// Sync details (one synced entry per placed note, plus any notes that had no free object).
    pub sync: SyncPreview,
    /// Deduplicated manifest of unique sample files to render.
    pub manifest: Vec<SampleManifestEntry>,
}

/// Quantizes a MIDI velocity (`1..=127`) into `buckets` evenly spaced representative velocities.
pub fn quantize_velocity(velocity: u8, buckets: u8) -> u8 {
    let v = velocity.clamp(1, 127) as u32;
    let b = buckets.clamp(1, 127) as u32;
    if b >= 127 {
        return v as u8;
    }
    let bucket_idx = ((v - 1) * b) / 127;
    let rep = ((bucket_idx + 1) * 127).div_ceil(b);
    rep.clamp(1, 127) as u8
}

/// Quantizes a note duration in milliseconds to the nearest bucket multiple (minimum one step).
pub fn quantize_duration_ms(duration_ms: f64, step_ms: f64) -> u32 {
    let step = if step_ms.is_finite() && step_ms >= 1.0 { step_ms } else { 1.0 };
    let dur = if duration_ms.is_finite() && duration_ms > 0.0 { duration_ms } else { step };
    let steps = (dur / step).round().max(1.0);
    (steps * step).round().clamp(1.0, 99_999.0) as u32
}

/// Builds a deterministic, filesystem-safe custom sample filename for a flattened polyphonic chord.
pub fn chord_sample_filename(
    track_id: u64,
    pitches: &[u8],
    velocity: u8,
    duration_ms: f64,
    settings: &ExportSettings,
) -> (String, u8, u32) {
    let q_vel = quantize_velocity(velocity, settings.velocity_buckets);
    let q_dur = quantize_duration_ms(duration_ms, settings.duration_bucket_ms);
    let prefix = sanitize_token(&settings.sample_prefix, "ks");
    let ext = sanitize_token(&settings.sample_extension, "wav");
    let voicing = chord_voicing_key(pitches);
    let name = format!("{prefix}_t{track_id}_{voicing}_v{q_vel:03}_d{q_dur:05}.{ext}");
    (name, q_vel, q_dur)
}

/// Collects all pitches belonging to a flattened chord (`split_group`) on `track` near `note.start_ms`.
fn flattened_chord_pitches(
    track: &KeysoundTrack,
    note: &KeysoundNote,
    tolerance_ms: f64,
    flatten_chords: bool,
) -> Vec<u8> {
    if !flatten_chords {
        return vec![note.pitch];
    }
    let Some(gid) = note.split_group else {
        return vec![note.pitch];
    };
    let tol = tolerance_ms.max(0.0);
    let mut pitches: Vec<u8> = track
        .notes
        .iter()
        .filter(|n| n.split_group == Some(gid) && (n.start_ms - note.start_ms).abs() <= tol)
        .map(|n| n.pitch)
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    if pitches.is_empty() {
        vec![note.pitch]
    } else {
        pitches
    }
}

/// Builds a deterministic, filesystem-safe custom sample filename for a note's bucketed identity.
pub fn sample_filename(
    track_id: u64,
    pitch: u8,
    velocity: u8,
    duration_ms: f64,
    settings: &ExportSettings,
) -> (String, u8, u32) {
    let q_vel = quantize_velocity(velocity, settings.velocity_buckets);
    let q_dur = quantize_duration_ms(duration_ms, settings.duration_bucket_ms);
    let prefix = sanitize_token(&settings.sample_prefix, "ks");
    let ext = sanitize_token(&settings.sample_extension, "wav");
    let name = format!("{prefix}_t{track_id}_p{:03}_v{q_vel:03}_d{q_dur:05}.{ext}", pitch.min(127));
    (name, q_vel, q_dur)
}

fn sanitize_token(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// Computes gameplay sync between unmuted notes in `doc` and hit objects in `reference_map`,
/// returning a [`SyncPreview`] without modifying any beatmap.
pub fn preview_sync(
    doc: &KeysoundDocument,
    reference_map: &OsuFile,
    settings: &ExportSettings,
) -> SyncPreview {
    let slots = extract_map_slots(reference_map);
    let mut used_slots = vec![false; slots.len()];
    let tolerance = settings.sync_tolerance_ms.max(0.0);

    // Collect all unmuted notes from unmuted tracks in deterministic time order.
    let mut candidates: Vec<(&KeysoundTrack, usize, &KeysoundNote)> = Vec::new();
    let any_solo = doc.tracks.iter().any(|t| t.solo);
    for track in &doc.tracks {
        if track.muted || (any_solo && !track.solo) {
            continue;
        }
        for (note_index, note) in track.notes.iter().enumerate() {
            if !note.muted {
                candidates.push((track, note_index, note));
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.2.start_ms
            .total_cmp(&b.2.start_ms)
            .then_with(|| a.0.id.cmp(&b.0.id))
            .then_with(|| a.2.pitch.cmp(&b.2.pitch))
    });

    let mut synced = Vec::new();
    let mut unsynced = Vec::new();

    for (track, note_index, note) in candidates {
        let mut best: Option<(usize, f64)> = None;
        for (slot_idx, slot) in slots.iter().enumerate() {
            if used_slots[slot_idx] {
                continue;
            }
            let diff = (slot.time_ms - note.start_ms).abs();
            if diff <= tolerance && best.is_none_or(|(_, best_diff)| diff < best_diff) {
                best = Some((slot_idx, diff));
            }
        }

        match best {
            Some((slot_idx, _)) => {
                used_slots[slot_idx] = true;
                let slot = slots[slot_idx];
                let chord_pitches = flattened_chord_pitches(
                    track,
                    note,
                    settings.sync_tolerance_ms,
                    settings.flatten_chords,
                );
                let (filename, _, _) = if chord_pitches.len() > 1 {
                    chord_sample_filename(
                        track.id,
                        &chord_pitches,
                        note.velocity,
                        note.duration_ms(),
                        settings,
                    )
                } else {
                    sample_filename(track.id, note.pitch, note.velocity, note.duration_ms(), settings)
                };
                synced.push(SyncedNote {
                    track_id: track.id,
                    note_index,
                    note_start_ms: note.start_ms,
                    target_time_ms: slot.time_ms,
                    delta_ms: slot.time_ms - note.start_ms,
                    target: slot.target,
                    pitch: note.pitch,
                    velocity: note.velocity,
                    duration_ms: note.duration_ms(),
                    filename,
                });
            }
            None => {
                unsynced.push(UnsyncedNote {
                    track_id: track.id,
                    note_index,
                    start_ms: note.start_ms,
                    pitch: note.pitch,
                    velocity: note.velocity,
                    duration_ms: note.duration_ms(),
                });
            }
        }
    }

    SyncPreview { synced, unsynced }
}

/// Plans Generate keysounds: clones `reference_map` into a new `[Keysounds]` difficulty,
/// places custom sample filenames on synced hit objects (never using storyboard samples or
/// custom indices), and builds the deduplicated [`SampleManifestEntry`] list.
///
/// The `reference_map` is never mutated (TDD Invariant 5), and editor bookmarks in `doc`
/// are never written into the generated map (TDD Invariant 3).
pub fn plan_generate_keysounds(
    doc: &KeysoundDocument,
    reference_map: &OsuFile,
    reference_file_name: &str,
    settings: &ExportSettings,
) -> GeneratePlan {
    let sync = preview_sync(doc, reference_map, settings);

    let version = if settings.target_version.trim().is_empty() {
        KEYSOUNDS_VERSION
    } else {
        settings.target_version.trim()
    };

    let mut file = reference_map.clone();
    let _ = file.set_value("Metadata", "Version", version);
    let _ = file.set_value("Metadata", "BeatmapID", "0");
    let file_name = file_name_for(reference_map, reference_file_name, version);

    apply_sync_filenames(&mut file, &sync.synced);

    // Build deduplicated sample manifest keyed by filename.
    let tracks_by_id: BTreeMap<u64, &KeysoundTrack> = doc.tracks.iter().map(|t| (t.id, t)).collect();
    let mut manifest_map: BTreeMap<String, SampleManifestEntry> = BTreeMap::new();
    let mut seen_filenames: BTreeSet<String> = BTreeSet::new();

    for placed in &sync.synced {
        if let Some(entry) = manifest_map.get_mut(&placed.filename) {
            entry.usage_count += 1;
            continue;
        }
        let track_ref = tracks_by_id.get(&placed.track_id).copied();
        let chord_pitches = track_ref
            .and_then(|t| {
                t.notes.get(placed.note_index).map(|note| {
                    flattened_chord_pitches(
                        t,
                        note,
                        settings.sync_tolerance_ms,
                        settings.flatten_chords,
                    )
                })
            })
            .unwrap_or_else(|| vec![placed.pitch]);
        let (_, q_vel, q_dur) = if chord_pitches.len() > 1 {
            chord_sample_filename(
                placed.track_id,
                &chord_pitches,
                placed.velocity,
                placed.duration_ms,
                settings,
            )
        } else {
            sample_filename(placed.track_id, placed.pitch, placed.velocity, placed.duration_ms, settings)
        };
        let instrument = track_ref
            .map(|t| t.instrument.clone())
            .unwrap_or_default();
        seen_filenames.insert(placed.filename.clone());
        manifest_map.insert(
            placed.filename.clone(),
            SampleManifestEntry {
                filename: placed.filename.clone(),
                track_id: placed.track_id,
                instrument,
                pitch: placed.pitch,
                chord_pitches,
                velocity: q_vel,
                duration_ms: f64::from(q_dur),
                usage_count: 1,
            },
        );
    }

    let manifest = manifest_map.into_values().collect();
    GeneratePlan {
        file,
        file_name,
        sync,
        manifest,
    }
}

impl GeneratePlan {
    /// Updates generated hit object, sync, and manifest references after resolving filename
    /// collisions in the mapset folder.
    pub fn remap_sample_filenames(&mut self, renames: &BTreeMap<String, String>) {
        if renames.is_empty() {
            return;
        }
        for placed in &mut self.sync.synced {
            if let Some(updated) = renames.get(&placed.filename) {
                placed.filename = updated.clone();
            }
        }
        for entry in &mut self.manifest {
            if let Some(updated) = renames.get(&entry.filename) {
                entry.filename = updated.clone();
            }
        }
        apply_sync_filenames(&mut self.file, &self.sync.synced);
    }
}

fn apply_sync_filenames(file: &mut OsuFile, synced: &[SyncedNote]) {
    // Index existing hit objects by LineId for patching.
    let objects_by_line: BTreeMap<LineId, HitObject> = file
        .hit_objects()
        .into_iter()
        .map(|o| (o.line(), o))
        .collect();

    // Collect slider node updates grouped by slider LineId so multi-node sliders are written in one pass.
    let mut slider_node_filenames: BTreeMap<LineId, BTreeMap<usize, String>> = BTreeMap::new();

    for placed in synced {
        match placed.target {
            SyncTarget::Circle { line } => {
                if let Some(obj) = objects_by_line.get(&line) {
                    let sample_field = format_hit_sample(obj.hit_sample(), &placed.filename);
                    let _ = file.set_field(line, 5, &sample_field);
                }
            }
            SyncTarget::SliderNode { line, node } => {
                slider_node_filenames
                    .entry(line)
                    .or_default()
                    .insert(node, placed.filename.clone());
            }
            SyncTarget::SpinnerEnd { line } => {
                if let Some(obj) = objects_by_line.get(&line) {
                    let sample_field = format_hit_sample(obj.hit_sample(), &placed.filename);
                    let _ = file.set_field(line, 6, &sample_field);
                }
            }
            SyncTarget::HoldHead { line, end_time_ms } => {
                if let Some(obj) = objects_by_line.get(&line) {
                    let sample_field = format_hit_sample(obj.hit_sample(), &placed.filename);
                    let hold_field = format!("{end_time_ms}:{sample_field}");
                    let _ = file.set_field(line, 5, &hold_field);
                }
            }
        }
    }

    for (line, node_map) in slider_node_filenames {
        if let Some(obj) = objects_by_line.get(&line)
            && let Some(slider) = obj.slider()
        {
            let node_count = (slider.slides - 1).max(0) as usize + 2;
            let mut entries: Vec<String> = Vec::with_capacity(node_count);
            for node_idx in 0..node_count {
                let existing = slider
                    .edge_sets
                    .as_ref()
                    .and_then(|sets| sets.get(node_idx))
                    .and_then(|opt| opt.as_ref());
                if let Some(filename) = node_map.get(&node_idx) {
                    entries.push(format_hit_sample(existing, filename));
                } else if let Some(s) = existing {
                    entries.push(format_existing_sample(s));
                } else {
                    entries.push(String::new());
                }
            }
            let _ = file.set_field(line, 9, &entries.join("|"));
        }
    }
}

/// Formats `normalSet:additionSet:0:volume:filename`, keeping existing sets/volume while forcing
/// custom sample index to 0 (TDD Responsibility 9).
fn format_hit_sample(existing: Option<&HitSample>, filename: &str) -> String {
    match existing {
        Some(s) => format!("{}:{}:0:{}:{filename}", s.normal_set, s.addition_set, s.volume),
        None => format!("0:0:0:0:{filename}"),
    }
}

fn format_existing_sample(s: &HitSample) -> String {
    match &s.filename {
        Some(f) if !f.is_empty() => {
            format!("{}:{}:{}:{}:{f}", s.normal_set, s.addition_set, s.index, s.volume)
        }
        _ if s.index != 0 || s.volume != 0 => {
            format!("{}:{}:{}:{}:", s.normal_set, s.addition_set, s.index, s.volume)
        }
        _ => format!("{}:{}", s.normal_set, s.addition_set),
    }
}

/// Extracts syncable object times from `file` (circles, slider head/repeats/tail, spinner ends,
/// and mania hold note heads).
pub(crate) fn extract_map_slots(file: &OsuFile) -> Vec<MapSlot> {
    let slider_multiplier = file
        .value("Difficulty", "SliderMultiplier")
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(1.4);
    let timing_points = file.timing_points();

    let mut slots = Vec::new();
    for obj in file.hit_objects() {
        match obj.kind() {
            ObjectKind::Circle => {
                slots.push(MapSlot {
                    time_ms: obj.time(),
                    target: SyncTarget::Circle { line: obj.line() },
                });
            }
            ObjectKind::Slider(slider) => {
                let node_count = (slider.slides - 1).max(0) as usize + 2;
                let span_ms = slider_span_duration_ms(obj.time(), slider.length, slider_multiplier, &timing_points);
                for node in 0..node_count {
                    slots.push(MapSlot {
                        time_ms: obj.time() + (node as f64) * span_ms,
                        target: SyncTarget::SliderNode { line: obj.line(), node },
                    });
                }
            }
            ObjectKind::Spinner { end } => {
                slots.push(MapSlot {
                    time_ms: *end,
                    target: SyncTarget::SpinnerEnd { line: obj.line() },
                });
            }
            ObjectKind::Hold { end } => {
                // TDD Responsibility 11: mania hold note keysound plays only at the head.
                slots.push(MapSlot {
                    time_ms: obj.time(),
                    target: SyncTarget::HoldHead {
                        line: obj.line(),
                        end_time_ms: end.round() as i64,
                    },
                });
            }
        }
    }
    slots
}

fn slider_span_duration_ms(
    start_ms: f64,
    length: Option<f64>,
    slider_multiplier: f64,
    timing_points: &[dawsu_osu_format::TimingPoint],
) -> f64 {
    let Some(length) = length else {
        return 0.0;
    };
    let slider_multiplier = slider_multiplier.clamp(0.4, 3.6);
    let mut sorted: Vec<_> = timing_points.iter().collect();
    // Match osu!lazer (LegacyBeatmapDecoder.cs:581-598): sort by time, red before green on ties.
    sorted.sort_by(|a, b| {
        a.time()
            .total_cmp(&b.time())
            .then_with(|| b.uninherited().cmp(&a.uninherited()))
    });
    // Match osu!lazer (ControlPointInfo.cs:75): before first red line use first red line, else 1000 ms.
    let mut beat_length = sorted
        .iter()
        .find(|tp| tp.uninherited() && tp.beat_length() > 0.0 && tp.beat_length().is_finite())
        .map(|tp| tp.beat_length())
        .unwrap_or(1000.0);
    let mut sv_multiplier = 1.0_f64;

    for tp in sorted {
        if tp.time() > start_ms {
            break;
        }
        if tp.uninherited() {
            if tp.beat_length() > 0.0 && tp.beat_length().is_finite() {
                beat_length = tp.beat_length();
            }
            sv_multiplier = 1.0;
        } else if tp.beat_length() < 0.0 && tp.beat_length().is_finite() {
            sv_multiplier = (-100.0 / tp.beat_length()).clamp(0.1, 10.0);
        }
    }

    let velocity = slider_multiplier * 100.0 * sv_multiplier;
    if velocity <= 0.0 {
        0.0
    } else {
        (length / velocity) * beat_length
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Bookmark, KeysoundNote};

    const SAMPLE_OSU: &str = "\
osu file format v14

[General]
AudioFilename: audio.mp3
Mode: 0

[Metadata]
Title:Test Song
Artist:Test Artist
Creator:Mapper
Version:Normal
BeatmapID:12345

[Difficulty]
SliderMultiplier:1.4

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,1000,1,0,0:0:0:0:
256,192,1500,1,0,0:0:0:0:
256,192,2000,2,0,L|396:192,1,140
64,192,3000,128,0,3500:0:0:0:0:
";

    fn parse_sample_map() -> OsuFile {
        OsuFile::read(SAMPLE_OSU.as_bytes().to_vec()).file
    }

    #[test]
    fn generates_keysounds_difficulty_with_custom_filenames_and_deduplicated_manifest() {
        let reference = parse_sample_map();
        let original_bytes = reference.to_bytes();

        let mut doc = KeysoundDocument::default();
        let track_id = doc.add_track("Piano", 0, false);
        doc.tracks[0].instrument.program = Some(0);
        // Two notes with identical pitch/velocity/duration bucket -> must share 1 manifest entry.
        doc.tracks[0].notes.push(KeysoundNote::new(1000.0, 1250.0, 60, 100));
        doc.tracks[0].notes.push(KeysoundNote::new(1502.0, 1752.0, 60, 100));
        // Slider head (2000ms) and slider tail (2500ms, since length=140, SM=1.4, beat_length=500 -> span=500ms).
        doc.tracks[0].notes.push(KeysoundNote::new(2000.0, 2200.0, 64, 90));
        doc.tracks[0].notes.push(KeysoundNote::new(2500.0, 2700.0, 67, 90));
        // Mania hold note head (3000ms).
        doc.tracks[0].notes.push(KeysoundNote::new(3000.0, 3500.0, 72, 110));
        // Unsynced note far from any object.
        doc.tracks[0].notes.push(KeysoundNote::new(9000.0, 9200.0, 60, 100));
        // Editor bookmark that must never be written into the beatmap (Invariant 3).
        doc.bookmarks.push(Bookmark { time_ms: 1000.0, label: "Drop".into() });

        let settings = ExportSettings::default();
        let plan = plan_generate_keysounds(
            &doc,
            &reference,
            "Test Artist - Test Song (Mapper) [Normal].osu",
            &settings,
        );

        // Invariant 5: reference map is untouched.
        assert_eq!(reference.to_bytes(), original_bytes);

        // Generated map metadata & filename.
        assert_eq!(plan.file.value("Metadata", "Version").as_deref(), Some("Keysounds"));
        assert_eq!(plan.file.value("Metadata", "BeatmapID").as_deref(), Some("0"));
        assert_eq!(plan.file_name, "Test Artist - Test Song (Mapper) [Keysounds].osu");

        // Invariant 3: no bookmarks written to the generated map.
        assert!(plan.file.value("Editor", "Bookmarks").is_none());

        // Non-goal 1: no storyboard samples added.
        assert!(plan.file.storyboard_samples().is_empty());

        // Sync counts: 5 synced, 1 unsynced.
        assert_eq!(plan.sync.synced.len(), 5);
        assert_eq!(plan.sync.unsynced.len(), 1);
        assert_eq!(plan.sync.unsynced[0].start_ms, 9000.0);

        // Deduplicated manifest: 5 synced notes, 4 unique sample files (first two circles share one).
        assert_eq!(plan.manifest.len(), 4);
        let shared = plan
            .manifest
            .iter()
            .find(|e| e.pitch == 60)
            .expect("pitch 60 manifest entry");
        assert_eq!(shared.track_id, track_id);
        assert_eq!(shared.usage_count, 2);

        // Verify osu!lazer-compatible placement on parsed output objects.
        let objects = plan.file.hit_objects();
        assert_eq!(objects.len(), 4);

        // Circle 1 & 2 carry the same custom filename and custom index 0.
        let c0 = objects[0].hit_sample().expect("circle 0 sample");
        let c1 = objects[1].hit_sample().expect("circle 1 sample");
        assert_eq!(c0.index, 0);
        assert_eq!(c0.filename.as_deref(), Some(shared.filename.as_str()));
        assert_eq!(c1.filename.as_deref(), Some(shared.filename.as_str()));

        // Slider carries custom filenames in its longer edgeSets node entries.
        let slider = objects[2].slider().expect("slider");
        let edge_sets = slider.edge_sets.as_ref().expect("slider edge_sets");
        assert_eq!(edge_sets.len(), 2);
        let head = edge_sets[0].as_ref().expect("slider head entry");
        let tail = edge_sets[1].as_ref().expect("slider tail entry");
        assert_eq!(head.index, 0);
        assert!(head.filename.as_deref().is_some_and(|f| f.contains("_p064_")));
        assert!(tail.filename.as_deref().is_some_and(|f| f.contains("_p067_")));

        // Mania hold note carries custom filename on its head sample and preserves end time.
        assert_eq!(objects[3].hold_end(), Some(3500.0));
        let hold_sample = objects[3].hit_sample().expect("hold sample");
        assert_eq!(hold_sample.index, 0);
        assert!(hold_sample.filename.as_deref().is_some_and(|f| f.contains("_p072_")));
    }

    #[test]
    fn velocity_and_duration_bucketing_is_deterministic_and_bounded() {
        assert_eq!(quantize_velocity(1, 16), 8);
        assert_eq!(quantize_velocity(127, 16), 127);
        assert_eq!(quantize_duration_ms(12.0, 50.0), 50);
        assert_eq!(quantize_duration_ms(126.0, 50.0), 150);

        let settings = ExportSettings::default();
        let (f1, _, _) = sample_filename(1, 60, 100, 240.0, &settings);
        let (f2, _, _) = sample_filename(1, 60, 100, 260.0, &settings);
        assert_eq!(f1, f2);
        assert!(f1.ends_with(".wav"));
    }

    #[test]
    fn flatten_polyphonic_chord_generates_polyphonic_manifest_entry_distinct_from_top_note_only() {
        use crate::chords::{ChordResolution, resolve_all_chords};
        let reference = parse_sample_map();

        let make_doc = || {
            let mut doc = KeysoundDocument::default();
            doc.add_track("Piano", 0, false);
            doc.tracks[0].notes.push(KeysoundNote::new(1000.0, 1250.0, 60, 100));
            doc.tracks[0].notes.push(KeysoundNote::new(1000.0, 1250.0, 64, 100));
            doc.tracks[0].notes.push(KeysoundNote::new(1000.0, 1250.0, 67, 100));
            doc
        };

        let mut flat_doc = make_doc();
        assert_eq!(resolve_all_chords(&mut flat_doc, &reference, 10.0, ChordResolution::FlattenPolyphonic), 1);
        let flat_plan = plan_generate_keysounds(&flat_doc, &reference, "ref.osu", &ExportSettings::default());
        assert_eq!(flat_plan.manifest.len(), 1);
        assert_eq!(flat_plan.manifest[0].chord_pitches, vec![60, 64, 67]);
        assert!(flat_plan.manifest[0].filename.contains("chord_c4_e4_g4"));

        let mut top_doc = make_doc();
        assert_eq!(resolve_all_chords(&mut top_doc, &reference, 10.0, ChordResolution::TopNoteOnly), 1);
        let top_plan = plan_generate_keysounds(&top_doc, &reference, "ref.osu", &ExportSettings::default());
        assert_eq!(top_plan.manifest.len(), 1);
        assert_eq!(top_plan.manifest[0].chord_pitches, vec![67]);
        assert!(top_plan.manifest[0].filename.contains("_p067_"));
    }
}
