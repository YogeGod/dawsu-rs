use super::helpers::{Fixture, open};
use crate::{Action, PlaybackStart, snapshot, snapshot_with_static};
use dawsu_application::Application;
use std::fs;

#[test]
fn output_device_change_is_refused_during_a_job_without_changing_transport() {
    let mut app = Application::headless();
    app.seek(350.0);
    app.set_locators(200.0, 450.0, true);
    app.set_song_offset(-17.0);
    app.update_preferences(|p| {
        p.master_gain = 0.45;
        p.device = Some("Original device".into());
    });
    app.set_busy_for_test(true);
    let action = serde_json::from_value::<Action>(serde_json::json!({
        "kind": "output_settings", "backend": "mme", "device_id": "mme:other", "ms": 30,
    })).unwrap();
    action.run(&mut app);
    assert!(app.error().unwrap().contains("finish the current job"));
    assert_eq!(app.position_ms(), 350.0);
    assert_eq!((app.locator_left(), app.locator_right(), app.cycle_enabled()), (200.0, 450.0, true));
    assert_eq!(app.song_offset(), -17.0);
    assert_eq!(app.preferences().master_gain, 0.45);
    assert_eq!(app.preferences().device.as_deref(), Some("Original device"));
}

#[test]
fn rejected_output_settings_keep_the_device_preferences_and_transport() {
    use dawsu_application::OutputBackend;
    use serde_json::json;

    let mut app = Application::headless();
    app.seek(350.0);
    app.set_locators(200.0, 450.0, true);
    app.set_song_offset(-17.0);
    app.update_preferences(|p| {
        p.master_gain = 0.45;
        p.device = Some("Original device".into());
        p.audio_buffer_ms = Some(40);
    });
    let saved = serde_json::to_vec(app.preferences()).unwrap();

    let sent = json!({"kind": "output_settings", "backend": "system", "device_id": "system:test", "ms": 0});
    serde_json::from_value::<Action>(sent).unwrap().run(&mut app);
    assert!(app.error().unwrap().contains("2 and 500"));
    assert_eq!(app.position_ms(), 350.0);
    assert_eq!((app.locator_left(), app.locator_right(), app.cycle_enabled()), (200.0, 450.0, true));
    assert_eq!(app.song_offset(), -17.0);
    assert_eq!(serde_json::to_vec(app.preferences()).unwrap(), saved);

    let sent_mme = json!({"kind": "output_settings", "backend": "mme", "device_id": "mme:test", "ms": 5});
    serde_json::from_value::<Action>(sent_mme).unwrap().run(&mut app);
    assert!(app.error().unwrap().contains("10 and 500"));
    assert_eq!(serde_json::to_vec(app.preferences()).unwrap(), saved);

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
            value: None, volume_lanes: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }

    // Mute the clap hit at 0 ms (listen-only mute, [row, selected, true]).
    Action::Tool { name: "Mute".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 0.0, row: clap_row, x: 12.0, y: 12.0,
            value: None, volume_lanes: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }

    // Select the custom.wav hit at 500 ms and set a custom per-hit gain ([row, true, false, 45]).
    Action::Tool { name: "Select".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 500.0, row: custom_row, x: 12.0, y: 12.0,
            value: None, volume_lanes: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    Action::VolumeScope { path: doc_path.clone(), lanes: Some(vec![custom_key]) }.run(&mut app);
    Action::Volume { value: 45, relative: false }.run(&mut app);

    // Also add the orphaned clap hit at 1500 ms to the selection (selected, [row, true]).
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1500.0, row: clap_row, x: 12.0, y: 12.0,
            value: None, volume_lanes: None, button: None, shift: false, ctrl: true, alt: false,
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

    let tok_objects = app.view_state_token();
    Action::Snap { on: true, mode: "grid".into(), beats: 1, divisor: 4 }.run(&mut app);
    let tok_grid = app.view_state_token();
    assert_ne!(tok_objects, tok_grid);
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

    // Status change invalidates the view token -> returns full view with bumped tickRev.
    app.set_status("Saved map");
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

    // Warning change invalidates the view token -> returns full view.
    app.push_warning("Missing sample");
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

    // Job busy flag change invalidates the view token -> returns full view.
    app.set_busy_for_test(true);
    let after_busy = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev3),
    ))
    .unwrap();
    assert!(after_busy.get("tick").is_none());
    assert_eq!(after_busy["busy"], true);
    let rev4 = after_busy["tickRev"].as_u64().unwrap();
    assert!(rev4 > rev3);

    // song_loading change invalidates the view token -> returns full view.
    app.set_song_loading_for_test(true);
    let while_loading = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev4),
    ))
    .unwrap();
    assert!(while_loading.get("tick").is_none());
    assert_eq!(while_loading["songLoading"], true);
    let rev5 = while_loading["tickRev"].as_u64().unwrap();
    assert!(rev5 > rev4);

    app.set_song_loading_for_test(false);
    let after_loading = serde_json::to_value(crate::next_view_response(
        &mut app, &mut home, &mut drums, &mut playback_start, &mut cache, &mut pending,
        false, None, false, Some(rev5),
    ))
    .unwrap();
    assert!(after_loading.get("tick").is_none());
    assert_eq!(after_loading["songLoading"], false);
    assert!(after_loading["tickRev"].as_u64().unwrap() > rev5);
}

#[test]
fn telemetry_frame_packs_v1_layout_and_hub_bumps_state_rev_on_idle_timeout_poll() {
    let bytes = crate::pack_telemetry_frame_v1(&crate::TelemetryFrameInput {
        flags: crate::TELEMETRY_FLAG_PLAYING | crate::TELEMETRY_FLAG_HAS_DOCUMENT,
        meter_num: 3,
        meter_den: 4,
        sub_tick: 240,
        state_rev: 42,
        position_ms: 12_345.5,
        bpm_centi: 18_500,
        bar: 9,
        beat: 2,
        master_l: 0.75,
        master_r: 0.5,
        song_peak: 0.625,
        hitsounds_peak: 0.375,
        rms_l: 0.25,
        rms_r: 0.125,
        phase_corr: 0.875,
    });
    assert_eq!(bytes.len(), crate::TELEMETRY_FRAME_BYTES);
    assert_eq!(bytes[0], 0x01);
    assert_eq!(bytes[1], crate::TELEMETRY_FLAG_PLAYING | crate::TELEMETRY_FLAG_HAS_DOCUMENT);
    assert_eq!(u16::from_le_bytes(bytes[2..4].try_into().unwrap()), 3);
    assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 4);
    assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 240);
    assert_eq!(u64::from_le_bytes(bytes[8..16].try_into().unwrap()), 42);
    assert_eq!(f64::from_le_bytes(bytes[16..24].try_into().unwrap()), 12_345.5);
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), 18_500);
    assert_eq!(i32::from_le_bytes(bytes[28..32].try_into().unwrap()), 9);
    assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 2);
    assert_eq!(f32::from_le_bytes(bytes[36..40].try_into().unwrap()), 0.75);
    assert_eq!(f32::from_le_bytes(bytes[40..44].try_into().unwrap()), 0.5);
    assert_eq!(f32::from_le_bytes(bytes[44..48].try_into().unwrap()), 0.625);
    assert_eq!(f32::from_le_bytes(bytes[48..52].try_into().unwrap()), 0.375);
    assert_eq!(f32::from_le_bytes(bytes[52..56].try_into().unwrap()), 0.25);
    assert_eq!(f32::from_le_bytes(bytes[56..60].try_into().unwrap()), 0.125);
    assert_eq!(f32::from_le_bytes(bytes[60..64].try_into().unwrap()), 0.875);

    let mut app = Application::headless();
    let home = crate::home::Route::default();
    let hub = crate::TelemetryHub::default();
    let rev0 = hub.sync(&app, &home, crate::PlaybackStart::default(), 0);
    app.set_song_loading_for_test(true);
    let rev1 = hub.sync(&app, &home, crate::PlaybackStart::default(), 0);
    assert!(rev1 > rev0, "expected state_rev bump when song_loading flips");
    let rev1_same = hub.sync(&app, &home, crate::PlaybackStart::default(), 0);
    assert_eq!(rev1_same, rev1, "unchanged state must not bump state_rev");

    // Verify background editor_thread Timeout path syncs TelemetryHub without a Request::View poll.
    let editor = crate::editor_thread(None);
    std::thread::sleep(std::time::Duration::from_millis(60));
    let before = editor.2.state_rev();
    let rev_from_cmd = crate::ask(&editor, crate::Request::Help).is_ok();
    assert!(rev_from_cmd);
    assert!(before >= 1);

    let mut bins = [-120.0_f32; dawsu_application::spectrum::SPECTRUM_BINS];
    bins[0] = -18.5;
    bins[127] = -42.25;
    let spec = crate::pack_spectrum_frame_v2(
        &dawsu_application::spectrum::SpectrumFrame {
            bins_db: bins,
            peak_hz: 440.0,
            peak_db: -3.5,
            midi_note: Some(69),
            cents: -4,
        },
        true,
    );
    assert_eq!(spec.len(), crate::SPECTRUM_FRAME_BYTES);
    assert_eq!(spec[0], 0x02);
    assert_eq!(spec[1], crate::SPECTRUM_FLAG_ACTIVE | crate::SPECTRUM_FLAG_HAS_PITCH);
    assert_eq!(u16::from_le_bytes(spec[2..4].try_into().unwrap()), 128);
    assert_eq!(f32::from_le_bytes(spec[4..8].try_into().unwrap()), 440.0);
    assert_eq!(f32::from_le_bytes(spec[8..12].try_into().unwrap()), -3.5);
    assert_eq!(i16::from_le_bytes(spec[12..14].try_into().unwrap()), 69);
    assert_eq!(i16::from_le_bytes(spec[14..16].try_into().unwrap()), -4);
    assert_eq!(i16::from_le_bytes(spec[16..18].try_into().unwrap()), -1850);
    assert_eq!(i16::from_le_bytes(spec[270..272].try_into().unwrap()), -4225);
}

#[test]
fn open_practice_mapset_respects_first_launch_and_works_from_cold_start_and_parked() {
    const CHILD_PROFILE: &str = "DAWSU_PRACTICE_ACTION_TEST_PROFILE";
    if let Some(profile) = std::env::var_os(CHILD_PROFILE) {
        assert_eq!(std::env::var_os("LOCALAPPDATA").as_ref(), Some(&profile));
        let editor = crate::editor_thread(None);

        // Blocked while first_launch == true until a keyboard preset is chosen.
        let blocked = serde_json::to_value(
            crate::ask(&editor, |tx| {
                crate::Request::View(false, Some(Action::OpenPracticeMapset), false, None, tx)
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(blocked["firstLaunch"], true);
        assert_eq!(blocked["busy"], false);
        assert_eq!(blocked["hasDocument"], false);

        // Choose preset, then open practice mapset from cold start.
        let chosen = serde_json::to_value(
            crate::ask(&editor, |tx| {
                crate::Request::View(
                    false,
                    Some(Action::KeyPreset {
                        name: "osu!".into(),
                    }),
                    false,
                    None,
                    tx,
                )
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(chosen["firstLaunch"], false);

        crate::ask(&editor, |tx| {
            crate::Request::View(false, Some(Action::OpenPracticeMapset), false, None, tx)
        })
        .unwrap();

        let wait_loaded = |label: &str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                let view = serde_json::to_value(
                    crate::ask(&editor, |tx| crate::Request::View(true, None, false, None, tx))
                        .unwrap(),
                )
                .unwrap();
                if view["busy"] == false && view["hasDocument"] == true && view["showHome"] == false {
                    return view;
                }
                assert!(std::time::Instant::now() < deadline, "{label}: {view}");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };

        let cold_open = wait_loaded("cold open");
        assert_eq!(cold_open["isPracticeMapset"], true);
        assert_eq!(cold_open["recent"], serde_json::json!([]));
        let first_path = std::path::PathBuf::from(cold_open["scene"]["path"].as_str().unwrap());
        let first_dir = first_path.parent().unwrap().to_path_buf();
        assert!(dawsu_application::practice::is_practice_mapset_path(&first_path));

        // Park via Action::Home -> Route::Parked, then open practice mapset again from Parked.
        let parked = serde_json::to_value(
            crate::ask(&editor, |tx| {
                crate::Request::View(false, Some(Action::Home), false, None, tx)
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(parked["showHome"], true);

        let opening = serde_json::to_value(
            crate::ask(&editor, |tx| {
                crate::Request::View(false, Some(Action::OpenPracticeMapset), false, None, tx)
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(opening["showHome"], true);

        let parked_open = wait_loaded("parked open");
        assert_eq!(parked_open["isPracticeMapset"], true);
        assert_eq!(parked_open["recent"], serde_json::json!([]));
        let second_path = std::path::PathBuf::from(parked_open["scene"]["path"].as_str().unwrap());
        let second_dir = second_path.parent().unwrap().to_path_buf();
        assert_ne!(second_dir, first_dir);

        crate::ask(&editor, |tx| crate::Request::Close(true, tx))
            .unwrap()
            .unwrap();
        drop(editor);
        for dir in [&first_dir, &second_dir] {
            if let Some(name) = dir.file_name() {
                let root = dawsu_application::practice::practice_temp_root();
                let _ = fs::remove_dir_all(root.join("projects").join(name));
                let _ = fs::remove_dir_all(root.join("backups").join(name));
            }
            let _ = fs::remove_dir_all(dir);
        }
        return;
    }

    let profile = std::env::temp_dir().join(format!("dawsu-tauri-practice-{}", std::process::id()));
    let _ = fs::remove_dir_all(&profile);
    fs::create_dir(&profile).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::shared_base_and_misc::open_practice_mapset_respects_first_launch_and_works_from_cold_start_and_parked",
        ])
        .env(CHILD_PROFILE, &profile)
        .env("LOCALAPPDATA", &profile)
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(&profile);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
