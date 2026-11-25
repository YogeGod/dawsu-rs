//! Tauri IPC handlers for Live Preview Diff and the 28 px 4-Diff Mini-Ribbon.

use dawsu_application::preview_diff::{DiffRibbonRow, PreviewDiffCache};
use std::path::PathBuf;
use std::sync::Mutex;

static RIBBON_CACHE: std::sync::LazyLock<Mutex<PreviewDiffCache>> =
    std::sync::LazyLock::new(|| Mutex::new(PreviewDiffCache::default()));

/// Projects up to 4 sibling difficulties into coverage strips against the provided source hit times.
#[tauri::command]
pub fn preview_diff_ribbon(
    siblings: Vec<String>,
    source_hit_times_ms: Vec<f64>,
    duration_ms: f64,
    tolerance_ms: Option<f64>,
    bins: Option<usize>,
) -> Result<Vec<DiffRibbonRow>, String> {
    let paths: Vec<PathBuf> = siblings.into_iter().map(PathBuf::from).collect();
    let mut cache = RIBBON_CACHE
        .lock()
        .map_err(|_| "Preview diff cache lock poisoned".to_string())?;
    Ok(cache.project_ribbon(
        &paths,
        &source_hit_times_ms,
        duration_ms,
        tolerance_ms.unwrap_or(5.0),
        bins.unwrap_or(96),
    ))
}
