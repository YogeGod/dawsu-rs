use tauri::State;

use crate::{Editor, Request, ask};

#[tauri::command]
pub(crate) fn shared_base_status(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, Request::SharedBaseStatus)?
}

#[tauri::command]
pub(crate) fn shared_base_set_enabled(
    enabled: bool,
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, |tx| Request::SharedBaseSetEnabled(enabled, tx))?
}

#[tauri::command]
pub(crate) fn shared_base_capture(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, Request::SharedBaseCapture)?
}

#[tauri::command]
pub(crate) fn shared_base_apply(
    leniency_ms: Option<f64>,
    overwrite_overrides: Option<bool>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseApplyResult, String> {
    ask(&state, |tx| {
        Request::SharedBaseApply(
            leniency_ms.unwrap_or(5.0),
            overwrite_overrides.unwrap_or(false),
            tx,
        )
    })?
}

#[tauri::command]
pub(crate) fn shared_base_apply_all(
    leniency_ms: Option<f64>,
    overwrite_overrides: Option<bool>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseApplyAllResult, String> {
    ask(&state, |tx| {
        Request::SharedBaseApplyAll(
            leniency_ms.unwrap_or(5.0),
            overwrite_overrides.unwrap_or(false),
            tx,
        )
    })?
}

#[tauri::command]
pub(crate) fn shared_base_clear_overrides(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, Request::SharedBaseClearOverrides)?
}

#[tauri::command]
pub(crate) fn shared_base_clear(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, Request::SharedBaseClear)?
}

#[tauri::command]
pub(crate) fn shared_base_rules(
    diff_file: Option<String>,
    state: State<'_, Editor>,
) -> Result<Vec<dawsu_application::BaseRule>, String> {
    ask(&state, |tx| Request::SharedBaseRules(diff_file, tx))?
}

#[tauri::command]
pub(crate) fn shared_base_add_rule(
    diff_file: Option<String>,
    rule: dawsu_application::BaseRule,
    state: State<'_, Editor>,
) -> Result<Vec<dawsu_application::BaseRule>, String> {
    ask(&state, |tx| Request::SharedBaseAddRule(diff_file, rule, tx))?
}

#[tauri::command]
pub(crate) fn shared_base_remove_rule(
    diff_file: Option<String>,
    index: usize,
    state: State<'_, Editor>,
) -> Result<Vec<dawsu_application::BaseRule>, String> {
    ask(&state, |tx| Request::SharedBaseRemoveRule(diff_file, index, tx))?
}

#[tauri::command]
pub(crate) fn shared_base_diff_projections(
    leniency_ms: Option<f64>,
    state: State<'_, Editor>,
) -> Result<Vec<dawsu_application::DiffProjectionTab>, String> {
    ask(&state, |tx| {
        Request::SharedBaseDiffProjections(leniency_ms.unwrap_or(5.0), tx)
    })?
}

#[tauri::command]
pub(crate) fn shared_base_volume_envelope(
    diff_file: Option<String>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::VolumeEnvelope, String> {
    ask(&state, |tx| Request::SharedBaseVolumeEnvelope(diff_file, tx))?
}

#[tauri::command]
pub(crate) fn shared_base_set_volume_envelope(
    diff_file: Option<String>,
    envelope: dawsu_application::VolumeEnvelope,
    state: State<'_, Editor>,
) -> Result<dawsu_application::VolumeEnvelope, String> {
    ask(&state, |tx| {
        Request::SharedBaseSetVolumeEnvelope(diff_file, envelope, tx)
    })?
}

