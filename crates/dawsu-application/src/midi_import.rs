//! Standard MIDI File (SMF) inspection and import into the active hitsound document.
//!
//! Parses `.mid` / `.midi` files via `dawsu_foundation::midi`, maps MIDI pitches to
//! hitsound lanes via `dawsu_hitsounding::drums::render_notes` (with Middle-C 60+ and
//! active-lane fallbacks), and applies all imported hits via [`Composer::add_batch`]
//! as a single undo step (`"Import MIDI"`).

use dawsu_editor::tempo::{TempoMap, TimingSection};
use dawsu_editor_composer::{BatchHit, BatchOptions, BatchRefusal, Composer, Policy};
use dawsu_foundation::midi::{
    MidiFile, MidiFormat, MidiNote, TempoMap as MidiTempoMap, TimeDivision, TimedMidi, TimedTrack,
};
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

fn lane_to_gm_pitch(identity: &SampleIdentity, fallback_index: usize) -> u8 {
    match identity {
        SampleIdentity::Standard { sound, .. } => match sound {
            Sound::Hit(Hitsound::Normal) => 36, // Acoustic Bass Drum
            Sound::Hit(Hitsound::Whistle) => 42, // Closed Hi-Hat
            Sound::Hit(Hitsound::Finish) => 49, // Crash Cymbal 1
            Sound::Hit(Hitsound::Clap) => 39, // Hand Clap
            _ => (60 + (fallback_index % 24)) as u8,
        },
        SampleIdentity::File(name) => {
            let lower = name.to_ascii_lowercase();
            if lower.contains("kick") || lower.contains("normal") {
                36
            } else if lower.contains("snare") || lower.contains("clap") {
                38
            } else if lower.contains("hat") || lower.contains("whistle") {
                42
            } else if lower.contains("crash") || lower.contains("finish") {
                49
            } else {
                (60 + (fallback_index % 24)) as u8
            }
        }
    }
}

/// Exports the active hitsound composer (and optional keysound project) to Standard MIDI File bytes.
pub fn export_hitsounds_to_midi_bytes(
    composer: &Composer,
    bpm: f64,
    keysounds: Option<&dawsu_keysounding::KeysoundProject>,
) -> Vec<u8> {
    let ppq = 480u16;
    let safe_bpm = if bpm.is_finite() && bpm > 1.0 { bpm } else { 120.0 };
    let us_per_quarter = (60_000_000.0 / safe_bpm).round().clamp(1.0, 16_777_215.0) as u32;
    let division = TimeDivision::TicksPerQuarter(ppq);
    let tempo_map = MidiTempoMap::from_changes(division, [(0, us_per_quarter)]);

    let mut tracks = Vec::new();
    let lane_index_map: HashMap<&SampleIdentity, usize> = composer
        .lanes
        .iter()
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect();

    let mut hs_notes = Vec::new();
    for moment in composer.hitsounds().moments() {
        let start_ms = moment.time().ms().max(0.0);
        let start_tick = tempo_map.ms_to_tick(start_ms).round().max(0.0) as u64;
        let end_ms = start_ms + 100.0;
        let end_tick = (start_tick + 120).max(start_tick + 1);
        for id in moment.hits() {
            let idx = lane_index_map.get(id).copied().unwrap_or(0);
            let pitch = lane_to_gm_pitch(id, idx);
            let vol = moment.hit_volume(id).clamp(1, 100);
            let velocity = ((vol as f64 / 100.0) * 127.0).round().clamp(1.0, 127.0) as u8;
            hs_notes.push(MidiNote {
                channel: 9,
                pitch,
                velocity,
                start_tick,
                end_tick,
                start_ms,
                end_ms,
            });
        }
    }
    if !hs_notes.is_empty() {
        tracks.push(TimedTrack {
            index: 0,
            name: Some("Hitsounds".into()),
            notes: hs_notes,
            program_changes: Vec::new(),
            sustain_events: Vec::new(),
        });
    }

    if let Some(project) = keysounds {
        for (idx, track) in project.document.tracks.iter().enumerate() {
            let mut ks_notes = Vec::new();
            for note in &track.notes {
                let start_ms = note.start_ms.max(0.0);
                let end_ms = (start_ms + note.duration_ms().max(10.0)).max(start_ms + 1.0);
                let start_tick = tempo_map.ms_to_tick(start_ms).round().max(0.0) as u64;
                let end_tick = tempo_map
                    .ms_to_tick(end_ms)
                    .round()
                    .max(start_tick as f64 + 1.0) as u64;
                ks_notes.push(MidiNote {
                    channel: (idx % 15) as u8,
                    pitch: note.pitch.clamp(0, 127),
                    velocity: note.velocity.clamp(1, 127),
                    start_tick,
                    end_tick,
                    start_ms,
                    end_ms,
                });
            }
            if !ks_notes.is_empty() {
                tracks.push(TimedTrack {
                    index: tracks.len(),
                    name: Some(track.name.clone()),
                    notes: ks_notes,
                    program_changes: Vec::new(),
                    sustain_events: Vec::new(),
                });
            }
        }
    }

    TimedMidi {
        format: MidiFormat::MultiTrack,
        division,
        tempo_map,
        time_signatures: Vec::new(),
        tracks,
    }
    .to_bytes()
}

/// Writes the exported Standard MIDI File to `path`, returning the number of exported notes.
pub fn export_midi_file(
    path: &Path,
    composer: &Composer,
    bpm: f64,
    keysounds: Option<&dawsu_keysounding::KeysoundProject>,
) -> Result<usize, String> {
    let bytes = export_hitsounds_to_midi_bytes(composer, bpm, keysounds);
    let parsed = TimedMidi::parse(&bytes).map_err(|e| format!("MIDI encode error: {e}"))?;
    let note_count: usize = parsed.tracks.iter().map(|t| t.notes.len()).sum();
    std::fs::write(path, bytes).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;
    Ok(note_count)
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

    #[test]
    fn exports_hitsounds_and_keysounds_to_midi_and_roundtrips() {
        let map = "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
        let osu = OsuFile::read(map.as_bytes().to_vec()).file;
        let hitsounds = match decompile(&osu, &[], &|| false, &mut |_, _| {}) {
            Decompiled::Done(hs) => hs,
            _ => panic!("minimal osu file must decompile"),
        };
        let composer = Composer::new(hitsounds);

        let mut ks = dawsu_keysounding::KeysoundProject::new("audio.mp3", "Keysounds");
        let tid = ks.document.add_track("Lead Synth", 0, false);
        if let Some(track) = ks.document.tracks.iter_mut().find(|t| t.id == tid) {
            track.notes.push(dawsu_keysounding::KeysoundNote::new(500.0, 750.0, 64, 110));
        }

        let bytes = export_hitsounds_to_midi_bytes(&composer, 120.0, Some(&ks));
        let parsed = TimedMidi::parse(&bytes).expect("valid exported MIDI");
        assert!(parsed.tracks.len() >= 3);
        let hs_track = parsed.tracks.iter().find(|t| t.name.as_deref() == Some("Hitsounds")).unwrap();
        assert_eq!(hs_track.notes.len(), 2);
        assert_eq!(hs_track.notes[0].channel, 9);
        assert!(hs_track.notes.iter().any(|n| n.pitch == 39)); // Hand Clap
        assert!((hs_track.notes[0].start_ms - 1000.0).abs() < 2.0);

        let ks_track = parsed.tracks.iter().find(|t| t.name.as_deref() == Some("Lead Synth")).unwrap();
        assert_eq!(ks_track.notes.len(), 1);
        assert_eq!(ks_track.notes[0].pitch, 64);
        assert_eq!(ks_track.notes[0].velocity, 110);
        assert!((ks_track.notes[0].start_ms - 500.0).abs() < 2.0);
    }
}
