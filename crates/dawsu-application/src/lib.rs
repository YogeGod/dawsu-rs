//! Documents, playback, storage and background jobs.

pub mod base_layer;
pub mod export_manifest;
pub mod stem_separation;
mod bookmarks;
mod default_sounds;
mod slider_body;
mod slider_export;
mod commands;
mod edit_workflows;
mod controller;
mod editor_meter;
pub mod drum_classify;
mod drum_detect;
pub mod drum_vst;
mod export;
mod export_settings;
mod generate_receipt;
mod gain_recovery;
mod lane_edit;
mod layers;
mod mixes;
mod original_snapshot;
mod pending;
mod sweep;
pub mod output;
pub mod node_graph;
pub mod midi_import;
pub mod midi_record;
pub mod vst3;
mod keysound_session;
pub mod bounce_plan;
pub mod preview_diff;
mod project;
mod app_drum_vst;
mod app_editing;
mod app_playback;
mod app_save_io;
mod document;
#[cfg(test)]
mod tests;
pub use document::{
    CoverageSummary, Document, SaveReport, save_document, save_document_with_cleanup,
};
use app_save_io::{problem_description, recorded_pending};
#[cfg(test)]
use app_save_io::{load, project_hits, restore_project_hits};
use document::{Pass, PreparedSave, compile_pass, compile_snapshot, last_written};
#[cfg(test)]
use document::{PrepareSaveInput, compile_for_save, prepare_save, unfinished_record};
pub use base_layer::{
    BaseHit, BaseMoment, BaseRule, DiffProjectionTab, RelinkMapsetResult, SharedBaseApplyAllResult,
    SharedBaseApplyResult, SharedBaseLayer, SharedBaseStatus, TimeRangeMs,
};
pub use dawsu_hitsounding::index_packing::{
    AdditionOnlyNormal, CustomIndexBucket, CustomIndexPool, STANDARD_SILENT_WAV, SlotContent,
    SlotKey,
};
pub use dawsu_hitsounding::tick_render::{SliderEditHistory, TickPolicy, TickWrite};
pub use dawsu_hitsounding::volume_envelope::{CurveKind, VolumeEnvelope, VolumePoint};
pub use export_manifest::{
    ExportOwnershipManifest, ExportWriteMode, HitsoundOnlyExportResult, SingleSampleFormat,
    StaleCleanupReport,
};
pub use stem_separation::{
    StemSeparationJobState, StemSeparationStartOptions, StemSeparationStatus,
};
pub use project::lane_key as lane_layout_key;
pub use node_graph::{NodeGraphAction, NodeGraphView, ViewerOverlay};
pub use dawsu_audio::{OutputBackend, OutputDevice};
pub use dawsu_audio_instruments::midi_in::{
    MidiInputEvent, MidiInputRouter, decode_midi_bytes, list_midi_input_ports,
};
pub use midi_import::{
    MidiImportOptions, MidiImportOutcome, MidiInspectResult, MidiTrackSummary, apply_midi_import,
    export_midi_file, inspect_midi_file,
};

/// Summary returned after importing hitsounds from an osu!mania (Mode 3) difficulty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManiaImportReport {
    pub hits_imported: usize,
    pub lanes_touched: usize,
    pub lanes_added: usize,
    pub matched_moments: usize,
    pub created_moments: usize,
}
pub use midi_record::MidiRecordStatus;
pub use vst3::{
    BounceFormat, BounceOptions, BounceOutput, LoadedPluginSummary, MidiPreviewNote, PluginRole,
    ScanSkip, VelocityExportMode, Vst3BounceJob, Vst3DrumExportPlan, Vst3DrumHit, Vst3Error,
    Vst3HitAssignment, Vst3Manager, Vst3ScanReport, plan_vst3_drum_export,
};
mod preferences;
mod shortcuts;
mod problems;
mod recovery;
mod refusals;
mod roll_back;
pub mod sample_checks;
pub mod songs_index;
mod sound_sets;
mod unwritten;
mod osu_watch;
mod sample_watch;
pub mod waveform;
use dawsu_editor::commands::{CommandCatalog, KeyMap};
#[cfg(test)]
use dawsu_editor::keys::KeyChord;
pub use problems::{OutputProblem, orphaned_hit_times};
pub use editor_meter::{EditorBarSection, EditorTimeSignature};
pub use dawsu_graph_audio::classic::Drum;
pub use drum_detect::{DrumDetectState, DrumDetectView, DrumLanes, DrumOptions, DrumRow, DrumSource, ObjectKind, RowLands, GRIDS, MAX_OBJECT_MS};
pub use export::{ExportChoices, ExportOptions, ExportPreview, ExportStatus, PreviewOrigin, Stale};
pub use export_settings::{ExportCombination, ExportSettings};
pub use mixes::MixFormat;
pub use original_snapshot::SnapshotPreview;
pub use project::{HsStarterMode, LaneLayout};
pub use recovery::RecoveryOffer;
pub use sample_checks::{FixedWav, SampleHealthIssue, SampleHealthReport};
pub use sound_sets::{LibrarySet, PlacementView, SaveSetError, SetUpdatePrompt, SoundSetLibrary};
pub use waveform::WaveBin;
pub use edit_workflows::{PatternStampOptions, RangeOperation, TimelineEdit, VolumeScope};
pub use sweep::{SweepDryRunSnapshot, UnusedSampleCandidate, run_plan_sweep};

use dawsu_audio::{AudioClip, AudioEngine};
#[cfg(test)]
use dawsu_editor::tempo::SnapSpacing;
use dawsu_editor::tools::ToolBox;
use dawsu_editor_composer::HitKey;
use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, SliderSound, Sound};
#[cfg(test)]
use dawsu_foundation::SongTime;
use dawsu_hitsounding::{Lane, LaneSource};
#[cfg(test)]
use dawsu_hitsounding::{Decompiled, Hitsounds, decompile};
use dawsu_osu_format::{check_map_path, find_case_insensitive};
#[cfg(test)]
use dawsu_osu_format::OsuFile;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};

/// The farthest the timeline zooms out, in pixels per millisecond: low enough that Fit shows a
/// 30-minute song in a 1000 px timeline. The view, Zoom out and reopening all use it.
pub const MIN_ZOOM: f64 = 0.0005;
pub const DEFAULT_SONG_OFFSET_MS: f64 = -22.0;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub recent: Vec<PathBuf>,
    pub songs_folder: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hitsound_library: Option<PathBuf>,
    pub default_samples: PathBuf,
    pub device: Option<String>,
    #[serde(skip_serializing_if = "system_audio_backend")]
    pub audio_backend: OutputBackend,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_buffer_ms: Option<u32>,
    pub song_gain: f32,
    pub song_muted: bool,
    #[serde(default = "default_hitsound_gain")]
    pub hitsound_gain: f32,
    #[serde(default)]
    pub hitsounds_muted: bool,
    pub master_gain: f32,
    pub audition: bool,
    pub follow: bool,
    pub return_on_stop: bool,
    pub show_browser: bool,
    pub show_inspector: bool,
    pub show_controller: bool,
    pub used_only: bool,
    pub lane_names: bool,
    pub object_lines: bool,
    pub classic_browser: bool,
    pub theme: usize,
    pub key_preset: String,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub key_overrides: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>>,
    pub protect_hitnormals: bool,
    pub clean_redundant_greenlines: bool,
    /// Keep original song pitch when slowed down or sped up (default true).
    #[serde(default = "default_true")]
    pub keep_pitch: bool,
    /// Automatically check GitHub Releases for newer builds when the app launches (default true).
    #[serde(default = "default_true")]
    pub auto_check_updates: bool,
    /// Compact beat-step pad toggle mode on lane rows (default false).
    #[serde(default)]
    pub step_pads_mode: bool,
    /// The map preview's own window: where it was, and whether it was open when the editor closed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_window: Option<WindowPlace>,
}

/// A window's place and size in logical pixels, and whether it was open.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPlace {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub open: bool,
}
fn default_hitsound_gain() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}

fn system_audio_backend(backend: &OutputBackend) -> bool {
    *backend == OutputBackend::System
}

fn open_preferred_audio(preferences: &Preferences, buffer_ms: Option<u32>) -> Result<AudioEngine, String> {
    match (preferences.audio_backend, preferences.audio_device_id.as_deref(), preferences.device.as_deref()) {
        (OutputBackend::System, None, Some(legacy_name)) => AudioEngine::open_with_buffer(Some(legacy_name), buffer_ms),
        (backend, device, _) => AudioEngine::open_output(backend, device, buffer_ms),
    }
}

impl Preferences {
    fn effective_song_gain(&self) -> f32 {
        if self.song_muted { 0.0 } else { self.song_gain }
    }
    fn effective_hitsound_gain(&self) -> f32 {
        if self.hitsounds_muted { 0.0 } else { self.hitsound_gain }
    }
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            songs_folder: PathBuf::new(),
            hitsound_library: None,
            default_samples: PathBuf::new(),
            device: None,
            audio_backend: OutputBackend::System,
            audio_device_id: None,
            audio_buffer_ms: None,
            song_gain: 0.7,
            song_muted: false,
            hitsound_gain: 1.0,
            hitsounds_muted: false,
            master_gain: 0.2,
            audition: true,
            follow: false,
            return_on_stop: true,
            show_browser: true,
            show_inspector: true,
            show_controller: true,
            used_only: false,
            lane_names: true,
            object_lines: false,
            classic_browser: false,
            theme: 0,
            key_preset: "DAW".into(),
            key_overrides: Default::default(),
            protect_hitnormals: false,
            clean_redundant_greenlines: false,
            keep_pitch: true,
            auto_check_updates: true,
            step_pads_mode: false,
            preview_window: None,
        }
    }
}
/// The developer's local default sounds, used by the desktop app when no default sounds
/// folder is saved ([`Application::for_desktop`]). On other computers the folder is missing
/// and nothing changes.
const LOCAL_DEFAULT_SAMPLES: &str = "E:/GitHub/dawsu-hitsounding-editor/src/OsuHitsound.Audio/DefaultSamples";
pub fn data_folder() -> PathBuf {
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(path).join("dawsu-rs")
    } else if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        PathBuf::from(path).join("dawsu")
    } else {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share/dawsu")
    }
}
#[derive(Debug, Clone)]
pub enum PlatformRequest {
    OpenMap,
    OpenSamples,
    OpenSongs,
    OpenFolder(PathBuf),
}
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
}
struct Loaded {
    document: Document,
    project: Option<project::Project>,
    song_path: Option<PathBuf>,
    samples: HashMap<SampleIdentity, Arc<AudioClip>>,
    warnings: Vec<String>,
    recovery: Option<recovery::RecoveryCopy>,
    /// The pending names a record of the outgoing mapset still lists, read by the open job.
    outgoing_keep: Option<BTreeSet<String>>,
}
enum OpenSource {
    Map(PathBuf),
    Recovery(PathBuf),
}
struct ReloadPreserve {
    position_ms: f64,
    view_start: f64,
    zoom: f64,
    row_height: f32,
    was_playing: bool,
    playback_speed: f64,
    song_path: Option<PathBuf>,
    forced: bool,
    status_note: &'static str,
    snap_mode: dawsu_editor::tempo::SnapMode,
    snap_enabled: bool,
    spacing: dawsu_editor::tempo::SnapSpacing,
    hidden_lanes: BTreeSet<SampleIdentity>,
}
struct PendingSave {
    version: u64,
    hs: bool,
    plan: PreparedSave,
    project: Option<project::Project>,
    settings: Option<project::SessionSettings>,
}

impl PendingSave {
    fn finish(mut self, cancel: &dyn Fn() -> bool) -> JobResult {
        let path = self.plan.path.clone();
        let result = self.plan.apply(self.project.as_mut(), cancel);
        JobResult::Save {
            version: self.version, path, hs_mode: self.hs, project: self.project.map(Box::new), settings: self.settings.map(Box::new),
            result: result.map(Box::new), diagnostics: Vec::new(),
        }
    }
}

type DrumVstSource = (usize, SampleIdentity, i32);
type OutputJobResult = (usize, Option<String>, Vec<String>, std::collections::BTreeMap<String, u32>);

enum JobResult {
    Open(Option<u64>, bool, Option<ReloadPreserve>, Result<Box<Loaded>, String>),
    SongReady {
        generation: u64,
        song: Option<Arc<AudioClip>>,
        waveform: Vec<WaveBin>,
        warning: Option<String>,
    },
    DrumStemReady {
        generation: u64,
        path: PathBuf,
        clip: Option<Arc<AudioClip>>,
        waveform: Vec<WaveBin>,
        warning: Option<String>,
    },
    DrumVstReady {
        generation: u64,
        saved: project::DrumVstSession,
        result: Result<
            dawsu_audio_instruments::HostSession<dawsu_audio_instruments::ProcessTransport>,
            String,
        >,
    },
    Save {
        version: u64,
        path: PathBuf,
        hs_mode: bool,
        settings: Option<Box<project::SessionSettings>>,
        project: Option<Box<project::Project>>,
        result: Result<Box<SaveReport>, String>,
        diagnostics: Vec<OutputProblem>,
    },
    SaveCheck {
        version: u64,
        path: PathBuf,
        hs: bool,
        result: Result<Box<PendingSave>, String>,
        diagnostics: Vec<OutputProblem>,
    },
    Review(Result<Box<output::WritePlan>, String>, Vec<OutputProblem>),
    /// The files written, a receipt warning, and the pending files the write put in the mapset.
    Output(Result<OutputJobResult, String>),
}
#[derive(Clone)]
struct JobProgress {
    label: &'static str,
    current: Arc<Mutex<Option<(usize, usize)>>>,
}

impl JobProgress {
    fn report(&self, done: usize, total: usize) {
        if let Ok(mut current) = self.current.lock() {
            *current = Some((done, total));
        }
    }
}

pub struct Application {
    sample_watch: sample_watch::Watch,
    osu_watch: osu_watch::OsuWatch,
    pub external_osu_changed: bool,
    document: Option<Document>,
    audio: Option<AudioEngine>,
    samples: HashMap<SampleIdentity, Arc<AudioClip>>,
    song: Option<Arc<AudioClip>>,
    /// The sample file last auditioned or outlined from the Browser, kept while the file is unchanged
    /// so a pick decodes it once for both.
    sample_file: Option<(PathBuf, (u64, std::time::SystemTime), Arc<AudioClip>)>,
    pub playback_speed: f64,
    pub waveform: Vec<WaveBin>,
    pub preferences: Preferences,
    pub status: String,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub requests: Vec<PlatformRequest>,
    pub pending_open: Option<PathBuf>,
    edit_dialog: Option<&'static str>,
    quantize_strength: f64,
    pub revert_requested: bool,
    has_original_snapshot: Arc<AtomicBool>,
    can_undo_revert: Arc<AtomicBool>,
    pub busy: bool,
    pub song_loading: bool,
    pending_play_on_song: bool,
    song_job: Option<mpsc::Receiver<JobResult>>,
    song_cancel: Arc<AtomicBool>,
    song_generation: u64,
    drum_stem_path: Option<PathBuf>,
    drum_stem: Option<Arc<AudioClip>>,
    drum_stem_waveform: Option<Arc<Vec<WaveBin>>>,
    pub drum_stem_loading: bool,
    monitor_stem: bool,
    drum_stem_auto_monitor: bool,
    drum_stem_job: Option<mpsc::Receiver<JobResult>>,
    drum_stem_cancel: Arc<AtomicBool>,
    drum_stem_generation: u64,
    stem_separation: stem_separation::StemSeparationManager,
    waveform_revision: u64,
    pending_locator_right: Option<(f64, Option<f64>, bool)>,
    #[cfg(test)]
    song_decode_hold: Option<Arc<AtomicBool>>,
    #[cfg(test)]
    started_play_count: usize,
    pub output_review: Option<output::WritePlan>,
    export: export::Slot,
    /// Counts loads and Saves of the open difficulty, for the Export preview's identity.
    document_generation: u64,
    pending_save_review: Option<PendingSave>,
    output_problems: Vec<OutputProblem>,
    output_problem_revision: Option<(u64, bool)>,
    save_problem_review: bool,
    tools: ToolBox<HitKey>,
    controller_stroke: Option<controller::ControllerStroke>,
    jobs: Option<mpsc::Receiver<JobResult>>,
    /// The running cleanup of unused sample files, which keeps `busy` set until close or open
    /// stops waiting for it.
    sweep: Option<mpsc::Receiver<sweep::Outcome>>,
    /// Set once close or open stopped waiting: the job's document is gone, so what it moved goes back.
    sweep_detached: bool,
    /// How long close and open wait for a cleanup job (Yoge, 2026-11-21: 5 seconds).
    sweep_wait: std::time::Duration,
    sweep_after_open: bool,
    job_progress: Option<JobProgress>,
    cancel: Arc<AtomicBool>,
    muted: Vec<SampleIdentity>,
    solo: Vec<SampleIdentity>,
    lane_gains: HashMap<SampleIdentity, f32>,
    song_solo: bool,
    hitsounds_solo: bool,
    cached_version: u64,
    cached_listen_version: u64,
    pub zoom: f64,
    pub row_height: f32,
    project_root: Option<PathBuf>,
    project: Option<project::Project>,
    base_layer_track: Option<base_layer::BaseLayerTrack>,
    pub locator_left: f64,
    pub locator_right: f64,
    pub cycle: bool,
    pub song_offset: f64,
    pub active_lane: usize,
    selection_lane_scope: Option<BTreeSet<SampleIdentity>>,
    playback_start: f64,
    return_position: f64,
    last_endpoint_jump: Option<(bool, std::time::Instant)>,
    key_lane_view: Option<(PathBuf, Vec<usize>, f64)>,
    pub first_launch: bool,
    catalogue: Arc<CommandCatalog<Application>>,
    pub keymap: KeyMap,
    pub help_requested: bool,
    /// The bookmark Previous or Next bookmark last jumped to, until the Tauri view takes it.
    pub bookmark_step: Option<f64>,
    persist_preferences: bool,
    data_root: PathBuf,
    checkpoint: Option<recovery::Checkpoint>,
    restored_recovery: Option<String>,
    /// Open-time questions about placements on an older version of a library set (M9).
    sound_set_prompts: Vec<sound_sets::SetUpdatePrompt>,
    /// The open-time check waits for the library index's background read.
    sound_set_check_pending: bool,
    /// Difficulties opened so far: an update question closed without an answer comes back on the
    /// next open.
    opens: u64,
    drum: drum_detect::DrumDetect,
    node_graph: node_graph::NodeGraphState,
    pub midi_recorder: midi_record::MidiRecorder,
    pub keysounds: dawsu_keysounding::KeysoundProject,
    pub keysound_history: keysound_session::KeysoundUndoHistory,
    pub bounce_cancel: bounce_plan::BounceCancelToken,
    pub vst3: vst3::Vst3Manager,
    pub drum_vst: Option<drum_vst::DrumVstLiveHandle>,
    pub drum_vst_loading: bool,
    drum_vst_job: Option<mpsc::Receiver<JobResult>>,
    drum_vst_cancel: Arc<AtomicBool>,
    drum_vst_generation: u64,
    drum_vst_state_b64: Option<String>,
    drum_vst_saved: Option<project::DrumVstSession>,
    #[cfg(test)]
    drum_vst_restore_hold: Option<Arc<AtomicBool>>,
}
impl Application {
    pub fn new() -> Self {
        Self::with_data_root(data_folder())
    }
    /// The desktop app's start: [`Self::with_data_root`], then the local default sounds when
    /// the preferences name no default sounds folder. Tests that build the app in the same
    /// process start with `with_data_root` alone, so they never see a folder that exists on
    /// one computer only.
    pub fn for_desktop(data_root: PathBuf) -> Self {
        let mut app = Self::with_data_root(data_root);
        app.use_local_default_samples(Path::new(LOCAL_DEFAULT_SAMPLES));
        app
    }
    /// Uses `local` as the default sounds folder when none is saved and the folder exists.
    /// The folder is only read.
    fn use_local_default_samples(&mut self, local: &Path) {
        if self.preferences.default_samples.as_os_str().is_empty() && local.is_dir() {
            self.preferences.default_samples = local.to_path_buf();
        }
    }
    pub fn with_data_root(data_root: PathBuf) -> Self {
        let file = data_root.join("preferences.json");
        let legacy = std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("OsuHitsoundEditor").join("preferences.json"));
        let (mut preferences, first_launch) = preferences::load(&file, legacy.as_deref());
        preferences::detect_songs_folder_if_empty(&mut preferences);
        preferences.master_gain = preferences.master_gain.clamp(0.2, 1.0);
        let output = open_preferred_audio(&preferences, preferences.audio_buffer_ms);
        let (audio, error) = match output {
            Ok(engine) => (Some(engine), None),
            Err(error) => {
                let same_device = preferences.audio_buffer_ms.is_some()
                    .then(|| open_preferred_audio(&preferences, None));
                match same_device {
                    Some(Ok(engine)) => (Some(engine), Some(format!("Using the default audio buffer. {error}"))),
                    _ if preferences.audio_backend != OutputBackend::System
                        || preferences.audio_device_id.is_some() || preferences.device.is_some() => {
                        match AudioEngine::open_output(OutputBackend::System, None, None) {
                            Ok(engine) => (Some(engine), Some(format!("Using the default system output. {error}"))),
                            Err(fallback) => (None, Some(format!("{error} System default: {fallback}"))),
                        }
                    }
                    Some(Err(fallback)) => (None, Some(format!("{error} Device default: {fallback}"))),
                    None => (None, Some(error)),
                }
            }
        };
        if let Some(a) = &audio {
            a.gains(preferences.effective_song_gain(), preferences.master_gain);
            a.hitsound_gain(preferences.effective_hitsound_gain());
        }
        Self::create(preferences, first_launch, audio, error, true, data_root)
    }
    /// An application with no audio and no saved preferences. Its data folder is a temporary
    /// one, so the files it keeps never reach the user's own.
    pub fn headless() -> Self {
        let data_root = std::env::temp_dir().join("dawsu-headless").join(std::process::id().to_string());
        Self::create(Preferences::default(), false, None, None, false, data_root)
    }
    pub fn headless_with_projects(data_root: PathBuf) -> Self {
        let mut app = Self::create(Preferences::default(), false, None, None, false, data_root.clone());
        app.project_root = Some(data_root.join("projects"));
        app
    }
    /// Resets first-open Path 3 view defaults (Objects snap + hidden empty lanes) back to Grid
    /// snap with all lanes visible and a clean undo stack. Used by integration tests that open
    /// fixtures expecting the legacy Grid + all-12-lanes baseline.
    #[doc(hidden)]
    pub fn reset_fresh_open_for_tests(&mut self) {
        if let Some(doc) = self.document.as_mut() {
            doc.composer.snap_mode = dawsu_editor::tempo::SnapMode::Grid;
            doc.composer.history.update_external(|hits| {
                hits.restore_hidden_lanes(std::iter::empty());
            });
            doc.composer.lanes = doc
                .composer
                .hitsounds()
                .lanes()
                .iter()
                .map(|lane| lane.identity.clone())
                .collect();
        }
    }
    fn create(
        preferences: Preferences,
        first_launch: bool,
        audio: Option<AudioEngine>,
        error: Option<String>,
        persist_preferences: bool,
        data_root: PathBuf,
    ) -> Self {
        let catalogue = Arc::new(commands::catalogue());
        let keymap = shortcuts::keymap(catalogue.definitions(), &preferences);
        Self {
            document: None,
            sample_watch: sample_watch::Watch::default(),
            osu_watch: osu_watch::OsuWatch::default(),
            external_osu_changed: false,
            audio,
            samples: HashMap::new(),
            song: None,
            playback_speed: 1.0,
            waveform: Vec::new(),
            preferences,
            status: "Open a difficulty to start".into(),
            error,
            warnings: Vec::new(),
            requests: Vec::new(),
            pending_open: None,
            edit_dialog: None,
            quantize_strength: 100.0,
            revert_requested: false,
            has_original_snapshot: Arc::new(AtomicBool::new(false)),
            can_undo_revert: Arc::new(AtomicBool::new(false)),
            busy: false,
            song_loading: false,
            pending_play_on_song: false,
            song_job: None,
            song_cancel: Arc::new(AtomicBool::new(false)),
            song_generation: 0,
            drum_stem_path: None,
            drum_stem: None,
            drum_stem_waveform: None,
            drum_stem_loading: false,
            monitor_stem: false,
            drum_stem_auto_monitor: false,
            drum_stem_job: None,
            drum_stem_cancel: Arc::new(AtomicBool::new(false)),
            drum_stem_generation: 0,
            stem_separation: stem_separation::StemSeparationManager::default(),
            waveform_revision: 0,
            pending_locator_right: None,
            #[cfg(test)]
            song_decode_hold: None,
            #[cfg(test)]
            started_play_count: 0,
            output_review: None,
            export: export::Slot::default(),
            document_generation: 0,
            pending_save_review: None,
            output_problems: Vec::new(),
            output_problem_revision: None,
            save_problem_review: false,
            tools: ToolBox::new(),
            controller_stroke: None,
            jobs: None,
            sweep: None,
            sweep_detached: false,
            sweep_wait: std::time::Duration::from_secs(5),
            sweep_after_open: false,
            job_progress: None,
            cancel: Arc::new(AtomicBool::new(false)),
            muted: Vec::new(),
            solo: Vec::new(),
            lane_gains: HashMap::new(),
            song_solo: false,
            hitsounds_solo: false,
            sample_file: None,
            cached_version: 0,
            cached_listen_version: 0,
            zoom: 0.12,
            row_height: 30.0,
            locator_left: 0.0,
            locator_right: 10000.0,
            cycle: false,
            song_offset: DEFAULT_SONG_OFFSET_MS,
            active_lane: 0,
            selection_lane_scope: None,
            playback_start: 0.0,
            return_position: 0.0,
            last_endpoint_jump: None,
            key_lane_view: None,
            first_launch,
            catalogue,
            keymap,
            help_requested: false,
            bookmark_step: None,
            persist_preferences,
            project_root: persist_preferences.then(|| data_root.join("projects")),
            data_root,
            project: None,
            base_layer_track: None,
            checkpoint: None,
            restored_recovery: None,
            sound_set_prompts: Vec::new(),
            sound_set_check_pending: false,
            opens: 0,
            drum: Default::default(),
            node_graph: Default::default(),
            midi_recorder: midi_record::MidiRecorder::new(),
            keysounds: dawsu_keysounding::KeysoundProject::new("", "Keysounds"),
            keysound_history: keysound_session::KeysoundUndoHistory::default(),
            bounce_cancel: bounce_plan::BounceCancelToken::new(),
            vst3: vst3::Vst3Manager::new(),
            drum_vst: None,
            drum_vst_loading: false,
            drum_vst_job: None,
            drum_vst_cancel: Arc::new(AtomicBool::new(false)),
            drum_vst_generation: 0,
            drum_vst_state_b64: None,
            drum_vst_saved: None,
            #[cfg(test)]
            drum_vst_restore_hold: None,
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }
    pub fn has_dirty_beatmap(&self) -> bool {
        self.document.as_ref().is_some_and(|d| d.composer.dirty())
    }
    pub fn has_dirty_keysounds(&self) -> bool {
        self.keysound_history.dirty
    }
    pub fn is_dirty(&self) -> bool {
        self.has_dirty_beatmap() || self.has_dirty_keysounds()
    }
    pub fn live_sample_rate(&self) -> u32 {
        self.audio.as_ref().map_or(44_100, |a| a.sample_rate())
    }

    /// Counts loads, Saves and reverts of the open difficulty: with the history version it
    /// tells whether anything read from the document is still current.
    pub fn document_generation(&self) -> u64 {
        self.document_generation
    }

    /// Counts background song/waveform completions for the open difficulty.
    pub fn waveform_revision(&self) -> u64 {
        self.waveform_revision
    }
}
impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}
fn toggle<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if let Some(i) = items.iter().position(|v| v == &item) {
        items.remove(i);
    } else {
        items.push(item);
    }
}
pub fn lane_name(identity: &SampleIdentity) -> String {
    match identity {
        SampleIdentity::File(name) => name.clone(),
        SampleIdentity::Standard { set, sound, index } => {
            let set = match set {
                SampleSet::Normal => "normal",
                SampleSet::Soft => "soft",
                SampleSet::Drum => "drum",
            };
            let sound = match sound {
                Sound::Hit(Hitsound::Normal) => "hitnormal",
                Sound::Hit(Hitsound::Whistle) => "hitwhistle",
                Sound::Hit(Hitsound::Finish) => "hitfinish",
                Sound::Hit(Hitsound::Clap) => "hitclap",
                Sound::Slider(SliderSound::Slide) => "sliderslide",
                Sound::Slider(SliderSound::Whistle) => "sliderwhistle",
                Sound::Slider(SliderSound::Tick) => "slidertick",
            };
            format!(
                "{set}-{sound}{}",
                if index.get() > 1 {
                    index.get().to_string()
                } else {
                    String::new()
                }
            )
        }
    }
}
fn safe_child(root: &Path, name: &str) -> Result<PathBuf, String> {
    check_map_path(name).map_err(|e| refusals::file_name(&e))?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let child = root
        .join(name.replace('\\', "/"))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !child.starts_with(&root) {
        return Err("A sample path leaves its folder".into());
    }
    Ok(child)
}
/// The file a lane plays: a pending file's kept copy, the mapset's file, or the skin's sound
/// (the default sounds folder's file, else the one dawsu! ships).
fn sample_path(
    lane: &Lane,
    folder: &Path,
    pending: &pending::Pending,
    defaults: &Path,
    defaults_list: &[String],
) -> Result<PathBuf, String> {
    match &lane.source {
        LaneSource::File(name) => match pending.get(name) {
            Some(file) => Ok(file.path.clone()),
            None => safe_child(folder, name),
        },
        LaneSource::Skin => {
            let name = lane_name(&lane.plays_as());
            ["wav", "ogg", "mp3"]
                .iter()
                .find_map(|ext| {
                    find_case_insensitive(defaults_list, &format!("{name}.{ext}"))
                        .and_then(|n| safe_child(defaults, n).ok())
                })
                // The chosen default sounds folder lacks it: osu!'s own sound, shipped with dawsu!.
                .or_else(|| default_sounds::path(&format!("{name}.wav")))
                .ok_or_else(|| format!("Sample unavailable: {}", lane_name(&lane.identity)))
        }
    }
}
