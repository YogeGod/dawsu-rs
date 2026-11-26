use dawsu_application::{Application, ExportSettings, OutputBackend};
use serde_json::{Value, json};
use std::path::PathBuf;
use tauri::State;

use crate::action::Action;
use crate::scene::ViewResponse;
use crate::{Editor, Request, ask, folders, osu_link};

static LAST_MAIN_WINDOW_TITLE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

#[tauri::command]
pub(crate) fn editor_view(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
    scene: bool,
    action: Option<Action>,
    fresh: Option<bool>,
    tick_rev: Option<u64>,
) -> Result<ViewResponse, String> {
    let view = ask(&state, |tx| Request::View(scene, action, fresh.unwrap_or(false), tick_rev, tx))?;
    if let ViewResponse::View(snapshot) = &view {
        let next_title = if let Some(scene_val) = &snapshot.scene {
            let map_title = scene_val.title.as_str();
            let version = scene_val.version.as_str();
            let file_stem = scene_val
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            Some(if !version.is_empty() && !map_title.is_empty() {
                format!("{map_title} [{version}] — dawsu!")
            } else if !file_stem.is_empty() {
                format!("{file_stem} — dawsu!")
            } else {
                "dawsu!".to_string()
            })
        } else if !snapshot.has_document {
            Some("dawsu!".to_string())
        } else {
            None
        };
        if let Some(title) = next_title {
            let changed = LAST_MAIN_WINDOW_TITLE
                .lock()
                .map(|mut last| {
                    if last.as_deref() == Some(title.as_str()) {
                        false
                    } else {
                        *last = Some(title.clone());
                        true
                    }
                })
                .unwrap_or(true);
            if changed {
                let _ = window.set_title(&title);
            }
        }
    }
    Ok(view)
}

#[tauri::command]
pub(crate) fn editor_waveform(state: State<'_, Editor>) -> Result<tauri::ipc::Response, String> {
    let bytes = ask(&state, Request::Waveform)??;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub(crate) fn editor_help(state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, Request::Help)?
}

#[tauri::command]
pub(crate) fn editor_export_settings(state: State<'_, Editor>) -> Result<Option<(PathBuf, ExportSettings)>, String> {
    ask(&state, Request::ExportSettings)
}


pub(crate) fn audio_output_settings(app: &Application, requested: Option<OutputBackend>) -> Value {
    let available = OutputBackend::available();
    let prefs = app.preferences();
    let preferred = prefs.audio_backend;
    let backend = requested.unwrap_or_else(|| {
        if available.contains(&preferred) { preferred } else { OutputBackend::System }
    });
    let (mut devices, error) = match app.output_devices(backend) {
        Ok(devices) => (devices, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let qualified_id = (preferred == backend).then_some(prefs.audio_device_id.as_deref()).flatten();
    let device = if let Some(id) = qualified_id {
        devices.iter().find(|device| device.id == id)
    } else if backend == OutputBackend::System {
        prefs.device.as_ref().and_then(|name| devices.iter().find(|device| &device.name == name))
    } else {
        None
    };

    json!({
        "backend": backend,
        "available": available.iter().map(|choice| json!({"id": choice, "label": choice.label()})).collect::<Vec<_>>(),
        "current": app.device_name(),
        "currentBackend": app.active_output_backend().map(OutputBackend::label),
        "devices": devices,
        "device": device.map(|device| &device.id),
        "bufferMs": prefs.audio_buffer_ms,
        "effectiveBufferMs": app.effective_output_buffer_ms(),
        "error": error,
    })
}

#[tauri::command]
pub(crate) fn audio_outputs(state: State<'_, Editor>, backend: Option<OutputBackend>) -> Result<Value, String> {
    ask(&state, |response| Request::AudioOutputs(backend, response))
}

#[tauri::command]
pub(crate) fn editor_shortcuts(state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, Request::Shortcuts)
}

#[tauri::command]
pub(crate) fn import_key_file(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<bool, String> {
    let selected = rfd::FileDialog::new().set_parent(&window)
        .set_title("Import shortcuts").add_filter("dawsu! shortcuts", &["dawsukeys"]).pick_file();
    let Some(path) = selected else { return Ok(false); };
    ask(&state, |tx| Request::KeyFile(path, true, tx))??;
    Ok(true)
}

#[tauri::command]
pub(crate) fn export_key_file(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<bool, String> {
    let selected = rfd::FileDialog::new().set_parent(&window)
        .set_title("Export shortcuts").add_filter("dawsu! shortcuts", &["dawsukeys"])
        .set_file_name("shortcuts.dawsukeys").save_file();
    let Some(mut path) = selected else { return Ok(false); };
    if path.extension().is_none() {
        path.set_extension("dawsukeys");
    }
    ask(&state, |tx| Request::KeyFile(path, false, tx))??;
    Ok(true)
}


#[tauri::command]
pub(crate) fn open_editor_folder(folder: folders::Folder, state: State<'_, Editor>) -> Result<(), String> {
    let path = ask(&state, |tx| Request::Folder(folder, tx))??;
    folder.open(&path)
}

/// Sync command: Windows `ShellExecuteW` relies on Tauri's STA-initialized UI thread.
#[tauri::command]
pub(crate) fn open_help_link(link: folders::HelpLink) -> Result<(), String> {
    folders::open_external_url(link.url())
}

#[tauri::command]
pub(crate) fn close_editor(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
    discard: bool,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the editor window can quit dawsu!.".into());
    }
    ask(&state, |tx| Request::Close(discard, tx))??;
    app.exit(0);
    Ok(())
}


#[tauri::command]
pub(crate) fn open_from_osu(state: State<'_, Editor>) -> Result<String, String> {
    let songs = ask(&state, Request::SongsDirectory)?;
    let path = osu_link::find_open_osu_map(songs.as_deref())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub(crate) fn reload_osu_editor(state: State<'_, Editor>) -> Result<osu_link::OsuReloadResult, String> {
    let Some(path) = ask(&state, Request::CurrentDocumentPath)? else {
        return Err("Open a difficulty before reloading it in osu!.".into());
    };
    Ok(osu_link::reload_osu_editor(&path))
}


#[tauri::command]
pub(crate) async fn relink_moved_mapset(
    missing_path: String,
    pick_folder: Option<bool>,
    state: State<'_, Editor>,
    cache: State<'_, crate::commands::SongsIndexCache>,
) -> Result<dawsu_application::RelinkMapsetResult, String> {
    let missing = PathBuf::from(&missing_path);
    if pick_folder.unwrap_or(false) {
        let picked = tauri::async_runtime::spawn_blocking(|| {
            rfd::FileDialog::new()
                .set_title("Locate moved mapset folder")
                .pick_folder()
        })
        .await
        .map_err(|e| e.to_string())?;
        let Some(folder) = picked else {
            return Err("Folder selection cancelled.".into());
        };
        return ask(&state, |tx| {
            Request::RelinkMovedMapset(missing, Some(folder), None, tx)
        })?;
    }

    let missing_file = missing
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("Missing difficulty filename.")?
        .to_string();
    let Some(songs_dir) = ask(&state, Request::SongsDirectory)? else {
        return Err(
            "Set your osu! Songs folder in Preferences or pick the moved folder directly.".into(),
        );
    };
    let cache = cache.inner().clone();
    let missing_clone = missing.clone();
    let resolved = tauri::async_runtime::spawn_blocking(move || {
        cache
            .entries_for(&songs_dir)
            .find_relink_candidate(&missing_clone)
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(candidate) = resolved else {
        return Err(format!(
            "Could not locate {missing_file} in the Songs folder."
        ));
    };
    ask(&state, |tx| {
        Request::RelinkMovedMapset(missing, None, Some(candidate), tx)
    })?
}


#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MapsetRevertAction {
    Preview,
    #[serde(rename_all = "camelCase")]
    Revert {
        include_deleted: Option<Vec<String>>,
    },
    Undo,
}

pub(crate) fn handle_mapset_revert_action(
    app: &mut Application,
    action: MapsetRevertAction,
) -> (Result<Value, String>, bool) {
    match action {
        MapsetRevertAction::Preview => (
            app.preview_revert_mapset_to_original()
                .and_then(|value| serde_json::to_value(value).map_err(|e| e.to_string())),
            false,
        ),
        MapsetRevertAction::Revert { include_deleted } => (
            app.revert_mapset_to_original(&include_deleted.unwrap_or_default())
                .and_then(|value| serde_json::to_value(value).map_err(|e| e.to_string())),
            true,
        ),
        MapsetRevertAction::Undo => (
            app.undo_mapset_revert()
                .and_then(|value| serde_json::to_value(value).map_err(|e| e.to_string())),
            true,
        ),
    }
}

#[tauri::command]
pub(crate) fn mapset_revert_action(
    action: MapsetRevertAction,
    state: State<'_, Editor>,
) -> Result<Value, String> {
    ask(&state, |tx| Request::MapsetRevert(action, tx))?
}

