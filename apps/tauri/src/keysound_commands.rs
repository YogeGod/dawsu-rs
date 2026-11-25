//! Tauri IPC helpers for Keysounding Undo/Redo and `.dawsuks` project persistence.

use serde_json::json;

/// Builds a `keysounds_action` JSON payload for Undo/Redo/Save without duplicating logic in `main.rs`.
pub fn undo_payload() -> serde_json::Value {
    json!({ "kind": "undo" })
}

pub fn redo_payload() -> serde_json::Value {
    json!({ "kind": "redo" })
}
