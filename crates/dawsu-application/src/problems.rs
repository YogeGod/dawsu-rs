use dawsu_hitsounding::{Hitsounds, Place, SaveProblem};
use std::path::{Path, PathBuf};

#[derive(Clone)]
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
            message: super::problem_description(&problem.kind, problem.time.ms()),
        }
    }

    pub(super) fn with_message(mut self, message: &str) -> Self {
        self.message = message.into();
        self
    }
}

/// The times of hits with no object (v3's "orphaned"): moments that hold at least one hit and sit
/// on no object, once per moment. Outside HS mode Save keeps them in the project and osu! never
/// plays them. In HS mode Save makes them circles, so none is orphaned.
pub fn orphaned_hit_times(hits: &Hitsounds, hs_mode: bool) -> Vec<f64> {
    if hs_mode {
        return Vec::new();
    }
    hits.moments().iter()
        .filter(|moment| moment.place() == Place::NoObject && moment.hits().next().is_some())
        .map(|moment| moment.time().ms())
        .collect()
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

    #[test]
    fn orphaned_hits_are_the_times_of_moments_with_hits_and_no_object() {
        let scratch = Scratch::new("orphaned-hits");
        let map = scratch.path("test.osu");
        std::fs::write(&map, include_bytes!("../tests/project-fixture.osu")).unwrap();
        let mut document = Document::read(&map).unwrap();
        assert!(crate::orphaned_hit_times(document.composer.hitsounds(), false).is_empty(), "the circle at 1000 ms has an object");
        for (time, lane) in [(250.0, "loose.wav"), (250.0, "other.wav"), (1000.0, "loose.wav"), (1500.0, "loose.wav")] {
            document.composer.add(time, SampleIdentity::File(lane.into()), 75, 0.0);
        }
        // Two hits at one time are one moment; a hit on the circle is not orphaned.
        assert_eq!(crate::orphaned_hit_times(document.composer.hitsounds(), false), [250.0, 1500.0]);
        assert!(crate::orphaned_hit_times(document.composer.hitsounds(), true).is_empty(), "HS mode makes them circles");
    }

    #[test]
    fn writable_toolbar_review_does_not_save_until_the_mapper_confirms() {
        let scratch = Scratch::new("staged-toolbar-save");
        let map = scratch.path("test.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let mut app = Application::headless();
        app.document = Some(Document::read(&map).unwrap());
        app.document.as_mut().unwrap().composer.apply("Change volume", |hits| {
            hits.set_volume(dawsu_hitsounding::MomentId(0), 75);
        });
        app.review_save();
        wait(&mut app);
        assert!(app.is_dirty());
        assert_eq!(std::fs::read(&map).unwrap(), original);
    }
}
