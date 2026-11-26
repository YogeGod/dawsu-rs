use dawsu_application::Application;
use serde::Deserialize;
use serde_json::Value;
use tauri::State;

use crate::{Editor, Request, ask};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SharedBaseAction {
    Status,
    SetEnabled {
        enabled: bool,
    },
    Capture,
    #[serde(rename_all = "camelCase")]
    Apply {
        leniency_ms: Option<f64>,
        overwrite_overrides: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    ApplyAll {
        leniency_ms: Option<f64>,
        overwrite_overrides: Option<bool>,
    },
    ClearOverrides,
    Clear,
    #[serde(rename_all = "camelCase")]
    Rules {
        diff_file: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    AddRule {
        diff_file: Option<String>,
        rule: dawsu_application::BaseRule,
    },
    #[serde(rename_all = "camelCase")]
    RemoveRule {
        diff_file: Option<String>,
        index: usize,
    },
    #[serde(rename_all = "camelCase")]
    DiffProjections {
        leniency_ms: Option<f64>,
    },
    #[serde(rename_all = "camelCase")]
    VolumeEnvelope {
        diff_file: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    SetVolumeEnvelope {
        diff_file: Option<String>,
        envelope: dawsu_application::VolumeEnvelope,
    },
}

fn to_json<T: serde::Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result.and_then(|value| serde_json::to_value(value).map_err(|e| e.to_string()))
}

pub(crate) fn handle_shared_base_action(
    app: &mut Application,
    action: SharedBaseAction,
) -> (Result<Value, String>, bool) {
    match action {
        SharedBaseAction::Status => (to_json(Ok(app.shared_base_status())), false),
        SharedBaseAction::SetEnabled { enabled } => {
            (to_json(app.shared_base_set_enabled(enabled)), false)
        }
        SharedBaseAction::Capture => (to_json(app.shared_base_capture_current()), true),
        SharedBaseAction::Apply {
            leniency_ms,
            overwrite_overrides,
        } => (
            to_json(app.shared_base_apply_to_current(
                leniency_ms.unwrap_or(5.0),
                overwrite_overrides.unwrap_or(false),
            )),
            true,
        ),
        SharedBaseAction::ApplyAll {
            leniency_ms,
            overwrite_overrides,
        } => (
            to_json(app.shared_base_apply_to_all_diffs(
                leniency_ms.unwrap_or(5.0),
                overwrite_overrides.unwrap_or(false),
            )),
            true,
        ),
        SharedBaseAction::ClearOverrides => {
            (to_json(app.shared_base_clear_current_overrides()), false)
        }
        SharedBaseAction::Clear => (to_json(app.shared_base_clear()), true),
        SharedBaseAction::Rules { diff_file } => {
            (to_json(Ok(app.shared_base_rules(diff_file.as_deref()))), false)
        }
        SharedBaseAction::AddRule { diff_file, rule } => (
            to_json(app.shared_base_add_rule(diff_file.as_deref(), rule)),
            true,
        ),
        SharedBaseAction::RemoveRule { diff_file, index } => (
            to_json(app.shared_base_remove_rule(diff_file.as_deref(), index)),
            true,
        ),
        SharedBaseAction::DiffProjections { leniency_ms } => (
            to_json(Ok(app.shared_base_diff_projections(leniency_ms.unwrap_or(5.0)))),
            false,
        ),
        SharedBaseAction::VolumeEnvelope { diff_file } => (
            to_json(Ok(app.shared_base_volume_envelope(diff_file.as_deref()))),
            false,
        ),
        SharedBaseAction::SetVolumeEnvelope {
            diff_file,
            envelope,
        } => (
            to_json(app.shared_base_set_volume_envelope(diff_file.as_deref(), envelope)),
            true,
        ),
    }
}

#[tauri::command]
pub(crate) fn shared_base_action(
    action: SharedBaseAction,
    state: State<'_, Editor>,
) -> Result<Value, String> {
    ask(&state, |tx| Request::SharedBase(action, tx))?
}

