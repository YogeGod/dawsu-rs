use dawsu_application::bounce_plan::{OszBudgetSummary, calculate_osz_budget};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::State;

use crate::{Editor, Request, ask, folders};

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
pub(crate) fn inspect_midi_file(
    path: String,
    state: State<'_, Editor>,
) -> Result<dawsu_application::MidiInspectResult, String> {
    ask(&state, |tx| Request::InspectMidiFile(path, tx))?
}

#[tauri::command]
pub(crate) fn apply_midi_import(
    path: String,
    tracks: Vec<usize>,
    use_beatmap_tempo: Option<bool>,
    offset_ms: Option<f64>,
    snap: Option<bool>,
    follow_volume: Option<bool>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::MidiImportOutcome, String> {
    let options = dawsu_application::MidiImportOptions {
        path,
        tracks,
        use_beatmap_tempo: use_beatmap_tempo.unwrap_or(true),
        offset_ms: offset_ms.unwrap_or(0.0),
        snap: snap.unwrap_or(true),
        follow_volume: follow_volume.unwrap_or(false),
    };
    ask(&state, |tx| Request::ApplyMidiImport(options, tx))?
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
pub(crate) fn midi_input_ports() -> Vec<String> {
    dawsu_application::list_midi_input_ports()
}

#[tauri::command]
pub(crate) fn midi_record_toggle(
    port_index: Option<usize>,
    snap_divisor: Option<u32>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::MidiRecordStatus, String> {
    ask(&state, |tx| Request::MidiRecordToggle(port_index, snap_divisor, tx))?
}

#[tauri::command]
pub(crate) fn vst3_scan(state: State<'_, Editor>) -> Result<dawsu_application::Vst3ScanReport, String> {
    ask(&state, Request::Vst3Scan)?
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
