//! Tauri IPC handlers for Live Preview Diff and the 28 px 4-Diff Mini-Ribbon.

use dawsu_application::preview_diff::{DiffRibbonRow, PreviewDiffCache};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

static RIBBON_CACHE: std::sync::LazyLock<Mutex<PreviewDiffCache>> =
    std::sync::LazyLock::new(|| Mutex::new(PreviewDiffCache::default()));

/// Projects up to 4 sibling difficulties inside the currently open mapset into coverage strips
/// against the provided source hit times. Never holds the cache mutex across disk reads or parsing.
#[tauri::command]
pub fn preview_diff_ribbon(
    state: State<'_, super::Editor>,
    siblings: Vec<String>,
    source_hit_times_ms: Vec<f64>,
    duration_ms: f64,
    tolerance_ms: Option<f64>,
    bins: Option<usize>,
) -> Result<Vec<DiffRibbonRow>, String> {
    let Some(mapset_root) = super::ask(&state, super::Request::OpenMapsetRoot)? else {
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
