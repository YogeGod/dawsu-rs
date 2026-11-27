//! Timed recovery copies racing a close, a Save, a switch, an Undo or a crash copy. The writer is
//! held past its cancel check, so the copy it makes lands after the drop started.

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const FIXTURE: &[u8] = include_bytes!("../../tests/project-fixture.osu");

struct Held {
    _scratch: Scratch,
    app: Application,
    map: PathBuf,
    data: PathBuf,
    hold: Arc<AtomicBool>,
}

fn held() -> Held {
    let scratch = Scratch::new("autosave-held");
    let folder = scratch.path("mapset");
    std::fs::create_dir_all(&folder).unwrap();
    let map = folder.join("copy.osu");
    std::fs::write(&map, FIXTURE).unwrap();
    let data = scratch.path("data");
    let mut app = Application::with_data_root(data.clone());
    app.clear_error();
    open(&mut app, map.clone());
    let hold = Arc::new(AtomicBool::new(true));
    app.autosave_write_hold = Some(hold.clone());
    Held { _scratch: scratch, app, map, data, hold }
}

fn edit(app: &mut Application, volume: i32) {
    app.document.as_mut().unwrap().composer.apply("Volume", |hits| {
        hits.set_volume(dawsu_hitsounding::MomentId(0), volume)
    });
}

fn files(data: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(data.join("recovery")) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".json"))
        .collect();
    names.sort();
    names
}

/// Starts a held write of an edit and gives the writer time to reach the hold.
fn start_held_write(h: &mut Held, volume: i32) {
    edit(&mut h.app, volume);
    h.app.poll();
    assert!(h.app.autosave_job.is_some(), "the edit starts a timed write");
    assert!(h.app.has_active_background_jobs(), "a timed write in flight is background work");
    std::thread::sleep(Duration::from_millis(50));
    assert!(files(&h.data).is_empty(), "held before the file is made");
}

#[test]
fn autosave_held_writer_racing_a_close_leaves_no_copy() {
    let mut h = held();
    start_held_write(&mut h, 63);
    h.app.close_document();
    assert_eq!(files(&h.data), Vec::<String>::new());
    assert!(!h.app.has_active_background_jobs());

    let mut h = held();
    start_held_write(&mut h, 63);
    h.app.close_document_nonblocking();
    assert_eq!(files(&h.data), Vec::<String>::new());
}

#[test]
fn autosave_held_writer_racing_a_save_leaves_no_copy() {
    let mut h = held();
    start_held_write(&mut h, 63);
    h.app.save();
    wait(&mut h.app);
    assert!(!h.app.has_dirty_beatmap(), "{:?}", h.app.error);
    assert_eq!(files(&h.data), Vec::<String>::new());
}

#[test]
fn autosave_held_writer_racing_a_switch_or_undo_leaves_no_copy() {
    let mut h = held();
    let other = h.map.with_file_name("other.osu");
    std::fs::write(&other, FIXTURE).unwrap();
    start_held_write(&mut h, 63);
    h.app.request_open(other.clone());
    h.app.discard_and_open();
    wait(&mut h.app);
    assert!(h.app.document().unwrap().path.ends_with("other.osu"));
    assert_eq!(files(&h.data), Vec::<String>::new());

    let mut h = held();
    start_held_write(&mut h, 63);
    h.app.command("undo");
    assert!(!h.app.has_dirty_beatmap());
    h.app.poll();
    assert_eq!(files(&h.data), Vec::<String>::new());
    assert!(!h.app.has_active_background_jobs());
}

#[test]
fn autosave_held_writer_then_a_crash_copy_leaves_exactly_the_crash_copy() {
    let mut h = held();
    start_held_write(&mut h, 63);
    let crash = h.app.write_recovery_copy().unwrap().unwrap();
    assert_eq!(files(&h.data), [crash.id]);
    assert!(!h.app.has_active_background_jobs());
}

#[test]
fn autosave_survives_a_failed_crash_copy() {
    let mut h = held();
    h.hold.store(false, Ordering::Relaxed);
    edit(&mut h.app, 63);
    h.app.autosave_recovery_now();
    let timed = files(&h.data);
    assert_eq!(timed.len(), 1);
    // A crash copy that cannot be written keeps the timed copy.
    let checkpoint = h.app.checkpoint.take();
    h.app.data_root = h.data.join("recovery").join(&timed[0]);
    h.app.checkpoint = checkpoint;
    assert!(h.app.write_recovery_copy().is_err());
    assert_eq!(files(&h.data), timed);
}

#[test]
fn autosave_follows_the_interval_and_a_shorter_one_takes_effect_on_the_next_poll() {
    let mut h = held();
    h.hold.store(false, Ordering::Relaxed);
    h.app.set_autosave_interval_secs(120);
    let start = Instant::now();
    edit(&mut h.app, 63);
    h.app.autosave_recovery(start);
    h.app.autosave_recovery_now();
    assert_eq!(files(&h.data).len(), 1, "the first edit after a quiet period is written at once");
    let first = files(&h.data);

    edit(&mut h.app, 64);
    h.app.refresh_recovery_edit();
    h.app.autosave_recovery(start + Duration::from_secs(16));
    assert!(h.app.autosave_job.is_none(), "not due before 120 s");
    h.app.set_autosave_interval_secs(15);
    h.app.autosave_recovery(start + Duration::from_secs(16));
    assert!(h.app.autosave_job.is_some(), "15 s have passed");
    h.app.autosave_recovery_now();
    assert_ne!(files(&h.data), first);
    assert_eq!(files(&h.data).len(), 1);

    // Off stops timed writes and drops this session's copy.
    h.app.set_autosave_interval_secs(0);
    assert_eq!(files(&h.data), Vec::<String>::new());
    edit(&mut h.app, 65);
    h.app.refresh_recovery_edit();
    h.app.autosave_recovery(start + Duration::from_secs(10_000));
    assert!(h.app.autosave_job.is_none(), "Off never writes");
    // Back on, the dirty map is written on the very next poll.
    h.app.set_autosave_interval_secs(30);
    h.app.autosave_recovery(start + Duration::from_secs(10_000));
    assert!(h.app.autosave_job.is_some());
    h.app.autosave_recovery_now();
}

#[test]
fn autosave_write_error_shows_one_status_line_and_retries_at_the_next_interval() {
    let mut h = held();
    h.hold.store(false, Ordering::Relaxed);
    // A file where the recovery folder should be makes every write fail.
    std::fs::create_dir_all(&h.data).unwrap();
    std::fs::write(h.data.join("recovery"), b"blocking").unwrap();
    let start = Instant::now();
    edit(&mut h.app, 63);
    h.app.refresh_recovery_edit();
    h.app.autosave_recovery(start);
    h.app.finish_autosave(true);
    assert!(h.app.status.starts_with("Could not write the recovery copy: "), "{}", h.app.status);
    assert!(h.app.error.is_none(), "no modal");
    h.app.status = "Editing".into();
    h.app.autosave_recovery(start + Duration::from_secs(1));
    assert!(h.app.autosave_job.is_none(), "no retry before the interval");
    h.app.autosave_recovery(start + Duration::from_secs(31));
    h.app.finish_autosave(true);
    assert_eq!(h.app.status, "Editing", "logged once");
    std::fs::remove_file(h.data.join("recovery")).unwrap();
    h.app.autosave_recovery(start + Duration::from_secs(62));
    h.app.finish_autosave(true);
    assert_eq!(files(&h.data).len(), 1, "a later interval writes the copy");
}

#[test]
fn autosave_dropped_mid_open_still_releases_outgoing_pending_claims() {
    let mut h = held();
    h.hold.store(false, Ordering::Relaxed);
    let other = h.map.with_file_name("other.osu");
    std::fs::write(&other, FIXTURE).unwrap();
    h.app.document.as_mut().unwrap().pending.add("normal-hitnormal2.wav", b"RIFF1").unwrap();
    edit(&mut h.app, 63);
    h.app.autosave_recovery_now();
    assert_eq!(files(&h.data).len(), 1);

    h.app.request_open(other.clone());
    h.app.discard_and_open();
    assert!(h.app.is_busy());
    // Let the open worker compute outgoing_keep before Off drops the outgoing autosave.
    std::thread::sleep(Duration::from_millis(100));
    h.app.set_autosave_interval_secs(0);
    wait(&mut h.app);
    assert!(h.app.document().unwrap().path.ends_with("other.osu"));
    assert!(files(&h.data).is_empty());
    assert!(h.app.document.as_mut().unwrap().pending.add("normal-hitnormal2.wav", b"RIFF2").is_ok());
}
