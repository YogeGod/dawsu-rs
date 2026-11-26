//! The Keysound domain (`dawsu-keysounding`): keysound projects, tracks, pure note editing,
//! MIDI import, gameplay sync preview, and `[Keysounds]` difficulty planning.
//!
//! References only `dawsu-foundation`, `dawsu-osu-format`, and `dawsu-hitsounding`
//! (TDD Invariant 1).

pub mod chords;
pub mod editing;
pub mod generate;
pub mod midi_import;
pub mod model;

pub use chords::{
    chord_voicing_key, detect_chords, resolve_all_chords, resolve_chord, ChordConflict,
    ChordResolution,
};
pub use editing::{
    apply_velocity_ramp, deduplicate_notes, glue_notes, humanise_notes, is_pitch_in_scale,
    quantize_notes, scale_pitch_mask, set_velocity, split_note_at, strum_notes, transpose_notes,
    trim_overlaps, ScaleKind, StrumDirection,
};
pub use generate::{
    plan_generate_keysounds, preview_sync, quantize_duration_ms, quantize_velocity,
    sync_flags,
    sample_filename, GeneratePlan, SampleManifestEntry, SyncPreview, SyncTarget, SyncedNote,
    UnsyncedNote, KEYSOUNDS_VERSION,
};
pub use midi_import::{import_midi_tracks, import_timed_midi, MIDI_DRUM_CHANNEL};
pub use model::{
    Bookmark, EditorTimeSignature, ExportSettings, InstrumentSettings, KeysoundDocument,
    KeysoundNote, KeysoundProject, KeysoundTrack, ProjectError, SustainSpan, TempoPoint,
    DEFAULT_TICKS_PER_QUARTER, PROJECT_VERSION,
};
