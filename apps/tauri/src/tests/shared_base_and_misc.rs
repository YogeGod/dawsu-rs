use super::helpers::{Fixture, open};
use crate::{Action, PlaybackStart, snapshot, snapshot_with_static};
use dawsu_application::Application;
use std::fs;

#[test]
fn output_device_change_is_refused_during_a_job_without_changing_transport() {
    let mut app = Application::headless();
    app.seek(350.0);
    app.locator_left = 200.0;
    app.locator_right = 450.0;
    app.cycle = true;
    app.song_offset = -17.0;
    app.preferences.master_gain = 0.45;
    app.preferences.device = Some("Original device".into());
    app.busy = true;
    let action = serde_json::from_value::<Action>(serde_json::json!({
        "kind": "output_settings", "backend": "mme", "device_id": "mme:other", "ms": 30,
    })).unwrap();
    action.run(&mut app);
    assert!(app.error.as_deref().unwrap().contains("finish the current job"));
    assert_eq!(app.position_ms(), 350.0);
    assert_eq!((app.locator_left, app.locator_right, app.cycle), (200.0, 450.0, true));
    assert_eq!(app.song_offset, -17.0);
    assert_eq!(app.preferences.master_gain, 0.45);
    assert_eq!(app.preferences.device.as_deref(), Some("Original device"));
}

#[test]
fn rejected_output_settings_keep_the_device_preferences_and_transport() {
    use dawsu_application::OutputBackend;
    use serde_json::json;

    let mut app = Application::headless();
    app.seek(350.0);
    app.locator_left = 200.0;
    app.locator_right = 450.0;
    app.cycle = true;
    app.song_offset = -17.0;
    app.preferences.master_gain = 0.45;
    app.preferences.device = Some("Original device".into());
    app.preferences.audio_buffer_ms = Some(40);
    let saved = serde_json::to_vec(&app.preferences).unwrap();

    let sent = json!({"kind": "output_settings", "backend": "system", "device_id": "system:test", "ms": 0});
    serde_json::from_value::<Action>(sent).unwrap().run(&mut app);
    assert!(app.error.as_deref().unwrap().contains("10 and 500"));
    assert_eq!(app.position_ms(), 350.0);
    assert_eq!((app.locator_left, app.locator_right, app.cycle), (200.0, 450.0, true));
    assert_eq!(app.song_offset, -17.0);
    assert_eq!(serde_json::to_vec(&app.preferences).unwrap(), saved);

    let restored: dawsu_application::Preferences = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored.audio_buffer_ms, Some(40));
    assert_eq!(restored.audio_backend, OutputBackend::System);
    assert!(restored.audio_device_id.is_none());
    assert_eq!(restored.device.as_deref(), Some("Original device"));

    let profile: dawsu_application::Preferences = serde_json::from_value(json!({
        "audio_backend": "direct_sound", "audio_device_id": "direct_sound:test", "audio_buffer_ms": 50,
        "device": "Original device"
    })).unwrap();
    assert_eq!(profile.audio_backend, OutputBackend::DirectSound);
    assert_eq!(profile.audio_device_id.as_deref(), Some("direct_sound:test"));
    assert_eq!(profile.device.as_deref(), Some("Original device"));
    assert_eq!(serde_json::to_value(profile).unwrap()["audio_backend"], "direct_sound");
}

/// The frontend's one command list (`ui/src/lib/commands.js`) names every catalogue command once,
/// either on a menu row (`command: "id"`) or under `keyOnly`, and names nothing the catalogue lacks.
#[test]
fn the_menu_command_list_matches_the_command_catalogue() {
    use std::collections::BTreeSet;
    let list = include_str!("../../ui/src/lib/commands.js");
    let mut named = BTreeSet::new();
    for part in list.split("command: \"").skip(1) {
        named.insert(part.split('"').next().unwrap().to_owned());
    }
    let key_only = list.split("export const keyOnly = [").nth(1).unwrap().split("];").next().unwrap();
    for (index, part) in key_only.split('"').enumerate() {
        if index % 2 == 1 {
            assert!(named.insert(part.to_owned()), "{part} is on a menu row and in keyOnly");
        }
    }
    let catalogue: BTreeSet<String> =
        Application::headless().shortcuts().into_iter().map(|(id, _, _)| id.to_owned()).collect();
    assert!(catalogue.len() > 60, "the catalogue lists its commands");
    let missing: Vec<_> = catalogue.difference(&named).collect();
    let unknown: Vec<_> = named.difference(&catalogue).collect();
    assert!(missing.is_empty(), "catalogue commands missing from commands.js: {missing:?}");
    assert!(unknown.is_empty(), "commands.js names commands the catalogue lacks: {unknown:?}");
}

#[test]
fn the_view_carries_the_map_title_version_and_output_device_for_the_frame() {
    let fixture = Fixture::new("shell-fields");
    let path = fixture.0.join("Artist - Song (Me) [File name].osu");
    let map = b"osu file format v14\n\n[Metadata]\nTitle:Song <b>\nVersion:Hard\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    let empty = snapshot(&app, true);
    assert_eq!(empty["outputDevice"], app.device_name());
    open(&mut app, &path);
    let view = snapshot(&app, true);
    assert_eq!(view["scene"]["mapTitle"], "Song <b>");
    assert_eq!(view["scene"]["version"], "Hard");
    assert_eq!(view["outputDevice"], app.device_name());
}

#[test]
fn node_graph_tauri_actions_round_trip_and_export_json() {
    let mut app = Application::headless();
    let initial = serde_json::to_value(app.node_graph_view()).unwrap();
    assert_eq!(initial["nodes"].as_array().unwrap().len(), 21);
    assert_eq!(initial["catalog"].as_array().unwrap().len(), 18);

    let add: dawsu_application::NodeGraphAction = serde_json::from_value(serde_json::json!({
        "kind": "add_node",
        "typeId": "output.viewer",
        "x": 120.0,
        "y": 240.0
    }))
    .unwrap();
    app.node_graph_action(add).unwrap();
    let after_add = serde_json::to_value(app.node_graph_view()).unwrap();
    assert_eq!(after_add["nodes"].as_array().unwrap().len(), 22);
    assert_eq!(after_add["canUndo"], true);

    let exported = app.node_graph_export_json().unwrap();
    assert!(exported.contains("\"dawsu-graph\""));
    let import: dawsu_application::NodeGraphAction = serde_json::from_value(serde_json::json!({
        "kind": "import_json",
        "json": exported
    }))
    .unwrap();
    app.node_graph_action(import).unwrap();
    assert_eq!(app.node_graph_view().nodes.len(), 22);
}

#[test]
fn typed_scene_serializes_hit_shapes_and_static_overlay_fields() {
    let fixture = Fixture::new("typed-scene-parity");
    let path = fixture.0.join("Parity.osu");
    // Four moments covering: normal hits, clap + custom gain, slider with body, and orphaned hit.
    fs::write(
        &path,
        b"osu file format v14\n\n[Metadata]\nTitle:Parity\nVersion:Hard\n\n[TimingPoints]\n0,500,4,2,0,80,1,0\n1000,-100,4,2,0,60,0,0\n\n[HitObjects]\n256,192,0,1,8,0:0:0:0:\n256,192,500,1,2,0:0:0:0:custom.wav\n256,192,1000,2,0,L|356:192,1,100,2|0,0:0|0:0,0:0:0:0:\n",
    ).unwrap();
    let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    fs::write(fixture.0.join("custom.wav"), wave).unwrap();

    let mut app = Application::headless();
    open(&mut app, &path);
    let doc_path = app.document().unwrap().path.clone();

    // Draw an extra hit at 1500 ms (NoObject) and select/mute/gain hits so all hit tuple shapes appear.
    let lanes = snapshot(&app, true)["scene"]["lanes"].as_array().unwrap().clone();
    let clap_row = lanes.iter().position(|lane| lane["name"] == "soft-hitclap" && lane["index"] == 0).unwrap() as i32;
    let custom_lane = lanes.iter().find(|lane| lane["sample"] == "custom.wav").unwrap();
    let custom_key = custom_lane["key"].as_str().unwrap().to_owned();
    let custom_row = lanes.iter().position(|lane| lane["sample"] == "custom.wav").unwrap() as i32;

    Action::Tool { name: "Draw".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1500.0, row: clap_row, x: 12.0, y: 12.0,
            value: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }

    // Mute the clap hit at 0 ms (listen-only mute, [row, selected, true]).
    Action::Tool { name: "Mute".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 0.0, row: clap_row, x: 12.0, y: 12.0,
            value: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }

    // Select the custom.wav hit at 500 ms and set a custom per-hit gain ([row, true, false, 45]).
    Action::Tool { name: "Select".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 500.0, row: custom_row, x: 12.0, y: 12.0,
            value: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    Action::VolumeScope { path: doc_path.clone(), lanes: Some(vec![custom_key]) }.run(&mut app);
    Action::Volume { value: 45, relative: false }.run(&mut app);

    // Also add the orphaned clap hit at 1500 ms to the selection (selected, [row, true]).
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1500.0, row: clap_row, x: 12.0, y: 12.0,
            value: None, button: None, shift: false, ctrl: true, alt: false,
        }.run(&mut app);
    }

    // Also add a bookmark so bookmarks are exercised.
    Action::BookmarkAdd { path: doc_path, time: 500.0, label: "Mid".into() }.run(&mut app);

    for include_static in [true, false] {
        let view = serde_json::to_value(snapshot_with_static(&app, true, include_static)).unwrap();
        let scene = &view["scene"];
        let mut has_plain = false;
        let mut has_selected = false;
        let mut has_muted = false;
        let mut has_custom_gain = false;
        for moment in scene["moments"].as_array().unwrap() {
            for hit in moment[3].as_array().unwrap() {
                match hit.as_array().unwrap().as_slice() {
                    [_, sel] if sel == &serde_json::json!(false) => has_plain = true,
                    [_, sel] if sel == &serde_json::json!(true) => has_selected = true,
                    [_, _, muted] if muted == &serde_json::json!(true) => has_muted = true,
                    [_, _, _, gain] if gain == &serde_json::json!(45) => has_custom_gain = true,
                    _ => {}
                }
            }
        }
        assert!(
            has_plain && has_selected && has_muted && has_custom_gain,
            "fixture should cover all hit shapes: plain={has_plain} selected={has_selected} muted={has_muted} gain={has_custom_gain}"
        );
        assert!(scene["bookmarks"].as_array().unwrap().iter().any(|b| b["label"] == "Mid" && b["time"] == 500.0));
        assert!(!scene["orphanedHits"].as_array().unwrap().is_empty());

        if include_static {
            assert!(scene["sections"].is_array());
            assert!(scene["combos"].is_object());
            assert!(scene["gameplay"].is_object());
            assert!(view["recentMeta"].is_array());
        } else {
            assert!(scene.get("sections").is_none());
            assert!(scene.get("combos").is_none());
            assert!(scene.get("gameplay").is_none());
            assert!(view.get("recentMeta").is_none());
        }
    }

    // Toggle HS mode on and verify gameplay stays present when include_static = false.
    Action::HsMode { on: true }.run(&mut app);
    for include_static in [true, false] {
        let view = serde_json::to_value(snapshot_with_static(&app, true, include_static)).unwrap();
        let scene = &view["scene"];
        assert!(scene["gameplay"].is_object(), "gameplay should remain present in HS mode (include_static={include_static})");
        assert_eq!(scene.get("sections").is_some(), include_static);
        assert_eq!(scene.get("combos").is_some(), include_static);
        assert_eq!(view.get("recentMeta").is_some(), include_static);
    }
}

#[test]
fn fresh_gameplay_diff_scene_reports_objects_snap_and_updates_signature() {
    let fixture = Fixture::new("task1-objects-snap-scene");
    let path = fixture.0.join("map.osu");
    std::fs::write(
        &path,
        "osu file format v14\n[TimingPoints]\n0,500,4,1,0,80,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n",
    )
    .unwrap();
    let mut app = Application::headless();
    super::helpers::open_raw(&mut app, &path);

    let view = snapshot(&app, true);
    assert_eq!(view["scene"]["snapMode"], "objects");
    assert!(
        view["musicalPosition"].as_str().unwrap().ends_with(" · Objects"),
        "expected Objects snap label, got {:?}",
        view["musicalPosition"]
    );
    assert_eq!(view["scene"]["laneOrder"].as_array().unwrap().len(), 2);

    let home = crate::home::Route::default();
    let sig_objects = crate::non_playback_signature(&app, &home, PlaybackStart::Cursor, 0);
    Action::Snap { on: true, mode: "grid".into(), beats: 1, divisor: 4 }.run(&mut app);
    let sig_grid = crate::non_playback_signature(&app, &home, PlaybackStart::Cursor, 0);
    assert_ne!(sig_objects, sig_grid);
}

#[test]
fn slim_tick_polls_return_tick_when_unchanged_and_full_view_on_status_warning_or_job_change() {
    let fixture = Fixture::new("slim-tick");
    let map = fixture.0.join("map.osu");
    fs::write(
        &map,
        "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let mut home = crate::home::Route::default();
    let mut drums = crate::drum_view::DrumView::for_editor();
    let mut playback_start = PlaybackStart::default();
    let mut cache = crate::ViewTickCache::default();
    let mut pending = None;

    let initial = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        true, None, false, None,
    ))
    .unwrap();
    assert!(initial.get("tick").is_none());
    assert!(initial["scene"].is_object());
    let rev1 = initial["tickRev"].as_u64().unwrap();

    let tick1 = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev1),
    ))
    .unwrap();
    assert_eq!(tick1["tick"], true);
    assert_eq!(tick1["rev"], rev1);
    assert!(tick1.get("scene").is_none());
    assert!(tick1["peaks"]["master"].is_number());
    assert!(tick1["musicalPosition"].is_string());

    // Status change invalidates the non-playback signature -> returns full view with bumped tickRev.
    app.status = "Saved map".into();
    let after_status = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev1),
    ))
    .unwrap();
    assert!(after_status.get("tick").is_none());
    assert_eq!(after_status["status"], "Saved map");
    let rev2 = after_status["tickRev"].as_u64().unwrap();
    assert!(rev2 > rev1);

    let tick2 = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev2),
    ))
    .unwrap();
    assert_eq!(tick2["tick"], true);
    assert_eq!(tick2["rev"], rev2);

    // Warning change invalidates the signature -> returns full view.
    app.warnings.push("Missing sample".into());
    let after_warning = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev2),
    ))
    .unwrap();
    assert!(after_warning.get("tick").is_none());
    assert_eq!(after_warning["warnings"], serde_json::json!(["Missing sample"]));
    let rev3 = after_warning["tickRev"].as_u64().unwrap();
    assert!(rev3 > rev2);

    let tick3 = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev3),
    ))
    .unwrap();
    assert_eq!(tick3["tick"], true);

    // Job busy flag change invalidates the signature -> returns full view.
    app.busy = true;
    let after_busy = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev3),
    ))
    .unwrap();
    assert!(after_busy.get("tick").is_none());
    assert_eq!(after_busy["busy"], true);
    let rev4 = after_busy["tickRev"].as_u64().unwrap();
    assert!(rev4 > rev3);

    // song_loading change invalidates the signature -> returns full view.
    app.song_loading = true;
    let while_loading = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev4),
    ))
    .unwrap();
    assert!(while_loading.get("tick").is_none());
    assert_eq!(while_loading["songLoading"], true);
    let rev5 = while_loading["tickRev"].as_u64().unwrap();
    assert!(rev5 > rev4);

    app.song_loading = false;
    let after_loading = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev5),
    ))
    .unwrap();
    assert!(after_loading.get("tick").is_none());
    assert_eq!(after_loading["songLoading"], false);
    assert!(after_loading["tickRev"].as_u64().unwrap() > rev5);
}
