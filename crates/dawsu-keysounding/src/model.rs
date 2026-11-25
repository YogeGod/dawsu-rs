//! Keysound domain model and `.dawsuks` v1 project serialization.

use dawsu_foundation::{MidiEvent, MidiEventKind, TempoMap, TimeDivision};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Current `.dawsuks` project schema version.
pub const PROJECT_VERSION: u32 = 1;

/// Default ticks per quarter note used when synthesizing a tempo map without an imported MIDI.
pub const DEFAULT_TICKS_PER_QUARTER: u16 = 480;

/// Errors returned when loading or validating a `.dawsuks` project.
#[derive(Debug)]
pub enum ProjectError {
    Json(serde_json::Error),
    UnsupportedVersion { found: u32, supported: u32 },
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::Json(err) => write!(f, "failed to parse .dawsuks JSON: {err}"),
            ProjectError::UnsupportedVersion { found, supported } => {
                write!(f, "unsupported .dawsuks version {found} (expected {supported})")
            }
        }
    }
}

impl std::error::Error for ProjectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ProjectError::Json(err) => Some(err),
            ProjectError::UnsupportedVersion { .. } => None,
        }
    }
}

impl From<serde_json::Error> for ProjectError {
    fn from(value: serde_json::Error) -> Self {
        ProjectError::Json(value)
    }
}

/// A single pitched note on a keysound track.
///
/// Editing `velocity` changes only the instrument velocity (TDD Responsibility 3a),
/// never the beatmap's sample volume directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeysoundNote {
    pub start_ms: f64,
    pub end_ms: f64,
    pub pitch: u8,
    pub velocity: u8,
    #[serde(default)]
    pub muted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_group: Option<u64>,
}

impl KeysoundNote {
    pub fn new(start_ms: f64, end_ms: f64, pitch: u8, velocity: u8) -> Self {
        Self {
            start_ms,
            end_ms: end_ms.max(start_ms),
            pitch: pitch.min(127),
            velocity: velocity.clamp(1, 127),
            muted: false,
            split_group: None,
        }
    }

    /// Note length in milliseconds (clamped at 0.0).
    pub fn duration_ms(&self) -> f64 {
        (self.end_ms - self.start_ms).max(0.0)
    }
}

/// Instrument configuration attached to a [`KeysoundTrack`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstrumentSettings {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<u8>,
    #[serde(default)]
    pub channel: u8,
    #[serde(default)]
    pub is_drum: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soundfont_path: Option<String>,
    #[serde(default)]
    pub gain_db: f64,
}

impl Default for InstrumentSettings {
    fn default() -> Self {
        Self {
            name: "Default Instrument".to_string(),
            program: None,
            channel: 0,
            is_drum: false,
            soundfont_path: None,
            gain_db: 0.0,
        }
    }
}

/// One CC64 sustain-pedal hold interval (`start_ms..=end_ms`) extracted from MIDI import.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SustainSpan {
    pub start_ms: f64,
    pub end_ms: f64,
}

/// One track of pitched keysound notes and its instrument settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeysoundTrack {
    pub id: u64,
    pub name: String,
    pub channel: u8,
    #[serde(default)]
    pub is_drum: bool,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub instrument: InstrumentSettings,
    #[serde(default)]
    pub notes: Vec<KeysoundNote>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sustain_spans: Vec<SustainSpan>,
}

impl KeysoundTrack {
    pub fn new(id: u64, name: impl Into<String>, channel: u8, is_drum: bool) -> Self {
        let name = name.into();
        Self {
            id,
            name: name.clone(),
            channel: channel & 0x0F,
            is_drum,
            muted: false,
            solo: false,
            instrument: InstrumentSettings {
                name,
                program: None,
                channel: channel & 0x0F,
                is_drum,
                soundfont_path: None,
                gain_db: 0.0,
            },
            notes: Vec::new(),
            sustain_spans: Vec::new(),
        }
    }

    /// Sorts notes deterministically by `(start_ms, pitch, end_ms)`.
    pub fn sort_notes(&mut self) {
        self.notes.sort_by(|a, b| {
            a.start_ms
                .total_cmp(&b.start_ms)
                .then_with(|| a.pitch.cmp(&b.pitch))
                .then_with(|| a.end_ms.total_cmp(&b.end_ms))
        });
    }

    /// Converts unmuted notes on this track into sorted MIDI events for the instrument engine
    /// (TDD Responsibility 10: the audio engine never sees a track).
    pub fn to_midi_events(&self, tempo_map: &TempoMap) -> Vec<MidiEvent> {
        if self.muted {
            return Vec::new();
        }
        let mut events = Vec::with_capacity(self.notes.len() * 2 + 1);
        if let Some(program) = self.instrument.program {
            events.push(MidiEvent {
                tick: 0,
                kind: MidiEventKind::ProgramChange {
                    channel: self.channel & 0x0F,
                    program: program.min(127),
                },
            });
        }
        for note in &self.notes {
            if note.muted {
                continue;
            }
            let start_tick = tempo_map.ms_to_tick(note.start_ms).round().max(0.0) as u64;
            let end_tick = (tempo_map.ms_to_tick(note.end_ms).round().max(0.0) as u64).max(start_tick);
            events.push(MidiEvent {
                tick: start_tick,
                kind: MidiEventKind::NoteOn {
                    channel: self.channel & 0x0F,
                    pitch: note.pitch.min(127),
                    velocity: note.velocity.clamp(1, 127),
                },
            });
            events.push(MidiEvent {
                tick: end_tick,
                kind: MidiEventKind::NoteOff {
                    channel: self.channel & 0x0F,
                    pitch: note.pitch.min(127),
                    velocity: 0,
                },
            });
        }
        // Sort by tick, placing NoteOff before NoteOn at identical ticks so retriggers are clean.
        events.sort_by_key(|ev| {
            let order = match ev.kind {
                MidiEventKind::ProgramChange { .. } => 0u8,
                MidiEventKind::NoteOff { .. } => 1u8,
                MidiEventKind::NoteOn { .. } => 2u8,
                _ => 3u8,
            };
            (ev.tick, order)
        });
        events
    }
}

/// An editor-only time signature (TDD Responsibility 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorTimeSignature {
    pub time_ms: f64,
    pub numerator: u8,
    pub denominator: u16,
}

/// An editor-only bookmark (TDD Responsibility 4, Invariant 3: never written into a map).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub time_ms: f64,
    #[serde(default)]
    pub label: String,
}

/// Serializable tempo map point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TempoPoint {
    pub tick: u64,
    pub time_ms: f64,
    pub us_per_quarter: u32,
}

/// The editable keysound document: tracks, tempo map, editor-only time signatures and bookmarks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeysoundDocument {
    #[serde(default)]
    pub tracks: Vec<KeysoundTrack>,
    #[serde(default = "default_tpq")]
    pub ticks_per_quarter: u16,
    #[serde(default)]
    pub tempo_points: Vec<TempoPoint>,
    #[serde(default)]
    pub time_signatures: Vec<EditorTimeSignature>,
    #[serde(default)]
    pub bookmarks: Vec<Bookmark>,
}

fn default_tpq() -> u16 {
    DEFAULT_TICKS_PER_QUARTER
}

impl Default for KeysoundDocument {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            ticks_per_quarter: DEFAULT_TICKS_PER_QUARTER,
            tempo_points: vec![TempoPoint {
                tick: 0,
                time_ms: 0.0,
                us_per_quarter: dawsu_foundation::midi::DEFAULT_US_PER_QUARTER,
            }],
            time_signatures: vec![EditorTimeSignature {
                time_ms: 0.0,
                numerator: 4,
                denominator: 4,
            }],
            bookmarks: Vec::new(),
        }
    }
}

impl KeysoundDocument {
    /// Returns a [`TempoMap`] built from this document's tempo points and PPQN.
    pub fn tempo_map(&self) -> TempoMap {
        let tpq = self.ticks_per_quarter.max(1);
        TempoMap::from_changes(
            TimeDivision::TicksPerQuarter(tpq),
            self.tempo_points.iter().map(|p| (p.tick, p.us_per_quarter)),
        )
    }

    /// Adds a new track with the next available id and returns its id.
    pub fn add_track(&mut self, name: impl Into<String>, channel: u8, is_drum: bool) -> u64 {
        let next_id = self.tracks.iter().map(|t| t.id).max().map_or(1, |m| m + 1);
        self.tracks.push(KeysoundTrack::new(next_id, name, channel, is_drum));
        next_id
    }

    /// Renames the track with `track_id`, returning `true` if found.
    pub fn rename_track(&mut self, track_id: u64, new_name: impl Into<String>) -> bool {
        if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
            track.name = new_name.into();
            true
        } else {
            false
        }
    }

    /// Deletes the track with `track_id`, returning the removed track if found.
    pub fn delete_track(&mut self, track_id: u64) -> Option<KeysoundTrack> {
        let pos = self.tracks.iter().position(|t| t.id == track_id)?;
        Some(self.tracks.remove(pos))
    }
}

/// Settings governing sample bucketing and `[Keysounds]` difficulty generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    /// Number of velocity quantization buckets across `1..=127` (e.g. 8 or 16).
    #[serde(default = "default_velocity_buckets")]
    pub velocity_buckets: u8,
    /// Duration quantization step in milliseconds (e.g. 50.0 ms).
    #[serde(default = "default_duration_bucket_ms")]
    pub duration_bucket_ms: f64,
    /// Prefix for generated sample filenames (e.g. `"ks"`).
    #[serde(default = "default_sample_prefix")]
    pub sample_prefix: String,
    /// File extension for generated samples (without leading dot, e.g. `"wav"` or `"ogg"`).
    #[serde(default = "default_sample_extension")]
    pub sample_extension: String,
    /// Maximum distance in milliseconds to sync a keysound note to a reference hit object.
    #[serde(default = "default_sync_tolerance_ms")]
    pub sync_tolerance_ms: f64,
    /// Difficulty Version name for the generated keysounds beatmap (default `"Keysounds"`).
    #[serde(default = "default_target_version")]
    pub target_version: String,
    /// When true, simultaneous chord notes on one osu!standard hit circle flatten into one polyphonic sample.
    #[serde(default = "default_flatten_chords")]
    pub flatten_chords: bool,
}

fn default_flatten_chords() -> bool {
    true
}

fn default_velocity_buckets() -> u8 {
    16
}
fn default_duration_bucket_ms() -> f64 {
    50.0
}
fn default_sample_prefix() -> String {
    "ks".to_string()
}
fn default_sample_extension() -> String {
    "wav".to_string()
}
fn default_sync_tolerance_ms() -> f64 {
    10.0
}
fn default_target_version() -> String {
    "Keysounds".to_string()
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            velocity_buckets: default_velocity_buckets(),
            duration_bucket_ms: default_duration_bucket_ms(),
            sample_prefix: default_sample_prefix(),
            sample_extension: default_sample_extension(),
            sync_tolerance_ms: default_sync_tolerance_ms(),
            target_version: default_target_version(),
            flatten_chords: default_flatten_chords(),
        }
    }
}

/// A `.dawsuks` v1 keysound project file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeysoundProject {
    pub version: u32,
    pub song_path: String,
    pub target_difficulty: String,
    #[serde(default)]
    pub export_settings: ExportSettings,
    #[serde(flatten)]
    pub document: KeysoundDocument,
}

impl KeysoundProject {
    pub fn new(song_path: impl Into<String>, target_difficulty: impl Into<String>) -> Self {
        Self {
            version: PROJECT_VERSION,
            song_path: song_path.into(),
            target_difficulty: target_difficulty.into(),
            export_settings: ExportSettings::default(),
            document: KeysoundDocument::default(),
        }
    }

    /// Serializes this project as pretty-printed `.dawsuks` v1 JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserializes a `.dawsuks` v1 JSON string, validating `version == PROJECT_VERSION`.
    pub fn from_json(json: &str) -> Result<Self, ProjectError> {
        let project: Self = serde_json::from_str(json)?;
        if project.version != PROJECT_VERSION {
            return Err(ProjectError::UnsupportedVersion {
                found: project.version,
                supported: PROJECT_VERSION,
            });
        }
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_round_trips_through_dawsuks_v1_json() {
        let mut project = KeysoundProject::new("audio.mp3", "Artist - Song (Mapper) [Normal].osu");
        let track_id = project.document.add_track("Lead Piano", 0, false);
        project.document.tracks[0].notes.push(KeysoundNote {
            start_ms: 1000.0,
            end_ms: 1500.0,
            pitch: 60,
            velocity: 100,
            muted: false,
            split_group: Some(7),
        });
        project.document.bookmarks.push(Bookmark {
            time_ms: 1000.0,
            label: "Chorus".into(),
        });

        assert!(project.document.rename_track(track_id, "Grand Piano"));
        let json = project.to_json().expect("serialize .dawsuks");
        assert!(json.contains("\"version\": 1"));
        assert!(json.contains("\"tracks\""));
        assert!(json.contains("\"target_difficulty\""));
        assert!(json.contains("\"export_settings\""));

        let loaded = KeysoundProject::from_json(&json).expect("deserialize .dawsuks");
        assert_eq!(loaded, project);
    }

    #[test]
    fn rejects_unsupported_project_version() {
        let json = r#"{"version": 99, "song_path": "a.mp3", "target_difficulty": "b.osu"}"#;
        let err = KeysoundProject::from_json(json).unwrap_err();
        assert!(matches!(err, ProjectError::UnsupportedVersion { found: 99, supported: 1 }));
    }

    #[test]
    fn track_lifecycle_and_midi_event_conversion() {
        let mut doc = KeysoundDocument::default();
        let t1 = doc.add_track("Bass", 1, false);
        let t2 = doc.add_track("Drums", 9, true);
        assert_eq!(doc.tracks.len(), 2);
        assert!(doc.delete_track(t2).is_some());
        assert_eq!(doc.tracks.len(), 1);
        assert_eq!(doc.tracks[0].id, t1);

        doc.tracks[0].instrument.program = Some(33);
        doc.tracks[0].notes.push(KeysoundNote::new(0.0, 500.0, 48, 96));
        let events = doc.tracks[0].to_midi_events(&doc.tempo_map());
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind, MidiEventKind::ProgramChange { channel: 1, program: 33 });
        assert_eq!(events[1].kind, MidiEventKind::NoteOn { channel: 1, pitch: 48, velocity: 96 });
        assert_eq!(events[2].kind, MidiEventKind::NoteOff { channel: 1, pitch: 48, velocity: 0 });
        assert_eq!(events[2].tick, 480);
    }
}
