use super::helpers::{Fixture, open};
use crate::{Action, PlaybackStart, encode_waveform_bytes, map_preview, snapshot};
use dawsu_application::{Application, WaveBin};
use std::fs;

#[test]
fn ruler_gestures_snap_and_move_locators_without_seeking_or_dirtying_the_map() {
    let fixture = Fixture::new("ruler-locators");
    let path = fixture.0.join("Hard.osu");
    let map = b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,2000,1,0,0:0:0:60:\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    app.seek(500.0);
    let scene = snapshot(&app, true);
    let map_path = &scene["scene"]["path"];
    let drag = |app: &mut Application, path: &serde_json::Value, gesture, from, to| {
        serde_json::from_value::<Action>(serde_json::json!({
            "kind": "ruler_locators", "path": path, "gesture": gesture, "from": from, "to": to,
        })).expect("Tauri must accept ruler locator gestures").run(app);
    };
    let range = |app: &Application| (app.locator_left(), app.locator_right(), app.cycle_enabled());
    drag(&mut app, map_path, "left", 0.0, 1115.0);
    drag(&mut app, map_path, "right", 0.0, 2390.0);
    assert_eq!(range(&app), (1125.0, 2375.0, false));
    // A drag never carries L past R or R past L: it stops on the other locator.
    drag(&mut app, map_path, "left", 1125.0, 3000.0);
    assert_eq!(range(&app), (2375.0, 2375.0, false));
    drag(&mut app, map_path, "left", 2375.0, 1115.0);
    drag(&mut app, map_path, "right", 2375.0, 600.0);
    assert_eq!(range(&app), (1125.0, 1125.0, false));
    drag(&mut app, map_path, "right", 1125.0, 2390.0);
    // Ctrl- or Alt-click (from == to) sets a locator where it lands, as the set shortcuts do.
    drag(&mut app, map_path, "left", 3000.0, 3000.0);
    assert_eq!(range(&app), (3000.0, 2375.0, false));
    drag(&mut app, map_path, "left", 1115.0, 1115.0);
    // Shift-click on a flag jumps the playhead to its locator.
    serde_json::from_value::<Action>(serde_json::json!({"kind": "locator", "end": "right"})).unwrap().run(&mut app);
    assert_eq!(app.position_ms(), 2375.0);
    app.seek(500.0);
    assert_eq!(range(&app), (1125.0, 2375.0, false));

    drag(&mut app, map_path, "draw", 860.0, 385.0);
    assert_eq!(range(&app), (375.0, 875.0, true));
    drag(&mut app, map_path, "move", 620.0, 920.0);
    assert_eq!(range(&app), (625.0, 1125.0, true));
    drag(&mut app, map_path, "move", 1000.0, 0.0);
    assert_eq!(range(&app), (0.0, 500.0, true));

    drag(&mut app, &serde_json::json!("Stale.osu"), "right", 0.0, 1500.0);
    assert_eq!(range(&app), (0.0, 500.0, true));
    assert_eq!(app.position_ms(), 500.0);
    assert!(!app.is_dirty());
    assert_eq!(fs::read(&path).unwrap(), map);
}

#[test]
fn a_ruler_click_moves_the_playhead_to_the_snapped_time_and_leaves_the_locators() {
    let fixture = Fixture::new("ruler-click");
    let path = fixture.0.join("Hard.osu");
    let map = b"osu file format v14

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,2000,1,0,0:0:0:60:
";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    app.seek(500.0);
    let scene = snapshot(&app, true);
    let map_path = scene["scene"]["path"].clone();
    let click = |app: &mut Application, path: &serde_json::Value, at: f64| {
        serde_json::from_value::<Action>(serde_json::json!({
            "kind": "ruler_locators", "path": path, "gesture": "seek", "from": at, "to": at,
        })).expect("Tauri must accept a ruler click").run(app);
    };
    let range = |app: &Application| (app.locator_left(), app.locator_right(), app.cycle_enabled());
    let before = range(&app);
    click(&mut app, &map_path, 1115.0);
    assert_eq!(app.position_ms(), 1125.0);
    assert_eq!(range(&app), before);
    click(&mut app, &serde_json::json!("Stale.osu"), 3000.0);
    assert_eq!(app.position_ms(), 1125.0);
    assert!(!app.is_dirty());
    assert_eq!(fs::read(&path).unwrap(), map);
}

#[test]
fn the_scene_lists_the_times_of_hits_with_no_object() {
    let fixture = Fixture::new("orphaned-hits-scene");
    let path = fixture.0.join("Hard.osu");
    fs::write(&path, b"osu file format v14

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,1000,1,8,0:0:0:60:
").unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    assert_eq!(snapshot(&app, true)["scene"]["orphanedHits"], serde_json::json!([]));
    let row = snapshot(&app, true)["scene"]["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["name"] == "normal-hitclap").unwrap() as i32;
    Action::Pointer {
        phase: "double_click".into(), ms: 1500.0, row, x: 12.0, y: 12.0, value: None, volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt: false,
    }.run(&mut app);
    assert_eq!(snapshot(&app, true)["scene"]["orphanedHits"], serde_json::json!([1500.0]));
    Action::HsMode { on: true }.run(&mut app);
    assert_eq!(snapshot(&app, true)["scene"]["orphanedHits"], serde_json::json!([]), "HS mode makes them circles at Save");
}

#[test]
fn select_double_click_seeks_to_an_existing_hit_and_inserts_once_on_empty_space() {
    let fixture = Fixture::new("select-double-click");
    let path = fixture.0.join("Hard.osu");
    let map = b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:60:\n256,192,2000,1,0,0:0:0:80:\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true);
    let row = scene["scene"]["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["name"] == "normal-hitclap").unwrap() as i32;
    let click = |app: &mut Application, ms, ctrl| {
        Action::Pointer {
            phase: "double_click".into(), ms, row, x: 12.0, y: 12.0, value: None, volume_lanes: None, visible_rows: None, button: None,
            shift: false, ctrl, alt: false,
        }.run(app);
    };

    click(&mut app, 1004.0, false);
    assert_eq!(app.position_ms(), 1000.0);
    assert_eq!(snapshot(&app, true)["scene"]["selection"]["count"], 1);
    assert!(!app.is_playing());
    assert!(!app.is_dirty());
    click(&mut app, 1370.0, true);
    assert!(!app.is_dirty());
    click(&mut app, 1030.0, false);
    assert!(!app.is_dirty(), "an occupied snapped position needs no edit");

    click(&mut app, 1370.0, false);
    let scene = snapshot(&app, true);
    assert_eq!(scene["scene"]["historyPosition"], 1);
    assert!(scene["scene"]["moments"].as_array().unwrap().iter()
        .any(|moment| moment[0] == 1375.0 && moment[3][0][0] == row));
    Action::Command { id: "undo".into() }.run(&mut app);
    assert!(!app.is_dirty());
    assert_eq!(fs::read(&path).unwrap(), map);
}

#[test]
fn waveform_object_click_selects_the_clicked_node_or_the_whole_slider() {
    let fixture = Fixture::new("waveform-selection");
    let path = fixture.0.join("Selection.osu");
    let map = b"osu file format v14\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:60:\n256,192,1500,2,0,L|396:192,2,140\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true)["scene"].clone();
    let click = |app: &mut Application, path: &serde_json::Value, object, time, node, preview| {
        serde_json::from_value::<Action>(serde_json::json!({
            "kind":"waveform_object", "path":path, "object":object, "time":time, "node":node, "preview":preview
        })).expect("Tauri waveform selection action").run(app);
    };
    let selected_times = |app: &Application| {
        let c = &app.document().unwrap().composer;
        let mut times: Vec<_> = c.selected.items().iter()
            .map(|key| c.hitsounds().moments()[key.moment].time().ms()).collect();
        times.sort_by(f64::total_cmp);
        times.dedup();
        times
    };
    click(&mut app, &scene["path"], 1, 2000.0, true, false);
    assert_eq!(app.position_ms(), 2000.0);
    assert_eq!(selected_times(&app), [2000.0]);
    click(&mut app, &scene["path"], 1, 1500.0, false, true);
    assert_eq!(app.position_ms(), 1500.0);
    assert_eq!(selected_times(&app), [1500.0, 2000.0, 2500.0]);
    click(&mut app, &serde_json::json!("Stale.osu"), 0, 1000.0, true, false);
    assert_eq!(app.position_ms(), 1500.0);
    assert!(!app.is_dirty());
    assert_eq!(fs::read(&path).unwrap(), map);
}

#[test]
fn gameplay_overlay_projects_objects_and_skips_unanchored_hits() {
    let fixture = Fixture::new("gameplay-overlay");
    let path = fixture.0.join("Hard.osu");
    let bytes = b"osu file format v14\n\n[General]\nMode:0\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,2,0,L|396:192,2,140\n256,192,3000,8,0,3800,0:0:0:0:\n";
    fs::write(&path, bytes).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    Action::Tool { name: "Draw".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 4250.0, row: 1, x: 4250.0, y: 12.0, value: None, volume_lanes: None, visible_rows: None, button: None,
            shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    assert!(app.document().unwrap().composer.hitsounds().moments().iter().any(|moment|
        moment.place() == dawsu_hitsounding::Place::NoObject && moment.hits().next().is_some()));
    assert_eq!(snapshot(&app, true)["scene"]["gameplay"], serde_json::json!({
        "points": [["circle", 1000.0, 0], ["slider", 1500.0, 1], ["slider", 2000.0, 1], ["slider", 2500.0, 1]],
        "spans": [["slider", 1500.0, 2500.0, 1], ["spinner", 3000.0, 3800.0, 2]],
    }));
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn combos_follow_lazer_new_combo_offsets_and_colours() {
    // osu!lazer at 131fe72: ConvertHitObjectParser.cs:458-525 (an offset counts only with the
    // new-combo bit, never on a spinner), OsuHitObject.cs:193-196 (a spinner never starts a combo,
    // even flagged or first; the first object and the one after a spinner do), LegacyBeatmapSkin.cs:90
    // (map colours by the index with offsets), LegacyDecoder.cs:130-145 ([Colours] in file order).
    let fixture = Fixture::new("combos");
    let path = fixture.0.join("Hard.osu");
    let bytes = b"osu file format v14

[General]
Mode:0

[Difficulty]
SliderMultiplier:1.4
SliderTickRate:1

[TimingPoints]
0,500,4,1,0,100,1,0

[Colours]
Combo2 : 0,255,0
Combo1 : 255,0,0
SliderBorder : 1,2,3
Combo3 : 0,0,255

[HitObjects]
256,192,1000,5,0,0:0:0:0:
256,192,1500,1,0,0:0:0:0:
256,192,2000,2,0,L|396:192,1,140
256,192,3000,12,0,3800,0:0:0:0:
256,192,4000,1,0,0:0:0:0:
256,192,4500,21,0,0:0:0:0:
256,192,5000,17,0,0:0:0:0:
";
    fs::write(&path, bytes).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    assert_eq!(snapshot(&app, true)["scene"]["combos"], serde_json::json!({
        // Map colours in file order: Combo2 first. SliderBorder is not a combo colour.
        "colours": ["#00ff00", "#ff0000", "#0000ff"],
        // [number in combo, colour] per object in file order. The first combo takes colour 1 % 3; the
        // flagged spinner stays in it; the circle after it starts the next.
        "objects": [[1, 1], [2, 1], [3, 1], [4, 1], [1, 2], [1, 1], [2, 1]],
    }));
    assert_eq!(fs::read(&path).unwrap(), bytes);

    // Without [Colours] lazer's default skin colours apply, by the index without offsets.
    let plain = fixture.0.join("Plain.osu");
    fs::write(&plain, b"osu file format v14

[General]
Mode:0

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,1000,1,0,0:0:0:0:
256,192,1500,85,0,0:0:0:0:
").unwrap();
    open(&mut app, &plain);
    assert_eq!(snapshot(&app, true)["scene"]["combos"], serde_json::json!({
        "colours": ["#ffc000", "#00ca00", "#127cff", "#f21839"],
        "objects": [[1, 1], [1, 2]],
    }));

    // A spinner first, even flagged, starts no combo: it takes combo 0, the circle after it combo 1.
    let spun = fixture.0.join("Spun.osu");
    fs::write(&spun, b"osu file format v14

[General]
Mode:0

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,1000,12,0,1800,0:0:0:0:
256,192,2000,1,0,0:0:0:0:
").unwrap();
    open(&mut app, &spun);
    assert_eq!(snapshot(&app, true)["scene"]["combos"]["objects"], serde_json::json!([[1, 0], [1, 1]]));
}

#[test]
fn view_keeps_the_fit_zoom_of_a_30_minute_song() {
    let fixture = Fixture::new("fit-zoom");
    let path = fixture.0.join("Hard.osu");
    fs::write(&path, b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n").unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    // Fit shows a 30-minute song in a 1000 px timeline.
    let fit = 1000.0 / (30.0 * 60_000.0);
    serde_json::from_value::<Action>(serde_json::json!({"kind": "view", "start": 0.0, "zoom": fit, "row_height": 30.0}))
        .unwrap().run(&mut app);
    assert_eq!(app.zoom(), fit);
}

#[test]
fn go_to_end_seeks_to_the_duration_the_view_reports() {
    let fixture = Fixture::new("go-to-end");
    let path = fixture.0.join("Hard.osu");
    fs::write(&path, b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n").unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    // The transport's Go to end sends a seek to view.duration; past it, seek stops at the end.
    let duration = snapshot(&app, false)["duration"].as_f64().unwrap();
    assert!(duration > 1000.0);
    for ms in [duration, duration + 5000.0] {
        app.seek(0.0);
        serde_json::from_value::<Action>(serde_json::json!({"kind": "seek", "ms": ms})).unwrap().run(&mut app);
        assert_eq!(snapshot(&app, false)["position"].as_f64(), Some(duration));
    }
}

#[test]
fn time_signature_action_changes_ruler_and_position_without_changing_map() {
    let fixture = Fixture::new("time-signature");
    let path = fixture.0.join("Meter.osu");
    let bytes = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,1,100,1,0\n\n[HitObjects]\n256,192,2000,1,0,0:0:0:0:\n";
    fs::write(&path, bytes).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let action: Action = serde_json::from_value(serde_json::json!({
        "kind": "time_signatures",
        "map": app.document().unwrap().path,
        "value": [{ "bar": 2, "numerator": 7, "denominator": 8 }]
    })).unwrap();
    action.run(&mut app);
    app.seek(2250.0);
    let state = snapshot(&app, true);
    assert_eq!(state["scene"]["editorBars"][1]["start"], 2000.0);
    assert_eq!(state["scene"]["editorBars"][1]["beat"], 250.0);
    assert_eq!(state["scene"]["sections"][0][1], 500.0);
    let position = state["musicalPosition"].as_str().unwrap();
    assert!(position.starts_with("2.2.000"), "{position}");
    assert!(position.contains("120.0 BPM · 7/8"), "{position}");
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn legacy_transport_keys_share_the_toolbar_behaviour_and_playback_start() {
    let fixture = Fixture::new("transport-shortcuts");
    let path = fixture.0.join("Hard.osu");
    let bytes = b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    fs::write(&path, bytes).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let press = |app: &mut Application, key: &str, code: &str, mode: &mut PlaybackStart| {
        Action::Key {
            key: key.into(), code: code.into(), ctrl: false, shift: false, alt: false, lane_view: None,
        }.run_with_start(app, mode);
    };
    let mut mode = PlaybackStart::Left;
    for preset in ["DAW", "osu!"] {
        app.choose_key_preset(preset);
        for (key, code) in [("Insert", "Numpad0"), ("Home", "Home")] {
            app.seek(1200.0);
            press(&mut app, key, code, &mut mode);
            assert_eq!(app.position_ms(), 0.0, "{preset}: {code}");
        }
    }
    app.set_locator_left(750.0);
    for (key, code) in [("x", "KeyX"), ("c", "KeyC"), (" ", "Space")] {
        app.seek(100.0);
        press(&mut app, key, code, &mut mode);
        assert_eq!(app.position_ms(), 750.0, "{code}");
    }
    assert!(!app.is_dirty());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn opened_song_exposes_three_frequency_bands_at_their_original_positions() {
    let fixture = Fixture::new("three-band-waveform");
    let map = fixture.0.join("Test.osu");
    let bytes = b"osu file format v14\n\n[General]\nMode:0\nAudioFilename: song.wav\n\n[TimingPoints]\n0,500,4,1,1,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    fs::write(&map, bytes).unwrap();
    let frames = 19_200u32;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + frames * 3 * 2).to_le_bytes());
    wav.extend(b"WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00");
    wav.extend(48_000u32.to_le_bytes());
    wav.extend(96_000u32.to_le_bytes());
    wav.extend(b"\x02\x00\x10\x00data");
    wav.extend((frames * 3 * 2).to_le_bytes());
    for (frequency, amplitude) in [(70.0, 0.4), (650.0, 0.6), (8_000.0, 0.8)] {
        for frame in 0..frames {
            let phase = std::f64::consts::TAU * f64::from(frame) * frequency / 48_000.0;
            let sample = (phase.cos() * amplitude * f64::from(i16::MAX)).round() as i16;
            wav.extend(sample.to_le_bytes());
        }
    }
    let song = fixture.0.join("song.wav");
    fs::write(&song, &wav).unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let state = snapshot(&app, true);
    assert_eq!(state["duration"], 1200.0);
    assert!(state["scene"].get("waveform").is_none());
    let bins = &app.waveform;
    assert!(bins.len() <= 4000);
    for (segment, amplitude) in [0.4, 0.6, 0.8].into_iter().enumerate() {
        let first = (segment * 3 + 1) * bins.len() / 9;
        let last = (segment * 3 + 2) * bins.len() / 9;
        let mut levels = [0.0f64; 4];
        for bin in &bins[first..last] {
            for (index, level) in levels.iter_mut().enumerate() {
                let value = [bin.peak, bin.low, bin.mid, bin.high][index];
                *level = level.max(f64::from(value));
            }
        }
        assert!((levels[0] - amplitude).abs() < 0.02, "{levels:?}");
        for other in (1..=3).filter(|index| *index != segment + 1) {
            assert!(levels[segment + 1] > 2.0 * levels[other], "{levels:?}");
        }
    }
    assert_eq!(fs::read(&song).unwrap(), wav);
    assert_eq!(fs::read(&map).unwrap(), bytes);
}

#[test]
fn the_map_preview_sends_stacked_objects_with_combos_and_slider_paths() {
    let fixture = Fixture::new("map-preview");
    let path = fixture.0.join("Preview.osu");
    // Two circles on one spot stack; a slider from 100,100 runs 100 px right over one beat.
    let map = b"osu file format v14\n\n[General]\nMode: 0\nStackLeniency: 0.7\n\n[Difficulty]\nCircleSize:4\nApproachRate:9\nSliderMultiplier:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n200,200,1000,5,0\n200,200,1100,1,0\n100,100,2000,6,0,L|200:100,1,100\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let opened = app.document().unwrap().path.clone();
    let preview = map_preview(&app, &opened).unwrap();
    assert_eq!(preview["preempt"], 600.0);
    assert_eq!(preview["key"], snapshot(&app, true)["scene"]["previewKey"]);
    let objects = preview["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 3);
    let scale = preview["radius"].as_f64().unwrap() / 64.0;
    // The first circle sits one stack step up and left of the second.
    let step = (scale * 6.4 * 100.0).round() / 100.0;
    assert!((objects[0]["x"].as_f64().unwrap() - (200.0 - step)).abs() < 0.02, "{preview}");
    assert_eq!(objects[1]["x"], 200.0);
    assert_eq!(objects[0]["combo"], 1);
    assert_eq!(objects[1]["combo"], 2);
    // The map has no colours, so the preview takes lazer's Argon six by the plain combo index:
    // the circles are combo 1, the slider starts combo 2.
    assert_eq!(preview["colours"].as_array().unwrap().len(), 6);
    assert_eq!(preview["colours"][0], "#f17400");
    assert_eq!(objects[0]["colour"], 1);
    assert_eq!(objects[2]["kind"], "slider");
    assert_eq!(objects[2]["combo"], 1);
    assert_eq!(objects[2]["colour"], 2);
    assert_eq!(objects[2]["end"], 2500.0);
    assert_eq!(objects[2]["path"], serde_json::json!([100.0, 100.0, 200.0, 100.0]));
    assert_eq!(objects[2]["lengths"], serde_json::json!([0.0, 100.0]));
    assert_eq!(objects[2]["spans"], 1);
    // One beat long at tick rate 1: the only tick would land on the tail, so there is none.
    assert_eq!(objects[2]["ticks"], serde_json::json!([]));
    assert!(map_preview(&app, &fixture.0.join("Other.osu")).is_err());
}

#[test]
fn encode_waveform_bytes_round_trips_exact_f32_bins() {
    assert!(encode_waveform_bytes(&[]).is_empty());

    let mut app = Application::headless();
    app.waveform = vec![
        WaveBin {
            peak: 0.912_345_6,
            low: 0.123_456_7,
            mid: 0.456_789_1,
            high: 0.789_012_3,
            amp_l: 0.812_5,
            amp_r: 0.637_5,
            true_peak: 0.912_345_6,
        },
        WaveBin {
            peak: 0.0,
            low: 1.0,
            mid: 0.5,
            high: 0.25,
            amp_l: 0.000_976_562_5,
            amp_r: 0.999_999_9,
            true_peak: 0.999_999_9,
        },
    ];
    let bytes = encode_waveform_bytes(&app.waveform);
    assert_eq!(bytes.len(), app.waveform.len() * 6 * 4);
    let decoded: Vec<WaveBin> = bytes
        .as_chunks::<24>()
        .0
        .iter()
        .map(|chunk| {
            let f = |offset: usize| {
                f32::from_le_bytes(chunk[offset..offset + 4].try_into().unwrap())
            };
            WaveBin {
                peak: f(0),
                low: f(4),
                mid: f(8),
                high: f(12),
                amp_l: f(16),
                amp_r: f(20),
                true_peak: f(0),
            }
        })
        .collect();
    assert_eq!(decoded.len(), app.waveform.len());
    for (got, expected) in decoded.iter().zip(app.waveform.iter()) {
        assert_eq!(got.peak.to_bits(), expected.peak.to_bits());
        assert_eq!(got.low.to_bits(), expected.low.to_bits());
        assert_eq!(got.mid.to_bits(), expected.mid.to_bits());
        assert_eq!(got.high.to_bits(), expected.high.to_bits());
        assert_eq!(got.amp_l.to_bits(), expected.amp_l.to_bits());
        assert_eq!(got.amp_r.to_bits(), expected.amp_r.to_bits());
    }
}

#[test]
fn encode_waveform_pyramid_bytes_packs_dwfp_r4_levels_and_clip_mask() {
    use crate::encode_waveform_pyramid_bytes;
    assert!(encode_waveform_pyramid_bytes(&[]).is_empty());
    let mut bins = vec![
        WaveBin {
            peak: 0.4,
            low: 0.5,
            mid: 0.3,
            high: 0.2,
            amp_l: 0.4,
            amp_r: 0.35,
            true_peak: 0.4,
        };
        1024
    ];
    bins[500].peak = 1.0;
    bins[500].amp_l = 1.0;
    bins[500].true_peak = 1.06;

    let bytes = encode_waveform_pyramid_bytes(&bins);
    assert_eq!(&bytes[0..4], b"DWFP");
    let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
    let levels = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
    let stride = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(version, 1);
    assert_eq!(levels, 2); // 1024 -> 256 (<= 256 stops)
    assert_eq!(stride, 6);

    let l0_count = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let l0_stride = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    let l1_count = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
    let l1_stride = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    assert_eq!((l0_count, l0_stride), (1024, 1));
    assert_eq!((l1_count, l1_stride), (256, 4));

    let l0_mask_offset = 12 + 2 * 8 + l0_count * 24;
    let l0_mask_len = l0_count.div_ceil(8).next_multiple_of(4);
    assert_ne!(bytes[l0_mask_offset + (500 >> 3)] & (1u8 << (500 & 7)), 0);
    assert_eq!(bytes[l0_mask_offset + (10 >> 3)] & (1u8 << (10 & 7)), 0);

    let l1_offset = l0_mask_offset + l0_mask_len;
    let l1_peak_125 = f32::from_le_bytes(bytes[l1_offset + 125 * 24..l1_offset + 125 * 24 + 4].try_into().unwrap());
    assert_eq!(l1_peak_125, 1.0);
    let l1_mask_offset = l1_offset + l1_count * 24;
    assert_ne!(bytes[l1_mask_offset + (125 >> 3)] & (1u8 << (125 & 7)), 0);
}
