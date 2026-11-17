use dawsu_hitsounding::SaveProblem;
use std::path::{Path, PathBuf};

pub struct OutputProblem {
    pub target: PathBuf,
    pub timeline: PathBuf,
    pub time: f64,
    pub message: String,
}

impl OutputProblem {
    pub(super) fn new(problem: &SaveProblem, target: &Path, timeline: &Path) -> Self {
        Self {
            target: target.into(),
            timeline: timeline.into(),
            time: problem.time.ms(),
            message: super::problem_description(&problem.kind).into(),
        }
    }

    pub(super) fn with_message(mut self, message: &str) -> Self {
        self.message = message.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::{Application, Document};
    use dawsu_foundation::SampleIdentity;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    fn wait(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline, "save check did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn save_review_preserves_diagnostics_and_continuation_uses_checked_save() {
        let scratch = Scratch::new("save-problem-review");
        let map = scratch.path("test.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let mut app = Application::headless();
        app.document = Some(Document::read(&map).unwrap());
        let d = app.document.as_mut().unwrap();
        let path = d.path.clone();
        for time in [250.0, 350.0] {
            d.composer
                .add(time, SampleIdentity::File("loose.wav".into()), 75, 0.0);
        }

        app.review_save();
        wait(&mut app);
        assert!(app.save_problem_review());
        assert!(app.error.is_none());
        assert_eq!(
            app.output_problems()
                .iter()
                .map(|p| p.time)
                .collect::<Vec<_>>(),
            [250.0, 350.0]
        );
        assert!(
            app.output_problems()
                .iter()
                .all(|p| p.target == path && p.timeline == path)
        );
        assert_eq!(std::fs::read(&map).unwrap(), original);

        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        assert!(!app.save_problem_review());
        assert!(
            app.error
                .as_deref()
                .unwrap()
                .contains("2 hits cannot be written")
        );
        assert_eq!(app.output_problems().len(), 2);
        assert!(app.is_dirty());
        assert_eq!(std::fs::read(&map).unwrap(), original);
        assert!(!scratch.path("backups").exists());

        // Diagnostics describe the checked edit revision. A subsequent edit
        // must not leave warnings on the old timeline positions.
        app.document.as_mut().unwrap().composer.undo();
        assert!(app.output_problems().is_empty());
        assert!(!app.save_problem_review());
    }
}
