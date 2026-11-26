use dawsu_application::Application;
use serde::Deserialize;
use tauri::State;

use crate::{Editor, Request, ask, updater};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum StemSeparationAction {
    Status,
    #[serde(rename_all = "camelCase")]
    Start {
        force_rerun: Option<bool>,
        custom_model_path: Option<String>,
    },
    Cancel,
    DeleteCache,
}

pub(crate) fn handle_stem_separation_action(
    app: &mut Application,
    action: StemSeparationAction,
) -> (Result<dawsu_application::StemSeparationStatus, String>, bool) {
    match action {
        StemSeparationAction::Status => (Ok(app.stem_separation_status()), true),
        StemSeparationAction::Start {
            force_rerun,
            custom_model_path,
        } => {
            let options = dawsu_application::StemSeparationStartOptions {
                force_rerun: force_rerun.unwrap_or(false),
                custom_model_path,
            };
            (app.stem_separation_start(options), true)
        }
        StemSeparationAction::Cancel => (Ok(app.stem_separation_cancel()), false),
        StemSeparationAction::DeleteCache => (app.stem_separation_delete_cache(), false),
    }
}

#[tauri::command]
pub(crate) fn stem_separation_action(
    action: StemSeparationAction,
    state: State<'_, Editor>,
) -> Result<dawsu_application::StemSeparationStatus, String> {
    ask(&state, |tx| Request::StemSeparation(action, tx))?
}


#[tauri::command]
pub(crate) async fn check_for_updates(
    staged: State<'_, updater::StagedUpdate>,
) -> Result<updater::UpdateCheckResult, String> {
    let (result, parsed) = tauri::async_runtime::spawn_blocking(updater::check_latest_release_parsed)
        .await
        .map_err(|e| format!("Update check task failed: {e}"))??;
    staged.record_release(parsed);
    Ok(result)
}

#[tauri::command]
pub(crate) async fn download_update(
    staged: State<'_, updater::StagedUpdate>,
) -> Result<updater::DownloadedUpdate, String> {
    let release = staged
        .last_release()
        .ok_or_else(|| "Click Check for updates first.".to_string())?;
    let data_root = dawsu_application::data_folder();
    let (ui_result, verified) = tauri::async_runtime::spawn_blocking(move || {
        updater::download_and_verify_release(&data_root, &release)
    })
    .await
    .map_err(|e| format!("Update download task failed: {e}"))??;
    staged.record_download(verified);
    Ok(ui_result)
}

#[tauri::command]
pub(crate) fn install_update_on_close(
    window: tauri::WebviewWindow,
    staged: State<'_, updater::StagedUpdate>,
) -> Result<(), String> {
    let data_root = dawsu_application::data_folder();
    staged.arm_verified_download(&data_root)?;
    // Trigger the normal window close so unsaved work is asked about first (same as v3.1.4).
    if let Err(e) = window.close() {
        staged.clear();
        return Err(format!("Could not close window to apply update: {e}"));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn cancel_staged_update(staged: State<'_, updater::StagedUpdate>) {
    staged.clear();
}

