use dawsu_application::Application;
use dawsu_application::bounce_plan::{OszBudgetSummary, calculate_osz_budget};
use dawsu_hitsounding::drums::DrumMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use tauri::State;

use crate::{Editor, Request, ask, folders};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MidiAction {
    InspectFile {
        path: String,
    },
    #[serde(rename_all = "camelCase")]
    ApplyImport {
        path: String,
        tracks: Vec<usize>,
        use_beatmap_tempo: Option<bool>,
        offset_ms: Option<f64>,
        snap: Option<bool>,
        follow_volume: Option<bool>,
    },
    InputPorts,
    #[serde(rename_all = "camelCase")]
    SelectPort {
        port_index: Option<usize>,
    },
    #[serde(rename_all = "camelCase")]
    RecordToggle {
        port_index: Option<usize>,
        snap_divisor: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    CaptureRetrospective {
        snap_divisor: Option<u32>,
    },
    InspectSoundfont {
        path: String,
    },
    #[serde(rename_all = "camelCase")]
    AuditionSoundfont {
        path: String,
        preset_index: Option<usize>,
        pitch: Option<u8>,
        velocity: Option<u8>,
        duration_ms: Option<f64>,
    },
}

fn to_json<T: Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result.and_then(|value| serde_json::to_value(value).map_err(|e| e.to_string()))
}

pub(crate) fn handle_midi_action(
    app: &mut Application,
    drum_map: Option<&DrumMap>,
    action: MidiAction,
) -> (Result<Value, String>, bool) {
    match action {
        MidiAction::InspectFile { path } => {
            let res = (|| {
                let document = app.document().ok_or("Open a difficulty first.")?;
                let lanes = document.composer.hitsounds().lanes();
                dawsu_application::inspect_midi_file(Path::new(&path), lanes, drum_map)
            })();
            (to_json(res), false)
        }
        MidiAction::ApplyImport {
            path,
            tracks,
            use_beatmap_tempo,
            offset_ms,
            snap,
            follow_volume,
        } => {
            let options = dawsu_application::MidiImportOptions {
                path,
                tracks,
                use_beatmap_tempo: use_beatmap_tempo.unwrap_or(true),
                offset_ms: offset_ms.unwrap_or(0.0),
                snap: snap.unwrap_or(true),
                follow_volume: follow_volume.unwrap_or(false),
            };
            (to_json(app.apply_midi_import(&options, drum_map)), true)
        }
        MidiAction::InputPorts => (to_json(Ok(dawsu_application::list_midi_input_ports())), false),
        MidiAction::SelectPort { port_index } => {
            (to_json(app.select_midi_input_port(port_index)), false)
        }
        MidiAction::RecordToggle {
            port_index,
            snap_divisor,
        } => (
            to_json(app.toggle_midi_recording(port_index, drum_map, snap_divisor)),
            true,
        ),
        MidiAction::CaptureRetrospective { snap_divisor } => (
            to_json(app.capture_retrospective(drum_map, snap_divisor)),
            true,
        ),
        MidiAction::InspectSoundfont { path } => {
            (to_json(app.inspect_soundfont_file(Path::new(&path))), false)
        }
        MidiAction::AuditionSoundfont {
            path,
            preset_index,
            pitch,
            velocity,
            duration_ms,
        } => (
            to_json(app.audition_soundfont_note(
                Path::new(&path),
                preset_index.unwrap_or(0),
                pitch.unwrap_or(60),
                velocity.unwrap_or(100),
                duration_ms.unwrap_or(280.0),
            )),
            false,
        ),
    }
}

#[tauri::command]
pub(crate) fn pick_midi_file(window: tauri::WebviewWindow) -> Option<String> {
    rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Import MIDI file")
        .add_filter("MIDI file", &["mid", "midi"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
pub(crate) fn midi_action(action: MidiAction, state: State<'_, Editor>) -> Result<Value, String> {
    if matches!(action, MidiAction::InputPorts) {
        return to_json(Ok(dawsu_application::list_midi_input_ports()));
    }
    ask(&state, |tx| Request::Midi(action, tx))?
}

#[tauri::command]
pub(crate) fn export_midi_file(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<usize>, String> {
    let mut dialog = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Export MIDI file")
        .add_filter("MIDI file", &["mid"])
        .set_file_name("hitsounds.mid");
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let Some(mut path) = dialog.save_file() else {
        return Ok(None);
    };
    if path.extension().is_none() {
        path.set_extension("mid");
    }
    ask(&state, |tx| Request::ExportMidiFile(path, tx))?.map(Some)
}

#[tauri::command]
pub(crate) fn import_mania_hitsounds(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<dawsu_application::ManiaImportReport>, String> {
    let mut dialog = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Import mania hitsound difficulty")
        .add_filter("osu! beatmap", &["osu"]);
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let Some(path) = dialog.pick_file() else {
        return Ok(None);
    };
    ask(&state, |tx| Request::ImportManiaHitsounds(path, tx))?.map(Some)
}

#[tauri::command]
pub(crate) fn vst3_scan(
    retry_blocked: Option<bool>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::Vst3ScanReport, String> {
    ask(&state, |tx| Request::Vst3Scan {
        retry_blocked: retry_blocked.unwrap_or(false),
        response: tx,
    })?
}

#[tauri::command]
pub(crate) fn pick_soundfont_file(window: tauri::WebviewWindow) -> Option<String> {
    rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Choose SoundFont or SFZ instrument")
        .add_filter("SoundFont / SFZ", &["sf2", "sfz"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
}


#[tauri::command]
pub(crate) fn keysounds_action(payload: Value, state: State<'_, Editor>) -> Result<Value, String> {
    if payload.get("kind").and_then(Value::as_str) == Some("cancel_bounce") {
        state.1.cancel();
        return Ok(json!({ "cancelled": true }));
    }
    ask(&state, |tx| Request::KeysoundsAction(payload, tx))?
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysoundDedupReport {
    pub source_notes: usize,
    pub unique_samples: usize,
    pub dedup_percent: u8,
    pub budget: OszBudgetSummary,
}

#[tauri::command]
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
