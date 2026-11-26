use dawsu_application::Application;
use serde::Deserialize;
use serde_json::Value;
use tauri::State;

use crate::{Editor, Request, ask};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MixerFxAction {
    State,
    AddInsert {
        strip: String,
        plugin_path: String,
        #[serde(default)]
        plugin_uid: Option<String>,
        #[serde(default)]
        name: Option<String>,
    },
    RemoveInsert {
        strip: String,
        slot_id: String,
    },
    MoveInsert {
        strip: String,
        slot_id: String,
        to_index: usize,
    },
    SetBypass {
        strip: String,
        slot_id: String,
        bypassed: bool,
    },
    SetWetDry {
        strip: String,
        slot_id: String,
        wet_dry: f32,
    },
    SwitchAb {
        strip: String,
        slot_id: String,
    },
    CopyAb {
        strip: String,
        slot_id: String,
    },
    OpenEditor {
        strip: String,
        slot_id: String,
    },
    CloseEditor {
        strip: String,
        slot_id: String,
    },
    SetPinned {
        strip: String,
        slot_id: String,
        pinned: bool,
    },
    Undo,
    Redo,
}

pub(crate) fn handle_mixer_fx_action(
    app: &mut Application,
    action: MixerFxAction,
) -> Result<(Value, bool), String> {
    let (snapshot, mutated) = match action {
        MixerFxAction::State => (app.mixer_fx_snapshot(), false),
        MixerFxAction::AddInsert {
            strip,
            plugin_path,
            plugin_uid,
            name,
        } => (app.mixer_fx_add_insert(&strip, plugin_path, plugin_uid, name)?, true),
        MixerFxAction::RemoveInsert { strip, slot_id } => {
            (app.mixer_fx_remove_insert(&strip, &slot_id)?, true)
        }
        MixerFxAction::MoveInsert {
            strip,
            slot_id,
            to_index,
        } => (app.mixer_fx_move_insert(&strip, &slot_id, to_index)?, true),
        MixerFxAction::SetBypass {
            strip,
            slot_id,
            bypassed,
        } => (app.mixer_fx_set_bypass(&strip, &slot_id, bypassed)?, true),
        MixerFxAction::SetWetDry {
            strip,
            slot_id,
            wet_dry,
        } => (app.mixer_fx_set_wet_dry(&strip, &slot_id, wet_dry)?, true),
        MixerFxAction::SwitchAb { strip, slot_id } => {
            (app.mixer_fx_switch_ab(&strip, &slot_id)?, true)
        }
        MixerFxAction::CopyAb { strip, slot_id } => {
            (app.mixer_fx_copy_ab(&strip, &slot_id)?, true)
        }
        MixerFxAction::OpenEditor { strip, slot_id } => {
            (app.mixer_fx_open_editor(&strip, &slot_id)?, true)
        }
        MixerFxAction::CloseEditor { strip, slot_id } => {
            (app.mixer_fx_close_editor(&strip, &slot_id)?, true)
        }
        MixerFxAction::SetPinned {
            strip,
            slot_id,
            pinned,
        } => (app.mixer_fx_set_pinned(&strip, &slot_id, pinned)?, true),
        MixerFxAction::Undo => (app.mixer_fx_undo()?, true),
        MixerFxAction::Redo => (app.mixer_fx_redo()?, true),
    };
    let value = serde_json::to_value(snapshot).map_err(|e| e.to_string())?;
    Ok((value, mutated))
}

#[tauri::command]
pub(crate) fn mixer_fx_action(
    state: State<'_, Editor>,
    action: MixerFxAction,
) -> Result<Value, String> {
    ask(&state, |tx| Request::MixerFx(action, tx))?
}
