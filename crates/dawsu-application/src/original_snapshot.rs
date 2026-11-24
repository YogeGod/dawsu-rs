//! Immutable first-open snapshots of a mapset's `.osu` files so the mapper
//! can revert the whole mapset to its original state after saving edits.

use crate::pending::mapset_key;
use dawsu_foundation::safe_write;
use dawsu_osu_format::OsuFile;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const META_FILE: &str = "snapshot-meta.json";
const PRE_REVERT_META_FILE: &str = "pre-revert-meta.json";
const PRE_REVERT_PREFIX: &str = "pre-revert-";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct BeatmapIds {
    beatmap_id: Option<String>,
    beatmap_set_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SnapshotMeta {
    created_unix_ms: u64,
    snapshot_date: String,
    files: BTreeMap<String, BeatmapIds>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PreRevertMeta {
    mapset_key: String,
    created_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotPreview {
    pub snapshot_date: String,
    pub existing_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub can_undo_revert: bool,
}

fn canonical_mapset_key(mapset_folder: &Path) -> String {
    let key_path = mapset_folder
        .canonicalize()
        .unwrap_or_else(|_| mapset_folder.to_path_buf());
    mapset_key(&key_path)
}

fn snapshot_dir(data_root: &Path, mapset_folder: &Path) -> PathBuf {
    data_root
        .join("original-snapshots")
        .join(canonical_mapset_key(mapset_folder))
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn format_unix_date(unix_ms: u64) -> String {
    let days = (unix_ms / 86_400_000) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn read_beatmap_ids(bytes: &[u8]) -> BeatmapIds {
    let read = OsuFile::read(bytes.to_vec());
    let clean = |key: &str| {
        read.file
            .value("Metadata", key)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    BeatmapIds {
        beatmap_id: clean("BeatmapID"),
        beatmap_set_id: clean("BeatmapSetID"),
    }
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

fn find_matching_name<'a>(names: impl IntoIterator<Item = &'a str>, target: &str) -> Option<&'a str> {
    let mut ci_match = None;
    for candidate in names {
        if candidate == target {
            return Some(candidate);
        }
        if ci_match.is_none() && candidate.eq_ignore_ascii_case(target) {
            ci_match = Some(candidate);
        }
    }
    ci_match
}

fn find_meta_entry<'a>(files: &'a BTreeMap<String, BeatmapIds>, name: &str) -> Option<&'a BeatmapIds> {
    if let Some(exact) = files.get(name) {
        return Some(exact);
    }
    files
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

fn meta_from_osu_entries(entries: &[(String, PathBuf)], unix_ms: u64) -> Result<SnapshotMeta, String> {
    let mut files = BTreeMap::new();
    for (name, path) in entries {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("Reading snapshot {name}: {e}"))?;
        files.insert(name.clone(), read_beatmap_ids(&bytes));
    }
    Ok(SnapshotMeta {
        created_unix_ms: unix_ms,
        snapshot_date: format_unix_date(unix_ms),
        files,
    })
}

fn read_or_synthesize_meta(dir: &Path, originals: &[(String, PathBuf)]) -> Result<SnapshotMeta, String> {
    let meta_path = dir.join(META_FILE);
    if let Ok(bytes) = std::fs::read(&meta_path)
        && let Ok(meta) = serde_json::from_slice::<SnapshotMeta>(&bytes)
    {
        return Ok(meta);
    }
    let fallback_ms = originals
        .iter()
        .filter_map(|(_, path)| std::fs::metadata(path).ok()?.modified().ok())
        .filter_map(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .min()
        .unwrap_or_else(|| now_unix_ms() as u64);
    meta_from_osu_entries(originals, fallback_ms)
}

fn verify_mapset_identity(
    meta: &SnapshotMeta,
    current_files: &[(String, Vec<u8>)],
) -> Result<(), String> {
    let mismatch = || {
        format!(
            "This mapset looks like a different download than the one dawsu! saved on {}.",
            meta.snapshot_date
        )
    };
    let snap_set_ids: BTreeSet<&str> = meta
        .files
        .values()
        .filter_map(|ids| ids.beatmap_set_id.as_deref())
        .collect();
    for (cur_name, cur_bytes) in current_files {
        let cur_ids = read_beatmap_ids(cur_bytes);
        let snap_entry = find_meta_entry(&meta.files, cur_name);
        if let Some(cur_set_id) = cur_ids.beatmap_set_id.as_deref() {
            let matches = match snap_entry {
                Some(snap) => snap.beatmap_set_id.as_deref() == Some(cur_set_id),
                None => snap_set_ids.contains(cur_set_id),
            };
            if !matches {
                return Err(mismatch());
            }
        }
        if let Some(cur_id) = cur_ids.beatmap_id.as_deref()
            && let Some(snap) = snap_entry
            && snap.beatmap_id.as_deref() != Some(cur_id)
        {
            return Err(mismatch());
        }
    }
    Ok(())
}

/// Maximum `.osu` files in a single folder to auto-snapshot (prevents copying thousands of
/// unrelated beatmaps if a user opens a loose `.osu` inside a giant folder).
const MAX_MAPSET_OSU_FILES: usize = 64;

/// Captures an immutable first-open snapshot of every `.osu` file in `mapset_folder`
/// under `data_root/original-snapshots/<key>/` along with `snapshot-meta.json`.
/// If a snapshot already exists with `.osu` files, it is left untouched and returns `Ok(false)`.
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
    let mut meta_files = BTreeMap::new();
    for (name, path) in files {
        let bytes =
            std::fs::read(&path).map_err(|e| format!("Reading {name} for snapshot: {e}"))?;
        meta_files.insert(name.clone(), read_beatmap_ids(&bytes));
        let target = dir.join(&name);
        if !target.exists() {
            safe_write::write_new(&target, &bytes)
                .map_err(|e| format!("Saving original snapshot for {name}: {e}"))?;
        }
    }
    let meta_path = dir.join(META_FILE);
    if !meta_path.exists() {
        let created_unix_ms = now_unix_ms() as u64;
        let meta = SnapshotMeta {
            created_unix_ms,
            snapshot_date: format_unix_date(created_unix_ms),
            files: meta_files,
        };
        let meta_bytes = serde_json::to_vec_pretty(&meta)
            .map_err(|e| format!("Serializing snapshot metadata: {e}"))?;
        safe_write::write_new(&meta_path, &meta_bytes)
            .map_err(|e| format!("Saving snapshot metadata: {e}"))?;
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

fn find_pre_revert_backups(data_root: &Path, mapset_folder: &Path) -> Vec<(u128, PathBuf)> {
    let key = canonical_mapset_key(mapset_folder);
    let backups_root = data_root.join("backups");
    let Ok(entries) = std::fs::read_dir(&backups_root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        let Some(suffix) = name_str.strip_prefix(PRE_REVERT_PREFIX) else {
            continue;
        };
        let Ok(ts) = suffix.parse::<u128>() else {
            continue;
        };
        let Ok(meta_bytes) = std::fs::read(path.join(PRE_REVERT_META_FILE)) else {
            continue;
        };
        let Ok(meta) = serde_json::from_slice::<PreRevertMeta>(&meta_bytes) else {
            continue;
        };
        if meta.mapset_key != key {
            continue;
        }
        if osu_entries(&path).is_ok_and(|files| !files.is_empty()) {
            found.push((ts, path));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    found
}

/// Returns true if at least one `pre-revert-*` backup exists for `mapset_folder`.
pub(crate) fn has_pre_revert_backup(data_root: &Path, mapset_folder: &Path) -> bool {
    !find_pre_revert_backups(data_root, mapset_folder).is_empty()
}

/// Inspects the original snapshot against the current mapset folder without modifying anything.
pub(crate) fn preview(data_root: &Path, mapset_folder: &Path) -> Result<SnapshotPreview, String> {
    let dir = snapshot_dir(data_root, mapset_folder);
    let originals = osu_entries(&dir)?;
    if originals.is_empty() {
        return Err("No original snapshot is saved for this mapset.".into());
    }
    let meta = read_or_synthesize_meta(&dir, &originals)?;
    let current = osu_entries(mapset_folder)?;
    let current_names: Vec<&str> = current.iter().map(|(n, _)| n.as_str()).collect();
    let mut existing_files = Vec::new();
    let mut deleted_files = Vec::new();
    for (name, _) in originals {
        if find_matching_name(current_names.iter().copied(), &name).is_some() {
            existing_files.push(name);
        } else {
            deleted_files.push(name);
        }
    }
    Ok(SnapshotPreview {
        snapshot_date: meta.snapshot_date,
        existing_files,
        deleted_files,
        can_undo_revert: has_pre_revert_backup(data_root, mapset_folder),
    })
}

fn create_unique_backup_dir(backups_root: &Path, prefix: &str) -> Result<(u128, PathBuf), String> {
    std::fs::create_dir_all(backups_root)
        .map_err(|e| format!("Creating backup root folder: {e}"))?;
    let mut ts = now_unix_ms();
    loop {
        let dir = backups_root.join(format!("{prefix}{ts}"));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok((ts, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ts += 1,
            Err(e) => return Err(format!("Creating backup folder: {e}")),
        }
    }
}

fn restore_from_dir(
    data_root: &Path,
    mapset_folder: &Path,
    source_dir: &Path,
    source_meta: &SnapshotMeta,
    include_deleted: &[String],
    backup_prefix: &str,
    write_backup_meta: bool,
) -> Result<usize, String> {
    let sources = osu_entries(source_dir)?;
    if sources.is_empty() {
        return Err("No snapshot files are available to restore.".into());
    }

    let current_entries = osu_entries(mapset_folder)?;
    let mut current_files = Vec::with_capacity(current_entries.len());
    for (name, path) in &current_entries {
        let bytes =
            std::fs::read(path).map_err(|e| format!("Reading {name} before restore: {e}"))?;
        current_files.push((name.clone(), bytes));
    }

    verify_mapset_identity(source_meta, &current_files)?;

    let mut plan: Vec<(String, Vec<u8>, Option<Vec<u8>>)> = Vec::new();
    for (name, src_path) in sources {
        let existing = current_files
            .iter()
            .find(|(cur_name, _)| cur_name == &name)
            .or_else(|| {
                current_files
                    .iter()
                    .find(|(cur_name, _)| cur_name.eq_ignore_ascii_case(&name))
            });
        let (target_name, prev_bytes) = if let Some((cur_name, cur_bytes)) = existing {
            (cur_name.clone(), Some(cur_bytes.clone()))
        } else if include_deleted
            .iter()
            .any(|wanted| wanted.eq_ignore_ascii_case(&name))
        {
            (name.clone(), None)
        } else {
            continue;
        };
        let orig_bytes = std::fs::read(&src_path)
            .map_err(|e| format!("Reading snapshot {name}: {e}"))?;
        plan.push((target_name, orig_bytes, prev_bytes));
    }

    if plan.is_empty() {
        return Ok(0);
    }

    let (ts, backup_dir) = create_unique_backup_dir(&data_root.join("backups"), backup_prefix)?;
    for (name, bytes) in &current_files {
        safe_write::write_new(&backup_dir.join(name), bytes)
            .map_err(|e| format!("Writing backup for {name}: {e}"))?;
    }
    if write_backup_meta {
        let meta = PreRevertMeta {
            mapset_key: canonical_mapset_key(mapset_folder),
            created_unix_ms: ts as u64,
        };
        let meta_bytes = serde_json::to_vec_pretty(&meta)
            .map_err(|e| format!("Serializing pre-revert backup metadata: {e}"))?;
        safe_write::write_new(&backup_dir.join(PRE_REVERT_META_FILE), &meta_bytes)
            .map_err(|e| format!("Writing pre-revert backup metadata: {e}"))?;
    }

    let mut backups: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    let mut restored = 0usize;
    for (target_name, orig_bytes, prev) in plan {
        let target = mapset_folder.join(&target_name);
        let write_res = safe_write::write_new(&target, &orig_bytes)
            .map_err(|e| format!("Restoring {target_name}: {e}"));
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

/// Backs up current `.osu` files to `data_root/backups/pre-revert-<unix_ms>/` and
/// restores the original snapshot bytes into `mapset_folder` for existing `.osu` files
/// (plus any deleted `.osu` files explicitly listed in `include_deleted`).
pub(crate) fn restore_with_options(
    data_root: &Path,
    mapset_folder: &Path,
    include_deleted: &[String],
) -> Result<usize, String> {
    let dir = snapshot_dir(data_root, mapset_folder);
    let originals = osu_entries(&dir)?;
    if originals.is_empty() {
        return Err("No original snapshot is saved for this mapset.".into());
    }
    let meta = read_or_synthesize_meta(&dir, &originals)?;
    restore_from_dir(
        data_root,
        mapset_folder,
        &dir,
        &meta,
        include_deleted,
        PRE_REVERT_PREFIX,
        true,
    )
}

/// Convenience wrapper for `restore_with_options` that keeps deleted difficulties deleted.
#[cfg(test)]
pub(crate) fn restore(data_root: &Path, mapset_folder: &Path) -> Result<usize, String> {
    restore_with_options(data_root, mapset_folder, &[])
}

/// Restores existing `.osu` files from the newest `pre-revert-*` backup folder for
/// `mapset_folder` through the same safe restore path.
pub(crate) fn undo_revert(data_root: &Path, mapset_folder: &Path) -> Result<usize, String> {
    let Some((ts, backup_dir)) = find_pre_revert_backups(data_root, mapset_folder).pop() else {
        return Err("No pre-revert backup is available for this mapset.".into());
    };
    let originals = osu_entries(&backup_dir)?;
    let meta = meta_from_osu_entries(&originals, ts as u64)?;
    restore_from_dir(
        data_root,
        mapset_folder,
        &backup_dir,
        &meta,
        &[],
        "pre-undo-revert-",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Application, Preferences, pending};
    use dawsu_foundation::digest::sha256_hex;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    fn test_root(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dawsu-orig-snap-{tag}-{}", now_unix_ms()))
    }

    #[test]
    fn capture_once_is_immutable_and_restore_creates_pre_revert_backup() {
        let root = test_root("immutable");
        let data_root = root.join("data");
        let mapset = root.join("mapset");
        std::fs::create_dir_all(&mapset).unwrap();

        let osu_a = mapset.join("DiffA.osu");
        let orig_bytes = b"osu file format v14\n[HitObjects]\n100,100,1000,1,0,0:0:0:0:\n";
        std::fs::write(&osu_a, orig_bytes).unwrap();

        assert!(!has_snapshot(&data_root, &mapset));
        assert!(capture_once(&data_root, &mapset).unwrap());
        assert!(has_snapshot(&data_root, &mapset));
        assert!(snapshot_dir(&data_root, &mapset).join(META_FILE).exists());

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

    #[test]
    fn deleted_diff_stays_deleted_unless_explicitly_included() {
        let root = test_root("deleted-diff");
        let data_root = root.join("data");
        let mapset = root.join("mapset");
        std::fs::create_dir_all(&mapset).unwrap();

        let osu_a = mapset.join("Easy.osu");
        let osu_b = mapset.join("Hard.osu");
        let orig_a = b"osu file format v14\n[Metadata]\nBeatmapID:101\nBeatmapSetID:555\n[HitObjects]\n100,100,1000,1,0,0:0:0:0:\n";
        let orig_b = b"osu file format v14\n[Metadata]\nBeatmapID:102\nBeatmapSetID:555\n[HitObjects]\n200,200,2000,1,0,0:0:0:0:\n";
        std::fs::write(&osu_a, orig_a).unwrap();
        std::fs::write(&osu_b, orig_b).unwrap();

        assert!(capture_once(&data_root, &mapset).unwrap());

        let edited_a = b"osu file format v14\n[Metadata]\nBeatmapID:101\nBeatmapSetID:555\n[HitObjects]\n100,100,1000,1,2,0:0:0:90:\n";
        std::fs::write(&osu_a, edited_a).unwrap();
        std::fs::remove_file(&osu_b).unwrap();

        let info = preview(&data_root, &mapset).unwrap();
        assert_eq!(info.existing_files, vec!["Easy.osu"]);
        assert_eq!(info.deleted_files, vec!["Hard.osu"]);
        assert!(!info.can_undo_revert);

        // Default revert restores Easy.osu and leaves Hard.osu deleted.
        let count = restore(&data_root, &mapset).unwrap();
        assert_eq!(count, 1);
        assert_eq!(std::fs::read(&osu_a).unwrap(), orig_a);
        assert!(!osu_b.exists(), "deleted difficulty must stay deleted by default");

        // Opting in brings Hard.osu back as well.
        std::fs::write(&osu_a, edited_a).unwrap();
        let count = restore_with_options(&data_root, &mapset, &["Hard.osu".to_owned()]).unwrap();
        assert_eq!(count, 2);
        assert_eq!(std::fs::read(&osu_a).unwrap(), orig_a);
        assert_eq!(std::fs::read(&osu_b).unwrap(), orig_b);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn different_beatmap_set_id_refuses_revert_and_writes_nothing() {
        let root = test_root("mismatch-set-id");
        let data_root = root.join("data");
        let mapset = root.join("mapset");
        std::fs::create_dir_all(&mapset).unwrap();

        let osu_a = mapset.join("DiffA.osu");
        let orig = b"osu file format v14\n[Metadata]\nTitle:Song\nBeatmapID:1001\nBeatmapSetID:4200\n[HitObjects]\n100,100,1000,1,0,0:0:0:0:\n";
        std::fs::write(&osu_a, orig).unwrap();
        assert!(capture_once(&data_root, &mapset).unwrap());

        let date = preview(&data_root, &mapset).unwrap().snapshot_date;
        let other = b"osu file format v14\n[Metadata]\nTitle:Song\nBeatmapID:1001\nBeatmapSetID:9999\n[HitObjects]\n200,200,2000,1,8,0:0:0:0:\n";
        std::fs::write(&osu_a, other).unwrap();

        let err = restore(&data_root, &mapset).unwrap_err();
        assert_eq!(
            err,
            format!("This mapset looks like a different download than the one dawsu! saved on {date}.")
        );
        assert_eq!(std::fs::read(&osu_a).unwrap(), other);
        assert!(!data_root.join("backups").exists(), "refused revert must create no backup");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn undo_revert_restores_from_newest_pre_revert_backup() {
        let root = test_root("undo-revert");
        let data_root = root.join("data");
        let mapset = root.join("mapset");
        let other_mapset = root.join("other-mapset");
        std::fs::create_dir_all(&mapset).unwrap();
        std::fs::create_dir_all(&other_mapset).unwrap();

        let osu_a = mapset.join("DiffA.osu");
        let orig = b"osu file format v14\n[Metadata]\nBeatmapID:10\nBeatmapSetID:77\n[HitObjects]\n100,100,1000,1,0,0:0:0:0:\n";
        let edit_1 = b"osu file format v14\n[Metadata]\nBeatmapID:10\nBeatmapSetID:77\n[HitObjects]\n100,100,1000,1,2,0:0:0:50:\n";
        let edit_2 = b"osu file format v14\n[Metadata]\nBeatmapID:10\nBeatmapSetID:77\n[HitObjects]\n100,100,1000,1,8,0:0:0:95:\n";
        std::fs::write(&osu_a, orig).unwrap();
        assert!(capture_once(&data_root, &mapset).unwrap());
        assert!(!has_pre_revert_backup(&data_root, &mapset));

        std::fs::write(&osu_a, edit_1).unwrap();
        assert_eq!(restore(&data_root, &mapset).unwrap(), 1);
        assert!(has_pre_revert_backup(&data_root, &mapset));
        assert!(!has_pre_revert_backup(&data_root, &other_mapset));

        std::fs::write(&osu_a, edit_2).unwrap();
        assert_eq!(restore(&data_root, &mapset).unwrap(), 1);
        assert_eq!(std::fs::read(&osu_a).unwrap(), orig);

        let restored = undo_revert(&data_root, &mapset).unwrap();
        assert_eq!(restored, 1);
        assert_eq!(std::fs::read(&osu_a).unwrap(), edit_2);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ledgered_mix_file_is_swept_after_revert() {
        let scratch = Scratch::new("revert-sweep-mix");
        let data_root = scratch.path("data");
        let folder = scratch.path("mapset");
        std::fs::create_dir_all(&folder).unwrap();
        let map = folder.join("DiffA.osu");
        let orig = "osu file format v14\r\n\r\n[General]\r\nMode: 0\r\n\r\n[Metadata]\r\nTitle:Sweep\r\nBeatmapID:50\r\nBeatmapSetID:60\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";
        std::fs::write(&map, orig).unwrap();

        let mut app = Application::create(Preferences::default(), false, None, None, false, data_root.clone());
        app.request_open(map.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy || !app.has_original_snapshot() {
            app.poll();
            assert!(Instant::now() < deadline, "timed out waiting for open & snapshot");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);

        let mix_bytes = b"RIFF\x24\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\0\0\0\0";
        let mix_sha = sha256_hex(mix_bytes);
        let mix_name = format!("dawsu-mix-{mix_sha}-0.wav");
        std::fs::write(folder.join(&mix_name), mix_bytes).unwrap();
        let ledger = app.document().unwrap().pending.ledger().unwrap();
        ledger
            .add(&[pending::PendingSample {
                name: mix_name.clone(),
                sha256: mix_sha,
            }])
            .unwrap();
        let edited = format!(
            "osu file format v14\r\n\r\n[General]\r\nMode: 0\r\n\r\n[Metadata]\r\nTitle:Sweep\r\nBeatmapID:50\r\nBeatmapSetID:60\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:{mix_name}\r\n"
        );
        std::fs::write(&map, edited).unwrap();

        assert_eq!(app.revert_mapset_to_original(&[]).unwrap(), 1);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline, "timed out waiting for revert reopen & sweep");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!folder.join(&mix_name).exists(), "ledgered mix file must be swept after revert");
        assert!(ledger.entries().unwrap().is_empty(), "ledger entry should be removed");
    }
}
