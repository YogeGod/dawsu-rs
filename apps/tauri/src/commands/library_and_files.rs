use dawsu_application::Application;
use dawsu_application::sample_fingerprint::{
    MAX_QUERY_RESULTS, SampleFingerprintIndex,
};
use dawsu_application::songs_index::{
    SongsIndex, SongsSearchResult, build_songs_index_from_cache,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tauri::State;

use crate::action::{Action, SampleTarget};
use crate::scene::{RecentMetaSnapshot, ViewResponse};
use crate::{Editor, Request, ask, ask_within, folders, library, skin_textures};

#[tauri::command]
pub(crate) fn pick_map(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
    archive_only: Option<bool>,
) -> Result<ViewResponse, String> {
    let dialog = songs_dialog(&window, &state)?;
    let selected = if archive_only == Some(true) {
        dialog.set_title("Import an .osz")
            .add_filter("osu! beatmap archive", &["osz"])
            .pick_file()
            .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("osz")))
    } else {
        dialog.add_filter("osu! difficulty or mapset", &["osu", "osz"]).pick_file()
    };
    ask(&state, |tx| {
        Request::View(true, selected.map(|path| Action::Open { path }), false, None, tx)
    })
}

pub(crate) fn is_osu_hs_diff(path: &Path) -> bool {
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
                version = rest.trim().to_ascii_lowercase();
            }
        } else if section.eq_ignore_ascii_case("HitObjects") && !line.is_empty() && !line.starts_with("//") {
            has_objects = true;
            break;
        }
    }
    version.contains("hitsound")
        || version == "hs"
        || version.ends_with(" hs")
        || version.contains("[hs]")
        || (!has_objects && !version.is_empty())
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
pub(crate) fn resolve_osu_background_path(osu_path: &Path) -> Option<PathBuf> {
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
                && parts
                    .get(1)
                    .is_some_and(|layer| {
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

/// Resolves a mapset's background image path (checking `osu_path` first, then sibling
/// `.osu` files in the same folder if e.g. `osu_path` is a blank `[Hitsounds]` diff),
/// with a small in-memory folder cache.
pub(crate) fn resolve_mapset_background_path(osu_path: &Path) -> Option<PathBuf> {
    type BgCache = std::collections::HashMap<PathBuf, Option<PathBuf>>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<BgCache>> = std::sync::OnceLock::new();
    let folder = osu_path.parent()?.to_path_buf();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = cache.lock()
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

    if let Ok(mut guard) = cache.lock() {
        if guard.len() >= 1024 {
            guard.clear();
        }
        guard.insert(folder, resolved.clone());
    }
    resolved
}

pub(crate) fn recent_osu_meta(path: &Path) -> (Option<String>, bool) {
    let is_hs = is_osu_hs_diff(path);
    let background = resolve_mapset_background_path(path)
        .as_deref()
        .and_then(skin_textures::image_data_url);
    (background, is_hs)
}

pub(crate) fn cached_recent_meta(path: &Path) -> RecentMetaSnapshot {
    type RecentMetaCache = std::collections::HashMap<PathBuf, (Option<String>, bool)>;
    static CACHE: std::sync::OnceLock<std::sync::Mutex<RecentMetaCache>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = cache.lock()
        && let Some((background, is_hs)) = guard.get(path) {
            return RecentMetaSnapshot {
                path: path.to_path_buf(),
                background: background.clone(),
                is_hs: *is_hs,
            };
        }
    let (background, is_hs) = recent_osu_meta(path);
    if let Ok(mut guard) = cache.lock() {
        if guard.len() >= 64 {
            guard.clear();
        }
        guard.insert(path.to_path_buf(), (background.clone(), is_hs));
    }
    RecentMetaSnapshot {
        path: path.to_path_buf(),
        background,
        is_hs,
    }
}

#[tauri::command]
pub(crate) fn pick_map_folder(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<Option<PathBuf>, String> {
    let dialog = songs_dialog(&window, &state)?;
    Ok(dialog.set_title("Open mapset folder").pick_folder())
}

#[tauri::command]
pub(crate) fn pick_map_for_hitsound_diff(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<PathBuf>, String> {
    let dialog = songs_dialog(&window, &state)?;
    Ok(dialog
        .set_title("Pick a difficulty in the mapset")
        .add_filter("osu! beatmap difficulty", &["osu"])
        .pick_file())
}

#[tauri::command]
pub(crate) fn inspect_mapset_folder(path: PathBuf) -> Result<Value, String> {
    if path.is_file() {
        return Ok(json!({ "kind": "open", "path": path }));
    }
    if !path.is_dir() {
        return Err("That folder does not exist.".into());
    }
    let mut diffs: Vec<PathBuf> = std::fs::read_dir(&path)
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
    let entries: Vec<Value> = diffs
        .iter()
        .map(|p| {
            let file_name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
            let fallback_name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
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
            let version = if version.is_empty() { fallback_name.clone() } else { version };
            let is_hs = is_osu_hs_diff(p);
            json!({
                "path": p,
                "name": fallback_name,
                "version": version,
                "fileName": file_name,
                "isHs": is_hs,
                "mode": mode,
            })
        })
        .collect();
    Ok(json!({
        "kind": "choose",
        "folder": path,
        "artist": artist,
        "title": title,
        "creator": creator,
        "difficulties": entries,
    }))
}

#[tauri::command]
pub(crate) fn create_hitsound_diff(
    sibling_path: PathBuf,
    reuse_existing: Option<bool>,
    starter_mode: Option<String>,
    state: State<'_, Editor>,
    cache: State<'_, SongsIndexCache>,
) -> Result<PathBuf, String> {
    let parsed_mode = match starter_mode.as_deref() {
        None | Some("") => None,
        Some("custom_only") => Some(dawsu_application::HsStarterMode::CustomOnly),
        Some("default_12") => Some(dawsu_application::HsStarterMode::Default12),
        Some(other) => return Err(format!("Unknown starter mode: {other}")),
    };
    let created = ask(&state, |tx| {
        Request::CreateHitsoundDiff(sibling_path, reuse_existing.unwrap_or(false), parsed_mode, tx)
    })??;
    if let Ok(Some(songs_dir)) = ask(&state, Request::SongsDirectory)
        && let Some(folder) = created.parent()
    {
        cache.patch_mapset_folder(&songs_dir, folder);
    }
    Ok(created)
}

#[tauri::command]
pub(crate) fn create_hitsound_diff_from_current(
    state: State<'_, Editor>,
    cache: State<'_, SongsIndexCache>,
) -> Result<PathBuf, String> {
    let created = ask(&state, Request::CreateHitsoundDiffFromCurrent)??;
    if let Ok(Some(songs_dir)) = ask(&state, Request::SongsDirectory)
        && let Some(folder) = created.parent()
    {
        cache.patch_mapset_folder(&songs_dir, folder);
    }
    Ok(created)
}

#[tauri::command]
pub(crate) fn pick_lane_sample(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
    map: PathBuf,
) -> Result<ViewResponse, String> {
    let mut dialog = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Add sample as lane")
        .add_filter("Audio sample", &["wav", "ogg", "mp3"]);
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let selected = dialog.pick_file();
    ask(&state, |tx| {
        Request::View(
            true,
            selected.map(|path| Action::AddLaneFromFile { map, path }),
            false,
            None,
            tx,
        )
    })
}

#[tauri::command]
pub(crate) fn pick_sample(window: tauri::WebviewWindow, state: State<'_, Editor>, target: SampleTarget) -> Result<ViewResponse, String> {
    let mut dialog = rfd::FileDialog::new().set_parent(&window)
        .set_title("Choose replacement sample")
        .add_filter("Audio sample", &["wav", "ogg", "mp3"]);
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let selected = dialog.pick_file();
    ask(&state, |tx| Request::View(true, selected.map(|path| Action::ReplaceSample { target, path, mapset: false }), false, None, tx))
}

#[tauri::command]
pub(crate) fn pick_drum_stem(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<ViewResponse, String> {
    let mut dialog = rfd::FileDialog::new().set_parent(&window)
        .set_title("Attach drum stem")
        .add_filter("Audio", &["wav", "ogg", "mp3", "flac"]);
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let selected = dialog.pick_file();
    ask(&state, |tx| Request::View(true, selected.map(|path| Action::LoadDrumStem { path }), false, None, tx))
}

/// The library list for Edit > Place sound set: names, ids and counts only.
pub(crate) fn sound_set_library_view(app: &Application) -> Value {
    let library = match app.sound_set_library() {
        Ok(library) => library,
        Err(error) => return json!({ "configured": true, "error": error, "sets": [], "unreadable": [], "reading": false }),
    };
    json!({
        "configured": library.configured,
        "sets": library.sets.iter().map(|set| json!({
            "id": set.id, "name": set.name, "sounds": set.sounds, "inMap": set.in_map, "duplicate": set.duplicate,
            "missing": set.missing,
        })).collect::<Vec<_>>(),
        "unreadable": library.unreadable,
        "reading": library.reading,
    })
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SoundSetAction {
    Library,
    Placement { map: PathBuf, id: u64 },
    Save { map: PathBuf, name: String },
}

/// What Edit > Sound set placement shows of one placement. Reads the library, so it is asked
/// for when the dialog opens, never on the poll.
pub(crate) fn sound_set_placement_view(app: &Application, map: &Path, id: u64) -> Result<Value, String> {
    if app.document().is_none_or(|d| d.path != map) {
        return Err("The difficulty changed.".into());
    }
    let view = app.sound_set_placement(id)?;
    Ok(json!({
        "id": view.id, "setId": view.set_id, "name": view.name, "edited": view.edited, "sounds": view.sounds,
        "gain": view.gain, "frozen": view.frozen, "newer": view.newer,
    }))
}

/// Saving reads every selected sample and the library, so it gets its own longer wait, off the
/// window's thread; a 7 s timeout while the editor still saves would make a retry look like a
/// name clash.
#[tauri::command]
pub(crate) async fn sound_set_action(
    state: State<'_, Editor>,
    action: SoundSetAction,
) -> Result<Value, String> {
    match action {
        SoundSetAction::Save { .. } => {
            let editor = state.0.clone();
            tauri::async_runtime::spawn_blocking(move || {
                ask_within(&editor, Duration::from_secs(120), |tx| {
                    Request::SoundSet(action, tx)
                })
            })
            .await
            .map_err(|e| e.to_string())??
        }
        other => ask(&state, |tx| Request::SoundSet(other, tx))?,
    }
}

#[tauri::command]
pub(crate) fn pick_sound_set(window: tauri::WebviewWindow, state: State<'_, Editor>, map: PathBuf) -> Result<ViewResponse, String> {
    let selected = rfd::FileDialog::new().set_parent(&window)
        .set_title("Place sound set")
        .add_filter("dawsu! sound set", &["dawsuset"])
        .pick_file();
    ask(&state, |tx| Request::View(true, selected.map(|path| Action::PlaceSoundSet { map, path }), false, None, tx))
}

pub(crate) fn songs_dialog(window: &tauri::WebviewWindow, state: &Editor) -> Result<rfd::FileDialog, String> {
    let dialog = rfd::FileDialog::new().set_parent(window);
    Ok(match ask(state, Request::SongsDirectory)? {
        Some(folder) => dialog.set_directory(folder),
        None => dialog,
    })
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
pub(crate) struct SongsIndexCache {
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
    pub(crate) fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            inner: Arc::new((Mutex::new(CacheState::default()), Condvar::new())),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn invalidate(&self) {
        let mut state = self.inner.0.lock().unwrap_or_else(|e| e.into_inner());
        state.generation = state.generation.wrapping_add(1);
        state.ready = None;
        state.ready_gen = 0;
        state.in_flight = None;
        self.inner.1.notify_all();
    }

    pub(crate) fn warm_up(&self, songs_dir: PathBuf) {
        if !songs_dir.is_dir() {
            return;
        }
        let cache = self.clone();
        std::thread::spawn(move || {
            let _ = cache.entries_for(&songs_dir);
        });
    }

    pub(crate) fn entries_for(&self, songs_dir: &Path) -> Arc<SongsIndex> {
        self.get_or_build_with(songs_dir, false, build_songs_index_from_cache)
    }

    pub(crate) fn search(
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

    fn get_or_build_with<F>(
        &self,
        songs_dir: &Path,
        refresh: bool,
        builder: F,
    ) -> Arc<SongsIndex>
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

    pub(crate) fn patch_mapset_folder(&self, songs_dir: &Path, mapset_folder: &Path) -> bool {
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

#[tauri::command]
pub(crate) async fn songs_index_search(
    query: String,
    refresh: bool,
    state: State<'_, Editor>,
    cache: State<'_, SongsIndexCache>,
) -> Result<SongsSearchResult, String> {
    let Some(songs_dir) = ask(&state, Request::SongsDirectory)? else {
        return Ok(SongsSearchResult {
            total_mapsets: 0,
            total_diffs: 0,
            source: "none",
            matches: Vec::new(),
        });
    };
    let cache = cache.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        const SEARCH_LIMIT: usize = 50;
        Ok(cache.search(&songs_dir, &query, SEARCH_LIMIT, refresh))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn mapset_background(osu_path: PathBuf) -> Result<tauri::ipc::Response, String> {
    const MAX_BG_BYTES: u64 = 12 * 1024 * 1024;
    if !osu_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("osu"))
    {
        return Ok(tauri::ipc::Response::new(Vec::new()));
    }
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let Some(bg_path) = resolve_mapset_background_path(&osu_path) else {
            return Vec::new();
        };
        let Ok(meta) = std::fs::metadata(&bg_path) else {
            return Vec::new();
        };
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_BG_BYTES {
            return Vec::new();
        }
        std::fs::read(&bg_path).unwrap_or_default()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub(crate) fn pick_songs_folder(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<ViewResponse>, String> {
    let Some(folder) = songs_dialog(&window, &state)?.set_title("Choose Songs folder").pick_folder() else {
        return Ok(None);
    };
    ask(&state, |tx| Request::SetSongsDirectory(folder, tx))?.map(Some)
}

#[tauri::command]
pub(crate) fn pick_hitsound_library(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<ViewResponse>, String> {
    let mut dialog = rfd::FileDialog::new().set_parent(&window).set_title("Choose hitsound library folder");
    if let Some(folder) = ask(&state, Request::LibraryDirectory)? {
        dialog = dialog.set_directory(folder);
    }
    let Some(folder) = dialog.pick_folder() else { return Ok(None) };
    ask(&state, |tx| Request::SetLibraryDirectory(folder, tx))?.map(Some)
}

#[tauri::command]
pub(crate) fn list_library(location: library::Location, state: State<'_, Editor>) -> Result<Vec<(String, bool)>, String> {
    ask(&state, |tx| Request::LibraryEntries(location, tx))?
}

/// Bars in the Browser's sample preview outline.
pub(crate) const PREVIEW_BARS: usize = 40;

/// The sample a Browser row stands for: a map folder sample by the lane that plays it, or a
/// library file.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PreviewSource {
    Lane { row: usize },
    Library { location: library::Location },
}

/// The Browser's sample preview: the sample's length in ms and its outline.
pub(crate) fn sample_outline(app: &mut Application, source: PreviewSource) -> Result<Value, String> {
    let (ms, peaks) = match source {
        PreviewSource::Lane { row } => {
            let lane = app.document().and_then(|d| d.composer.lanes.get(row)).ok_or("No such lane.")?;
            app.lane_sample_outline(lane, PREVIEW_BARS).ok_or("The sample isn't loaded.")?
        }
        PreviewSource::Library { location } => {
            let sample = location.sample(app.preferences().hitsound_library.as_deref())?;
            app.sample_file_outline(&sample, PREVIEW_BARS)?
        }
    };
    Ok(json!({ "ms": ms, "peaks": peaks }))
}

#[tauri::command]
pub(crate) fn sample_preview(source: PreviewSource, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::SamplePreview(source, tx))?
}

#[tauri::command]
pub(crate) fn lane_add_candidates(map: PathBuf, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::LaneAddCandidates(map, tx))?
}

#[tauri::command]
pub(crate) fn map_preview_objects(map: PathBuf, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::MapPreview(map, tx))?
}


#[tauri::command]
pub(crate) fn skin_textures() -> Value {
    skin_textures::load_skin_textures_json()
}

#[tauri::command]
pub(crate) fn preview_skins(state: State<'_, Editor>) -> Result<Value, String> {
    let songs = ask(&state, Request::SongsDirectory)?;
    Ok(skin_textures::list_available_skins(songs.as_deref()))
}

#[tauri::command]
pub(crate) fn preview_skin_load(skin_id: String) -> Result<Value, String> {
    if skin_id == "argon" || skin_id.is_empty() {
        return Ok(Value::Null);
    }
    if skin_id == "classic" {
        return Ok(skin_textures::load_skin_textures_json());
    }
    if let Some(dir) = skin_id.strip_prefix("dir:") {
        return skin_textures::load_skin_from_dir(Path::new(dir));
    }
    Err(format!("Unknown preview skin: {skin_id}"))
}

#[tauri::command]
pub(crate) fn pick_skin_folder(window: tauri::WebviewWindow) -> Option<String> {
    rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Choose osu! skin folder")
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned())
}

const MAX_INDEXED_SAMPLES: usize = 4_000;

#[derive(Clone)]
pub(crate) struct SampleFingerprintCache {
    cache_dir: PathBuf,
    inner: Arc<Mutex<FingerprintCacheState>>,
}

#[derive(Default)]
struct FingerprintCacheState {
    indices: HashMap<PathBuf, Arc<SampleFingerprintIndex>>,
    scanning: HashSet<PathBuf>,
}

impl SampleFingerprintCache {
    pub(crate) fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            inner: Arc::new(Mutex::new(FingerprintCacheState::default())),
        }
    }

    pub(crate) fn snapshot(&self, root: &Path) -> (Option<Arc<SampleFingerprintIndex>>, bool) {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let idx = state.indices.get(root).cloned();
        let scanning = state.scanning.contains(root);
        (idx, scanning)
    }

    pub(crate) fn ensure_scan(&self, root: PathBuf) -> bool {
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

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SampleLibraryAction {
    Status {
        source: String,
    },
    Scan {
        source: String,
        force: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    Query {
        source: String,
        tag: Option<String>,
        similar_to: Option<String>,
        filter: Option<String>,
        limit: Option<usize>,
    },
}

fn resolve_sample_root(state: &State<'_, Editor>, source: &str) -> Result<Option<PathBuf>, String> {
    if source == "library" {
        ask(state, Request::LibraryDirectory)
    } else {
        Ok(ask(state, Request::CurrentDocumentPath)?
            .and_then(|p| p.parent().map(Path::to_path_buf)))
    }
}

#[tauri::command]
pub(crate) async fn sample_library_action(
    action: SampleLibraryAction,
    state: State<'_, Editor>,
    fp_cache: State<'_, SampleFingerprintCache>,
) -> Result<Value, String> {
    let source = match &action {
        SampleLibraryAction::Status { source }
        | SampleLibraryAction::Scan { source, .. }
        | SampleLibraryAction::Query { source, .. } => source.as_str(),
    };
    let Some(root) = resolve_sample_root(&state, source)? else {
        return Ok(json!({
            "scanning": false,
            "totalIndexed": 0,
            "tags": [],
            "entries": [],
        }));
    };
    match action {
        SampleLibraryAction::Status { .. } => {
            let (idx, scanning) = fp_cache.snapshot(&root);
            Ok(json!({
                "scanning": scanning,
                "totalIndexed": idx.as_ref().map_or(0, |i| i.len()),
            }))
        }
        SampleLibraryAction::Scan { force, .. } => {
            if force == Some(true) {
                let mut guard = fp_cache.inner.lock().unwrap_or_else(|e| e.into_inner());
                guard.indices.remove(&root);
            }
            let scanning = fp_cache.ensure_scan(root.clone());
            let (idx, still_scanning) = fp_cache.snapshot(&root);
            Ok(json!({
                "scanning": scanning || still_scanning,
                "totalIndexed": idx.as_ref().map_or(0, |i| i.len()),
            }))
        }
        SampleLibraryAction::Query {
            tag,
            similar_to,
            filter,
            limit,
            ..
        } => {
            let (idx_opt, scanning_now) = fp_cache.snapshot(&root);
            let idx = if let Some(idx) = idx_opt {
                idx
            } else {
                let is_scanning = fp_cache.ensure_scan(root.clone());
                return Ok(json!({
                    "scanning": scanning_now || is_scanning,
                    "totalIndexed": 0,
                    "tags": [],
                    "entries": [],
                }));
            };
            let result = idx.query(
                tag.as_deref(),
                similar_to.as_deref(),
                filter.as_deref(),
                limit.unwrap_or(MAX_QUERY_RESULTS),
            );
            Ok(json!({
                "scanning": scanning_now,
                "totalIndexed": result.total_indexed,
                "tags": result.tags,
                "entries": result.entries,
            }))
        }
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

        // Standard quoted 0,0,"cover.jpg",0,0
        let diff_a = dir.join("a.osu");
        std::fs::write(&diff_a, "[Events]\n0,0,\"cover.jpg\",0,0\n[TimingPoints]\n").unwrap();
        assert_eq!(resolve_osu_background_path(&diff_a), Some(bg_jpg.clone()));

        // Unquoted 0,0,cover.jpg and image-in-Video (1,0,"cover.jpg")
        let diff_b = dir.join("b.osu");
        std::fs::write(&diff_b, "[Events]\n1,0,\"cover.jpg\"\n[HitObjects]\n").unwrap();
        assert_eq!(resolve_osu_background_path(&diff_b), Some(bg_jpg.clone()));

        // Blank [Hitsounds] diff with no background falls back to sibling diff in same folder
        let diff_hs = dir.join("hs.osu");
        std::fs::write(&diff_hs, "[Events]\n// no bg\n[TimingPoints]\n").unwrap();
        assert_eq!(resolve_osu_background_path(&diff_hs), None);
        assert_eq!(resolve_mapset_background_path(&diff_hs), Some(bg_jpg.clone()));

        // Path traversal (..) is rejected
        let diff_bad = dir.join("bad.osu");
        std::fs::write(&diff_bad, "[Events]\n0,0,\"../secret.png\",0,0\n").unwrap();
        assert_eq!(resolve_osu_background_path(&diff_bad), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod songs_index_cache_tests {
    use super::*;
    use dawsu_application::songs_index::{
        SongDiffEntry, SongMapsetEntry, SongsIndex, build_songs_index_from_cache,
        load_or_build_songs_index,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Barrier, mpsc};
    use std::time::{SystemTime, UNIX_EPOCH};

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

        // Give t2 a moment to block on Condvar, then release builder 1.
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

        // Stale build must NOT have committed to `state.ready` or written `.sidx`.
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

        // Out-of-Songs folder patch must be a safe no-op.
        assert!(!cache.patch_mapset_folder(&songs, &outside));

        // Add `[Hitsounds].osu` inside Songs and patch in place.
        std::fs::write(
            map_dir.join("Track [Hitsounds].osu"),
            "[Metadata]\nArtist:Artist\nTitle:Track\nVersion:Hitsounds\n",
        )
        .unwrap();
        assert!(cache.patch_mapset_folder(&songs, &map_dir));

        let updated = cache.search(&songs, "Hitsounds", 50, false);
        assert_eq!(updated.total_diffs, 2);
        assert_eq!(updated.matches.len(), 1);

        // Give async `.sidx` write a moment, then verify persisted on disk via fresh load.
        std::thread::sleep(Duration::from_millis(80));
        let disk = load_or_build_songs_index(&songs, &data, false);
        assert_eq!(disk.total_diffs, 2);

        // External folder addition bumps `songs_dir` mtime and auto-reloads on `refresh=false`.
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

        // Smoke type assertions so imports are verified used in test builds too.
        let _: fn(&[SongMapsetEntry], &Path) -> Option<PathBuf> =
            dawsu_application::songs_index::find_relink_candidate;
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


