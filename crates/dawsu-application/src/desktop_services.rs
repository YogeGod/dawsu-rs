use crate::Application;
use crate::bounce_plan::{OszBudgetSummary, calculate_osz_budget};
use crate::sample_fingerprint::SampleFingerprintIndex;
use crate::songs_index::{SongsIndex, SongsSearchResult, build_songs_index_from_cache};
use dawsu_audio::{OutputBackend, OutputDevice};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

/// Bars in the Sample Browser preview outline.
pub const PREVIEW_BARS: usize = 40;
const MAX_INDEXED_SAMPLES: usize = 4_000;

/// Shared predicate identifying a `[Hitsounds]`-style difficulty version or empty hit-object chart.
pub fn is_hs_version_name(version: &str, has_objects: bool) -> bool {
    let lower = version.trim().to_ascii_lowercase();
    lower.contains("hitsound")
        || lower == "hs"
        || lower.ends_with(" hs")
        || lower.contains("[hs]")
        || (!has_objects && !lower.is_empty())
}

/// Fast header scan determining whether a `.osu` file is a hitsounding difficulty.
pub fn is_osu_hs_diff(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let mut section = "";
    let mut version = String::new();
    let mut has_objects = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = &line[1..line.len() - 1];
            continue;
        }
        if section.eq_ignore_ascii_case("Metadata") {
            if let Some(rest) = line.strip_prefix("Version:") {
                version = rest.trim().to_string();
            }
        } else if section.eq_ignore_ascii_case("HitObjects")
            && !line.is_empty()
            && !line.starts_with("//")
        {
            has_objects = true;
            break;
        }
    }
    is_hs_version_name(&version, has_objects)
}

fn is_supported_bg_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"
            )
        })
}

/// Resolves a relative beatmap background path inside `folder`, rejecting `..` or
/// rooted paths and falling back to case-insensitive matching on case-sensitive OSes.
fn resolve_safe_folder_image(folder: &Path, raw_rel: &str) -> Option<PathBuf> {
    let cleaned = raw_rel.trim().trim_matches('"').trim();
    if cleaned.is_empty() {
        return None;
    }
    let normalized = cleaned.replace('\\', "/");
    let rel_path = Path::new(&normalized);
    let mut parts: Vec<&std::ffi::OsStr> = Vec::new();
    for comp in rel_path.components() {
        match comp {
            std::path::Component::Normal(seg) => parts.push(seg),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut exact = folder.to_path_buf();
    for seg in &parts {
        exact.push(seg);
    }
    if exact.is_file() && is_supported_bg_extension(&exact) {
        return Some(exact);
    }
    // Case-insensitive walk for macOS/Linux mapsets whose `.osu` casing differs from disk.
    let mut cursor = folder.to_path_buf();
    for seg in &parts {
        let wanted = seg.to_string_lossy().to_ascii_lowercase();
        let next = std::fs::read_dir(&cursor).ok()?.flatten().find_map(|entry| {
            if entry.file_name().to_string_lossy().to_ascii_lowercase() == wanted {
                Some(entry.path())
            } else {
                None
            }
        })?;
        cursor = next;
    }
    (cursor.is_file() && is_supported_bg_extension(&cursor)).then_some(cursor)
}

/// Extracts the mapset background path from a `.osu` `[Events]` section following
/// osu!lazer's `LegacyBeatmapDecoder` rules (`0`/`Background`, `1`/`Video` when
/// pointing at an image, and Background-layer `4`/`Sprite` fallback).
pub fn resolve_osu_background_path(osu_path: &Path) -> Option<PathBuf> {
    use std::io::{BufRead, BufReader};
    let folder = osu_path.parent()?;
    let file = std::fs::File::open(osu_path).ok()?;
    let mut reader = BufReader::new(file);
    let mut section = String::new();
    let mut sprite_fallback: Option<PathBuf> = None;
    let mut line = String::new();

    while reader.read_line(&mut line).ok()? > 0 {
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section.clear();
            section.push_str(&trimmed[1..trimmed.len() - 1]);
            if section.eq_ignore_ascii_case("TimingPoints")
                || section.eq_ignore_ascii_case("HitObjects")
            {
                break;
            }
            line.clear();
            continue;
        }
        if section.eq_ignore_ascii_case("Events")
            && !trimmed.is_empty()
            && !trimmed.starts_with("//")
            && !trimmed.starts_with(' ')
            && !trimmed.starts_with('_')
        {
            let parts: Vec<&str> = trimmed.split(',').collect();
            let event_type = parts.first().map(|s| s.trim()).unwrap_or("");
            if (event_type == "0" || event_type.eq_ignore_ascii_case("Background"))
                && let Some(raw) = parts.get(2)
                && let Some(found) = resolve_safe_folder_image(folder, raw)
            {
                return Some(found);
            }
            if (event_type == "1" || event_type.eq_ignore_ascii_case("Video"))
                && let Some(raw) = parts.get(2)
                && let Some(found) = resolve_safe_folder_image(folder, raw)
            {
                return Some(found);
            }
            if sprite_fallback.is_none()
                && (event_type == "4" || event_type.eq_ignore_ascii_case("Sprite"))
                && parts.get(1).is_some_and(|layer| {
                    let l = layer.trim();
                    l == "0" || l.eq_ignore_ascii_case("Background")
                })
                && let Some(raw) = parts.get(3)
                && let Some(found) = resolve_safe_folder_image(folder, raw)
            {
                sprite_fallback = Some(found);
            }
        }
        line.clear();
    }
    sprite_fallback
}

/// Instance-owned cache for resolving a mapset's background image path across sibling `.osu` files.
#[derive(Default)]
pub struct BackgroundPathCache {
    folders: Mutex<HashMap<PathBuf, Option<PathBuf>>>,
}

impl BackgroundPathCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.folders.lock() {
            guard.clear();
        }
    }

    pub fn resolve(&self, osu_path: &Path) -> Option<PathBuf> {
        let folder = osu_path.parent()?.to_path_buf();
        if let Ok(guard) = self.folders.lock()
            && let Some(cached) = guard.get(&folder)
            && cached.as_ref().is_none_or(|p| p.is_file())
        {
            return cached.clone();
        }

        let mut resolved = resolve_osu_background_path(osu_path);
        if resolved.is_none()
            && let Ok(entries) = std::fs::read_dir(&folder)
        {
            let mut checked = 0usize;
            for entry in entries.flatten() {
                if checked >= 8 {
                    break;
                }
                let sibling = entry.path();
                if sibling != osu_path
                    && sibling
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("osu"))
                {
                    checked += 1;
                    if let Some(found) = resolve_osu_background_path(&sibling) {
                        resolved = Some(found);
                        break;
                    }
                }
            }
        }

        if let Ok(mut guard) = self.folders.lock() {
            if guard.len() >= 1024 {
                guard.clear();
            }
            guard.insert(folder, resolved.clone());
        }
        resolved
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapsetInspectEntry {
    pub path: PathBuf,
    pub name: String,
    pub version: String,
    pub file_name: String,
    pub is_hs: bool,
    pub mode: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MapsetInspectResult {
    Open {
        path: PathBuf,
    },
    Choose {
        folder: PathBuf,
        artist: String,
        title: String,
        creator: String,
        difficulties: Vec<MapsetInspectEntry>,
    },
}

pub fn inspect_mapset_folder(path: &Path) -> Result<MapsetInspectResult, String> {
    if path.is_file() {
        return Ok(MapsetInspectResult::Open {
            path: path.to_path_buf(),
        });
    }
    if !path.is_dir() {
        return Err("That folder does not exist.".into());
    }
    let mut diffs: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("osu")))
        .collect();
    diffs.sort();
    if diffs.is_empty() {
        return Err("This folder contains no .osu difficulty files.".into());
    }
    let mut artist = String::new();
    let mut title = String::new();
    let mut creator = String::new();
    let entries: Vec<MapsetInspectEntry> = diffs
        .iter()
        .map(|p| {
            let file_name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let fallback_name = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let mut version = String::new();
            let mut mode: u8 = 0;
            if let Ok(bytes) = std::fs::read(p) {
                let text = String::from_utf8_lossy(&bytes);
                let mut section = "";
                for raw in text.lines() {
                    let line = raw.trim().trim_start_matches('\u{feff}');
                    if line.starts_with('[') && line.ends_with(']') {
                        section = &line[1..line.len() - 1];
                        if section.eq_ignore_ascii_case("HitObjects")
                            || section.eq_ignore_ascii_case("TimingPoints")
                        {
                            break;
                        }
                        continue;
                    }
                    if section.eq_ignore_ascii_case("General") {
                        if let Some(rest) = line.strip_prefix("Mode:")
                            && let Ok(parsed) = rest.trim().parse::<u8>()
                        {
                            mode = parsed;
                        }
                    } else if section.eq_ignore_ascii_case("Metadata") {
                        if artist.is_empty()
                            && let Some(rest) = line.strip_prefix("Artist:")
                        {
                            artist = rest.trim().to_string();
                        } else if title.is_empty()
                            && let Some(rest) = line.strip_prefix("Title:")
                        {
                            title = rest.trim().to_string();
                        } else if creator.is_empty()
                            && let Some(rest) = line.strip_prefix("Creator:")
                        {
                            creator = rest.trim().to_string();
                        } else if version.is_empty()
                            && let Some(rest) = line.strip_prefix("Version:")
                        {
                            version = rest.trim().to_string();
                        }
                    }
                }
            }
            let version = if version.is_empty() {
                fallback_name.clone()
            } else {
                version
            };
            let is_hs = is_osu_hs_diff(p);
            MapsetInspectEntry {
                path: p.clone(),
                name: fallback_name,
                version,
                file_name,
                is_hs,
                mode,
            }
        })
        .collect();
    Ok(MapsetInspectResult::Choose {
        folder: path.to_path_buf(),
        artist,
        title,
        creator,
        difficulties: entries,
    })
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewSource {
    Lane { row: usize },
    Library { location: crate::library::Location },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SampleOutline {
    pub ms: f64,
    pub peaks: Vec<f32>,
}

/// The Sample Browser's preview outline: length in ms and bar peaks.
pub fn sample_outline(app: &mut Application, source: PreviewSource) -> Result<SampleOutline, String> {
    let (ms, peaks) = match source {
        PreviewSource::Lane { row } => {
            let lane = app
                .document()
                .and_then(|d| d.composer.lanes.get(row))
                .ok_or("No such lane.")?;
            app.lane_sample_outline(lane, PREVIEW_BARS)
                .ok_or("The sample isn't loaded.")?
        }
        PreviewSource::Library { location } => {
            let sample = location.sample(app.preferences().hitsound_library.as_deref())?;
            app.sample_file_outline(&sample, PREVIEW_BARS)?
        }
    };
    Ok(SampleOutline { ms, peaks })
}

#[derive(Default)]
struct CacheState {
    ready: Option<Arc<SongsIndex>>,
    ready_gen: u64,
    generation: u64,
    in_flight: Option<(u64, PathBuf)>,
}

/// Thread-safe, generation-guarded single-flight cache for the Start Screen Songs index.
#[derive(Clone)]
pub struct SongsIndexCache {
    data_root: PathBuf,
    inner: Arc<(Mutex<CacheState>, Condvar)>,
    write_lock: Arc<Mutex<()>>,
}

struct BuildGuard<'a> {
    inner: &'a (Mutex<CacheState>, Condvar),
    my_gen: u64,
}

impl Drop for BuildGuard<'_> {
    fn drop(&mut self) {
        let Ok(mut state) = self.inner.0.lock() else {
            self.inner.1.notify_all();
            return;
        };
        if state
            .in_flight
            .as_ref()
            .is_some_and(|(g, _)| *g == self.my_gen)
        {
            state.in_flight = None;
        }
        self.inner.1.notify_all();
    }
}

impl SongsIndexCache {
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            inner: Arc::new((Mutex::new(CacheState::default()), Condvar::new())),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn invalidate(&self) {
        let mut state = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.ready = None;
        state.ready_gen = 0;
        state.in_flight = None;
        self.inner.1.notify_all();
    }

    pub fn warm_up(&self, songs_dir: PathBuf) {
        if !songs_dir.is_dir() {
            return;
        }
        let cache = self.clone();
        std::thread::spawn(move || {
            let _ = cache.entries_for(&songs_dir);
        });
    }

    pub fn entries_for(&self, songs_dir: &Path) -> Arc<SongsIndex> {
        self.get_or_build_with(songs_dir, false, build_songs_index_from_cache)
    }

    pub fn search(
        &self,
        songs_dir: &Path,
        query: &str,
        limit: usize,
        refresh: bool,
    ) -> SongsSearchResult {
        self.search_with(songs_dir, query, limit, refresh, build_songs_index_from_cache)
    }

    fn search_with<F>(
        &self,
        songs_dir: &Path,
        query: &str,
        limit: usize,
        refresh: bool,
        builder: F,
    ) -> SongsSearchResult
    where
        F: FnOnce(&Path, &Path, bool) -> (SongsIndex, bool),
    {
        let index = self.get_or_build_with(songs_dir, refresh, builder);
        SongsSearchResult {
            total_mapsets: index.mapsets.len(),
            total_diffs: index.total_diffs,
            source: index.source,
            matches: index.search(query, limit),
        }
    }

    fn get_or_build_with<F>(&self, songs_dir: &Path, refresh: bool, builder: F) -> Arc<SongsIndex>
    where
        F: FnOnce(&Path, &Path, bool) -> (SongsIndex, bool),
    {
        if !refresh {
            let candidate = {
                let state = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .ready
                    .as_ref()
                    .filter(|idx| idx.songs_dir == songs_dir)
                    .cloned()
            };
            if let Some(idx) = candidate
                && idx.is_fresh()
            {
                return idx;
            }
        }

        let mut waited_for_gen: Option<u64> = None;
        let (my_gen, _guard) = loop {
            let mut state = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            if !refresh
                && let Some(expected) = waited_for_gen
                && state.ready_gen >= expected
                && let Some(idx) = state
                    .ready
                    .as_ref()
                    .filter(|idx| idx.songs_dir == songs_dir)
            {
                return Arc::clone(idx);
            }
            if !refresh
                && let Some((in_flight_gen, ref dir)) = state.in_flight
                && dir == songs_dir
            {
                waited_for_gen = Some(in_flight_gen);
                state = self.inner.1.wait(state).unwrap_or_else(|e| e.into_inner());
                continue;
            }
            state.generation = state.generation.wrapping_add(1);
            let my_gen = state.generation;
            state.in_flight = Some((my_gen, songs_dir.to_path_buf()));
            break (
                my_gen,
                BuildGuard {
                    inner: &self.inner,
                    my_gen,
                },
            );
        };

        let (index, needs_save) = builder(songs_dir, &self.data_root, refresh);
        let arc = Arc::new(index);
        let should_save = {
            let mut state = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            if state.generation != my_gen {
                return Arc::clone(&arc);
            }
            state.ready = Some(Arc::clone(&arc));
            state.ready_gen = my_gen;
            if state
                .in_flight
                .as_ref()
                .is_some_and(|(g, _)| *g == my_gen)
            {
                state.in_flight = None;
            }
            self.inner.1.notify_all();
            needs_save
        };
        if should_save {
            self.save_if_current(&arc, my_gen);
        }
        arc
    }

    pub fn patch_mapset_folder(&self, songs_dir: &Path, mapset_folder: &Path) -> bool {
        self.patch_mapset_folder_with(songs_dir, mapset_folder, |idx, folder| {
            idx.patch_folder(folder)
        })
    }

    fn patch_mapset_folder_with<F>(
        &self,
        songs_dir: &Path,
        mapset_folder: &Path,
        patcher: F,
    ) -> bool
    where
        F: FnOnce(&mut SongsIndex, &Path) -> bool,
    {
        let (base, base_gen) = {
            let mut state = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            let matching = state
                .ready
                .as_ref()
                .filter(|idx| idx.songs_dir == songs_dir)
                .cloned();
            let Some(base) = matching else {
                if state
                    .in_flight
                    .as_ref()
                    .is_some_and(|(_, dir)| dir == songs_dir)
                {
                    state.generation = state.generation.wrapping_add(1);
                    state.in_flight = None;
                    self.inner.1.notify_all();
                }
                return false;
            };
            (base, state.generation)
        };
        let mut updated = (*base).clone();
        if !patcher(&mut updated, mapset_folder) {
            return false;
        }
        let updated_arc = Arc::new(updated);
        let my_gen = {
            let mut state = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
            if state.generation != base_gen
                || state
                    .ready
                    .as_ref()
                    .is_none_or(|r| r.songs_dir != songs_dir)
            {
                return false;
            }
            state.generation = state.generation.wrapping_add(1);
            let g = state.generation;
            state.ready = Some(Arc::clone(&updated_arc));
            state.ready_gen = g;
            state.in_flight = None;
            self.inner.1.notify_all();
            g
        };
        let cache_clone = self.clone();
        std::thread::spawn(move || {
            cache_clone.save_if_current(&updated_arc, my_gen);
        });
        true
    }

    fn save_if_current(&self, index: &SongsIndex, expected_gen: u64) {
        let _write_guard = self.write_lock.lock().unwrap_or_else(|e| e.into_inner());
        let current_gen = self
            .inner
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation;
        if current_gen == expected_gen {
            let _ = index.save_to_cache(&self.data_root);
        }
    }
}

#[derive(Clone)]
pub struct SampleFingerprintCache {
    cache_dir: PathBuf,
    inner: Arc<Mutex<FingerprintCacheState>>,
}

#[derive(Default)]
struct FingerprintCacheState {
    indices: HashMap<PathBuf, Arc<SampleFingerprintIndex>>,
    scanning: HashSet<PathBuf>,
}

impl SampleFingerprintCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            inner: Arc::new(Mutex::new(FingerprintCacheState::default())),
        }
    }

    pub fn invalidate(&self, root: &Path) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.indices.remove(root);
    }

    pub fn snapshot(&self, root: &Path) -> (Option<Arc<SampleFingerprintIndex>>, bool) {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let idx = state.indices.get(root).cloned();
        let scanning = state.scanning.contains(root);
        (idx, scanning)
    }

    pub fn ensure_scan(&self, root: PathBuf) -> bool {
        {
            let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if state.scanning.contains(&root) {
                return true;
            }
            if state.indices.contains_key(&root) {
                return false;
            }
            state.scanning.insert(root.clone());
        }
        let cache = self.clone();
        std::thread::Builder::new()
            .name("dawsu-fp-scan".into())
            .spawn(move || {
                let idx = Arc::new(SampleFingerprintIndex::load_or_update(
                    &root,
                    &cache.cache_dir,
                    MAX_INDEXED_SAMPLES,
                ));
                let mut state = cache.inner.lock().unwrap_or_else(|e| e.into_inner());
                state.indices.insert(root.clone(), idx);
                state.scanning.remove(&root);
            })
            .ok();
        true
    }
}

/// Cached per-lane hit counts and target difficulty list for Export Review views.
#[derive(Default)]
pub struct ExportCache {
    /// Path, load or Save, and the mapset's difficulties.
    pub targets_for: Option<(PathBuf, u64, Vec<PathBuf>)>,
    pub targets: Vec<PathBuf>,
    /// Path, load or Save, history version and lane count.
    pub counts_for: Option<(PathBuf, u64, u64, usize)>,
    /// Hits per lane, in lane order.
    pub counts: Vec<usize>,
    /// How often either was read, for tests.
    pub reads: usize,
}

impl ExportCache {
    pub fn refresh_lane_counts(&mut self, app: &Application) -> Option<&[usize]> {
        let d = app.document()?;
        let generation = app.document_generation();
        let key = (
            d.path.clone(),
            generation,
            d.composer.history.version(),
            d.composer.lanes.len(),
        );
        if self.counts_for.as_ref() != Some(&key) {
            let mut hit_counts = HashMap::new();
            for lane in d
                .composer
                .hitsounds()
                .moments()
                .iter()
                .flat_map(|moment| moment.hits())
            {
                *hit_counts.entry(lane).or_insert(0_usize) += 1;
            }
            self.counts = d
                .composer
                .lanes
                .iter()
                .map(|lane| hit_counts.get(lane).copied().unwrap_or(0))
                .collect();
            self.counts_for = Some(key);
            self.reads += 1;
        }
        Some(&self.counts)
    }

    pub fn refresh_targets(&mut self, app: &Application) -> &[PathBuf] {
        match app.document() {
            Some(d) => {
                let generation = app.document_generation();
                let fresh = self.targets_for.as_ref().is_some_and(|(path, read, list)| {
                    *path == d.path && *read == generation && *list == d.difficulties
                });
                if !fresh {
                    self.targets = app.export_targets();
                    self.targets_for = Some((d.path.clone(), generation, d.difficulties.clone()));
                    self.reads += 1;
                }
                &self.targets
            }
            None => &[],
        }
    }
}

pub fn export_title_from_parts(title: Option<&str>, difficulty: Option<&str>) -> String {
    match (title, difficulty) {
        (Some(title), Some(difficulty)) if !title.ends_with(&format!("[{difficulty}]")) => {
            format!("Export · {title} [{difficulty}]")
        }
        (Some(title), _) => format!("Export · {title}"),
        _ => "Export".into(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysoundDedupReport {
    pub source_notes: usize,
    pub unique_samples: usize,
    pub dedup_percent: u8,
    pub budget: OszBudgetSummary,
}

pub fn keysounds_dedup_report(
    source_notes: usize,
    unique_samples: usize,
    base_bytes: u64,
    bounced_bytes: u64,
) -> KeysoundDedupReport {
    let dedup_percent = if source_notes == 0 {
        0
    } else {
        (((source_notes.saturating_sub(unique_samples)) as f64 / source_notes as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    };
    KeysoundDedupReport {
        source_notes,
        unique_samples,
        dedup_percent,
        budget: calculate_osz_budget(base_bytes, bounced_bytes),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioBackendOption {
    pub id: OutputBackend,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOutputSettingsSnapshot {
    pub backend: OutputBackend,
    pub available: Vec<AudioBackendOption>,
    pub current: String,
    pub current_backend: Option<&'static str>,
    pub devices: Vec<OutputDevice>,
    pub device: Option<String>,
    pub buffer_ms: Option<u32>,
    pub effective_buffer_ms: Option<u32>,
    pub error: Option<String>,
}

pub fn audio_output_settings(
    app: &Application,
    requested: Option<OutputBackend>,
) -> AudioOutputSettingsSnapshot {
    let available = OutputBackend::available();
    let prefs = app.preferences();
    let preferred = prefs.audio_backend;
    let backend = requested.unwrap_or_else(|| {
        if available.contains(&preferred) {
            preferred
        } else {
            OutputBackend::System
        }
    });
    let (mut devices, error) = match app.output_devices(backend) {
        Ok(devices) => (devices, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let qualified_id = (preferred == backend)
        .then_some(prefs.audio_device_id.as_deref())
        .flatten();
    let device = if let Some(id) = qualified_id {
        devices.iter().find(|device| device.id == id)
    } else if backend == OutputBackend::System {
        prefs
            .device
            .as_ref()
            .and_then(|name| devices.iter().find(|device| &device.name == name))
    } else {
        None
    };

    AudioOutputSettingsSnapshot {
        backend,
        available: available
            .iter()
            .map(|choice| AudioBackendOption {
                id: *choice,
                label: choice.label(),
            })
            .collect(),
        current: app.device_name().to_owned(),
        current_backend: app.active_output_backend().map(OutputBackend::label),
        device: device.map(|d| d.id.clone()),
        devices,
        buffer_ms: prefs.audio_buffer_ms,
        effective_buffer_ms: app.effective_output_buffer_ms(),
        error,
    }
}

#[cfg(test)]
mod background_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dawsu_bg_{label}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolves_quoted_unquoted_video_image_and_sibling_backgrounds() {
        let dir = temp_dir("formats");
        let bg_jpg = dir.join("cover.jpg");
        std::fs::write(&bg_jpg, b"\xFF\xD8\xFF\xE0fakejpg").unwrap();

        let diff_a = dir.join("a.osu");
        std::fs::write(&diff_a, "[Events]\n0,0,\"cover.jpg\",0,0\n[TimingPoints]\n").unwrap();
        assert_eq!(resolve_osu_background_path(&diff_a), Some(bg_jpg.clone()));

        let diff_b = dir.join("b.osu");
        std::fs::write(&diff_b, "[Events]\n1,0,\"cover.jpg\"\n[HitObjects]\n").unwrap();
        assert_eq!(resolve_osu_background_path(&diff_b), Some(bg_jpg.clone()));

        let diff_hs = dir.join("hs.osu");
        std::fs::write(&diff_hs, "[Events]\n// no bg\n[TimingPoints]\n").unwrap();
        assert_eq!(resolve_osu_background_path(&diff_hs), None);
        let cache = BackgroundPathCache::new();
        assert_eq!(cache.resolve(&diff_hs), Some(bg_jpg.clone()));

        let diff_bad = dir.join("bad.osu");
        std::fs::write(&diff_bad, "[Events]\n0,0,\"../secret.png\",0,0\n").unwrap();
        assert_eq!(resolve_osu_background_path(&diff_bad), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod songs_index_cache_tests {
    use super::*;
    use crate::songs_index::{
        SongDiffEntry, SongMapsetEntry, SongsIndex, build_songs_index_from_cache,
        load_or_build_songs_index,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Barrier, mpsc};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn scratch_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dawsu_sidx_cache_{label}_{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn single_flight_coalesces_concurrent_cold_searches_to_one_build() {
        let root = scratch_dir("single-flight");
        let songs = root.join("Songs");
        let map_dir = songs.join("1 Artist - Title");
        std::fs::create_dir_all(&map_dir).unwrap();
        std::fs::write(
            map_dir.join("Map.osu"),
            "[Metadata]\nArtist:Artist\nTitle:Title\nVersion:Hard\n",
        )
        .unwrap();

        let cache = SongsIndexCache::new(root.join("data"));
        let builds = Arc::new(AtomicUsize::new(0));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));

        let c1 = cache.clone();
        let s1 = songs.clone();
        let b1 = Arc::clone(&builds);
        let entered1 = Arc::clone(&entered);
        let release1 = Arc::clone(&release);
        let t1 = std::thread::spawn(move || {
            c1.search_with(&s1, "", 50, false, |dir, data, force| {
                b1.fetch_add(1, Ordering::SeqCst);
                entered1.wait();
                release1.wait();
                build_songs_index_from_cache(dir, data, force)
            })
        });

        entered.wait();
        let c2 = cache.clone();
        let s2 = songs.clone();
        let b2 = Arc::clone(&builds);
        let t2 = std::thread::spawn(move || {
            c2.search_with(&s2, "", 50, false, |dir, data, force| {
                b2.fetch_add(1, Ordering::SeqCst);
                build_songs_index_from_cache(dir, data, force)
            })
        });

        std::thread::sleep(Duration::from_millis(30));
        release.wait();

        let r1 = t1.join().unwrap();
        let r2 = t2.join().unwrap();
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(r1.total_mapsets, 1);
        assert_eq!(r2.total_mapsets, 1);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn panic_during_build_releases_waiters_and_allows_recovery() {
        let root = scratch_dir("panic-recover");
        let songs = root.join("Songs");
        let map_dir = songs.join("1 Artist - Title");
        std::fs::create_dir_all(&map_dir).unwrap();
        std::fs::write(
            map_dir.join("Map.osu"),
            "[Metadata]\nArtist:Artist\nTitle:Title\nVersion:Hard\n",
        )
        .unwrap();

        let cache = SongsIndexCache::new(root.join("data"));
        let entered = Arc::new(Barrier::new(2));
        let release_panic = Arc::new(Barrier::new(2));

        let c1 = cache.clone();
        let s1 = songs.clone();
        let e1 = Arc::clone(&entered);
        let r1 = Arc::clone(&release_panic);
        let t1 = std::thread::spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                c1.search_with(&s1, "", 50, false, |_dir, _data, _force| -> (SongsIndex, bool) {
                    e1.wait();
                    r1.wait();
                    panic!("simulated scan panic");
                })
            }));
        });

        entered.wait();
        let (done_tx, done_rx) = mpsc::channel();
        let c2 = cache.clone();
        let s2 = songs.clone();
        let t2 = std::thread::spawn(move || {
            let res = c2.search(&s2, "", 50, false);
            let _ = done_tx.send(res);
        });

        std::thread::sleep(Duration::from_millis(30));
        release_panic.wait();

        let recovered = done_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("waiter must not deadlock after builder panic");
        assert_eq!(recovered.total_mapsets, 1);
        t1.join().unwrap();
        t2.join().unwrap();

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn invalidate_mid_build_discards_stale_ram_and_disk_commit() {
        let root = scratch_dir("invalidate-mid");
        let songs_a = root.join("SongsA");
        let songs_b = root.join("SongsB");
        let data = root.join("data");
        std::fs::create_dir_all(songs_a.join("1 A - Song")).unwrap();
        std::fs::write(
            songs_a.join("1 A - Song/a.osu"),
            "[Metadata]\nArtist:OldA\nTitle:SongA\nVersion:Normal\n",
        )
        .unwrap();
        std::fs::create_dir_all(songs_b.join("2 B - Song")).unwrap();
        std::fs::write(
            songs_b.join("2 B - Song/b.osu"),
            "[Metadata]\nArtist:NewB\nTitle:SongB\nVersion:Hard\n",
        )
        .unwrap();

        let cache = SongsIndexCache::new(data.clone());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));

        let c1 = cache.clone();
        let sa = songs_a.clone();
        let e1 = Arc::clone(&entered);
        let r1 = Arc::clone(&release);
        let t1 = std::thread::spawn(move || {
            c1.search_with(&sa, "", 50, false, |dir, d, force| {
                e1.wait();
                r1.wait();
                build_songs_index_from_cache(dir, d, force)
            })
        });

        entered.wait();
        cache.invalidate();
        release.wait();
        let _ = t1.join().unwrap();

        {
            let guard = cache.inner.0.lock().unwrap();
            assert!(guard.ready.is_none());
        }
        let entries_in_cache_dir = std::fs::read_dir(data.join("songs-index"))
            .map(|rd| rd.count())
            .unwrap_or(0);
        assert_eq!(entries_in_cache_dir, 0);

        let b_res = cache.search(&songs_b, "", 50, false);
        assert_eq!(b_res.total_mapsets, 1);
        assert_eq!(b_res.matches[0].artist, "NewB");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn patch_mapset_folder_and_auto_reload_work_and_ignore_outside_paths() {
        let root = scratch_dir("patch-and-reload");
        let songs = root.join("Songs");
        let map_dir = songs.join("10 Artist - Track");
        let outside = root.join("DesktopMap");
        let data = root.join("data");
        std::fs::create_dir_all(&map_dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(
            map_dir.join("Track [Hard].osu"),
            "[Metadata]\nArtist:Artist\nTitle:Track\nVersion:Hard\n",
        )
        .unwrap();
        std::fs::write(
            outside.join("Out.osu"),
            "[Metadata]\nArtist:Out\nTitle:Side\nVersion:Normal\n",
        )
        .unwrap();

        let cache = SongsIndexCache::new(data.clone());
        let initial = cache.search(&songs, "", 50, false);
        assert_eq!(initial.total_diffs, 1);

        assert!(!cache.patch_mapset_folder(&songs, &outside));

        std::fs::write(
            map_dir.join("Track [Hitsounds].osu"),
            "[Metadata]\nArtist:Artist\nTitle:Track\nVersion:Hitsounds\n",
        )
        .unwrap();
        assert!(cache.patch_mapset_folder(&songs, &map_dir));

        let updated = cache.search(&songs, "Hitsounds", 50, false);
        assert_eq!(updated.total_diffs, 2);
        assert_eq!(updated.matches.len(), 1);

        std::thread::sleep(Duration::from_millis(80));
        let disk = load_or_build_songs_index(&songs, &data, false);
        assert_eq!(disk.total_diffs, 2);

        std::thread::sleep(Duration::from_millis(25));
        let added = songs.join("20 Another - Song");
        std::fs::create_dir_all(&added).unwrap();
        std::fs::write(
            added.join("Another.osu"),
            "[Metadata]\nArtist:Another\nTitle:Song\nVersion:Normal\n",
        )
        .unwrap();
        let reloaded = cache.search(&songs, "", 50, false);
        assert_eq!(reloaded.total_mapsets, 2);

        let _: fn(&[SongMapsetEntry], &Path) -> Option<PathBuf> =
            crate::songs_index::find_relink_candidate;
        let _ = SongDiffEntry {
            version: String::new(),
            path: String::new(),
            mode: 0,
        };
        let _ = SongMapsetEntry {
            set_id: -1,
            artist: String::new(),
            title: String::new(),
            creator: String::new(),
            tags: String::new(),
            folder: String::new(),
            diffs: Vec::new(),
        };

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn patch_mapset_folder_discards_stale_commit_when_invalidated_mid_patch() {
        let root = scratch_dir("patch-mid-invalidate");
        let songs = root.join("Songs");
        let map_dir = songs.join("10 Artist - Track");
        let data = root.join("data");
        std::fs::create_dir_all(&map_dir).unwrap();
        std::fs::write(
            map_dir.join("Track [Hard].osu"),
            "[Metadata]\nArtist:Artist\nTitle:Track\nVersion:Hard\n",
        )
        .unwrap();

        let cache = SongsIndexCache::new(data.clone());
        let seeded = cache.search(&songs, "", 50, false);
        assert_eq!(seeded.total_diffs, 1);

        let _ = std::fs::remove_dir_all(data.join("songs-index"));
        std::fs::write(
            map_dir.join("Track [Hitsounds].osu"),
            "[Metadata]\nArtist:Artist\nTitle:Track\nVersion:Hitsounds\n",
        )
        .unwrap();

        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let c1 = cache.clone();
        let s1 = songs.clone();
        let m1 = map_dir.clone();
        let e1 = Arc::clone(&entered);
        let r1 = Arc::clone(&release);
        let t1 = std::thread::spawn(move || {
            c1.patch_mapset_folder_with(&s1, &m1, |idx, folder| {
                e1.wait();
                r1.wait();
                idx.patch_folder(folder)
            })
        });

        entered.wait();
        cache.invalidate();
        release.wait();
        assert!(!t1.join().unwrap());

        {
            let guard = cache.inner.0.lock().unwrap();
            assert!(guard.ready.is_none());
        }
        std::thread::sleep(Duration::from_millis(50));
        let entries_in_cache_dir = std::fs::read_dir(data.join("songs-index"))
            .map(|rd| rd.count())
            .unwrap_or(0);
        assert_eq!(entries_in_cache_dir, 0);

        let _ = std::fs::remove_dir_all(&root);
    }
}
