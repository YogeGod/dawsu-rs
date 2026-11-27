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
        app.clear_error();
        app
    }
}

fn finish_job(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.is_busy() {
        assert!(
            Instant::now() < deadline,
            "Job did not finish: {}",
            app.status()
        );
        app.poll();
        thread::sleep(Duration::from_millis(2));
    }
}

fn open(app: &mut Application, path: &Path) {
    app.request_open(path.to_owned());
    finish_job(app);
    assert!(app.document().is_some(), "{:?}", app.error());
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
    assert_eq!(restarted.recovery_offers().unwrap(), std::slice::from_ref(&copy));
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
    assert_eq!(restarted.error(), None);
    assert_eq!(restarted.status(), "Saved with backup");
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
    assert!(!editor.is_dirty(), "{:?}", editor.error());
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
    assert!(restarted.document().is_some(), "{:?}", restarted.error());
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
    assert_eq!(restarted.error(), None);
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
    assert!(restarted.take_error().unwrap().contains("changed on disk"));
    assert_eq!(restarted.recovery_offers().unwrap(), std::slice::from_ref(&copy));
    assert_eq!(fs::read(&map.path).unwrap(), changed);

    let moved = map.path.with_extension("moved.osu");
    fs::rename(&map.path, &moved).unwrap();
    restore(&mut restarted, &copy.id);
    assert!(restarted.document().is_none());
    assert!(restarted.take_error().unwrap().contains("missing or moved"));
    assert_eq!(restarted.recovery_offers().unwrap(), std::slice::from_ref(&copy));

    fs::write(&map.path, FIXTURE).unwrap();
    restore(&mut restarted, &copy.id);
    assert_eq!(restarted.error(), None);
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
    assert!(!restarted.is_dirty(), "{:?}", restarted.error());
    assert!(restarted.status().contains("Difficulty saved"));
    assert!(restarted.take_error().unwrap().contains("archive"));
    assert_ne!(fs::read(&map.path).unwrap(), FIXTURE);
    assert_eq!(restarted.recovery_offers().unwrap(), std::slice::from_ref(&copy));

    fs::remove_file(&archive).unwrap();
    restarted.save();
    finish_job(&mut restarted);
    assert_eq!(restarted.error(), None);
    assert!(restarted.recovery_offers().unwrap().is_empty());
    assert!(archive.join(copy.id).is_file());
}

fn pending_files(map: &TestMap) -> Vec<String> {
    let Ok(entries) = fs::read_dir(map.data.join("recovery")) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    names
}

/// Polls as the editor's idle loop does until the timed recovery copy is on disk and its
/// writer has finished.
fn poll_until_autosaved(app: &mut Application, map: &TestMap, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll();
        if pending_files(map).len() == count && !app.has_active_background_jobs() {
            return;
        }
        assert!(Instant::now() < deadline, "no autosave: {:?}", pending_files(map));
        thread::sleep(Duration::from_millis(2));
    }
}

fn poll_a_while(app: &mut Application) {
    for _ in 0..20 {
        app.poll();
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn autosave_writes_one_copy_of_a_dirty_map_that_a_restart_restores() {
    let map = TestMap::new();
    let mut editor = map.app();
    open(&mut editor, &map.path);
    poll_a_while(&mut editor);
    assert!(pending_files(&map).is_empty(), "a clean map gets no copy");

    change_volume(&mut editor, 63);
    poll_until_autosaved(&mut editor, &map, 1);
    let first = pending_files(&map);

    change_volume(&mut editor, 71);
    editor.autosave_recovery_now();
    let second = pending_files(&map);
    assert_eq!(second.len(), 1, "turnover keeps exactly one copy");
    assert_ne!(second, first, "the newer edit is a new copy");
    editor.autosave_recovery_now();
    assert_eq!(pending_files(&map), second, "an unchanged edit writes nothing");
    drop(editor);

    let mut restarted = map.app();
    let offers = restarted.recovery_offers().unwrap();
    assert_eq!(offers.iter().map(|o| o.id.clone()).collect::<Vec<_>>(), second);
    restore(&mut restarted, &offers[0].id);
    assert_eq!(volume(&restarted), 71);
    assert!(restarted.is_dirty());
    assert_eq!(fs::read(&map.path).unwrap(), FIXTURE);
    // The restored copy already holds this state: no second card until the next edit.
    poll_a_while(&mut restarted);
    assert_eq!(pending_files(&map), second);

    change_volume(&mut restarted, 80);
    poll_until_autosaved(&mut restarted, &map, 2);
    assert!(pending_files(&map).contains(&second[0]), "turnover never removes a restored copy");
}

fn autosaved_edit(map: &TestMap) -> Application {
    let mut editor = map.app();
    open(&mut editor, &map.path);
    change_volume(&mut editor, 63);
    poll_until_autosaved(&mut editor, map, 1);
    editor
}

#[test]
fn autosave_copy_is_removed_by_save_discard_close_switching_and_undo_to_clean() {
    let map = TestMap::new();

    let mut editor = autosaved_edit(&map);
    editor.save();
    finish_job(&mut editor);
    assert!(!editor.is_dirty(), "{:?}", editor.error());
    assert_eq!(pending_files(&map), Vec::<String>::new(), "after Save");
    poll_a_while(&mut editor);
    assert_eq!(pending_files(&map), Vec::<String>::new(), "a saved map gets no new copy");
    drop(editor);

    let map = TestMap::new();
    let mut editor = autosaved_edit(&map);
    editor.close_document();
    assert_eq!(pending_files(&map), Vec::<String>::new(), "after close_document");
    drop(editor);

    let map = TestMap::new();
    let mut editor = autosaved_edit(&map);
    editor.close_document_nonblocking();
    assert_eq!(pending_files(&map), Vec::<String>::new(), "after close_document_nonblocking");
    drop(editor);

    let map = TestMap::new();
    let other = map.path.with_file_name("other.osu");
    fs::write(&other, FIXTURE).unwrap();
    let mut editor = autosaved_edit(&map);
    editor.request_open(other.clone());
    editor.discard_and_open();
    finish_job(&mut editor);
    assert!(editor.document().unwrap().path.ends_with("other.osu"));
    assert_eq!(pending_files(&map), Vec::<String>::new(), "after switching difficulty");
    drop(editor);

    let map = TestMap::new();
    let mut editor = autosaved_edit(&map);
    editor.command("undo");
    assert!(!editor.is_dirty());
    editor.poll();
    assert_eq!(pending_files(&map), Vec::<String>::new(), "after Undo to the saved state");
    editor.command("redo");
    poll_until_autosaved(&mut editor, &map, 1);
}

#[test]
fn autosave_copy_answered_after_a_close_starts_is_removed_too() {
    let map = TestMap::new();
    let mut editor = map.app();
    open(&mut editor, &map.path);
    change_volume(&mut editor, 63);
    editor.poll();
    // The writer finishes on its own; its answer is still unread when the map closes.
    let deadline = Instant::now() + Duration::from_secs(5);
    while pending_files(&map).is_empty() {
        assert!(Instant::now() < deadline, "the writer never wrote");
        thread::sleep(Duration::from_millis(2));
    }
    editor.close_document();
    assert_eq!(pending_files(&map), Vec::<String>::new());
    assert!(!editor.has_active_background_jobs());
}

#[test]
fn a_crash_copy_replaces_the_timed_copy_and_a_restored_copy_survives_a_discard_close() {
    let map = TestMap::new();
    let mut editor = autosaved_edit(&map);
    change_volume(&mut editor, 71);
    let crash = editor.write_recovery_copy().unwrap().unwrap();
    assert_eq!(pending_files(&map), std::slice::from_ref(&crash.id), "one card after a crash");
    drop(editor);

    let mut restarted = map.app();
    restore(&mut restarted, &crash.id);
    change_volume(&mut restarted, 80);
    poll_until_autosaved(&mut restarted, &map, 2);
    restarted.close_document();
    assert_eq!(pending_files(&map), [crash.id], "the restored copy stays until a Save");
}

#[test]
fn discard_moves_a_copy_to_the_archive_and_refuses_what_restore_refuses() {
    let map = TestMap::new();
    let mut editor = map.app();
    open(&mut editor, &map.path);
    change_volume(&mut editor, 63);
    let copy = editor.write_recovery_copy().unwrap().unwrap();
    assert!(editor.discard_recovery(&copy.id).is_err(), "only from the start screen");
    drop(editor);

    let mut restarted = map.app();
    assert!(restarted.discard_recovery("../copy.json").is_err());
    let before = restarted.view_state_token();
    restarted.discard_recovery(&copy.id).unwrap();
    assert_ne!(restarted.view_state_token(), before, "the next view drops the card");
    assert!(restarted.recovery_offers().unwrap().is_empty());
    assert!(map.data.join("recovery/archive").join(&copy.id).is_file());
    assert_eq!(fs::read(&map.path).unwrap(), FIXTURE, "the map is untouched");
}

#[test]
fn a_copy_that_cannot_be_restored_says_to_discard_it() {
    let map = TestMap::new();
    let mut editor = map.app();
    open(&mut editor, &map.path);
    change_volume(&mut editor, 63);
    let copy = editor.write_recovery_copy().unwrap().unwrap();
    drop(editor);
    let mut changed = FIXTURE.to_vec();
    changed.extend_from_slice(b"\n// external edit\n");
    fs::write(&map.path, &changed).unwrap();
    let mut restarted = map.app();
    restore(&mut restarted, &copy.id);
    let error = restarted.take_error().unwrap();
    assert!(error.contains("changed on disk. The recovery copy was kept. Use Discard to remove this card."), "{error}");

    fs::remove_file(&map.path).unwrap();
    restore(&mut restarted, &copy.id);
    let missing = restarted.take_error().unwrap();
    assert!(missing.contains("missing or moved") && missing.contains("Use Discard to remove this card."), "{missing}");
}

#[test]
fn autosave_off_removes_the_live_copy_but_never_a_restored_one() {
    let map = TestMap::new();
    let mut editor = autosaved_edit(&map);
    let crash = editor.write_recovery_copy().unwrap().unwrap();
    drop(editor);

    let mut restarted = map.app();
    restore(&mut restarted, &crash.id);
    change_volume(&mut restarted, 80);
    poll_until_autosaved(&mut restarted, &map, 2);
    restarted.set_autosave_interval_secs(0);
    assert_eq!(pending_files(&map), std::slice::from_ref(&crash.id));
    change_volume(&mut restarted, 81);
    poll_a_while(&mut restarted);
    assert_eq!(pending_files(&map), [crash.id], "Off writes no timed copy");
}
