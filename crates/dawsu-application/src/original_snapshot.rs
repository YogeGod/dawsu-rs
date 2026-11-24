//! Immutable first-open snapshots of a mapset's `.osu` files so the mapper
//! can revert the whole mapset to its original state after saving edits.

use crate::pending::mapset_key;
use dawsu_foundation::safe_write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn snapshot_dir(data_root: &Path, mapset_folder: &Path) -> PathBuf {
    let key_path = mapset_folder
        .canonicalize()
        .unwrap_or_else(|_| mapset_folder.to_path_buf());
    data_root
        .join("original-snapshots")
        .join(mapset_key(&key_path))
}

fn osu_entries(folder: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let entries = std::fs::read_dir(folder)
        .map_err(|e| format!("Reading {}: {e}", folder.display()))?;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("osu"))
        {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                files.push((name.to_owned(), path));
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Maximum `.osu` files in a single folder to auto-snapshot (prevents copying thousands of
/// unrelated beatmaps if a user opens a loose `.osu` inside a giant folder).
const MAX_MAPSET_OSU_FILES: usize = 64;

/// Captures an immutable first-open snapshot of every `.osu` file in `mapset_folder`
/// under `data_root/original-snapshots/<key>/`. If a snapshot already exists with
/// `.osu` files, it is left untouched and returns `Ok(false)`.
pub(crate) fn capture_once(data_root: &Path, mapset_folder: &Path) -> Result<bool, String> {
    if has_snapshot(data_root, mapset_folder) {
        return Ok(false);
    }
    let files = osu_entries(mapset_folder)?;
    if files.is_empty() || files.len() > MAX_MAPSET_OSU_FILES {
        return Ok(false);
    }
    let dir = snapshot_dir(data_root, mapset_folder);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Creating snapshot folder: {e}"))?;
    for (name, path) in files {
        let bytes =
            std::fs::read(&path).map_err(|e| format!("Reading {name} for snapshot: {e}"))?;
        let target = dir.join(&name);
        if !target.exists() {
            safe_write::write_new(&target, &bytes)
                .map_err(|e| format!("Saving original snapshot for {name}: {e}"))?;
        }
    }
    Ok(true)
}

/// Returns true if an original snapshot with at least one `.osu` file exists for `mapset_folder`.
pub(crate) fn has_snapshot(data_root: &Path, mapset_folder: &Path) -> bool {
    let dir = snapshot_dir(data_root, mapset_folder);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("osu"))
    })
}

/// Backs up current `.osu` files to `data_root/backups/pre-revert-<unix_ms>/` and
/// restores the original snapshot bytes into `mapset_folder`. Returns the number of
/// restored `.osu` files.
pub(crate) fn restore(data_root: &Path, mapset_folder: &Path) -> Result<usize, String> {
    let dir = snapshot_dir(data_root, mapset_folder);
    let originals = osu_entries(&dir)?;
    if originals.is_empty() {
        return Err("No original snapshot is saved for this mapset.".into());
    }

    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let backup_dir = data_root
        .join("backups")
        .join(format!("pre-revert-{unix_ms}"));
    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| format!("Creating pre-revert backup folder: {e}"))?;

    let mut backups: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    for (name, current_path) in osu_entries(mapset_folder)? {
        let bytes = std::fs::read(&current_path)
            .map_err(|e| format!("Backing up {name} before revert: {e}"))?;
        safe_write::write_new(&backup_dir.join(&name), &bytes)
            .map_err(|e| format!("Writing pre-revert backup for {name}: {e}"))?;
    }

    let mut restored = 0usize;
    for (name, snap_path) in originals {
        let orig_bytes = std::fs::read(&snap_path)
            .map_err(|e| format!("Reading original snapshot {name}: {e}"))?;
        let target = mapset_folder.join(&name);
        let prev = std::fs::read(&target).ok();
        let write_res = safe_write::write_new(&target, &orig_bytes)
            .map_err(|e| format!("Restoring {name}: {e}"));
        if let Err(err) = write_res {
            for (prev_path, prev_bytes) in backups.into_iter().rev() {
                match prev_bytes {
                    Some(b) => {
                        let _ = safe_write::write_new(&prev_path, &b);
                    }
                    None => {
                        let _ = std::fs::remove_file(&prev_path);
                    }
                }
            }
            return Err(err);
        }
        backups.push((target, prev));
        restored += 1;
    }

    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_once_is_immutable_and_restore_creates_pre_revert_backup() {
        let root = std::env::temp_dir().join(format!(
            "dawsu-orig-snap-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let data_root = root.join("data");
        let mapset = root.join("mapset");
        std::fs::create_dir_all(&mapset).unwrap();

        let osu_a = mapset.join("DiffA.osu");
        let orig_bytes = b"osu file format v14\n[HitObjects]\n100,100,1000,1,0,0:0:0:0:\n";
        std::fs::write(&osu_a, orig_bytes).unwrap();

        assert!(!has_snapshot(&data_root, &mapset));
        assert!(capture_once(&data_root, &mapset).unwrap());
        assert!(has_snapshot(&data_root, &mapset));

        // Modify the .osu file on disk; second capture_once must NOT overwrite the snapshot.
        let edited_bytes = b"osu file format v14\n[HitObjects]\n200,200,2000,1,2,0:0:0:80:\n";
        std::fs::write(&osu_a, edited_bytes).unwrap();
        assert!(!capture_once(&data_root, &mapset).unwrap());

        // Restore must back up edited_bytes to data/backups/pre-revert-* and restore orig_bytes.
        let count = restore(&data_root, &mapset).unwrap();
        assert_eq!(count, 1);
        assert_eq!(std::fs::read(&osu_a).unwrap(), orig_bytes);

        let backups_root = data_root.join("backups");
        let pre_revert_dirs: Vec<_> = std::fs::read_dir(&backups_root)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("pre-revert-")
            })
            .collect();
        assert_eq!(pre_revert_dirs.len(), 1);
        let backed_up = pre_revert_dirs[0].path().join("DiffA.osu");
        assert_eq!(std::fs::read(&backed_up).unwrap(), edited_bytes);

        let _ = std::fs::remove_dir_all(&root);
    }
}
