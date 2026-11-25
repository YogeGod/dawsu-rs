use dawsu_application::bounce_plan::{OszBudgetSummary, calculate_osz_budget};
use dawsu_application::preview_diff::{DiffRibbonRow, PreviewDiffCache};
use dawsu_application::{ExportOptions, ExportSettings, MixFormat};
use dawsu_hitsounding::copy::CopyOptions;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

use crate::action::CopyOptionsJson;
use crate::{Editor, Request, ask};

pub(crate) fn default_export_true() -> bool {
    true
}

#[derive(Deserialize)]
pub(crate) struct ExportOptionsJson {
    source: PathBuf,
    targets: Vec<PathBuf>,
    #[serde(with = "CopyOptionsJson")]
    options: CopyOptions,
    /// Also write the base hitsound difficulty (Generate), with or without its slider audio.
    #[serde(default)]
    base: bool,
    #[serde(default, rename = "sliderAudio")]
    slider_audio: bool,
    #[serde(default, rename = "autoFixStagedMixes")]
    auto_fix_staged_mixes: bool,
    #[serde(default, rename = "mixFormat")]
    mix_format: MixFormat,
    #[serde(default, rename = "writeMode")]
    write_mode: dawsu_application::ExportWriteMode,
    #[serde(default, rename = "hitsoundOnlyDir")]
    hitsound_only_dir: Option<PathBuf>,
    #[serde(default, rename = "singleFormat")]
    single_format: dawsu_application::SingleSampleFormat,
    #[serde(default, rename = "additionOnlyNormal")]
    addition_only_normal: dawsu_application::AdditionOnlyNormal,
    #[serde(default, rename = "tickPolicy")]
    tick_policy: dawsu_application::TickPolicy,
    #[serde(default = "default_export_true", rename = "allowSchemaGrowth")]
    allow_schema_growth: bool,
    #[serde(default = "default_export_true", rename = "autoCleanupStale")]
    auto_cleanup_stale: bool,
    #[serde(default, rename = "takeOverSamples")]
    take_over_samples: bool,
    #[serde(default = "default_export_true", rename = "applyVolumeEnvelope")]
    apply_volume_envelope: bool,
}

/// "Export · <map title> [<difficulty>]", or "Export" with no map open.
pub(crate) fn export_title(view: &Value) -> String {
    match (view["source"]["title"].as_str(), view["source"]["difficulty"].as_str()) {
        (Some(title), Some(difficulty)) if !title.ends_with(&format!("[{difficulty}]")) => {
            format!("Export · {title} [{difficulty}]")
        }
        (Some(title), _) => format!("Export · {title}"),
        _ => "Export".into(),
    }
}

#[tauri::command]
pub(crate) fn export_view(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<Value, String> {
    let view = ask(&state, Request::ExportView)??;
    // The title follows the map the editor has open, also after a switch while Export is open.
    let title = export_title(&view);
    if window.title().ok().as_deref() != Some(title.as_str()) {
        let _ = window.set_title(&title);
    }
    Ok(view)
}

#[tauri::command]
pub(crate) fn export_preview(state: State<'_, Editor>, options: ExportOptionsJson) -> Result<u64, String> {
    let source = options.source;
    let options = ExportOptions {
        targets: options.targets,
        copy: options.options,
        base: options.base,
        slider_audio: options.slider_audio,
        auto_fix_staged_mixes: options.auto_fix_staged_mixes,
        mix_format: options.mix_format,
        write_mode: options.write_mode,
        hitsound_only_dir: options.hitsound_only_dir,
        single_format: options.single_format,
        addition_only_normal: options.addition_only_normal,
        tick_policy: options.tick_policy,
        allow_schema_growth: options.allow_schema_growth,
        auto_cleanup_stale: options.auto_cleanup_stale,
        take_over_samples: options.take_over_samples,
        apply_volume_envelope: options.apply_volume_envelope,
    };
    ask(&state, |tx| Request::ExportPreview(source, options, tx))?
}

#[tauri::command]
pub(crate) fn export_run(state: State<'_, Editor>, preview_id: u64) -> Result<u64, String> {
    ask(&state, |tx| Request::ExportRun(preview_id, tx))?
}

#[tauri::command]
pub(crate) fn export_unused_dry_run(
    state: State<'_, Editor>,
) -> Result<Vec<dawsu_application::UnusedSampleCandidate>, String> {
    ask(&state, Request::ExportUnusedDryRun)?
}

#[tauri::command]
pub(crate) fn export_listen_target(state: State<'_, Editor>, target_path: String) -> Result<(), String> {
    ask(&state, |tx| Request::ExportListenTarget(target_path, tx))?
}

#[tauri::command]
pub(crate) fn export_stop_listen(state: State<'_, Editor>) -> Result<(), String> {
    ask(&state, Request::ExportStopListen)?
}


#[tauri::command]
pub(crate) fn pick_merge_preset(window: tauri::WebviewWindow) -> Result<Option<ExportSettings>, String> {
    rfd::FileDialog::new().set_parent(&window).set_title("Load merge preset")
        .add_filter("Merge preset", &["json"]).pick_file()
        .map(|path| ExportSettings::read_file(&path)).transpose()
}

#[tauri::command]
pub(crate) fn save_merge_preset(window: tauri::WebviewWindow, value: ExportSettings) -> Result<bool, String> {
    use dawsu_foundation::safe_write::{self, BackupPolicy, WriteOutcome};
    let bytes = value.checked_bytes()?;
    let Some(mut path) = rfd::FileDialog::new().set_parent(&window).set_title("Save merge preset")
        .add_filter("Merge preset", &["json"]).set_file_name("hitsound-merge.json").save_file()
        else { return Ok(false) };
    if path.extension().is_none() {
        path.set_extension("json");
    }
    if !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json")) {
        return Err("Choose a .json file for the merge preset.".into());
    }
    match std::fs::read(&path) {
        Ok(expected) => {
            let backups = BackupPolicy::new(dawsu_application::data_folder().join("backups"), 5);
            match safe_write::replace(&path, &expected, &bytes, &backups).map_err(|error| error.to_string())? {
                WriteOutcome::Saved { .. } => {}
                WriteOutcome::ChangedOnDisk => return Err("The preset file changed while saving. Choose a new file or try again.".into()),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            safe_write::create(&path, &bytes).map_err(|error| error.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(true)
}


#[tauri::command]
pub(crate) fn export_pick_hitsound_only_dir() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose standalone hitsound-only export folder")
        .pick_folder()
        .map(|p| p.to_string_lossy().into_owned())
}


static RIBBON_CACHE: std::sync::LazyLock<Mutex<PreviewDiffCache>> =
    std::sync::LazyLock::new(|| Mutex::new(PreviewDiffCache::default()));

/// Projects up to 4 sibling difficulties inside the currently open mapset into coverage strips
/// against the provided source hit times. Never holds the cache mutex across disk reads or parsing.
#[tauri::command]
pub fn preview_diff_ribbon(
    state: State<'_, crate::Editor>,
    siblings: Vec<String>,
    source_hit_times_ms: Vec<f64>,
    duration_ms: f64,
    tolerance_ms: Option<f64>,
    bins: Option<usize>,
) -> Result<Vec<DiffRibbonRow>, String> {
    let Some(mapset_root) = crate::ask(&state, crate::Request::OpenMapsetRoot)? else {
        return Ok(Vec::new());
    };
    let paths: Vec<PathBuf> = siblings.into_iter().map(PathBuf::from).collect();
    Ok(PreviewDiffCache::project_ribbon_with_lock(
        &RIBBON_CACHE,
        &mapset_root,
        &paths,
        &source_hit_times_ms,
        duration_ms,
        tolerance_ms.unwrap_or(5.0),
        bins.unwrap_or(96),
    ))
}

/// Calculates live `.osz` ranking budget status (`Safe | Caution | OverLimit`) against 10.0 MB.
#[tauri::command]
pub fn bounce_osz_budget(base_bytes: u64, bounced_bytes: u64) -> OszBudgetSummary {
    calculate_osz_budget(base_bytes, bounced_bytes)
}
