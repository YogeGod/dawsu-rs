use dawsu_application::Application;
use serde_json::{Value, json};
use std::path::PathBuf;
use tauri::{Emitter, State};

use crate::scene::ViewResponse;
use crate::{Editor, Request, ask, drum_view, drum_window};

pub(crate) fn ask_drum(state: &Editor, request: drum_window::DrumRequest) -> Result<Value, String> {
    ask(state, |tx| Request::Drum(request, tx))?
}

#[tauri::command]
pub(crate) fn drum_context(state: State<'_, Editor>, known: Option<String>) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Context { known })
}

#[tauri::command]
pub(crate) fn drum_detect_start(state: State<'_, Editor>, map: PathBuf, source: drum_window::SourceJson) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Start { map, source })
}

#[tauri::command]
pub(crate) fn drum_review(state: State<'_, Editor>, map: PathBuf, options: drum_window::OptionsJson) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Review { map, options })
}

#[tauri::command]
pub(crate) fn drum_listen(state: State<'_, Editor>, map: PathBuf, from: f64, ticked: Vec<String>) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Listen { map, from, ticked })
}

#[tauri::command]
pub(crate) fn drum_listen_stop(state: State<'_, Editor>) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::StopListen)
}

#[tauri::command]
pub(crate) fn drum_apply(state: State<'_, Editor>, map: PathBuf, review: String, ticked: Vec<String>) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Apply { map, review, ticked })
}

/// A drum stem file, picked in a dialog over the drum detection window.
#[tauri::command]
pub(crate) fn drum_pick_stem(window: tauri::WebviewWindow) -> Option<PathBuf> {
    rfd::FileDialog::new().set_parent(&window).set_title("Pick a drum stem")
        .add_filter("Audio", &["wav", "ogg", "mp3", "flac"]).pick_file()
}


pub(crate) fn emit_node_graph_sync(app: &tauri::AppHandle, view: &Value, refresh_timeline: bool) {
    let viewer = view.get("viewer").cloned().unwrap_or(Value::Null);
    let _ = app.emit_to("main", "node-graph-viewer", viewer);
    if refresh_timeline {
        let _ = app.emit_to("main", "node-graph-applied", true);
    }
}

#[tauri::command]
pub(crate) fn node_graph_view(app: tauri::AppHandle, state: State<'_, Editor>) -> Result<Value, String> {
    let view = ask(&state, Request::NodeGraphView)??;
    emit_node_graph_sync(&app, &view, false);
    Ok(view)
}

#[tauri::command]
pub(crate) fn node_graph_action(
    app: tauri::AppHandle,
    state: State<'_, Editor>,
    action: dawsu_application::NodeGraphAction,
) -> Result<Value, String> {
    let refresh_timeline = matches!(
        action,
        dawsu_application::NodeGraphAction::Apply | dawsu_application::NodeGraphAction::SelectLastHits
    );
    let view = ask(&state, |tx| Request::NodeGraphAction(action, tx))??;
    emit_node_graph_sync(&app, &view, refresh_timeline);
    Ok(view)
}

#[tauri::command]
pub(crate) fn node_graph_export_json(state: State<'_, Editor>) -> Result<String, String> {
    ask(&state, Request::NodeGraphExportJson)?
}

#[tauri::command]
pub(crate) fn node_graph_pick_open(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, Editor>,
) -> Result<Option<Value>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Load node graph recipe")
        .add_filter("Node graph", &["dawsugraph", "json"])
        .pick_file()
    else {
        return Ok(None);
    };
    let json = std::fs::read_to_string(&path).map_err(|e| format!("Could not read recipe: {e}"))?;
    let view = ask(&state, |tx| {
        Request::NodeGraphAction(dawsu_application::NodeGraphAction::ImportJson { json }, tx)
    })??;
    emit_node_graph_sync(&app, &view, false);
    Ok(Some(view))
}

#[tauri::command]
pub(crate) fn node_graph_pick_save(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<bool, String> {
    use dawsu_foundation::safe_write::{self, BackupPolicy, WriteOutcome};
    let json = ask(&state, Request::NodeGraphExportJson)??;
    let Some(mut path) = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Save node graph recipe")
        .add_filter("Node graph", &["dawsugraph"])
        .save_file()
    else {
        return Ok(false);
    };
    if path.extension().is_none() {
        path.set_extension("dawsugraph");
    }
    let bytes = json.as_bytes();
    match std::fs::read(&path) {
        Ok(expected) => {
            let backups = BackupPolicy::new(dawsu_application::data_folder().join("backups"), 5);
            match safe_write::replace(&path, &expected, bytes, &backups).map_err(|e| e.to_string())? {
                WriteOutcome::Saved { .. } => {}
                WriteOutcome::ChangedOnDisk => {
                    return Err("The recipe file changed while saving. Choose a new file or try again.".into());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            safe_write::create(&path, bytes).map_err(|e| e.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(true)
}

#[tauri::command]
pub(crate) fn node_graph_pick_audio(window: tauri::WebviewWindow) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Pick an audio or stem file")
        .add_filter("Audio", &["wav", "ogg", "mp3", "flac"])
        .pick_file()
}


#[tauri::command]
pub(crate) fn pick_drum_map(
    map: PathBuf,
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<ViewResponse>, String> {
    let Some(path) = rfd::FileDialog::new().set_parent(&window).set_title("Open drum map")
        .add_filter("Drum map", &["dawsudrum"]).pick_file() else { return Ok(None) };
    ask(&state, |tx| Request::DrumView(map, Some(path), true, tx))?.map(Some)
}

#[tauri::command]
pub(crate) fn set_drum_view(map: PathBuf, on: bool, state: State<'_, Editor>) -> Result<ViewResponse, String> {
    ask(&state, |tx| Request::DrumView(map, None, on, tx))?
}

#[tauri::command]
pub(crate) fn drum_map_action(
    map: PathBuf,
    action: drum_view::DrumMapAction,
    state: State<'_, Editor>,
) -> Result<ViewResponse, String> {
    ask(&state, |tx| Request::DrumMapAction(map, action, tx))?
}


#[tauri::command]
pub(crate) fn drum_vst_action(payload: Value, state: State<'_, Editor>) -> Result<Value, String> {
    if payload.get("kind").and_then(Value::as_str) == Some("cancel_bounce") {
        state.1.cancel();
        return Ok(json!({ "cancelled": true }));
    }
    ask(&state, |tx| Request::DrumVstAction(payload, tx))?
}

pub(crate) fn handle_drum_vst_action(
    app: &mut Application,
    drums: &mut drum_view::DrumView,
    payload: &Value,
) -> Result<Value, String> {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("Missing drum_vst_action kind")?;
    match kind {
        "status" => Ok(drum_vst_status_json(app)),
        "list_installed" => {
            let plugins = dawsu_application::vst3::list_installed_vst3_in(app.vst3.search_dirs());
            let folders: Vec<String> = app
                .vst3
                .search_dirs()
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            Ok(json!({
                "plugins": plugins.into_iter().map(|p| json!({
                    "name": p.label,
                    "path": p.path.to_string_lossy(),
                })).collect::<Vec<_>>(),
                "folders": folders,
            }))
        }
        "browse" => {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Choose a VST3 instrument")
                .add_filter("VST3 plug-in", &["vst3"]);
            if let Some(dir) = app.vst3.search_dirs().iter().find(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.pick_file() {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("VST3")
                    .to_string();
                Ok(json!({
                    "picked": true,
                    "name": name,
                    "path": path.to_string_lossy(),
                }))
            } else {
                Ok(json!({ "picked": false }))
            }
        }
        "load" => {
            let path = payload
                .get("pluginPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or("Choose a VST3 plug-in file first.")?;
            let uid = payload
                .get("pluginUid")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            app.load_drum_vst(PathBuf::from(path), uid)
                .map_err(|e| e.to_string())?;
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            app.set_drum_vst_choke_groups(drums.choke_groups(app));
            Ok(drum_vst_status_json(app))
        }
        "toggle" => {
            let on = payload.get("enabled").and_then(Value::as_bool).unwrap_or(false);
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            app.set_drum_vst_choke_groups(drums.choke_groups(app));
            app.set_drum_vst_enabled(on);
            Ok(drum_vst_status_json(app))
        }
        "toggle_lane" => {
            let row = payload
                .get("row")
                .and_then(Value::as_u64)
                .ok_or("Missing lane row")? as usize;
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            app.set_drum_vst_choke_groups(drums.choke_groups(app));
            app.toggle_drum_vst_lane(row)?;
            Ok(drum_vst_status_json(app))
        }
        "set_choke_group" => {
            let row_index = payload
                .get("rowIndex")
                .and_then(Value::as_u64)
                .ok_or("Missing rowIndex")? as usize;
            let choke_group = payload
                .get("chokeGroup")
                .and_then(Value::as_u64)
                .map(|v| v.min(255) as u8)
                .filter(|&g| g > 0);
            let map = app
                .document()
                .ok_or("Open a difficulty first.")?
                .path
                .clone();
            drums.set_row_choke_group(app, &map, row_index, choke_group)?;
            app.set_drum_vst_choke_groups(drums.choke_groups(app));
            Ok(json!({ "ok": true, "rowIndex": row_index, "chokeGroup": choke_group }))
        }
        "unload" => {
            app.unload_drum_vst();
            Ok(drum_vst_status_json(app))
        }
        "open_editor" => {
            let id = app
                .drum_vst()
                .as_ref()
                .map(|v| v.instance_id.clone())
                .ok_or("Load a Drum VST3 instrument first.")?;
            app.vst3.open_editor(&id).map_err(|e| e.to_string())?;
            Ok(drum_vst_status_json(app))
        }
        "close_editor" => {
            let id = app
                .drum_vst()
                .as_ref()
                .map(|v| v.instance_id.clone())
                .ok_or("Load a Drum VST3 instrument first.")?;
            app.vst3.close_editor(&id).map_err(|e| e.to_string())?;
            let _ = app.save_drum_vst_session_state();
            Ok(drum_vst_status_json(app))
        }
        "preview_note" => {
            let note = payload.get("note").and_then(Value::as_u64).unwrap_or(36).min(127) as u8;
            let velocity = payload.get("velocity").and_then(Value::as_u64).unwrap_or(100).clamp(1, 127) as u8;
            if let Some(vst) = app.drum_vst() {
                vst.preview(note, velocity);
            }
            Ok(json!({ "ok": true }))
        }
        "note_names" => {
            let id = app
                .drum_vst()
                .map(|v| v.instance_id.clone())
                .ok_or("Load a Drum VST3 instrument first.")?;
            let names = app.vst3.note_names(&id).map_err(|e| e.to_string())?;
            Ok(json!({
                "notes": names.into_iter().map(|(note, name)| json!({ "note": note, "name": name })).collect::<Vec<_>>()
            }))
        }
        "sync_state" => {
            app.save_drum_vst_session_state().map_err(|e| e.to_string())?;
            Ok(drum_vst_status_json(app))
        }
        "save_state" => {
            let b64 = app.save_drum_vst_session_state().map_err(|e| e.to_string())?;
            let picked = rfd::FileDialog::new()
                .set_title("Save Drum VST3 state")
                .set_file_name("drums-state.bin")
                .add_filter("VST3 state", &["bin", "vststate"])
                .save_file();
            if let Some(path) = picked {
                std::fs::write(&path, b64.as_bytes()).map_err(|e| e.to_string())?;
                Ok(json!({ "saved": true, "path": path.to_string_lossy() }))
            } else {
                Ok(json!({ "saved": false }))
            }
        }
        "load_state" => {
            let picked = rfd::FileDialog::new()
                .set_title("Load Drum VST3 state")
                .add_filter("VST3 state", &["bin", "vststate"])
                .pick_file();
            if let Some(path) = picked {
                let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                app.restore_drum_vst_state_b64(raw.trim()).map_err(|e| e.to_string())?;
                Ok(json!({ "loaded": true, "path": path.to_string_lossy() }))
            } else {
                Ok(json!({ "loaded": false }))
            }
        }
        "import_dawsudrum" => {
            let map = app
                .document()
                .ok_or("Open a difficulty first.")?
                .path
                .clone();
            let picked = rfd::FileDialog::new()
                .set_title("Import drum map")
                .add_filter("Drum maps", &["dawsudrum", "drm"])
                .pick_file();
            if let Some(path) = picked {
                drums.load(app, &map, &path)?;
                app.set_drum_vst_routing(drums.routed_midi_notes(app));
                app.set_drum_vst_choke_groups(drums.choke_groups(app));
                Ok(json!({ "imported": true, "path": path.to_string_lossy() }))
            } else {
                Ok(json!({ "imported": false }))
            }
        }
        "export_dawsudrum" => {
            let map_kit = drums
                .active_map(app)
                .ok_or("Open a drum map first.")?;
            let text = dawsu_hitsounding::drums::write_dawsudrum(map_kit).map_err(|e| e.to_string())?;
            let picked = rfd::FileDialog::new()
                .set_title("Save drum map")
                .set_file_name(format!("{}.dawsudrum", map_kit.name))
                .add_filter("dawsu! drum map", &["dawsudrum"])
                .save_file();
            if let Some(path) = picked {
                std::fs::write(&path, text.as_bytes()).map_err(|e| e.to_string())?;
                Ok(json!({ "exported": true, "path": path.to_string_lossy() }))
            } else {
                Ok(json!({ "exported": false }))
            }
        }
        "plan_render" => {
            let exact = payload.get("exact").and_then(Value::as_bool).unwrap_or(false);
            let mode = if exact {
                dawsu_application::VelocityExportMode::ExactPerVolume
            } else {
                dawsu_application::VelocityExportMode::Tiered { max_interval: 20 }
            };
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            app.set_drum_vst_choke_groups(drums.choke_groups(app));
            let plan = app.plan_drum_vst_render(mode)?;
            let base_bytes: u64 = app
                .document()
                .and_then(|d| d.path.parent())
                .and_then(|dir| std::fs::read_dir(dir).ok())
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .fold(0u64, u64::saturating_add);
            Ok(json!({
                "exact": exact,
                "baseBytes": base_bytes,
                "hitCount": plan.assignments.len(),
                "jobCount": plan.jobs.len(),
                "jobs": plan.jobs.iter().map(|j| json!({
                    "filename": j.filename,
                    "pitch": j.pitch,
                    "midiVelocity": j.midi_velocity,
                    "hitCount": j.hit_count,
                })).collect::<Vec<_>>(),
            }))
        }
        "preview_render" => {
            let exact = payload.get("exact").and_then(Value::as_bool).unwrap_or(false);
            let format = match payload.get("format").and_then(Value::as_str) {
                Some("ogg_vorbis" | "ogg") => dawsu_application::bounce_plan::BounceAudioFormat::OggVorbis,
                _ => dawsu_application::bounce_plan::BounceAudioFormat::Wav16,
            };
            let normalize = payload.get("normalize").and_then(Value::as_bool).unwrap_or(true);
            let auto_trim = payload.get("autoTrim").and_then(Value::as_bool).unwrap_or(true);
            let mode = if exact {
                dawsu_application::VelocityExportMode::ExactPerVolume
            } else {
                dawsu_application::VelocityExportMode::Tiered { max_interval: 20 }
            };
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            app.set_drum_vst_choke_groups(drums.choke_groups(app));
            app.preview_drum_vst_render_with_options(mode, format, normalize, auto_trim)
        }
        "render_wavs" => {
            let exact = payload.get("exact").and_then(Value::as_bool).unwrap_or(false);
            let format = match payload.get("format").and_then(Value::as_str) {
                Some("ogg_vorbis" | "ogg") => dawsu_application::bounce_plan::BounceAudioFormat::OggVorbis,
                _ => dawsu_application::bounce_plan::BounceAudioFormat::Wav16,
            };
            let normalize = payload.get("normalize").and_then(Value::as_bool).unwrap_or(false);
            let auto_trim = payload.get("autoTrim").and_then(Value::as_bool).unwrap_or(true);
            let mode = if exact {
                dawsu_application::VelocityExportMode::ExactPerVolume
            } else {
                dawsu_application::VelocityExportMode::Tiered { max_interval: 20 }
            };
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            app.set_drum_vst_choke_groups(drums.choke_groups(app));
            let (plan, outputs) =
                app.render_drum_vst_with_options(mode, format, normalize, auto_trim)?;
            Ok(json!({
                "renderedCount": outputs.len(),
                "hitCount": plan.assignments.len(),
                "format": format,
                "files": outputs.iter().map(|o| o.path.file_name().and_then(|n| n.to_str()).unwrap_or("")).collect::<Vec<_>>(),
            }))
        }
        _ => Err(format!("Unknown drum_vst_action: {kind}")),
    }
}

pub(crate) fn drum_vst_status_json(app: &Application) -> Value {
    match app.drum_vst() {
        Some(v) => json!({
            "loaded": true,
            "instanceId": v.instance_id,
            "pluginName": v.plugin_name,
            "pluginPath": v.plugin_path,
            "pluginUid": v.plugin_uid,
            "enabled": v.is_enabled(),
            "mappedLanes": v.routed_notes().len(),
        }),
        None => json!({
            "loaded": false,
            "enabled": false,
            "mappedLanes": 0,
        }),
    }
}

