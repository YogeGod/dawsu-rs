//! The Export window's preview, kept apart from main's job, error, problem list and review.
use crate::{Application, OutputProblem, output};
use dawsu_hitsounding::copy::CopyOptions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

/// What the Export window asks to write. For now only "Into the mapset" (Copy).
#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub targets: Vec<PathBuf>,
    pub copy: CopyOptions,
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
}

type PreviewResult = (Result<output::WritePlan, String>, Vec<OutputProblem>);

struct Running {
    id: u64,
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<PreviewResult>,
    progress: Arc<Mutex<Option<(usize, usize)>>>,
}

#[derive(Default)]
pub(crate) struct Slot {
    last_id: u64,
    waiting: Option<(u64, ExportOptions)>,
    running: Option<Running>,
    preview: Option<ExportPreview>,
    problems: Vec<OutputProblem>,
    error: Option<String>,
}

impl Slot {
    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.waiting = None;
    }
}

pub struct ExportStatus<'a> {
    /// The id of the latest preview request, finished or not.
    pub requested: Option<u64>,
    pub preview: Option<&'a ExportPreview>,
    /// The preview was made from another difficulty or an older edit.
    pub stale: bool,
    /// Options wait for main's job or a drag to end.
    pub waiting: bool,
    pub running: bool,
    pub progress: Option<(usize, usize)>,
    pub problems: &'a [OutputProblem],
    pub error: Option<&'a str>,
}

impl Application {
    /// Queues a preview and answers at once with its id. A newer request drops the
    /// finished preview and any running one, so Export is never run on old options.
    pub fn request_export_preview(&mut self, options: ExportOptions) -> Result<u64, String> {
        if self.document.is_none() {
            return Err("Open a difficulty first.".into());
        }
        self.export.stop();
        self.export.preview = None;
        self.export.problems.clear();
        self.export.error = None;
        self.export.last_id += 1;
        let id = self.export.last_id;
        self.export.waiting = Some((id, options));
        self.start_export_preview();
        Ok(id)
    }

    pub fn cancel_export_preview(&mut self) {
        self.export.stop();
    }

    pub fn export_status(&self) -> ExportStatus<'_> {
        let slot = &self.export;
        let current = self.document.as_ref().map(|d| (d.path.as_path(), d.composer.history.version()));
        ExportStatus {
            requested: (slot.last_id > 0).then_some(slot.last_id),
            preview: slot.preview.as_ref(),
            stale: slot.preview.as_ref().is_some_and(|preview| {
                current != Some((preview.plan.source.as_path(), preview.plan.version))
            }),
            waiting: slot.waiting.is_some(),
            running: slot.running.is_some(),
            progress: slot.running.as_ref()
                .and_then(|running| running.progress.lock().ok().and_then(|current| *current)),
            problems: &slot.problems,
            error: slot.error.as_deref(),
        }
    }

    pub fn backups_folder(&self) -> PathBuf {
        self.data_root.join("backups")
    }

    /// Writes the preview with this id. Refused unless it is the latest preview and
    /// the difficulty is still the one, at the same edit, that it was made from.
    pub fn run_export(&mut self, preview_id: u64) -> Result<(), String> {
        self.run_export_in(preview_id, self.backups_folder())
    }

    pub(crate) fn run_export_in(&mut self, preview_id: u64, backups: PathBuf) -> Result<(), String> {
        if self.busy {
            return Err("Wait for the current job to finish.".into());
        }
        if self.is_stroke_open() {
            return Err("Finish or cancel the current drag before exporting.".into());
        }
        let current = self.document.as_ref().map(|d| (d.path.clone(), d.composer.history.version()));
        let matches = self.export.preview.as_ref().is_some_and(|preview| {
            preview.id == preview_id
                && current == Some((preview.plan.source.clone(), preview.plan.version))
        });
        if !matches {
            return Err("The map or the options changed since this preview. Preview again.".into());
        }
        let preview = self.export.preview.take().expect("checked above");
        self.write_reviewed(preview.plan, backups);
        Ok(())
    }

    /// Collects a finished preview and starts waiting options once main is free.
    pub(crate) fn poll_export(&mut self) {
        if let Some(running) = &self.export.running {
            match running.receive.try_recv() {
                Ok((result, problems)) => {
                    let id = running.id;
                    self.export.running = None;
                    self.export.problems = problems;
                    match result {
                        Ok(plan) => {
                            self.export.preview = Some(ExportPreview { id, origin: PreviewOrigin::Export, plan });
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
        if self.export.running.is_some() || self.busy || self.is_stroke_open() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some((id, options)) = self.export.waiting.take() else {
            return;
        };
        let source = output::Source::from_document(document);
        let defaults = self.preferences.default_samples.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(None));
        let (send, receive) = mpsc::channel();
        self.export.running = Some(Running { id, cancel: cancel.clone(), receive, progress: progress.clone() });
        std::thread::spawn(move || {
            let mut problems = Vec::new();
            let result = output::copy_report(
                &source,
                &defaults,
                &options.targets,
                &options.copy,
                &|| cancel.load(Ordering::Relaxed),
                &mut problems,
                &mut |done, total| {
                    if let Ok(mut current) = progress.lock() {
                        *current = Some((done, total));
                    }
                },
            );
            let _ = send.send((result, problems));
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

    fn options(targets: &[PathBuf]) -> ExportOptions {
        ExportOptions {
            targets: targets.to_vec(),
            copy: CopyOptions { leniency: 5.0, copy_storyboarded_samples: true, ..CopyOptions::default() },
        }
    }

    #[test]
    fn export_preview_plans_the_same_writes_as_copy() {
        let set = mapset("export-parity");
        let mut app = opened(&set.source);
        assert!(app.preferences.default_samples.as_os_str().is_empty());
        let options = options(&[set.target.clone()]);
        let id = app.request_export_preview(options.clone()).unwrap();
        wait_preview(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let preview = status.preview.expect("a finished preview");
        assert_eq!((preview.id, preview.origin), (id, PreviewOrigin::Export));
        assert!(!status.stale);

        let source = output::Source::from_document(app.document().unwrap());
        let copy = output::copy(&source, &options.targets, &options.copy, &|| false).unwrap();
        let plan = &preview.plan;
        assert_eq!((&plan.label, &plan.source, plan.version), (&copy.label, &copy.source, copy.version));
        assert!(!plan.details.is_empty());
        assert_eq!(plan.details, copy.details);
        assert_eq!(plan.mix_paths().collect::<Vec<_>>(), copy.mix_paths().collect::<Vec<_>>());
        let files = |plan: &output::WritePlan| plan.files.iter()
            .map(|file| (file.path.clone(), file.file.to_bytes(), file.expected.clone()))
            .collect::<Vec<_>>();
        assert_eq!(files(plan), files(&copy));
        assert_ne!(Some(plan.files[0].file.to_bytes()), plan.files[0].expected, "the preview changes the target");
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
        assert_eq!(status.error, Some("Choose at least one target difficulty"));
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
}
