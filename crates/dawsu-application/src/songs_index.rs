//! Fast Songs folder index: discovers osu!'s binary `osu!.db` (beside `Songs/` or via
//! `osu!.*.cfg` `BeatmapDirectory`), persists a compact `.sidx` cache under `data_root`,
//! reconciles newly added or locally modified folders, and falls back to a bounded parallel
//! header-only `.osu` scan when `osu!.db` is absent or unreadable.

use dawsu_foundation::{digest::sha256_hex, safe_write};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CACHE_MAGIC: &[u8; 8] = b"DWSIDX01";
const CACHE_VERSION: u32 = 1;
pub const SOURCE_OSU_DB: u8 = 1;
pub const SOURCE_FOLDER_SCAN: u8 = 2;

const MAX_LOCAL_NEWER_RESCAN: usize = 256;
const PARALLEL_THRESHOLD: usize = 16;
const MAX_SCAN_WORKERS: usize = 8;
const HEADER_MAX_LINES: usize = 512;
const HEADER_MAX_BYTES: usize = 65_536;
const MIN_OSU_DB_ENTRY_BYTES: usize = 50;
const MIN_SIDX_RECORD_BYTES: usize = 24;
const MIN_SIDX_DIFF_BYTES: usize = 5;

/// One `.osu` difficulty inside a mapset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongDiffEntry {
    pub version: String,
    pub path: String,
    pub mode: u8,
}

/// One mapset folder grouping all of its difficulties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongMapsetEntry {
    pub set_id: i32,
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub tags: String,
    pub folder: String,
    pub diffs: Vec<SongDiffEntry>,
}

/// Search response returned to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongsSearchResult {
    pub total_mapsets: usize,
    pub total_diffs: usize,
    pub source: &'static str,
    pub matches: Vec<SongMapsetEntry>,
}

/// Size and modification timestamp used to detect changes without hashing whole files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
}

impl FileStamp {
    pub fn mtime(&self) -> (i64, u32) {
        (self.mtime_secs, self.mtime_nanos)
    }
}

/// Identity and size/timestamp stamp for a paired `osu!.db` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsuDbFingerprint {
    pub path: PathBuf,
    pub stamp: FileStamp,
}

/// Cheaply verifiable filesystem stamp for an in-memory [`SongsIndex`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongsFingerprint {
    pub songs_dir: PathBuf,
    pub songs_dir_mtime: (i64, u32),
    pub osu_db: Option<OsuDbFingerprint>,
}

/// One indexed mapset paired with cache metadata and a pre-lowercased search haystack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedMapset {
    pub entry: SongMapsetEntry,
    pub folder_mtime: (i64, u32),
    pub from_osu_db: bool,
    pub unicode_terms: String,
    diff_filenames: Vec<String>,
    search_haystack: String,
}

impl IndexedMapset {
    fn from_record(record: FolderRecord, songs_dir: &Path) -> Self {
        let folder_path = songs_dir.join(&record.folder);
        let mut pairs: Vec<(String, String, u8)> = record
            .diffs
            .into_iter()
            .filter(|d| is_safe_osu_filename(&d.filename))
            .map(|d| {
                let version = if d.version.is_empty() {
                    Path::new(&d.filename)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| d.filename.clone())
                } else {
                    d.version
                };
                (version, d.filename, d.mode)
            })
            .collect();
        pairs.sort_by_cached_key(|(version, filename, _)| {
            (version.to_lowercase(), filename.to_lowercase())
        });

        let mut diffs = Vec::with_capacity(pairs.len());
        let mut diff_filenames = Vec::with_capacity(pairs.len());
        for (version, filename, mode) in pairs {
            diffs.push(SongDiffEntry {
                version,
                path: folder_path.join(&filename).to_string_lossy().into_owned(),
                mode,
            });
            diff_filenames.push(filename);
        }

        let entry = SongMapsetEntry {
            set_id: record.set_id,
            artist: record.artist,
            title: record.title,
            creator: record.creator,
            tags: record.tags,
            folder: record.folder,
            diffs,
        };
        let search_haystack = build_search_haystack(&entry, &record.unicode_terms);
        Self {
            entry,
            folder_mtime: record.folder_mtime,
            from_osu_db: record.from_osu_db,
            unicode_terms: record.unicode_terms,
            diff_filenames,
            search_haystack,
        }
    }

    fn matches_tokens(&self, tokens: &[String]) -> bool {
        tokens
            .iter()
            .all(|tok| self.search_haystack.contains(tok.as_str()))
    }

    fn to_record(&self) -> FolderRecord {
        let diffs = self
            .entry
            .diffs
            .iter()
            .zip(self.diff_filenames.iter())
            .map(|(diff, filename)| CachedDiff {
                version: diff.version.clone(),
                filename: filename.clone(),
                mode: diff.mode,
            })
            .collect();
        FolderRecord {
            folder: self.entry.folder.clone(),
            folder_mtime: self.folder_mtime,
            from_osu_db: self.from_osu_db,
            set_id: self.entry.set_id,
            artist: self.entry.artist.clone(),
            title: self.entry.title.clone(),
            creator: self.entry.creator.clone(),
            tags: self.entry.tags.clone(),
            unicode_terms: self.unicode_terms.clone(),
            diffs,
        }
    }
}

/// Complete searchable Songs folder index with cache metadata and precomputed haystacks.
#[derive(Debug, Clone)]
pub struct SongsIndex {
    pub songs_dir: PathBuf,
    pub source: &'static str,
    pub source_kind: u8,
    pub total_diffs: usize,
    pub fingerprint: SongsFingerprint,
    pub persisted_osu_db: Option<OsuDbFingerprint>,
    pub mapsets: Vec<IndexedMapset>,
    pub empty_folders: HashMap<String, (String, (i64, u32))>,
}

impl SongsIndex {
    /// Multi-word case-insensitive search with zero per-mapset heap allocations.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SongMapsetEntry> {
        if limit == 0 {
            return Vec::new();
        }
        let tokens = query_tokens(query);
        if tokens.is_empty() {
            return self
                .mapsets
                .iter()
                .take(limit)
                .map(|m| m.entry.clone())
                .collect();
        }
        let mut matches = Vec::new();
        for mapset in &self.mapsets {
            if mapset.matches_tokens(&tokens) {
                matches.push(mapset.entry.clone());
                if matches.len() >= limit {
                    break;
                }
            }
        }
        matches
    }

    /// Cheap filesystem staleness check (at most 2 `stat` calls).
    pub fn is_fresh(&self) -> bool {
        if folder_mtime(&self.songs_dir) != self.fingerprint.songs_dir_mtime {
            return false;
        }
        match &self.fingerprint.osu_db {
            Some(prev) => stat_osu_db(&prev.path).as_ref() == Some(prev),
            None => !self
                .songs_dir
                .parent()
                .is_some_and(|p| p.join("osu!.db").is_file()),
        }
    }

    /// Re-scans one direct subfolder of `songs_dir` in place after e.g. `create_hitsound_diff`.
    /// Returns `false` (no-op) when `mapset_folder` is not a direct child of `self.songs_dir`.
    pub fn patch_folder(&mut self, mapset_folder: &Path) -> bool {
        let Some(parent) = mapset_folder.parent() else {
            return false;
        };
        if clean_path_for_compare(parent) != clean_path_for_compare(&self.songs_dir) {
            return false;
        }
        let Some(folder_name) = mapset_folder.file_name().map(|n| n.to_string_lossy().into_owned())
        else {
            return false;
        };
        if !is_safe_folder_name(&folder_name) {
            return false;
        }

        let folder_key = folder_name.to_lowercase();
        let existing_pos = self
            .mapsets
            .iter()
            .position(|m| m.entry.folder.to_lowercase() == folder_key);

        if !mapset_folder.is_dir() {
            let mut changed = false;
            if let Some(pos) = existing_pos {
                let removed = self.mapsets.remove(pos);
                self.total_diffs = self.total_diffs.saturating_sub(removed.entry.diffs.len());
                changed = true;
            }
            if self.empty_folders.remove(&folder_key).is_some() {
                changed = true;
            }
            if changed {
                self.fingerprint.songs_dir_mtime = folder_mtime(&self.songs_dir);
            }
            return changed;
        }

        let info = SubfolderInfo {
            path: self.songs_dir.join(&folder_name),
            name: folder_name,
            mtime: folder_mtime(mapset_folder),
        };
        let scanned = scan_single_folder(&info);

        if scanned.diffs.is_empty() {
            if let Some(pos) = existing_pos {
                let removed = self.mapsets.remove(pos);
                self.total_diffs = self.total_diffs.saturating_sub(removed.entry.diffs.len());
            }
            self.empty_folders
                .insert(folder_key, (scanned.folder, scanned.folder_mtime));
        } else {
            self.empty_folders.remove(&folder_key);
            let indexed = IndexedMapset::from_record(scanned, &self.songs_dir);
            if let Some(pos) = existing_pos {
                let old_diffs = self.mapsets[pos].entry.diffs.len();
                self.total_diffs = self.total_diffs.saturating_sub(old_diffs) + indexed.entry.diffs.len();
                self.mapsets[pos] = indexed;
            } else {
                self.total_diffs += indexed.entry.diffs.len();
                self.mapsets.push(indexed);
            }
            self.mapsets.sort_by_cached_key(|m| {
                (
                    m.entry.artist.to_lowercase(),
                    m.entry.title.to_lowercase(),
                    m.entry.folder.to_lowercase(),
                )
            });
        }

        self.fingerprint.songs_dir_mtime = folder_mtime(&self.songs_dir);
        true
    }

    /// Persists this index atomically to `<cache_root>/songs-index/<hash>.sidx`.
    pub fn save_to_cache(&self, cache_root: &Path) -> io::Result<()> {
        let cache_path = cache_file_path(cache_root, &self.songs_dir);
        if let Some(parent) = cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = encode_cache_bytes(self);
        safe_write::write_new(&cache_path, &bytes).map_err(|e| io::Error::other(e.to_string()))
    }

    /// Finds the best matching `.osu` path for a moved recent map without cloning the index.
    pub fn find_relink_candidate(&self, missing_osu_path: &Path) -> Option<PathBuf> {
        find_relink_candidate_iter(self.mapsets.iter().map(|m| &m.entry), missing_osu_path)
    }
}

/// Locates the best candidate `.osu` path across pre-indexed mapsets.
pub fn find_relink_candidate(
    entries: &[SongMapsetEntry],
    missing_osu_path: &Path,
) -> Option<PathBuf> {
    find_relink_candidate_iter(entries.iter(), missing_osu_path)
}

fn find_relink_candidate_iter<'a>(
    entries: impl Iterator<Item = &'a SongMapsetEntry>,
    missing_osu_path: &Path,
) -> Option<PathBuf> {
    let missing_file = missing_osu_path.file_name()?.to_str()?;
    let old_folder_name = missing_osu_path
        .parent()?
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let mut best: Option<(u32, PathBuf)> = None;
    for set in entries {
        let folder_matches = set.folder.to_ascii_lowercase() == old_folder_name;
        for diff in &set.diffs {
            let candidate = PathBuf::from(&diff.path);
            let file_matches = candidate
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(missing_file));
            let score = u32::from(folder_matches) * 2 + u32::from(file_matches) * 3;
            if score > 0
                && candidate.is_file()
                && best.as_ref().is_none_or(|(s, _)| score > *s)
            {
                best = Some((score, candidate));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Build the Songs folder index, using `osu!.db` when available
/// and falling back to a parallel header-only `.osu` scan otherwise.
pub fn build_songs_index(songs_dir: &Path) -> Vec<SongMapsetEntry> {
    build_songs_index_with_source(songs_dir).0
}

/// Same as [`build_songs_index`], also reporting which source populated it
/// (`"osu!.db"` or `"folder-scan"`).
pub fn build_songs_index_with_source(songs_dir: &Path) -> (Vec<SongMapsetEntry>, &'static str) {
    if let Some(db_path) = discover_osu_db(songs_dir)
        && let Ok(bytes) = read_file_with_share_retry(&db_path)
        && let Ok(entries) = parse_osu_db(&bytes, songs_dir)
        && !entries.is_empty()
    {
        return (entries, "osu!.db");
    }
    (scan_songs_folder_fallback(songs_dir), "folder-scan")
}

/// Loads from the on-disk `.sidx` cache when fresh, reconciles deltas / `osu!.db`,
/// and persists the updated `.sidx` cache under `cache_root`.
pub fn load_or_build_songs_index(
    songs_dir: &Path,
    cache_root: &Path,
    force_refresh: bool,
) -> SongsIndex {
    let (index, needs_save) = build_songs_index_from_cache(songs_dir, cache_root, force_refresh);
    if needs_save {
        let _ = index.save_to_cache(cache_root);
    }
    index
}

/// Builds a [`SongsIndex`] using `.sidx` and `osu!.db` without writing the `.sidx` file yet.
/// Returns `(index, needs_save)` so callers with generation guards can skip stale disk writes.
pub fn build_songs_index_from_cache(
    songs_dir: &Path,
    cache_root: &Path,
    force_refresh: bool,
) -> (SongsIndex, bool) {
    let songs_dir_mtime = folder_mtime(songs_dir);
    let osu_db_fp = discover_osu_db(songs_dir)
        .as_deref()
        .and_then(stat_osu_db);
    let subfolders = list_subfolders(songs_dir);
    let live_fp = SongsFingerprint {
        songs_dir: songs_dir.to_path_buf(),
        songs_dir_mtime,
        osu_db: osu_db_fp.clone(),
    };
    let cached = load_cache_file(&cache_file_path(cache_root, songs_dir), songs_dir);

    // 1. WarmDb: `.sidx` already matches live `osu!.db`; only delta-scan new/changed folders.
    if !force_refresh
        && let Some(db_fp) = osu_db_fp.as_ref()
        && let Some(c) = cached.as_ref()
        && c.source_kind == SOURCE_OSU_DB
        && c.osu_db.as_ref().is_some_and(|prev| db_fp_matches(prev, db_fp))
    {
        let (records, changed) = reconcile_cached_records(&c.records, &subfolders);
        let index = finalize_songs_index(
            songs_dir,
            "osu!.db",
            SOURCE_OSU_DB,
            live_fp,
            Some(db_fp.clone()),
            records,
        );
        return (index, changed);
    }

    // 2. ParseDb (or CacheRescue when `osu!.db` fails and `!force_refresh`).
    if let Some(db_fp) = osu_db_fp.as_ref() {
        if let Ok(bytes) = read_file_with_share_retry(&db_fp.path)
            && let Ok(mut db_map) = parse_osu_db_records(&bytes, &subfolders)
            && !db_map.is_empty()
        {
            let mut empty_records = Vec::new();
            let mut cached_by_key: HashMap<String, &FolderRecord> = cached
                .as_ref()
                .map(|c| {
                    c.records
                        .iter()
                        .map(|r| (r.folder.to_lowercase(), r))
                        .collect()
                })
                .unwrap_or_default();

            let mut to_scan = Vec::new();
            let mut always_scan = Vec::new();
            let mut locally_newer = Vec::new();
            let db_mtime = db_fp.stamp.mtime();

            for (key, live) in &subfolders {
                if let Some(db_rec) = db_map.get_mut(key) {
                    db_rec.folder_mtime = live.mtime;
                    // M3: preserve locally created diffs (`from_osu_db == false`) even if
                    // osu!stable rewrote `osu!.db` on exit without scanning that folder.
                    if let Some(prev) = cached_by_key.remove(key)
                        && !prev.from_osu_db
                        && has_extra_diffs(prev, db_rec)
                    {
                        if !force_refresh && prev.folder_mtime == live.mtime {
                            *db_rec = prev.clone();
                            continue;
                        }
                        always_scan.push(live.clone());
                        continue;
                    }
                    if live.mtime > db_mtime {
                        locally_newer.push(live.clone());
                    }
                } else if !force_refresh
                    && let Some(prev) = cached_by_key.remove(key)
                    && prev.folder_mtime == live.mtime
                {
                    if prev.diffs.is_empty() {
                        empty_records.push(prev.clone());
                    } else {
                        db_map.insert(key.clone(), prev.clone());
                    }
                } else {
                    to_scan.push(live.clone());
                }
            }

            to_scan.extend(always_scan);
            if locally_newer.len() <= MAX_LOCAL_NEWER_RESCAN {
                to_scan.extend(locally_newer);
            }

            if !to_scan.is_empty() {
                for rec in scan_folders_parallel(&to_scan) {
                    let key = rec.folder.to_lowercase();
                    if rec.diffs.is_empty() {
                        db_map.remove(&key);
                        empty_records.push(rec);
                    } else {
                        db_map.insert(key, rec);
                    }
                }
            }

            let mut records: Vec<FolderRecord> = db_map.into_values().collect();
            records.extend(empty_records);
            let index = finalize_songs_index(
                songs_dir,
                "osu!.db",
                SOURCE_OSU_DB,
                live_fp,
                Some(db_fp.clone()),
                records,
            );
            return (index, true);
        }

        // C3: On `!force_refresh`, rescue from `.sidx` and stamp the LIVE observed `osu!.db`
        // fingerprint in RAM so typing does not re-read and re-fail a corrupt DB per keystroke.
        if !force_refresh && let Some(c) = cached {
            let (records, changed) = reconcile_cached_records(&c.records, &subfolders);
            let index = finalize_songs_index(
                songs_dir,
                "cache",
                c.source_kind,
                live_fp,
                c.osu_db,
                records,
            );
            return (index, changed);
        }
    }

    // 3. WarmScan (`osu!.db` absent, valid `.sidx`, `!force_refresh`).
    if !force_refresh && osu_db_fp.is_none() && let Some(c) = cached {
        let (records, changed) = reconcile_cached_records(&c.records, &subfolders);
        let index = finalize_songs_index(
            songs_dir,
            "cache",
            SOURCE_FOLDER_SCAN,
            live_fp,
            None,
            records,
        );
        return (index, changed);
    }

    // 4. FullScan (cold or `force_refresh` fallback).
    let folder_list: Vec<SubfolderInfo> = subfolders.into_values().collect();
    let records = scan_folders_parallel(&folder_list);
    let index = finalize_songs_index(
        songs_dir,
        "folder-scan",
        SOURCE_FOLDER_SCAN,
        live_fp,
        None,
        records,
    );
    (index, true)
}

fn db_fp_matches(a: &OsuDbFingerprint, b: &OsuDbFingerprint) -> bool {
    a.stamp == b.stamp && clean_path_for_compare(&a.path) == clean_path_for_compare(&b.path)
}

fn has_extra_diffs(local_rec: &FolderRecord, db_rec: &FolderRecord) -> bool {
    let db_files: HashSet<String> = db_rec
        .diffs
        .iter()
        .map(|d| d.filename.to_lowercase())
        .collect();
    local_rec
        .diffs
        .iter()
        .any(|d| !db_files.contains(&d.filename.to_lowercase()))
}

fn reconcile_cached_records(
    cached_records: &[FolderRecord],
    subfolders: &HashMap<String, SubfolderInfo>,
) -> (Vec<FolderRecord>, bool) {
    let mut kept_keys = HashSet::with_capacity(cached_records.len().min(subfolders.len()));
    let mut kept = Vec::with_capacity(cached_records.len().min(subfolders.len()));
    let mut deleted_any = false;
    let mut to_scan = Vec::new();

    for rec in cached_records {
        let key = rec.folder.to_lowercase();
        let Some(live) = subfolders.get(&key) else {
            deleted_any = true;
            continue;
        };
        if rec.folder_mtime == live.mtime {
            kept_keys.insert(key);
            kept.push(rec.clone());
        } else {
            kept_keys.insert(key);
            to_scan.push(live.clone());
        }
    }

    for (key, live) in subfolders {
        if !kept_keys.contains(key) {
            to_scan.push(live.clone());
        }
    }

    let changed = deleted_any || !to_scan.is_empty();
    if !to_scan.is_empty() {
        kept.extend(scan_folders_parallel(&to_scan));
    }
    (kept, changed)
}

fn finalize_songs_index(
    songs_dir: &Path,
    source: &'static str,
    source_kind: u8,
    fingerprint: SongsFingerprint,
    persisted_osu_db: Option<OsuDbFingerprint>,
    records: Vec<FolderRecord>,
) -> SongsIndex {
    let mut empty_folders = HashMap::new();
    let mut mapsets = Vec::with_capacity(records.len());
    let mut total_diffs = 0usize;

    for record in records {
        if record.diffs.is_empty() {
            empty_folders.insert(
                record.folder.to_lowercase(),
                (record.folder, record.folder_mtime),
            );
            continue;
        }
        let indexed = IndexedMapset::from_record(record, songs_dir);
        total_diffs += indexed.entry.diffs.len();
        mapsets.push(indexed);
    }

    mapsets.sort_by_cached_key(|m| {
        (
            m.entry.artist.to_lowercase(),
            m.entry.title.to_lowercase(),
            m.entry.folder.to_lowercase(),
        )
    });

    SongsIndex {
        songs_dir: songs_dir.to_path_buf(),
        source,
        source_kind,
        total_diffs,
        fingerprint,
        persisted_osu_db,
        mapsets,
        empty_folders,
    }
}

/// Discovers `osu!.db` beside `songs_dir` (or canonical parent) or via `BeatmapDirectory`
/// inside standard osu!stable install folders.
pub fn discover_osu_db(songs_dir: &Path) -> Option<PathBuf> {
    if let Some(parent_db) = songs_dir.parent().map(|p| p.join("osu!.db"))
        && parent_db.is_file()
    {
        return Some(parent_db);
    }
    if let Ok(canon) = fs::canonicalize(songs_dir)
        && let Some(canon_db) = canon.parent().map(|p| p.join("osu!.db"))
        && canon_db.is_file()
    {
        return Some(canon_db);
    }
    discover_osu_db_with_candidates(songs_dir, &default_osu_roots())
}

/// Checks `osu_roots` for an `osu!.db` whose `osu!.*.cfg` `BeatmapDirectory` resolves to
/// `songs_dir`.
pub fn discover_osu_db_with_candidates(
    songs_dir: &Path,
    osu_roots: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(parent_db) = songs_dir.parent().map(|p| p.join("osu!.db"))
        && parent_db.is_file()
    {
        return Some(parent_db);
    }
    let songs_norm = clean_path_for_compare(songs_dir);
    for root in osu_roots {
        let db_path = root.join("osu!.db");
        if !db_path.is_file() {
            continue;
        }
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !(name == "osu!.cfg" || (name.starts_with("osu!.") && name.ends_with(".cfg"))) {
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let text = String::from_utf8_lossy(&bytes);
            if let Some(beatmap_dir) = parse_beatmap_directory_from_cfg(&text, root)
                && clean_path_for_compare(&beatmap_dir) == songs_norm
            {
                return Some(db_path);
            }
        }
    }
    None
}

fn default_osu_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("osu!"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(
            home.join("Library")
                .join("Application Support")
                .join("osu!"),
        );
        roots.push(home.join(".local").join("share").join("osu-wine").join("osu!"));
        roots.push(home.join(".local").join("share").join("osu!"));
    }
    roots
}

fn parse_beatmap_directory_from_cfg(text: &str, osu_root: &Path) -> Option<PathBuf> {
    for raw_line in text.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("BeatmapDirectory") {
            continue;
        }
        let mut val = value.trim();
        if (val.starts_with('"') && val.ends_with('"') && val.len() >= 2)
            || (val.starts_with('\'') && val.ends_with('\'') && val.len() >= 2)
        {
            val = val[1..val.len() - 1].trim();
        }
        val = val.trim_end_matches(['/', '\\']);
        if val.is_empty() {
            return None;
        }
        let p = Path::new(val);
        return Some(if p.is_relative() {
            osu_root.join(p)
        } else {
            p.to_path_buf()
        });
    }
    None
}

fn clean_path_for_compare(path: &Path) -> String {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| lexical_normalize(path));
    let raw = resolved.to_string_lossy();
    let stripped = if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(drive) = raw.strip_prefix(r"\\?\") {
        drive.to_string()
    } else {
        raw.into_owned()
    };
    let unified = stripped.replace('\\', "/");
    let trimmed = unified.trim_end_matches('/');
    if cfg!(any(windows, target_os = "macos")) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(comp);
                }
            }
            _ => out.push(comp),
        }
    }
    out
}

/// Reads a file, retrying briefly on sharing/lock violations (e.g. while osu!stable saves).
pub fn read_file_with_share_retry(path: &Path) -> io::Result<Vec<u8>> {
    const WAITS: [Duration; 2] = [Duration::from_millis(50), Duration::from_millis(100)];
    let mut attempt = 0usize;
    loop {
        match fs::read(path) {
            Ok(bytes) => return Ok(bytes),
            Err(err) if attempt < WAITS.len() && is_share_or_lock_error(&err) => {
                std::thread::sleep(WAITS[attempt]);
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

fn is_share_or_lock_error(err: &io::Error) -> bool {
    if err.kind() == ErrorKind::PermissionDenied {
        return true;
    }
    #[cfg(windows)]
    if matches!(err.raw_os_error(), Some(32 | 33)) {
        return true;
    }
    false
}

/// Converts `SystemTime` into a monotonically ordered `(secs, nanos)` tuple (including pre-1970).
pub fn system_time_stamp(t: SystemTime) -> (i64, u32) {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => (
            i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            d.subsec_nanos(),
        ),
        Err(err) => {
            let d = err.duration();
            let secs = i64::try_from(d.as_secs()).unwrap_or(i64::MAX);
            if d.subsec_nanos() == 0 {
                (-secs, 0)
            } else {
                (-secs.saturating_add(1), 1_000_000_000 - d.subsec_nanos())
            }
        }
    }
}

fn folder_mtime(path: &Path) -> (i64, u32) {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(system_time_stamp)
        .unwrap_or((0, 0))
}

fn stat_osu_db(path: &Path) -> Option<OsuDbFingerprint> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let (mtime_secs, mtime_nanos) = meta.modified().map(system_time_stamp).unwrap_or((0, 0));
    Some(OsuDbFingerprint {
        path: path.to_path_buf(),
        stamp: FileStamp {
            len: meta.len(),
            mtime_secs,
            mtime_nanos,
        },
    })
}

fn is_safe_folder_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
}

fn is_safe_osu_filename(name: &str) -> bool {
    is_safe_folder_name(name)
        && Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("osu"))
}

#[derive(Debug, Clone)]
struct SubfolderInfo {
    path: PathBuf,
    name: String,
    mtime: (i64, u32),
}

fn list_subfolders(songs_dir: &Path) -> HashMap<String, SubfolderInfo> {
    let mut map = HashMap::new();
    let Ok(entries) = fs::read_dir(songs_dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !(ft.is_dir() || ft.is_symlink()) {
            continue;
        }
        let path = entry.path();
        // Use `fs::metadata(&path)` rather than `entry.metadata()`: on Windows NTFS,
        // `FindNextFileW` (`DirEntry::metadata`) reads the parent `$I30` index cache,
        // which can stay stale after files are added/removed inside a mapset folder,
        // whereas `fs::metadata` reads the folder's live mtime (matching `folder_mtime`).
        let Ok(meta) = fs::metadata(&path) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let mtime = meta
            .modified()
            .map(system_time_stamp)
            .unwrap_or((0, 0));

        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_safe_folder_name(&name) {
            continue;
        }
        map.insert(name.to_lowercase(), SubfolderInfo { path, name, mtime });
    }
    map
}

#[derive(Debug, Clone)]
struct CachedDiff {
    version: String,
    filename: String,
    mode: u8,
}

#[derive(Debug, Clone)]
struct FolderRecord {
    folder: String,
    folder_mtime: (i64, u32),
    from_osu_db: bool,
    set_id: i32,
    artist: String,
    title: String,
    creator: String,
    tags: String,
    unicode_terms: String,
    diffs: Vec<CachedDiff>,
}

/// Parse osu!'s binary `osu!.db` beatmap table (v20140609 through v2025+) and
/// resolve difficulty paths against `songs_dir`, skipping missing folders.
pub fn parse_osu_db(bytes: &[u8], songs_dir: &Path) -> Result<Vec<SongMapsetEntry>, String> {
    let subfolders = list_subfolders(songs_dir);
    let records = parse_osu_db_records(bytes, &subfolders)?;
    Ok(records_to_entries(records.into_values().collect(), songs_dir))
}

/// Fast in-memory `osu!.db` parse given a pre-supplied `(folder_name, mtime)` list,
/// avoiding filesystem directory creation in large benchmarks and unit tests.
pub fn parse_osu_db_with_folders(
    bytes: &[u8],
    songs_dir: &Path,
    folders: &[(String, (i64, u32))],
) -> Result<Vec<SongMapsetEntry>, String> {
    let mut subfolders = HashMap::with_capacity(folders.len());
    for (name, mtime) in folders {
        if is_safe_folder_name(name) {
            subfolders.insert(
                name.to_lowercase(),
                SubfolderInfo {
                    path: songs_dir.join(name),
                    name: name.clone(),
                    mtime: *mtime,
                },
            );
        }
    }
    let records = parse_osu_db_records(bytes, &subfolders)?;
    Ok(records_to_entries(records.into_values().collect(), songs_dir))
}

fn records_to_entries(records: Vec<FolderRecord>, songs_dir: &Path) -> Vec<SongMapsetEntry> {
    let mut mapsets: Vec<SongMapsetEntry> = records
        .into_iter()
        .filter(|r| !r.diffs.is_empty())
        .map(|r| IndexedMapset::from_record(r, songs_dir).entry)
        .collect();
    mapsets.sort_by_cached_key(|m| {
        (
            m.artist.to_lowercase(),
            m.title.to_lowercase(),
            m.folder.to_lowercase(),
        )
    });
    mapsets
}

fn parse_osu_db_records(
    bytes: &[u8],
    existing_folders: &HashMap<String, SubfolderInfo>,
) -> Result<HashMap<String, FolderRecord>, String> {
    let mut reader = DbReader::new(bytes);
    let version = reader.read_i32()?;
    let _folder_count = reader.read_i32()?;
    let _account_unlocked = reader.read_u8()?;
    let _unlock_date = reader.read_i64()?;
    reader.skip_osu_string()?; // player_name
    let beatmap_count = reader.read_i32()?;
    if beatmap_count < 0 {
        return Err("Negative beatmap count in osu!.db".into());
    }
    if (beatmap_count as usize) > bytes.len() / MIN_OSU_DB_ENTRY_BYTES {
        return Err("Unexpected end of osu!.db".into());
    }

    let cap = (beatmap_count as usize)
        .min(bytes.len() / 64)
        .min(existing_folders.len());
    let mut groups: HashMap<String, FolderRecord> = HashMap::with_capacity(cap);
    let mut seen_diffs: HashMap<String, HashSet<String>> = HashMap::with_capacity(cap);

    for _ in 0..beatmap_count {
        if version < 20191106 {
            let _entry_size = reader.read_i32()?;
        }

        let artist = reader.read_osu_string()?;
        let artist_unicode = reader.read_osu_string()?;
        let title = reader.read_osu_string()?;
        let title_unicode = reader.read_osu_string()?;
        let creator = reader.read_osu_string()?;
        let diff_version = reader.read_osu_string()?;
        reader.skip_osu_string()?; // audio_filename
        reader.skip_osu_string()?; // md5_hash
        let osu_filename = reader.read_osu_string()?;

        // ranked_status(1) + hit circle(2) + slider(2) + spinner(2) + last_modified(8)
        reader.skip(15)?;
        // AR, CS, HP, OD
        if version < 20140609 {
            reader.skip(4)?;
        } else {
            reader.skip(16)?;
        }
        // slider_velocity (f64)
        reader.skip(8)?;

        if version >= 20140609 {
            // 4 star-rating Int-Double / Int-Float dictionaries (osu, taiko, catch, mania).
            for _ in 0..4 {
                reader.skip_star_rating_dict()?;
            }
        }

        // drain_time(4) + total_time(4) + preview_time(4)
        reader.skip(12)?;
        // timing points: count * (f64 + f64 + u8 = 17 bytes)
        let timing_count = reader.read_i32()?;
        if timing_count < 0 {
            return Err("Negative timing point count in osu!.db".into());
        }
        let timing_bytes = (timing_count as usize)
            .checked_mul(17)
            .ok_or_else(|| "Timing point count overflow in osu!.db".to_string())?;
        reader.skip(timing_bytes)?;

        let _beatmap_id = reader.read_i32()?;
        let set_id = reader.read_i32()?;
        let _thread_id = reader.read_i32()?;
        // 4 grades(4) + local_offset(2) + stack_leniency(4)
        reader.skip(10)?;
        let mode = reader.read_u8()?;

        reader.skip_osu_string()?; // song_source
        let tags = reader.read_osu_string()?;
        // online_offset(2)
        reader.skip(2)?;
        reader.skip_osu_string()?; // title_font
        // unplayed(1) + last_played(8) + is_osz2(1)
        reader.skip(10)?;
        let folder_name = reader.read_osu_string()?;
        // last_checked(8) + ignore_sound(1) + ignore_skin(1) + disable_storyboard(1)
        // + disable_video(1) + visual_override(1)
        reader.skip(13)?;
        if version < 20140609 {
            reader.skip(2)?;
        }
        // last_modified_int(4) + mania_scroll_speed(1)
        reader.skip(5)?;

        let folder_trimmed = folder_name.trim();
        let file_trimmed = osu_filename.trim();
        if !is_safe_folder_name(folder_trimmed) || !is_safe_osu_filename(file_trimmed) {
            continue;
        }
        let folder_key = folder_trimmed.to_lowercase();
        let Some(actual_folder) = existing_folders.get(&folder_key) else {
            continue;
        };

        let file_key = file_trimmed.to_lowercase();
        let diff_set = seen_diffs.entry(folder_key.clone()).or_default();
        if !diff_set.insert(file_key) {
            continue;
        }

        let entry = groups.entry(folder_key).or_insert_with(|| FolderRecord {
            folder: actual_folder.name.clone(),
            folder_mtime: actual_folder.mtime,
            from_osu_db: true,
            set_id,
            artist: artist.clone(),
            title: title.clone(),
            creator: creator.clone(),
            tags: tags.clone(),
            unicode_terms: String::new(),
            diffs: Vec::new(),
        });
        if entry.set_id <= 0 && set_id > 0 {
            entry.set_id = set_id;
        }
        if entry.artist.is_empty() && !artist.is_empty() {
            entry.artist = artist.clone();
        }
        if entry.title.is_empty() && !title.is_empty() {
            entry.title = title.clone();
        }
        if entry.creator.is_empty() && !creator.is_empty() {
            entry.creator = creator;
        }
        if entry.tags.is_empty() && !tags.is_empty() {
            entry.tags = tags;
        }
        append_unicode_term(&mut entry.unicode_terms, &artist_unicode, &entry.artist);
        append_unicode_term(&mut entry.unicode_terms, &title_unicode, &entry.title);
        entry.diffs.push(CachedDiff {
            version: if diff_version.is_empty() {
                Path::new(file_trimmed)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file_trimmed.to_string())
            } else {
                diff_version
            },
            filename: file_trimmed.to_string(),
            mode,
        });
    }

    Ok(groups)
}

fn append_unicode_term(terms: &mut String, unicode: &str, romanized: &str) {
    let trimmed = unicode.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(romanized.trim()) {
        return;
    }
    if !terms.contains(trimmed) {
        if !terms.is_empty() {
            terms.push('\n');
        }
        terms.push_str(trimmed);
    }
}

/// Fast header-only directory scan of `songs_dir`, reading only `[General]` and
/// `[Metadata]` of each `.osu` file in bounded parallel workers and stopping before `[HitObjects]`.
pub fn scan_songs_folder_fallback(songs_dir: &Path) -> Vec<SongMapsetEntry> {
    let folders: Vec<SubfolderInfo> = list_subfolders(songs_dir).into_values().collect();
    let records = scan_folders_parallel(&folders);
    records_to_entries(records, songs_dir)
}

fn scan_folders_parallel(folders: &[SubfolderInfo]) -> Vec<FolderRecord> {
    if folders.is_empty() {
        return Vec::new();
    }
    if folders.len() <= PARALLEL_THRESHOLD {
        let mut buf = Vec::with_capacity(512);
        return folders
            .iter()
            .map(|f| scan_single_folder_with_buf(f, &mut buf))
            .collect();
    }

    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, MAX_SCAN_WORKERS))
        .unwrap_or(4)
        .min(folders.len().max(1));

    let next_idx = AtomicUsize::new(0);
    let mut buckets: Vec<Vec<(usize, FolderRecord)>> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            handles.push(scope.spawn(|| {
                let mut buf = Vec::with_capacity(512);
                let mut local = Vec::new();
                loop {
                    let i = next_idx.fetch_add(1, Ordering::Relaxed);
                    if i >= folders.len() {
                        break;
                    }
                    local.push((i, scan_single_folder_with_buf(&folders[i], &mut buf)));
                }
                local
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });

    let mut ordered: Vec<(usize, FolderRecord)> = Vec::with_capacity(folders.len());
    for bucket in &mut buckets {
        ordered.append(bucket);
    }
    ordered.sort_by_key(|(idx, _)| *idx);
    ordered.into_iter().map(|(_, rec)| rec).collect()
}

fn scan_single_folder(folder: &SubfolderInfo) -> FolderRecord {
    let mut buf = Vec::with_capacity(512);
    scan_single_folder_with_buf(folder, &mut buf)
}

fn scan_single_folder_with_buf(folder: &SubfolderInfo, line_buf: &mut Vec<u8>) -> FolderRecord {
    let mut set_id = -1i32;
    let mut artist = String::new();
    let mut title = String::new();
    let mut creator = String::new();
    let mut tags = String::new();
    let mut unicode_terms = String::new();
    let mut diffs = Vec::new();

    let Ok(files) = fs::read_dir(&folder.path) else {
        return FolderRecord {
            folder: folder.name.clone(),
            folder_mtime: folder.mtime,
            from_osu_db: false,
            set_id,
            artist,
            title,
            creator,
            tags,
            unicode_terms,
            diffs,
        };
    };

    for file_entry in files.flatten() {
        let filename = file_entry.file_name().to_string_lossy().into_owned();
        if !is_safe_osu_filename(&filename) {
            continue;
        }
        let file_path = file_entry.path();
        let Some(header) = read_osu_header_with_buf(&file_path, line_buf) else {
            continue;
        };
        if set_id <= 0 && header.set_id > 0 {
            set_id = header.set_id;
        }
        if artist.is_empty() && !header.artist.is_empty() {
            artist = header.artist;
        }
        if title.is_empty() && !header.title.is_empty() {
            title = header.title;
        }
        if creator.is_empty() && !header.creator.is_empty() {
            creator = header.creator;
        }
        if tags.is_empty() && !header.tags.is_empty() {
            tags = header.tags;
        }
        append_unicode_term(&mut unicode_terms, &header.artist_unicode, &artist);
        append_unicode_term(&mut unicode_terms, &header.title_unicode, &title);

        let version = if header.version.is_empty() {
            Path::new(&filename)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| filename.clone())
        } else {
            header.version
        };
        diffs.push(CachedDiff {
            version,
            filename,
            mode: header.mode,
        });
    }

    FolderRecord {
        folder: folder.name.clone(),
        folder_mtime: folder.mtime,
        from_osu_db: false,
        set_id,
        artist,
        title,
        creator,
        tags,
        unicode_terms,
        diffs,
    }
}

/// Multi-word case-insensitive search across artist, title, creator, difficulty
/// version, tags, folder name, and BeatmapSetID.
pub fn search_songs_index(
    index: &[SongMapsetEntry],
    query: &str,
    limit: usize,
) -> Vec<SongMapsetEntry> {
    if limit == 0 {
        return Vec::new();
    }
    let tokens = query_tokens(query);
    if tokens.is_empty() {
        return index.iter().take(limit).cloned().collect();
    }

    let mut matches = Vec::new();
    for mapset in index {
        if mapset_matches(mapset, &tokens) {
            matches.push(mapset.clone());
            if matches.len() >= limit {
                break;
            }
        }
    }
    matches
}

fn query_tokens(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn build_search_haystack(entry: &SongMapsetEntry, unicode_terms: &str) -> String {
    let mut raw = String::with_capacity(
        entry.artist.len()
            + entry.title.len()
            + entry.creator.len()
            + entry.tags.len()
            + entry.folder.len()
            + unicode_terms.len()
            + entry.diffs.len() * 20
            + 24,
    );
    for field in [
        entry.artist.as_str(),
        entry.title.as_str(),
        entry.creator.as_str(),
        entry.tags.as_str(),
        entry.folder.as_str(),
        unicode_terms,
    ] {
        if !field.is_empty() {
            raw.push_str(field);
            raw.push('\n');
        }
    }
    if entry.set_id > 0 {
        use std::fmt::Write as _;
        let _ = writeln!(raw, "{}", entry.set_id);
    }
    for diff in &entry.diffs {
        raw.push_str(&diff.version);
        raw.push('\n');
    }
    raw.to_lowercase()
}

fn mapset_matches(mapset: &SongMapsetEntry, tokens: &[String]) -> bool {
    let artist = mapset.artist.to_lowercase();
    let title = mapset.title.to_lowercase();
    let creator = mapset.creator.to_lowercase();
    let tags = mapset.tags.to_lowercase();
    let folder = mapset.folder.to_lowercase();
    let set_id = if mapset.set_id > 0 {
        mapset.set_id.to_string()
    } else {
        String::new()
    };

    tokens.iter().all(|tok| {
        artist.contains(tok)
            || title.contains(tok)
            || creator.contains(tok)
            || tags.contains(tok)
            || folder.contains(tok)
            || (!set_id.is_empty() && set_id.contains(tok))
            || mapset
                .diffs
                .iter()
                .any(|d| d.version.to_lowercase().contains(tok))
    })
}

struct OsuHeader {
    artist: String,
    artist_unicode: String,
    title: String,
    title_unicode: String,
    creator: String,
    version: String,
    tags: String,
    set_id: i32,
    mode: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderSection {
    None,
    General,
    Metadata,
    Other,
}

fn read_osu_header_with_buf(path: &Path, line_buf: &mut Vec<u8>) -> Option<OsuHeader> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::with_capacity(4096, file);
    let mut section = HeaderSection::None;
    let mut saw_general = false;
    let mut saw_metadata = false;
    let mut total_bytes = 0usize;
    let mut header = OsuHeader {
        artist: String::new(),
        artist_unicode: String::new(),
        title: String::new(),
        title_unicode: String::new(),
        creator: String::new(),
        version: String::new(),
        tags: String::new(),
        set_id: -1,
        mode: 0,
    };

    for _ in 0..HEADER_MAX_LINES {
        if total_bytes >= HEADER_MAX_BYTES {
            break;
        }
        line_buf.clear();
        let n = reader.read_until(b'\n', line_buf).ok()?;
        if n == 0 {
            break;
        }
        total_bytes += n;
        let line = String::from_utf8_lossy(line_buf);
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if trimmed == "[HitObjects]" || trimmed == "[TimingPoints]" {
                break;
            }
            if saw_general && saw_metadata {
                break;
            }
            section = match trimmed {
                "[General]" => {
                    saw_general = true;
                    HeaderSection::General
                }
                "[Metadata]" => {
                    saw_metadata = true;
                    HeaderSection::Metadata
                }
                _ => HeaderSection::Other,
            };
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match section {
            HeaderSection::General if key == "Mode" => {
                if let Ok(m) = value.parse::<u8>() {
                    header.mode = m;
                }
            }
            HeaderSection::Metadata => match key {
                "Artist" => header.artist = value.to_string(),
                "ArtistUnicode" => header.artist_unicode = value.to_string(),
                "Title" => header.title = value.to_string(),
                "TitleUnicode" => header.title_unicode = value.to_string(),
                "Creator" => header.creator = value.to_string(),
                "Version" => header.version = value.to_string(),
                "Tags" => header.tags = value.to_string(),
                "BeatmapSetID" => {
                    if let Ok(id) = value.parse::<i32>() {
                        header.set_id = id;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    Some(header)
}

fn cache_file_path(cache_root: &Path, songs_dir: &Path) -> PathBuf {
    let norm = clean_path_for_compare(songs_dir);
    let digest = sha256_hex(norm.as_bytes());
    cache_root
        .join("songs-index")
        .join(format!("{}.sidx", &digest[..16]))
}

struct LoadedCache {
    source_kind: u8,
    osu_db: Option<OsuDbFingerprint>,
    records: Vec<FolderRecord>,
}

fn encode_cache_bytes(index: &SongsIndex) -> Vec<u8> {
    let total_records = index.mapsets.len() + index.empty_folders.len();
    let mut out = Vec::with_capacity(256 + total_records * 128 + index.total_diffs * 48);
    out.extend_from_slice(CACHE_MAGIC);
    out.extend_from_slice(&CACHE_VERSION.to_le_bytes());
    write_cache_string(&mut out, &clean_path_for_compare(&index.songs_dir));
    out.push(index.source_kind);
    match &index.persisted_osu_db {
        Some(db) => {
            out.push(1);
            write_cache_string(&mut out, &db.path.to_string_lossy());
            out.extend_from_slice(&db.stamp.len.to_le_bytes());
            out.extend_from_slice(&db.stamp.mtime_secs.to_le_bytes());
            out.extend_from_slice(&db.stamp.mtime_nanos.to_le_bytes());
        }
        None => out.push(0),
    }
    out.extend_from_slice(&index.fingerprint.songs_dir_mtime.0.to_le_bytes());
    out.extend_from_slice(&index.fingerprint.songs_dir_mtime.1.to_le_bytes());
    out.extend_from_slice(&(total_records as u32).to_le_bytes());

    for mapset in &index.mapsets {
        let rec = mapset.to_record();
        write_cache_record(&mut out, &rec);
    }
    for (folder, mtime) in index.empty_folders.values() {
        let empty_rec = FolderRecord {
            folder: folder.clone(),
            folder_mtime: *mtime,
            from_osu_db: false,
            set_id: -1,
            artist: String::new(),
            title: String::new(),
            creator: String::new(),
            tags: String::new(),
            unicode_terms: String::new(),
            diffs: Vec::new(),
        };
        write_cache_record(&mut out, &empty_rec);
    }
    out
}

fn write_cache_record(out: &mut Vec<u8>, rec: &FolderRecord) {
    write_cache_string(out, &rec.folder);
    out.extend_from_slice(&rec.folder_mtime.0.to_le_bytes());
    out.extend_from_slice(&rec.folder_mtime.1.to_le_bytes());
    out.push(u8::from(rec.from_osu_db));
    out.extend_from_slice(&rec.set_id.to_le_bytes());
    write_cache_string(out, &rec.artist);
    write_cache_string(out, &rec.title);
    write_cache_string(out, &rec.creator);
    write_cache_string(out, &rec.tags);
    write_cache_string(out, &rec.unicode_terms);
    out.extend_from_slice(&(rec.diffs.len() as u32).to_le_bytes());
    for diff in &rec.diffs {
        write_cache_string(out, &diff.version);
        write_cache_string(out, &diff.filename);
        out.push(diff.mode);
    }
}

fn write_cache_string(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
}

fn load_cache_file(path: &Path, expected_songs_dir: &Path) -> Option<LoadedCache> {
    let bytes = fs::read(path).ok()?;
    decode_cache_bytes(&bytes, expected_songs_dir)
}

fn decode_cache_bytes(bytes: &[u8], expected_songs_dir: &Path) -> Option<LoadedCache> {
    let mut reader = DbReader::new(bytes);
    if reader.read_slice(8).ok()? != CACHE_MAGIC {
        return None;
    }
    if reader.read_u32().ok()? != CACHE_VERSION {
        return None;
    }
    let cached_dir = reader.read_cache_string().ok()?;
    if cached_dir != clean_path_for_compare(expected_songs_dir) {
        return None;
    }
    let source_kind = reader.read_u8().ok()?;
    let osu_db = match reader.read_u8().ok()? {
        0 => None,
        1 => {
            let db_path = PathBuf::from(reader.read_cache_string().ok()?);
            let len = reader.read_u64().ok()?;
            let mtime_secs = reader.read_i64().ok()?;
            let mtime_nanos = reader.read_u32().ok()?;
            Some(OsuDbFingerprint {
                path: db_path,
                stamp: FileStamp {
                    len,
                    mtime_secs,
                    mtime_nanos,
                },
            })
        }
        _ => return None,
    };
    let _songs_mtime_secs = reader.read_i64().ok()?;
    let _songs_mtime_nanos = reader.read_u32().ok()?;
    let entry_count = reader.read_u32().ok()? as usize;
    if entry_count > reader.remaining() / MIN_SIDX_RECORD_BYTES {
        return None;
    }

    let mut records = Vec::with_capacity(entry_count.min(reader.remaining() / MIN_SIDX_RECORD_BYTES));
    for _ in 0..entry_count {
        let folder = reader.read_cache_string().ok()?;
        let folder_mtime = (reader.read_i64().ok()?, reader.read_u32().ok()?);
        let from_osu_db = match reader.read_u8().ok()? {
            0 => false,
            1 => true,
            _ => return None,
        };
        let set_id = reader.read_i32().ok()?;
        let artist = reader.read_cache_string().ok()?;
        let title = reader.read_cache_string().ok()?;
        let creator = reader.read_cache_string().ok()?;
        let tags = reader.read_cache_string().ok()?;
        let unicode_terms = reader.read_cache_string().ok()?;
        let diff_count = reader.read_u32().ok()? as usize;
        if diff_count > reader.remaining() / MIN_SIDX_DIFF_BYTES {
            return None;
        }
        let mut diffs = Vec::with_capacity(diff_count.min(reader.remaining() / MIN_SIDX_DIFF_BYTES));
        for _ in 0..diff_count {
            let version = reader.read_cache_string().ok()?;
            let filename = reader.read_cache_string().ok()?;
            let mode = reader.read_u8().ok()?;
            if is_safe_osu_filename(&filename) {
                diffs.push(CachedDiff {
                    version,
                    filename,
                    mode,
                });
            }
        }
        if is_safe_folder_name(&folder) {
            records.push(FolderRecord {
                folder,
                folder_mtime,
                from_osu_db,
                set_id,
                artist,
                title,
                creator,
                tags,
                unicode_terms,
                diffs,
            });
        }
    }

    Some(LoadedCache {
        source_kind,
        osu_db,
        records,
    })
}

struct DbReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> DbReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| "Unexpected end of binary index".to_string())?;
        if end > self.bytes.len() {
            return Err("Unexpected end of binary index".into());
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn skip(&mut self, len: usize) -> Result<(), String> {
        self.read_slice(len).map(|_| ())
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        Ok(self.read_slice(1)?[0])
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let b = self.read_slice(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i32(&mut self) -> Result<i32, String> {
        let b = self.read_slice(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, String> {
        let b = self.read_slice(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_i64(&mut self) -> Result<i64, String> {
        let b = self.read_slice(8)?;
        Ok(i64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_cache_string(&mut self) -> Result<String, String> {
        let len = self.read_u32()? as usize;
        let slice = self.read_slice(len)?;
        Ok(String::from_utf8_lossy(slice).into_owned())
    }

    fn read_uleb128(&mut self) -> Result<usize, String> {
        let mut result: usize = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.read_u8()?;
            let low = (byte & 0x7f) as usize;
            result |= low
                .checked_shl(shift)
                .ok_or_else(|| "ULEB128 overflow in osu!.db".to_string())?;
            if (byte & 0x80) == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift >= usize::BITS {
                return Err("ULEB128 too large in osu!.db".into());
            }
        }
    }

    fn skip_osu_string(&mut self) -> Result<(), String> {
        match self.read_u8()? {
            0x00 => Ok(()),
            0x0b => {
                let len = self.read_uleb128()?;
                self.skip(len)
            }
            other => Err(format!("Invalid osu!.db string tag: 0x{other:02x}")),
        }
    }

    fn read_osu_string(&mut self) -> Result<String, String> {
        match self.read_u8()? {
            0x00 => Ok(String::new()),
            0x0b => {
                let len = self.read_uleb128()?;
                let slice = self.read_slice(len)?;
                Ok(String::from_utf8_lossy(slice).into_owned())
            }
            other => Err(format!("Invalid osu!.db string tag: 0x{other:02x}")),
        }
    }

    fn skip_star_rating_dict(&mut self) -> Result<(), String> {
        let count = self.read_i32()?;
        if count < 0 {
            return Err("Negative star-rating pair count in osu!.db".into());
        }
        if (count as usize).saturating_mul(10) > self.remaining() {
            return Err("Unexpected end of osu!.db".into());
        }
        for _ in 0..count {
            let key_tag = self.read_u8()?;
            if key_tag != 0x08 {
                return Err(format!("Unexpected star-rating key tag: 0x{key_tag:02x}"));
            }
            self.skip(4)?; // mods i32
            match self.read_u8()? {
                0x0c => self.skip(4)?, // f32 (v20250107+)
                0x0d => self.skip(8)?, // f64 (v20140609..v20250106)
                other => {
                    return Err(format!("Unexpected star-rating value tag: 0x{other:02x}"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;
    use std::time::Instant;

    fn write_osu_string(out: &mut Vec<u8>, s: &str) {
        if s.is_empty() {
            out.push(0x00);
            return;
        }
        out.push(0x0b);
        let mut len = s.len();
        loop {
            let mut byte = (len & 0x7f) as u8;
            len >>= 7;
            if len != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(s.as_bytes());
    }

    #[allow(clippy::too_many_arguments)]
    fn push_synthetic_beatmap(
        out: &mut Vec<u8>,
        version: i32,
        artist: &str,
        artist_unicode: &str,
        title: &str,
        title_unicode: &str,
        creator: &str,
        diff: &str,
        osu_file: &str,
        set_id: i32,
        mode: u8,
        tags: &str,
        folder: &str,
        use_f32_stars: bool,
    ) {
        if version < 20191106 {
            out.extend_from_slice(&0i32.to_le_bytes());
        }
        write_osu_string(out, artist);
        write_osu_string(out, artist_unicode);
        write_osu_string(out, title);
        write_osu_string(out, title_unicode);
        write_osu_string(out, creator);
        write_osu_string(out, diff);
        write_osu_string(out, "audio.mp3");
        write_osu_string(out, "deadbeef");
        write_osu_string(out, osu_file);
        out.extend_from_slice(&[0u8; 15]); // ranked + counts + last_modified
        if version < 20140609 {
            out.extend_from_slice(&[5u8; 4]);
        } else {
            out.extend_from_slice(&[0u8; 16]);
        }
        out.extend_from_slice(&1.4f64.to_le_bytes()); // slider velocity
        if version >= 20140609 {
            for i in 0..4 {
                if i == 0 {
                    out.extend_from_slice(&1i32.to_le_bytes());
                    out.push(0x08);
                    out.extend_from_slice(&0i32.to_le_bytes());
                    if use_f32_stars {
                        out.push(0x0c);
                        out.extend_from_slice(&5.25f32.to_le_bytes());
                    } else {
                        out.push(0x0d);
                        out.extend_from_slice(&5.25f64.to_le_bytes());
                    }
                } else {
                    out.extend_from_slice(&0i32.to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&[0u8; 12]); // drain/total/preview
        out.extend_from_slice(&1i32.to_le_bytes()); // 1 timing point
        out.extend_from_slice(&[0u8; 17]);
        out.extend_from_slice(&100i32.to_le_bytes()); // beatmap_id
        out.extend_from_slice(&set_id.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes()); // thread_id
        out.extend_from_slice(&[0u8; 10]); // grades + local_offset + stack_leniency
        out.push(mode);
        write_osu_string(out, "source");
        write_osu_string(out, tags);
        out.extend_from_slice(&0i16.to_le_bytes());
        write_osu_string(out, "");
        out.extend_from_slice(&[0u8; 10]); // unplayed + last_played + is_osz2
        write_osu_string(out, folder);
        out.extend_from_slice(&[0u8; 13]);
        if version < 20140609 {
            out.extend_from_slice(&0i16.to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 5]);
    }

    fn build_synthetic_db(version: i32, use_f32_stars: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes()); // folder_count
        out.push(1); // account_unlocked
        out.extend_from_slice(&0i64.to_le_bytes()); // unlock_date
        write_osu_string(&mut out, "Yoge");
        out.extend_from_slice(&5i32.to_le_bytes()); // beatmap_count

        push_synthetic_beatmap(
            &mut out,
            version,
            "Camellia",
            "かめりあ",
            "Ghost",
            "ゴースト",
            "Monstrata",
            "Expert",
            "Camellia - Ghost (Monstrata) [Expert].osu",
            123456,
            0,
            "edm bass tech",
            "123456 Camellia - Ghost",
            use_f32_stars,
        );
        // Duplicate filename in same folder should be deduped.
        push_synthetic_beatmap(
            &mut out,
            version,
            "Camellia",
            "かめりあ",
            "Ghost",
            "ゴースト",
            "Monstrata",
            "Expert",
            "Camellia - Ghost (Monstrata) [Expert].osu",
            123456,
            0,
            "edm bass tech",
            "123456 Camellia - Ghost",
            use_f32_stars,
        );
        push_synthetic_beatmap(
            &mut out,
            version,
            "Camellia",
            "かめりあ",
            "Ghost",
            "ゴースト",
            "Monstrata",
            "Hard",
            "Camellia - Ghost (Monstrata) [Hard].osu",
            123456,
            0,
            "edm bass tech",
            "123456 Camellia - Ghost",
            use_f32_stars,
        );
        // Path traversal filename must be rejected cleanly.
        push_synthetic_beatmap(
            &mut out,
            version,
            "Camellia",
            "",
            "Ghost",
            "",
            "Monstrata",
            "Evil",
            "../evil.osu",
            123456,
            0,
            "",
            "123456 Camellia - Ghost",
            use_f32_stars,
        );
        // Entry whose folder does not exist on disk should be skipped cleanly.
        push_synthetic_beatmap(
            &mut out,
            version,
            "Missing",
            "",
            "Deleted Map",
            "",
            "Nobody",
            "Normal",
            "missing.osu",
            999999,
            1,
            "gone",
            "999999 Missing - Deleted Map",
            use_f32_stars,
        );
        out
    }

    #[test]
    fn parses_synthetic_osu_db_v2021_and_v2025_dedups_and_rejects_traversal() {
        let scratch = Scratch::new("songs-index-db");
        let songs_dir = scratch.path("Songs");
        let existing = songs_dir.join("123456 Camellia - Ghost");
        fs::create_dir_all(&existing).unwrap();

        for (version, f32_stars) in [(20211103, false), (20250107, true)] {
            let db_bytes = build_synthetic_db(version, f32_stars);
            let parsed = parse_osu_db(&db_bytes, &songs_dir).expect("parse synthetic osu!.db");
            assert_eq!(parsed.len(), 1);
            let set = &parsed[0];
            assert_eq!(set.set_id, 123456);
            assert_eq!(set.artist, "Camellia");
            assert_eq!(set.title, "Ghost");
            assert_eq!(set.creator, "Monstrata");
            assert_eq!(set.tags, "edm bass tech");
            assert_eq!(set.diffs.len(), 2);
            assert_eq!(set.diffs[0].version, "Expert");
            assert_eq!(set.diffs[1].version, "Hard");
            assert!(set.diffs[0]
                .path
                .ends_with("Camellia - Ghost (Monstrata) [Expert].osu"));
        }
    }

    #[test]
    fn discovers_osu_db_via_parent_and_beatmap_directory_cfg_and_ignores_unrelated() {
        let scratch = Scratch::new("songs-index-discover");
        let external_songs = scratch.path("DriveD/MySongs");
        fs::create_dir_all(&external_songs).unwrap();

        // Unrelated osu! install pointing elsewhere must be ignored.
        let other_root = scratch.path("OtherOsu");
        let other_songs = scratch.path("OtherOsu/Songs");
        fs::create_dir_all(&other_root).unwrap();
        fs::create_dir_all(&other_songs).unwrap();
        fs::write(other_root.join("osu!.db"), b"other").unwrap();
        fs::write(
            other_root.join("osu!.User.cfg"),
            "BeatmapDirectory = Songs\r\n",
        )
        .unwrap();

        // Matching osu! install pointing to external_songs via quoted BeatmapDirectory with BOM.
        let real_root = scratch.path("AppDataOsu");
        fs::create_dir_all(&real_root).unwrap();
        let real_db = real_root.join("osu!.db");
        fs::write(&real_db, b"real").unwrap();
        fs::write(
            real_root.join("osu!.Yoge.cfg"),
            format!("\u{feff}BeatmapDirectory = \"{}/\"\r\n", external_songs.display()),
        )
        .unwrap();

        let found = discover_osu_db_with_candidates(
            &external_songs,
            &[other_root, real_root.clone()],
        );
        assert_eq!(found.as_deref(), Some(real_db.as_path()));

        // Empty BeatmapDirectory must not falsely match.
        let empty_cfg_root = scratch.path("EmptyCfgOsu");
        fs::create_dir_all(&empty_cfg_root).unwrap();
        fs::write(empty_cfg_root.join("osu!.db"), b"empty").unwrap();
        fs::write(empty_cfg_root.join("osu!.cfg"), "BeatmapDirectory =\n").unwrap();
        assert_eq!(
            discover_osu_db_with_candidates(&external_songs, &[empty_cfg_root]),
            None
        );

        // Parent directory check takes precedence.
        let parent_db = scratch.path("DriveD/osu!.db");
        fs::write(&parent_db, b"parent").unwrap();
        assert_eq!(
            discover_osu_db_with_candidates(&external_songs, &[real_root]),
            Some(parent_db)
        );
    }

    #[test]
    fn header_scanner_handles_section_order_and_non_utf8_legacy_bytes() {
        let scratch = Scratch::new("songs-index-headers");
        let songs_dir = scratch.path("Songs");
        let map_folder = songs_dir.join("42000 xi - Freedom Dive");
        fs::create_dir_all(&map_folder).unwrap();

        // [Difficulty] before [Metadata], [Metadata] before [General], and Shift-JIS/Latin-1 bytes.
        let mut osu_bytes = Vec::new();
        osu_bytes.extend_from_slice(b"osu file format v9\n// comment ");
        osu_bytes.extend_from_slice(&[0x82, 0xa0, 0xff, b'\n']);
        osu_bytes.extend_from_slice(b"[Difficulty]\nHPDrainRate:6\n[Metadata]\n");
        osu_bytes.extend_from_slice("Title:Freedom Dive\nTitleUnicode:フリーダムダイヴ\nArtist:xi\nArtistUnicode:悪\nCreator:Nakagawa-Kanon\nVersion:Four Dimensions\nTags:bms stream\nBeatmapSetID:42000\n".as_bytes());
        osu_bytes.extend_from_slice(b"[General]\nMode: 2\n[HitObjects]\nTitle:Wrong\n");
        fs::write(map_folder.join("xi - Freedom Dive [Four Dimensions].osu"), osu_bytes).unwrap();

        let scanned = scan_songs_folder_fallback(&songs_dir);
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].title, "Freedom Dive");
        assert_eq!(scanned[0].artist, "xi");
        assert_eq!(scanned[0].set_id, 42000);
        assert_eq!(scanned[0].diffs.len(), 1);
        assert_eq!(scanned[0].diffs[0].mode, 2);

        // Also test parallel path (> PARALLEL_THRESHOLD folders).
        for i in 0..20 {
            let dir = songs_dir.join(format!("10{i:02} Parallel - Map {i}"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("p.osu"),
                format!("[General]\nMode: 0\n[Metadata]\nTitle:Map {i}\nArtist:Parallel\nVersion:Normal\nBeatmapSetID:{i}\n"),
            )
            .unwrap();
        }
        let parallel = scan_songs_folder_fallback(&songs_dir);
        assert_eq!(parallel.len(), 21);
    }

    #[test]
    fn fallback_scan_reads_headers_only_and_handles_missing_or_corrupt_db() {
        let scratch = Scratch::new("songs-index-fallback");
        let songs_dir = scratch.path("Songs");
        let map_folder = songs_dir.join("42000 xi - Freedom Dive");
        fs::create_dir_all(&map_folder).unwrap();

        let osu_a = "\
osu file format v14

[General]
AudioFilename: audio.mp3
Mode: 0

[Metadata]
Title:Freedom Dive
Artist:xi
Creator:Nakagawa-Kanon
Version:Four Dimensions
Tags:bms stream classic
BeatmapSetID:42000

[HitObjects]
this should never be parsed as metadata:
Title:WrongTitle
";
        let osu_b = "\
osu file format v14

[General]
Mode: 1

[Metadata]
Title:Freedom Dive
Artist:xi
Creator:Nakagawa-Kanon
Version:Another
Tags:bms stream classic
BeatmapSetID:42000
";
        fs::write(map_folder.join("xi - Freedom Dive [Four Dimensions].osu"), osu_a).unwrap();
        fs::write(map_folder.join("xi - Freedom Dive [Another].osu"), osu_b).unwrap();

        // Write a corrupt osu!.db next to Songs to verify graceful fallback.
        fs::write(scratch.path("osu!.db"), b"not a valid osu db").unwrap();

        let (index, source) = build_songs_index_with_source(&songs_dir);
        assert_eq!(source, "folder-scan");
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].title, "Freedom Dive");
        assert_eq!(index[0].artist, "xi");
        assert_eq!(index[0].creator, "Nakagawa-Kanon");
        assert_eq!(index[0].set_id, 42000);
        assert_eq!(index[0].diffs.len(), 2);
        assert_eq!(index[0].diffs[0].version, "Another");
        assert_eq!(index[0].diffs[0].mode, 1);
        assert_eq!(index[0].diffs[1].version, "Four Dimensions");
        assert_eq!(index[0].diffs[1].mode, 0);
    }

    #[test]
    fn build_songs_index_prefers_osu_db_when_valid() {
        let scratch = Scratch::new("songs-index-prefers-db");
        let songs_dir = scratch.path("Songs");
        fs::create_dir_all(songs_dir.join("123456 Camellia - Ghost")).unwrap();
        fs::write(scratch.path("osu!.db"), build_synthetic_db(20250107, true)).unwrap();

        let (index, source) = build_songs_index_with_source(&songs_dir);
        assert_eq!(source, "osu!.db");
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].artist, "Camellia");
    }

    #[test]
    fn sidx_cache_persists_nested_tracks_empty_folders_recovers_corrupt_and_supports_cjk() {
        let scratch = Scratch::new("songs-index-sidx");
        let songs_dir = scratch.path("Songs");
        let map_dir = songs_dir.join("123456 Camellia - Ghost");
        let empty_dir = songs_dir.join("Failed");
        fs::create_dir_all(&map_dir).unwrap();
        fs::create_dir_all(&empty_dir).unwrap();
        fs::write(
            map_dir.join("Camellia - Ghost [Expert].osu"),
            "[General]\nMode:0\n[Metadata]\nArtist:Camellia\nArtistUnicode:かめりあ\nTitle:Ghost\nTitleUnicode:ゴースト\nCreator:Monstrata\nVersion:Expert\nBeatmapSetID:123456\n",
        )
        .unwrap();

        let nested_cache = scratch.path("nested/app/data");

        // 1. Cold scan creates nested cache_root and persists both mapset and 0-diff `Failed/`.
        let first = load_or_build_songs_index(&songs_dir, &nested_cache, false);
        assert_eq!(first.source, "folder-scan");
        assert_eq!(first.mapsets.len(), 1);
        assert!(first.empty_folders.contains_key("failed"));
        assert!(cache_file_path(&nested_cache, &songs_dir).is_file());
        // Unicode CJK query matches in `SongsIndex::search`.
        assert_eq!(first.search("かめりあ ゴースト", 10).len(), 1);

        // 2. Warm load uses `.sidx`, detects added & deleted folders without re-reading unchanged ones.
        let added = songs_dir.join("777 New - Song");
        fs::create_dir_all(&added).unwrap();
        fs::write(
            added.join("New - Song [Hard].osu"),
            "[Metadata]\nArtist:New\nTitle:Song\nVersion:Hard\n",
        )
        .unwrap();
        let warm = load_or_build_songs_index(&songs_dir, &nested_cache, false);
        assert_eq!(warm.source, "cache");
        assert_eq!(warm.mapsets.len(), 2);

        // Removing all `.osu` files inside `777 New - Song` moves it out of `mapsets`.
        std::thread::sleep(Duration::from_millis(30));
        fs::remove_file(added.join("New - Song [Hard].osu")).unwrap();
        let after_del = load_or_build_songs_index(&songs_dir, &nested_cache, false);
        assert_eq!(after_del.source, "cache");
        assert_eq!(after_del.mapsets.len(), 1);

        // 3. Corrupt `.sidx` with absurd counts must not OOM and recovers cleanly.
        let cache_file = cache_file_path(&nested_cache, &songs_dir);
        let mut bad = Vec::new();
        bad.extend_from_slice(CACHE_MAGIC);
        bad.extend_from_slice(&CACHE_VERSION.to_le_bytes());
        write_cache_string(&mut bad, &clean_path_for_compare(&songs_dir));
        bad.push(SOURCE_FOLDER_SCAN);
        bad.push(0); // no db
        bad.extend_from_slice(&0i64.to_le_bytes());
        bad.extend_from_slice(&0u32.to_le_bytes());
        bad.extend_from_slice(&u32::MAX.to_le_bytes()); // huge entry count
        fs::write(&cache_file, bad).unwrap();

        let recovered = load_or_build_songs_index(&songs_dir, &nested_cache, false);
        assert_eq!(recovered.source, "folder-scan");
        assert_eq!(recovered.mapsets.len(), 1);
    }

    #[test]
    fn corrupt_osu_db_rescues_from_sidx_without_repeat_fail_and_refresh_escapes() {
        let scratch = Scratch::new("songs-index-rescue");
        let songs_dir = scratch.path("Songs");
        let map_dir = songs_dir.join("123456 Camellia - Ghost");
        fs::create_dir_all(&map_dir).unwrap();
        fs::write(
            map_dir.join("Camellia - Ghost [Expert].osu"),
            "[Metadata]\nArtist:Camellia\nTitle:Ghost\nVersion:Expert\n",
        )
        .unwrap();

        let cache_root = scratch.path("data");
        // Seed valid `.sidx` while DB is absent.
        let seeded = load_or_build_songs_index(&songs_dir, &cache_root, false);
        assert_eq!(seeded.source, "folder-scan");

        // Now add a corrupt `osu!.db` AND add a new `.osu` folder.
        fs::write(scratch.path("osu!.db"), b"corrupt binary payload").unwrap();
        let extra = songs_dir.join("999 Extra - Track");
        fs::create_dir_all(&extra).unwrap();
        fs::write(
            extra.join("Extra.osu"),
            "[Metadata]\nArtist:Extra\nTitle:Track\nVersion:Insane\n",
        )
        .unwrap();

        let rescued = load_or_build_songs_index(&songs_dir, &cache_root, false);
        assert_eq!(rescued.source, "cache");
        assert_eq!(rescued.mapsets.len(), 2);
        // C3: live observed fingerprint matches even though `osu!.db` is corrupt, so `is_fresh` is true!
        assert!(rescued.is_fresh());

        // `force_refresh = true` must NOT serve `CacheRescue`; it runs `FullScan` (`"folder-scan"`).
        let refreshed = load_or_build_songs_index(&songs_dir, &cache_root, true);
        assert_eq!(refreshed.source, "folder-scan");
        assert_eq!(refreshed.mapsets.len(), 2);
        assert!(refreshed.is_fresh());
    }

    #[test]
    fn parse_db_preserves_locally_patched_hs_diff_after_osu_db_rewrite() {
        let scratch = Scratch::new("songs-index-m3-preserve");
        let songs_dir = scratch.path("Songs");
        let map_dir = songs_dir.join("123456 Camellia - Ghost");
        fs::create_dir_all(&map_dir).unwrap();
        fs::write(
            map_dir.join("Camellia - Ghost (Monstrata) [Expert].osu"),
            "[Metadata]\nArtist:Camellia\nTitle:Ghost\nCreator:Monstrata\nVersion:Expert\nBeatmapSetID:123456\n",
        )
        .unwrap();
        fs::write(
            map_dir.join("Camellia - Ghost (Monstrata) [Hard].osu"),
            "[Metadata]\nArtist:Camellia\nTitle:Ghost\nCreator:Monstrata\nVersion:Hard\nBeatmapSetID:123456\n",
        )
        .unwrap();
        fs::write(scratch.path("osu!.db"), build_synthetic_db(20250107, true)).unwrap();

        let cache_root = scratch.path("data");
        let mut idx = load_or_build_songs_index(&songs_dir, &cache_root, false);
        assert_eq!(idx.source, "osu!.db");
        assert_eq!(idx.mapsets[0].entry.diffs.len(), 2);

        // User creates a [Hitsounds] diff inside `dawsu!` and patches `.sidx` (`from_osu_db = false`).
        fs::write(
            map_dir.join("Camellia - Ghost (Monstrata) [Hitsounds].osu"),
            "[Metadata]\nArtist:Camellia\nTitle:Ghost\nCreator:Monstrata\nVersion:Hitsounds\nBeatmapSetID:123456\n",
        )
        .unwrap();
        assert!(idx.patch_folder(&map_dir));
        assert_eq!(idx.mapsets[0].entry.diffs.len(), 3);
        idx.save_to_cache(&cache_root).unwrap();

        // Out-of-Songs patch must be rejected cleanly.
        let outside = scratch.path("OutsideFolder");
        fs::create_dir_all(&outside).unwrap();
        assert!(!idx.patch_folder(&outside));

        // Simulate osu!stable rewriting `osu!.db` slightly later (newer mtime) without scanning `[Hitsounds]`.
        std::thread::sleep(Duration::from_millis(25));
        let db2 = build_synthetic_db(20250107, true);
        fs::write(scratch.path("osu!.db"), &db2).unwrap();

        let reloaded = load_or_build_songs_index(&songs_dir, &cache_root, false);
        assert_eq!(reloaded.source, "osu!.db");
        assert_eq!(reloaded.mapsets[0].entry.diffs.len(), 3);
        assert!(reloaded.mapsets[0]
            .entry
            .diffs
            .iter()
            .any(|d| d.version == "Hitsounds"));
    }

    #[test]
    fn pre_1970_timestamps_order_monotonically() {
        let t_earlier = UNIX_EPOCH - Duration::from_millis(1200);
        let t_later = UNIX_EPOCH - Duration::from_millis(200);
        let t_epoch = UNIX_EPOCH;
        let t_after = UNIX_EPOCH + Duration::from_millis(200);
        assert!(system_time_stamp(t_earlier) < system_time_stamp(t_later));
        assert!(system_time_stamp(t_later) < system_time_stamp(t_epoch));
        assert!(system_time_stamp(t_epoch) < system_time_stamp(t_after));
    }

    #[test]
    fn fast_5000_mapset_in_memory_db_parse_sidx_and_search_under_one_second() {
        const MAPSETS: usize = 5_000;
        const DIFFS_PER_SET: usize = 5;

        let mut db = Vec::with_capacity(4 * 1024 * 1024);
        db.extend_from_slice(&20250107i32.to_le_bytes());
        db.extend_from_slice(&(MAPSETS as i32).to_le_bytes());
        db.push(1);
        db.extend_from_slice(&0i64.to_le_bytes());
        write_osu_string(&mut db, "BenchUser");
        db.extend_from_slice(&((MAPSETS * DIFFS_PER_SET) as i32).to_le_bytes());

        let mut folders = Vec::with_capacity(MAPSETS);
        for m in 0..MAPSETS {
            let folder = format!("{m} Artist{m} - SongTitle{m}");
            let artist = format!("Artist{m}");
            let title = format!("SongTitle{m}");
            let creator = format!("Mapper{}", m % 200);
            for d in 0..DIFFS_PER_SET {
                let diff = match d {
                    0 => "Easy",
                    1 => "Normal",
                    2 => "Hard",
                    3 => "Insane",
                    _ => "Expert",
                };
                let file = format!("Artist{m} - SongTitle{m} [{diff}].osu");
                push_synthetic_beatmap(
                    &mut db,
                    20250107,
                    &artist,
                    "",
                    &title,
                    "",
                    &creator,
                    diff,
                    &file,
                    m as i32 + 1,
                    0,
                    "electronic rhythm benchmark",
                    &folder,
                    true,
                );
            }
            folders.push((folder, (1_700_000_000i64, m as u32)));
        }

        let songs_dir = PathBuf::from("BenchSongs");
        let start_parse = Instant::now();
        let entries = parse_osu_db_with_folders(&db, &songs_dir, &folders).unwrap();
        let parse_ms = start_parse.elapsed();
        assert_eq!(entries.len(), MAPSETS);

        let records: Vec<FolderRecord> = entries
            .into_iter()
            .map(|e| FolderRecord {
                folder: e.folder,
                folder_mtime: (1_700_000_000, 0),
                from_osu_db: true,
                set_id: e.set_id,
                artist: e.artist,
                title: e.title,
                creator: e.creator,
                tags: e.tags,
                unicode_terms: String::new(),
                diffs: e
                    .diffs
                    .into_iter()
                    .map(|d| CachedDiff {
                        filename: Path::new(&d.path)
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                        version: d.version,
                        mode: d.mode,
                    })
                    .collect(),
            })
            .collect();

        let idx = finalize_songs_index(
            &songs_dir,
            "osu!.db",
            SOURCE_OSU_DB,
            SongsFingerprint {
                songs_dir: songs_dir.clone(),
                songs_dir_mtime: (1_700_000_000, 0),
                osu_db: None,
            },
            None,
            records,
        );
        let encoded = encode_cache_bytes(&idx);
        let start_decode = Instant::now();
        let decoded = decode_cache_bytes(&encoded, &songs_dir).unwrap();
        let decode_ms = start_decode.elapsed();
        assert_eq!(decoded.records.len(), MAPSETS);

        let start_search = Instant::now();
        let found = idx.search("Artist4321 Expert", 50);
        let search_ms = start_search.elapsed();
        assert_eq!(found.len(), 1);

        assert!(
            parse_ms < Duration::from_millis(1_000),
            "5k parse took {parse_ms:?}"
        );
        assert!(
            decode_ms < Duration::from_millis(500),
            "5k sidx decode took {decode_ms:?}"
        );
        assert!(
            search_ms < Duration::from_millis(50),
            "5k search took {search_ms:?}"
        );
    }

    #[test]
    #[ignore]
    fn songs_index_bench_5000() {
        const MAPSETS: usize = 5_000;
        const DIFFS_PER_SET: usize = 5;

        let scratch = Scratch::new("songs-bench-5000");
        let songs_dir = scratch.path("Songs");
        let cache_root = scratch.path("Cache");
        fs::create_dir_all(&songs_dir).unwrap();

        let mut db = Vec::with_capacity(4 * 1024 * 1024);
        db.extend_from_slice(&20250107i32.to_le_bytes());
        db.extend_from_slice(&(MAPSETS as i32).to_le_bytes());
        db.push(1);
        db.extend_from_slice(&0i64.to_le_bytes());
        write_osu_string(&mut db, "BenchUser");
        db.extend_from_slice(&((MAPSETS * DIFFS_PER_SET) as i32).to_le_bytes());

        for m in 0..MAPSETS {
            let folder = format!("{m} Artist{m} - SongTitle{m}");
            fs::create_dir_all(songs_dir.join(&folder)).unwrap();
            for d in 0..DIFFS_PER_SET {
                let diff = ["Easy", "Normal", "Hard", "Insane", "Expert"][d];
                let file = format!("Artist{m} - SongTitle{m} [{diff}].osu");
                push_synthetic_beatmap(
                    &mut db,
                    20250107,
                    &format!("Artist{m}"),
                    "",
                    &format!("SongTitle{m}"),
                    "",
                    "Mapper",
                    diff,
                    &file,
                    m as i32 + 1,
                    0,
                    "bench",
                    &folder,
                    true,
                );
            }
        }
        fs::write(scratch.path("osu!.db"), &db).unwrap();

        let t_cold_db = Instant::now();
        let cold_db = load_or_build_songs_index(&songs_dir, &cache_root, false);
        let cold_db_elapsed = t_cold_db.elapsed();
        assert_eq!(cold_db.source, "osu!.db");
        assert_eq!(cold_db.mapsets.len(), MAPSETS);

        // Warm load with `osu!.db` removed -> tests 5,000-folder `.sidx` + mtime reconciliation.
        fs::remove_file(scratch.path("osu!.db")).unwrap();
        let t_warm_cache = Instant::now();
        let warm_cache = load_or_build_songs_index(&songs_dir, &cache_root, false);
        let warm_cache_elapsed = t_warm_cache.elapsed();
        assert_eq!(warm_cache.source, "cache");
        assert_eq!(warm_cache.mapsets.len(), MAPSETS);

        println!(
            "songs_index_bench_5000: cold_osu_db={cold_db_elapsed:?}, warm_sidx_no_db={warm_cache_elapsed:?}"
        );
        assert!(
            cold_db_elapsed < Duration::from_millis(1_000),
            "cold osu!.db took {cold_db_elapsed:?}"
        );
        assert!(
            warm_cache_elapsed < Duration::from_millis(1_000),
            "warm .sidx without osu!.db took {warm_cache_elapsed:?}"
        );
    }

    #[test]
    fn multi_word_search_matches_artist_title_mapper_diff_tags_and_set_id() {
        let index = vec![
            SongMapsetEntry {
                set_id: 123456,
                artist: "Camellia".into(),
                title: "Ghost".into(),
                creator: "Monstrata".into(),
                tags: "edm bass tech".into(),
                folder: "123456 Camellia - Ghost".into(),
                diffs: vec![SongDiffEntry {
                    version: "Expert".into(),
                    path: "Songs/123456 Camellia - Ghost/expert.osu".into(),
                    mode: 0,
                }],
            },
            SongMapsetEntry {
                set_id: 42000,
                artist: "xi".into(),
                title: "Freedom Dive".into(),
                creator: "Nakagawa-Kanon".into(),
                tags: "bms stream classic".into(),
                folder: "42000 xi - Freedom Dive".into(),
                diffs: vec![SongDiffEntry {
                    version: "Four Dimensions".into(),
                    path: "Songs/42000 xi - Freedom Dive/fd.osu".into(),
                    mode: 0,
                }],
            },
        ];

        assert_eq!(search_songs_index(&index, "", 10).len(), 2);
        assert_eq!(search_songs_index(&index, "camellia expert", 10).len(), 1);
        assert_eq!(search_songs_index(&index, "monstrata tech", 10).len(), 1);
        assert_eq!(search_songs_index(&index, "42000 dimensions", 10).len(), 1);
        assert_eq!(search_songs_index(&index, "xi kanon bms", 10).len(), 1);
        assert_eq!(search_songs_index(&index, "camellia freedom", 10).len(), 0);
        assert_eq!(search_songs_index(&index, "", 1).len(), 1);
    }
}
