// Every command the app registers. With this list Tauri lets a window call only the commands its
// capability in capabilities/ allows (apps/tauri/tests/capabilities.test.mjs checks the lists).
const COMMANDS: &[&str] = &[
    "editor_view",
    "editor_help",
    "editor_export_settings",
    "export_view",
    "export_preview",
    "export_run",
    "open_export_window",
    "close_export_window",
    "show_in_editor",
    "drum_window_open",
    "close_drum_window",
    "drum_context",
    "drum_detect_start",
    "drum_review",
    "drum_listen",
    "drum_listen_stop",
    "drum_apply",
    "drum_pick_stem",
    "pick_merge_preset",
    "save_merge_preset",
    "editor_shortcuts",
    "audio_outputs",
    "import_key_file",
    "export_key_file",
    "pick_map",
    "pick_sample",
    "pick_sound_set",
    "sound_set_library",
    "sound_set_placement",
    "save_sound_set",
    "pick_songs_folder",
    "pick_hitsound_library",
    "list_library",
    "lane_add_candidates",
    "pick_drum_map",
    "set_drum_view",
    "open_editor_folder",
    "close_editor",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("build the dawsu! Tauri app");
}
