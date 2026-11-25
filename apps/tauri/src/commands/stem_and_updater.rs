use tauri::State;

use crate::{Editor, Request, ask, updater};

#[tauri::command]
pub(crate) fn stem_separation_status(
    state: State<'_, Editor>,
) -> Result<dawsu_application::StemSeparationStatus, String> {
    ask(&state, Request::StemSeparationStatus)?
}

#[tauri::command]
pub(crate) fn stem_separation_start(
    force_rerun: Option<bool>,
    custom_model_path: Option<String>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::StemSeparationStatus, String> {
    let options = dawsu_application::StemSeparationStartOptions {
        force_rerun: force_rerun.unwrap_or(false),
        custom_model_path,
    };
    ask(&state, |tx| Request::StemSeparationStart(options, tx))?
}

#[tauri::command]
pub(crate) fn stem_separation_cancel(
    state: State<'_, Editor>,
) -> Result<dawsu_application::StemSeparationStatus, String> {
    ask(&state, Request::StemSeparationCancel)?
}

#[tauri::command]
pub(crate) fn stem_separation_delete_cache(
    state: State<'_, Editor>,
) -> Result<dawsu_application::StemSeparationStatus, String> {
    ask(&state, Request::StemSeparationDeleteCache)?
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

