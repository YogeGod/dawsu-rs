//! `.osz` import and export (osu format design doc Responsibilities 10 and 13, invariants 10, 13
//! and 14; ADR 0011; the last-four-parts plan, part C), after the old app's `OszImport` and
//! `SafeFile.FreeFolder` at c81774e. Refusals are outcomes; only file-system failures are errors
//! (ADR 0034). Both calls block, take a stop check between entries and report entries done out of
//! the total (ADR 0035).

use crate::Error;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The import's limits (osu format invariant 10(b)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OszLimits {
    /// The unpacked size, summed over the entries.
    pub max_bytes: u64,
    pub max_entries: usize,
}

impl Default for OszLimits {
    /// 2 GB unpacked and 20,000 entries.
    fn default() -> OszLimits {
        OszLimits { max_bytes: 2 * 1024 * 1024 * 1024, max_entries: 20_000 }
    }
}

/// The outcome of an `.osz` import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OszImport {
    /// The new mapset folder in the Songs folder.
    Imported { folder: PathBuf },
    /// The file is not a zip archive.
    NotAnArchive(String),
    TooLarge,
    TooManyEntries,
    /// An entry's name could leave the folder or is not a plain file name on every system.
    UnsafeName(String),
    Cancelled,
}

/// The outcome of an `.osz` export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OszExport {
    Exported,
    /// The archive would be written inside the folder it packs.
    TargetInsideFolder,
    Cancelled,
}

/// A part of a path that is a plain file or folder name on Windows, macOS and Linux.
fn safe_part(part: &str) -> bool {
    if part.is_empty() || part == "." || part == ".." || part.ends_with('.') || part.ends_with(' ') {
        return false;
    }
    if part.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c)) {
        return false;
    }
    let stem = part.split('.').next().unwrap_or("").trim_end_matches(' ').to_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit());
    !device
}

/// An entry name as a relative path with `/`, or `None` when it is unsafe. Both slashes separate.
fn entry_path(name: &str) -> Option<String> {
    if name.starts_with(['/', '\\']) {
        return None;
    }
    let parts: Vec<&str> = name.split(['/', '\\']).collect();
    parts.iter().all(|p| safe_part(p)).then(|| parts.join("/"))
}

/// The new folder's name: the archive's name with forbidden characters made `_` and trailing dots
/// and spaces trimmed, or "Imported beatmap" when nothing usable is left.
fn folder_name(archive: &Path) -> String {
    let stem = archive.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let cleaned: String = stem.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']).to_string();
    if safe_part(&cleaned) { cleaned } else { "Imported beatmap".into() }
}

fn unique() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{nanos:x}-{seq}", std::process::id())
}

/// Imports an `.osz` archive into a new folder of `songs` (Responsibility 10, invariant 10).
/// Every entry is checked before anything is written; files are unpacked into a fresh staging
/// folder, each written new, never over a file; the staging folder is then renamed to the
/// archive's name, or `name (2)`, `name (3)` and so on, so an import never merges into an existing
/// folder. On any failure or stop only the staging folder is removed. Folder entries are skipped,
/// and symlink entries and names that differ only in case are refused.
pub fn import_osz(archive: &Path, songs: &Path, limits: OszLimits, cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize, usize)) -> Result<OszImport, Error> {
    let file = fs::File::open(archive).map_err(|e| Error::io("opening the archive", archive, e))?;
    let mut zip = match zip::ZipArchive::new(file) {
        Ok(z) => z,
        Err(zip::result::ZipError::Io(e)) => return Err(Error::io("reading the archive", archive, e)),
        Err(e) => return Ok(OszImport::NotAnArchive(e.to_string())),
    };
    if zip.len() > limits.max_entries {
        return Ok(OszImport::TooManyEntries);
    }
    // Check every entry first: names, kinds and the summed stated size.
    let (mut plan, mut seen, mut total) = (Vec::new(), HashSet::new(), 0u64);
    for i in 0..zip.len() {
        if cancel() {
            return Ok(OszImport::Cancelled);
        }
        let entry = match zip.by_index_raw(i) {
            Ok(e) => e,
            Err(e) => return Ok(OszImport::NotAnArchive(e.to_string())),
        };
        // A name the zip crate cannot decode is not one dawsu! can match to the maps' references.
        let name = match entry.name() {
            Ok(n) => n.to_string(),
            Err(_) => return Ok(OszImport::UnsafeName(String::from_utf8_lossy(entry.name_raw()).into_owned())),
        };
        if name.ends_with(['/', '\\']) || entry.is_dir() {
            continue;
        }
        if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
            return Ok(OszImport::UnsafeName(name));
        }
        let Some(rel) = entry_path(&name) else { return Ok(OszImport::UnsafeName(name)) };
        if !seen.insert(rel.to_lowercase()) {
            return Ok(OszImport::UnsafeName(name));
        }
        total = total.saturating_add(entry.size());
        if total > limits.max_bytes {
            return Ok(OszImport::TooLarge);
        }
        plan.push((i, rel));
    }

    let staging = songs.join(format!(".importing-{}", unique()));
    fs::create_dir(&staging).map_err(|e| Error::io("making the staging folder", &staging, e))?;
    let result = unpack(&mut zip, &plan, &staging, limits.max_bytes, cancel, progress);
    match result {
        Ok(Some(outcome)) => {
            let _ = fs::remove_dir_all(&staging);
            Ok(outcome)
        }
        Ok(None) => place(&staging, songs, &folder_name(archive)).inspect_err(|_| {
            let _ = fs::remove_dir_all(&staging);
        }),
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            Err(e)
        }
    }
}

/// Unpacks the checked entries; `Some` is an outcome that stops the import.
fn unpack(zip: &mut zip::ZipArchive<fs::File>, plan: &[(usize, String)], staging: &Path, max_bytes: u64, cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize, usize)) -> Result<Option<OszImport>, Error> {
    let mut written = 0u64;
    for (done, (i, rel)) in plan.iter().enumerate() {
        if cancel() {
            return Ok(Some(OszImport::Cancelled));
        }
        let mut entry = match zip.by_index(*i) {
            Ok(e) => e,
            Err(e) => return Ok(Some(OszImport::NotAnArchive(e.to_string()))),
        };
        let path = staging.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| Error::io("making a folder", parent, e))?;
        }
        let mut out = fs::File::create_new(&path).map_err(|e| Error::io("writing a file", &path, e))?;
        // The bytes written are counted too, whatever an entry's stated size says.
        let mut limited = (&mut entry).take(max_bytes - written + 1);
        let n = match io::copy(&mut limited, &mut out) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => return Ok(Some(OszImport::NotAnArchive(e.to_string()))),
            Err(e) => return Err(Error::io("unpacking a file", &path, e)),
        };
        written += n;
        if written > max_bytes {
            return Ok(Some(OszImport::TooLarge));
        }
        progress(done + 1, plan.len());
    }
    Ok(None)
}

/// Renames the staging folder to the first free name; when a name is taken between the check and
/// the rename, the next one is tried, up to 20 times.
fn place(staging: &Path, songs: &Path, name: &str) -> Result<OszImport, Error> {
    let mut n = 1;
    for _ in 0..20 {
        let folder = loop {
            let candidate = songs.join(if n == 1 { name.to_string() } else { format!("{name} ({n})") });
            n += 1;
            if !candidate.exists() {
                break candidate;
            }
        };
        if folder.exists() {
            continue;
        }
        match dawsu_foundation::safe_write::rename_waiting(staging, &folder) {
            Ok(()) => return Ok(OszImport::Imported { folder }),
            Err(_) if folder.exists() => continue,
            Err(e) => return Err(Error::Write(e)),
        }
    }
    Err(Error::io("moving the imported folder into place", songs, io::Error::new(io::ErrorKind::AlreadyExists, "no free folder name after 20 tries")))
}

/// Writes `folder` as an `.osz` archive at `target` (Responsibility 13, invariants 13 and 14): its
/// files at the root of the zip with subfolders as relative paths, in byte order of their `/`
/// paths, deflated, each with the time 1980-01-01, so two exports of one folder give the same
/// bytes. Symlinks and the import's own staging folders are skipped. The archive goes to a
/// temporary file beside the target and is then moved over it; a failure or a stop removes the
/// temporary file and leaves the target as it was.
pub fn export_osz(folder: &Path, target: &Path, cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize, usize)) -> Result<OszExport, Error> {
    let root = fs::canonicalize(folder).map_err(|e| Error::io("opening the mapset folder", folder, e))?;
    let target_dir = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let target_dir = fs::canonicalize(target_dir).map_err(|e| Error::io("opening the target folder", target_dir, e))?;
    if target_dir.starts_with(&root) {
        return Ok(OszExport::TargetInsideFolder);
    }
    let mut files = Vec::new();
    collect(&root, "", &mut files)?;
    files.sort();

    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "export.osz".into());
    let (temp, out) = {
        let mut created = None;
        for _ in 0..32 {
            let candidate = target_dir.join(format!(".{name}.partial-{}", unique()));
            match fs::File::create_new(&candidate) {
                Ok(file) => {
                    created = Some((candidate, file));
                    break;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::io("writing the archive", &candidate, e)),
            }
        }
        created.ok_or_else(|| Error::io("writing the archive", target, io::Error::new(io::ErrorKind::AlreadyExists, "could not allocate temporary archive name")))?
    };
    let result = write_zip(out, &root, &files, &temp, cancel, progress);
    let outcome = match result {
        Ok(true) => dawsu_foundation::safe_write::rename_waiting(&temp, target)
            .map(|()| OszExport::Exported)
            .map_err(Error::Write),
        Ok(false) => Ok(OszExport::Cancelled),
        Err(e) => Err(e),
    };
    if !matches!(outcome, Ok(OszExport::Exported)) {
        let _ = fs::remove_file(&temp);
    }
    outcome
}

/// Every plain file under `dir` as a relative `/` path; symlinks and `.importing-*` folders are
/// skipped.
fn collect(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), Error> {
    for entry in fs::read_dir(dir).map_err(|e| Error::io("listing the folder", dir, e))? {
        let entry = entry.map_err(|e| Error::io("listing the folder", dir, e))?;
        let kind = entry.file_type().map_err(|e| Error::io("listing the folder", entry.path(), e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = format!("{prefix}{name}");
        if kind.is_symlink() || (kind.is_dir() && name.starts_with(".importing-")) {
            continue;
        }
        if kind.is_dir() {
            collect(&entry.path(), &format!("{rel}/"), out)?;
        } else if kind.is_file() {
            out.push(rel);
        }
    }
    Ok(())
}

/// Writes the archive; `false` when stopped.
fn write_zip(out: fs::File, root: &Path, files: &[String], temp: &Path, cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(usize, usize)) -> Result<bool, Error> {
    let zip_err = |e: zip::result::ZipError| Error::io("writing the archive", temp, io::Error::other(e));
    let mut w = zip::ZipWriter::new(out);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (done, rel) in files.iter().enumerate() {
        if cancel() {
            return Ok(false);
        }
        w.start_file(rel.as_str(), options).map_err(zip_err)?;
        let path = root.join(rel);
        let mut f = fs::File::open(&path).map_err(|e| Error::io("reading a file", &path, e))?;
        io::copy(&mut f, &mut w).map_err(|e| Error::io("writing the archive", temp, e))?;
        progress(done + 1, files.len());
    }
    let mut file = w.finish().map_err(zip_err)?;
    file.flush().map_err(|e| Error::io("writing the archive", temp, e))?;
    file.sync_all().map_err(|e| Error::io("writing the archive", temp, e))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names_follow_the_old_checks() {
        assert_eq!(entry_path("sb\\bg.png").as_deref(), Some("sb/bg.png"));
        for bad in ["", "a/", "/a", "\\a", "a/../b", "./a", "con.wav", "LPT9", "com1.txt", "a.", "a ", "a?b", "a\u{1}b"] {
            assert_eq!(entry_path(bad), None, "{bad:?}");
        }
        assert!(entry_path("COMPUTER.wav").is_some() && entry_path("console.osu").is_some());
        assert_eq!(folder_name(Path::new("x/Hello..osz")), "Hello");
        assert_eq!(folder_name(Path::new("nul.osz")), "Imported beatmap");
        assert_eq!(folder_name(Path::new("a|b.osz")), "a_b");
    }
}
