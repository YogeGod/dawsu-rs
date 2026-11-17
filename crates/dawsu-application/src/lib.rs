//! Documents, playback, storage and background jobs.

mod commands;
pub mod output;
mod project;
pub mod waveform;
use dawsu_editor::commands::{
    CommandCatalog, EditorScope, HelpGroup, KeyMap, KeyOutcome, KeyPreset, RunOutcome,
};
use dawsu_editor::keys::KeyChord;
pub use waveform::WaveBin;

use dawsu_audio::{AudioClip, AudioEngine, ScheduledSound};
use dawsu_editor::tempo::{SnapSpacing, TempoMap, TimingSection};
use dawsu_editor::tools::{PointerInput, ToolBox, ToolKind};
use dawsu_editor_composer::{Composer, HitKey};
use dawsu_foundation::safe_write::BackupPolicy;
use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, SliderSound, SongTime, Sound};
use dawsu_hitsounding::{
    Compiled, Decompiled, Hitsounds, LaneSource, Placement, SaveProblem, compile, cue_list,
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
        Ok(Self {
            path,
            file: read.file.clone(),
            compile_base: read.file,
            composer,
            title,
            files,
            difficulties,
            hs_mode: false,
        })
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
        &|| false,
    )
}
fn save_snapshot(
    file: OsuFile,
    hits: Hitsounds,
    expected: Vec<u8>,
    path: &Path,
    backups: PathBuf,
    hs: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<SaveReport, String> {
    let (out, problems) = if hs {
        match dawsu_hitsounding::hs::compile_hs(
            &file,
            &hits,
            Placement::OnObjects,
            cancel,
            &mut |_, _| {},
        ) {
            dawsu_hitsounding::hs::CompiledHs::Done { file, problems, .. } => (file, problems),
            _ => return Err("Save cancelled".into()),
        }
    } else {
        match compile(&file, &hits, Placement::GreenLines, cancel, &mut |_, _| {}) {
            Compiled::Done { file, problems } => (file, problems),
            _ => return Err("Save cancelled".into()),
        }
    };
    // A partial compile is a review result, never a successful save of the whole document.
    if !problems.is_empty() {
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
    match out
        .save_over_expected(path, &expected, &BackupPolicy::new(backups, 20))
        .map_err(|e| e.to_string())?
    {
        SaveOutcome::Saved { .. } | SaveOutcome::NothingToSave => Ok(SaveReport {
            file: OsuFile::read(out.to_bytes()).file,
            problems,
        }),
        SaveOutcome::ChangedOnDisk => Err(
            "The difficulty changed on disk. Your edits are kept. Reopen or save a separate copy."
                .into(),
        ),
        SaveOutcome::AlreadyExists => Err("The target already exists".into()),
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub recent: Vec<PathBuf>,
    pub songs_folder: PathBuf,
    pub default_samples: PathBuf,
    pub device: Option<String>,
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
            default_samples: PathBuf::new(),
            device: None,
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
    Open(Result<Loaded, String>),
    Save {
        version: u64,
        path: PathBuf,
        result: Result<SaveReport, String>,
    },
    Review(Result<output::WritePlan, String>),
    Output(Result<usize, String>),
    Stretched {
        path: PathBuf,
        speed: f64,
        result: Result<Arc<AudioClip>, String>,
    },
}
pub struct Application {
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
    pub revert_requested: bool,
    pub busy: bool,
    pub output_review: Option<output::WritePlan>,
    tools: ToolBox<HitKey>,
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
    pub first_launch: bool,
    catalogue: Arc<CommandCatalog<Application>>,
    pub keymap: KeyMap,
    pub help_requested: bool,
    persist_preferences: bool,
}
impl Application {
    pub fn new() -> Self {
        let file = data_folder().join("preferences.json");
        let first_launch = !file.exists();
        let mut preferences: Preferences = std::fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // Local development fallback is read-only. These non-commercial assets are never bundled.
        let local = PathBuf::from(
            "E:/GitHub/dawsu-hitsounding-editor/src/OsuHitsound.Audio/DefaultSamples",
        );
        if preferences.default_samples.as_os_str().is_empty() && local.is_dir() {
            preferences.default_samples = local;
        }
        let output = AudioEngine::open(preferences.device.as_deref());
        let (audio, error) = match output {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e)),
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
        let keymap = KeyMap::from_preset(
            catalogue.definitions(),
            if preferences.key_preset == "osu!" {
                KeyPreset::Osu
            } else {
                KeyPreset::DawLike
            },
        );
        Self {
            document: None,
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
            revert_requested: false,
            busy: false,
            output_review: None,
            tools: ToolBox::new(),
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
        match AudioEngine::open(Some(&name)) {
            Ok(a) => {
                self.audio = Some(a);
                self.preferences.device = Some(name);
                self.sync_audio();
            }
            Err(e) => self.error = Some(e),
        }
    }
    pub fn save_session(&mut self) -> Result<(), String> {
        let Some(document) = &self.document else {
            return Ok(());
        };
        let settings = project::SessionSettings {
            song_offset: self.song_offset,
            locator_left: self.locator_left,
            locator_right: Some(self.locator_right),
            cycle: self.cycle,
            zoom: Some(self.zoom),
            row_height: Some(self.row_height),
            view_start: document.composer.view_start,
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
            self.open_now(path);
        }
    }

    pub fn discard_and_open(&mut self) {
        if let Some(path) = self.pending_open.take() {
            self.open_now(path);
        }
    }
    fn open_now(&mut self, path: PathBuf) {
        if let Err(error) = self.save_session() {
            self.error = Some(error);
        }
        self.stop();
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
            let _ = send.send(JobResult::Open(result));
        });
    }
    pub fn cancel_job(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.status = "Cancelling…".into();
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
                JobResult::Open(Ok(loaded)) => {
                    let path = loaded.document.path.clone();
                    self.document = Some(loaded.document);
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
                JobResult::Open(Err(e)) => {
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
                JobResult::Review(result) => match result {
                    Ok(plan) => {
                        self.output_review = Some(plan);
                        self.status = "Review the output before writing".into();
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.status = "Output planning stopped; no files written".into();
                    }
                },
                JobResult::Output(result) => match result {
                    Ok(count) => {
                        self.status = format!("Wrote {count} reviewed files with backups");
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
                    result,
                } => match result {
                    Ok(report) => {
                        if let Some(d) = self.document.as_mut() {
                            if d.path == path {
                                d.file = report.file;
                                if d.composer.history.version() == version {
                                    d.composer.mark_saved();
                                }
                            }
                        }
                        if let Err(error) = self.save_session() {
                            self.error = Some(error);
                            self.status =
                                "Difficulty saved. Session settings could not be saved.".into();
                            return;
                        }
                        self.status = "Saved with backup".into();
                        if let Some(path) = self.pending_open.take() {
                            self.open_now(path);
                        }
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.status = "Save did not complete; edits kept".into();
                    }
                },
            }
        }
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
    pub fn save(&mut self) {
        if self.busy {
            return;
        }
        if self.tools.is_stroke_open() {
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
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Compiling and saving…".into();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        std::thread::spawn(move || {
            let result = save_snapshot(
                file,
                hits,
                expected,
                &path,
                data_folder().join("backups"),
                hs,
                &|| cancel.load(Ordering::Relaxed),
            );
            let _ = send.send(JobResult::Save {
                version,
                path,
                result,
            });
        });
    }
    pub fn play_pause(&mut self) {
        if self.document.is_none() {
            return;
        }
        if self.is_playing() {
            self.audio.as_ref().map(|a| a.play(false));
        } else {
            self.playback_start = self.position_ms();
            if let Some(a) = &self.audio {
                a.play(true);
            }
        }
    }
    pub fn stop(&mut self) {
        if let Some(a) = &self.audio {
            a.play(false);
            if self.preferences.return_on_stop {
                a.seek(self.playback_start);
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
    pub fn audition(&self, lane: &SampleIdentity) {
        if let (Some(a), Some(clip)) = (&self.audio, self.samples.get(lane)) {
            a.audition(clip.clone(), 1.0);
        }
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
        let time = self.position_ms();
        if let Some(d) = self.document.as_ref() {
            let next = d
                .composer
                .tempo
                .grid_step(SongTime::from_ms(time), direction, d.composer.spacing)
                .ms();
            self.seek(next);
        }
    }
    pub fn set_tool(&mut self, tool: ToolKind) {
        if let Some(d) = self.document.as_mut() {
            self.tools.select_tool(tool, &mut d.composer);
        }
    }
    pub fn tool(&self) -> ToolKind {
        self.tools.active()
    }
    pub fn pointer(&mut self, phase: PointerPhase, input: PointerInput) {
        if self.busy {
            return;
        }
        if matches!(phase, PointerPhase::Down) {
            self.active_lane = input.row.max(0) as usize;
        }
        if let Some(d) = self.document.as_mut() {
            match phase {
                PointerPhase::Down => self.tools.down(&mut d.composer, input),
                PointerPhase::Move => self.tools.moved(&mut d.composer, input),
                PointerPhase::Up => self.tools.up(&mut d.composer, input),
                PointerPhase::Cancel => self.tools.cancel(&mut d.composer),
            }
        }
    }
    pub fn go_to_history(&mut self, position: usize) {
        if self.busy || self.tools.is_stroke_open() {
            return;
        }
        if let Some(document) = self.document.as_mut() {
            document.composer.history.go_to(position);
            document.composer.selected.clear();
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
        if let Some(d) = self.document.as_mut() {
            d.composer.volume(v, relative);
        }
    }
    pub fn set_hs_mode(&mut self, hs: bool) {
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
        prepare: impl FnOnce(output::Source, Arc<AtomicBool>) -> Result<output::WritePlan, String>
        + Send
        + 'static,
    ) {
        if self.busy {
            return;
        }
        if self.tools.is_stroke_open() {
            self.error = Some("Finish or cancel the current drag before preparing output.".into());
            return;
        }
        let Some(d) = self.document.as_ref() else {
            return;
        };
        let source = output::Source::from_document(d);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Preparing output review…".into();
        std::thread::spawn(move || {
            let _ = send.send(JobResult::Review(prepare(source, cancel)));
        });
    }
    pub fn generate(&mut self) {
        self.review_job(|source, cancel| {
            output::generate(&source, &|| cancel.load(Ordering::Relaxed))
        });
    }
    pub fn new_diff(&mut self, name: String) {
        self.review_job(move |source, _| output::new_diff(&source, &name));
    }
    pub fn copy_hitsounds(
        &mut self,
        targets: Vec<PathBuf>,
        options: dawsu_hitsounding::copy::CopyOptions,
    ) {
        self.review_job(move |source, cancel| {
            output::copy(&source, &targets, &options, &|| {
                cancel.load(Ordering::Relaxed)
            })
        });
    }
    pub fn apply_output(&mut self) {
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
            let _ = send.send(JobResult::Output(
                plan.apply(&data_folder().join("backups"), &|| {
                    cancel.load(Ordering::Relaxed)
                }),
            ));
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
        self.keymap = KeyMap::from_preset(
            self.catalogue.definitions(),
            if preset == "osu!" {
                KeyPreset::Osu
            } else {
                KeyPreset::DawLike
            },
        );
        self.first_launch = false;
        self.save_preferences();
    }
    fn sync_audio(&mut self) {
        if let Some(d) = &self.document {
            self.cached_version = d.composer.history.version();
            self.cached_listen_version = d.composer.listen_version;
            let mut cues = Vec::new();
            for cue in cue_list(d.composer.hitsounds(), &self.muted, &self.solo) {
                if d.composer.listen_muted.iter().any(|key| {
                    key.lane == *cue.sample()
                        && d.composer
                            .hitsounds()
                            .moments()
                            .get(key.moment)
                            .is_some_and(|m| m.time() == cue.time())
                }) {
                    continue;
                }
                if let Some(clip) = self.samples.get(cue.sample()) {
                    cues.push(ScheduledSound {
                        at_ms: cue.time().ms(),
                        end_ms: cue.loop_end().map(|t| t.ms()),
                        clip: clip.clone(),
                        gain: cue.gain() as f32,
                    });
                }
            }
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
fn load(
    path: &Path,
    defaults: &Path,
    cancel: &AtomicBool,
    project_root: Option<&Path>,
    legacy_root: Option<&Path>,
) -> Result<Loaded, String> {
    let document = Document::read(path)?;
    let folder = document.path.parent().ok_or("Missing folder")?;
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
        let path = match &lane.source {
            LaneSource::File(name) => safe_child(folder, name).ok(),
            LaneSource::Skin => {
                let name = lane_name(&lane.plays_as());
                ["wav", "ogg", "mp3"].iter().find_map(|ext| {
                    find_case_insensitive(&defaults_list, &format!("{name}.{ext}"))
                        .and_then(|n| safe_child(defaults, n).ok())
                })
            }
        };
        if let Some(path) = path {
            match dawsu_audio::decode_with_cancel(&path, &|| cancel.load(Ordering::Relaxed)) {
                Ok(clip) => {
                    samples.insert(lane.identity.clone(), clip);
                }
                Err(e) => warnings.push(e),
            }
        } else {
            warnings.push(format!("Sample unavailable: {}", lane_name(&lane.identity)));
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

fn problem_description(problem: &dawsu_hitsounding::ProblemKind) -> &'static str {
    use dawsu_hitsounding::ProblemKind::*;
    match problem {
        NoObject => "This hit has no object. Enable HS mode or Generate a hitsound difficulty.",
        NeedsSampleMix(_) => {
            "These sounds need a rendered sample mix; this development build cannot write that mix yet."
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
