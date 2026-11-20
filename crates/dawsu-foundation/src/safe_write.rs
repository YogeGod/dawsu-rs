//! Safe file writes (foundation Responsibilities 1 to 3a).
//!
//! A write goes to a temporary file next to the target and then replaces it, so a failed
//! write leaves the target as it was. Writing over an existing file first checks that the
//! file has not changed since the caller read it, and keeps a backup.

use crate::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where backups go and how many are kept for each file (the count is the caller's choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPolicy {
    folder: PathBuf,
    keep: usize,
}

impl BackupPolicy {
    pub fn new(folder: impl Into<PathBuf>, keep: usize) -> BackupPolicy {
        BackupPolicy { folder: folder.into(), keep }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn keep(&self) -> usize {
        self.keep
    }
}

/// A problem that did not stop the write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// An old backup could not be deleted; the write still counts.
    PruneFailed { path: PathBuf, reason: String },
}

/// The expected outcomes of writing over a file (ADR 0034: outcomes, not errors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    Saved { warnings: Vec<Warning> },
    /// The file changed since the caller read it; nothing was written.
    ChangedOnDisk,
}

/// Writes a file that is new or that only dawsu! writes: temporary file first, then rename.
pub fn write_new(target: &Path, bytes: &[u8]) -> Result<(), Error> {
    let temp = write_temporary(target, bytes)?;
    let mut waits = REPLACE_WAITS.iter();
    loop {
        match fs::rename(&temp, target) {
            Ok(()) => return Ok(()),
            Err(e) if wait_if_held(&e, &mut waits) => continue,
            Err(e) => {
                let _ = fs::remove_file(&temp);
                return Err(Error::io("replacing the target", target, e));
            }
        }
    }
}

/// Publishes a new file atomically without replacing an existing destination.
pub fn create(target: &Path, bytes: &[u8]) -> Result<(), Error> {
    let temp = write_temporary(target, bytes)?;
    let result = fs::hard_link(&temp, target);
    let _ = fs::remove_file(&temp);
    result.map_err(|e| Error::io("creating the file", target, e))
}

/// Writes `new_bytes` over `target`, which the caller read as `read_bytes`, in the order of
/// Responsibility 3a: check, backup, temporary file, check again, replace, prune.
pub fn replace(target: &Path, read_bytes: &[u8], new_bytes: &[u8], backups: &BackupPolicy) -> Result<WriteOutcome, Error> {
    // 1. The file must not have changed since the caller read it.
    if !unchanged(target, read_bytes)? {
        return Ok(WriteOutcome::ChangedOnDisk);
    }

    // 2. Keep the backup, never over an existing one. If this fails, nothing is written.
    let backup_dir = backup_folder_for(target, backups.folder());
    let name = file_name(target);
    keep_backup(&backup_dir, &name, read_bytes)?;

    // 3. Write the new file next to the target.
    let temp = write_temporary(target, new_bytes)?;

    let mut waits = REPLACE_WAITS.iter();
    loop {
        // 4. Check again just before replacing (invariant 4), and again before every retry.
        match unchanged(target, read_bytes) {
            Ok(true) => {}
            Ok(false) => {
                let _ = fs::remove_file(&temp);
                return Ok(WriteOutcome::ChangedOnDisk);
            }
            Err(e) => {
                let _ = fs::remove_file(&temp);
                return Err(e);
            }
        }

        // 5. Replace the target. Another program that briefly holds it open (a virus scanner,
        // the search indexer, a cloud-sync client) can make this fail; wait a little and retry.
        match fs::rename(&temp, target) {
            Ok(()) => break,
            Err(e) if wait_if_held(&e, &mut waits) => continue,
            Err(e) => {
                let _ = fs::remove_file(&temp);
                return Err(Error::io("replacing the target", target, e));
            }
        }
    }

    // 6. Delete old backups; a failure here is only a warning.
    let warnings = prune(&backup_dir, &name, backups.keep());
    Ok(WriteOutcome::Saved { warnings })
}

/// How long `replace` and `write_new` wait before each retry of a replace that another program blocked:
/// about 3 s in total. Only Windows keeps other programs from replacing an open file.
#[cfg(windows)]
const REPLACE_WAITS: [Duration; 12] = [
    Duration::from_millis(10),
    Duration::from_millis(20),
    Duration::from_millis(40),
    Duration::from_millis(80),
    Duration::from_millis(150),
    Duration::from_millis(200),
    Duration::from_millis(250),
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(500),
    Duration::from_millis(500),
    Duration::from_millis(500),
];
#[cfg(not(windows))]
const REPLACE_WAITS: [Duration; 0] = [];

/// Windows refuses to replace a file another program has open without delete sharing:
/// "Access is denied" (error 5) or a sharing violation (error 32).
fn held_by_another_program(e: &std::io::Error) -> bool {
    cfg!(windows) && (e.kind() == ErrorKind::PermissionDenied || e.raw_os_error() == Some(32))
}

/// When another program holds the target and the wait budget is not used up, waits once and
/// returns true so the caller retries.
fn wait_if_held(e: &std::io::Error, waits: &mut std::slice::Iter<'_, Duration>) -> bool {
    if !held_by_another_program(e) {
        return false;
    }
    match waits.next() {
        Some(wait) => {
            std::thread::sleep(*wait);
            true
        }
        None => false,
    }
}

fn unchanged(target: &Path, read_bytes: &[u8]) -> Result<bool, Error> {
    match fs::read(target) {
        Ok(current) => Ok(current == read_bytes),
        Err(e) => Err(Error::io("reading the target", target, e)),
    }
}

fn file_name(target: &Path) -> String {
    target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Backups of files in different folders never share a count (Responsibility 3): each folder
/// of a file gets its own subfolder, named after a stable hash of the folder's path.
fn backup_folder_for(target: &Path, root: &Path) -> PathBuf {
    let parent = target.parent().unwrap_or(Path::new(""));
    let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    root.join(format!("{:016x}", fnv1a(parent.to_string_lossy().as_bytes())))
}

/// FNV-1a, 64 bit: stable across Rust versions, unlike the standard library's hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Backup files are named `<file name>.<number>.bak`, numbered upwards.
fn backup_number(entry_name: &str, name: &str) -> Option<u64> {
    let rest = entry_name.strip_prefix(name)?.strip_prefix('.')?;
    let digits = rest.strip_suffix(".bak")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn existing_backups(dir: &Path, name: &str) -> Vec<(u64, PathBuf)> {
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(n) = backup_number(&entry.file_name().to_string_lossy(), name) {
                found.push((n, entry.path()));
            }
        }
    }
    found.sort();
    found
}

fn keep_backup(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), Error> {
    fs::create_dir_all(dir).map_err(|e| Error::io("creating the backup folder", dir, e))?;
    let mut next = existing_backups(dir, name).last().map_or(1, |(n, _)| n + 1);
    loop {
        let path = dir.join(format!("{name}.{next}.bak"));
        // create_new: a new backup never replaces an existing one (invariant 5).
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return write_and_sync(file, bytes).map_err(|e| {
                let _ = fs::remove_file(&path);
                Error::io("writing the backup", &path, e)
            }),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => next += 1,
            Err(e) => return Err(Error::io("creating the backup", &path, e)),
        }
    }
}

fn prune(dir: &Path, name: &str, keep: usize) -> Vec<Warning> {
    let backups = existing_backups(dir, name);
    let surplus = backups.len().saturating_sub(keep);
    let mut warnings = Vec::new();
    for (_, path) in backups.into_iter().take(surplus) {
        // Only files matching the backup pattern are ever deleted (invariant 6).
        if let Err(e) = fs::remove_file(&path) {
            warnings.push(Warning::PruneFailed { path, reason: e.to_string() });
        }
    }
    warnings
}

fn write_temporary(target: &Path, bytes: &[u8]) -> Result<PathBuf, Error> {
    let dir = target.parent().unwrap_or(Path::new("."));
    let name = file_name(target);
    let pid = std::process::id();
    for attempt in 0u32.. {
        let temp = dir.join(format!(".{name}.{pid}.{attempt}.tmp"));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => {
                return match write_and_sync(file, bytes) {
                    Ok(()) => Ok(temp),
                    Err(e) => {
                        let _ = fs::remove_file(&temp);
                        Err(Error::io("writing the temporary file", &temp, e))
                    }
                };
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::io("creating the temporary file", &temp, e)),
        }
    }
    unreachable!("the attempt counter is unbounded")
}

fn write_and_sync(mut file: File, bytes: &[u8]) -> std::io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names_match_only_their_own_pattern() {
        assert_eq!(backup_number("Normal.osu.3.bak", "Normal.osu"), Some(3));
        assert_eq!(backup_number("Normal.osu.bak", "Normal.osu"), None);
        assert_eq!(backup_number("Normal.osu.3x.bak", "Normal.osu"), None);
        assert_eq!(backup_number("Other.osu.3.bak", "Normal.osu"), None);
        assert_eq!(backup_number("notes.txt", "Normal.osu"), None);
    }

    #[test]
    fn fnv1a_is_stable() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
