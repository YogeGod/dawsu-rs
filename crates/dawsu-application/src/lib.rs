//! Documents, playback, storage and background jobs.

mod bookmarks;
mod slider_body;
mod slider_export;
mod commands;
mod edit_workflows;
mod controller;
mod editor_meter;
mod export_settings;
mod generate_receipt;
mod layers;
mod mixes;
pub mod output;
mod project;
mod preferences;
mod shortcuts;
mod problems;
mod sample_watch;
pub mod waveform;
use dawsu_editor::commands::{
    CommandCatalog, EditorScope, HelpGroup, KeyMap, KeyOutcome, RunOutcome,
};
use dawsu_editor::keys::KeyChord;
use dawsu_editor::transport::BookmarkList;
pub use problems::OutputProblem;
pub use editor_meter::{EditorBarSection, EditorTimeSignature};
pub use export_settings::{ExportCombination, ExportSettings};
pub use project::LaneLayout;
pub use waveform::WaveBin;
pub use edit_workflows::{RangeOperation, TimelineEdit, VolumeScope};

use dawsu_audio::{AudioClip, AudioEngine, ScheduledSound};
use dawsu_editor::tempo::{SnapSpacing, TempoMap, TimingSection};
use dawsu_editor::tools::{PointerInput, PointerSurface, ToolBox, ToolKind};
use dawsu_editor::volume_ramp::RampShape;
use dawsu_editor_composer::{Composer, HitKey};
use dawsu_foundation::safe_write::BackupPolicy;
use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, SliderSound, SongTime, Sound};
use dawsu_hitsounding::{
    Compiled, Decompiled, Hitsounds, Lane, LaneSource, Placement, SaveProblem, compile, cue_list,
    decompile,
};
use dawsu_osu_format::{OsuFile, SaveOutcome, check_map_path, find_case_insensitive, list_folder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

pub struct Document {
    pub path: PathBuf,
    pub file: OsuFile,
    pub composer: Composer,
    pub title: String,
    pub files: Vec<String>,
    pub difficulties: Vec<PathBuf>,
    pub hs_mode: bool,
    compile_base: OsuFile,
    bookmarks: BookmarkList,
    bookmark_override: bool,
    lane_layout: Option<LaneLayout>,
    time_signatures: Vec<EditorTimeSignature>,
    editor_bars: Vec<EditorBarSection>,
    export_settings: Option<ExportSettings>,
}
impl Document {
    pub fn read(path: &Path) -> Result<Self, String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("Opening {}: {e}", path.display()))?;
        let read = OsuFile::read(std::fs::read(&path).map_err(|e| e.to_string())?);
        if read.file.format_version().is_none() {
            return Err("This is not an osu! difficulty".into());
        }
        let root = path.parent().ok_or("Missing mapset folder")?;
        let files = list_folder(root).map_err(|e| e.to_string())?;
        let h = match decompile(&read.file, &files, &|| false, &mut |_, _| {}) {
            Decompiled::Done(h) => h,
            other => return Err(format!("Cannot open difficulty: {other:?}")),
        };
        let sections = read
            .file
            .timing_points()
            .iter()
            .filter(|p| p.uninherited() && p.beat_length() > 0.0)
            .map(|p| TimingSection {
                start: SongTime::from_ms(p.time()),
                beat_length_ms: p.beat_length(),
                beats_per_bar: p.meter().unwrap_or(4).max(1) as u32,
            })
            .collect();
        let title = format!(
            "{} — {} [{}]",
            read.file.value("Metadata", "Artist").unwrap_or_default(),
            read.file.value("Metadata", "Title").unwrap_or_default(),
            read.file.value("Metadata", "Version").unwrap_or_default()
        );
        let difficulties = std::fs::read_dir(root)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("osu")))
            .collect();
        let mut composer = Composer::new(h);
        composer.tempo = TempoMap::new(sections);
        let bookmarks = bookmarks::import(&read.file);
        Ok(Self {
            path,
            file: read.file.clone(),
            compile_base: read.file,
            bookmarks,
            bookmark_override: false,
            composer,
            title,
            files,
            difficulties,
            hs_mode: false,
            lane_layout: None,
            time_signatures: Vec::new(),
            editor_bars: Vec::new(),
            export_settings: None,
        })
    }
    pub fn lane_layout(&self) -> Option<&LaneLayout> {
        self.lane_layout.as_ref()
    }
    pub fn time_signatures(&self) -> &[EditorTimeSignature] {
        &self.time_signatures
    }
    pub fn export_settings(&self) -> Option<&ExportSettings> {
        self.export_settings.as_ref()
    }
    pub fn editor_bars(&self) -> &[EditorBarSection] {
        &self.editor_bars
    }
    pub fn editor_bar_at(&self, time: SongTime) -> (dawsu_editor::tempo::BarBeat, (u32, u32)) {
        let index = self.editor_bars.partition_point(|section| section.start <= time.ms());
        if let Some(section) = self.editor_bars.get(index.saturating_sub(1)) {
            return (section.position(time.ms()), (section.numerator, section.denominator));
        }
        (self.composer.tempo.bar_beat_at(time), (self.composer.tempo.section_at(time).beats_per_bar, 4))
    }
    fn update_time_signatures(&mut self, mut signatures: Vec<EditorTimeSignature>) {
        editor_meter::normalise(&mut signatures);
        self.editor_bars = editor_meter::sections(&self.composer.tempo, &signatures);
        self.time_signatures = signatures;
    }
    pub fn duration_ms(&self) -> f64 {
        self.composer
            .hitsounds()
            .moments()
            .iter()
            .map(|m| m.time().ms())
            .fold(0.0, f64::max)
            + 2000.0
    }
}
const DIFFICULTY_CHANGED: &str =
    "The difficulty changed on disk. Your edits are kept. Reopen or save a separate copy.";

pub struct SaveReport {
    pub file: OsuFile,
    pub problems: Vec<SaveProblem>,
}
pub fn save_document(document: &Document, backups: PathBuf) -> Result<SaveReport, String> {
    save_snapshot(
        document.compile_base.clone(),
        document.composer.hitsounds().clone(),
        document.file.original_bytes().to_vec(),
        &document.path,
        backups,
        document.hs_mode,
        false,
        Path::new(""),
        &|| false,
        &mut Vec::new(),
    )
}
fn compile_for_save(
    file: &OsuFile,
    hits: &Hitsounds,
    hs: bool,
    loose_hits_saved_in_project: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<(OsuFile, Vec<SaveProblem>), String> {
    let (out, mut problems) = if hs {
        match dawsu_hitsounding::hs::compile_hs(file, hits, Placement::OnObjects, cancel, &mut |_, _| {}) {
            dawsu_hitsounding::hs::CompiledHs::Done { file, problems, .. } => (file, problems),
            _ => return Err("Save cancelled".into()),
        }
    } else {
        match compile(file, hits, Placement::GreenLines, cancel, &mut |_, _| {}) {
            Compiled::Done { file, problems } => (file, problems),
            _ => return Err("Save cancelled".into()),
        }
    };
    if loose_hits_saved_in_project && !hs {
        problems.retain(|problem| !matches!(problem.kind, dawsu_hitsounding::ProblemKind::NoObject));
    }
    Ok((out, problems))
}
fn save_snapshot(
    file: OsuFile,
    hits: Hitsounds,
    expected: Vec<u8>,
    path: &Path,
    backups: PathBuf,
    hs: bool,
    loose_hits_saved_in_project: bool,
    defaults: &Path,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
) -> Result<SaveReport, String> {
    let (mut out, mut problems) =
        compile_for_save(&file, &hits, hs, loose_hits_saved_in_project, cancel)?;
    let mut mix_files = mixes::MixFiles::default();
    let placement = if hs { Placement::OnObjects } else { Placement::GreenLines };
    if !problems.is_empty() && problems.iter().all(|problem| mixes::can_render(problem, placement)) {
        // Check before touching the mapset. The format writer checks again before replacement.
        if std::fs::read(path).map_err(|error| error.to_string())? != expected {
            return Err(DIFFICULTY_CHANGED.into());
        }
        let root = path.parent().ok_or("Missing mapset folder")?;
        let files = list_folder(root).map_err(|error| error.to_string())?;
        // This snapshot includes circles added by HS mode and all edits the first pass encoded.
        let mut target = match decompile(&out, &files, cancel, &mut |_, _| {}) {
            Decompiled::Done(target) => target,
            Decompiled::Cancelled => return Err("Save cancelled".into()),
            Decompiled::UnsupportedMode { .. } => return Err("Cannot mix this difficulty".into()),
        };
        if let Err(error) = mix_files.prepare(
            &hits, &mut target, &problems, root, defaults, &expected, &[], cancel,
        ) {
            diagnostics.extend(problems.iter().map(|problem| {
                OutputProblem::new(problem, path, path).with_message(&error)
            }));
            return Err(error);
        }
        (out, problems) = match compile(&out, &target, placement, cancel, &mut |_, _| {}) {
            Compiled::Done { file, problems } => (file, problems),
            Compiled::Cancelled => return Err("Save cancelled".into()),
        };
    }
    if !problems.is_empty() {
        diagnostics.extend(problems.iter().map(|problem| OutputProblem::new(problem, path, path)));
        return Err(format!(
            "Save needs review: {} hits cannot be written. First problem at {:.0} ms: {}",
            problems.len(),
            problems[0].time.ms(),
            problem_description(&problems[0].kind)
        ));
    }
    if cancel() {
        return Err("Save cancelled before writing".into());
    }
    let publication = mixes::publish(&mix_files.staged, cancel)?;
    match out
        .save_over_expected(path, &expected, &BackupPolicy::new(backups, 20))
        .map_err(|e| e.to_string())?
    {
        SaveOutcome::Saved { .. } | SaveOutcome::NothingToSave => {
            publication.keep();
            Ok(SaveReport {
                file: OsuFile::read(out.to_bytes()).file,
                problems,
            })
        }
        SaveOutcome::ChangedOnDisk => Err(DIFFICULTY_CHANGED.into()),
        SaveOutcome::AlreadyExists => Err("The target already exists".into()),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub recent: Vec<PathBuf>,
    pub songs_folder: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hitsound_library: Option<PathBuf>,
    pub default_samples: PathBuf,
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_buffer_ms: Option<u32>,
    pub song_gain: f32,
    pub song_muted: bool,
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
}
impl Preferences {
    fn effective_song_gain(&self) -> f32 {
        if self.song_muted { 0.0 } else { self.song_gain }
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
            audio_buffer_ms: None,
            song_gain: 0.7,
            song_muted: false,
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
        }
    }
}
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
    song: Option<Arc<AudioClip>>,
    samples: HashMap<SampleIdentity, Arc<AudioClip>>,
    waveform: Vec<WaveBin>,
    warnings: Vec<String>,
}
enum JobResult {
    Open(Option<u64>, bool, Result<Loaded, String>),
    Save {
        version: u64,
        path: PathBuf,
        saved_hits: Option<Vec<project::ProjectHit>>,
        saved_slots: Option<std::collections::BTreeMap<String, String>>,
        result: Result<SaveReport, String>,
        diagnostics: Vec<OutputProblem>,
    },
    SaveCheck {
        version: u64,
        path: PathBuf,
        hs: bool,
        result: Result<Vec<OutputProblem>, String>,
    },
    Review(Result<output::WritePlan, String>, Vec<OutputProblem>),
    Output(Result<(usize, Option<String>), String>),
    Stretched {
        path: PathBuf,
        speed: f64,
        result: Result<Arc<AudioClip>, String>,
    },
}
pub struct Application {
    sample_watch: sample_watch::Watch,
    document: Option<Document>,
    audio: Option<AudioEngine>,
    samples: HashMap<SampleIdentity, Arc<AudioClip>>,
    song: Option<Arc<AudioClip>>,
    stretched_song: Option<Arc<AudioClip>>,
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
    pub busy: bool,
    pub output_review: Option<output::WritePlan>,
    output_problems: Vec<OutputProblem>,
    output_problem_revision: Option<(u64, bool)>,
    save_problem_review: bool,
    tools: ToolBox<HitKey>,
    controller_stroke: Option<controller::ControllerStroke>,
    jobs: Option<mpsc::Receiver<JobResult>>,
    cancel: Arc<AtomicBool>,
    muted: Vec<SampleIdentity>,
    solo: Vec<SampleIdentity>,
    cached_version: u64,
    cached_listen_version: u64,
    pub zoom: f64,
    pub row_height: f32,
    project_root: Option<PathBuf>,
    project: Option<project::Project>,
    pub locator_left: f64,
    pub locator_right: f64,
    pub cycle: bool,
    pub song_offset: f64,
    pub active_lane: usize,
    playback_start: f64,
    return_position: f64,
    pub first_launch: bool,
    catalogue: Arc<CommandCatalog<Application>>,
    pub keymap: KeyMap,
    pub help_requested: bool,
    persist_preferences: bool,
}
impl Application {
    pub fn new() -> Self {
        let file = data_folder().join("preferences.json");
        let legacy = std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("OsuHitsoundEditor").join("preferences.json"));
        let (mut preferences, first_launch) = preferences::load(&file, legacy.as_deref());
        preferences.master_gain = preferences.master_gain.clamp(0.2, 1.0);
        // Local development fallback is read-only. These non-commercial assets are never bundled.
        let local = PathBuf::from(
            "E:/GitHub/dawsu-hitsounding-editor/src/OsuHitsound.Audio/DefaultSamples",
        );
        if preferences.default_samples.as_os_str().is_empty() && local.is_dir() {
            preferences.default_samples = local;
        }
        let device = preferences.device.as_deref();
        let output = AudioEngine::open_with_buffer(device, preferences.audio_buffer_ms);
        let (audio, error) = match output {
            Ok(a) => (Some(a), None),
            Err(error) if preferences.audio_buffer_ms.is_some() => match AudioEngine::open(device) {
                Ok(a) => (Some(a), Some(format!("Using the default audio buffer. {error}"))),
                Err(fallback) => (None, Some(format!("{error} Device default: {fallback}"))),
            },
            Err(error) => (None, Some(error)),
        };
        if let Some(a) = &audio {
            a.gains(preferences.effective_song_gain(), preferences.master_gain);
        }
        Self::create(preferences, first_launch, audio, error, true)
    }
    pub fn headless() -> Self {
        Self::create(Preferences::default(), false, None, None, false)
    }
    fn create(
        preferences: Preferences,
        first_launch: bool,
        audio: Option<AudioEngine>,
        error: Option<String>,
        persist_preferences: bool,
    ) -> Self {
        let catalogue = Arc::new(commands::catalogue());
        let keymap = shortcuts::keymap(catalogue.definitions(), &preferences);
        Self {
            document: None,
            sample_watch: sample_watch::Watch::default(),
            audio,
            samples: HashMap::new(),
            song: None,
            stretched_song: None,
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
            busy: false,
            output_review: None,
            output_problems: Vec::new(),
            output_problem_revision: None,
            save_problem_review: false,
            tools: ToolBox::new(),
            controller_stroke: None,
            jobs: None,
            cancel: Arc::new(AtomicBool::new(false)),
            muted: Vec::new(),
            solo: Vec::new(),
            cached_version: 0,
            cached_listen_version: 0,
            zoom: 0.12,
            row_height: 30.0,
            locator_left: 0.0,
            locator_right: 10000.0,
            cycle: false,
            song_offset: 0.0,
            active_lane: 0,
            playback_start: 0.0,
            return_position: 0.0,
            first_launch,
            catalogue,
            keymap,
            help_requested: false,
            persist_preferences,
            project_root: persist_preferences.then(|| data_folder().join("projects")),
            project: None,
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }
    pub fn is_dirty(&self) -> bool {
        self.document.as_ref().is_some_and(|d| d.composer.dirty())
    }
    pub fn is_playing(&self) -> bool {
        self.audio.as_ref().is_some_and(|a| a.is_playing())
    }
    pub fn position_ms(&self) -> f64 {
        self.audio
            .as_ref()
            .map(|a| a.position_ms())
            .unwrap_or(self.playback_start)
    }
    pub fn duration_ms(&self) -> f64 {
        self.song
            .as_ref()
            .map(|s| s.duration_ms())
            .unwrap_or_else(|| {
                self.document
                    .as_ref()
                    .map(|d| d.duration_ms())
                    .unwrap_or(10000.0)
            })
    }
    pub fn device_name(&self) -> &str {
        self.audio
            .as_ref()
            .map(|a| a.device_name.as_str())
            .unwrap_or("Audio unavailable")
    }
    pub fn devices(&self) -> Vec<String> {
        AudioEngine::devices()
    }
    pub fn select_device(&mut self, name: String) {
        if let Err(error) = self.configure_audio_output(Some(name), self.preferences.audio_buffer_ms) {
            self.error = Some(error);
        }
    }

    pub fn set_output_buffer(&mut self, buffer_ms: Option<u32>) -> Result<(), String> {
        self.configure_audio_output(self.preferences.device.clone(), buffer_ms)
    }

    fn configure_audio_output(&mut self, device: Option<String>, buffer_ms: Option<u32>) -> Result<(), String> {
        if self.busy || self.is_playing() {
            return Err("Stop playback and finish the current job before changing audio output.".into());
        }
        let cursor = self.position_ms();
        let audio = AudioEngine::open_with_buffer(device.as_deref(), buffer_ms)?;
        self.audio = Some(audio);
        self.preferences.device = device;
        self.preferences.audio_buffer_ms = buffer_ms;
        self.error = None;
        self.sync_audio();
        self.seek(cursor);
        self.cycle_changed();
        self.gains_changed();
        self.save_preferences();
        Ok(())
    }
    pub fn set_lane_layout(&mut self, mut layout: LaneLayout) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current edit before changing the lane layout".into());
        }
        let document = self.document.as_mut().ok_or("No difficulty is open")?;
        layout.normalise();
        document.lane_layout = Some(layout);
        Ok(())
    }

    pub fn save_session(&mut self) -> Result<(), String> {
        self.write_session(None, None)
    }

    pub fn set_export_settings(&mut self, settings: ExportSettings) -> Result<(), String> {
        if self.busy {
            return Err("Wait for the current job before changing export settings.".into());
        }
        settings.checked_bytes()?;
        let doc = self.document.as_mut().ok_or("Open a difficulty first.")?;
        doc.export_settings = Some(settings);
        self.output_review = None;
        Ok(())
    }

    pub fn set_time_signatures(&mut self, signatures: Vec<EditorTimeSignature>) -> Result<(), String> {
        if self.busy {
            return Err("Wait for the current job before changing time signatures".into());
        }
        if !signatures.iter().all(EditorTimeSignature::valid) {
            return Err("Each time signature needs a bar of 1 or later, 1–32 beats, and a beat unit of 1, 2, 4, 8 or 16.".into());
        }
        let document = self.document.as_mut().ok_or("Open a difficulty first")?;
        document.update_time_signatures(signatures);
        Ok(())
    }

    fn write_session(
        &mut self,
        saved_hits: Option<Vec<project::ProjectHit>>,
        saved_slots: Option<std::collections::BTreeMap<String, String>>,
    ) -> Result<(), String> {
        let Some(document) = &self.document else {
            return Ok(());
        };
        let settings = project::SessionSettings {
            unanchored: saved_hits.or_else(|| {
                self.project
                    .as_ref()
                    .and_then(|p| p.settings.unanchored.clone())
            }),
            song_offset: self.song_offset,
            locator_left: self.locator_left,
            locator_right: Some(self.locator_right),
            cycle: self.cycle,
            zoom: Some(self.zoom),
            row_height: Some(self.row_height),
            view_start: document.composer.view_start,
            bookmarks: document.bookmark_override.then(|| {
                document.bookmarks.items().iter().map(|mark| project::ProjectBookmark {
                    time: mark.time.ms(),
                    label: mark.label.clone(),
                }).collect()
            }),
            lane_layout: document.lane_layout.clone(),
            hs_mode: Some(document.hs_mode),
            time_signatures: Some(document.time_signatures.clone()),
            export_settings: document.export_settings.clone(),
            lane_slots: saved_slots.or_else(|| self.project.as_ref().and_then(|p| p.settings.lane_slots.clone())),
        };
        if let Some(project) = &mut self.project {
            project
                .save(settings)
                .map_err(|e| format!("Saving session settings: {e}"))?;
        }
        Ok(())
    }

    pub fn save_preferences(&mut self) {
        if !self.persist_preferences {
            return;
        }
        let root = data_folder();
        let result = std::fs::create_dir_all(&root)
            .map_err(|e| e.to_string())
            .and_then(|_| serde_json::to_vec_pretty(&self.preferences).map_err(|e| e.to_string()))
            .and_then(|bytes| {
                dawsu_foundation::safe_write::write_new(&root.join("preferences.json"), &bytes)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            self.error = Some(e);
        }
    }
    pub fn request_open(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        if self.is_dirty() {
            self.pending_open = Some(path);
            return;
        }
        self.open_now(path);
    }
    pub fn confirm_revert(&mut self) {
        if self.busy || !std::mem::take(&mut self.revert_requested) {
            return;
        }
        if let Some(path) = self.document.as_ref().map(|d| d.path.clone()) {
            self.pending_open = None;
            self.start_open(path, false);
        }
    }

    pub fn discard_and_open(&mut self) {
        if self.busy {
            return;
        }
        if let Some(path) = self.pending_open.take() {
            self.open_now(path);
        }
    }
    fn open_now(&mut self, path: PathBuf) {
        self.start_open(path, true);
    }

    fn start_open(&mut self, path: PathBuf, keep_settings: bool) {
        self.cancel_timeline_edit();
        if keep_settings && let Err(error) = self.save_session() {
            self.error = Some(error);
            return;
        }
        self.stop();
        self.cancel_controller_stroke();
        if let Some(d) = self.document.as_mut() {
            self.tools.cancel(&mut d.composer);
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let defaults = self.preferences.default_samples.clone();
        let project_root = self.project_root.clone();
        let legacy_root = self
            .persist_preferences
            .then(|| std::env::var_os("LOCALAPPDATA"))
            .flatten()
            .map(|base| {
                PathBuf::from(base)
                    .join("OsuHitsoundEditor")
                    .join("projects")
            });
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        let outgoing = self.document.as_ref().map(|d| d.composer.history.version());
        self.status = "Opening map and decoding audio…".into();
        std::thread::spawn(move || {
            let result = (|| {
                let path = if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("osz"))
                {
                    let imports = data_folder().join("imports");
                    std::fs::create_dir_all(&imports).map_err(|e| e.to_string())?;
                    match dawsu_osu_format::import_osz(
                        &path,
                        &imports,
                        Default::default(),
                        &|| cancel.load(Ordering::Relaxed),
                        &mut |_, _| {},
                    )
                    .map_err(|e| e.to_string())?
                    {
                        dawsu_osu_format::OszImport::Imported { folder } => {
                            std::fs::read_dir(folder)
                                .map_err(|e| e.to_string())?
                                .filter_map(Result::ok)
                                .map(|e| e.path())
                                .find(|p| {
                                    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu"))
                                })
                                .ok_or("Archive contains no difficulty")?
                        }
                        other => return Err(format!("Import refused: {other:?}")),
                    }
                } else {
                    path
                };
                load(
                    &path,
                    &defaults,
                    &cancel,
                    project_root.as_deref(),
                    legacy_root.as_deref(),
                )
            })();
            let _ = send.send(JobResult::Open(outgoing, keep_settings, result));
        });
    }
    pub fn cancel_job(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.status = "Cancelling…".into();
    }
    pub fn collect_retired_audio(&self) -> bool {
        self.audio
            .as_ref()
            .is_some_and(AudioEngine::collect_retired)
    }

    pub fn poll(&mut self) {
        let result = match self.jobs.as_ref().map(|r| r.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.jobs = None;
                self.busy = false;
                self.error = Some(
                    "The background job stopped unexpectedly. Your open edits are kept.".into(),
                );
                None
            }
            _ => None,
        };
        if let Some(result) = result {
            self.jobs = None;
            self.busy = false;
            match result {
                JobResult::Open(outgoing, keep_settings, Ok(loaded)) => {
                    if self.document.as_ref().map(|d| d.composer.history.version()) != outgoing {
                        self.pending_open = Some(loaded.document.path);
                        self.status =
                            "The outgoing difficulty changed. Review your edits before switching."
                                .into();
                        return;
                    }
                    if keep_settings && let Err(error) = self.save_session() {
                        self.error = Some(error);
                        return;
                    }
                    let path = loaded.document.path.clone();
                    self.document = Some(loaded.document);
                    self.output_problems.clear();
                    self.output_problem_revision = None;
                    self.save_problem_review = false;
                    self.sample_watch = sample_watch::Watch::default();
                    if let Some(d) = self.document.as_mut() {
                        d.composer.protect_normals = self.preferences.protect_hitnormals;
                    }
                    self.song = loaded.song;
                    self.stretched_song = self.song.clone();
                    self.playback_speed = 1.0;
                    self.samples = loaded.samples;
                    self.waveform = loaded.waveform;
                    self.warnings = loaded.warnings;
                    self.muted.clear();
                    self.solo.clear();
                    self.active_lane = 0;
                    self.preferences.recent.retain(|p| p != &path);
                    self.preferences.recent.insert(0, path);
                    self.preferences.recent.truncate(20);
                    self.project = loaded.project;
                    let settings = self
                        .project
                        .as_ref()
                        .map(|project| project.settings.clone())
                        .unwrap_or_default();
                    let duration = self.duration_ms();
                    self.locator_left = settings.locator_left.clamp(0.0, duration);
                    self.locator_right = settings
                        .locator_right
                        .unwrap_or(duration)
                        .clamp(0.0, duration);
                    self.cycle = settings.cycle && self.locator_left < self.locator_right;
                    self.song_offset = settings.song_offset;
                    self.zoom = settings.zoom.unwrap_or(0.12).clamp(0.005, 3.0);
                    self.row_height = settings.row_height.unwrap_or(30.0).clamp(20.0, 65.0);
                    self.set_view_start(settings.view_start.clamp(0.0, duration));
                    self.seek(0.0);
                    self.cycle_changed();
                    self.sync_audio();
                    self.status = "Difficulty loaded".into();
                    self.save_preferences();
                }
                JobResult::Open(_, _, Err(e)) => {
                    self.error = Some(e);
                    self.status = "Open failed; previous document kept".into();
                }
                JobResult::Stretched {
                    path,
                    speed,
                    result,
                } => {
                    if self.document.as_ref().is_some_and(|d| d.path == path) {
                        match result {
                            Ok(clip) => {
                                self.stretched_song = Some(clip);
                                self.playback_speed = speed;
                                self.sync_audio();
                                self.status = format!("Playback speed: {:.0}%", speed * 100.0);
                            }
                            Err(e) => self.error = Some(e),
                        }
                    }
                }
                JobResult::Review(result, diagnostics) => {
                    self.output_problems = diagnostics;
                    self.save_problem_review = false;
                    match result {
                        Ok(plan) => {
                            self.output_review = Some(plan);
                            self.status = "Review the output before writing".into();
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.status = "Output planning stopped; no files written".into();
                        }
                    }
                }
                JobResult::SaveCheck { version, path, hs, result } => {
                    if !self.document.as_ref().is_some_and(|d| {
                        d.path == path && d.composer.history.version() == version && d.hs_mode == hs
                    }) {
                        self.status = "The difficulty changed. Review Save again.".into();
                    } else {
                        match result {
                            Ok(problems) if problems.is_empty() => self.save(),
                            Ok(problems) => {
                                self.status = "Review problems before saving".into();
                                self.output_problems = problems;
                                self.save_problem_review = true;
                            }
                            Err(error) => {
                                self.error = Some(error);
                                self.status = "Save review did not complete".into();
                            }
                        }
                    }
                }
                JobResult::Output(result) => match result {
                    Ok((count, tracking_warning)) => {
                        self.status = format!("Wrote {count} reviewed files with backups");
                        if let Some(warning) = tracking_warning {
                            self.warnings.push(warning);
                            self.status = format!("Wrote {count} reviewed files. Generate replacement could not be recorded.");
                        }
                        if let Some(d) = self.document.as_mut() {
                            if let Ok(entries) = std::fs::read_dir(d.path.parent().unwrap()) {
                                d.difficulties = entries
                                    .filter_map(Result::ok)
                                    .map(|e| e.path())
                                    .filter(|p| {
                                        p.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu"))
                                    })
                                    .collect();
                            }
                        }
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.status = "Output did not complete".into();
                    }
                },
                JobResult::Save {
                    version,
                    path,
                    saved_hits,
                    saved_slots,
                    result,
                    diagnostics,
                } => {
                    self.output_problems = diagnostics;
                    match result {
                        Ok(report) => {
                            if let Some(d) = self.document.as_mut() {
                                if d.path == path {
                                    d.file = report.file;
                                }
                            }
                            let current = self.document.as_ref().is_some_and(|d| {
                                d.path == path && d.composer.history.version() == version
                            });
                            if let Err(error) = self.write_session(saved_hits, saved_slots) {
                                self.error = Some(error);
                                self.status =
                                    "Difficulty saved. Session settings could not be saved.".into();
                                return;
                            }
                            if !current {
                                self.status =
                                    "Saved the earlier revision. Newer edits are still open.".into();
                                return;
                            }
                            self.document.as_mut().unwrap().composer.mark_saved();
                            self.status = "Saved with backup".into();
                            if let Some(path) = self.pending_open.take() {
                                self.open_now(path);
                            }
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.status = "Save did not complete; edits kept".into();
                        }
                    }
                }
            }
        }
        self.poll_sample_folder();
        if self.document.as_ref().is_some_and(|d| {
            d.composer.history.version() != self.cached_version
                || d.composer.listen_version != self.cached_listen_version
        }) {
            self.sync_audio();
        }
        let requests = self
            .document
            .as_mut()
            .map(|d| std::mem::take(&mut d.composer.audition_requests))
            .unwrap_or_default();
        if self.preferences.audition {
            for lane in requests {
                self.audition(&lane);
            }
        }
    }
    pub fn output_problems(&self) -> &[OutputProblem] {
        let revision = self
            .document
            .as_ref()
            .map(|doc| (doc.composer.history.version(), doc.hs_mode));
        if revision == self.output_problem_revision {
            &self.output_problems
        } else {
            &[]
        }
    }

    pub fn save_problem_review(&self) -> bool {
        self.save_problem_review && !self.output_problems().is_empty()
    }

    pub fn dismiss_save_problem_review(&mut self) {
        self.save_problem_review = false;
    }

    pub fn review_save(&mut self) {
        if self.busy {
            return;
        }
        if self.is_stroke_open() {
            self.error = Some("Finish or cancel the current drag before saving.".into());
            return;
        }
        let Some(d) = &self.document else {
            return;
        };
        let file = d.compile_base.clone();
        let hits = d.composer.hitsounds().clone();
        let path = d.path.clone();
        let version = d.composer.history.version();
        let hs = d.hs_mode;
        let has_project = self.project.is_some();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.save_problem_review = false;
        self.output_problems.clear();
        self.output_problem_revision = Some((version, hs));
        self.status = "Checking Save…".into();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        std::thread::spawn(move || {
            let result = compile_for_save(&file, &hits, hs, has_project, &|| cancel.load(Ordering::Relaxed))
                .map(|(_, problems)| {
                    let placement = if hs { Placement::OnObjects } else { Placement::GreenLines };
                    problems.iter().filter(|problem| !mixes::can_render(problem, placement))
                        .map(|problem| OutputProblem::new(problem, &path, &path)).collect()
                });
            let _ = send.send(JobResult::SaveCheck { version, path, hs, result });
        });
    }

    pub fn save(&mut self) {
        self.save_with_backups(data_folder().join("backups"));
    }

    fn save_with_backups(&mut self, backups: PathBuf) {
        if self.busy {
            return;
        }
        if self.is_stroke_open() {
            self.error = Some("Finish or cancel the current drag before saving.".into());
            return;
        }
        let Some(d) = &self.document else {
            return;
        };
        let file = d.compile_base.clone();
        let expected = d.file.original_bytes().to_vec();
        let hits = d.composer.hitsounds().clone();
        let path = d.path.clone();
        let version = d.composer.history.version();
        let hs = d.hs_mode;
        // A successful HS save puts all loose hits in the difficulty, including merged voices.
        // Clear the saved loose rows only when the write completes.
        let saved_hits = self.project.as_ref().map(|_| {
            if hs {
                Vec::new()
            } else {
                project_hits(d)
            }
        });
        let has_project = saved_hits.is_some();
        let saved_slots = has_project.then(|| hits.lane_slots()
            .map(|(source, slot)| (project::lane_key(source), project::lane_key(slot)))
            .collect());
        let defaults = self.preferences.default_samples.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.save_problem_review = false;
        self.output_problems.clear();
        self.output_problem_revision = Some((version, hs));
        self.status = "Compiling and saving…".into();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        std::thread::spawn(move || {
            let mut diagnostics = Vec::new();
            let result = save_snapshot(
                file,
                hits,
                expected,
                &path,
                backups,
                hs,
                has_project,
                &defaults,
                &|| cancel.load(Ordering::Relaxed),
                &mut diagnostics,
            );
            let _ = send.send(JobResult::Save {
                version,
                path,
                saved_hits,
                saved_slots,
                result,
                diagnostics,
            });
        });
    }
    pub fn play_pause(&mut self) {
        self.play_pause_from(None);
    }
    pub fn play_pause_from(&mut self, start: Option<f64>) {
        if self.document.is_none() {
            return;
        }
        if self.is_playing() {
            self.stop();
            return;
        }
        if let Some(start) = start {
            self.seek(start);
        } else {
            self.playback_start = self.position_ms();
        }
        self.return_position = self.playback_start;
        if let Some(a) = &self.audio {
            a.play(true);
        }
    }
    pub fn stop(&mut self) {
        if let Some(a) = &self.audio {
            a.play(false);
            if self.preferences.return_on_stop {
                a.seek(self.return_position);
            }
        }
    }
    pub fn seek(&mut self, ms: f64) {
        let ms = ms.clamp(0.0, self.duration_ms());
        self.playback_start = ms;
        if let Some(a) = &self.audio {
            a.seek(ms);
        }
    }
    pub fn gains_changed(&mut self) {
        self.preferences.master_gain = self.preferences.master_gain.clamp(0.2, 1.0);
        if let Some(a) = &self.audio {
            a.gains(
                self.preferences.effective_song_gain(),
                self.preferences.master_gain,
            );
        }
    }
    pub fn cycle_changed(&mut self) {
        if let Some(a) = &self.audio {
            a.cycle(
                self.cycle
                    .then_some((self.locator_left, self.locator_right)),
            );
            a.offset(self.song_offset);
        }
    }
    fn clip_for_lane(&self, lane: &SampleIdentity) -> Option<&Arc<AudioClip>> {
        let model = self.document()?.composer.hitsounds().lane(lane)?;
        self.samples.get(&model.plays_as())
    }
    pub fn audition(&self, lane: &SampleIdentity) {
        if let (Some(a), Some(clip)) = (&self.audio, self.clip_for_lane(lane)) {
            a.audition(clip.clone(), 1.0);
        }
    }
    pub fn audition_sample_file(&self, source: &Path) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before previewing a sample.".into());
        }
        let clip = dawsu_audio::decode(source)?;
        if let Some(audio) = &self.audio {
            audio.audition(clip, 1.0);
        }
        Ok(())
    }
    pub fn is_muted(&self, lane: &SampleIdentity) -> bool {
        self.muted.contains(lane)
    }
    pub fn is_solo(&self, lane: &SampleIdentity) -> bool {
        self.solo.contains(lane)
    }
    pub fn mute(&mut self, lane: SampleIdentity) {
        toggle(&mut self.muted, lane);
        self.sync_audio();
    }
    pub fn solo(&mut self, lane: SampleIdentity) {
        toggle(&mut self.solo, lane);
        self.sync_audio();
    }
    pub fn set_speed(&mut self, speed: f64) {
        if self.busy {
            return;
        }
        let (Some(song), Some(d)) = (self.song.clone(), self.document.as_ref()) else {
            return;
        };
        if (speed - 1.0).abs() < 0.001 {
            self.stretched_song = Some(song);
            self.playback_speed = 1.0;
            self.sync_audio();
            return;
        }
        let path = d.path.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Preparing pitch-preserving playback…".into();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        std::thread::spawn(move || {
            let result = dawsu_audio::stretch(&song, speed).and_then(|clip| {
                if cancel.load(Ordering::Relaxed) {
                    Err("Speed change cancelled".into())
                } else {
                    Ok(clip)
                }
            });
            let _ = send.send(JobResult::Stretched {
                path,
                speed,
                result,
            });
        });
    }
    pub fn grid_seek(&mut self, direction: i32) {
        self.grid_seek_steps(direction, 1);
    }
    fn grid_seek_steps(&mut self, direction: i32, count: u8) {
        let time = SongTime::from_ms(self.position_ms());
        if let Some(d) = self.document.as_ref() {
            let composer = &d.composer;
            let target = (0..count).fold(time, |at, _| {
                composer.tempo.grid_step(at, direction, composer.spacing)
            });
            self.seek(target.ms());
        }
    }
    pub fn set_tool(&mut self, tool: ToolKind) {
        self.cancel_controller_stroke();
        if let Some(d) = self.document.as_mut() {
            self.tools.select_tool(tool, &mut d.composer);
        }
    }
    pub fn tool(&self) -> ToolKind {
        self.tools.active()
    }
    pub fn ramp_shape(&self) -> RampShape {
        self.tools.ramp_shape
    }
    pub fn set_ramp_shape(&mut self, shape: RampShape) {
        if self.document.is_some() && !self.busy && !self.is_stroke_open() {
            self.tools.ramp_shape = shape;
        }
    }
    fn is_stroke_open(&self) -> bool {
        self.controller_stroke.is_some() || self.tools.is_stroke_open()
    }
    fn cancel_controller_stroke(&mut self) {
        if self.controller_stroke.take().is_some() && let Some(d) = self.document.as_mut() {
            d.composer.history.cancel();
        }
    }
    pub fn pointer(&mut self, phase: PointerPhase, input: PointerInput) {
        if self.busy {
            return;
        }
        let Some(d) = self.document.as_mut() else { return };
        if let Some(stroke) = &mut self.controller_stroke {
            match phase {
                PointerPhase::Down => {}
                PointerPhase::Move => stroke.apply(&mut d.composer, input),
                PointerPhase::Up => {
                    stroke.apply(&mut d.composer, input);
                    d.composer.history.commit();
                    self.controller_stroke = None;
                }
                PointerPhase::Cancel => self.cancel_controller_stroke(),
            }
            return;
        }
        if matches!(phase, PointerPhase::Down) {
            if input.surface == PointerSurface::Grid {
                self.active_lane = input.row.max(0) as usize;
            } else if !input.mods.shift() && self.tools.active() != ToolKind::Ramp {
                if !self.tools.is_stroke_open() {
                    self.controller_stroke = controller::ControllerStroke::start(&mut d.composer, input, self.zoom);
                }
                return;
            }
        }
        match phase {
            PointerPhase::Down => self.tools.down(&mut d.composer, input),
            PointerPhase::Move => self.tools.moved(&mut d.composer, input),
            PointerPhase::Up => self.tools.up(&mut d.composer, input),
            PointerPhase::Cancel => self.tools.cancel(&mut d.composer),
        }
    }
    pub fn go_to_history(&mut self, position: usize) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        let slot = self.active_lane_slot();
        if let Some(document) = self.document.as_mut() {
            document.composer.history.go_to(position);
            document.composer.selected.clear();
        }
        self.restore_active_lane(slot);
    }

    fn active_lane_slot(&self) -> Option<SampleIdentity> {
        let composer = &self.document()?.composer;
        let lane = composer.lanes.get(self.active_lane)?;
        Some(composer.hitsounds().lane_slot(lane).unwrap_or(lane).clone())
    }

    fn restore_active_lane(&mut self, slot: Option<SampleIdentity>) {
        if let (Some(slot), Some(document)) = (slot, &self.document) {
            let hits = document.composer.hitsounds();
            let physical = hits.lane_slots().find_map(|(source, logical)| (logical == &slot).then_some(source)).unwrap_or(&slot);
            if let Some(row) = document.composer.lanes.iter().position(|lane| lane == physical) {
                self.active_lane = row;
            }
        }
    }

    pub fn set_view_start(&mut self, ms: f64) {
        if let Some(d) = self.document.as_mut() {
            d.composer.view_start = ms.max(0.0);
        }
    }
    pub fn set_snap(&mut self, enabled: bool, division: u32) {
        if let Some(d) = self.document.as_mut() {
            d.composer.snap_enabled = enabled;
            d.composer.spacing = SnapSpacing::fraction(division.max(1)).unwrap();
        }
    }
    pub fn set_normal_protection(&mut self, on: bool) {
        self.preferences.protect_hitnormals = on;
        if let Some(d) = self.document.as_mut() {
            d.composer.protect_normals = on;
        }
    }
    pub fn set_snap_options(
        &mut self,
        enabled: bool,
        beats: u32,
        divisions: u32,
        mode: dawsu_editor::tempo::SnapMode,
    ) {
        if let Some(d) = self.document.as_mut() {
            if let Ok(spacing) = SnapSpacing::new(beats, divisions) {
                d.composer.snap_enabled = enabled;
                d.composer.spacing = spacing;
                d.composer.snap_mode = mode;
            }
        }
    }
    pub fn set_insert_volume(&mut self, v: i32) {
        if let Some(d) = self.document.as_mut() {
            d.composer.insert_volume = v.clamp(0, 100);
        }
    }
    pub fn selection_volume(&mut self, v: i32, relative: bool) {
        if self.busy {
            return;
        }
        if let Some(d) = self.document.as_mut() {
            d.composer.volume(v, relative);
        }
    }
    pub fn set_hs_mode(&mut self, hs: bool) {
        if self.busy {
            return;
        }
        if let Some(d) = self.document.as_mut() {
            d.hs_mode = hs;
        }
    }
    pub fn select_hit(&mut self, key: HitKey, add: bool) {
        if let Some(d) = self.document.as_mut() {
            if add {
                d.composer.selected.add(key);
            } else {
                d.composer.selected.replace([key]);
            }
        }
    }
    fn review_job(
        &mut self,
        prepare: impl FnOnce(output::Source, Arc<AtomicBool>, &mut Vec<OutputProblem>) -> Result<output::WritePlan, String>
        + Send
        + 'static,
    ) {
        if self.busy {
            return;
        }
        if self.is_stroke_open() {
            self.error = Some("Finish or cancel the current drag before preparing output.".into());
            return;
        }
        let Some(d) = self.document.as_ref() else {
            return;
        };
        let source = output::Source::from_document(d);
        self.output_problems.clear();
        self.output_problem_revision = Some((source.version, d.hs_mode));
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Preparing output review…".into();
        std::thread::spawn(move || {
            let mut diagnostics = Vec::new();
            let result = prepare(source, cancel, &mut diagnostics);
            let _ = send.send(JobResult::Review(result, diagnostics));
        });
    }
    pub fn generate(&mut self) {
        self.generate_with_slider_audio(false);
    }

    pub fn generate_with_slider_audio(&mut self, include_slider_audio: bool) {
        let defaults = self.preferences.default_samples.clone();
        let settings = self.document.as_ref().and_then(|document| document.export_settings.clone());
        let projects = self.project_root.clone();
        self.review_job(move |source, cancel, diagnostics| {
            output::generate_report(&source, &defaults, settings.as_ref(), projects.as_deref(),
                include_slider_audio, &|| cancel.load(Ordering::Relaxed), diagnostics)
        });
    }
    pub fn new_diff(&mut self, name: String) {
        self.review_job(move |source, _, _| output::new_diff(&source, &name));
    }
    pub fn copy_hitsounds(
        &mut self,
        targets: Vec<PathBuf>,
        options: dawsu_hitsounding::copy::CopyOptions,
    ) {
        let defaults = self.preferences.default_samples.clone();
        self.review_job(move |source, cancel, diagnostics| {
            output::copy_report(&source, &defaults, &targets, &options, &|| cancel.load(Ordering::Relaxed), diagnostics)
        });
    }
    pub fn audition_output_mix(&self, index: usize) -> Result<(), String> {
        let clip = self.output_review.as_ref()
            .and_then(|plan| plan.mix_clip(index))
            .ok_or("This sample mix is no longer in the review.")?;
        let audio = self.audio.as_ref().ok_or("Audio output is unavailable.")?;
        audio.audition(Arc::new(clip), 1.0);
        Ok(())
    }

    pub fn apply_output(&mut self) {
        self.apply_output_in(data_folder().join("backups"));
    }

    fn apply_output_in(&mut self, backups: PathBuf) {
        if self.busy {
            return;
        }
        let Some(plan) = self.output_review.take() else {
            return;
        };
        if !self
            .document
            .as_ref()
            .is_some_and(|d| d.path == plan.source && d.composer.history.version() == plan.version)
        {
            self.error =
                Some("The source changed since this review. Prepare the output again.".into());
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Writing reviewed output…".into();
        std::thread::spawn(move || {
            let result = plan.apply(&backups, &|| cancel.load(Ordering::Relaxed))
                .map(|count| (count, generate_receipt::record(&plan).err()));
            let _ = send.send(JobResult::Output(result));
        });
    }
    pub fn command(&mut self, id: &str) {
        let catalogue = self.catalogue.clone();
        if let RunOutcome::Failed { message, .. } = catalogue.run(self, id, EditorScope::Hitsound) {
            self.error = Some(message);
        }
    }
    pub fn key(&mut self, chord: KeyChord) {
        let catalogue = self.catalogue.clone();
        let keymap = self.keymap.clone();
        if let KeyOutcome::Command {
            outcome: RunOutcome::Failed { message, .. },
            ..
        } = catalogue.handle_key(self, &keymap, chord, EditorScope::Hitsound)
        {
            self.error = Some(message);
        }
    }
    pub fn help(&self) -> Vec<HelpGroup> {
        self.catalogue.help(&self.keymap)
    }
    pub fn choose_key_preset(&mut self, preset: &str) {
        self.preferences.key_preset = preset.into();
        self.keymap = shortcuts::keymap(self.catalogue.definitions(), &self.preferences);
        self.first_launch = false;
        self.save_preferences();
    }
    /// Replaces the active lane's editable hits, retaining its logical slot.
    pub fn replace_lane_sample(
        &mut self,
        expected_map: &Path,
        expected_lane: &SampleIdentity,
        source: &Path,
    ) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before replacing a sample.".into());
        }
        let d = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let current = d.composer.lanes.get(self.active_lane);
        if d.path != expected_map || current != Some(expected_lane) {
            return Err("The active lane changed while choosing a sample.".into());
        }
        if matches!(expected_lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. }) {
            return Err("Slider body samples are read-only here.".into());
        }

        let root = d.path.parent().ok_or("Missing mapset folder")?;
        let source = source.canonicalize().map_err(|e| format!("Opening {}: {e}", source.display()))?;
        let extension = source.extension().and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase).filter(|ext| matches!(ext.as_str(), "wav" | "ogg" | "mp3"))
            .ok_or("Choose a WAV, OGG, or MP3 sample.")?;
        let hits = d.composer.hitsounds();
        let logical = hits.lane_slot(expected_lane).unwrap_or(expected_lane);
        let file_path = |lane: &SampleIdentity| match &hits.lane(lane)?.source {
            LaneSource::File(name) => safe_child(root, name).ok(),
            LaneSource::Skin => None,
        };
        if file_path(expected_lane).as_ref() == Some(&source) {
            return Ok(());
        }
        let existing = source.starts_with(root).then(|| hits.lanes().iter().find(|candidate| {
            let lane = &candidate.identity;
            let compatible = match (expected_lane, lane) {
                (SampleIdentity::Standard { set, sound, .. }, SampleIdentity::Standard { set: other_set, sound: other_sound, .. }) =>
                    set == other_set && sound == other_sound,
                (SampleIdentity::File(_), SampleIdentity::File(_)) => true,
                _ => false,
            };
            compatible && self.clip_for_lane(lane).is_some()
                && (lane == logical || hits.lane_slot(lane).is_none())
                && !hits.lane_slots().any(|(_, slot)| slot == lane)
                && !hits.moments().iter().any(|moment| moment.hits().any(|used| used == lane))
                && file_path(lane).as_ref() == Some(&source)
        })).flatten().map(|lane| lane.identity.clone());
        let replacement = if let Some(lane) = existing {
            lane
        } else {
            let bytes = std::fs::read(&source).map_err(|e| format!("Reading sample: {e}"))?;
            let clip = dawsu_audio::decode(&source)?;
            if !std::fs::read(&source).is_ok_and(|current| current == bytes) {
                return Err("The sample changed while it was being read. Choose it again.".into());
            }
            let name = layers::create_backing(d, expected_lane, &extension, &bytes)?;
            let lane = dawsu_hitsounding::rules::read_sample_file_name(&name)
                .unwrap_or_else(|| SampleIdentity::File(name.clone()));
            self.register_added_samples(vec![(name, clip)]);
            lane
        };
        let composer = &mut self.document.as_mut().expect("open document").composer;
        let selected = composer.selected.items().iter().cloned().map(|mut hit| {
            if hit.lane == *expected_lane {
                hit.lane = replacement.clone();
            }
            hit
        }).collect::<Vec<_>>();
        composer.apply("Replace lane sample", |hits| hits.replace_lane(expected_lane, replacement.clone()));
        composer.selected.replace(selected);
        self.active_lane = composer.lanes.iter().position(|lane| lane == &replacement)
            .expect("the backing lane is registered before replacing hits");
        self.status = format!("Assigned {}", source.file_name().unwrap_or_default().to_string_lossy());
        self.sync_audio();
        Ok(())
    }

    fn add_numbered_layer(&mut self) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before adding a sample layer.".into());
        }
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let family = document.composer.lanes.get(self.active_lane).ok_or("Choose a sample family.")?;
        let (name, clip) = layers::create(document, &self.preferences.default_samples, family)?;
        let lane = dawsu_hitsounding::rules::read_sample_file_name(&name)
            .expect("numbered layer names are constructed from sample identities");
        self.register_added_samples(vec![(name.clone(), clip)]);
        if let Some(document) = &self.document
            && let Some(row) = document.composer.lanes.iter().position(|identity| *identity == lane)
        {
            self.active_lane = row;
        }
        self.status = format!("Added {name} with the default family sound.");
        Ok(())
    }

    fn poll_sample_folder(&mut self) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let additions = self.sample_watch.poll(document);
        self.register_added_samples(additions);
    }

    fn register_added_samples(&mut self, additions: Vec<(String, Arc<AudioClip>)>) {
        if additions.is_empty() {
            return;
        }
        let Some(document) = &mut self.document else {
            return;
        };
        let mut names = Vec::with_capacity(additions.len());
        for (name, clip) in additions {
            self.samples.insert(SampleIdentity::File(name.clone()), clip);
            names.push(name);
        }
        document.files.extend_from_slice(&names);
        document.files.sort();
        document.files.dedup();
        let composer = &mut document.composer;
        composer.history.update_external(|hits| hits.add_folder_samples(&names));
        for lane in composer.history.document().lanes() {
            if !composer.lanes.contains(&lane.identity) {
                composer.lanes.push(lane.identity.clone());
            }
        }
    }

    fn scheduled_sounds(&self, d: &Document) -> Vec<ScheduledSound> {
        let composer = &d.composer;
        let hits = composer.hitsounds();
        // Per-hit mute is keyed by moment and lane. Apply it before the cue list
        // resolves multiple lanes to possibly the same physical sample.
        let mut filtered;
        let audible = if composer.listen_muted.is_empty() {
            hits
        } else {
            filtered = hits.clone();
            for key in &composer.listen_muted {
                if key.moment < filtered.moments().len() {
                    filtered.remove_hit(dawsu_hitsounding::MomentId(key.moment), &key.lane);
                }
            }
            &filtered
        };
        cue_list(audible, &self.muted, &self.solo)
            .into_iter()
            .filter_map(|cue| {
                Some(ScheduledSound {
                    at_ms: cue.time().ms(),
                    end_ms: cue.loop_end().map(|t| t.ms()),
                    clip: self.samples.get(cue.sample())?.clone(),
                    gain: cue.gain() as f32,
                })
            })
            .collect()
    }
    fn sync_audio(&mut self) {
        if let Some(d) = &self.document {
            self.cached_version = d.composer.history.version();
            self.cached_listen_version = d.composer.listen_version;
            let cues = self.scheduled_sounds(d);
            if let Some(a) = &self.audio {
                a.load_at_speed(
                    self.stretched_song.clone().or_else(|| self.song.clone()),
                    cues,
                    self.playback_speed,
                );
                a.gains(
                    self.preferences.effective_song_gain(),
                    self.preferences.master_gain,
                );
            }
        }
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
    check_map_path(name).map_err(|e| format!("Unsafe file name: {e:?}"))?;
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
fn sample_path(
    lane: &Lane,
    folder: &Path,
    defaults: &Path,
    defaults_list: &[String],
) -> Result<PathBuf, String> {
    match &lane.source {
        LaneSource::File(name) => safe_child(folder, name),
        LaneSource::Skin => {
            let name = lane_name(&lane.plays_as());
            ["wav", "ogg", "mp3"]
                .iter()
                .find_map(|ext| {
                    find_case_insensitive(defaults_list, &format!("{name}.{ext}"))
                        .and_then(|n| safe_child(defaults, n).ok())
                })
                .ok_or_else(|| format!("Sample unavailable: {}", lane_name(&lane.identity)))
        }
    }
}

fn load(
    path: &Path,
    defaults: &Path,
    cancel: &AtomicBool,
    project_root: Option<&Path>,
    legacy_root: Option<&Path>,
) -> Result<Loaded, String> {
    let mut document = Document::read(path)?;
    let mut warnings = Vec::new();
    let project = project_root.and_then(|root| {
        match project::Project::load(root, legacy_root, &document.path) {
            Ok(project) => Some(project),
            Err(error) => {
                warnings.push(format!(
                    "Session settings unavailable: {error}. The existing project file was kept."
                ));
                None
            }
        }
    });
    if let Some(saved) = project
        .as_ref()
        .and_then(|p| p.settings.unanchored.as_deref())
    {
        restore_project_hits(&mut document.composer, saved)?;
    }
    if let Some(slots) = project.as_ref().and_then(|p| p.settings.lane_slots.as_ref()) {
        let composer = &mut document.composer;
        composer.history.update_external(|hits| {
            for (source, slot) in slots {
                if let (Some(source), Some(slot)) = (project::lane_identity(source), project::lane_identity(slot)) {
                    hits.bind_lane_slot(source, slot);
                }
            }
        });
        composer.lanes = composer.hitsounds().lanes().iter().map(|lane| lane.identity.clone()).collect();
    }
    if let Some(saved) = project.as_ref().and_then(|p| p.settings.bookmarks.as_deref()) {
        document.bookmarks = BookmarkList::default();
        for mark in saved {
            if mark.time.is_finite() && mark.time >= 0.0 {
                document.bookmarks.add(SongTime::from_ms(mark.time), mark.label.clone());
            }
        }
        document.bookmark_override = true;
    }
    document.lane_layout = project.as_ref().and_then(|p| p.settings.lane_layout.clone());
    document.hs_mode = project.as_ref().and_then(|p| p.settings.hs_mode).unwrap_or(false);
    if let Some(signatures) = project.as_ref().and_then(|p| p.settings.time_signatures.clone()) {
        document.update_time_signatures(signatures);
    }
    document.export_settings = project.as_ref().and_then(|p| p.settings.export_settings.clone());
    if let Some(layout) = &mut document.lane_layout {
        layout.normalise();
    }
    let folder = document.path.parent().ok_or("Missing folder")?;
    let song = if let Some(name) = document.file.general("AudioFilename") {
        match safe_child(folder, &name)
            .and_then(|p| dawsu_audio::decode_with_cancel(&p, &|| cancel.load(Ordering::Relaxed)))
        {
            Ok(audio) => Some(audio),
            Err(e) => {
                warnings.push(format!("Song audio unavailable: {e}"));
                None
            }
        }
    } else {
        None
    };
    let mut samples = HashMap::new();
    let defaults_list = if defaults.is_dir() {
        list_folder(defaults).unwrap_or_default()
    } else {
        Vec::new()
    };
    for lane in document.composer.hitsounds().lanes() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Open cancelled".into());
        }
        let source = lane.plays_as();
        if samples.contains_key(&source) {
            continue;
        }
        match sample_path(lane, folder, defaults, &defaults_list)
            .and_then(|path| dawsu_audio::decode_with_cancel(&path, &|| cancel.load(Ordering::Relaxed)))
        {
            Ok(clip) => {
                samples.insert(source, clip);
            }
            Err(error) => warnings.push(error),
        }
    }
    let waveform = song
        .as_ref()
        .map(|audio| waveform::analyse(audio))
        .unwrap_or_default();
    if cancel.load(Ordering::Relaxed) {
        return Err("Open cancelled".into());
    }
    Ok(Loaded {
        document,
        project,
        song,
        samples,
        waveform,
        warnings,
    })
}

fn project_hits(document: &Document) -> Vec<project::ProjectHit> {
    let mut saved = Vec::new();
    for (group, moment) in document
        .composer
        .hitsounds()
        .moments()
        .iter()
        .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject)
        .enumerate()
    {
        for lane in moment.hits() {
            let key = project::lane_key(lane);
            saved.push(project::ProjectHit {
                group: Some(group),
                lane: key,
                time: moment.time().ms(),
                volume: Some(moment.volume()),
            });
        }
    }
    saved
}

fn restore_project_hits(
    composer: &mut Composer,
    saved: &[project::ProjectHit],
) -> Result<(), String> {
    if saved.is_empty() {
        return Ok(());
    }
    let mut hits = composer.hitsounds().clone();
    let mut anchored = HashMap::new();
    for m in hits.moments() {
        for lane in m.hits() {
            anchored
                .entry((m.time().ms().to_bits(), lane.clone(), m.volume()))
                .and_modify(|count| *count += 1)
                .or_insert(1usize);
        }
    }
    let mut groups = HashMap::new();
    let mut voices = HashMap::new();
    for hit in saved {
        let lane = project::lane_identity(&hit.lane)
            .ok_or_else(|| format!("Cannot restore project hit on lane {}", hit.lane))?;
        let volume = hit.volume.unwrap_or(100).clamp(0, 100);
        let time = hit.time.to_bits();
        if let Some(count) = anchored.get_mut(&(time, lane.clone(), volume)) {
            if *count > 0 {
                *count -= 1;
                continue;
            }
        }
        // Legacy rows have no moment identifier. Co-timed lanes share volume;
        // repeated voices on the same lane need distinct moments.
        let group = hit.group.unwrap_or_else(|| {
            let voice = voices.entry((time, volume, lane.clone())).or_insert(0);
            let index = *voice;
            *voice += 1;
            index
        });
        let moment = groups
            .entry((time, volume, group))
            .or_insert_with(|| hits.add_moment(SongTime::from_ms(hit.time), volume));
        hits.add_hit(*moment, lane);
    }
    let tempo = std::mem::take(&mut composer.tempo);
    *composer = Composer::new(hits);
    composer.tempo = tempo;
    Ok(())
}

fn problem_description(problem: &dawsu_hitsounding::ProblemKind) -> &'static str {
    use dawsu_hitsounding::ProblemKind::*;
    match problem {
        NoObject => "This hit has no object. Enable HS mode or Generate a hitsound difficulty.",
        NeedsSampleMix(_) => {
            "This hit needs a sample mix that the current Save mode cannot write."
        }
        NeedsGreenLine => "This hit needs a green line that the output mode does not allow.",
        LookupWindowTaken => "Another lookup overlaps the timing line needed by this hit.",
        LineRefused => "This object cannot hold the requested change.",
        BodyVolumeNeedsHead => "The slider body volume conflicts with its head.",
    }
}

#[cfg(test)]
mod project_workflow {
    use super::*;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    fn open(app: &mut Application, path: PathBuf) {
        app.request_open(path);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn regenerate_checks_owned_output_reuses_mix_and_backs_up_replacement() {
        let fixture = Scratch::new("checked-regenerate");
        let source = fixture.path("source.osu");
        let projects = fixture.path("sessions");
        let backups = fixture.path("backups");
        let body = b"osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Song\nCreator:Mapper\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:50:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n";
        std::fs::write(&source, body).unwrap();
        for (name, amplitude) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
            wav.extend(amplitude.to_le_bytes());
            std::fs::write(fixture.path(name), wav).unwrap();
        }
        let wait = |app: &mut Application| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let mut app = Application::headless();
        app.project_root = Some(projects.clone());
        open(&mut app, source.clone());
        app.generate();
        wait(&mut app);
        let review = app.output_review.as_ref().unwrap();
        let target = review.files[0].path.clone();
        let mix = review.mix_paths().next().unwrap().to_path_buf();
        assert!(!target.exists() && !mix.exists());
        app.apply_output_in(backups.clone());
        wait(&mut app);
        assert!(app.error.is_none());
        let initial = std::fs::read(&target).unwrap();
        let rendered = std::fs::read(&mix).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(projects);
        open(&mut app, source.clone());
        app.document.as_mut().unwrap().composer.add(1500.0, SampleIdentity::File("one.wav".into()), 60, 1.0);
        app.generate();
        wait(&mut app);
        let replacement = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        assert_eq!(replacement.files[0].expected.as_deref(), Some(initial.as_slice()));
        assert_eq!(replacement.mix_paths().collect::<Vec<_>>(), [mix.as_path()]);
        assert_eq!(std::fs::read(&target).unwrap(), initial);
        app.apply_output_in(backups.clone());
        wait(&mut app);
        assert!(app.error.is_none());
        assert_ne!(std::fs::read(&target).unwrap(), initial);
        assert_eq!(std::fs::read(&mix).unwrap(), rendered);
        let backup_dir = std::fs::read_dir(&backups).unwrap().next().unwrap().unwrap().path();
        let backup = std::fs::read_dir(backup_dir).unwrap().next().unwrap().unwrap().path();
        assert_eq!(std::fs::read(backup).unwrap(), initial);
        std::fs::write(&target, b"Another editor changed the output").unwrap();
        app.generate();
        wait(&mut app);
        assert!(app.output_review.is_none());
        assert!(app.error.as_deref().unwrap().contains("changed since Generate"));
        assert_eq!(std::fs::read(&target).unwrap(), b"Another editor changed the output");
        assert_eq!(std::fs::read(&source).unwrap(), body);
    }

    #[test]
    fn generate_uses_legacy_rule_to_pair_incompatible_roles_after_reopen() {
        let fixture = Scratch::new("generate-paired-preset");
        let map = fixture.path("Artist - Song (Mapper) [Hard].osu");
        let body = b"osu file format v14\n[General]\nMode:0\n[Metadata]\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,1:0:2:50:\n";
        std::fs::write(&map, body).unwrap();
        let mut layers = Vec::new();
        for (name, amplitude) in [("normal-hitnormal2.wav", 8192_i16), ("soft-hitclap3.wav", 4096), ("drum-hitfinish4.wav", 16384)] {
            let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
            wav.extend(amplitude.to_le_bytes());
            std::fs::write(fixture.path(name), &wav).unwrap();
            let hash = dawsu_foundation::digest::sha256_hex(&wav).to_ascii_uppercase();
            layers.push(format!("hit:BeatmapFile:{name}@{hash}@balance:1"));
        }
        layers.sort();
        let key = serde_json::to_string(&layers).unwrap();
        let mut settings = ExportSettings::default();
        settings.pair_samples = true;
        settings.rules.insert(key, "soft-hitwhistle".into());
        let path = fixture.path("merge-preset.json");
        let export = serde_json::to_vec_pretty(&settings).unwrap();
        std::fs::write(&path, &export).unwrap();
        let loaded = ExportSettings::read_file(&path).unwrap();
        assert_eq!(loaded, settings);
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        open(&mut app, map.clone());
        app.set_export_settings(loaded).unwrap();
        app.save_session().unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        open(&mut app, map.clone());
        assert_eq!(app.document().unwrap().export_settings(), Some(&settings));
        let d = app.document.as_mut().unwrap();
        for (set, sound, index) in [(SampleSet::Soft, Hitsound::Clap, 3), (SampleSet::Drum, Hitsound::Finish, 4)] {
            d.composer.add(1000.0, SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: dawsu_foundation::CustomIndex::new(index) }, 50, 1.0);
        }
        app.generate();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let plan = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        let names = plan.mix_paths().map(|path| path.file_name().unwrap().to_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(names, ["normal-hitnormal5.wav", "soft-hitfinish5.wav", "soft-hitclap5.wav"]);
        assert!(plan.mix_paths().all(|path| !path.exists()));
        assert!(!plan.files[0].path.exists());
        let target = plan.files[0].path.clone();
        app.apply_output_in(fixture.path("backups"));
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        let reopened = Document::read(&target).unwrap();
        let moment = &reopened.composer.hitsounds().moments()[0];
        assert_eq!(moment.volume(), 100);
        assert_eq!(moment.hits().count(), 3);
        for (file, amplitude) in names.iter().zip([0.125, 0.25, 0.0625]) {
            assert_eq!(dawsu_audio::decode(&fixture.path(file)).unwrap().samples, [amplitude; 2]);
        }
        app.generate();
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let replacement = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        assert_eq!(replacement.mix_paths().map(|path| path.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(), names);
        assert_eq!(std::fs::read(&map).unwrap(), body);
        assert_eq!(std::fs::read(&path).unwrap(), export);
    }

    #[test]
    fn bookmark_commands_navigate_imported_and_new_bookmarks_per_difficulty() {
        let fixture = Scratch::new("bookmark-commands");
        let first = fixture.path("first.osu");
        let second = fixture.path("second.osu");
        let map = include_str!("../tests/project-fixture.osu");
        let original = map.replace("[Metadata]", "[Editor]\nBookmarks: 100,500\n[Metadata]");
        std::fs::write(&first, &original).unwrap();
        std::fs::write(&second, map).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        open(&mut app, first.clone());

        app.seek(0.0);
        app.command("nextBookmark");
        assert_eq!(app.position_ms(), 100.0, "next bookmark should navigate to the first map bookmark");
        app.seek(750.0);
        app.command("addBookmark");
        app.command("previousBookmark");
        assert_eq!(app.position_ms(), 500.0);
        app.command("nextBookmark");
        assert_eq!(app.position_ms(), 750.0);
        app.rename_bookmark(750.0, "Chorus".into()).unwrap();
        app.seek(1250.0);
        app.add_bookmark(Some("Break".into())).unwrap();
        open(&mut app, second.clone());
        app.seek(0.0);
        app.command("nextBookmark");
        assert_eq!(app.position_ms(), 0.0);
        open(&mut app, first.clone());
        let marks = app.document().unwrap().bookmarks();
        assert_eq!(marks[2].label.as_deref(), Some("Chorus"));
        assert_eq!(marks[3].label.as_deref(), Some("Break"));
        app.seek(500.0);
        app.command("nextBookmark");
        assert_eq!(app.position_ms(), 750.0);
        app.rename_bookmark(750.0, " ".into()).unwrap();
        assert_eq!(app.document().unwrap().bookmarks()[2].label, None);
        app.command("removeBookmark");
        for time in [100.0, 500.0, 1250.0] {
            app.remove_bookmark(time).unwrap();
        }
        open(&mut app, first.clone());
        assert!(app.document().unwrap().bookmarks().is_empty(), "the saved empty list overrides imported marks");
        assert_eq!(std::fs::read(&first).unwrap(), original.as_bytes());
        assert_eq!(std::fs::read(&second).unwrap(), map.as_bytes());
    }

    #[test]
    fn lane_layout_is_read_per_difficulty_without_writing_the_map() {
        let fixture = Scratch::new("lane-layout");
        let path = fixture.path("first.osu");
        let other = fixture.path("second.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&path, original).unwrap();
        std::fs::write(&other, original).unwrap();
        let projects = fixture.path("projects");
        let mut app = Application::headless();
        app.project_root = Some(projects.clone());
        let mut project = project::Project::load(&projects, None, &path.canonicalize().unwrap()).unwrap();
        let settings = project::SessionSettings {
            lane_layout: Some(project::LaneLayout {
                folder_order: vec!["drum".into(), "normal".into(), "soft".into()],
                collapsed_folders: ["normal".into()].into(),
                collapsed_families: ["soft-hitclap".into()].into(),
            }),
            ..Default::default()
        };
        project.save(settings.clone()).unwrap();
        open(&mut app, path.clone());
        assert_eq!(app.document().unwrap().lane_layout(), settings.lane_layout.as_ref());
        let updated = LaneLayout {
            folder_order: vec!["soft".into(), "drum".into(), "normal".into()],
            collapsed_folders: ["drum".into()].into(),
            collapsed_families: ["normal-hitfinish".into()].into(),
        };
        app.set_lane_layout(updated.clone()).unwrap();
        app.save_session().unwrap();
        open(&mut app, other);
        assert!(app.document().unwrap().lane_layout().is_none());
        app.save_session().unwrap();
        open(&mut app, path.clone());
        assert_eq!(app.document().unwrap().lane_layout(), Some(&updated));
        assert_eq!(std::fs::read(path).unwrap().as_slice(), original);
    }

    #[test]
    fn explicit_play_start_keeps_the_return_point_when_seeking() {
        let fixture = Scratch::new("play-start");
        let path = fixture.path("test.osu");
        std::fs::write(&path, include_bytes!("../tests/project-fixture.osu")).unwrap();
        let mut app = Application::headless();
        app.document = Some(Document::read(&path).unwrap());
        app.seek(700.0);
        app.play_pause_from(Some(500.0));
        assert_eq!(app.position_ms(), 500.0);
        assert_eq!(app.return_position, 500.0);
        app.seek(850.0);
        assert_eq!(app.return_position, 500.0);
        app.play_pause_from(None);
        assert_eq!(app.return_position, 850.0);
    }

    #[test]
    fn replacing_a_lane_keeps_the_slot_and_saves_a_playable_new_backing_sample() {
        let scratch = Scratch::new("lane-replacement");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("test.osu");
        let sibling = folder.join("other.osu");
        let original_hit = "256,192,1000,1,0,1:1:1:67:";
        let unrelated_hit = "320,192,1250,1,8,2:2:0:91:";
        let source = include_str!("../tests/project-fixture.osu")
            .replace("256,192,1000,1,8,0:0:0:0:", &format!("{original_hit}\n{unrelated_hit}"));
        let sibling_bytes = source.replace(original_hit, "256,192,1000,1,0,1:1:3:67:");
        std::fs::write(&map, &source).unwrap();
        std::fs::write(&sibling, &sibling_bytes).unwrap();
        let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
        let mut quieter = wav.to_vec();
        *quieter.last_mut().unwrap() = 0x20;
        let external = scratch.path("replacement.wav");
        let backing = folder.join("normal-hitnormal4.wav");
        std::fs::write(&external, &quieter).unwrap();
        for name in ["normal-hitnormal.wav", "normal-hitnormal2.wav"] {
            std::fs::write(folder.join(name), wav).unwrap();
        }

        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        let old = dawsu_hitsounding::rules::read_sample_file_name("normal-hitnormal.wav").unwrap();
        let new = dawsu_hitsounding::rules::read_sample_file_name("normal-hitnormal4.wav").unwrap();
        app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &old).unwrap();
        let map_key = app.document().unwrap().path.clone();
        assert!(app.replace_lane_sample(&sibling, &old, &external).is_err());
        assert!(app.replace_lane_sample(&map_key, &new, &external).is_err());
        assert!(!backing.exists());
        app.replace_lane_sample(&map_key, &old, &external).unwrap();
        let composer = &app.document().unwrap().composer;
        assert_eq!(composer.lanes[app.active_lane], new);
        assert_eq!(composer.hitsounds().lane_slot(&new), Some(&old));
        assert!(composer.hitsounds().moments()[0].hits().any(|sample| sample == &new));
        assert_eq!(composer.hitsounds().moments()[0].volume(), 67);
        assert!(app.is_dirty());
        assert_eq!(app.clip_for_lane(&new).unwrap().samples[0], 0.25);
        assert_eq!(std::fs::read(&backing).unwrap(), quieter);
        assert_eq!(std::fs::read(&map).unwrap(), source.as_bytes());

        app.command("undo");
        let composer = &app.document().unwrap().composer;
        assert_eq!(composer.lanes[app.active_lane], old);
        assert!(composer.hitsounds().moments()[0].hits().any(|sample| sample == &old));
        assert!(composer.hitsounds().lane_slot(&new).is_none());
        assert!(!app.is_dirty());
        app.command("redo");
        assert_eq!(app.document().unwrap().composer.lanes[app.active_lane], new);

        app.save_with_backups(scratch.path("backups"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.is_dirty());
        let saved = std::fs::read(&map).unwrap();
        assert!(saved.windows(unrelated_hit.len()).any(|window| window == unrelated_hit.as_bytes()));
        assert_eq!(std::fs::read(&sibling).unwrap(), sibling_bytes.as_bytes());
        assert_eq!(std::fs::read(&external).unwrap(), quieter);
        assert_eq!(std::fs::read(folder.join("normal-hitnormal.wav")).unwrap(), wav);

        let mut reopened = Application::headless();
        reopened.project_root = Some(scratch.path("projects"));
        open(&mut reopened, map);
        let hits = reopened.document().unwrap().composer.hitsounds();
        assert_eq!(hits.lane_slot(&new), Some(&old));
        assert!(hits.moments()[0].hits().any(|sample| sample == &new));
        assert_eq!(reopened.clip_for_lane(&new).unwrap().samples[0], 0.25);
    }

    #[test]
    fn add_numbered_layers_skips_reserved_indexes_and_preserves_mapset_files() {
        let scratch = Scratch::new("numbered-layers");
        let folder = scratch.path("mapset");
        let defaults = scratch.path("skin");
        for directory in [&folder, &defaults] {
            std::fs::create_dir(directory).unwrap();
        }
        let map = folder.join("test.osu");
        let sibling = folder.join("other.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        let other = include_str!("../tests/project-fixture.osu")
            .replace("256,192,1000,1,8,0:0:0:0:", "256,192,1000,1,8,1:2:3:50:");
        std::fs::write(&map, original).unwrap();
        std::fs::write(&sibling, &other).unwrap();
        let tone = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
        let occupied = folder.join("SOFT-HITCLAP2.wav");
        std::fs::write(&occupied, tone).unwrap();
        std::fs::write(folder.join("drum-hitfinish7.wav"), tone).unwrap();
        let default_file = defaults.join("soft-hitclap.wav");
        std::fs::write(&default_file, tone).unwrap();

        let mut app = Application::headless();
        app.preferences.default_samples = defaults;
        open(&mut app, map.clone());
        let family = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &family).unwrap();
        app.command("addNumberedLayer");
        assert!(app.error.is_none(), "{:?}", app.error);
        let first = folder.join("soft-hitclap8.wav");
        assert_eq!(std::fs::read(&first).expect("added numbered layer"), tone);

        let mut quieter = tone.to_vec();
        *quieter.last_mut().unwrap() = 0x20;
        std::fs::write(&default_file, &quieter).unwrap();
        app.command("addNumberedLayer");
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(std::fs::read(folder.join("soft-hitclap9.wav")).unwrap(), quieter);
        assert_eq!(std::fs::read(&occupied).unwrap(), tone);
        assert_eq!(std::fs::read(&first).unwrap(), tone);
        assert_eq!(std::fs::read(&map).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&sibling).unwrap(), other);
        assert!(!app.is_dirty());
        let first_lane = dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap8.wav").unwrap();
        let second_lane = dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap9.wav").unwrap();
        {
            let composer = &mut app.document.as_mut().unwrap().composer;
            composer.add(1000.0, first_lane.clone(), 100, 0.0);
            composer.add(1500.0, second_lane.clone(), 80, 0.0);
        }
        assert_eq!(app.clip_for_lane(&first_lane).unwrap().samples, [0.5]);
        assert_eq!(app.clip_for_lane(&second_lane).unwrap().samples, [0.25]);
        let cues = app.scheduled_sounds(app.document().unwrap());
        assert!(cues.iter().any(|c| c.at_ms == 1000.0 && c.clip.samples == [0.5]));
        assert!(cues.iter().any(|c| c.at_ms == 1500.0 && c.clip.samples == [0.25]));
    }

    #[test]
    fn sample_added_to_open_map_is_playable_without_losing_edits_or_redo() {
        let scratch = Scratch::new("sample-watch");
        let map = scratch.path("test.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let mut app = Application::headless();
        open(&mut app, map.clone());

        let clap = SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        {
            let composer = &mut app.document.as_mut().unwrap().composer;
            composer.add(1500.0, clap.clone(), 80, 0.0);
            composer.add(2000.0, clap, 80, 0.0);
            composer.undo();
        }
        let name = "soft-hitfinish2.wav";
        let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
        std::fs::write(scratch.path(name), wave).unwrap();
        let lane = dawsu_hitsounding::rules::read_sample_file_name(name).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while app.clip_for_lane(&lane).is_none() {
            app.poll();
            assert!(Instant::now() < deadline, "new sample did not become playable");
            std::thread::sleep(Duration::from_millis(10));
        }
        let document = app.document().unwrap();
        assert!(document.files.iter().any(|file| file == name));
        assert!(document.composer.lanes.contains(&lane));
        assert!(document.composer.dirty());
        assert!(document.composer.history.can_redo());
        assert_eq!(document.composer.history.steps(), ["Draw hit", "Draw hit"]);

        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.redo();
        assert!(composer.hitsounds().lane(&lane).is_some());
        assert!(composer.hitsounds().moments().iter().any(|m| m.time().ms() == 2000.0));
        composer.add(1000.0, lane, 100, 0.0);
        let document = app.document().unwrap();
        let cues = app.scheduled_sounds(document);
        assert!(cues.iter().any(|cue| cue.at_ms == 1000.0 && cue.clip.samples == [0.5]));
        assert_eq!(std::fs::read(map).unwrap(), original);
    }

    #[test]
    fn transport_plays_numbered_map_additions_alongside_default_hitnormal() {
        let dir = Scratch::new("addition-playback");
        let folder = dir.path("mapset");
        let defaults = dir.path("skin");
        for path in [&folder, &defaults] {
            std::fs::create_dir(path).unwrap();
        }
        let map = folder.join("test.osu");
        let fixture = include_str!("../tests/project-fixture.osu")
            .replace("256,192,1000,1,8,0:0:0:0:", "256,192,1000,1,14,1:2:2:50:");
        std::fs::write(&map, fixture).unwrap();
        let header = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0";
        for (path, amplitude) in [
            (defaults.join("normal-hitnormal.wav"), 4096i16),
            (folder.join("soft-hitwhistle2.wav"), 8192),
            (folder.join("soft-hitfinish2.wav"), 12288),
            (folder.join("soft-hitclap2.wav"), 16384),
        ] {
            let mut wav = header.to_vec();
            wav.extend(amplitude.to_le_bytes());
            std::fs::write(path, wav).unwrap();
        }
        let loaded = load(&map, &defaults, &AtomicBool::new(false), None, None).unwrap();
        let mut app = Application::headless();
        app.document = Some(loaded.document);
        app.samples = loaded.samples;
        let mix = |app: &Application| {
            let mut mixer = dawsu_audio::Mixer::new(48_000);
            mixer.cues = app.scheduled_sounds(app.document().unwrap());
            mixer.seek(1000.0);
            mixer.playing = true;
            let mut frame = [0.0];
            mixer.render(&mut frame, 1);
            (mixer.cues.len(), frame[0])
        };
        assert_eq!(mix(&app), (4, 0.625));
        let clap = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(2),
        };
        let clip = app.clip_for_lane(&clap).unwrap();
        assert_eq!(clip.samples[0], 0.5);
        app.document.as_mut().unwrap().composer.listen_muted.push(HitKey {
            moment: 0,
            lane: clap,
        });
        assert_eq!(mix(&app), (3, 0.375));
    }

    #[test]
    fn time_signatures_change_editor_bars_without_resnapping_or_writing_map_bytes() {
        let scratch = Scratch::new("editor-meter");
        let map = scratch.path("Hard.osu");
        let sessions = scratch.path("sessions");
        let original = b"osu file format v14\r\n\r\n[General]\r\nMode:0\r\n\r\n[TimingPoints]\r\n0,500,4,1,1,100,1,0\r\n4000,250,4,1,1,100,1,0\r\n\r\n[HitObjects]\r\n256,192,2000,1,0,0:0:0:0:\r\n";
        std::fs::write(&map, original).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(sessions.clone());
        open(&mut app, map.clone());
        app.set_time_signatures(vec![EditorTimeSignature { bar: 2, numerator: 7, denominator: 8 }]).unwrap();
        let document = app.document().unwrap();
        let point = SongTime::from_ms(3750.0);
        let (bar, meter) = document.editor_bar_at(point);
        assert_eq!((bar.bar, bar.beat, meter), (3, 1, (7, 8)));
        let (bar_at_red, red_meter) = document.editor_bar_at(SongTime::from_ms(4000.0));
        assert_eq!((bar_at_red.bar, bar_at_red.beat, red_meter), (4, 1, (7, 8)));
        assert_eq!(document.composer.tempo.snap(point, SnapSpacing::new(1, 1).unwrap()).ms(), 4000.0);
        assert!(!app.is_dirty());
        app.save_session().unwrap();
        let mut reopened = Application::headless();
        reopened.project_root = Some(sessions);
        open(&mut reopened, map.clone());
        assert_eq!(reopened.document().unwrap().editor_bar_at(point), (bar, meter));
        assert_eq!(std::fs::read(&map).unwrap(), original);
    }

    #[test]
    fn hs_mode_is_restored_per_difficulty_without_writing_map_bytes() {
        let fixture = Scratch::new("hs-mode-state");
        let a = fixture.path("Hard.osu");
        let b = fixture.path("Easy.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        for path in [&a, &b] {
            std::fs::write(path, original).unwrap();
        }
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("sessions"));
        open(&mut app, a.clone());
        app.set_hs_mode(true);
        assert!(!app.is_dirty());
        open(&mut app, b.clone());
        assert!(!app.document().unwrap().hs_mode);
        open(&mut app, a.clone());
        assert!(app.document().unwrap().hs_mode);
        app.set_hs_mode(false);
        app.save_session().unwrap();
        let mut reopened = Application::headless();
        reopened.project_root = app.project_root.clone();
        open(&mut reopened, a.clone());
        assert!(!reopened.document().unwrap().hs_mode);
        for path in [a, b] {
            assert_eq!(std::fs::read(path).unwrap(), original);
        }
    }

    #[test]
    fn hs_save_mix_reopens_without_duplicate_project_hits() {
        let scratch = Scratch::new("hs-save-mix");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let wave_header = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0";
        for (name, frames) in [
            ("normal-hitnormal2.wav", [8192i16, 0]),
            ("soft-hitclap3.wav", [0, 8192i16]),
        ] {
            let mut wav = wave_header.to_vec();
            wav.extend(frames.iter().flat_map(|value| value.to_le_bytes()));
            std::fs::write(folder.join(name), wav).unwrap();
        }
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        let composer = &mut app.document.as_mut().unwrap().composer;
        // Distinct unanchored moments round to one file time, with two different gains.
        composer.add(10_000.0, SampleIdentity::File("normal-hitnormal2.wav".into()), 80, 0.0);
        composer.add(10_000.25, SampleIdentity::File("soft-hitclap3.wav".into()), 40, 0.0);
        // Save the loose hits once so the HS save must clear the now-materialised project rows.
        let save = |app: &mut Application| {
            app.save_with_backups(scratch.path("backups"));
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(app.error.is_none(), "{:?}", app.error);
        };
        save(&mut app);
        assert_eq!(std::fs::read(&map).unwrap(), original);
        app.set_hs_mode(true);
        save(&mut app);
        assert!(!app.is_dirty());
        let saved = std::fs::read(&map).unwrap();
        assert!(saved.starts_with(original));
        let restart = |app: &mut Application| {
            app.project_root = Some(scratch.path("projects"));
            open(app, map.clone());
        };
        let mut reopened = Application::headless();
        restart(&mut reopened);
        let document = reopened.document().unwrap();
        let hits = document.composer.hitsounds();
        assert_eq!(project_hits(document).len(), 0);
        let [moment] = hits.moments().iter()
            .filter(|moment| moment.time().ms() >= 10_000.0)
            .collect::<Vec<_>>()[..] else {
            panic!("expected exactly one saved circle");
        };
        let [SampleIdentity::File(name)] = moment.hits().collect::<Vec<_>>()[..] else {
            panic!("expected one generated WAV");
        };
        let mix = dawsu_audio::decode(&folder.join(name)).unwrap();
        assert_eq!(mix.samples.len(), 4);
        for (actual, expected) in mix.samples.iter().zip([0.2, 0.2, 0.1, 0.1]) {
            assert!((actual - expected).abs() < 0.0001, "{actual} vs {expected}");
        }
        save(&mut app);
        save(&mut reopened);
        assert_eq!(std::fs::read(&map).unwrap(), saved);
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 4);
    }


    #[test]
    fn loose_lanes_reopen_as_one_moment_beside_an_unrelated_object() {
        let dir = Scratch::new("grouped-project-hits");
        let path = dir.path("map.osu");
        std::fs::write(&path, include_bytes!("../tests/project-fixture.osu")).unwrap();
        let mut doc = Document::read(&path).unwrap();
        let rows = ["Skin|soft-hitclap", "Skin|soft-hitwhistle"].map(|name| project::ProjectHit {
            lane: name.into(),
            time: 1000.0,
            volume: Some(75),
            group: None,
        });
        restore_project_hits(&mut doc.composer, &rows).unwrap();
        let grouped = project_hits(&doc);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].group, grouped[1].group);
        let mut reopened = Document::read(&path).unwrap();
        restore_project_hits(&mut reopened.composer, &grouped).unwrap();
        let loose: Vec<_> = reopened
            .composer
            .hitsounds()
            .moments()
            .iter()
            .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject)
            .collect();
        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].hits().count(), 2);
    }

    #[test]
    fn legacy_clap_survives_edit_save_reopen_and_deletion() {
        let dir = Scratch::new("legacy-clap");
        let map = dir.path("a.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let path = map.canonicalize().unwrap();
        let legacy = dir.path("legacy");
        let root = dir.path("sessions");
        std::fs::create_dir(&legacy).unwrap();
        let key = path
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase();
        let hash = dawsu_foundation::digest::sha256_hex(key.as_bytes());
        let source = legacy.join(format!("A-{}.json", hash[..12].to_uppercase()));
        let bytes =
            br#"{"Version":3,"Unanchored":[{"Lane":"Skin|soft-hitclap","Time":250,"Volume":63}]}"#;
        std::fs::write(&source, bytes).unwrap();
        let load_map = || {
            load(
                &path,
                dir.dir(),
                &AtomicBool::new(false),
                Some(&root),
                Some(&legacy),
            )
            .unwrap()
        };
        let mut app = Application::headless();
        let loaded = load_map();
        app.project = loaded.project;
        app.document = Some(loaded.document);
        let hit = project_hits(app.document().unwrap()).pop().unwrap();
        assert_eq!(
            (hit.lane.as_str(), hit.time, hit.volume),
            ("Skin|soft-hitclap", 250.0, Some(63))
        );
        assert!(!app.is_dirty());
        let c = &mut app.document.as_mut().unwrap().composer;
        let last = c.hitsounds().moments().len() - 1;
        let lane = c.hitsounds().moments()[last].hits().next().unwrap().clone();
        c.selected.replace([HitKey { moment: last, lane }]);
        app.selection_volume(77, false);
        app.save();
        let wait = |app: &mut Application| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert!(app.error.is_none(), "{:?}", app.error);
        };
        wait(&mut app);
        assert_eq!(project_hits(&load_map().document)[0].volume, Some(77));
        app.document.as_mut().unwrap().composer.remove_selected();
        app.save();
        wait(&mut app);
        assert!(project_hits(&load_map().document).is_empty());
        assert_eq!(std::fs::read(source).unwrap(), bytes);
        assert_eq!(std::fs::read(map).unwrap(), original);
    }

    #[test]
    fn session_conflict_keeps_the_outgoing_map_until_explicit_revert() {
        let fixture = Scratch::new("switch-conflict");
        let map = fixture.path("a.osu");
        let other = fixture.path("b.osu");
        let root = fixture.path("sessions");
        for path in [&map, &other] {
            std::fs::write(path, include_bytes!("../tests/project-fixture.osu")).unwrap();
        }
        let mut app = Application::headless();
        app.project_root = Some(root.clone());
        open(&mut app, map.clone());
        app.song_offset = 5.0;
        app.save_session().unwrap();
        let mut external =
            project::Project::load(&root, None, &map.canonicalize().unwrap()).unwrap();
        external
            .save(project::SessionSettings {
                song_offset: 15.0,
                ..external.settings.clone()
            })
            .unwrap();
        app.song_offset = 10.0;
        app.request_open(other);
        assert!(!app.busy);
        assert_eq!(app.document().unwrap().path, map.canonicalize().unwrap());
        assert_eq!(app.song_offset, 10.0);
        assert!(app.error.take().unwrap().contains("changed on disk"));
        app.revert_requested = true;
        app.confirm_revert();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(app.song_offset, 15.0);
    }

    #[test]
    fn difficulty_switch_protects_inspector_edits() {
        let fixture = Scratch::new("switch-inspector");
        let a = fixture.path("a.osu");
        let b = fixture.path("b.osu");
        for path in [&a, &b] {
            std::fs::write(path, include_bytes!("../tests/project-fixture.osu")).unwrap();
        }
        let mut app = Application::headless();
        open(&mut app, a);
        app.document.as_mut().unwrap().composer.select_all();
        app.request_open(b);
        assert!(app.busy);
        app.selection_volume(7, false);
        assert!(!app.is_dirty(), "the inspector edited an outgoing document");
    }

    #[test]
    fn reverting_reloads_disk_and_requires_an_explicit_confirm() {
        let fixture = Scratch::new("revert-workflow");
        let path = fixture.path("map.osu");
        let data = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&path, data).unwrap();
        let mut app = Application::headless();
        open(&mut app, path.clone());
        app.document.as_mut().unwrap().composer.select_all();
        app.command("volumeDown");
        assert!(app.is_dirty());
        app.command("revert");
        assert!(app.revert_requested);
        assert!(app.is_dirty());
        app.confirm_revert();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.is_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }

    #[test]
    fn reopening_a_difficulty_restores_its_session_without_changing_the_map() {
        let fixture = Scratch::new("project-workflow");
        let map = fixture.path("map.osu");
        let other = fixture.path("other.osu");
        let data = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, data).unwrap();
        std::fs::write(&other, data).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("application-data"));
        open(&mut app, map.clone());
        app.song_offset = -42.5;
        app.locator_left = 100.0;
        app.locator_right = 900.0;
        app.cycle = true;
        app.zoom = 0.35;
        app.set_view_start(125.0);
        open(&mut app, other);
        assert_eq!(app.song_offset, 0.0);
        open(&mut app, map.clone());
        assert_eq!(app.song_offset, -42.5);
        assert_eq!(
            (app.locator_left, app.locator_right, app.cycle),
            (100.0, 900.0, true)
        );
        assert_eq!(app.zoom, 0.35);
        assert_eq!(app.document().unwrap().composer.view_start, 125.0);
        assert_eq!(std::fs::read(&map).unwrap(), data);
    }
}
