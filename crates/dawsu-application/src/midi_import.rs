//! Standard MIDI File (SMF) inspection and import into the active hitsound document.
//!
//! Parses `.mid` / `.midi` files via `dawsu_foundation::midi`, maps MIDI pitches to
//! hitsound lanes via `dawsu_hitsounding::drums::render_notes` (with Middle-C 60+ and
//! active-lane fallbacks), and applies all imported hits via [`Composer::add_batch`]
//! as a single undo step (`"Import MIDI"`).

use dawsu_editor::tempo::{TempoMap, TimingSection};
use dawsu_editor_composer::{BatchHit, BatchOptions, BatchRefusal, Composer, Policy};
use dawsu_foundation::midi::{MidiFile, TimeDivision, TimedMidi};
use dawsu_foundation::{Hitsound, SampleIdentity, Sound};
use dawsu_hitsounding::drums::{DrumMap, render_notes};
use dawsu_hitsounding::Lane;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const IMPORT_UNDO_LABEL: &str = "Import MIDI";
const MAX_MIDI_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// Summary of one MIDI track that contains at least one note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiTrackSummary {
    pub index: usize,
    pub channel: u8,
    pub name: String,
    pub is_drum: bool,
    pub note_count: usize,
}

/// Inspection result returned before importing a MIDI file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiInspectResult {
    pub path: String,
    pub name: String,
    pub bpm: f64,
    pub tracks: Vec<MidiTrackSummary>,
    pub note_count: usize,
    pub mapped_count: usize,
}

/// User-selected options for importing a MIDI file into the active document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiImportOptions {
    pub path: String,
    pub tracks: Vec<usize>,
    pub use_beatmap_tempo: bool,
    pub offset_ms: f64,
    pub snap: bool,
    pub follow_volume: bool,
}

/// Result of applying a MIDI import to the active document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiImportOutcome {
    pub added: usize,
    pub skipped: usize,
}

/// Reads and inspects a MIDI file against the active document's lanes and optional drum map.
pub fn inspect_midi_file(
    path: &Path,
    lanes: &[Lane],
    drum_map: Option<&DrumMap>,
) -> Result<MidiInspectResult, String> {
    let bytes = read_midi_bytes(path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("midi.mid")
        .to_string();
    inspect_midi_bytes(&path.to_string_lossy(), &name, &bytes, lanes, drum_map)
}

/// Inspects raw SMF bytes against `lanes` and an optional `drum_map`.
pub fn inspect_midi_bytes(
    path: &str,
    name: &str,
    bytes: &[u8],
    lanes: &[Lane],
    drum_map: Option<&DrumMap>,
) -> Result<MidiInspectResult, String> {
    let midi = MidiFile::parse(bytes).map_err(|e| format!("Could not parse MIDI file: {e}"))?;
    let timed = TimedMidi::from_midi(&midi);
    let bpm = timed.tempo_map.bpm_at_tick(0);
    let note_map = build_note_to_lane_map(lanes, drum_map);

    let mut tracks = Vec::new();
    let mut note_count = 0usize;
    let mut mapped_count = 0usize;

    for track in &timed.tracks {
        if track.notes.is_empty() {
            continue;
        }
        let channel = track.notes[0].channel;
        let is_drum = channel == 9 || track.notes.iter().any(|n| n.channel == 9);
        let track_name = track
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Track {}", track.index + 1));
        note_count += track.notes.len();
        mapped_count += track
            .notes
            .iter()
            .filter(|n| note_map.contains_key(&n.pitch))
            .count();
        tracks.push(MidiTrackSummary {
            index: track.index,
            channel,
            name: track_name,
            is_drum,
            note_count: track.notes.len(),
        });
    }

    Ok(MidiInspectResult {
        path: path.to_owned(),
        name: name.to_owned(),
        bpm,
        tracks,
        note_count,
        mapped_count,
    })
}

/// Reads the MIDI file in `options.path` and imports selected tracks into `composer`
/// as a single undo step (`"Import MIDI"`).
pub fn apply_midi_import(
    composer: &mut Composer,
    options: &MidiImportOptions,
    drum_map: Option<&DrumMap>,
) -> Result<MidiImportOutcome, String> {
    let bytes = read_midi_bytes(Path::new(&options.path))?;
    apply_midi_import_bytes(composer, &bytes, options, drum_map)
}

/// Imports selected tracks from raw SMF bytes into `composer` as a single undo step (`"Import MIDI"`).
pub fn apply_midi_import_bytes(
    composer: &mut Composer,
    bytes: &[u8],
    options: &MidiImportOptions,
    drum_map: Option<&DrumMap>,
) -> Result<MidiImportOutcome, String> {
    if options.tracks.is_empty() {
        return Err("Select at least one MIDI track to import.".into());
    }
    let midi = MidiFile::parse(bytes).map_err(|e| format!("Could not parse MIDI file: {e}"))?;
    let timed = TimedMidi::from_midi(&midi);

    let lanes = composer.hitsounds().lanes().to_vec();
    let note_map = build_note_to_lane_map(&lanes, drum_map);
    let ordered_lanes = ordered_lane_identities(composer);
    let fallback_lane = ordered_lanes.first().cloned();

    let selected: HashSet<usize> = options.tracks.iter().copied().collect();
    let offset_ms = if options.offset_ms.is_finite() {
        options.offset_ms
    } else {
        0.0
    };
    let divisor = composer.spacing.divisions().max(1);

    let mut batch_hits = Vec::new();
    let mut skipped = 0usize;

    for track in &timed.tracks {
        if !selected.contains(&track.index) {
            continue;
        }
        for note in &track.notes {
            let Some(lane) = resolve_note_lane(
                note.pitch,
                &note_map,
                &ordered_lanes,
                fallback_lane.as_ref(),
            ) else {
                skipped += 1;
                continue;
            };

            let raw_time = if options.use_beatmap_tempo {
                note_time_on_beatmap(note.start_tick, note.start_ms, timed.division, &composer.tempo)
                    + offset_ms
            } else {
                note.start_ms + offset_ms
            };

            if !raw_time.is_finite() || raw_time < 0.0 {
                skipped += 1;
                continue;
            }

            let grid = if options.snap {
                let (line, _) = composer.grid_line(raw_time, divisor);
                Some(line)
            } else {
                None
            };

            let strength = if options.follow_volume {
                1.0
            } else {
                (f64::from(note.velocity) / 127.0).clamp(0.01, 1.0)
            };

            batch_hits.push(BatchHit {
                time: raw_time,
                lane,
                strength,
                grid,
            });
        }
    }

    if batch_hits.is_empty() {
        return Ok(MidiImportOutcome { added: 0, skipped });
    }

    let batch_options = if options.snap {
        BatchOptions {
            object_ms: 30.0,
            dedupe_ms: 2.0,
            shift_ms: 0.0,
            range: None,
            policy: Policy::Snap,
        }
    } else {
        BatchOptions {
            object_ms: 0.0,
            dedupe_ms: 2.0,
            shift_ms: 0.0,
            range: None,
            policy: Policy::Classic {
                filter_grid: false,
                include_unmatched: true,
            },
        }
    };

    let report = composer
        .add_batch(IMPORT_UNDO_LABEL, &batch_hits, &batch_options)
        .map_err(|refusal| match refusal {
            BatchRefusal::StepOpen => "Finish the current edit before importing MIDI.".to_owned(),
            BatchRefusal::MissingLane(_) => {
                "A target lane is no longer available in this difficulty.".to_owned()
            }
        })?;

    let unplaced = batch_hits.len().saturating_sub(report.counts.added);
    Ok(MidiImportOutcome {
        added: report.counts.added,
        skipped: skipped + unplaced,
    })
}

fn read_midi_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("Could not read MIDI file {}: {e}", path.display()))?;
    if meta.len() > MAX_MIDI_FILE_BYTES {
        return Err("MIDI file is larger than 32 MB.".into());
    }
    std::fs::read(path).map_err(|e| format!("Could not read MIDI file {}: {e}", path.display()))
}

/// Builds a MIDI pitch -> lane identity lookup table from [`render_notes`] (which combines
/// skin GM defaults with any active drum map), plus GM drum fallbacks for custom-file
/// standard hitsound lanes.
fn build_note_to_lane_map(
    lanes: &[Lane],
    drum_map: Option<&DrumMap>,
) -> HashMap<u8, SampleIdentity> {
    let lane_notes = render_notes(drum_map, lanes);
    let mut by_note: HashMap<u8, SampleIdentity> = HashMap::new();

    // Walk in `lanes` order so the first lane mapped to a pitch wins deterministically.
    for lane in lanes {
        if let Some(&note) = lane_notes.get(&lane.identity) {
            by_note.entry(note).or_insert_with(|| lane.identity.clone());
        }
    }

    // Secondary GM drum fallback for standard hitsound lanes backed by mapset files.
    let first_hit_lane = |wanted: Hitsound| {
        lanes.iter().find_map(|lane| match &lane.identity {
            SampleIdentity::Standard {
                sound: Sound::Hit(h),
                ..
            } if *h == wanted => Some(lane.identity.clone()),
            _ => None,
        })
    };

    if let Some(kick) = first_hit_lane(Hitsound::Normal) {
        for note in [35_u8, 36] {
            by_note.entry(note).or_insert_with(|| kick.clone());
        }
    }
    if let Some(snare) = first_hit_lane(Hitsound::Clap) {
        for note in [38_u8, 39, 40] {
            by_note.entry(note).or_insert_with(|| snare.clone());
        }
    }
    if let Some(hat) = first_hit_lane(Hitsound::Whistle) {
        for note in [42_u8, 44, 46, 51] {
            by_note.entry(note).or_insert_with(|| hat.clone());
        }
    }
    if let Some(crash) = first_hit_lane(Hitsound::Finish) {
        for note in [49_u8, 52, 55, 57] {
            by_note.entry(note).or_insert_with(|| crash.clone());
        }
    }

    by_note
}

fn ordered_lane_identities(composer: &Composer) -> Vec<SampleIdentity> {
    let model = composer.hitsounds();
    let mut out: Vec<SampleIdentity> = composer
        .lanes
        .iter()
        .filter(|id| model.lane(id).is_some())
        .cloned()
        .collect();
    if out.is_empty() {
        out = model.lanes().iter().map(|l| l.identity.clone()).collect();
    }
    out
}

fn resolve_note_lane(
    pitch: u8,
    note_map: &HashMap<u8, SampleIdentity>,
    ordered_lanes: &[SampleIdentity],
    fallback_lane: Option<&SampleIdentity>,
) -> Option<SampleIdentity> {
    if let Some(lane) = note_map.get(&pitch) {
        return Some(lane.clone());
    }
    if pitch >= 60 {
        let idx = usize::from(pitch - 60);
        if let Some(lane) = ordered_lanes.get(idx) {
            return Some(lane.clone());
        }
    }
    fallback_lane.cloned()
}

fn note_time_on_beatmap(
    start_tick: u64,
    start_ms: f64,
    division: TimeDivision,
    tempo: &TempoMap,
) -> f64 {
    match division {
        TimeDivision::TicksPerQuarter(tpq) => {
            let beats = (start_tick as f64) / f64::from(tpq);
            beats_to_beatmap_ms(tempo, beats)
        }
        TimeDivision::Smpte { .. } => start_ms,
    }
}

fn beats_to_beatmap_ms(tempo: &TempoMap, beats: f64) -> f64 {
    let sections = tempo.sections();
    if sections.is_empty() {
        return beats * TimingSection::DEFAULT.beat_length_ms;
    }
    let mut remaining = beats;
    for pair in sections.windows(2) {
        let cur = pair[0];
        let next = pair[1];
        let span_ms = (next.start.ms() - cur.start.ms()).max(0.0);
        let span_beats = span_ms / cur.beat_length_ms;
        if remaining <= span_beats {
            return cur.start.ms() + remaining * cur.beat_length_ms;
        }
        remaining -= span_beats;
    }
    let last = sections[sections.len() - 1];
    last.start.ms() + remaining * last.beat_length_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::SongTime;
    use dawsu_hitsounding::{Decompiled, Hitsounds, decompile};
    use dawsu_osu_format::OsuFile;

    fn push_vlq(out: &mut Vec<u8>, mut v: u32) {
        let mut buf = [0u8; 4];
        let mut n = 0;
        buf[n] = (v & 0x7F) as u8;
        n += 1;
        v >>= 7;
        while v > 0 {
            buf[n] = ((v & 0x7F) as u8) | 0x80;
            n += 1;
            v >>= 7;
        }
        for i in (0..n).rev() {
            out.push(buf[i]);
        }
    }

    fn build_track(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + body.len());
        out.extend_from_slice(b"MTrk");
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    fn build_smf_format1(tpq: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"MThd");
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes()); // Format 1
        out.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        out.extend_from_slice(&tpq.to_be_bytes());
        for t in tracks {
            out.extend_from_slice(&build_track(t));
        }
        out
    }

    fn synthesized_format1_midi() -> Vec<u8> {
        // Track 0: Tempo track at 120 BPM (500,000 us/quarter = 0x07A120), no notes.
        let mut t0 = Vec::new();
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xFF, 0x51, 3, 0x07, 0xA1, 0x20]);
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xFF, 0x2F, 0]);

        // Track 1: Drums on channel 9 (GM Ch 10): Kick (36) at beat 0, Snare (38) at beat 1, Hat (42) at beat 2.
        let mut t1 = Vec::new();
        push_vlq(&mut t1, 0);
        t1.extend_from_slice(&[0xFF, 0x03, 5]);
        t1.extend_from_slice(b"Drums");
        // Kick 36 at tick 0
        push_vlq(&mut t1, 0);
        t1.extend_from_slice(&[0x99, 36, 110]);
        push_vlq(&mut t1, 120);
        t1.extend_from_slice(&[0x89, 36, 0]);
        // Snare 38 at tick 480 (beat 1 = 500 ms at 120 BPM)
        push_vlq(&mut t1, 360);
        t1.extend_from_slice(&[0x99, 38, 100]);
        push_vlq(&mut t1, 120);
        t1.extend_from_slice(&[0x89, 38, 0]);
        // Closed Hi-Hat 42 at tick 960 (beat 2 = 1000 ms at 120 BPM)
        push_vlq(&mut t1, 360);
        t1.extend_from_slice(&[0x99, 42, 85]);
        push_vlq(&mut t1, 120);
        t1.extend_from_slice(&[0x89, 42, 0]);
        push_vlq(&mut t1, 0);
        t1.extend_from_slice(&[0xFF, 0x2F, 0]);

        // Track 2: Synth on channel 0: C4 (60) at beat 0, C#4 (61) at beat 1.
        let mut t2 = Vec::new();
        push_vlq(&mut t2, 0);
        t2.extend_from_slice(&[0xFF, 0x03, 5]);
        t2.extend_from_slice(b"Synth");
        push_vlq(&mut t2, 0);
        t2.extend_from_slice(&[0x90, 60, 95]);
        push_vlq(&mut t2, 240);
        t2.extend_from_slice(&[0x80, 60, 0]);
        push_vlq(&mut t2, 240);
        t2.extend_from_slice(&[0x90, 61, 90]);
        push_vlq(&mut t2, 240);
        t2.extend_from_slice(&[0x80, 61, 0]);
        push_vlq(&mut t2, 0);
        t2.extend_from_slice(&[0xFF, 0x2F, 0]);

        build_smf_format1(480, &[t0, t1, t2])
    }

    fn test_hitsounds() -> Hitsounds {
        let osu = b"osu file format v14\n\n[General]\nMode: 0\n\n[TimingPoints]\n1000,400,4,1,0,100,1,0\n\n[HitObjects]\n";
        let file = OsuFile::read(osu.to_vec()).file;
        match decompile(&file, &[], &|| false, &mut |_, _| {}) {
            Decompiled::Done(hs) => hs,
            _ => panic!("minimal osu file must decompile"),
        }
    }

    #[test]
    fn inspects_synthesized_format_1_midi() {
        let bytes = synthesized_format1_midi();
        let hs = test_hitsounds();
        let result =
            inspect_midi_bytes("song.mid", "song.mid", &bytes, hs.lanes(), None).expect("inspect");

        assert_eq!(result.name, "song.mid");
        assert!((result.bpm - 120.0).abs() < 1e-6);
        assert_eq!(result.tracks.len(), 2);
        assert_eq!(result.note_count, 5);
        // Track 1 has 3 GM drum notes (36, 38, 42) that match skin lanes; Track 2 has C4/C#4 (60, 61).
        assert_eq!(result.mapped_count, 3);

        assert_eq!(result.tracks[0].index, 1);
        assert_eq!(result.tracks[0].name, "Drums");
        assert_eq!(result.tracks[0].channel, 9);
        assert!(result.tracks[0].is_drum);
        assert_eq!(result.tracks[0].note_count, 3);

        assert_eq!(result.tracks[1].index, 2);
        assert_eq!(result.tracks[1].name, "Synth");
        assert_eq!(result.tracks[1].channel, 0);
        assert!(!result.tracks[1].is_drum);
        assert_eq!(result.tracks[1].note_count, 2);
    }

    #[test]
    fn imports_midi_notes_as_single_undo_step() {
        let bytes = synthesized_format1_midi();
        let hs = test_hitsounds();
        let mut composer = Composer::new(hs);
        // Beatmap at 150 BPM (400 ms per beat) starting at 1000 ms.
        composer.tempo = TempoMap::new(vec![TimingSection {
            start: SongTime::from_ms(1000.0),
            beat_length_ms: 400.0,
            beats_per_bar: 4,
        }]);

        let options = MidiImportOptions {
            path: "song.mid".into(),
            tracks: vec![1],
            use_beatmap_tempo: true,
            offset_ms: 0.0,
            snap: true,
            follow_volume: false,
        };

        let before_moments = composer.hitsounds().moments().len();
        let outcome =
            apply_midi_import_bytes(&mut composer, &bytes, &options, None).expect("import");

        assert_eq!(outcome.added, 3);
        assert_eq!(outcome.skipped, 0);
        assert_eq!(composer.history.steps(), vec!["Import MIDI"]);

        let placed_times: Vec<f64> = composer
            .hitsounds()
            .moments()
            .iter()
            .map(|m| m.time().ms())
            .collect();
        // Beats 0, 1, 2 on 150 BPM starting at 1000 ms -> 1000, 1400, 1800 ms.
        assert_eq!(placed_times.len(), 3);
        assert!((placed_times[0] - 1000.0).abs() < 1e-6);
        assert!((placed_times[1] - 1400.0).abs() < 1e-6);
        assert!((placed_times[2] - 1800.0).abs() < 1e-6);

        // Single undo restores the document completely.
        composer.undo();
        assert_eq!(composer.hitsounds().moments().len(), before_moments);
        assert!(!composer.history.can_undo());
    }
}
