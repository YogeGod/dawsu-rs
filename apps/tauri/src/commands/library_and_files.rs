use dawsu_application::Application;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
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
    if let Ok(mut guard) = cache.0.lock() {
        *guard = None;
    }
    Ok(created)
}

#[tauri::command]
pub(crate) fn create_hitsound_diff_from_current(
    state: State<'_, Editor>,
    cache: State<'_, SongsIndexCache>,
) -> Result<PathBuf, String> {
    let created = ask(&state, Request::CreateHitsoundDiffFromCurrent)??;
    if let Ok(mut guard) = cache.0.lock() {
        *guard = None;
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

pub(crate) type SongsIndexEntry = (PathBuf, &'static str, Vec<dawsu_application::songs_index::SongMapsetEntry>);
pub(crate) struct SongsIndexCache(pub(crate) std::sync::Arc<std::sync::Mutex<Option<SongsIndexEntry>>>);

#[tauri::command]
pub(crate) async fn songs_index_search(
    query: String,
    refresh: bool,
    state: State<'_, Editor>,
    cache: State<'_, SongsIndexCache>,
) -> Result<dawsu_application::songs_index::SongsSearchResult, String> {
    let Some(songs_dir) = ask(&state, Request::SongsDirectory)? else {
        return Ok(dawsu_application::songs_index::SongsSearchResult {
            total_mapsets: 0,
            total_diffs: 0,
            source: "none",
            matches: Vec::new(),
        });
    };
    let cache_arc = std::sync::Arc::clone(&cache.0);
    tauri::async_runtime::spawn_blocking(move || {
        const SEARCH_LIMIT: usize = 50;
        if !refresh
            && let Ok(guard) = cache_arc.lock()
                && let Some((cached_dir, source, entries)) = guard.as_ref()
                    && cached_dir == &songs_dir {
                        let total_diffs = entries.iter().map(|m| m.diffs.len()).sum();
                        let matches = dawsu_application::songs_index::search_songs_index(
                            entries,
                            &query,
                            SEARCH_LIMIT,
                        );
                        return Ok(dawsu_application::songs_index::SongsSearchResult {
                            total_mapsets: entries.len(),
                            total_diffs,
                            source,
                            matches,
                        });
                    }
        let (entries, source) =
            dawsu_application::songs_index::build_songs_index_with_source(&songs_dir);
        let total_mapsets = entries.len();
        let total_diffs = entries.iter().map(|m| m.diffs.len()).sum();
        let matches =
            dawsu_application::songs_index::search_songs_index(&entries, &query, SEARCH_LIMIT);
        if let Ok(mut guard) = cache_arc.lock() {
            *guard = Some((songs_dir, source, entries));
        }
        Ok(dawsu_application::songs_index::SongsSearchResult {
            total_mapsets,
            total_diffs,
            source,
            matches,
        })
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

