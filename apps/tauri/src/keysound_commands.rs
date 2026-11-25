//! Tauri IPC command for Keysounding deduplication & `.osz` budget preview.

use dawsu_application::bounce_plan::{OszBudgetSummary, calculate_osz_budget};
use serde::Serialize;

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
