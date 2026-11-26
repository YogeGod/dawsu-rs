use super::helpers::{Fixture, open};
use crate::{Action, PlaybackStart, scene_lane_key, snapshot, snapshot_with_static};
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
fn typed_scene_matches_legacy_json_bytes_exactly() {
    use dawsu_application::lane_name;
    use dawsu_editor::{
        keys::{Key, KeyCombination, Mods},
        tempo::SnapMode,
        tools::ToolPreview,
        volume_ramp::ramp_at,
    };
    use dawsu_foundation::{CustomIndex, SampleIdentity, SongTime, Sound};
    use dawsu_hitsounding::{LaneSource, Place};
    use serde_json::{Value, json};

    fn legacy_snapshot_json(app: &Application, include_scene: bool, include_static: bool) -> Value {
        let document = app.document();
        let position = app.position_ms();
        let musical_position = document.map(|document| {
            let composer = &document.composer;
            let time = SongTime::from_ms(position);
            let tempo = composer.tempo.section_at(time);
            let (beat, (numerator, denominator)) = document.editor_bar_at(time);
            let tick = (beat.beat_fraction * 480.0 + 1e-9).floor() as u32;
            let snap = if composer.snap_enabled {
                format!("{}/{}", composer.spacing.beats(), composer.spacing.divisions())
            } else {
                "Snap off".into()
            };
            format!(
                "{}.{}.{:03} · {:.1} BPM · {numerator}/{denominator} · {snap}",
                beat.bar, beat.beat, tick.min(479), 60_000.0 / tempo.beat_length_ms,
            )
        });
        let scene = if include_scene {
            document.map(|d| {
                let c = &d.composer;
                let model = c.hitsounds();
                let row_by_lane: std::collections::HashMap<_, _> =
                    c.lanes.iter().enumerate().map(|(row, lane)| (lane, row)).collect();
                let mut hit_counts = std::collections::HashMap::new();
                for lane in model.moments().iter().flat_map(|moment| moment.hits()) {
                    *hit_counts.entry(lane).or_insert(0) += 1;
                }
                let (lane_order, backing_slots) = crate::lane_header_rows(d, &|lane| hit_counts.contains_key(lane));
                let lanes = c.lanes.iter().enumerate().map(|(row, lane)| {
                    let slot = backing_slots.get(&row).copied().unwrap_or(lane);
                    let (family, index, group) = match slot {
                        SampleIdentity::Standard { set, sound, index } => {
                            let family = lane_name(&SampleIdentity::Standard { set: *set, sound: *sound, index: CustomIndex::new(0) });
                            (Some(family), Some(index.get()), Some(crate::sample_set_name(*set)))
                        }
                        SampleIdentity::File(_) => (None, None, None),
                    };
                    let sample = model.lane(lane).and_then(|lane| match &lane.source {
                        LaneSource::File(name) => Some(name),
                        LaneSource::Skin => None,
                    });
                    let hit_count = hit_counts.get(lane).copied().unwrap_or(0);
                    let read_only = matches!(slot, SampleIdentity::Standard { sound: Sound::Slider(_), .. });
                    let protected = c.protect_normals && hit_count > 0
                        && matches!(slot, SampleIdentity::Standard { sound: Sound::Hit(dawsu_foundation::Hitsound::Normal), .. });
                    let plays_as = model.lane(lane).map(|m| m.plays_as()).unwrap_or_else(|| lane.clone());
                    json!({
                        "family": family, "index": index, "group": group, "key": scene_lane_key(slot),
                        "sourceKey": scene_lane_key(lane), "sample": sample,
                        "colourKey": dawsu_application::lane_layout_key(slot),
                        "name": lane_name(slot), "muted": app.is_muted(lane), "solo": app.is_solo(lane),
                        "gain": app.lane_gain(&plays_as),
                        "canAddLayer": index.is_some(),
                        "canReplace": !matches!(lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. }),
                        "hitCount": hit_count, "canDelete": !read_only && !protected,
                    })
                }).collect::<Vec<_>>();
                let selected: std::collections::HashSet<_> = c.selected.items().iter()
                    .map(|key| (key.moment, &key.lane)).collect();
                let listen_muted: std::collections::HashSet<_> = c.listen_muted.iter()
                    .map(|key| (key.moment, &key.lane)).collect();
                let moments = c.hitsounds().moments().iter().enumerate().map(|(i, moment)| {
                    let hits = moment.hits().filter_map(|lane| row_by_lane.get(lane)
                        .map(|row| {
                            let selected = selected.contains(&(i, lane));
                            let muted = listen_muted.contains(&(i, lane));
                            let gain = moment.hit_volume(lane);
                            if gain != moment.volume() {
                                json!([row, selected, muted, gain])
                            } else if muted {
                                json!([row, selected, true])
                            } else {
                                json!([row, selected])
                            }
                        })).collect::<Vec<_>>();
                    json!([moment.time().ms(), moment.reference_volume(), moment.place() != Place::NoObject, hits])
                }).collect::<Vec<_>>();
                let sections = include_static.then(|| {
                    c.tempo.sections().iter()
                        .map(|s| json!([s.start.ms(), s.beat_length_ms, s.beats_per_bar]))
                        .collect::<Vec<_>>()
                });
                let ramp_preview = match c.preview {
                    ToolPreview::RampLine { from, to, shape } => {
                        let (from, to) = if from.0 > to.0 { (to, from) } else { (from, to) };
                        Some((0..=16).map(|sample| {
                            let fraction = sample as f64 / 16.0;
                            let time = from.0.ms() + (to.0.ms() - from.0.ms()) * fraction;
                            (time, ramp_at(shape, fraction, from.1, to.1))
                        }).collect::<Vec<_>>())
                    }
                    _ => None,
                };
                let mut selection = crate::selection::summary(model, selected.iter().copied())
                    .map(|s| serde_json::to_value(s).unwrap());
                if let Some(selection) = selection.as_mut()
                    && let Some(index) = d.selected_slider_body_index()
                {
                    let body = &model.bodies()[index];
                    selection["sliderBody"] = json!({
                        "index": index,
                        "start": body.start().ms(),
                        "set": body.set().map(crate::sample_set_name),
                        "whistle": body.whistle(),
                        "whistleSet": body.whistle_set().map_or("auto", crate::sample_set_name),
                        "volume": body.volume(),
                    });
                }
                let mut scene = json!({
                    "title": d.title, "mapTitle": d.map_title(), "version": d.version(),
                    "path": d.path, "difficulties": d.difficulties, "coverage": d.coverage_summary(),
                    "lanes": lanes, "laneOrder": lane_order, "moments": moments,
                    "previewKey": app.document_generation(),
                    "orphanedHits": dawsu_application::orphaned_hit_times(model, d.hs_mode),
                    "laneRevision": c.history.version(),
                    "selection": selection, "history": c.history.steps(), "historyPosition": c.history.position(),
                    "snap": c.snap_enabled, "snapMode": match c.snap_mode { SnapMode::Grid => "grid", SnapMode::GridRelative => "relative", SnapMode::Objects => "objects" },
                    "beats": c.spacing.beats(), "divisor": c.spacing.divisions(),
                    "insertVolume": c.insert_volume, "protect": c.protect_normals, "hs": d.hs_mode,
                    "hsDifficulties": d.difficulties.iter().filter(|p| crate::is_osu_hs_diff(p)).cloned().collect::<Vec<_>>(),
                    "viewStart": c.view_start, "zoom": app.zoom, "rowHeight": app.row_height,
                    "range": c.range.map(|r| [r.from.ms(), r.to.ms()]),
                    "repeatGap": app.requested_edit().filter(|&id| id == "repeat").and_then(|_| c.repeat_gap()),
                    "patternStampStep": app.requested_edit().filter(|&id| id == "pattern_stamp").and_then(|_| c.pattern_stamp_step()),
                    "laneLayout": d.lane_layout(),
                    "timeSignatures": d.time_signatures(), "editorBars": d.editor_bars(),
                    "bookmarks": d.bookmarks().iter().map(|b| json!({"time": b.time.ms(), "label": b.label})).collect::<Vec<_>>(),
                    "rampPreview": ramp_preview,
                    "placement": c.selected_placement(),
                });
                if include_static {
                    scene["sections"] = json!(sections.unwrap_or_default());
                    scene["gameplay"] = serde_json::to_value(crate::gameplay_overlay(d)).unwrap();
                    scene["combos"] = serde_json::to_value(crate::combo_overlay(d)).unwrap();
                } else if d.hs_mode {
                    scene["gameplay"] = serde_json::to_value(crate::gameplay_overlay(d)).unwrap();
                }
                scene
            })
        } else {
            None
        };
        let mut result = json!({
            "scene": scene, "hasDocument": document.is_some(), "busy": app.busy, "playing": app.is_playing(),
            "laneCount": document.map_or(0, |d| d.composer.lanes.len()),
            "position": position, "musicalPosition": musical_position, "duration": app.duration_ms(), "dirty": app.is_dirty(),
            "status": app.status, "error": app.error, "warnings": app.warnings, "speed": app.playback_speed,
            "song": app.preferences.song_gain, "main": app.preferences.master_gain,
            "returnOnStop": app.preferences.return_on_stop,
            "cleanRedundantGreenlines": app.preferences.clean_redundant_greenlines,
            "songMuted": app.preferences.song_muted,
            "audition": app.preferences.audition, "offset": app.song_offset,
            "left": app.locator_left, "right": app.locator_right, "cycle": app.cycle, "follow": app.preferences.follow,
            "tool": format!("{:?}", app.tool()), "pendingOpen": app.pending_open.is_some(),
            "keyPreset": app.preferences.key_preset,
            "rampShape": format!("{:?}", app.ramp_shape()),
            "revert": app.revert_requested, "review": Value::Null, "activeLane": app.active_lane,
            "saveProblemReview": app.save_problem_review_requested() && !app.output_problems().is_empty(), "objectLines": app.preferences.object_lines,
            "classicBrowser": app.preferences.classic_browser,
            "hitsoundLibrary": app.preferences.hitsound_library,
            "panels": {
                "browser": app.preferences.show_browser,
                "inspector": app.preferences.show_inspector,
                "controller": app.preferences.show_controller,
                "laneNames": app.preferences.lane_names,
            },
            "recent": app.preferences.recent, "firstLaunch": app.first_launch,
        });
        if document.is_none() || include_static {
            result["recentMeta"] = json!(app.preferences.recent.iter().map(|p| crate::cached_recent_meta(p)).collect::<Vec<_>>());
        }
        result["hsMode"] = json!(document.is_some_and(|d| d.hs_mode));
        result["hasOriginalSnapshot"] = json!(app.has_original_snapshot());
        result["canUndoRevert"] = json!(app.can_undo_revert());
        result["songsFolder"] = json!((!app.preferences.songs_folder.as_os_str().is_empty()).then_some(&app.preferences.songs_folder));
        result["outputProblems"] = json!([]);
        result["previewWindow"] = json!(app.preferences.preview_window.is_some_and(|place| place.open));
        result["outputDevice"] = json!(app.device_name());
        result["saveReview"] = json!(app.save_review_files());
        result["soundSetPrompts"] = json!([]);
        result["songSolo"] = json!(app.song_solo());
        result["songLoading"] = json!(app.song_loading);
        result["waveformRevision"] = json!(app.waveform_revision());
        result["drumVst"] = json!(Value::Null);
        result["hitsounds"] = json!(app.preferences.hitsound_gain);
        result["hitsoundsMuted"] = json!(app.preferences.hitsounds_muted);
        result["hitsoundsSolo"] = json!(app.hitsounds_solo());
        let (master_peak, song_peak, hitsound_peak) = app.audio_peaks();
        result["peaks"] = json!({
            "master": master_peak,
            "song": song_peak,
            "hitsounds": hitsound_peak,
        });
        result["editDialog"] = json!(app.requested_edit());
        result["quantizeStrength"] = json!(app.quantize_strength());
        let tab_routes = [false, true].into_iter().flat_map(|ctrl| {
            [false, true].into_iter().flat_map(move |shift| {
                [false, true].into_iter().filter_map(move |alt| {
                    let mut mods = Mods::NONE;
                    for (pressed, flag) in [(ctrl, Mods::CTRL), (shift, Mods::SHIFT), (alt, Mods::ALT)] {
                        if pressed {
                            mods = mods.union(flag);
                        }
                    }
                    let id = app.keymap.command_for(KeyCombination::new(Key::Tab, mods))?;
                    Some((ctrl, shift, alt, matches!(id, "open" | "help")))
                })
            })
        }).collect::<Vec<_>>();
        result["tabRoutes"] = json!(tab_routes);
        result["keepPitch"] = json!(app.preferences.keep_pitch);
        result["autoCheckUpdates"] = json!(app.preferences.auto_check_updates);
        result["stepPadsMode"] = json!(app.preferences.step_pads_mode);
        result["reloadOsuAfterSave"] = json!(app.preferences.reload_osu_after_save);
        result["tapRecordedCount"] = json!(app.tap_recorded_count());
        result["tapRecordingArmed"] = json!(app.tap_recording_armed());
        result
    }

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
        let old = legacy_snapshot_json(&app, true, include_static);
        let new = snapshot_with_static(&app, true, include_static);
        if include_static {
            let mut has_plain = false;
            let mut has_selected = false;
            let mut has_muted = false;
            let mut has_custom_gain = false;
            for moment in old["scene"]["moments"].as_array().unwrap() {
                for hit in moment[3].as_array().unwrap() {
                    let parts = hit.as_array().unwrap();
                    match parts.as_slice() {
                        [_, sel] if sel == &serde_json::json!(false) => has_plain = true,
                        [_, sel] if sel == &serde_json::json!(true) => has_selected = true,
                        [_, _, muted] if muted == &serde_json::json!(true) => has_muted = true,
                        [_, _, _, _] => has_custom_gain = true,
                        _ => {}
                    }
                }
            }
            assert!(has_plain && has_selected && has_muted && has_custom_gain,
                "fixture should cover all hit shapes: plain={has_plain} selected={has_selected} muted={has_muted} gain={has_custom_gain}");
        }
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            serde_json::to_string(&new).unwrap(),
            "mismatch in standard mode (include_static={include_static})"
        );
    }

    // Toggle HS mode on and verify both include_static=true and include_static=false.
    Action::HsMode { on: true }.run(&mut app);
    for include_static in [true, false] {
        let old = legacy_snapshot_json(&app, true, include_static);
        let new = snapshot_with_static(&app, true, include_static);
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            serde_json::to_string(&new).unwrap(),
            "mismatch in HS mode (include_static={include_static})"
        );
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
