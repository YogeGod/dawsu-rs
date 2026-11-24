//! The Export window's preview, kept apart from main's job, error, problem list and review.
use crate::{Application, OutputProblem, output};
use dawsu_hitsounding::copy::CopyOptions;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

/// What the Export window asks to write: "Into the mapset" (Copy), with or without the base
/// hitsound difficulty (Generate) in the same write.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExportOptions {
    pub targets: Vec<PathBuf>,
    pub copy: CopyOptions,
    /// Also write the base hitsound difficulty, as Generate does.
    pub base: bool,
    /// Generate's slider slide, whistle and tick audio, for the base difficulty.
    pub slider_audio: bool,
    /// Opt-in: auto-fix osu! RC health issues (late impact, <25 ms, <1024 B, bloated silence)
    /// on newly rendered/staged mixes. Default `false` preserves exact raw PCM output.
    pub auto_fix_staged_mixes: bool,
}

/// What the Export window last chose for a map, kept with the session. Older builds ignore it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportChoices {
    /// File names of the difficulties left out; every other one is written into.
    pub left_out: Vec<String>,
    /// Match within, in ms.
    pub match_within: f64,
    pub copy_volumes: bool,
    /// Also write the base hitsound difficulty. Choices saved before it existed read as off.
    #[serde(default)]
    pub base: bool,
    #[serde(default)]
    pub slider_audio: bool,
    /// Use "later moment wins" instead of the default Merge/Union for nearby source hits.
    #[serde(default)]
    pub later_wins: bool,
}

/// Which window a preview belongs to, so main never shows it as its own review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewOrigin {
    Export,
}

pub struct ExportPreview {
    pub id: u64,
    pub origin: PreviewOrigin,
    pub plan: output::WritePlan,
    /// The load or Save of the difficulty it was made from. The history version starts
    /// again at every load, so path and version alone cannot tell two loads apart.
    generation: u64,
    /// Hitsounding mode when it was made: switching it changes what the source writes.
    hs_mode: bool,
    /// The merge settings it was made with: changing them changes what the base difficulty writes.
    settings: Option<crate::ExportSettings>,
    /// Whether redundant greenline cleanup was enabled when previewed.
    clean_redundant_greenlines: bool,
}

type PreviewResult = (Result<output::WritePlan, String>, Vec<OutputProblem>);

struct Running {
    id: u64,
    generation: u64,
    hs_mode: bool,
    settings: Option<crate::ExportSettings>,
    clean_redundant_greenlines: bool,
    handle: JoinHandle<()>,
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<PreviewResult>,
    progress: Arc<Mutex<Option<(usize, usize)>>>,
}

#[derive(Default)]
pub(crate) struct Slot {
    last_id: u64,
    /// The id, the difficulty it was asked for, and the options.
    waiting: Option<(u64, PathBuf, ExportOptions)>,
    running: Option<Running>,
    /// Cancelled previews still finishing. The next one starts after they end.
    ending: Vec<JoinHandle<()>>,
    preview: Option<ExportPreview>,
    problems: Vec<OutputProblem>,
    error: Option<String>,
    /// An Export write is running on main's job.
    pub(crate) writing: bool,
    /// The id of the last Export write started; `written` and `write_error` belong to it.
    write_id: Option<u64>,
    /// The last Export write's result: files written, or why nothing was.
    written: Option<usize>,
    write_error: Option<String>,
}

impl Slot {
    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.store(true, Ordering::Relaxed);
            self.ending.push(running.handle);
        }
        self.waiting = None;
    }

    /// Stops the preview and forgets the finished one, its problems and its error.
    pub(crate) fn clear(&mut self) {
        self.stop();
        self.preview = None;
        self.problems.clear();
        self.error = None;
        self.writing = false;
        self.written = None;
        self.write_error = None;
    }

    /// Takes a finished write's result for the Export window. False when the window closed
    /// during the write, so main reports it instead.
    pub(crate) fn take_write(&mut self, result: &Result<usize, String>) -> bool {
        if !std::mem::take(&mut self.writing) {
            return false;
        }
        match result {
            Ok(count) => self.written = Some(*count),
            Err(error) => self.write_error = Some(error.clone()),
        }
        true
    }
}

/// Why a finished preview no longer matches the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stale {
    /// Another difficulty, an older edit, a reload or another hitsounding mode.
    Map,
    /// The merge settings the base difficulty was previewed with changed.
    MergeSettings,
}

pub struct ExportStatus<'a> {
    /// The id of the latest preview request, finished or not.
    pub requested: Option<u64>,
    pub preview: Option<&'a ExportPreview>,
    /// The preview was made from another difficulty or an older edit.
    pub stale: bool,
    pub stale_reason: Option<Stale>,
    /// Options wait for main's job or a drag to end.
    pub waiting: bool,
    pub running: bool,
    pub progress: Option<(usize, usize)>,
    pub problems: &'a [OutputProblem],
    pub error: Option<&'a str>,
    pub writing: bool,
    pub write_id: Option<u64>,
    pub written: Option<usize>,
    pub write_error: Option<&'a str>,
}

impl Application {
    /// Queues a preview and answers at once with its id. A newer request drops the
    /// finished preview and any running one, so Export is never run on old options.
    pub fn request_export_preview(&mut self, options: ExportOptions) -> Result<u64, String> {
        let source = self.document.as_ref().ok_or("Open a difficulty first.")?.path.clone();
        let targets = self.export_targets();
        // Never the source or the difficulty Generate made, whatever the window sends.
        if options.targets.iter().any(|path| !targets.contains(path)) {
            return Err("Export writes only into the other difficulties of this mapset.".into());
        }
        let left_out = targets.iter()
            .filter(|path| !options.targets.contains(path))
            .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .collect();
        if let Some(document) = self.document.as_mut() {
            document.export_choices = Some(ExportChoices {
                left_out,
                match_within: options.copy.leniency,
                copy_volumes: options.copy.copy_volumes,
                base: options.base,
                slider_audio: options.slider_audio,
                later_wins: options.copy.collision_policy == dawsu_hitsounding::copy::CopyCollisionPolicy::LaterWins,
            });
        }
        self.export.stop();
        self.export.preview = None;
        self.export.problems.clear();
        self.export.error = None;
        self.export.last_id += 1;
        let id = self.export.last_id;
        // Nothing chosen to write: the choice is kept, and there is nothing to preview.
        if !options.targets.is_empty() || options.base {
            self.export.waiting = Some((id, source, options));
            self.start_export_preview();
        }
        Ok(id)
    }

    pub fn cancel_export_preview(&mut self) {
        self.export.clear();
    }

    fn preview_is_current(&self, preview: &ExportPreview) -> bool {
        self.staleness(preview).is_none()
    }

    fn staleness(&self, preview: &ExportPreview) -> Option<Stale> {
        let document = self.document.as_ref().filter(|d| {
            preview.origin == PreviewOrigin::Export
                && preview.generation == self.document_generation
                && d.path == preview.plan.source && d.composer.history.version() == preview.plan.version
                && d.hs_mode == preview.hs_mode
                && self.preferences.clean_redundant_greenlines == preview.clean_redundant_greenlines
        });
        match document {
            None => Some(Stale::Map),
            // Merge settings change only what the base difficulty writes; Copy does not use them yet.
            Some(d) if preview.plan.generated.is_some() && d.export_settings != preview.settings => {
                Some(Stale::MergeSettings)
            }
            Some(_) => None,
        }
    }

    pub fn export_status(&self) -> ExportStatus<'_> {
        let slot = &self.export;
        ExportStatus {
            requested: (slot.last_id > 0).then_some(slot.last_id),
            preview: slot.preview.as_ref(),
            stale: slot.preview.as_ref().is_some_and(|preview| !self.preview_is_current(preview)),
            stale_reason: slot.preview.as_ref().and_then(|preview| self.staleness(preview)),
            waiting: slot.waiting.is_some(),
            running: slot.running.is_some(),
            progress: slot.running.as_ref()
                .and_then(|running| running.progress.lock().ok().and_then(|current| *current)),
            problems: &slot.problems,
            error: slot.error.as_deref(),
            writing: slot.writing,
            write_id: slot.write_id,
            written: slot.written,
            write_error: slot.write_error.as_deref(),
        }
    }

    /// The difficulties Export can write into: the mapset's others, without the source
    /// and without the difficulty Generate made from this source.
    pub fn export_targets(&self) -> Vec<PathBuf> {
        let Some(d) = self.document.as_ref() else {
            return Vec::new();
        };
        let generated = self.project_root.as_deref()
            .and_then(|projects| crate::generate_receipt::output_path(projects, &d.path));
        d.difficulties.iter()
            .filter(|path| **path != d.path && Some(path.as_path()) != generated.as_deref())
            .cloned()
            .collect()
    }

    pub fn backups_folder(&self) -> PathBuf {
        self.data_root.join("backups")
    }

    /// Writes the preview with this id. Refused unless it is the latest preview and
    /// the difficulty is still the one, at the same edit, that it was made from.
    pub fn run_export(&mut self, preview_id: u64) -> Result<u64, String> {
        self.run_export_in(preview_id, self.backups_folder())
    }

    /// Answers with the write's id, which the window matches against `write_id`.
    pub(crate) fn run_export_in(&mut self, preview_id: u64, backups: PathBuf) -> Result<u64, String> {
        if self.busy {
            return Err("Wait for the current job to finish.".into());
        }
        if self.is_stroke_open() {
            return Err("Finish or cancel the current drag before exporting.".into());
        }
        let matches = self.export.preview.as_ref()
            .is_some_and(|preview| preview.id == preview_id && self.preview_is_current(preview));
        if !matches {
            return Err("The map or the options changed since this preview. Preview again.".into());
        }
        let preview = self.export.preview.take().expect("checked above");
        self.write_reviewed(preview.plan, backups);
        self.export.writing = true;
        self.export.written = None;
        self.export.write_error = None;
        let id = self.export.write_id.map_or(1, |last| last + 1);
        self.export.write_id = Some(id);
        Ok(id)
    }

    /// Collects a finished preview and starts waiting options once main is free.
    pub(crate) fn poll_export(&mut self) {
        if let Some(running) = &self.export.running {
            match running.receive.try_recv() {
                Ok((result, problems)) => {
                    let (id, generation, hs_mode, clean_redundant_greenlines) = (
                        running.id, running.generation, running.hs_mode, running.clean_redundant_greenlines,
                    );
                    let settings = running.settings.clone();
                    self.export.running = None;
                    self.export.problems = problems;
                    match result {
                        Ok(plan) => {
                            self.export.preview = Some(ExportPreview {
                                id, origin: PreviewOrigin::Export, plan, generation, hs_mode, settings,
                                clean_redundant_greenlines,
                            });
                        }
                        Err(error) => self.export.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.export.running = None;
                    self.export.error = Some("The preview stopped unexpectedly. Preview again.".into());
                }
            }
        }
        self.start_export_preview();
    }

    fn start_export_preview(&mut self) {
        self.export.ending.retain(|handle| !handle.is_finished());
        if self.export.running.is_some() || !self.export.ending.is_empty() || self.busy || self.is_stroke_open() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some((id, asked_for, options)) = self.export.waiting.take() else {
            return;
        };
        if asked_for != document.path {
            self.export.error = Some("Another difficulty was opened. Preview again.".into());
            return;
        }
        let source = output::Source::from_document(document);
        let defaults = self.preferences.default_samples.clone();
        let clean_redundant_greenlines = self.preferences.clean_redundant_greenlines;
        // As Generate in main: the map's merge settings, and the receipts in the project folder.
        let settings = document.export_settings.clone();
        let projects = self.project_root.clone();
        let thread_settings = settings.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(None));
        let (send, receive) = mpsc::channel();
        let generation = self.document_generation;
        let hs_mode = document.hs_mode;
        let (thread_cancel, thread_progress) = (cancel.clone(), progress.clone());
        let handle = std::thread::spawn(move || {
            let (cancel, progress) = (thread_cancel, thread_progress);
            let mut problems = Vec::new();
            let result = output::export_report(
                &source,
                &defaults,
                thread_settings.as_ref(),
                projects.as_deref(),
                &options,
                &|| cancel.load(Ordering::Relaxed),
                &mut problems,
                &mut |done, total| {
                    if let Ok(mut current) = progress.lock() {
                        *current = Some((done, total));
                    }
                },
            ).map(|mut plan| {
                if clean_redundant_greenlines {
                    plan.clean_redundant_greenlines();
                }
                plan
            });
            let _ = send.send((result, problems));
        });
        self.export.running = Some(Running {
            id, generation, hs_mode, settings, clean_redundant_greenlines, handle, cancel, receive, progress,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::SampleIdentity;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    const MAP: &str = "osu file format v14\n\n[General]\nMode:0\n\n[Metadata]\nVersion:DIFF\n\n[Events]\nEVENTS\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";

    struct Mapset {
        scratch: Scratch,
        source: PathBuf,
        target: PathBuf,
        target_bytes: String,
    }

    fn mapset(name: &str) -> Mapset {
        let scratch = Scratch::new(name);
        let source = scratch.path("Source.osu");
        let target = scratch.path("Target.osu");
        std::fs::write(&source, MAP.replace("DIFF", "Source")
            .replace("EVENTS", "Sample,1005,0,\"near.wav\",70\nSample,1200,0,\"far.wav\",60")).unwrap();
        let target_bytes = MAP.replace("DIFF", "Target").replace("EVENTS", "Sample,1400,0,\"target.wav\",80");
        std::fs::write(&target, &target_bytes).unwrap();
        let source = source.canonicalize().unwrap();
        let target = target.canonicalize().unwrap();
        Mapset { scratch, source, target, target_bytes }
    }

    fn opened(source: &std::path::Path) -> Application {
        let mut app = Application::headless();
        // Backups and receipts stay in the fixture, never in the real data folder.
        let data = source.parent().unwrap().join("data");
        app.data_root = data.clone();
        app.project_root = Some(data.join("projects"));
        app.request_open(source.to_path_buf());
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        app
    }

    fn wait_main(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline, "main job did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_preview(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            let status = app.export_status();
            if !status.running && !status.waiting {
                return;
            }
            assert!(Instant::now() < deadline, "preview did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn save(app: &mut Application) {
        app.save();
        wait_main(app);
        if app.save_review_pending() {
            app.answer_save_review(true);
            wait_main(app);
        }
        assert!(app.error.is_none() && !app.is_dirty(), "{:?}", app.error);
    }

    fn reopen(app: &mut Application, path: &std::path::Path) {
        app.request_open(path.to_path_buf());
        wait_main(app);
        assert_eq!(app.document().unwrap().path, path);
    }

    fn edit(app: &mut Application) {
        app.document.as_mut().unwrap().composer.add(1500.0, SampleIdentity::File("far.wav".into()), 60, 1.0);
    }

    /// Edits until the history version is back at `version`, as after the same number of steps.
    fn edit_to(app: &mut Application, version: u64) {
        let mut time = 1700.0;
        loop {
            let composer = &mut app.document.as_mut().unwrap().composer;
            let now = composer.history.version();
            if now >= version {
                break;
            }
            // An added hit takes two steps, Undo one.
            if version - now == 1 && composer.history.can_undo() {
                composer.history.undo();
            } else {
                composer.add(time, SampleIdentity::File("far.wav".into()), 60, 1.0);
                time += 100.0;
            }
        }
        assert_eq!(app.document().unwrap().composer.history.version(), version);
    }

    fn options(targets: &[PathBuf]) -> ExportOptions {
        ExportOptions {
            targets: targets.to_vec(),
            copy: CopyOptions { leniency: 5.0, copy_storyboarded_samples: true, ..CopyOptions::default() },
            ..ExportOptions::default()
        }
    }

    fn plan_parts(plan: &output::WritePlan) -> impl PartialEq + std::fmt::Debug {
        (
            plan.label.clone(), plan.source.clone(), plan.version, plan.details.clone(),
            plan.mix_paths().map(PathBuf::from).collect::<Vec<_>>(),
            (0..plan.mixes.len()).map(|index| plan.mix_clip(index).unwrap().samples).collect::<Vec<_>>(),
            plan.files.iter().map(|file| (file.path.clone(), file.file.to_bytes(), file.expected.clone()))
                .collect::<Vec<_>>(),
        )
    }

    fn problem_parts(problems: &[OutputProblem]) -> Vec<(PathBuf, PathBuf, String, String)> {
        problems.iter().map(|problem| {
            (problem.target.clone(), problem.timeline.clone(), problem.time.to_string(), problem.message.clone())
        }).collect()
    }

    /// The preview plans the same writes, sample mixes and problems as the editor's Copy.
    fn assert_parity(source: &std::path::Path, options: ExportOptions) -> Application {
        let mut app = opened(source);
        let defaults = source.parent().unwrap().join("defaults");
        std::fs::create_dir_all(&defaults).unwrap();
        app.preferences.default_samples = defaults;
        let id = app.request_export_preview(options.clone()).unwrap();
        wait_preview(&mut app);
        app.copy_hitsounds(options.targets.clone(), options.copy.clone());
        wait_main(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let preview = status.preview.expect("a finished preview");
        assert_eq!((preview.id, preview.origin), (id, PreviewOrigin::Export));
        assert!(!status.stale);
        let copy = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        assert_eq!(plan_parts(&preview.plan), plan_parts(copy));
        assert_eq!(problem_parts(status.problems), problem_parts(&app.output_problems));
        assert!(!preview.plan.details.is_empty());
        assert_ne!(Some(preview.plan.files[0].file.to_bytes()), preview.plan.files[0].expected, "the preview changes the target");
        app
    }

    #[test]
    fn export_preview_plans_the_same_writes_as_copy() {
        let set = mapset("export-parity");
        let app = assert_parity(&set.source, options(&[set.target.clone()]));
        assert_eq!(app.export_status().preview.unwrap().plan.mixes.len(), 0);
    }

    #[test]
    fn export_preview_stages_the_same_sample_mixes_as_copy() {
        // Two samples at one moment need a mix in the target (tests/copy_mixes.rs).
        let scratch = Scratch::new("export-parity-mix");
        let header = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Test\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n";
        let source = scratch.path("source.osu");
        let target = scratch.path("target.osu");
        std::fs::write(&source, format!("{header}256,192,1000,1,0,0:0:0:40:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n")).unwrap();
        std::fs::write(&target, format!("{header}100,100,1000,1,0,0:0:0:0:\n")).unwrap();
        for (name, value) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            for _ in 0..2 {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let options = ExportOptions { targets: vec![target.canonicalize().unwrap()], ..ExportOptions::default() };
        let mut app = assert_parity(&source.canonicalize().unwrap(), options);
        let preview = app.export_status().preview.unwrap();
        assert!(!preview.plan.mixes.is_empty());
        let (id, waves) = (preview.id, preview.plan.mix_paths().map(PathBuf::from).collect::<Vec<_>>());
        assert!(waves.iter().all(|wave| !wave.exists()));
        app.output_review = None;
        app.run_export(id).unwrap();
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(waves.iter().all(|wave| wave.exists()), "the export publishes its mixes");
    }

    #[test]
    fn export_preview_never_uses_main_job_error_or_review() {
        let set = mapset("export-apart");
        let mut app = opened(&set.source);
        app.status = "untouched".into();
        app.request_export_preview(options(&[set.target.clone()])).unwrap();
        assert!(!app.busy);
        assert!(app.jobs.is_none());
        wait_preview(&mut app);
        assert!(app.export_status().preview.is_some());
        assert!(app.output_review.is_none());
        assert_eq!(app.status, "untouched");

        app.request_export_preview(options(&[])).unwrap();
        wait_preview(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None, "every difficulty left out is a choice, not an error");
        assert!(status.preview.is_none());
        assert!(app.error.is_none());
        assert!(app.output_problems.is_empty());
    }

    #[test]
    fn export_preview_waits_for_main_job_and_close_cancels_it() {
        let set = mapset("export-wait");
        let mut app = opened(&set.source);
        app.busy = true;
        app.request_export_preview(options(&[set.target.clone()])).unwrap();
        app.poll_export();
        let status = app.export_status();
        assert!(status.waiting && !status.running);
        assert_eq!(status.error, None);
        app.busy = false;
        app.poll_export();
        assert!(app.export_status().running || app.export_status().preview.is_some());
        app.cancel_export_preview();
        let status = app.export_status();
        assert!(!status.running && !status.waiting);
        app.poll_export();
        assert!(app.export_status().preview.is_none());

        app.request_export_preview(options(&[set.target.clone()])).unwrap();
        app.cancel_job();
        wait_preview(&mut app);
        assert!(app.export_status().preview.is_some());
        app.cancel_export_preview();
        let status = app.export_status();
        assert!(status.preview.is_none() && status.problems.is_empty());
    }

    #[test]
    fn export_runs_only_the_latest_preview_of_the_current_edit() {
        let set = mapset("export-run");
        let backups = set.scratch.path("backups");
        let mut app = opened(&set.source);
        let first = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        assert!(app.run_export_in(first + 1, backups.clone()).is_err());

        let second = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        assert!(second > first);
        assert!(app.run_export_in(first, backups.clone()).is_err());
        wait_preview(&mut app);
        assert!(app.run_export_in(first, backups.clone()).is_err());

        app.document.as_mut().unwrap().composer.add(1500.0, SampleIdentity::File("far.wav".into()), 60, 1.0);
        assert!(app.export_status().stale);
        assert!(app.run_export_in(second, backups.clone()).is_err());
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);

        let third = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        app.run_export_in(third, backups.clone()).unwrap();
        assert!(app.output_review.is_none());
        assert!(app.export_status().preview.is_none());
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        let written = std::fs::read_to_string(&set.target).unwrap();
        assert!(written.contains("Sample,1200,0,\"far.wav\",60"), "{written}");
        assert!(app.run_export_in(third, backups).is_err());
    }

    #[test]
    fn waiting_options_are_dropped_when_another_difficulty_opens() {
        let set = mapset("export-switch");
        let mut app = opened(&set.source);
        app.busy = true;
        app.request_export_preview(options(&[set.target.clone()])).unwrap();
        app.busy = false;
        app.request_open(set.target.clone());
        wait_main(&mut app);
        wait_preview(&mut app);
        let status = app.export_status();
        assert!(status.preview.is_none() && !status.waiting && !status.running);
        assert_eq!(status.error, None);
        assert!(app.error.is_none());
    }

    #[test]
    fn export_refuses_a_preview_of_another_difficulty() {
        let set = mapset("export-other-map");
        let mut app = opened(&set.source);
        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        let version = app.export_status().preview.unwrap().plan.version;
        app.request_open(set.target.clone());
        wait_main(&mut app);
        assert_eq!(app.document().unwrap().path, set.target);
        assert_eq!(app.document().unwrap().composer.history.version(), version);
        assert!(app.export_status().preview.is_none());
        assert!(app.run_export_in(id, set.scratch.path("backups")).is_err());
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);
    }

    #[test]
    fn reopening_or_reverting_the_same_difficulty_refuses_an_older_preview() {
        let set = mapset("export-reload");
        let backups = set.scratch.path("backups");
        let mut app = opened(&set.source);
        edit_to(&mut app, 4);
        let version = app.document().unwrap().composer.history.version();
        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        app.copy_hitsounds(vec![set.target.clone()], options(&[]).copy);
        wait_main(&mut app);
        assert!(app.output_review.is_some());
        save(&mut app);
        reopen(&mut app, &set.target);
        reopen(&mut app, &set.source);
        edit_to(&mut app, version);
        assert!(app.run_export_in(id, backups.clone()).is_err());
        assert!(app.output_review.is_none(), "main's old review does not survive a reload");
        app.apply_output_in(backups.clone());
        wait_main(&mut app);
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);

        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        let version = app.document().unwrap().composer.history.version();
        app.revert_requested = true;
        app.confirm_revert();
        wait_main(&mut app);
        edit_to(&mut app, version);
        assert!(app.run_export_in(id, backups).is_err());
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);
    }

    #[test]
    fn saving_the_difficulty_refuses_a_preview_made_before_the_save() {
        let set = mapset("export-save");
        let mut app = opened(&set.source);
        edit(&mut app);
        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        save(&mut app);
        assert!(app.run_export_in(id, set.scratch.path("backups")).is_err());
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);
    }

    #[test]
    fn export_preview_counts_each_target_as_copy_planned_it() {
        let set = mapset("export-counts");
        let app = assert_parity(&set.source, options(&[set.target.clone()]));
        let preview = app.export_status().preview.unwrap();
        assert_eq!(preview.plan.targets.len(), 1);
        let counts = &preview.plan.targets[0];
        assert_eq!(counts.path, set.target);
        // The same numbers Copy's review line shows.
        let line = format!("Target.osu: {} matched, {} unmatched source moments", counts.matched, counts.unmatched);
        assert_eq!(preview.plan.details[0], line);
        assert_eq!((counts.collisions, counts.kept_from_silent, counts.muted), (0, 0, 0));
    }

    #[test]
    fn a_failed_export_write_reports_to_export_not_main() {
        let set = mapset("export-write-error");
        let mut app = opened(&set.source);
        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        // Another program changes the target after the preview: the write refuses.
        std::fs::write(&set.target, set.target_bytes.replace("Target", "Changed")).unwrap();
        app.run_export_in(id, set.scratch.path("backups")).unwrap();
        assert!(app.export_status().writing);
        wait_main(&mut app);
        let status = app.export_status();
        assert!(!status.writing);
        assert!(status.write_error.is_some_and(|error| error.contains("changed since this review")), "{:?}", status.write_error);
        assert_eq!(status.written, None);
        assert!(app.error.is_none(), "{:?}", app.error);

        // A new preview keeps the write result until the next write.
        app.request_export_preview(options(&[set.target.clone()])).unwrap();
        assert!(app.export_status().write_error.is_some());
        wait_preview(&mut app);
        let id = app.export_status().preview.unwrap().id;
        app.run_export_in(id, set.scratch.path("backups")).unwrap();
        assert_eq!(app.export_status().write_error, None);
        wait_main(&mut app);
        assert_eq!(app.export_status().written, Some(1));
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn a_write_still_running_when_export_closes_reports_to_main() {
        let set = mapset("export-write-closed");
        let mut app = opened(&set.source);
        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        std::fs::write(&set.target, "changed").unwrap();
        app.run_export_in(id, set.scratch.path("backups")).unwrap();
        app.cancel_export_preview();
        wait_main(&mut app);
        assert!(app.export_status().write_error.is_none());
        assert!(app.error.as_deref().is_some_and(|error| error.contains("changed since this review")), "{:?}", app.error);
    }

    #[test]
    fn export_choices_are_kept_per_map_with_the_session() {
        let set = mapset("export-choices");
        let mut app = opened(&set.source);
        assert_eq!(app.document().unwrap().export_choices(), None);
        let mut chosen = options(&[]);
        chosen.copy.leniency = 3.0;
        chosen.copy.copy_volumes = false;
        app.request_export_preview(chosen).unwrap();
        let expected = ExportChoices {
            left_out: vec!["Target.osu".into()],
            match_within: 3.0,
            copy_volumes: false,
            base: false,
            slider_audio: false,
            later_wins: false,
        };
        assert_eq!(app.document().unwrap().export_choices(), Some(&expected));
        wait_preview(&mut app);
        app.save_session().unwrap();

        let mut app = opened(&set.source);
        assert_eq!(app.document().unwrap().export_choices(), Some(&expected));
        app.request_export_preview(options(&[set.target.clone()])).unwrap();
        assert_eq!(app.document().unwrap().export_choices().unwrap().left_out, Vec::<String>::new());
    }

    #[test]
    fn a_session_without_export_choices_still_loads() {
        let old: crate::project::SessionSettings =
            serde_json::from_str(r#"{"song_offset":0.0,"view_start":0.0}"#).unwrap();
        assert_eq!(old.export_choices, None);
        let json = serde_json::to_value(&old).unwrap();
        assert!(json.get("export_choices").is_none(), "an unset field is not written");
    }

    #[test]
    fn export_refuses_a_target_outside_the_mapset_list() {
        let set = mapset("export-foreign-target");
        let mut app = opened(&set.source);
        let elsewhere = set.scratch.path("elsewhere.osu");
        std::fs::write(&elsewhere, &set.target_bytes).unwrap();
        for target in [set.source.clone(), elsewhere.canonicalize().unwrap()] {
            let error = app.request_export_preview(options(&[target])).unwrap_err();
            assert_eq!(error, "Export writes only into the other difficulties of this mapset.");
        }
        assert_eq!(app.document().unwrap().export_choices(), None, "a refused request stores nothing");
        assert!(!app.export_status().running && !app.export_status().waiting);
    }

    #[test]
    fn every_difficulty_left_out_stores_the_choice_without_a_preview() {
        let set = mapset("export-all-left-out");
        let mut app = opened(&set.source);
        app.request_export_preview(options(&[])).unwrap();
        let status = app.export_status();
        assert!(!status.running && !status.waiting && status.preview.is_none() && status.error.is_none());
        assert_eq!(app.document().unwrap().export_choices().unwrap().left_out, ["Target.osu"]);
    }

    #[test]
    fn switching_hitsounding_mode_makes_the_preview_stale() {
        let set = mapset("export-hs-mode");
        let mut app = opened(&set.source);
        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        assert!(!app.export_status().stale);
        app.document.as_mut().unwrap().hs_mode ^= true;
        assert!(app.export_status().stale);
        assert!(app.run_export_in(id, set.scratch.path("backups")).is_err());
    }

    #[test]
    fn each_export_write_has_its_own_id() {
        let set = mapset("export-write-id");
        let mut app = opened(&set.source);
        assert_eq!(app.export_status().write_id, None);
        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        let first = app.run_export_in(id, set.scratch.path("backups")).unwrap();
        assert_eq!(app.export_status().write_id, Some(first));
        wait_main(&mut app);
        let id = app.request_export_preview(options(&[set.target.clone()])).unwrap();
        wait_preview(&mut app);
        let second = app.run_export_in(id, set.scratch.path("backups")).unwrap();
        assert!(second > first);
        assert_eq!(app.export_status().write_id, Some(second));
    }

    #[test]
    fn export_targets_leave_out_the_source_and_the_generate_output() {
        let set = mapset("export-targets");
        // Generate names its output from the artist and title.
        let source = std::fs::read_to_string(&set.source).unwrap()
            .replace("Version:Source", "Artist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Source");
        std::fs::write(&set.source, source).unwrap();
        let mut app = opened(&set.source);
        app.generate();
        wait_main(&mut app);
        let generated = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error)).files[0].path.clone();
        app.apply_output_in(set.scratch.path("backups"));
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.document().unwrap().difficulties.iter().any(|path| *path == generated));
        assert_eq!(app.export_targets(), [set.target.clone()]);
    }

    // Slice 2: the base hitsound difficulty (Generate) in the same write as Copy.

    const MIXED: &str = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:VERSION\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n";

    /// A source with two samples at one moment, so Generate and Copy both need a sample mix,
    /// and two other difficulties.
    fn mixed_mapset(name: &str) -> (Scratch, PathBuf, PathBuf, PathBuf) {
        let scratch = Scratch::new(name);
        let source = scratch.path("source.osu");
        std::fs::write(&source, format!("{}256,192,1000,1,0,0:0:0:40:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n", MIXED.replace("VERSION", "Source"))).unwrap();
        for target in ["target", "other"] {
            std::fs::write(scratch.path(&format!("{target}.osu")), format!("{}100,100,1000,1,0,0:0:0:0:\n", MIXED.replace("VERSION", target))).unwrap();
        }
        for (name, value) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            for _ in 0..2 {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let canonical = |name: &str| scratch.path(name).canonicalize().unwrap();
        let (source, target, other) = (canonical("source.osu"), canonical("target.osu"), canonical("other.osu"));
        (scratch, source, target, other)
    }

    /// Generate written, then Copy written, as the editor's two old buttons do, without the
    /// editor's polling in between (it would add the new WAVs as samples to the source).
    fn generate_then_copy(source: &std::path::Path, target: PathBuf, copy: &CopyOptions, slider_audio: bool) {
        let app = opened(source);
        let snapshot = output::Source::from_document(app.document().unwrap());
        let defaults = app.preferences.default_samples.clone();
        let backups = app.data_root.join("backups");
        let generate = output::generate_report(
            &snapshot, &defaults, None, app.project_root.as_deref(), slider_audio, &|| false,
            &mut Vec::new(), &mut |_, _| {},
        ).unwrap();
        generate.apply(&backups, &|| false).unwrap();
        crate::generate_receipt::record(&generate).unwrap();
        let copy = output::copy_report(&snapshot, &defaults, &[target], copy, &|| false, &mut Vec::new(), &mut |_, _| {})
            .unwrap();
        copy.apply(&backups, &|| false).unwrap();
    }

    fn with_base(targets: &[PathBuf], slider_audio: bool) -> ExportOptions {
        ExportOptions { base: true, slider_audio, ..options(targets) }
    }

    /// Every file in the mapset folder by name, without the editor's data folder.
    fn folder_files(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(root).unwrap().map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| (path.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&path).unwrap()))
            .collect()
    }

    fn wavs(root: &std::path::Path) -> Vec<String> {
        folder_files(root).into_keys().filter(|name| name.starts_with("dawsu-mix-")).collect()
    }

    fn run_preview(app: &mut Application, options: ExportOptions) {
        // The editor adds new WAVs in the mapset folder as samples about once a second, which
        // makes a preview stale as it would in the window. Preview again until one is current.
        for _ in 0..20 {
            app.request_export_preview(options.clone()).unwrap();
            wait_preview(app);
            if !app.export_status().stale {
                break;
            }
        }
        let status = app.export_status();
        assert_eq!(status.error, None);
        let id = status.preview.expect("a finished preview").id;
        let backups = app.data_root.join("backups");
        app.run_export_in(id, backups).unwrap();
        wait_main(app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.export_status().write_error, None);
    }

    #[test]
    fn base_only_export_plans_the_same_writes_as_generate() {
        let (_scratch, source, _, _) = mixed_mapset("export-base-parity");
        let mut app = opened(&source);
        let id = app.request_export_preview(with_base(&[], true)).unwrap();
        wait_preview(&mut app);
        app.generate_with_slider_audio(true);
        wait_main(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let preview = status.preview.expect("a base-only preview");
        assert_eq!(preview.id, id);
        let generate = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        assert_eq!(plan_parts(&preview.plan), plan_parts(generate));
        assert_eq!(problem_parts(status.problems), problem_parts(&app.output_problems));
        assert!(preview.plan.includes_slider_audio());
        assert!(!preview.plan.mixes.is_empty(), "the fixture needs a sample mix");
        assert_eq!(preview.plan.generated_file(), Some(preview.plan.files[0].path.as_path()));
    }

    #[test]
    fn base_and_targets_write_as_generate_then_copy_and_leave_out_the_rest() {
        let (_export_set, source, target, other) = mixed_mapset("export-base-copy");
        let other_bytes = std::fs::read(&other).unwrap();
        let mut app = opened(&source);
        run_preview(&mut app, with_base(&[target.clone()], false));
        let exported = folder_files(source.parent().unwrap());

        let (_set, reference, reference_target, _) = mixed_mapset("export-base-copy-reference");
        generate_then_copy(&reference, reference_target, &options(&[]).copy, false);

        assert_eq!(exported, folder_files(reference.parent().unwrap()));
        assert_eq!(exported["other.osu"], other_bytes, "a left-out difficulty is not touched");
        assert_eq!(wavs(source.parent().unwrap()).len(), 2, "Generate and Copy each wrote their mix");
    }

    #[test]
    fn the_receipt_lists_only_the_base_and_its_mixes_and_re_export_adds_no_wav() {
        let (_scratch, source, target, _) = mixed_mapset("export-base-receipt");
        let mut app = opened(&source);
        app.request_export_preview(with_base(&[target.clone()], false)).unwrap();
        wait_preview(&mut app);
        let plan = &app.export_status().preview.unwrap().plan;
        let generated = plan.generated_file().unwrap().to_path_buf();
        let (base_mixes, copy_mixes) = plan.mixes.split_at(plan.generated.as_ref().unwrap().1);
        let names = |mixes: &[crate::mixes::StagedMix]| mixes.iter()
            .map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        let (base_names, copy_names) = (names(base_mixes), names(copy_mixes));
        assert!(!base_names.is_empty() && !copy_names.is_empty());
        run_preview(&mut app, with_base(&[target.clone()], false));

        let projects = app.project_root.clone().unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(
            &std::fs::read(crate::generate_receipt::path(&projects, &source)).unwrap()).unwrap();
        assert_eq!(receipt["file_name"], generated.file_name().unwrap().to_string_lossy().as_ref());
        assert_eq!(receipt["mixes"], serde_json::json!(base_names), "Copy's mixes are not Generate's");
        let targets = app.export_targets();
        assert_eq!(targets.len(), 2, "the base difficulty is never a target");
        assert!(targets.iter().all(|path| path.file_name() != generated.file_name()));

        let before = wavs(source.parent().unwrap());
        app.request_export_preview(with_base(&[target.clone()], false)).unwrap();
        wait_preview(&mut app);
        let plan = &app.export_status().preview.unwrap().plan;
        assert!(plan.mixes.iter().all(|mix| mix.is_reused()), "every mix is already on disk");
        run_preview(&mut app, with_base(&[target], false));
        assert_eq!(wavs(source.parent().unwrap()), before);
    }

    #[test]
    fn a_failed_export_write_rolls_back_the_base_difficulty_too() {
        let (scratch, source, target, _) = mixed_mapset("export-base-rollback");
        let target_bytes = std::fs::read(&target).unwrap();
        let app = opened(&source);
        let plan = output::export_report(
            &output::Source::from_document(app.document().unwrap()), std::path::Path::new(""), None,
            app.project_root.as_deref(), &with_base(&[target.clone()], false), &|| false,
            &mut Vec::new(), &mut |_, _| {},
        ).unwrap();
        let generated = plan.generated_file().unwrap().to_path_buf();
        let waves = plan.mix_paths().map(PathBuf::from).collect::<Vec<_>>();
        assert_eq!(waves.len(), 2);
        // The write stops once the base difficulty is on disk, before Copy's target.
        let error = plan.apply(&scratch.path("backups"), &|| generated.exists()).unwrap_err();
        assert!(error.contains("Previous targets were restored"), "{error}");
        assert!(!generated.exists());
        assert!(waves.iter().all(|wave| !wave.exists()));
        assert_eq!(std::fs::read(&target).unwrap(), target_bytes);
        assert!(!crate::generate_receipt::path(app.project_root.as_deref().unwrap(), &source).exists());
    }

    #[test]
    fn export_refuses_the_base_difficulty_as_a_copy_target() {
        let (_scratch, source, target, _) = mixed_mapset("export-base-twice");
        let mut app = opened(&source);
        run_preview(&mut app, with_base(&[target], false));
        let generated = crate::generate_receipt::output_path(app.project_root.as_deref().unwrap(), &source).unwrap();
        let generated = generated.canonicalize().unwrap();
        let error = output::export_report(
            &output::Source::from_document(app.document().unwrap()), std::path::Path::new(""), None,
            app.project_root.as_deref(), &with_base(&[generated], false), &|| false,
            &mut Vec::new(), &mut |_, _| {},
        ).err().unwrap();
        assert_eq!(error, "The base hitsound difficulty cannot also be a Copy target.");
    }

    #[test]
    fn merge_settings_changed_in_the_editor_make_the_preview_stale() {
        let (_scratch, source, _, _) = mixed_mapset("export-base-settings");
        let mut app = opened(&source);
        let id = app.request_export_preview(with_base(&[], false)).unwrap();
        wait_preview(&mut app);
        assert!(!app.export_status().stale);
        let settings = crate::ExportSettings { pair_samples: true, ..crate::ExportSettings::default() };
        app.set_export_settings(settings.clone()).unwrap();
        assert!(app.export_status().stale);
        assert_eq!(app.export_status().stale_reason, Some(Stale::MergeSettings));
        let backups = app.data_root.join("backups");
        assert!(app.run_export_in(id, backups).is_err());

        // Copy does not use merge settings yet, so a Copy-only preview stays current.
        let (_scratch, source, target, _) = mixed_mapset("export-copy-settings");
        let mut app = opened(&source);
        app.request_export_preview(options(&[target])).unwrap();
        wait_preview(&mut app);
        app.set_export_settings(settings).unwrap();
        assert!(!app.export_status().stale);
        assert_eq!(app.export_status().stale_reason, None);
        edit(&mut app);
        assert_eq!(app.export_status().stale_reason, Some(Stale::Map));
    }

    #[test]
    fn the_base_choice_is_kept_and_older_choices_read_it_as_off() {
        let (_scratch, source, target, _) = mixed_mapset("export-base-choice");
        let mut app = opened(&source);
        app.request_export_preview(with_base(&[target], true)).unwrap();
        let choices = app.document().unwrap().export_choices().unwrap();
        assert!(choices.base && choices.slider_audio);
        let old: ExportChoices =
            serde_json::from_str(r#"{"left_out":[],"match_within":5.0,"copy_volumes":true}"#).unwrap();
        assert!(!old.base && !old.slider_audio);
    }

    /// The sample index files Generate stages under merge settings, after `already` and from `start`.
    fn indexed_mixes(source: &std::path::Path, already: &[crate::mixes::StagedMix], start: u32) -> Vec<crate::mixes::StagedMix> {
        let document = crate::Document::read(source).unwrap();
        let hits = document.composer.hitsounds();
        let dawsu_hitsounding::generate::GenerateOutcome::Planned { file, problems, .. } = dawsu_hitsounding::generate::generate(
            hits, &document.file, "source.osu", &[], &|| false, &mut |_, _| {},
        ) else {
            panic!("Generate planned nothing");
        };
        let settings = crate::ExportSettings::default();
        crate::mixes::stage_in_file(
            hits, file, &problems, source.parent().unwrap(), std::path::Path::new(""), &[],
            dawsu_hitsounding::Placement::OnObjects, already, Some(&settings), &[], start, &crate::pending::Pending::default(), &|| false,
        ).unwrap().1.staged
    }

    fn index_names(mixes: &[crate::mixes::StagedMix]) -> Vec<String> {
        mixes.iter().map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn merge_settings_mixes_skip_indexes_staged_earlier_and_honour_the_start_index() {
        let (_scratch, source, _, _) = mixed_mapset("export-indexes");
        let first = indexed_mixes(&source, &[], 2);
        assert!(!first.is_empty() && index_names(&first).iter().all(|name| name.ends_with("2.wav")), "{:?}", index_names(&first));
        let second = indexed_mixes(&source, &first, 2);
        assert!(index_names(&second).iter().all(|name| name.ends_with("3.wav")), "{:?}", index_names(&second));
        let custom = indexed_mixes(&source, &[], 7);
        assert!(index_names(&custom).iter().all(|name| name.ends_with("7.wav")), "{:?}", index_names(&custom));
        let both = first.into_iter().chain(second).collect::<Vec<_>>();
        let published = crate::mixes::publish(&both, None, &|| false).unwrap();
        drop(published);
    }

    /// A source slider whose tick plays soft-slidertick.wav, and a source circle on that tick
    /// with the same file: Generate's slider audio and Copy's timed tick sound render the same WAV.
    fn timed_mapset(name: &str) -> (Scratch, PathBuf, PathBuf) {
        let scratch = Scratch::new(name);
        let header = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:VERSION\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,2,1,100,1,0\n[HitObjects]\n";
        let slider = "100,100,1000,2,0,L|380:100,1,280,0|0,0:0|0:0,0:0:0:0:\n";
        std::fs::write(scratch.path("source.osu"), format!("{}{slider}256,192,1500,1,0,0:0:0:0:soft-slidertick.wav\n", header.replace("VERSION", "Source"))).unwrap();
        std::fs::write(scratch.path("target.osu"), format!("{}{slider}", header.replace("VERSION", "Target"))).unwrap();
        for (name, value) in [("soft-slidertick.wav", 8192_i16), ("soft-sliderslide.wav", 4096)] {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            for _ in 0..2 {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let canonical = |name: &str| scratch.path(name).canonicalize().unwrap();
        let (source, target) = (canonical("source.osu"), canonical("target.osu"));
        (scratch, source, target)
    }

    fn timed_copy() -> CopyOptions {
        CopyOptions {
            mode: dawsu_hitsounding::copy::CopyMode::OverwriteOnlyDefined,
            copy_to_slider_ticks: true, copy_to_slider_slides: true, ..CopyOptions::default()
        }
    }

    #[test]
    fn copy_never_points_a_target_at_a_wav_generate_owns() {
        let (_scratch, source, target) = timed_mapset("export-timed-owned");
        let mut app = opened(&source);
        let options = ExportOptions { targets: vec![target.clone()], copy: timed_copy(), base: true, slider_audio: true, ..ExportOptions::default() };
        app.request_export_preview(options.clone()).unwrap();
        wait_preview(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let plan = &status.preview.unwrap().plan;
        let count = plan.generated.as_ref().unwrap().1;
        let target_file = &plan.files.iter().find(|file| file.path == target).unwrap().file;
        let used = target_file.storyboard_samples().into_iter().map(|event| event.path).collect::<Vec<_>>();
        let generated = plan.mixes[..count].iter()
            .map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(!used.is_empty());
        assert!(used.iter().all(|name| !generated.contains(name)), "{used:?} uses {generated:?}");

        run_preview(&mut app, options);
        let exported = folder_files(source.parent().unwrap());
        let (_set, reference, reference_target) = timed_mapset("export-timed-owned-reference");
        generate_then_copy(&reference, reference_target, &timed_copy(), true);
        assert_eq!(exported, folder_files(reference.parent().unwrap()));
    }

    /// Every .wav in the mapset folder with its bytes.
    fn wav_files(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        folder_files(root).into_iter().filter(|(name, _)| name.ends_with(".wav")).collect()
    }

    fn index_of(name: &str) -> u32 {
        let stem = name.strip_suffix(".wav").unwrap();
        stem[stem.trim_end_matches(|c: char| c.is_ascii_digit()).len()..].parse().unwrap()
    }

    #[test]
    fn with_merge_settings_base_and_copy_never_share_an_index_and_re_export_adds_none() {
        // Generate mixes the circle under merge settings (indexed); Copy mixes a slider tail (indexed).
        let scratch = Scratch::new("export-merge-indexes");
        let header = MIXED.replace("[TimingPoints]", "[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]");
        let source = scratch.path("source.osu");
        std::fs::write(&source, format!("{}256,192,1500,1,0,0:0:0:40:one.wav\n300,192,1500,1,0,0:0:0:80:two.wav\n", header.replace("VERSION", "Source"))).unwrap();
        let target = scratch.path("target.osu");
        std::fs::write(&target, format!("{}256,192,1000,2,0,L|396:192,1,140,0|0,1:0|1:0,1:0:0:0:\n", header.replace("VERSION", "Target"))).unwrap();
        for (name, value) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            for _ in 0..2 {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let (source, target) = (source.canonicalize().unwrap(), target.canonicalize().unwrap());
        let root = source.parent().unwrap().to_path_buf();
        let mut app = opened(&source);
        app.set_export_settings(crate::ExportSettings::default()).unwrap();
        let copy = |start_index| CopyOptions {
            mode: dawsu_hitsounding::copy::CopyMode::OverwriteOnlyDefined, start_index, ..CopyOptions::default()
        };
        let export = |start_index| ExportOptions { targets: vec![target.clone()], copy: copy(start_index), base: true, slider_audio: false, ..ExportOptions::default() };
        let names = |mixes: &[crate::mixes::StagedMix]| mixes.iter()
            .map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();

        // A custom start index is honoured where it is free.
        app.request_export_preview(export(10)).unwrap();
        wait_preview(&mut app);
        assert_eq!(app.export_status().error, None);
        let plan = &app.export_status().preview.unwrap().plan;
        let count = plan.generated.as_ref().unwrap().1;
        let copy_names = names(&plan.mixes[count..]);
        assert_eq!(copy_names.iter().map(|name| index_of(name)).collect::<Vec<_>>(), [10], "{copy_names:?}");

        // Start 2 is Generate's: Copy takes the next free index, and no index holds two WAVs.
        let before = wav_files(&root);
        app.request_export_preview(export(2)).unwrap();
        wait_preview(&mut app);
        let plan = &app.export_status().preview.unwrap().plan;
        let count = plan.generated.as_ref().unwrap().1;
        let (base_names, copy_names) = (names(&plan.mixes[..count]), names(&plan.mixes[count..]));
        let indexes = |names: &[String]| names.iter().map(|name| index_of(name)).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(indexes(&base_names), [2].into(), "{base_names:?}");
        assert_eq!(indexes(&copy_names), [3].into(), "{copy_names:?}");
        run_preview(&mut app, export(2));
        let written = wav_files(&root);
        assert_eq!(written.len(), before.len() + base_names.len() + copy_names.len(), "one file per staged mix");

        let projects = app.project_root.clone().unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(
            &std::fs::read(crate::generate_receipt::path(&projects, &source)).unwrap()).unwrap();
        assert_eq!(receipt["mixes"], serde_json::json!(base_names), "only the base difficulty's mixes");

        app.request_export_preview(export(2)).unwrap();
        wait_preview(&mut app);
        assert!(app.export_status().preview.unwrap().plan.mixes.iter().all(|mix| mix.is_reused()));
        run_preview(&mut app, export(2));
        assert_eq!(wav_files(&root), written, "re-export adds and changes no WAV");
    }

    /// Asks for a base-only preview and answers its refusal; nothing in the mapset changes.
    fn base_refusal(app: &mut Application) -> String {
        let root = app.document().unwrap().path.parent().unwrap().to_path_buf();
        let before = folder_files(&root);
        app.request_export_preview(with_base(&[], false)).unwrap();
        wait_preview(app);
        let status = app.export_status();
        assert!(status.preview.is_none());
        let error = status.error.expect("Generate refused").to_owned();
        assert_eq!(folder_files(&root), before, "nothing is written");
        error
    }

    #[test]
    fn a_foreign_hitsounds_difficulty_refuses_the_base() {
        let (scratch, source, _, _) = mixed_mapset("export-base-foreign");
        std::fs::write(scratch.path("theirs.osu"), format!("{}100,100,1000,1,0,0:0:0:0:\n", MIXED.replace("VERSION", "Hitsounds"))).unwrap();
        let mut app = opened(&source);
        assert_eq!(base_refusal(&mut app), "A difficulty in this mapset is already called \"Hitsounds\". Rename it, then generate again.");
    }

    #[test]
    fn a_source_called_hitsounds_refuses_the_base() {
        let (scratch, _, _, _) = mixed_mapset("export-base-source-hitsounds");
        let source = scratch.path("Artist - Title (Mapper) [Hitsounds].osu");
        std::fs::write(&source, format!("{}256,192,1000,1,0,0:0:0:40:one.wav\n", MIXED.replace("VERSION", "Hitsounds"))).unwrap();
        let mut app = opened(&source.canonicalize().unwrap());
        assert_eq!(base_refusal(&mut app), "A difficulty already has the Generate output name. Only an earlier output tracked by this editor can be replaced.");
    }

    #[test]
    fn a_base_difficulty_edited_outside_is_kept() {
        let (_scratch, source, _, _) = mixed_mapset("export-base-edited");
        let mut app = opened(&source);
        run_preview(&mut app, with_base(&[], false));
        let generated = crate::generate_receipt::output_path(app.project_root.as_deref().unwrap(), &source).unwrap();
        let edited = std::fs::read_to_string(&generated).unwrap().replace("Version:Hitsounds", "Version:Hitsounds\nTags:edited");
        std::fs::write(&generated, &edited).unwrap();
        assert_eq!(base_refusal(&mut app), "The output changed since Generate. The existing difficulty was kept.");
        assert_eq!(std::fs::read_to_string(&generated).unwrap(), edited);
    }

    #[test]
    fn export_leaves_out_a_removed_lane_and_its_hits() {
        let scratch = Scratch::new("export-removed-lane");
        let header = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Test\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n";
        let source = scratch.path("source.osu");
        let target = scratch.path("target.osu");
        // A finish on the map's own normal-hitfinish.wav, and a whistle.
        std::fs::write(&source, format!("{header}256,192,1000,1,4,1:0:1:0:\n256,192,1500,1,2,0:0:0:0:\n")).unwrap();
        std::fs::write(&target, format!("{header}100,100,1000,1,0,0:0:0:0:\n100,100,1500,1,0,0:0:0:0:\n")).unwrap();
        std::fs::write(scratch.path("normal-hitfinish.wav"),
            b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40").unwrap();
        let mut app = opened(&source.canonicalize().unwrap());
        let lane = dawsu_hitsounding::rules::read_sample_file_name("normal-hitfinish.wav").unwrap();
        let path = app.document().unwrap().path.clone();
        let revision = app.document().unwrap().composer.history.version();
        app.delete_lane(&path, &lane, revision).unwrap();
        let options = ExportOptions { targets: vec![target.canonicalize().unwrap()], ..ExportOptions::default() };
        app.request_export_preview(options).unwrap();
        wait_preview(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let preview = status.preview.expect("a finished preview");
        let written = String::from_utf8(preview.plan.files[0].file.to_bytes()).unwrap();
        assert!(written.contains("100,100,1000,1,0,"), "no finish at 1000: {written}");
        assert!(written.contains("100,100,1500,1,2,"), "the whistle stays: {written}");
        assert!(preview.plan.mixes.is_empty());
    }
}
