//! Whole-crate tests for safe writes (foundation Responsibilities 1-3a, invariants 3-6).
//! Every test works in its own temporary folder.

use dawsu_foundation::safe_write::{self, BackupPolicy, WriteOutcome};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A fresh folder for one test, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("dawsu-foundation-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|it| it.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

fn all_backups(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(folder) {
        for e in entries {
            let p = e.unwrap().path();
            if p.is_dir() {
                for f in fs::read_dir(&p).unwrap() {
                    out.push(f.unwrap().path());
                }
            }
        }
    }
    out.sort();
    out
}

#[test]
fn write_new_creates_the_file_and_leaves_no_temporary_file() {
    let s = Scratch::new("write-new");
    let target = s.path("new.osu");
    safe_write::write_new(&target, b"hello\r\n").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"hello\r\n");
    assert_eq!(files_in(&s.0), vec!["new.osu"]);
}

#[test]
fn replace_writes_the_new_bytes_and_keeps_a_backup_of_the_old_ones() {
    let s = Scratch::new("replace");
    let target = s.path("map.osu");
    fs::write(&target, b"old").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 20);

    let outcome = safe_write::replace(&target, b"old", b"new", &policy).unwrap();

    assert_eq!(outcome, WriteOutcome::Saved { warnings: vec![] });
    assert_eq!(fs::read(&target).unwrap(), b"new");
    let backups = all_backups(&s.path("backups"));
    assert_eq!(backups.len(), 1);
    assert_eq!(fs::read(&backups[0]).unwrap(), b"old");
    assert_eq!(files_in(&s.0), vec!["backups", "map.osu"], "no temporary file is left");
}

#[test]
fn replace_refuses_when_the_target_changed_on_disk_and_writes_nothing() {
    let s = Scratch::new("changed");
    let target = s.path("map.osu");
    fs::write(&target, b"changed by osu!").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 20);

    let outcome = safe_write::replace(&target, b"what we read", b"new", &policy).unwrap();

    assert_eq!(outcome, WriteOutcome::ChangedOnDisk);
    assert_eq!(fs::read(&target).unwrap(), b"changed by osu!");
    assert!(all_backups(&s.path("backups")).is_empty(), "no backup is kept for a refused write");
    assert_eq!(files_in(&s.0), vec!["map.osu"], "no temporary file is left");
}

#[test]
fn a_failed_backup_writes_nothing_and_returns_an_error() {
    let s = Scratch::new("backup-fails");
    let target = s.path("map.osu");
    fs::write(&target, b"old").unwrap();
    // The backup folder cannot be created: a file sits where it should be.
    fs::write(s.path("backups"), b"not a folder").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 20);

    let result = safe_write::replace(&target, b"old", b"new", &policy);

    assert!(result.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"old", "the target is unchanged");
    assert_eq!(files_in(&s.0), vec!["backups", "map.osu"], "no temporary file is left");
}

#[test]
fn a_new_backup_never_replaces_an_existing_one() {
    let s = Scratch::new("no-overwrite");
    let target = s.path("map.osu");
    fs::write(&target, b"v1").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 20);

    safe_write::replace(&target, b"v1", b"v2", &policy).unwrap();
    safe_write::replace(&target, b"v2", b"v3", &policy).unwrap();
    safe_write::replace(&target, b"v3", b"v4", &policy).unwrap();

    let contents: Vec<Vec<u8>> = all_backups(&s.path("backups")).iter().map(|p| fs::read(p).unwrap()).collect();
    assert_eq!(contents.len(), 3);
    for expected in [b"v1".to_vec(), b"v2".to_vec(), b"v3".to_vec()] {
        assert!(contents.contains(&expected), "backup {expected:?} kept");
    }
}

#[test]
fn pruning_keeps_the_newest_backups_and_deletes_only_backup_files() {
    let s = Scratch::new("prune");
    let target = s.path("map.osu");
    fs::write(&target, b"v0").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 2);

    for i in 0..4 {
        let old = format!("v{i}");
        let new = format!("v{}", i + 1);
        safe_write::replace(&target, old.as_bytes(), new.as_bytes(), &policy).unwrap();
        if i == 0 {
            // A file that is not a backup, put next to the backups after the first save.
            let folder = all_backups(&s.path("backups"))[0].parent().unwrap().to_path_buf();
            fs::write(folder.join("notes.txt"), b"keep me").unwrap();
        }
    }

    let backups = all_backups(&s.path("backups"));
    let names: Vec<String> = backups.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    assert!(names.contains(&"notes.txt".to_string()), "a file that is not a backup is never deleted");
    let kept: Vec<Vec<u8>> = backups
        .iter()
        .filter(|p| !p.ends_with("notes.txt"))
        .map(|p| fs::read(p).unwrap())
        .collect();
    assert_eq!(kept.len(), 2);
    assert!(kept.contains(&b"v2".to_vec()) && kept.contains(&b"v3".to_vec()), "the newest two are kept: {kept:?}");
}

#[test]
fn files_with_the_same_name_in_different_folders_never_share_backups() {
    let s = Scratch::new("per-folder");
    fs::create_dir_all(s.path("a")).unwrap();
    fs::create_dir_all(s.path("b")).unwrap();
    let a = s.path("a/Normal.osu");
    let b = s.path("b/Normal.osu");
    fs::write(&a, b"a0").unwrap();
    fs::write(&b, b"b0").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 1);

    safe_write::replace(&a, b"a0", b"a1", &policy).unwrap();
    for i in 0..3 {
        let old = format!("b{i}");
        let new = format!("b{}", i + 1);
        safe_write::replace(&b, old.as_bytes(), new.as_bytes(), &policy).unwrap();
    }

    let contents: Vec<Vec<u8>> = all_backups(&s.path("backups")).iter().map(|p| fs::read(p).unwrap()).collect();
    assert!(contents.contains(&b"a0".to_vec()), "pruning b's backups left a's only backup: {contents:?}");
    assert!(contents.contains(&b"b2".to_vec()));
    assert_eq!(contents.len(), 2);
}

#[test]
fn a_failed_prune_still_counts_as_saved_with_a_warning() {
    let s = Scratch::new("prune-fails");
    let target = s.path("map.osu");
    fs::write(&target, b"v0").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 1);
    safe_write::replace(&target, b"v0", b"v1", &policy).unwrap();

    // Make the oldest backup impossible to delete: replace it with a non-empty folder of the same name.
    let old_backup = all_backups(&s.path("backups"))[0].clone();
    fs::remove_file(&old_backup).unwrap();
    fs::create_dir_all(old_backup.join("inner")).unwrap();

    let outcome = safe_write::replace(&target, b"v1", b"v2", &policy).unwrap();

    match outcome {
        WriteOutcome::Saved { warnings } => assert_eq!(warnings.len(), 1, "one warning for the failed prune"),
        other => panic!("expected Saved, got {other:?}"),
    }
    assert_eq!(fs::read(&target).unwrap(), b"v2");
}

/// Opens `path` for reading the way a virus scanner, the search indexer or a cloud-sync client
/// can: other programs may read and write it, but not rename or delete it.
#[cfg(windows)]
fn hold_without_delete_sharing(path: &Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).open(path).unwrap()
}

#[cfg(windows)]
#[test]
fn replace_waits_for_another_program_that_briefly_holds_the_target() {
    let s = Scratch::new("held-briefly");
    let target = s.path("map.osu");
    fs::write(&target, b"old").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 20);

    let handle = hold_without_delete_sharing(&target);
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(handle);
    });

    let outcome = safe_write::replace(&target, b"old", b"new", &policy);
    release.join().unwrap();

    assert_eq!(outcome.unwrap(), WriteOutcome::Saved { warnings: vec![] });
    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert_eq!(files_in(&s.0), vec!["backups", "map.osu"], "no temporary file is left");
}

#[cfg(windows)]
#[test]
fn replace_gives_up_cleanly_when_the_target_stays_held() {
    let s = Scratch::new("held-long");
    let target = s.path("map.osu");
    fs::write(&target, b"old").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 20);

    let handle = hold_without_delete_sharing(&target);
    let started = std::time::Instant::now();
    let outcome = safe_write::replace(&target, b"old", b"new", &policy);
    let waited = started.elapsed();
    let names = files_in(&s.0);
    drop(handle);

    assert!(outcome.is_err(), "expected an error, got {outcome:?}");
    assert!(waited >= std::time::Duration::from_millis(2900), "waits about 3 s before giving up, waited {waited:?}");
    assert_eq!(fs::read(&target).unwrap(), b"old");
    assert_eq!(names, vec!["backups", "map.osu"], "no temporary file is left");
}

#[cfg(windows)]
#[test]
fn replace_never_overwrites_a_change_made_while_it_waited() {
    let s = Scratch::new("held-and-changed");
    let target = s.path("map.osu");
    fs::write(&target, b"old").unwrap();
    let policy = BackupPolicy::new(s.path("backups"), 20);

    let handle = hold_without_delete_sharing(&target);
    let changed = target.clone();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(&changed, b"changed by osu!").unwrap();
        // Keep holding a while longer, so every replace attempt after the change re-checks first.
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(handle);
    });

    let outcome = safe_write::replace(&target, b"old", b"new", &policy);
    release.join().unwrap();

    assert_eq!(outcome.unwrap(), WriteOutcome::ChangedOnDisk);
    assert_eq!(fs::read(&target).unwrap(), b"changed by osu!");
    assert_eq!(files_in(&s.0), vec!["backups", "map.osu"], "no temporary file is left");
}

#[cfg(windows)]
#[test]
fn write_new_waits_for_another_program_that_briefly_holds_the_target() {
    let s = Scratch::new("write-new-held");
    let target = s.path("preferences.json");
    fs::write(&target, b"old").unwrap();

    let handle = hold_without_delete_sharing(&target);
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(handle);
    });

    let outcome = safe_write::write_new(&target, b"new");
    release.join().unwrap();

    outcome.unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"new");
    assert_eq!(files_in(&s.0), vec!["preferences.json"], "no temporary file is left");
}
