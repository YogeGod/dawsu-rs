use super::{Action, PlaybackStart, lane_add_choices, lane_by_key, scene_lane_key, snapshot, sound_set_library_view};
use dawsu_application::{Application, WaveBin};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);

impl Fixture {
    fn new(label: &str) -> Self {
        let folder =
            std::env::temp_dir().join(format!("dawsu-tauri-tests-{label}-{}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        Self(folder)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wait_for_job(app: &mut Application) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline, "editor job did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
}

fn open(app: &mut Application, path: &Path) {
    Action::Open { path: path.into() }.run(app);
    wait_for_job(app);
}

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
    let range = |app: &Application| (app.locator_left, app.locator_right, app.cycle);
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
            phase: "double_click".into(), ms, row, x: 12.0, y: 12.0, value: None,
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
            phase: phase.into(), ms: 4250.0, row: 1, x: 4250.0, y: 12.0, value: None,
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
fn drum_view_uses_a_preset_without_changing_the_mapset_and_follows_sibling_difficulties() {
    use dawsu_hitsounding::drums::{DrumMap, DrumRow, write_dawsudrum};

    let fixture = Fixture::new("drum-view");
    let path = fixture.0.join("Hard.osu");
    let sibling = fixture.0.join("Expert.osu");
    let other = fixture.0.join("other");
    fs::create_dir(&other).unwrap();
    let foreign = other.join("Hard.osu");
    let bytes = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
    for map in [&path, &sibling, &foreign] {
        fs::write(map, bytes).unwrap();
    }
    let preset = fixture.0.join("Kit.dawsudrum");
    let row = |id, name: &str, lane: Option<&str>| DrumRow {
        id: format!("aaaaaaaa-0000-4000-8000-{id:012}"),
        name: name.into(),
        lane: lane.map(str::to_owned),
        slot: None,
        midi_note: None,
    };
    let kit = DrumMap {
        id: "bbbbbbbb-0000-4000-8000-000000000001".into(),
        name: "Kit".into(),
        rows: vec![
            row(1, "Snare", Some("Skin|normal-hitclap")),
            row(2, "Ghost notes", None),
            row(3, "Rim", Some("Skin|normal-hitclap")),
        ],
        audio: Vec::new(),
    };
    let preset_bytes = write_dawsudrum(&kit).unwrap();
    fs::write(&preset, &preset_bytes).unwrap();

    let mut app = Application::headless();
    let mut drums = super::drum_view::DrumView::default();
    open(&mut app, &path);
    let expected = app.document().unwrap().path.clone();
    let original = snapshot(&app, true);
    assert!(drums.load(&app, &sibling, &preset).is_err());
    drums.load(&app, &expected, &preset).unwrap();
    let read = |drums: &super::drum_view::DrumView, app: &Application| {
        let mut scene = snapshot(app, true);
        drums.decorate(app, &mut scene);
        scene
    };
    let shown = read(&drums, &app);
    assert_eq!(shown["drumView"]["enabled"], true);
    assert_eq!(shown["scene"]["lanes"], original["scene"]["lanes"]);
    assert_eq!(shown["scene"]["moments"], original["scene"]["moments"]);
    let rows = shown["scene"]["drumRows"].as_array().unwrap();
    // Two rows on one lane show as one row under both names, so the lane shows once.
    assert_eq!(rows[0]["name"], "Snare / Rim");
    assert!(rows[1]["row"].is_null());
    assert_eq!(rows[1]["name"], "Ghost notes");
    assert_ne!(rows[0]["row"], serde_json::Value::Null);
    assert_eq!(rows.len(), original["scene"]["laneOrder"].as_array().unwrap().len() + 1);

    drums.toggle(&app, &expected, false).unwrap();
    assert!(read(&drums, &app)["scene"]["drumRows"].is_null());
    drums.toggle(&app, &expected, true).unwrap();
    open(&mut app, &sibling);
    assert_eq!(read(&drums, &app)["drumView"]["name"], "Kit");
    open(&mut app, &foreign);
    assert!(read(&drums, &app)["drumView"].is_null());
    open(&mut app, &path);
    assert_eq!(read(&drums, &app)["drumView"]["enabled"], true);
    assert!(!app.is_dirty());
    assert_eq!(fs::read_to_string(preset).unwrap(), preset_bytes);
    for map in [&path, &sibling, &foreign] {
        assert_eq!(fs::read(map).unwrap(), bytes);
    }
}

#[test]
fn drum_view_rows_follow_a_replaced_lane_and_leave_hidden_lanes_hidden() {
    use dawsu_hitsounding::drums::{DrumMap, DrumRow, lane_key, write_dawsudrum};

    let fixture = Fixture::new("drum-view-replaced");
    let mapset = fixture.0.join("mapset");
    fs::create_dir(&mapset).unwrap();
    let path = mapset.join("Hard.osu");
    let bytes = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n256,192,2000,1,2,0:0:0:0:\n";
    fs::write(&path, bytes).unwrap();
    let external = fixture.0.join("snare.wav");
    fs::write(&external, b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40").unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let map = app.document().unwrap().path.clone();
    let lanes = snapshot(&app, true)["scene"]["lanes"].clone();
    let identity = |name: &str| {
        let row = lanes.as_array().unwrap().iter().position(|lane| lane["name"] == name).unwrap();
        app.document().unwrap().composer.lanes[row].clone()
    };
    let (clap, whistle) = (identity("normal-hitclap"), identity("normal-hitwhistle"));
    let key = |app: &Application, lane| lane_key(app.document().unwrap().composer.hitsounds().lane(lane).unwrap());
    let mut drums = super::drum_view::DrumView::default();
    let load = |drums: &mut super::drum_view::DrumView, app: &Application, rows: Vec<(&str, Option<String>)>| {
        let kit = DrumMap {
            id: "bbbbbbbb-0000-4000-8000-000000000002".into(),
            name: "Kit".into(),
            rows: rows.into_iter().enumerate().map(|(id, (name, lane))| DrumRow {
                id: format!("aaaaaaaa-0000-4000-8000-{id:012}"), name: name.into(), lane, slot: None, midi_note: None,
            }).collect(),
            audio: Vec::new(),
        };
        let preset = fixture.0.join("Kit.dawsudrum");
        fs::write(&preset, write_dawsudrum(&kit).unwrap()).unwrap();
        drums.load(app, &map, &preset).unwrap();
    };
    load(&mut drums, &app, vec![
        ("Snare", Some(key(&app, &clap))), ("Ride", Some(key(&app, &whistle))), ("Ghost notes", None),
    ]);

    app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &clap).unwrap();
    app.replace_lane_sample(&map, &clap, &external).unwrap();
    let replacement = app.document().unwrap().composer.lanes[app.active_lane].clone();
    let revision = app.document().unwrap().composer.history.version();
    app.delete_lane(&map, &whistle, revision).unwrap();

    let plain = snapshot(&app, true);
    let order = plain["scene"]["laneOrder"].as_array().unwrap().clone();
    let read = |drums: &super::drum_view::DrumView| {
        let mut shown = plain.clone();
        drums.decorate(&app, &mut shown);
        assert_eq!(shown["scene"]["lanes"], plain["scene"]["lanes"]);
        assert_eq!(shown["scene"]["laneOrder"], plain["scene"]["laneOrder"]);
        shown["scene"]["drumRows"].as_array().unwrap().clone()
    };
    let row_of = |rows: &[serde_json::Value], name: &str| rows.iter().find(|row| row["name"] == name).map(|row| row["row"].clone());
    // The view never adds, drops or repeats a lane: each shown lane appears exactly once.
    let lanes_once = |rows: &[serde_json::Value]| {
        let mut shown: Vec<_> = rows.iter().filter_map(|row| row["row"].as_u64()).collect();
        let mut expected: Vec<_> = order.iter().filter_map(serde_json::Value::as_u64).collect();
        shown.sort_unstable();
        expected.sort_unstable();
        assert_eq!(shown, expected, "{rows:?}");
    };

    let rows = read(&drums);
    let snare = row_of(&rows, "Snare").unwrap();
    assert!(order.contains(&snare), "Snare reads Unassigned on the replaced lane: {rows:?}");
    assert_eq!(plain["scene"]["lanes"][snare.as_u64().unwrap() as usize]["name"], "normal-hitclap");
    assert_eq!(row_of(&rows, "Ride"), None, "a hidden lane's row shows: {rows:?}");
    assert_eq!(row_of(&rows, "Ghost notes"), Some(serde_json::Value::Null));
    lanes_once(&rows);

    // A row may also name the replacement sample itself; the one lane shows once, under both names.
    load(&mut drums, &app, vec![("Snare", Some(key(&app, &clap))), ("Rim", Some(key(&app, &replacement)))]);
    let rows = read(&drums);
    assert_eq!(row_of(&rows, "Snare / Rim"), Some(snare), "{rows:?}");
    lanes_once(&rows);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

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
    assert!(app.error.take().is_some());
    assert!(visible(&scene(&app)));
    remove(&mut app, revision);
    assert!(app.error.is_none(), "{:?}", app.error);
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
    assert!(app.error.is_none(), "{:?}", app.error);
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
    assert!(app.error.is_none(), "{:?}", app.error);
    let state = scene(&app);
    assert!(state["laneOrder"].as_array().unwrap().iter().any(|index| {
        state["lanes"][index.as_u64().unwrap() as usize]["sourceKey"] == fresh["key"]
    }));
    assert_eq!(fs::read(&fresh_path).unwrap(), wave);
    assert_eq!(fs::read(&sample_path).unwrap(), wave);
    assert_eq!(fs::read(&map_path).unwrap(), original);
}

#[test]
fn switching_dirty_difficulties_respects_all_three_choices_and_stale_save() {
    let root = Fixture::new("dirty-switch-choices");
    let hard = root.0.join("Hard.osu");
    let expert = root.0.join("Expert.osu");
    let original = concat!("osu file format v14\n\n[General]\nMode:0\n",
        "\n[Metadata]\nVersion:Hard\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n",
        "\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n");
    let sibling = original.replace("Version:Hard", "Version:Expert");
    fs::write(&hard, original).unwrap();
    fs::write(&expert, &sibling).unwrap();
    let mut app = Application::headless();
    let edit = |app: &mut Application| {
        let scene = snapshot(app, true);
        let row = scene["scene"]["lanes"].as_array().unwrap().iter()
            .position(|lane| lane["name"] == "normal-hitclap").unwrap() as i32;
        Action::Tool { name: "Draw".into() }.run(app);
        for phase in ["down", "up"] {
            Action::Pointer {
                phase: phase.into(), ms: 1000.0, row, x: 100.0, y: 0.0,
                value: None, shift: false, ctrl: false, alt: false,
            }.run(app);
        }
        assert!(app.is_dirty());
    };
    let switch = |app: &mut Application, path: &Path, choice: &str| {
        let current = app.document().unwrap().path.clone();
        Action::Open { path: path.into() }.run(app);
        assert_eq!(snapshot(app, false)["pendingOpen"], true);
        assert_eq!(app.document().unwrap().path, current);
        Action::Pending { choice: choice.into() }.run(app);
    };
    open(&mut app, &hard);
    edit(&mut app);
    switch(&mut app, &expert, "cancel");
    assert!(app.document().unwrap().title.contains("Hard"));
    assert!(app.is_dirty());
    assert_eq!(snapshot(&app, false)["pendingOpen"], false);
    switch(&mut app, &expert, "discard");
    wait_for_job(&mut app);
    assert!(app.document().unwrap().title.contains("Expert"));
    assert_eq!(fs::read_to_string(&hard).unwrap(), original);
    assert!(!app.is_dirty());

    open(&mut app, &hard);
    edit(&mut app);
    switch(&mut app, &expert, "save");
    wait_for_job(&mut app);
    assert!(app.document().unwrap().title.contains("Expert"));
    assert_eq!(snapshot(&app, false)["pendingOpen"], false);
    let saved = fs::read_to_string(&hard).unwrap();
    assert!(saved.contains("256,192,1000,1,8,"));
    assert_eq!(fs::read_to_string(&expert).unwrap(), sibling);
    assert!(!app.is_dirty());

    edit(&mut app);
    let external = format!("{sibling}// External edit\n");
    fs::write(&expert, &external).unwrap();
    switch(&mut app, &hard, "save");
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.as_deref().is_some_and(|error| error.contains("changed on disk")));
    assert!(app.document().unwrap().title.contains("Expert"));
    assert!(app.is_dirty());
    assert_eq!(fs::read_to_string(&expert).unwrap(), external);
    assert_eq!(fs::read_to_string(&hard).unwrap(), saved);
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
            value: None, shift: false, ctrl: false, alt: false,
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
fn library_sample_actions_scope_paths_and_preserve_the_assignment_target() {
    let fixture = Fixture::new("library-samples");
    let library = fixture.0.join("library");
    let mapset = fixture.0.join("mapset");
    fs::create_dir_all(library.join("drums")).unwrap();
    fs::create_dir(&mapset).unwrap();
    let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    fs::write(library.join("drums/snare.WAV"), wave).unwrap();
    fs::write(fixture.0.join("outside.wav"), wave).unwrap();
    fs::write(library.join("drums/text.txt"), wave).unwrap();
    let location = |path: &str| -> super::library::Location {
        serde_json::from_value(serde_json::json!({ "root": library, "path": path })).unwrap()
    };
    assert_eq!(location("").entries(Some(&library)).unwrap(), [("drums".into(), true)]);
    assert_eq!(location("drums").entries(Some(&library)).unwrap(), [("snare.WAV".into(), false)]);
    assert!(location("drums").entries(Some(&mapset)).is_err());
    let path = mapset.join("Hard.osu");
    let original = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
    fs::write(&path, original).unwrap();
    let mut app = Application::headless();
    app.preferences.hitsound_library = Some(library.clone());
    open(&mut app, &path);
    let active = app.active_lane;
    let target = serde_json::json!({
        "map": app.document().unwrap().path,
        "lane": format!("{:?}", app.document().unwrap().composer.lanes[active]),
    });
    let action = |kind: &str, path: &str, target: &serde_json::Value| -> Action {
        serde_json::from_value(serde_json::json!({
            "kind": kind, "location": {"root": library, "path": path}, "target": target,
        })).unwrap()
    };
    action("preview_library_sample", "drums/snare.WAV", &target).run(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.active_lane, active);
    for path in ["../outside.wav", "drums/text.txt"] {
        action("replace_library_sample", path, &target).run(&mut app);
        assert!(app.error.take().is_some());
    }
    let absolute = fixture.0.join("outside.wav");
    action("replace_library_sample", absolute.to_str().unwrap(), &target).run(&mut app);
    assert!(app.error.take().is_some());
    let wrong = serde_json::json!({"map": mapset.join("Other.osu"), "lane": target["lane"]});
    action("replace_library_sample", "drums/snare.WAV", &wrong).run(&mut app);
    assert!(app.error.take().is_some());
    assert_eq!(fs::read_dir(&mapset).unwrap().count(), 1);
    action("replace_library_sample", "drums/snare.WAV", &target).run(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.is_dirty());
    assert_eq!(fs::read_dir(&mapset).unwrap().count(), 2);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read(library.join("drums/snare.WAV")).unwrap(), wave);
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
    assert_eq!(app.zoom, fit);
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
            value: None, shift: false, ctrl: false, alt: false,
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
            value: Some(100.0-y), shift: false, ctrl: true, alt: false,
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
                phase:phase.into(), ms:1000.0, row, x:120.0, y:12.0, value:None,
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
            value: Some(value), shift: false, ctrl: false, alt,
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
    assert_eq!(levels(&app), [0, 0, 0, 0]);
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
            value: None, shift: false, ctrl: false, alt: false,
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
    assert!(app.error.is_some());
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
            value: None, shift: false, ctrl: false, alt: false,
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
                value: Some(value), shift, ctrl: false, alt: false,
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
    app.locator_left = 750.0;
    for (key, code) in [("x", "KeyX"), ("c", "KeyC"), (" ", "Space")] {
        app.seek(100.0);
        press(&mut app, key, code, &mut mode);
        assert_eq!(app.position_ms(), 750.0, "{code}");
    }
    assert!(!app.is_dirty());
    assert_eq!(fs::read(path).unwrap(), bytes);
}

#[test]
fn dense_scene_and_dirty_switch_use_the_real_editor_state() {
    let root = Fixture::new("dense");
    let hard = root.0.join("Hard.osu");
    let expert = root.0.join("Expert.osu");
    let header = "osu file format v14\n\n[General]\nMode:0\n\n[Metadata]\nVersion:Hard\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n";
    let mut map = header.to_string();
    for i in 0..20_000 {
        writeln!(map, "256,192,{},1,0,0:0:0:0:", 1000 + i * 20).unwrap();
    }
    fs::write(&hard, &map).unwrap();
    fs::write(&expert, header.replace("Version:Hard", "Version:Expert")).unwrap();
    let mut app = Application::headless();
    open(&mut app, &hard);
    // Use the production waveform bin limit and four nonzero bands.
    app.waveform = vec![
        WaveBin {
            peak: 0.8,
            low: 0.3,
            mid: 0.2,
            high: 0.1
        };
        4_000
    ];

    let start = Instant::now();
    let full = snapshot(&app, true);
    let full_bytes = serde_json::to_vec(&full).unwrap().len();
    let elapsed = start.elapsed();
    assert_eq!(full["scene"]["moments"].as_array().unwrap().len(), 20_000);
    assert_eq!(full["scene"]["waveform"].as_array().unwrap().len(), 4_000);
    let status = snapshot(&app, false);
    assert!(status["scene"].is_null());
    app.seek(1200.0);
    assert_eq!(
        snapshot(&app, false)["musicalPosition"],
        "1.3.192 · 120.0 BPM · 4/4 · 1/4"
    );
    Action::Stop.run(&mut app);
    assert_eq!(app.position_ms(), 0.0);
    app.seek(1200.0);
    Action::Start.run(&mut app);
    assert_eq!(app.position_ms(), 0.0);
    app.locator_left = 1000.0;
    let mut start_mode = PlaybackStart::Cursor;
    Action::PlaybackStart {
        value: PlaybackStart::Left,
    }
    .run_with_start(&mut app, &mut start_mode);
    Action::Play.run_with_start(&mut app, &mut start_mode);
    assert_eq!(app.position_ms(), 1000.0);
    app.seek(0.0);
    Action::Key {
        key: " ".into(),
        code: "Space".into(),
        ctrl: false,
        shift: false,
        alt: false,
        lane_view: None,
    }
    .run_with_start(&mut app, &mut start_mode);
    assert_eq!(app.position_ms(), 1000.0);
    assert!(snapshot(&app, false)["returnOnStop"].as_bool().unwrap());
    Action::ReturnOnStop { on: false }.run(&mut app);
    assert!(!snapshot(&app, false)["returnOnStop"].as_bool().unwrap());
    let status_bytes = serde_json::to_vec(&status).unwrap().len();
    eprintln!(
        "20,000 objects and 4,000 bins: {full_bytes} byte scene ({elapsed:?}), {status_bytes} byte status"
    );

    let send_key = |app: &mut Application, key: &str, code: &str, ctrl, shift| {
        Action::Key {
            key: key.into(),
            code: code.into(),
            ctrl,
            shift,
            alt: false,
            lane_view: None,
        }
        .run(app);
    };
    let initial_snap = app.document().unwrap().composer.snap_enabled;
    send_key(&mut app, "j", "KeyJ", false, false);
    assert_ne!(app.document().unwrap().composer.snap_enabled, initial_snap);
    send_key(&mut app, "!", "Digit1", false, true);
    assert_eq!(app.document().unwrap().composer.spacing.divisions(), 1);
    let row_height = app.row_height;
    send_key(&mut app, "ArrowDown", "ArrowDown", true, false);
    assert!(app.row_height > row_height);
    app.locator_right = 1200.0;
    send_key(&mut app, "2", "Numpad2", false, false);
    assert_eq!(app.position_ms(), 1200.0);
    app.choose_key_preset("osu!");
    assert_eq!(snapshot(&app, false)["keyPreset"], "osu!");
    send_key(&mut app, "ArrowRight", "ArrowRight", false, true);
    assert_eq!(app.position_ms(), 3000.0);
    send_key(&mut app, "ArrowLeft", "ArrowLeft", false, true);
    assert_eq!(app.position_ms(), 1000.0);
    Action::Command { id: "seekForward".into() }.run(&mut app);
    assert_eq!(app.position_ms(), 1500.0);
    Action::Command { id: "seekForwardFar".into() }.run(&mut app);
    assert_eq!(app.position_ms(), 3500.0);

    let clap = full["scene"]["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|lane| lane["name"] == "normal-hitclap")
        .unwrap() as i32;
    Action::Tool {
        name: "Draw".into(),
    }
    .run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(),
            ms: 1000.0,
            row: clap,
            x: 10.0,
            y: 10.0,
            value: None,
            shift: false,
            ctrl: false,
            alt: false,
        }
        .run(&mut app);
    }
    Action::Tool {
        name: "Select".into(),
    }
    .run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(),
            ms: 1000.0,
            row: clap,
            x: 10.0,
            y: 10.0,
            value: None,
            shift: false,
            ctrl: false,
            alt: false,
        }
        .run(&mut app);
    }
    send_key(&mut app, "ArrowRight", "ArrowRight", true, false);
    let composer = &app.document().unwrap().composer;
    let lane = &composer.lanes[clap as usize];
    assert!(
        composer
            .hitsounds()
            .moments()
            .iter()
            .any(|moment| { moment.time().ms() == 1500.0 && moment.has_hit(lane) })
    );
    send_key(&mut app, "z", "KeyZ", true, false);
    assert!(snapshot(&app, false)["dirty"].as_bool().unwrap());
    Action::Open {
        path: expert.clone(),
    }
    .run(&mut app);
    let pending = snapshot(&app, true);
    assert_eq!(pending["pendingOpen"], true);
    assert_eq!(
        pending["scene"]["path"],
        hard.canonicalize().unwrap().to_str().unwrap()
    );

    Action::Pending {
        choice: "cancel".into(),
    }
    .run(&mut app);
    send_key(&mut app, "z", "KeyZ", true, false);
    assert!(!app.is_dirty());
    open(&mut app, &expert);
    assert!(app.document().unwrap().title.contains("Expert"));
    assert_eq!(fs::read_to_string(&hard).unwrap(), map);
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
fn copy_review_uses_the_exact_requested_options() {
    use dawsu_foundation::SampleSet;
    use dawsu_hitsounding::copy::{CopyMode, CopyOptions};
    use serde_json::json;

    let root = Fixture::new("copy-options");
    let source = root.0.join("Source.osu");
    let target = root.0.join("Target.osu");
    let map = "osu file format v14\n\n[General]\nMode:0\n\n[Metadata]\nVersion:DIFF\n\n[Events]\nEVENTS\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    let source_bytes = map.replace("DIFF", "Source").replace(
        "EVENTS",
        "Sample,1005,0,\"near.wav\",70\nSample,1200,0,\"far.wav\",60",
    );
    let target_bytes = map
        .replace("DIFF", "Target")
        .replace("EVENTS", "Sample,1400,0,\"target.wav\",80");
    fs::write(&source, &source_bytes).unwrap();
    fs::write(&target, &target_bytes).unwrap();

    let action = serde_json::from_value::<Action>(json!({
        "kind": "copy",
        "targets": [&target],
        "options": {
            "mode": "overwrite_only_defined",
            "leniency": 5.0,
            "start_index": 123,
            "never_silent_hitnormals": true,
            "copy_hitsounds": true,
            "copy_body_hitsounds": false,
            "copy_to_slider_ticks": true,
            "copy_to_slider_slides": true,
            "copy_sample_sets": true,
            "copy_volumes": false,
            "always_preserve_5_volume": false,
            "copy_storyboarded_samples": true,
            "ignore_hitsound_satisfied_samples": false,
            "ignore_whenever_hitsound": true,
            "mute_slider_ends": true,
            "beat_divisors": [4, 1, 3],
            "muted_divisors": [4, 3],
            "min_length": 0.75,
            "muted_index": 7,
            "muted_sample_set": "drum"
        }
    })).unwrap();
    let Action::Copy { options, .. } = &action else {
        panic!("Copy action did not deserialize");
    };
    assert_eq!(*options, CopyOptions {
        mode: CopyMode::OverwriteOnlyDefined,
        leniency: 5.0,
        start_index: 123,
        never_silent_hitnormals: true,
        copy_hitsounds: true,
        copy_body_hitsounds: false,
        copy_to_slider_ticks: true,
        copy_to_slider_slides: true,
        copy_sample_sets: true,
        copy_volumes: false,
        always_preserve_5_volume: false,
        copy_storyboarded_samples: true,
        ignore_hitsound_satisfied_samples: false,
        ignore_whenever_hitsound: true,
        mute_slider_ends: true,
        beat_divisors: vec![4, 1, 3],
        muted_divisors: vec![4, 3],
        min_length: 0.75,
        muted_index: Some(7),
        muted_sample_set: Some(SampleSet::Drum),
    });

    let mut app = Application::headless();
    open(&mut app, &source);
    action.run(&mut app);
    wait_for_job(&mut app);
    assert!(app.output_review.is_some());
    assert_eq!(fs::read_to_string(&target).unwrap(), target_bytes);

    Action::Output { apply: true }.run(&mut app);
    wait_for_job(&mut app);
    let target_after = fs::read_to_string(&target).unwrap();
    assert!(target_after.contains("Sample,1200,0,\"far.wav\",60"));
    assert!(target_after.contains("Sample,1400,0,\"target.wav\",80"));
    assert!(!target_after.contains("near.wav"));
    assert_eq!(fs::read_to_string(&source).unwrap(), source_bytes);
}

#[test]
fn home_route_preserves_cancel_and_failed_save_and_reloads_after_discard() {
    use super::home::Route;

    let fixture = Fixture::new("home");
    let map = fixture.0.join("Hard.osu");
    let recent = fixture.0.join("Recent.osu");
    let original = "osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,1,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    fs::write(&map, original).unwrap();
    fs::write(&recent, original).unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let mut route = Route::default();
    let row = snapshot(&app, true)["scene"]["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["name"] == "normal-hitclap").unwrap() as i32;
    Action::Tool { name: "Draw".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1000.0, row, x: 12.0, y: 12.0,
            value: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    assert!(app.is_dirty());

    let send = |route: &mut Route, app: &mut Application, action| {
        assert!(route.handle(action, app).is_none());
        let mut view = snapshot(app, false);
        route.add_status(&mut view);
        view
    };
    let confirm = |choice: &str| Action::HomeChoice { choice: choice.into() };
    assert_eq!(send(&mut route, &mut app, Action::Home)["pendingHome"], true);
    assert_eq!(send(&mut route, &mut app, confirm("cancel"))["pendingHome"], false);
    assert!(app.is_dirty());
    assert!(!route.visible());

    let edited_on_disk = format!("{original}// External edit\n");
    fs::write(&map, &edited_on_disk).unwrap();
    send(&mut route, &mut app, Action::Home);
    send(&mut route, &mut app, confirm("save"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    route.settle(&mut app);
    assert!(app.error.is_some());
    assert!(app.is_dirty());
    assert!(!route.visible());
    assert_eq!(fs::read_to_string(&map).unwrap(), edited_on_disk);
    Action::DismissError.run(&mut app);

    send(&mut route, &mut app, Action::Home);
    send(&mut route, &mut app, confirm("discard"));
    wait_for_job(&mut app);
    route.settle(&mut app);
    assert!(route.visible());
    assert!(!app.is_dirty());
    assert_eq!(fs::read_to_string(&map).unwrap(), edited_on_disk);
    send(&mut route, &mut app, Action::Resume);
    assert!(!route.visible());

    send(&mut route, &mut app, Action::Home);
    assert!(route.visible());
    send(&mut route, &mut app, Action::Open { path: recent.clone() });
    assert!(route.visible());
    wait_for_job(&mut app);
    route.settle(&mut app);
    assert!(!route.visible());
    assert_eq!(app.document().unwrap().path, recent.canonicalize().unwrap());
}

#[test]
fn slider_body_selection_exposes_its_slide_whistle_and_ticked_set() {
    let Some(root) = isolated_save_fixture("slider-body-selection",
        "tests::slider_body_selection_exposes_its_slide_whistle_and_ticked_set") else { return };
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
        assert!(app.error.is_none(), "{:?}", app.error);
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


fn isolated_save_fixture(label: &str, test: &str) -> Option<Fixture> {
    const PROFILE: &str = "DAWSU_TAURI_SAVE_TEST_PROFILE";
    let root = Fixture::new(label);
    if let Some(profile) = std::env::var_os(PROFILE) {
        assert_eq!(std::env::var_os("LOCALAPPDATA"), Some(profile));
        return Some(root);
    }
    // Give the test a private backup folder without changing other tests' environment.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test])
        .env(PROFILE, &root.0)
        .env("LOCALAPPDATA", &root.0)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
    None
}

#[test]
fn save_applies_volume_floor_only_to_the_edited_hit() {
    let Some(root) = isolated_save_fixture("volume-floor",
        "tests::save_applies_volume_floor_only_to_the_edited_hit") else { return };
    let path = root.0.join("Volume.osu");
    let map = concat!(
        "osu file format v14\r\n\r\n[TimingPoints]\r\n0,500,4,1,1,100,1,0\r\n\r\n",
        "[HitObjects]\r\n256,192,1000,1,0,0:0:0:80:\r\n256,192,2000,1,0,0:0:0:2:\r\n",
    );
    fs::write(&path, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let row = snapshot(&app, true)["scene"]["moments"][0][3][0][0]
        .as_i64().unwrap() as i32;
    Action::Tool { name: "Select".into() }.run(&mut app);
    for phase in ["down", "up"] {
        Action::Pointer {
            phase: phase.into(), ms: 1000.0, row, x: 12.0, y: 12.0,
            value: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    assert_eq!(snapshot(&app, true)["scene"]["selection"]["count"], 1);
    Action::Volume { value: 3, relative: false }.run(&mut app);
    assert!(app.is_dirty());
    Action::Save.run(&mut app);
    wait_for_job(&mut app);
    assert!(!app.is_dirty());
    assert_eq!(fs::read(&path).unwrap(), map.replace("0:0:0:80:", "0:0:0:5:").as_bytes());

    open(&mut app, &path);
    let volumes: Vec<_> = app.document().unwrap().composer.hitsounds().moments()
        .iter().map(|moment| moment.volume()).collect();
    assert_eq!(volumes, [5, 2]);
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
            value: None, shift: false, ctrl: false, alt: false,
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
    let mut route = super::home::Route::default();
    Action::ChangeShortcut { id: "help".into(), chord: None }.run(&mut app);
    let change = serde_json::from_value::<Action>(serde_json::json!({
        "kind": "change_shortcut", "id": "help",
        "chord": {"key":"F5", "code":"F5", "ctrl":false, "shift":false, "alt":false},
    })).unwrap();
    change.run(&mut app);
    assert!(app.error.is_none());

    let key = |name: &str| Action::Key {
        key: name.into(), code: name.into(), ctrl: false, shift: false, alt: false, lane_view: None,
    };
    assert!(route.handle(key("F1"), &mut app).is_none());
    let action = route.handle(key("F5"), &mut app).unwrap();
    action.run(&mut app);
    assert!(app.help_requested);
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
            value: None, shift: false, ctrl: false, alt: false,
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
                phase: phase.into(), ms, row, x: ms, y: 12.0, value: None,
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
    let bins = state["scene"]["waveform"].as_array().unwrap();
    assert!(bins.len() <= 4000);
    for (segment, amplitude) in [0.4, 0.6, 0.8].into_iter().enumerate() {
        let first = (segment * 3 + 1) * bins.len() / 9;
        let last = (segment * 3 + 2) * bins.len() / 9;
        let mut levels = [0.0f64; 4];
        for bin in &bins[first..last] {
            for (index, level) in levels.iter_mut().enumerate() {
                *level = level.max(bin[index].as_f64().unwrap());
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


#[test]
fn displayed_tab_and_plain_home_shortcuts_reach_the_command_resolver() {
    use dawsu_editor::keys::{Key, KeyCombination, Mods};
    let mut app = Application::headless();
    app.change_shortcut("help", Some(KeyCombination::new(Key::Tab, Mods::CTRL))).unwrap();
    app.change_shortcut("open", Some(KeyCombination::plain(Key::H))).unwrap();
    let state = snapshot(&app, false);
    assert_eq!(state["tabRoutes"], serde_json::json!([[true, false, false, true]]));
    let mut route = super::home::Route::default();
    let tab = Action::Key {
        key: "Tab".into(), code: "Tab".into(), ctrl: true, shift: false, alt: false, lane_view: None,
    };
    route.handle(tab, &mut app).unwrap().run(&mut app);
    assert!(app.help_requested);
    let open = Action::Key {
        key: "h".into(), code: "KeyH".into(), ctrl: false, shift: false, alt: false, lane_view: None,
    };
    route.handle(open, &mut app).unwrap().run(&mut app);
    assert!(matches!(app.requests.last(), Some(dawsu_application::PlatformRequest::OpenMap)));
}

#[test]
fn home_save_with_left_out_moments_goes_home_after_the_summarys_save() {
    use super::home::Route;

    let fixture = Fixture::new("home-left-out");
    let map = fixture.0.join("a.osu");
    fs::write(&map, "osu file format v14\r\n\r\n[General]\r\nMode: 0\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,8,2:0:0:0:\r\n256,192,1500,1,8,3:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n").unwrap();
    // A data folder of its own gives the difficulty an editor project to keep left-out moments.
    let mut app = Application::with_data_root(fixture.0.join("data"));
    app.error = None;
    open(&mut app, &map);
    Action::HsMode { on: true }.run(&mut app);
    let draw_claps = |app: &mut Application| {
        Action::Tool { name: "Draw".into() }.run(app);
        for name in ["soft-hitclap", "drum-hitclap"] {
            let row = snapshot(app, true)["scene"]["lanes"].as_array().unwrap().iter()
                .position(|lane| lane["name"] == name).unwrap() as i32;
            for phase in ["down", "up"] {
                Action::Pointer {
                    phase: phase.into(), ms: 2500.0, row, x: 12.0, y: 12.0,
                    value: None, shift: false, ctrl: false, alt: false,
                }.run(app);
            }
        }
    };
    draw_claps(&mut app);
    assert!(app.is_dirty());
    let mut route = Route::default();
    let confirm = |choice: &str| Action::HomeChoice { choice: choice.into() };
    let step = |route: &mut Route, app: &mut Application, action: Action| {
        if let Some(action) = route.handle(action, app) {
            action.run(app);
        }
        wait_for_job(app);
        route.settle(app);
    };

    // Cancel in the summary keeps the editor open with the edits.
    step(&mut route, &mut app, Action::Home);
    step(&mut route, &mut app, confirm("save"));
    assert!(app.save_problem_review(), "Home → Save shows the Save hitsounds summary first");
    assert!(!route.visible());
    step(&mut route, &mut app, Action::DismissSaveProblemReview);
    assert!(!route.visible());
    assert!(app.is_dirty());

    // The summary's Save writes the rest, then goes Home.
    step(&mut route, &mut app, Action::Home);
    step(&mut route, &mut app, confirm("save"));
    assert!(app.save_problem_review());
    step(&mut route, &mut app, Action::AnswerSaveReview { apply: true });
    assert!(!app.is_dirty());
    assert!(route.visible(), "Home continues after the summary's Save");
}

#[test]
fn drum_window_requests_answer_with_its_view_and_never_touch_main_errors() {
    use super::drum_window::{DrumRequest, OptionsJson, SourceJson, answer};
    let fixture = Fixture::new("drum-window");
    let map = fixture.0.join("map.osu");
    fs::write(&map, "osu file format v14\n[General]\nMode:0\nSampleSet:Soft\n[TimingPoints]\n0,500,4,2,0,100,1,0\n[HitObjects]\n256,192,500,1,10,0:0:0:0:\n").unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    assert!(snapshot(&app, false)["drumDetect"].is_null(), "main's view no longer carries drum detection");
    let view = answer(&mut app, DrumRequest::Context { known: None }).unwrap();
    assert_eq!(view["state"], "idle");
    assert_eq!(view["hasSong"], false);
    assert_eq!(view["options"]["sensitivity"], 50);
    assert_eq!(view["editorGrid"], 4);
    let keys: Vec<String> = view["lanes"].as_array().unwrap().iter()
        .map(|lane| lane["key"].as_str().unwrap().to_owned()).collect();
    let lanes = app.document().unwrap().composer.hitsounds().lanes().iter()
        .map(|lane| scene_lane_key(&lane.identity)).collect::<Vec<_>>();
    assert_eq!(keys, lanes);
    assert!(view["lanes"].as_array().unwrap().iter().all(|lane| lane["hidden"] == false));
    let kick = keys.iter().find(|key| key.contains("Whistle")).unwrap().clone();
    let path = app.document().unwrap().path.clone();
    let options = |kick: &str, snare: Option<&str>| OptionsJson {
        kick: Some(kick.to_owned()), snare: snare.map(str::to_owned), sensitivity: 50, object_ms: 30.0, grid: None, stem_offset: 0.0,
    };
    let view = answer(&mut app, DrumRequest::Review { map: path.clone(), options: options(&kick, None) }).unwrap();
    assert_eq!(view["options"]["kick"], kick.as_str());
    assert_eq!(answer(&mut app, DrumRequest::Review { map: path.clone(), options: options(&kick, Some(&kick)) }).unwrap_err(),
        "Pick different lanes for kicks and snares.");
    assert_eq!(answer(&mut app, DrumRequest::Start { map: path.clone(), source: SourceJson::Song }).unwrap_err(),
        "This difficulty's song could not be loaded. Pick a drum stem.");
    let other = DrumRequest::Start { map: fixture.0.join("other.osu"), source: SourceJson::Song };
    assert!(answer(&mut app, other).unwrap_err().contains("difficulty changed"));
    let apply = DrumRequest::Apply { map: path.clone(), review: "1".into(), ticked: Vec::new() };
    assert_eq!(answer(&mut app, apply).unwrap_err(), "Detect drum hits first.");
    assert!(app.error.is_none(), "drum requests answer the drum window, not main");

    // A drum stem (the detector's test pattern at half level): the rows come once per review, and
    // Apply adds the checked ones.
    let pattern = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/dawsu-graph-audio/tests/fixtures/classic-3.1.4/pattern.f32");
    let pcm: Vec<u8> = fs::read(pattern).unwrap().chunks_exact(4)
        .flat_map(|b| (((f32::from_le_bytes(b.try_into().unwrap()) * 0.5).clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + pcm.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    for field in [16_u32, 1 | 1 << 16, 44_100, 88_200, 2 | 16 << 16] {
        wav.extend(field.to_le_bytes());
    }
    wav.extend(b"data");
    wav.extend((pcm.len() as u32).to_le_bytes());
    wav.extend(pcm);
    let stem = fixture.0.join("drums.wav");
    fs::write(&stem, wav).unwrap();
    answer(&mut app, DrumRequest::Start { map: path.clone(), source: SourceJson::Stem { path: stem } }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let view = loop {
        let view = answer(&mut app, DrumRequest::Context { known: None }).unwrap();
        if view["state"] == "ready" {
            break view;
        }
        assert_eq!(view["state"], "running", "{view}");
        assert!(Instant::now() < deadline, "detection did not finish");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(view["source"]["name"], "drums.wav");
    let review = view["review"].as_str().unwrap().to_owned();
    let rows = view["rows"].as_array().unwrap();
    assert_eq!(rows[0]["drum"], "kick");
    assert_eq!(rows[0]["lands"], "circle");
    assert!(rows.iter().any(|row| row["drum"] == "snare" && row["lands"] == "noLane"));
    let again = answer(&mut app, DrumRequest::Context { known: Some(review.clone()) }).unwrap();
    assert!(again.get("rows").is_none(), "rows are sent only when the review changed");
    let ticked = vec![rows[0]["id"].as_str().unwrap().to_owned()];
    let view = answer(&mut app, DrumRequest::Apply { map: path, review, ticked }).unwrap();
    assert_eq!(view["state"], "idle");
    assert!(app.status.starts_with("Added 1 drum hits: 1 on objects"), "{}", app.status);
}

#[test]
fn drum_window_lanes_carry_the_numbers_their_lane_headers_show() {
    use super::drum_window::{DrumRequest, answer};
    let fixture = Fixture::new("drum-window-numbers");
    let map = fixture.0.join("map.osu");
    // Soft hitnormal at custom index 0 and 1 share the name "soft-hitnormal"; a whistle lane is hidden.
    fs::write(&map, "osu file format v14\n[General]\nMode:0\nSampleSet:Soft\n[TimingPoints]\n0,500,4,2,0,100,1,0\n[HitObjects]\n256,192,500,1,2,0:0:0:0:\n256,192,1000,1,0,2:0:1:0:\n256,192,1500,1,0,1:0:0:0:\n").unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let whistle = app.document().unwrap().composer.hitsounds().lanes().iter()
        .find(|lane| dawsu_application::lane_name(&lane.identity).contains("whistle")).unwrap().identity.clone();
    let revision = app.document().unwrap().composer.history.version();
    let path = app.document().unwrap().path.clone();
    app.delete_lane(&path, &whistle, revision).unwrap();
    assert!(app.document().unwrap().composer.hitsounds().is_lane_hidden(&whistle), "Delete lane hides it");
    let view = answer(&mut app, DrumRequest::Context { known: None }).unwrap();
    let lanes = view["lanes"].as_array().unwrap();
    // The header numbers: each row's place in the scene's lane order.
    let scene = &snapshot(&app, true)["scene"];
    let headers: std::collections::HashMap<String, u64> = scene["laneOrder"].as_array().unwrap().iter().enumerate()
        .map(|(n, row)| (scene["lanes"][row.as_u64().unwrap() as usize]["sourceKey"].as_str().unwrap().to_owned(), n as u64 + 1))
        .collect();
    for lane in lanes {
        let key = lane["key"].as_str().unwrap();
        assert_eq!(lane["number"].as_u64(), headers.get(key).copied(), "{key}");
    }
    let soft: Vec<&serde_json::Value> = lanes.iter().filter(|lane| lane["name"] == "soft-hitnormal").collect();
    assert_eq!(soft.len(), 2);
    assert_ne!(soft[0]["number"], soft[1]["number"]);
    assert!(soft.iter().all(|lane| lane["number"].is_u64()));
    let hidden = lanes.iter().find(|lane| lane["hidden"] == true).unwrap();
    assert!(hidden["number"].is_null(), "a hidden lane has no header, so no number");
    // A lane order saved by moving lanes reorders the numbers the same way.
    let layout_keys: Vec<String> = scene["laneOrder"].as_array().unwrap().iter().rev()
        .map(|row| scene["lanes"][row.as_u64().unwrap() as usize]["colourKey"].as_str().unwrap().to_owned()).collect();
    app.set_lane_layout(dawsu_application::LaneLayout { lane_order: layout_keys, ..Default::default() }).unwrap();
    let view = answer(&mut app, DrumRequest::Context { known: None }).unwrap();
    let count = headers.len() as u64;
    let family: Vec<&str> = soft.iter().map(|lane| lane["key"].as_str().unwrap()).collect();
    for lane in view["lanes"].as_array().unwrap() {
        let key = lane["key"].as_str().unwrap();
        // The soft hitnormal family keeps its main lane (index 1) above its skin lane, so within the
        // reversed order the pair keeps its own order.
        let from = match family.iter().position(|member| *member == key) {
            Some(at) => family[1 - at],
            None => key,
        };
        assert_eq!(lane["number"].as_u64(), headers.get(from).map(|n| count + 1 - n), "{key} reversed");
    }
}

#[test]
fn place_sound_set_action_places_the_picked_set_at_the_playhead_or_reports_why_not() {
    use dawsu_hitsounding::sound_sets::{Kept, SetLane, SetMember, SetSource, SoundSet, write_dawsuset};
    let fixture = Fixture::new("place-sound-set");
    let path = fixture.0.join("Hard.osu");
    fs::write(&path, b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,2000,1,0,0:0:0:60:\n").unwrap();
    let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
    wav.extend(1000i16.to_le_bytes());
    let set = SoundSet {
        id: "9e7b1d18-3186-4778-9a6a-b6baef95cbe7".into(), name: "Clap".into(), library_id: None, kept: Kept::default(),
        members: vec![SetMember {
            id: "aaaaaaaa-0000-4000-8000-000000000001".into(), original_key: "BeatmapFile|soft-hitclap.wav".into(),
            lane: SetLane { source: SetSource::File, name: "soft-hitclap.wav".into(), missing: false, kept: Kept::default() },
            slot: "soft-hitclap".into(), volume: 80, audio: wav, kept: Kept::default(),
        }],
    };
    let file = fixture.0.join("Clap.dawsuset");
    fs::write(&file, write_dawsuset(&set).unwrap()).unwrap();
    let mut app = Application::with_data_root(fixture.0.join("data"));
    app.error = None;
    open(&mut app, &path);
    let map = snapshot(&app, true)["scene"]["path"].clone();
    let place = |app: &mut Application| serde_json::from_value::<Action>(serde_json::json!({
        "kind": "place_sound_set", "map": map, "path": file,
    })).unwrap().run(app);
    app.preferences.hitsound_library = None;
    place(&mut app);
    assert!(app.error.take().is_some_and(|e| e.contains("hitsound library")));
    app.preferences.hitsound_library = Some(fixture.0.join("library"));
    app.seek(2000.0);
    place(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    let hits = app.document().unwrap().composer.hitsounds();
    assert_eq!(hits.placements().len(), 1);
    assert!(fixture.0.join("soft-hitclap2.wav").is_file());
    assert!(fixture.0.join("library").join("Sound sets").join("Clap.dawsuset").is_file());
}

#[test]
fn only_closing_the_main_window_closes_the_editor() {
    use super::{CloseDecision, Editor, Request, close_requested, closed_with_main};
    let (send, receive) = std::sync::mpsc::channel();
    let editor = Editor(send);
    assert_eq!(close_requested("drum", &editor), CloseDecision::Allow);
    assert!(matches!(receive.try_recv(), Ok(Request::DrumClose)), "closing drum detection stops its run and Listen");
    assert!(receive.try_recv().is_err());
    assert_eq!(close_requested("export", &editor), CloseDecision::Allow);
    assert!(matches!(receive.try_recv(), Ok(Request::ExportClose)), "closing Export cancels its preview");
    assert!(receive.try_recv().is_err());
    assert_eq!(closed_with_main(["main", "export", "drum"]), ["export", "drum"]);

    let refusal = "Save or discard the current difficulty before quitting.";
    let responder = std::thread::spawn(move || {
        [Ok(()), Err(refusal.to_owned())].into_iter().map(|answer| match receive.recv().unwrap() {
            Request::Close(discard, response) => {
                response.send(answer).unwrap();
                discard
            }
            _ => panic!("closing main sends only Close"),
        }).collect::<Vec<_>>()
    });
    assert_eq!(close_requested("main", &editor), CloseDecision::CloseAll);
    assert_eq!(close_requested("main", &editor), CloseDecision::Keep(refusal.into()));
    assert_eq!(responder.join().unwrap(), [false, false]);
}

#[test]
fn export_requests_answer_with_the_export_view_and_errors_after_a_fault() {
    use super::{Request, answer_export};
    use dawsu_application::ExportOptions;
    use std::sync::mpsc::channel;

    let root = Fixture::new("export-view");
    let map = "osu file format v14\n\n[General]\nMode:0\n\n[Metadata]\nVersion:DIFF\n\n[Events]\nEVENTS\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
    let source = root.0.join("Source.osu");
    let target = root.0.join("Target.osu");
    fs::write(&source, map.replace("DIFF", "Source").replace("EVENTS", "Sample,1200,0,\"far.wav\",60")).unwrap();
    fs::write(&target, map.replace("DIFF", "Target").replace("EVENTS", "")).unwrap();
    let target = target.canonicalize().unwrap();
    let mut app = Application::headless();
    open(&mut app, &source);
    let mut fault = None;
    let mut cache = super::ExportCache::default();

    let source = app.document().unwrap().path.clone();
    let (tx, rx) = channel();
    answer_export(&mut app, &mut fault, &mut cache, Request::ExportPreview(target.clone(), ExportOptions {
        targets: vec![target.clone()],
        ..Default::default()
    }, tx));
    assert_eq!(rx.recv().unwrap().unwrap_err(), "Another difficulty was opened. Preview again.");
    assert_eq!(app.document().unwrap().export_choices(), None, "a refused preview stores no choices");
    let (tx, rx) = channel();
    answer_export(&mut app, &mut fault, &mut cache, Request::ExportPreview(source.clone(), ExportOptions {
        targets: vec![target.clone()],
        ..Default::default()
    }, tx));
    let id = rx.recv().unwrap().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let view = loop {
        app.poll();
        let (tx, rx) = channel();
        answer_export(&mut app, &mut fault, &mut cache, Request::ExportView(tx));
        let view = rx.recv().unwrap().unwrap();
        if view["preview"].is_object() {
            break view;
        }
        assert!(Instant::now() < deadline, "{view}");
        std::thread::sleep(Duration::from_millis(5));
    };
    let document = app.document().unwrap();
    assert_eq!(view["source"]["path"], serde_json::json!(document.path));
    assert_eq!(view["source"]["version"], document.composer.history.version());
    assert_eq!(view["source"]["hits"], 1);
    assert_eq!(view["source"]["lanes"][0]["hitCount"], 1);
    assert!(view["source"]["lanes"][0]["colourKey"].is_string());
    assert!(view["source"]["laneColours"].is_object());
    assert_eq!(view["targets"], serde_json::json!([target]));
    assert_eq!(view["backups"], serde_json::json!(app.backups_folder()));
    assert_eq!(view["busy"], false);
    assert_eq!(view["previewing"], false);
    assert!(view["error"].is_null());
    assert_eq!(view["preview"]["id"], id);
    assert_eq!(view["preview"]["origin"], "export");
    assert_eq!(view["preview"]["stale"], false);
    assert_eq!(view["preview"]["files"], serde_json::json!([target]));
    let counts = &view["preview"]["targets"][0];
    assert_eq!(counts["path"], serde_json::json!(target));
    for field in ["matched", "unmatched", "changed", "collisions", "keptFromSilent", "muted"] {
        assert!(counts[field].is_u64(), "{field}: {counts}");
    }
    assert_eq!(view["choices"], serde_json::json!({
        "leftOut": [], "matchWithin": 5.0, "copyVolumes": true, "base": false, "sliderAudio": false,
    }));
    assert!(view["preview"]["base"].is_null(), "no base difficulty without the base choice");
    assert_eq!((&view["writing"], &view["written"], &view["writeError"]), (&serde_json::json!(false), &serde_json::Value::Null, &serde_json::Value::Null));
    assert!(view.get("review").is_none() && view.get("shortcutAction").is_none());
    assert!(app.output_review.is_none());

    let (tx, rx) = channel();
    answer_export(&mut app, &mut fault, &mut cache, Request::ExportRun(id + 1, tx));
    assert!(rx.recv().unwrap().is_err());

    fault = Some("The editor stopped.".into());
    let (view, view_rx) = channel();
    answer_export(&mut app, &mut fault, &mut cache, Request::ExportView(view));
    let (preview, preview_rx) = channel();
    answer_export(&mut app, &mut fault, &mut cache, Request::ExportPreview(source.clone(), ExportOptions {
        targets: vec![target.clone()],
        ..Default::default()
    }, preview));
    let (run, run_rx) = channel();
    answer_export(&mut app, &mut fault, &mut cache, Request::ExportRun(id, run));
    let wait = Duration::from_millis(100);
    assert_eq!(view_rx.recv_timeout(wait).unwrap().unwrap_err(), "The editor stopped.");
    assert_eq!(preview_rx.recv_timeout(wait).unwrap().unwrap_err(), "The editor stopped.");
    assert_eq!(run_rx.recv_timeout(wait).unwrap().unwrap_err(), "The editor stopped.");
}

#[test]
fn export_preview_waits_without_an_error_while_a_drag_is_open() {
    use dawsu_application::ExportOptions;
    let root = Fixture::new("export-drag");
    let path = root.0.join("Source.osu");
    let target = root.0.join("Target.osu");
    let map = "osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,10,0:0:0:60:\n";
    fs::write(&path, map).unwrap();
    fs::write(&target, map).unwrap();
    let mut app = Application::headless();
    open(&mut app, &path);
    let send = |app: &mut Application, phase: &str| {
        Action::Pointer {
            phase: phase.into(), ms: 1000.0, row: 0, x: 1000.0, y: 70.0,
            value: Some(30.0), shift: false, ctrl: false, alt: false,
        }.run(app);
    };
    send(&mut app, "down");
    // The window sends the paths export_view listed, which are canonical.
    let target = target.canonicalize().unwrap();
    app.request_export_preview(ExportOptions { targets: vec![target], ..Default::default() }).unwrap();
    app.poll();
    let status = app.export_status();
    assert!(status.waiting && !status.running, "the preview waits for the drag");
    assert_eq!(status.error, None);
    assert!(app.error.is_none(), "{:?}", app.error);
    send(&mut app, "up");
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.export_status().preview.is_none() {
        app.poll();
        assert_eq!(app.export_status().error, None);
        assert!(Instant::now() < deadline, "the preview did not run after the drag");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_library_list_names_each_set_and_places_one_by_its_id() {
    use dawsu_hitsounding::sound_sets::{Kept, SetLane, SetMember, SetSource, SoundSet, write_dawsuset};
    let fixture = Fixture::new("library-sound-set");
    let path = fixture.0.join("Hard.osu");
    fs::write(&path, b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,2000,1,0,0:0:0:60:\n").unwrap();
    let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
    wav.extend(1000i16.to_le_bytes());
    let set = SoundSet {
        id: "9e7b1d18-3186-4778-9a6a-b6baef95cbe7".into(), name: "Clap".into(), library_id: None, kept: Kept::default(),
        members: vec![SetMember {
            id: "aaaaaaaa-0000-4000-8000-000000000001".into(), original_key: "BeatmapFile|soft-hitclap.wav".into(),
            lane: SetLane { source: SetSource::File, name: "soft-hitclap.wav".into(), missing: false, kept: Kept::default() },
            slot: "soft-hitclap".into(), volume: 80, audio: wav, kept: Kept::default(),
        }],
    };
    let folder = fixture.0.join("library").join("Sound sets");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("Clap.dawsuset"), write_dawsuset(&set).unwrap()).unwrap();
    let mut app = Application::with_data_root(fixture.0.join("data"));
    app.error = None;
    app.preferences.hitsound_library = Some(fixture.0.join("library"));
    open(&mut app, &path);
    let deadline = Instant::now() + Duration::from_secs(10);
    while sound_set_library_view(&app)["reading"] == true {
        assert!(Instant::now() < deadline, "the library is read in the background");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(sound_set_library_view(&app), serde_json::json!({
        "configured": true,
        "sets": [{ "id": set.id, "name": "Clap", "sounds": 1, "inMap": false, "duplicate": false, "missing": false }],
        "unreadable": [], "reading": false,
    }), "names, ids and counts only");
    app.preferences.hitsound_library = Some(fixture.0.join("gone"));
    assert!(sound_set_library_view(&app)["error"].as_str().is_some_and(|e| e.starts_with("Hitsound library folder not found")));
    app.preferences.hitsound_library = Some(fixture.0.join("library"));
    let map = snapshot(&app, true)["scene"]["path"].clone();
    app.seek(2000.0);
    serde_json::from_value::<Action>(serde_json::json!({ "kind": "place_library_sound_set", "map": map, "id": set.id })).unwrap().run(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.document().unwrap().composer.hitsounds().placements().len(), 1);
    assert_eq!(sound_set_library_view(&app)["sets"][0]["inMap"], true);
}

#[test]
fn the_export_view_reads_targets_and_hit_counts_once_per_edit() {
    use super::{ExportCache, export_snapshot};
    let root = Fixture::new("export-cache");
    let map = "osu file format v14

[General]
Mode:0

[Metadata]
Version:DIFF

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,1000,1,0,0:0:0:0:
";
    let source = root.0.join("Source.osu");
    fs::write(&source, map.replace("DIFF", "Source")).unwrap();
    fs::write(root.0.join("Target.osu"), map.replace("DIFF", "Target")).unwrap();
    let mut app = Application::headless();
    open(&mut app, &source);
    let mut cache = ExportCache::default();
    let first = export_snapshot(&app, &mut cache);
    assert_eq!(cache.reads, 2, "targets and hit counts read once");
    for _ in 0..3 {
        assert_eq!(export_snapshot(&app, &mut cache), first);
    }
    assert_eq!(cache.reads, 2, "polls of an unchanged map read nothing again");
    Action::Command { id: "selectAll".into() }.run(&mut app);
    Action::Volume { value: -10, relative: true }.run(&mut app);
    let edited = export_snapshot(&app, &mut cache);
    assert!(edited["source"]["version"] != first["source"]["version"]);
    assert_eq!(cache.reads, 3, "an edit reads the hit counts again, not the targets");
}

#[test]
fn the_export_window_is_titled_with_the_map_and_difficulty() {
    use super::export_title;
    let view = serde_json::json!({ "source": { "title": "yuikonnu - Kokuhaku", "difficulty": "Hard" } });
    assert_eq!(export_title(&view), "Export · yuikonnu - Kokuhaku [Hard]");
    // The editor's title already names the difficulty: it is not named twice.
    let named = serde_json::json!({ "source": { "title": "yuikonnu - Kokuhaku [Hard]", "difficulty": "Hard" } });
    assert_eq!(export_title(&named), "Export · yuikonnu - Kokuhaku [Hard]");
    assert_eq!(export_title(&serde_json::json!({ "source": null })), "Export");
}

/// The frontend's one command list (`ui/src/lib/commands.js`) names every catalogue command once,
/// either on a menu row (`command: "id"`) or under `keyOnly`, and names nothing the catalogue lacks.
#[test]
fn the_menu_command_list_matches_the_command_catalogue() {
    use std::collections::BTreeSet;
    let list = include_str!("../ui/src/lib/commands.js");
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

fn finish(index: u32) -> dawsu_foundation::SampleIdentity {
    dawsu_foundation::SampleIdentity::Standard {
        set: dawsu_foundation::SampleSet::Normal,
        sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Finish),
        index: dawsu_foundation::CustomIndex::new(index),
    }
}

#[test]
fn numbered_layers_sort_by_number_directly_under_their_main_lane() {
    use super::family_order;
    let clap = dawsu_foundation::SampleIdentity::Standard {
        set: dawsu_foundation::SampleSet::Normal,
        sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Clap),
        index: dawsu_foundation::CustomIndex::new(0),
    };
    let file = dawsu_foundation::SampleIdentity::File("kick.wav".into());
    let order = |lanes: &[&dawsu_foundation::SampleIdentity], rows: &[usize]| family_order(rows.to_vec(), |row| lanes[row]);
    // Finish 3 and Finish 2 were added after Clap and a map file; they move under Finish (skin), by number.
    let (skin, one, two, three) = (finish(0), finish(1), finish(2), finish(3));
    let lanes = [&skin, &clap, &file, &three, &two];
    assert_eq!(order(&lanes, &[0, 1, 2, 3, 4]), [0, 4, 3, 1, 2]);
    // The family's file at custom index 1 is its main lane; the skin sample is its first layer, and the
    // family sits where the main lane sits.
    let lanes = [&skin, &clap, &three, &two, &one];
    assert_eq!(order(&lanes, &[0, 1, 2, 3, 4]), [1, 4, 0, 3, 2]);
    // A saved order that splits the family does not split it.
    assert_eq!(order(&lanes, &[3, 1, 4, 2, 0]), [1, 4, 0, 3, 2]);
    // Without a main lane (removed, hidden or missing), layers stay ordinary lanes where they are.
    let lanes = [&clap, &three, &file, &two];
    assert_eq!(order(&lanes, &[0, 1, 2, 3]), [0, 1, 2, 3]);
    // Rows not in the list (hidden lanes) take no part.
    let lanes = [&skin, &clap, &two];
    assert_eq!(order(&lanes, &[1, 2]), [1, 2]);
}

#[test]
fn scene_and_drum_window_put_a_numbered_layer_under_its_main_lane_even_after_a_saved_order() {
    use super::drum_window::{DrumRequest, answer};
    let fixture = Fixture::new("layer-family-order");
    let map = fixture.0.join("map.osu");
    let tone = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    fs::write(fixture.0.join("normal-hitfinish3.wav"), tone).unwrap();
    // A skin finish, a clap and a whistle, then a finish on custom index 3.
    fs::write(&map, "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,4,0:0:0:0:\n256,192,1000,1,8,0:0:0:0:\n256,192,1500,1,2,0:0:0:0:\n256,192,2000,1,4,1:0:3:0:\n").unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let scene = snapshot(&app, true)["scene"].clone();
    let names: Vec<String> = scene["laneOrder"].as_array().unwrap().iter()
        .map(|row| scene["lanes"][row.as_u64().unwrap() as usize]["name"].as_str().unwrap().to_owned()).collect();
    let at = |name: &str| names.iter().position(|shown| shown == name).unwrap_or_else(|| panic!("{name} in {names:?}"));
    assert_eq!(at("normal-hitfinish3"), at("normal-hitfinish") + 1, "{names:?}");

    // The drum window numbers its lanes as the headers do, also after a saved order puts the layer first.
    let keys: Vec<String> = scene["laneOrder"].as_array().unwrap().iter().rev()
        .map(|row| scene["lanes"][row.as_u64().unwrap() as usize]["colourKey"].as_str().unwrap().to_owned()).collect();
    app.set_lane_layout(dawsu_application::LaneLayout { lane_order: keys, ..Default::default() }).unwrap();
    let view = answer(&mut app, DrumRequest::Context { known: None }).unwrap();
    let number = |name: &str| view["lanes"].as_array().unwrap().iter()
        .find(|lane| lane["name"] == name).and_then(|lane| lane["number"].as_u64()).unwrap();
    assert_eq!(number("normal-hitfinish3"), number("normal-hitfinish") + 1);
}

#[test]
fn a_folded_family_survives_saving_and_reopening_the_difficulty() {
    let fixture = Fixture::new("layer-family-fold");
    let map = fixture.0.join("map.osu");
    fs::write(&map, "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,4,0:0:0:0:\n").unwrap();
    let mut app = Application::with_data_root(fixture.0.join("data"));
    app.error = None;
    open(&mut app, &map);
    let path = app.document().unwrap().path.clone();
    serde_json::from_value::<Action>(serde_json::json!({
        "kind": "lane_layout", "path": path, "persist": true,
        "value": { "folder_order": ["normal", "soft", "drum"], "collapsed_folders": [],
            "collapsed_families": ["normal-hitfinish"], "lane_colours": {} },
    })).unwrap().run(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    open(&mut app, &map);
    let scene = snapshot(&app, true)["scene"].clone();
    assert_eq!(scene["laneLayout"]["collapsed_families"], serde_json::json!(["normal-hitfinish"]));
}

#[test]
fn removing_a_lane_takes_its_hits_in_one_step_and_keeps_its_layers() {
    let fixture = Fixture::new("remove-lane");
    let map = fixture.0.join("map.osu");
    let tone = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    fs::write(fixture.0.join("normal-hitfinish.wav"), tone).unwrap();
    fs::write(fixture.0.join("normal-hitfinish2.wav"), tone).unwrap();
    let original = "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,4,1:0:1:0:\n256,192,1000,1,4,1:0:1:0:\n256,192,1500,1,4,1:0:2:0:\n";
    fs::write(&map, original).unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let scene = |app: &Application| snapshot(app, true)["scene"].clone();
    let before = scene(&app);
    let row_of = |scene: &serde_json::Value, index: u64| scene["lanes"].as_array().unwrap().iter()
        .position(|lane| lane["family"] == "normal-hitfinish" && lane["index"] == index).unwrap();
    let shown = |scene: &serde_json::Value, row: usize| scene["laneOrder"].as_array().unwrap().iter()
        .any(|value| value.as_u64() == Some(row as u64));
    let main = row_of(&before, 1);
    let layer = row_of(&before, 2);
    assert_eq!(before["lanes"][main]["index"], 1);
    assert_eq!(before["lanes"][main]["hitCount"], 2);
    serde_json::from_value::<Action>(serde_json::json!({
        "kind": "delete_lane", "target": {"map": before["path"], "lane": before["lanes"][main]["sourceKey"]},
        "revision": before["laneRevision"],
    })).unwrap().run(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.status, "Removed normal-hitfinish and 2 hits.");
    let after = scene(&app);
    assert!(!shown(&after, main));
    assert_eq!(after["lanes"][main]["hitCount"], 0);
    // Only the main lane goes: its layer keeps its lane and its hit.
    assert!(shown(&after, layer));
    assert_eq!(after["lanes"][layer]["hitCount"], 1);
    // One undo step brings back the lane and both hits.
    Action::Command { id: "undo".into() }.run(&mut app);
    let undone = scene(&app);
    assert!(shown(&undone, main));
    assert_eq!(undone["lanes"][main]["hitCount"], 2);
    Action::Command { id: "redo".into() }.run(&mut app);

    // Nothing is written to the mapset on removal, and the sample file stays.
    assert_eq!(fs::read_to_string(&map).unwrap(), original);
    assert!(fixture.0.join("normal-hitfinish.wav").exists());
    // The saved map leaves the removed lane's hits out and keeps the layer's.
    Action::Command { id: "save".into() }.run(&mut app);
    wait_for_job(&mut app);
    let saved = fs::read_to_string(&map).unwrap();
    assert!(saved.contains("256,192,500,1,0,") && saved.contains("256,192,1000,1,0,"), "{saved}");
    assert!(saved.contains("256,192,1500,1,4,1:0:2:0:"), "{saved}");
}
