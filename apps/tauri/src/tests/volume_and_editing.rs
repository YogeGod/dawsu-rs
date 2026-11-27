use super::helpers::{Fixture, isolated_save_fixture, open, skin_sounds, wait_for_job};
use crate::{Action, lane_add_choices, lane_by_key, scene_lane_key, snapshot};
use dawsu_application::Application;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

#[test]
fn lane_actions_hide_restore_and_undo_without_removing_the_sample() {
    let fixture = Fixture::new("lane-actions");
    let map_path = fixture.0.join("Lane.osu");
    let sample_path = fixture.0.join("normal-hitclap.wav");
    let original = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,1,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
    let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    fs::write(&map_path, original).unwrap();
    fs::write(&sample_path, wave).unwrap();
    let mut app = Application::headless();
    open(&mut app, &map_path);
    let scene = |app: &Application| snapshot(app, true)["scene"].clone();
    let before = scene(&app);
    let row = before["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["sample"] == "normal-hitclap.wav").unwrap();
    let key = &before["lanes"][row]["sourceKey"];
    let target = serde_json::json!({"map": before["path"], "lane": key});
    let visible = |scene: &serde_json::Value| scene["laneOrder"].as_array().unwrap().iter()
        .any(|value| value.as_u64() == Some(row as u64));
    assert!(visible(&before));
    assert_eq!(before["lanes"][row]["hitCount"], 1);
    let revision = before["laneRevision"].as_u64().unwrap();
    let remove = |app: &mut Application, revision| {
        serde_json::from_value::<Action>(serde_json::json!({
            "kind": "delete_lane", "target": target, "revision": revision,
        })).unwrap().run(app);
    };
    remove(&mut app, revision + 1);
    assert!(app.take_error().is_some());
    assert!(visible(&scene(&app)));
    remove(&mut app, revision);
    assert!(app.error().is_none(), "{:?}", app.error());
    let after = scene(&app);
    assert!(!visible(&after));
    assert_eq!(after["lanes"][row]["hitCount"], 0);
    let map = app.document().unwrap().path.clone();
    assert!(lane_add_choices(&app, &map).unwrap().as_array().unwrap().iter().any(|lane| lane["key"] == *key));

    Action::Command { id: "undo".into() }.run(&mut app);
    let undone = scene(&app);
    assert!(visible(&undone));
    assert_eq!(undone["lanes"][row]["hitCount"], 1);
    Action::Command { id: "redo".into() }.run(&mut app);
    serde_json::from_value::<Action>(serde_json::json!({
        "kind": "show_lane", "target": target,
    })).unwrap().run(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    let restored = scene(&app);
    assert!(visible(&restored));
    assert_eq!(restored["lanes"][row]["hitCount"], 0);
    let fresh_path = fixture.0.join("fresh.wav");
    fs::write(&fresh_path, wave).unwrap();
    let choices = lane_add_choices(&app, &map).unwrap();
    let fresh = choices.as_array().unwrap().iter().find(|lane| lane["name"] == "fresh.wav").unwrap();
    serde_json::from_value::<Action>(serde_json::json!({
        "kind": "show_lane", "target": {"map": map, "lane": fresh["key"]},
    })).unwrap().run(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    let state = scene(&app);
    assert!(state["laneOrder"].as_array().unwrap().iter().any(|index| {
        state["lanes"][index.as_u64().unwrap() as usize]["sourceKey"] == fresh["key"]
    }));
    assert_eq!(fs::read(&fresh_path).unwrap(), wave);
    assert_eq!(fs::read(&sample_path).unwrap(), wave);
    assert_eq!(fs::read(&map_path).unwrap(), original);
}

#[test]
fn grab_and_mute_are_view_only_and_erasing_a_stroke_is_one_undo_step() {
    let root = Fixture::new("remaining-tools");
    let path = root.0.join("Tools.osu");
    let map = concat!("osu file format v14\n\n[General]\nMode:0\n",
        "\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n",
        "256,192,1000,1,8,0:0:0:0:\n256,192,1500,1,8,0:0:0:0:\n");
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true);
    let row = scene["scene"]["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["name"] == "normal-hitclap").unwrap() as i32;
    let clap = app.document().unwrap().composer.lanes[row as usize].clone();
    let tool = |app: &mut Application, name: &str| Action::Tool { name: name.into() }.run(app);
    let point = |app: &mut Application, phase: &str, ms| {
        Action::Pointer {
            phase: phase.into(), ms, row, x: ms, y: 0.0,
            value: None, volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(app);
    };
    let clap_count = |app: &Application| app.document().unwrap().composer.hitsounds()
        .moments().iter().filter(|moment| moment.has_hit(&clap)).count();
    assert_eq!(clap_count(&app), 2);

    app.set_view_start(900.0);
    tool(&mut app, "Grab");
    point(&mut app, "down", 1050.0);
    point(&mut app, "move", 1000.0);
    point(&mut app, "up", 1000.0);
    assert_eq!(snapshot(&app, true)["scene"]["viewStart"], 950.0);

    tool(&mut app, "Mute");
    point(&mut app, "down", 1000.0);
    point(&mut app, "up", 1000.0);
    let scene = snapshot(&app, true);
    let hit = scene["scene"]["moments"][0][3].as_array().unwrap().iter()
        .find(|hit| hit[0] == row).unwrap();
    assert_eq!(hit[2], true, "the listening-only mute is visible in the scene");
    point(&mut app, "down", 1000.0);
    point(&mut app, "up", 1000.0);
    assert!(app.document().unwrap().composer.listen_muted.is_empty());
    assert_eq!(clap_count(&app), 2);
    assert!(!app.is_dirty());
    assert_eq!(snapshot(&app, true)["scene"]["historyPosition"], 0);

    tool(&mut app, "Erase");
    point(&mut app, "down", 1000.0);
    point(&mut app, "move", 1500.0);
    point(&mut app, "up", 1500.0);
    assert_eq!(clap_count(&app), 0);
    assert_eq!(snapshot(&app, true)["scene"]["historyPosition"], 1);
    Action::Command { id: "undo".into() }.run(&mut app);
    assert_eq!(clap_count(&app), 2);
    assert_eq!(fs::read_to_string(&path).unwrap(), map);
}

#[test]
fn every_lane_key_the_scene_sends_comes_from_one_helper_and_finds_its_lane() {
    let fixture = Fixture::new("lane-keys");
    let path = fixture.0.join("Hard.osu");
    fs::write(&path, b"osu file format v14

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,1000,1,8,0:0:0:0:
").unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true);
    for (row, lane) in app.document().unwrap().composer.lanes.iter().enumerate() {
        assert_eq!(scene["scene"]["lanes"][row]["sourceKey"], scene_lane_key(lane));
        assert_eq!(&lane_by_key(&app, &scene_lane_key(lane)).unwrap(), lane);
    }
}

#[test]
fn volume_scene_contains_one_shared_level_per_circle_and_slider_node() {
    let root = Fixture::new("volume-anchors");
    let path = root.0.join("Slider.osu");
    fs::write(
        &path,
        "osu file format v14\n\n[General]\nMode:0\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n0,500,4,1,0,75,1,0\n\n[HitObjects]\n256,192,1000,1,10,0:0:0:65:\n256,192,2000,2,0,L|396:192,2,140,0|0|0,0:0|0:0|0:0,0:0:0:0:\n",
    ).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true)["scene"].clone();
    let moments = scene["moments"].as_array().unwrap();
    assert_eq!(moments.len(), 4);
    assert_eq!(moments[0][0], 1000.0);
    assert_eq!(moments[0][1], 65);
    assert_eq!(moments[0][3].as_array().unwrap().len(), 3);
    for (moment, time) in moments.iter().skip(1).zip([2000.0, 2500.0, 3000.0]) {
        assert_eq!(moment[0], time);
        assert_eq!(moment[1], 75);
    }
}

#[test]
fn ctrl_drag_spreads_and_squeezes_volumes_with_different_effective_gains() {
    let root = Fixture::new("volume-spread");
    let path = root.0.join("Controller.osu");
    let map = "osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:60:\n256,192,1500,1,8,0:0:0:80:\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true)["scene"].clone();
    let clap = scene["lanes"].as_array().unwrap().iter().position(|lane| lane["name"] == "normal-hitclap").unwrap();
    let normal = scene["lanes"].as_array().unwrap().iter().position(|lane| lane["name"] == "normal-hitnormal").unwrap();
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1000.0, row: clap as i32, x: 120.0, y: 12.0,
            value: None, volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    let set_scope = |app: &mut Application, lanes: serde_json::Value| {
        serde_json::from_value::<Action>(serde_json::json!({
            "kind":"volume_scope", "path":scene["path"], "lanes":lanes,
        })).unwrap().run(app);
    };
    set_scope(&mut app, serde_json::json!([scene["lanes"][clap]["key"]]));
    Action::Volume { value: 40, relative: false }.run(&mut app);
    set_scope(&mut app, serde_json::Value::Null);
    Action::Command { id: "selectAll".into() }.run(&mut app);
    let levels = |app: &Application| {
        let c = &app.document().unwrap().composer;
        c.hitsounds().moments().iter().map(|m| {
            [m.hit_volume(&c.lanes[normal]), m.hit_volume(&c.lanes[clap])]
        }).collect::<Vec<_>>()
    };
    let drag = |app: &mut Application, phase: &str, y: f64| {
        Action::Pointer {
            phase: phase.into(), ms: 1000.0, row: 0, x: 120.0, y,
            value: Some(100.0-y), volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: true, alt: false,
        }.run(app);
    };
    let originals = [[60,40],[80,80]];
    assert_eq!(levels(&app), originals);
    drag(&mut app, "down", 60.0);
    assert_eq!(levels(&app), originals);
    drag(&mut app, "move", 30.0);
    assert_eq!(levels(&app), [[58,28],[88,88]]);
    drag(&mut app, "up", 60.0);
    assert_eq!(levels(&app), originals);
    assert_eq!(snapshot(&app, true)["scene"]["historyPosition"], 1);

    drag(&mut app, "down", 30.0);
    drag(&mut app, "move", 90.0);
    assert_eq!(levels(&app), [[65,65],[65,65]]);
    drag(&mut app, "cancel", 90.0);
    assert_eq!(levels(&app), originals);

    drag(&mut app, "down", 30.0);
    drag(&mut app, "up", 90.0);
    assert_eq!(levels(&app), [[65,65],[65,65]]);
    assert_eq!(snapshot(&app, true)["scene"]["historyPosition"], 2);
    Action::Command { id: "undo".into() }.run(&mut app);
    assert_eq!(levels(&app), originals);
    assert_eq!(fs::read_to_string(&path).unwrap(), map);
}

#[test]
fn protect_normals_keeps_noop_deletes_clean_and_still_removes_additions() {
    let fixture = Fixture::new("protect-normals");
    let path = fixture.0.join("Protection.osu");
    let map = b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:60:\n256,192,1500,1,0,0:0:0:80:\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    Action::Protect { on:true }.run(&mut app);
    let scene = snapshot(&app, true)["scene"].clone();
    let row = scene["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["name"] == "normal-hitnormal").unwrap() as i32;
    let click = |app: &mut Application| {
        for phase in ["down","up"] {
            Action::Pointer {
                phase:phase.into(), ms:1000.0, row, x:120.0, y:12.0, value:None, volume_lanes:None, visible_rows:None, button:None,
                shift:false, ctrl:false, alt:false,
            }.run(app);
        }
    };
    let command = |app: &mut Application, id: &str| Action::Command { id:id.into() }.run(app);
    click(&mut app);
    command(&mut app,"delete");
    assert!(!app.is_dirty());
    assert_eq!(snapshot(&app,true)["scene"]["selection"]["count"],1);
    for tool in ["Erase","Draw"] {
        Action::Tool {name:tool.into()}.run(&mut app);
        click(&mut app);
        assert!(!app.is_dirty(), "{tool} on a protected hit");
        assert_eq!(snapshot(&app,true)["scene"]["historyPosition"],0);
    }
    command(&mut app,"selectAll");
    command(&mut app,"delete");
    let after = snapshot(&app,true);
    assert_eq!(after["scene"]["selection"]["count"],2);
    assert_eq!(after["scene"]["historyPosition"],1);
    assert!(after["scene"]["moments"].as_array().unwrap().iter().all(|m| m[3][0][0] == row && m[3].as_array().unwrap().len() == 1));
    command(&mut app,"undo");
    assert!(!app.is_dirty());
    assert_eq!(fs::read(&path).unwrap(),map);
}

#[test]
fn controller_drags_paint_shared_moments_and_undo_as_one_gesture() {
    let root = Fixture::new("volume-drags");
    let path = root.0.join("Controller.osu");
    let mut map = "osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n".to_string();
    for (time, volume) in [(1000, 60), (1250, 70), (1500, 80), (1750, 90)] {
        writeln!(map, "256,192,{time},1,10,0:0:0:{volume}:").unwrap();
    }
    fs::write(&path, &map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let send = |app: &mut Application, phase: &str, ms, value, alt| {
        Action::Pointer {
            phase: phase.into(), ms, row: 0, x: ms, y: 100.0 - value,
            value: Some(value), volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt,
        }.run(app);
    };
    let levels = |app: &Application| -> Vec<i64> {
        snapshot(app, true)["scene"]["moments"].as_array().unwrap().iter()
            .map(|moment| moment[1].as_i64().unwrap()).collect()
    };
    let undo = |app: &mut Application| Action::Command { id: "undo".into() }.run(app);

    send(&mut app, "down", 1000.0, 30.0, false);
    send(&mut app, "move", 1500.0, 40.0, false);
    assert_eq!(levels(&app), [40, 70, 80, 90], "only the grabbed bar moves");
    send(&mut app, "up", 1500.0, 40.0, false);
    undo(&mut app);
    assert_eq!(levels(&app), [60, 70, 80, 90]);

    send(&mut app, "down", 1000.0, 20.0, true);
    send(&mut app, "move", 1500.0, 60.0, true);
    send(&mut app, "up", 1500.0, 60.0, true);
    assert_eq!(levels(&app), [20, 40, 60, 90]);
    undo(&mut app);
    assert_eq!(levels(&app), [60, 70, 80, 90]);

    Action::Command { id: "selectAll".into() }.run(&mut app);
    send(&mut app, "down", 1000.0, 30.0, false);
    assert_eq!(levels(&app), [60, 70, 80, 90], "a relative grab does not jump");
    send(&mut app, "move", 1500.0, 20.0, false);
    assert_eq!(levels(&app), [50, 60, 70, 80]);
    send(&mut app, "cancel", 1500.0, 20.0, false);
    assert_eq!(levels(&app), [60, 70, 80, 90]);
    Action::Volume { value: 0, relative: false }.run(&mut app);
    let history = snapshot(&app, true)["scene"]["historyPosition"].clone();
    send(&mut app, "down", 1000.0, 30.0, false);
    send(&mut app, "up", 1000.0, 30.0, false);
    assert_eq!(levels(&app), [5, 5, 5, 5], "a volume set to 0 stops at 5%");
    assert_eq!(snapshot(&app, true)["scene"]["historyPosition"], history);
    assert_eq!(fs::read_to_string(&path).unwrap(), map);
}

#[test]
fn selection_volume_scope_applies_to_the_chosen_lane_and_shortcut() {
    use serde_json::{Value, json};

    let root = Fixture::new("selection-volume-scope");
    let path = root.0.join("Scope.osu");
    let map = "osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:60:\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true)["scene"].clone();
    let lanes = scene["lanes"].as_array().unwrap();
    let row = |sound: &str| lanes.iter().position(|lane| {
        lane["name"].as_str().unwrap().to_lowercase().contains(sound)
    }).unwrap();
    let normal = row("hitnormal");
    let clap = row("clap");
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1000.0, row: normal as i32, x: 0.0, y: 0.0,
            value: None, volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    let apply_scope = |app: &mut Application, path: &Value, lanes: Value| {
        let action: Action = serde_json::from_value(json!({
            "kind": "volume_scope", "path": path, "lanes": lanes,
        })).unwrap();
        action.run(app);
    };
    apply_scope(&mut app, &scene["path"], json!([lanes[clap]["key"]]));
    Action::Volume { value: 35, relative: false }.run(&mut app);
    let levels = |app: &Application, normal_volume, clap_volume| {
        let scene = snapshot(app, true)["scene"].clone();
        let moment = &scene["moments"][0];
        assert_eq!(moment[1], normal_volume);
        let hits = moment[3].as_array().unwrap();
        assert_eq!(hits.iter().find(|hit| hit[0] == normal).unwrap(), &json!([normal, true]));
        assert_eq!(hits.iter().find(|hit| hit[0] == clap).unwrap(), &json!([clap, false, false, clap_volume]));
        assert_eq!(scene["selection"]["count"], 1);
    };
    levels(&app, 60, 35);

    Action::Key {
        key: "ArrowUp".into(), code: "ArrowUp".into(), ctrl: false, shift: false, alt: true, lane_view: None,
    }.run(&mut app);
    levels(&app, 60, 40);

    apply_scope(&mut app, &json!("Stale.osu"), Value::Null);
    assert!(app.error().is_some());
    apply_scope(&mut app, &scene["path"], json!([]));
    Action::Volume { value: 90, relative: false }.run(&mut app);
    levels(&app, 60, 40);
    assert_eq!(fs::read_to_string(&path).unwrap(), map);
}

#[test]
fn a_volume_bar_shows_where_its_hits_play_when_none_plays_at_the_moment_s_own_volume() {
    use serde_json::json;

    let root = Fixture::new("volume-bar-reference");
    let path = root.0.join("Bar.osu");
    let map = "osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true)["scene"].clone();
    let normal = scene["lanes"].as_array().unwrap().iter().position(|lane| {
        lane["name"].as_str().unwrap().to_lowercase().contains("hitnormal")
    }).unwrap();
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1000.0, row: normal as i32, x: 0.0, y: 0.0,
            value: None, volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    let action: Action = serde_json::from_value(json!({
        "kind": "volume_scope", "path": scene["path"], "lanes": [scene["lanes"][normal]["key"]],
    })).unwrap();
    action.run(&mut app);
    Action::Volume { value: 50, relative: false }.run(&mut app);
    let moment = snapshot(&app, true)["scene"]["moments"][0].clone();
    assert_eq!(moment[1], 50, "the bar stands where the only hit plays, not at the moment's own 100");
    assert_eq!(moment[3], json!([[normal, true, false, 50]]));
    assert_eq!(fs::read_to_string(&path).unwrap(), map);
}

#[test]
fn ramp_shape_action_applies_curves_through_the_volume_controller() {
    let root = Fixture::new("ramp-shapes");
    let path = root.0.join("Ramp.osu");
    let mut map = "osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n".to_string();
    for time in [1000, 1125, 1250, 1375, 1500] {
        writeln!(map, "256,192,{time},1,0,0:0:0:0:").unwrap();
    }
    fs::write(&path, &map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    for (shape, expected, shift, backwards) in [
        ("Linear", [20, 40, 60, 80, 100], false, false),
        ("EaseIn", [20, 25, 40, 65, 100], false, false),
        ("EaseOut", [20, 55, 80, 95, 100], false, true),
        ("SCurve", [20, 33, 60, 88, 100], false, false),
        ("EaseIn", [20, 25, 40, 65, 100], true, false),
    ] {
        let action: Action = serde_json::from_value(serde_json::json!({
            "kind": "ramp_shape", "value": shape,
        })).unwrap();
        action.run(&mut app);
        assert_eq!(snapshot(&app, false)["rampShape"], shape);
        Action::Tool { name: if shift { "Select" } else { "Ramp" }.into() }.run(&mut app);
        let (start, end) = if backwards {
            ((1500.0, 100.0), (1000.0, 20.0))
        } else {
            ((1000.0, 20.0), (1500.0, 100.0))
        };
        for (phase, (ms, value)) in [("down", start), ("move", end), ("up", end)] {
            Action::Pointer {
                phase: phase.into(), ms, row: 0, x: ms, y: 100.0 - value,
                value: Some(value), volume_lanes: None, visible_rows: None, button: None, shift, ctrl: false, alt: false,
            }.run(&mut app);
            if phase == "move" {
                let scene = snapshot(&app, true)["scene"].clone();
                let preview: Vec<_> = scene["rampPreview"].as_array().unwrap().iter()
                    .step_by(4).map(|point| point[1].as_i64().unwrap()).collect();
                assert_eq!(preview, expected, "{shape} preview with Shift={shift}");
            }
        }
        let scene = snapshot(&app, true)["scene"].clone();
        let values: Vec<_> = scene["moments"].as_array().unwrap().iter()
            .map(|moment| moment[1].as_i64().unwrap()).collect();
        assert_eq!(values, expected, "{shape} with Shift={shift}");
        assert!(scene["rampPreview"].is_null());
        assert_eq!(scene["historyPosition"], 1);
        Action::Command { id: "undo".into() }.run(&mut app);
    }
    assert_eq!(fs::read_to_string(&path).unwrap(), map);
}

#[test]
fn selected_lane_mix_follows_the_clicked_lane_and_rejects_stale_targets() {
    let root = Fixture::new("selected-lane-mix");
    let map = root.0.join("Hard.osu");
    let original = b"osu file format v14\n\n[General]\nMode:0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    fs::write(&map, original).unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let map = app.document().unwrap().path.clone();
    let lanes = app.document().unwrap().composer.lanes.clone();
    let [first, second, other] = [&lanes[0], &lanes[1], &lanes[2]];
    let click = |app: &mut Application, path: &Path, lane: &str, operation: &str, selected: &[String]| {
        let action: Action = serde_json::from_value(serde_json::json!({
            "kind": "lane_selection_mix", "target": {"map": path, "lane": lane},
            "operation": operation, "lanes": selected,
        })).unwrap();
        action.run(app);
    };
    let key = format!("{first:?}");
    let selected = vec![format!("{first:?}"), format!("{second:?}")];
    app.mute(first.clone());
    app.mute(other.clone());
    click(&mut app, &map, &key, "mute", &selected);
    assert!(!app.is_muted(first) && !app.is_muted(second));
    assert!(app.is_muted(other));
    click(&mut app, &map, &key, "mute", &selected);
    assert!(app.is_muted(first) && app.is_muted(second));

    app.solo(second.clone());
    click(&mut app, &map.with_file_name("Stale.osu"), &key, "solo", &selected);
    assert!(!app.is_solo(first) && app.is_solo(second));
    click(&mut app, &map, &key, "solo", &[key.clone(), "stale".into()]);
    assert!(!app.is_solo(first) && app.is_solo(second));
    click(&mut app, &map, &key, "solo", &selected);
    assert!(app.is_solo(first) && app.is_solo(second) && !app.is_solo(other));
    assert!(!app.is_dirty());
    assert_eq!(fs::read(&map).unwrap(), original);
}

#[test]
fn sample_set_mix_toggles_only_the_requested_set() {
    use dawsu_foundation::{SampleIdentity, SampleSet};

    let root = Fixture::new("sample-set-mix");
    let map = root.0.join("Hard.osu");
    fs::write(
        &map,
        "osu file format v14\n\n[General]\nMode:0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let lanes = &app.document().unwrap().composer.lanes;
    let normal: Vec<_> = lanes
        .iter()
        .filter(|lane| {
            matches!(
                lane,
                SampleIdentity::Standard {
                    set: SampleSet::Normal,
                    ..
                }
            )
        })
        .cloned()
        .collect();
    let soft = lanes
        .iter()
        .find(|lane| {
            matches!(
                lane,
                SampleIdentity::Standard {
                    set: SampleSet::Soft,
                    ..
                }
            )
        })
        .unwrap()
        .clone();
    assert!(normal.len() >= 4);

    let toggle = |app: &mut Application, operation: &str| {
        let action: Action = serde_json::from_value(serde_json::json!({
            "kind": "group_mix", "set": "normal", "operation": operation,
        }))
        .unwrap();
        action.run(app);
    };
    app.mute(normal[0].clone());
    app.mute(soft.clone());
    toggle(&mut app, "mute");
    assert!(normal.iter().all(|lane| app.is_muted(lane)));
    toggle(&mut app, "mute");
    assert!(normal.iter().all(|lane| !app.is_muted(lane)));
    assert!(app.is_muted(&soft));

    app.solo(soft.clone());
    toggle(&mut app, "solo");
    assert!(normal.iter().all(|lane| app.is_solo(lane)));
    toggle(&mut app, "solo");
    assert!(normal.iter().all(|lane| !app.is_solo(lane)));
    assert!(app.is_solo(&soft));
}

#[test]
fn slider_body_selection_exposes_its_slide_whistle_and_ticked_set() {
    let Some(root) = isolated_save_fixture("slider-body-selection",
        "tests::volume_and_editing::slider_body_selection_exposes_its_slide_whistle_and_ticked_set") else { return };
    let path = root.0.join("Sliders.osu");
    let map = "osu file format v14\n\n[General]\nMode:0\nSampleSet: Normal\n\n[Metadata]\nVersion:Sliders\n\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,2,1,60,1,0\n\n[HitObjects]\n100,100,1000,2,0,L|300:100,2,200,0|0|0,0:0|0:0|0:0,0:0:0:0:\n";
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let document = app.document().unwrap();
    let body = &document.composer.hitsounds().bodies()[0];
    assert!(!body.ticks().is_empty(), "fixture must generate ticks");

    Action::Command { id: "selectAll".into() }.run(&mut app);
    let selection = snapshot(&app, true)["scene"]["selection"].clone();
    assert_eq!(selection["sliderBody"]["start"].as_f64(), Some(1000.0));
    assert_eq!(selection["sliderBody"]["set"], "soft");
    assert_eq!(selection["sliderBody"]["whistle"], false);
    assert_eq!(selection["sliderBody"]["whistleSet"], "auto");
    assert_eq!(selection["sliderBody"]["volume"], 60);

    let select = || Action::Command { id: "selectAll".into() };
    let change = || serde_json::from_value::<Action>(serde_json::json!({
        "kind": "slider_body", "path": path.canonicalize().unwrap(), "index": 0,
        "set": "drum", "whistle": true, "volume": 80,
    })).unwrap();
    change().run(&mut app);
    let check = |app: &Application| {
        use dawsu_foundation::{SampleIdentity, SampleSet, SliderSound, Sound};
        let doc = app.document().unwrap();
        let hits = doc.composer.hitsounds();
        let body = &hits.bodies()[0];
        assert_eq!(body.set(), Some(SampleSet::Drum));
        assert!(matches!(
            body.tick(), SampleIdentity::Standard { set: SampleSet::Drum, sound: Sound::Slider(SliderSound::Tick), .. }
        ));
        assert!(matches!(
            body.whistle_lane(), SampleIdentity::Standard { set: SampleSet::Soft, sound: Sound::Slider(SliderSound::Whistle), .. }
        ));
        assert!(body.whistle());
        assert_eq!((body.volume(), hits.moments()[0].volume(), hits.moments()[1].volume()), (80, 80, 60));
    };
    check(&app);
    assert!(app.is_dirty());
    assert_eq!(fs::read(&path).unwrap(), map.as_bytes());

    Action::Command { id: "undo".into() }.run(&mut app);
    assert!(!app.is_dirty());
    let doc = app.document().unwrap();
    assert!(!doc.composer.hitsounds().bodies()[0].whistle());
    select().run(&mut app);
    change().run(&mut app);
    Action::Save.run(&mut app);
    wait_for_job(&mut app);
    assert!(!app.is_dirty());
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains("1000,-100,4,2,1,80,0,0\n"));
    assert!(written.contains("1007,-100,4,2,1,60,0,0\n"));
    assert!(written.contains("100,100,1000,2,2,L|300:100,2,200,0|0|0,0:0|0:0|0:0,3:2:0:0:"));
    open(&mut app, &path);
    check(&app);

    for choice in ["normal", "auto"] {
        select().run(&mut app);
        let action = serde_json::from_value::<Action>(serde_json::json!({
            "kind": "slider_body", "path": path.canonicalize().unwrap(), "index": 0,
            "whistle_set": choice, "whistle": true, "volume": 80,
        })).unwrap();
        action.run(&mut app);
        assert!(app.error().is_none(), "{:?}", app.error());
        let state = snapshot(&app, true);
        assert_eq!(state["scene"]["selection"]["sliderBody"]["whistleSet"], choice);
        assert_eq!(state["scene"]["selection"]["sliderBody"]["set"], "drum");
        Action::Save.run(&mut app);
        wait_for_job(&mut app);
        open(&mut app, &path);
        select().run(&mut app);
        let state = snapshot(&app, true);
        assert_eq!(state["scene"]["selection"]["sliderBody"]["whistleSet"], choice);
        assert_eq!(state["scene"]["selection"]["sliderBody"]["set"], "drum");
    }
}

#[test]
fn shortcut_adds_the_nearest_hit_in_the_next_eligible_visible_lane() {
    let fixture = Fixture::new("select-adjacent-lane");
    let path = fixture.0.join("Selection.osu");
    let map = concat!(
        "osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n",
        "256,192,1000,1,8,1:1:0:50:\n256,192,1050,1,8,1:3:0:50:\n",
        "256,192,1100,1,8,1:2:0:50:\n256,192,1150,1,2,1:1:0:50:\n",
        "256,192,1275,1,8,1:2:0:50:\n256,192,4000,1,4,1:1:0:50:\n",
    );
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let scene = snapshot(&app, true)["scene"].clone();
    let row = |name| scene["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["name"] == name).unwrap();
    let anchor = row("normal-hitclap");
    let above = row("normal-hitwhistle");
    let far = row("normal-hitfinish");
    let below = row("soft-hitclap");
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1000.0, row: anchor as i32, x: 10.0, y: 10.0,
            value: None, volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }

    let selected = |app: &Application| {
        let composer = &app.document().unwrap().composer;
        let mut times: Vec<_> = composer.selected.items().iter()
            .map(|hit| composer.hitsounds().moments()[hit.moment].time().ms()).collect();
        times.sort_by(f64::total_cmp);
        times
    };
    let press = |app: &mut Application, key, path: &serde_json::Value, rows: &[usize], span| {
        serde_json::from_value::<Action>(serde_json::json!({
            "kind": "key", "key": key, "code": key, "ctrl": false, "shift": true, "alt": false,
            "lane_view": {"path": path, "rows": rows, "span": span}
        })).unwrap().run(app);
    };
    let rows = [above, anchor, anchor, far, below];
    assert_eq!(selected(&app), [1000.0]);
    press(&mut app, "ArrowDown", &serde_json::json!("Stale.osu"), &rows, 700.0);
    press(&mut app, "ArrowDown", &scene["path"], &rows, 150.0);
    assert_eq!(selected(&app), [1000.0]);

    press(&mut app, "ArrowDown", &scene["path"], &rows, 700.0);
    assert_eq!(selected(&app), [1000.0, 1100.0]);
    press(&mut app, "ArrowUp", &scene["path"], &rows, 700.0);
    assert_eq!(selected(&app), [1000.0, 1100.0, 1150.0]);
    assert!(!app.is_dirty());
    assert_eq!(fs::read_to_string(&path).unwrap(), map);
}

#[test]
fn shortcut_change_routes_the_new_f1_alternative_on_home() {
    let mut app = Application::headless();
    let mut route = crate::home::Route::default();
    Action::ChangeShortcut { id: "help".into(), chord: None }.run(&mut app);
    let change = serde_json::from_value::<Action>(serde_json::json!({
        "kind": "change_shortcut", "id": "help",
        "chord": {"key":"F5", "code":"F5", "ctrl":false, "shift":false, "alt":false},
    })).unwrap();
    change.run(&mut app);
    assert!(app.error().is_none());

    let key = |name: &str| Action::Key {
        key: name.into(), code: name.into(), ctrl: false, shift: false, alt: false, lane_view: None,
    };
    assert!(route.handle(key("F1"), &mut app).is_none());
    let action = route.handle(key("F5"), &mut app).unwrap();
    action.run(&mut app);
    assert!(app.help_requested());

    app.take_shortcut_help();
    serde_json::from_value::<Action>(serde_json::json!({ "kind": "reset_shortcuts" }))
        .unwrap()
        .run(&mut app);
    assert!(app.error().is_none());
    assert!(route.handle(key("F5"), &mut app).is_none());
    let restored = route.handle(key("F1"), &mut app).unwrap();
    restored.run(&mut app);
    assert!(app.help_requested());
}

#[test]
fn timeline_edit_actions_quantize_repeat_edit_the_range_and_scale_once() {
    use serde_json::json;

    let fixture = Fixture::new("timeline-edits");
    let path = fixture.0.join("Edit.osu");
    let map = concat!(
        "osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n1300,400,4,1,0,100,1,0\n",
        "\n[HitObjects]\n256,192,1000,1,8,0:0:0:50:\n256,192,1250,1,0,0:0:0:70:\n",
        "256,192,1400,1,0,0:0:0:90:\n256,192,1500,1,0,0:0:0:40:\n",
    );
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let path = path.canonicalize().unwrap();
    let scene = snapshot(&app, true);
    let clap_row = scene["scene"]["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["name"] == "normal-hitclap" && lane["index"] == 0).unwrap() as i32;
    let clap = app.document().unwrap().composer.lanes[clap_row as usize].clone();
    let tool = |app: &mut Application, name: &str| Action::Tool { name: name.into() }.run(app);
    let command = |app: &mut Application, id: &str| Action::Command { id: id.into() }.run(app);
    let point = |app: &mut Application, phase: &str, ms: f64| {
        Action::Pointer {
            phase: phase.into(), ms, row: clap_row, x: ms, y: 12.0,
            value: None, volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(app);
    };
    let apply = |app: &mut Application, edit| {
        serde_json::from_value::<Action>(json!({
            "kind": "timeline_edit", "path": path, "edit": edit
        })).unwrap().run(app);
    };
    let clap_times = |app: &Application| {
        app.document().unwrap().composer.hitsounds().moments().iter()
            .filter(|moment| moment.has_hit(&clap))
            .map(|moment| moment.time().ms()).collect::<Vec<_>>()
    };

    Action::Snap { on: false, mode: "grid".into(), beats: 1, divisor: 4 }.run(&mut app);
    tool(&mut app, "Draw");
    point(&mut app, "down", 1160.0);
    point(&mut app, "up", 1160.0);
    assert_eq!(clap_times(&app), [1000.0, 1160.0]);
    tool(&mut app, "Select");
    point(&mut app, "down", 1160.0);
    point(&mut app, "up", 1160.0);
    command(&mut app, "quantizeDialog");
    assert_eq!(snapshot(&app, false)["editDialog"], "quantize");
    apply(&mut app, json!({"kind": "quantize", "strength": 50}));
    assert_eq!(clap_times(&app), [1000.0, 1142.5]);
    command(&mut app, "undo");
    assert_eq!(clap_times(&app), [1000.0, 1160.0]);
    command(&mut app, "undo");

    tool(&mut app, "Select");
    point(&mut app, "down", 1000.0);
    point(&mut app, "up", 1000.0);
    Action::Key {
        key: "r".into(), code: "KeyR".into(), ctrl: true, shift: false, alt: false, lane_view: None,
    }.run(&mut app);
    assert_eq!(snapshot(&app, false)["editDialog"], "repeat");
    assert_eq!(snapshot(&app, true)["scene"]["repeatGap"], 1);
    apply(&mut app, json!({"kind": "repeat", "copies": 2, "gap_steps": 2}));
    assert_eq!(clap_times(&app), [1000.0, 1250.0, 1400.0]);

    tool(&mut app, "Range");
    point(&mut app, "down", 1000.0);
    point(&mut app, "move", 1250.0);
    point(&mut app, "up", 1250.0);
    command(&mut app, "editRange");
    assert_eq!(snapshot(&app, false)["editDialog"], "range");
    apply(&mut app, json!({"kind": "range", "operation": "copy", "offset": 500, "rows": [clap_row]}));
    assert_eq!(clap_times(&app), [1000.0, 1250.0, 1400.0, 1500.0]);

    command(&mut app, "selectAll");
    command(&mut app, "scaleByVolume");
    assert_eq!(snapshot(&app, false)["editDialog"], "scale");
    apply(&mut app, json!({"kind": "scale", "scope": "selected", "low": 35, "high": 45, "percent": 150}));
    let volume = |app: &Application| app.document().unwrap().composer.hitsounds().moments()[3].volume();
    assert_eq!(volume(&app), 60);
    command(&mut app, "undo");
    assert_eq!(volume(&app), 40);

    for operation in ["move", "delete"] {
        command(&mut app, "editRange");
        apply(&mut app, json!({"kind": "range", "operation": operation, "offset": 500, "rows": [clap_row]}));
        assert_eq!(clap_times(&app), [1250.0, 1400.0, 1500.0]);
        command(&mut app, "undo");
        assert_eq!(clap_times(&app), [1000.0, 1250.0, 1400.0, 1500.0]);
    }
    command(&mut app, "deselect");
    assert!(snapshot(&app, true)["scene"]["range"].is_null());
    assert_eq!(fs::read_to_string(&path).unwrap(), map);
}

#[test]
fn selection_start_moves_the_chosen_hits_together_and_undoes_once() {
    use dawsu_foundation::{Hitsound, SampleIdentity, Sound};
    use dawsu_hitsounding::Place;
    use serde_json::json;

    let fixture = Fixture::new("selection-start");
    let path = fixture.0.join("Selected.osu");
    let map = concat!(
        "osu file format v14\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,100,1,0\r\n\r\n",
        "[HitObjects]\r\n256,192,1120,1,8,0:0:0:40:\r\n256,192,1295,1,2,0:0:0:70:\r\n",
    );
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let path = path.canonicalize().unwrap();
    let rows = {
        let document = app.document().unwrap();
        [Hitsound::Clap, Hitsound::Whistle].map(|sound| document.composer.lanes.iter().position(|lane| {
            matches!(lane, SampleIdentity::Standard { sound: Sound::Hit(hit), index, .. } if *hit == sound && index.get() == 0)
        }).unwrap() as i32)
    };
    Action::Tool { name: "Select".into() }.run(&mut app);
    for (index, (ms, row)) in [1120.0, 1295.0].into_iter().zip(rows).enumerate() {
        for phase in ["down", "up"] {
            Action::Pointer {
                phase: phase.into(), ms, row, x: ms, y: 12.0, value: None, volume_lanes: None, visible_rows: None, button: None,
                shift: false, ctrl: index > 0, alt: false,
            }.run(&mut app);
        }
    }
    let selection = snapshot(&app, true)["scene"]["selection"].clone();
    assert_eq!(selection["start"].as_f64(), Some(1120.0));
    assert_eq!(selection["count"], 2);
    let apply = |app: &mut Application, path: &Path, position: f64| {
        serde_json::from_value::<Action>(json!({
            "kind": "timeline_edit", "path": path, "edit": { "kind": "selection_start", "position": position },
        })).unwrap().run(app);
    };
    apply(&mut app, &path.with_file_name("Sibling.osu"), 1330.5);
    apply(&mut app, &path, 1120.4);
    assert!(!app.is_dirty(), "stale paths and sub-half-ms changes are skipped");

    apply(&mut app, &path, 1330.5);
    let doc = app.document().unwrap();
    let hits = doc.composer.hitsounds().moments();
    let anchored: Vec<_> = hits.iter().filter(|hit| hit.place() != Place::NoObject)
        .map(|hit| (hit.time().ms(), hit.hits().count())).collect();
    assert_eq!(anchored, [(1120.0, 1), (1295.0, 1)], "untargeted hitnormals stay at their objects");
    let mut loose: Vec<_> = hits.iter().filter(|hit| hit.place() == Place::NoObject && hit.hits().next().is_some())
        .map(|hit| (hit.time().ms(), hit.volume())).collect();
    loose.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(loose, [(1330.5, 40), (1505.5, 70)]);
    let selection = snapshot(&app, true)["scene"]["selection"].clone();
    assert_eq!(selection["start"].as_f64(), Some(1330.5));
    assert_eq!(selection["count"], 2);
    Action::Command { id: "undo".into() }.run(&mut app);
    assert!(!app.is_dirty());
    assert_eq!(fs::read(&path).unwrap(), map.as_bytes());
}

#[test]
fn displayed_tab_and_plain_home_shortcuts_reach_the_command_resolver() {
    use dawsu_editor::keys::{Key, KeyCombination, Mods};
    let mut app = Application::headless();
    app.change_shortcut("help", Some(KeyCombination::new(Key::Tab, Mods::CTRL))).unwrap();
    app.change_shortcut("open", Some(KeyCombination::plain(Key::H))).unwrap();
    let state = snapshot(&app, false);
    assert_eq!(state["tabRoutes"], serde_json::json!([[true, false, false, true]]));
    let mut route = crate::home::Route::default();
    let tab = Action::Key {
        key: "Tab".into(), code: "Tab".into(), ctrl: true, shift: false, alt: false, lane_view: None,
    };
    route.handle(tab, &mut app).unwrap().run(&mut app);
    assert!(app.help_requested());
    let open = Action::Key {
        key: "h".into(), code: "KeyH".into(), ctrl: false, shift: false, alt: false, lane_view: None,
    };
    route.handle(open, &mut app).unwrap().run(&mut app);
    assert!(matches!(app.requests().last(), Some(dawsu_application::PlatformRequest::OpenMap)));
}

#[test]
fn removing_a_lane_takes_its_hits_in_one_step_and_keeps_its_layers() {
    let fixture = Fixture::new("remove-lane");
    let map = fixture.0.join("map.osu");
    let tone = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    fs::write(fixture.0.join("normal-hitfinish.wav"), tone).unwrap();
    fs::write(fixture.0.join("normal-hitfinish2.wav"), tone).unwrap();
    // Finish (index 1) at two volumes, Finish 2 at 1500 and a soft whistle.
    let original = "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,4,1:0:1:60:\n256,192,1000,1,4,1:0:1:40:\n256,192,1500,1,4,1:0:2:0:\n256,192,2000,1,2,2:0:0:0:\n";
    fs::write(&map, original).unwrap();
    let mut app = Application::with_data_root(fixture.0.join("data"));
    app.clear_error();
    let skin = skin_sounds("remove-lane-skin", tone);
    app.update_preferences(|p| p.default_samples = skin.0.clone());
    open(&mut app, &map);
    let scene = |app: &Application| snapshot(app, true)["scene"].clone();
    let model = |app: &Application| app.document().unwrap().composer.hitsounds().clone();
    let before = scene(&app);
    let hits_before = model(&app);
    let row_of = |scene: &serde_json::Value, family: &str, index: u64| scene["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["family"] == family && lane["index"] == index).unwrap();
    let shown = |scene: &serde_json::Value, row: usize| scene["laneOrder"].as_array().unwrap().iter()
        .any(|value| value.as_u64() == Some(row as u64));
    let main = row_of(&before, "normal-hitfinish", 1);
    let layer = row_of(&before, "normal-hitfinish", 2);
    assert_eq!(before["lanes"][main]["hitCount"], 2);
    let remove = |app: &mut Application, scene: &serde_json::Value, row: usize| {
        serde_json::from_value::<Action>(serde_json::json!({
            "kind": "delete_lane", "target": {"map": scene["path"], "lane": scene["lanes"][row]["sourceKey"]},
            "revision": scene["laneRevision"],
        })).unwrap().run(app);
        assert!(app.error().is_none(), "{:?}", app.error());
    };
    remove(&mut app, &before, main);
    let hits_after = model(&app);
    assert_eq!(app.status(), "Removed Finish and 2 hits. Undo restores them.");
    let after = scene(&app);
    assert!(!shown(&after, main));
    assert_eq!(after["lanes"][main]["hitCount"], 0);
    // Only the main lane goes: its layer keeps its lane and its hit.
    assert!(shown(&after, layer));
    assert_eq!(after["lanes"][layer]["hitCount"], 1);
    // One undo step brings back the lane and every hit as it was: volumes, sample sets, placements, links.
    Action::Command { id: "undo".into() }.run(&mut app);
    assert_eq!(model(&app), hits_before);
    assert!(shown(&scene(&app), main));
    Action::Command { id: "redo".into() }.run(&mut app);
    assert_eq!(model(&app), hits_after);

    // Nothing is written to the mapset on removal, and the sample file stays.
    assert_eq!(fs::read_to_string(&map).unwrap(), original);
    assert!(fixture.0.join("normal-hitfinish.wav").exists());
    // A clap drawn beside Finish 2 makes a mixed moment; Save writes it and leaves the removed lane out.
    let clap = row_of(&scene(&app), "normal-hitclap", 0) as i32;
    Action::Tool { name: "Draw".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1500.0, row: clap, x: 12.0, y: 12.0,
            value: None, volume_lanes: None, visible_rows: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    assert!(app.error().is_none(), "{:?}", app.error());
    Action::Command { id: "save".into() }.run(&mut app);
    wait_for_job(&mut app);
    let saved = fs::read_to_string(&map).unwrap();
    assert!(saved.contains("256,192,500,1,0,") && saved.contains("256,192,1000,1,0,"), "{saved}");
    let mixed = saved.lines().find(|line| line.starts_with("256,192,1500,")).unwrap();
    let sound: u32 = mixed.split(',').nth(4).unwrap().parse().unwrap();
    assert_eq!(sound & 4, 4, "Finish 2 stays: {mixed}");
    assert_eq!(sound & 8, 8, "the clap is written: {mixed}");

    // A renamed lane reads by its own name, as on its − button.
    let now = scene(&app);
    let key = now["lanes"][layer]["colourKey"].as_str().unwrap().to_owned();
    app.set_lane_layout(dawsu_application::LaneLayout { lane_names: [(key, "Crash".into())].into(), ..Default::default() }).unwrap();
    remove(&mut app, &now, layer);
    assert_eq!(app.status(), "Removed Crash and 1 hit. Undo restores them.");
}

#[test]
fn select_box_selects_only_requested_non_contiguous_visual_rows() {
    let fixture = Fixture::new("select-box-visual-rows");
    let map = fixture.0.join("VisualRows.osu");
    let content = "osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n\
        256,192,1000,1,14,0:0:0:100:\n\
        256,192,1500,1,14,0:0:0:100:\n";
    fs::write(&map, content).unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let s = snapshot(&app, true)["scene"].clone();
    let row_of = |name: &str| {
        s["lanes"]
            .as_array()
            .unwrap()
            .iter()
            .position(|l| l["name"] == name)
            .unwrap()
    };
    let normal = row_of("normal-hitnormal");
    let whistle = row_of("normal-hitwhistle");
    let finish = row_of("normal-hitfinish");
    let clap = row_of("normal-hitclap");
    assert!(normal < whistle && whistle < finish && finish < clap);

    // Select only normal and clap (skipping whistle & finish numerically in between!).
    Action::SelectBox {
        from_ms: 900.0,
        to_ms: 1600.0,
        rows: vec![clap, normal],
        additive: false,
        active_lane: Some(clap),
    }
    .run(&mut app);

    let after = snapshot(&app, true)["scene"].clone();
    assert_eq!(after["selection"]["count"], 4, "2 moments x 2 lanes = 4 hits");
    let selected_rows: std::collections::BTreeSet<usize> = after["moments"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m[3].as_array().unwrap().iter())
        .filter(|h| h[1].as_bool() == Some(true))
        .map(|h| h[0].as_u64().unwrap() as usize)
        .collect();
    assert_eq!(
        selected_rows,
        [normal, clap].into_iter().collect(),
        "must not select intermediate lanes whistle/finish"
    );

    // Additive select adds whistle without losing normal/clap.
    Action::SelectBox {
        from_ms: 1400.0,
        to_ms: 1600.0,
        rows: vec![whistle],
        additive: true,
        active_lane: None,
    }
    .run(&mut app);
    let added = snapshot(&app, true)["scene"].clone();
    assert_eq!(added["selection"]["count"], 5);

    // Empty rows with additive=false clears selection cleanly.
    Action::SelectBox {
        from_ms: 0.0,
        to_ms: 0.0,
        rows: vec![],
        additive: false,
        active_lane: None,
    }
    .run(&mut app);
    let cleared = snapshot(&app, true)["scene"].clone();
    assert!(cleared["selection"].is_null());
}
