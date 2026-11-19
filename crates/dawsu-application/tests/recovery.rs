use dawsu_application::{Application, PointerPhase};
use dawsu_editor::tools::{PointerInput, ToolKind};
use dawsu_foundation::{SampleIdentity, digest::sha256_hex};
use dawsu_hitsounding::{LaneSource, Place};
use dawsu_test_support::Scratch;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const FIXTURE: &[u8] = include_bytes!("project-fixture.osu");

struct TestMap {
    _scratch: Scratch,
    path: PathBuf,
    data: PathBuf,
}

impl TestMap {
    fn new() -> Self {
        let scratch = Scratch::new("recovery");
        let path = scratch.path("copy.osu");
        fs::write(&path, FIXTURE).unwrap();
        Self {
            data: scratch.path("app-data"),
            _scratch: scratch,
            path,
        }
    }

    fn app(&self) -> Application {
        let mut app = Application::with_data_root(self.data.clone());
        // A machine without an audio device can still edit a difficulty.
        app.error = None;
        app
    }
}

fn finish_job(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy {
        assert!(
            Instant::now() < deadline,
            "Job did not finish: {}",
            app.status
        );
        app.poll();
        thread::sleep(Duration::from_millis(2));
    }
}

fn open(app: &mut Application, path: &Path) {
    app.request_open(path.to_owned());
    finish_job(app);
    assert!(app.document().is_some(), "{:?}", app.error);
}

fn draw(app: &mut Application, time: f64, row: i32, phase: PointerPhase) {
    app.pointer(phase, PointerInput::at(time, row));
}

fn change_volume(app: &mut Application, value: i32) {
    app.command("selectAll");
    app.selection_volume(value, false);
}

fn volume(app: &Application) -> i32 {
    app.document().unwrap().composer.hitsounds().moments()[0].volume()
}

fn restore(app: &mut Application, id: &str) {
    app.restore_recovery(id).unwrap();
    finish_job(app);
}

#[test]
fn recovery_restores_a_complete_unplaced_hit_and_leaves_an_open_stroke_out() {
    let map = TestMap::new();
    let sample = "snare.wav";
    fs::write(map.path.with_file_name(sample), []).unwrap();
    let mut editor = map.app();
    open(&mut editor, &map.path);
    let lanes = editor.document().unwrap().composer.lanes.clone();
    let lane = SampleIdentity::File(sample.into());
    let row = lanes.iter().position(|identity| identity == &lane).unwrap() as i32;
    let time = 1375.0000000000007_f64;
    editor.set_tool(ToolKind::Draw);
    editor.set_snap(false, 4);
    editor.set_insert_volume(42);
    draw(&mut editor, time, row, PointerPhase::Down);
    draw(&mut editor, time, row, PointerPhase::Up);
    draw(&mut editor, 1625.0, row, PointerPhase::Down);
    let copy = editor.write_recovery_copy().unwrap().unwrap();
    drop(editor);

    let mut restarted = map.app();
    assert_eq!(restarted.recovery_offers().unwrap(), [copy.clone()]);
    assert_eq!(copy.mapset, "Project persistence test");
    assert_eq!(copy.difficulty, "Hard");
    restore(&mut restarted, &copy.id);
    let doc = restarted.document().unwrap();
    let hits = doc.composer.hitsounds();
    let recovered = hits.moments();
    assert_eq!(recovered.len(), 2);
    assert_eq!(recovered[1].time().ms().to_bits(), time.to_bits());
    assert_eq!(recovered[1].volume(), 42);
    assert_eq!(recovered[1].place(), Place::NoObject);
    assert!(recovered[1].has_hit(&lane));
    assert_eq!(doc.composer.lanes, lanes);
    assert_eq!(
        hits.lane(&lane).unwrap().source,
        LaneSource::File(sample.into())
    );
    assert!(restarted.is_dirty());
    assert_eq!(
        sha256_hex(&fs::read(&map.path).unwrap()),
        sha256_hex(FIXTURE)
    );
    // Save keeps an unplaced hit in the editor project, so the difficulty bytes stay the same.
    restarted.save();
    finish_job(&mut restarted);
    assert_eq!(restarted.error, None);
    assert_eq!(restarted.status, "Saved with backup");
    assert!(!restarted.is_dirty());
    assert_eq!(fs::read(&map.path).unwrap(), FIXTURE);
    assert!(restarted.recovery_offers().unwrap().is_empty());
    assert!(map.data.join("recovery/archive").join(&copy.id).is_file());
    drop(restarted);

    let mut reopened = map.app();
    open(&mut reopened, &map.path);
    let hits = reopened.document().unwrap().composer.hitsounds();
    let moments = hits.moments();
    assert_eq!(moments.len(), 2);
    assert_eq!(moments[1].volume(), 42);
    assert_eq!(moments[1].place(), Place::NoObject);
    assert!(moments[1].has_hit(&lane));
    assert!(!reopened.is_dirty());
}

#[test]
fn recovery_keeps_the_saved_baseline_and_can_save_a_recovered_undo() {
    let map = TestMap::new();
    let mut editor = map.app();
    open(&mut editor, &map.path);
    change_volume(&mut editor, 63);
    editor.save();
    finish_job(&mut editor);
    assert!(!editor.is_dirty(), "{:?}", editor.error);
    let saved = fs::read(&map.path).unwrap();
    assert_ne!(saved, FIXTURE);
    editor.command("undo");
    assert_eq!(volume(&editor), 100);
    assert!(editor.is_dirty());
    let copy = editor.write_recovery_copy().unwrap().unwrap();
    let other = editor.write_recovery_copy().unwrap().unwrap();
    drop(editor);

    let mut restarted = map.app();
    restore(&mut restarted, &copy.id);
    assert!(restarted.document().is_some(), "{:?}", restarted.error);
    assert_eq!(volume(&restarted), 100);
    assert!(restarted.is_dirty());
    assert_eq!(fs::read(&map.path).unwrap(), saved);
    assert_eq!(restarted.recovery_offers().unwrap().len(), 2);
    restarted.command("undo");
    assert_eq!(volume(&restarted), 63);
    assert!(!restarted.is_dirty());
    restarted.command("redo");
    assert_eq!(volume(&restarted), 100);
    assert!(restarted.is_dirty());
    restarted.save();
    finish_job(&mut restarted);
    assert_eq!(restarted.error, None);
    assert!(!restarted.is_dirty());
    assert_eq!(fs::read(&map.path).unwrap(), FIXTURE);
    assert_eq!(restarted.recovery_offers().unwrap(), [other]);
    assert!(map.data.join("recovery/archive").join(copy.id).is_file());
}

#[test]
fn changed_or_missing_source_keeps_the_offer_available_for_retry() {
    let map = TestMap::new();
    let mut editor = map.app();
    open(&mut editor, &map.path);
    change_volume(&mut editor, 63);
    let copy = editor.write_recovery_copy().unwrap().unwrap();
    drop(editor);

    let mut restarted = map.app();
    assert!(restarted.restore_recovery("../copy.json").is_err());
    let mut changed = FIXTURE.to_vec();
    changed.extend_from_slice(b"\n// external edit\n");
    fs::write(&map.path, &changed).unwrap();
    restore(&mut restarted, &copy.id);
    assert!(restarted.document().is_none());
    assert!(restarted.error.take().unwrap().contains("changed on disk"));
    assert_eq!(restarted.recovery_offers().unwrap(), [copy.clone()]);
    assert_eq!(fs::read(&map.path).unwrap(), changed);

    let moved = map.path.with_extension("moved.osu");
    fs::rename(&map.path, &moved).unwrap();
    restore(&mut restarted, &copy.id);
    assert!(restarted.document().is_none());
    assert!(restarted.error.take().unwrap().contains("missing or moved"));
    assert_eq!(restarted.recovery_offers().unwrap(), [copy.clone()]);

    fs::write(&map.path, FIXTURE).unwrap();
    restore(&mut restarted, &copy.id);
    assert_eq!(restarted.error, None);
    assert_eq!(volume(&restarted), 63);
    assert!(restarted.is_dirty());
    assert_eq!(restarted.recovery_offers().unwrap(), [copy]);
}

#[test]
fn archive_failure_keeps_the_copy_and_reports_the_successful_save_separately() {
    let map = TestMap::new();
    let mut editor = map.app();
    open(&mut editor, &map.path);
    change_volume(&mut editor, 63);
    let copy = editor.write_recovery_copy().unwrap().unwrap();
    drop(editor);

    let mut restarted = map.app();
    restore(&mut restarted, &copy.id);
    let archive = map.data.join("recovery/archive");
    fs::write(&archive, b"blocking the archive directory").unwrap();
    restarted.save();
    finish_job(&mut restarted);
    assert!(!restarted.is_dirty(), "{:?}", restarted.error);
    assert!(restarted.status.contains("Difficulty saved"));
    assert!(restarted.error.take().unwrap().contains("archive"));
    assert_ne!(fs::read(&map.path).unwrap(), FIXTURE);
    assert_eq!(restarted.recovery_offers().unwrap(), [copy.clone()]);

    fs::remove_file(&archive).unwrap();
    restarted.save();
    finish_job(&mut restarted);
    assert_eq!(restarted.error, None);
    assert!(restarted.recovery_offers().unwrap().is_empty());
    assert!(archive.join(copy.id).is_file());
}
