//! The Keysound domain (`dawsu-keysounding`): keysound projects, tracks, pure note editing,
//! MIDI import, gameplay sync preview, and `[Keysounds]` difficulty planning.
//!
//! References only `dawsu-foundation`, `dawsu-osu-format`, and `dawsu-hitsounding`
//! (TDD Invariant 1).

pub mod editing;
pub mod generate;
pub mod midi_import;
pub mod model;

pub use editing::{
    deduplicate_notes, glue_notes, quantize_notes, set_velocity, split_note_at, transpose_notes,
    trim_overlaps,
};
pub use generate::{
    plan_generate_keysounds, preview_sync, quantize_duration_ms, quantize_velocity,
    sample_filename, GeneratePlan, SampleManifestEntry, SyncPreview, SyncTarget, SyncedNote,
    UnsyncedNote, KEYSOUNDS_VERSION,
};
pub use midi_import::{import_midi_tracks, import_timed_midi, MIDI_DRUM_CHANNEL};
pub use model::{
    Bookmark, EditorTimeSignature, ExportSettings, InstrumentSettings, KeysoundDocument,
    KeysoundNote, KeysoundProject, KeysoundTrack, ProjectError, TempoPoint,
    DEFAULT_TICKS_PER_QUARTER, PROJECT_VERSION,
};
