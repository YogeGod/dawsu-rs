//! Tauri IPC handlers for VST3 Drum & Keysound Bounce `.osz` budget planning.

use dawsu_application::bounce_plan::{OszBudgetSummary, calculate_osz_budget};

/// Calculates live `.osz` ranking budget status (`Safe | Caution | OverLimit`) against 10.0 MB.
#[tauri::command]
pub fn bounce_osz_budget(base_bytes: u64, bounced_bytes: u64) -> OszBudgetSummary {
    calculate_osz_budget(base_bytes, bounced_bytes)
}
