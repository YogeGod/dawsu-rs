use dawsu_application::Application;
use dawsu_application::sample_fingerprint::MAX_QUERY_RESULTS;
use dawsu_application::songs_index::SongsSearchResult;
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

/// Resolves a mapset's background image path (checking `osu_path` first, then sibling
/// `.osu` files in the same folder if e.g. `osu_path` is a blank `[Hitsounds]` diff),
/// with a small in-memory folder cache.
pub(crate) use dawsu_application::desktop_services::{
    BackgroundPathCache, PreviewSource, SampleFingerprintCache, SongsIndexCache, is_osu_hs_diff,
};
#[cfg(test)]
pub(crate) use dawsu_application::desktop_services::{PREVIEW_BARS, resolve_osu_background_path};

pub(crate) fn resolve_mapset_background_path(osu_path: &Path) -> Option<PathBuf> {
    static CACHE: std::sync::OnceLock<BackgroundPathCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(BackgroundPathCache::new).resolve(osu_path)
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
    Ok(json!(dawsu_application::desktop_services::inspect_mapset_folder(&path)?))
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
/// The Browser's sample preview: the sample's length in ms and its outline.
pub(crate) fn sample_outline(app: &mut Application, source: PreviewSource) -> Result<Value, String> {
    Ok(json!(dawsu_application::desktop_services::sample_outline(app, source)?))
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
                fp_cache.invalidate(&root);
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


