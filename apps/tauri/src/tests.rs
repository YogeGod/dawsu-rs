use super::{Action, PlaybackStart, snapshot};
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
    send_key(&mut app, "ArrowRight", "ArrowRight", false, true);
    assert_eq!(app.position_ms(), 3000.0);
    send_key(&mut app, "ArrowLeft", "ArrowLeft", false, true);
    assert_eq!(app.position_ms(), 1000.0);

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
            "copy_hitsounds": true,
            "copy_body_hitsounds": false,
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
        copy_hitsounds: true,
        copy_body_hitsounds: false,
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
        key: name.into(), code: name.into(), ctrl: false, shift: false, alt: false,
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
    Action::Key { key: "r".into(), code: "KeyR".into(), ctrl: true, shift: false, alt: false }.run(&mut app);
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
        "kind": "output_device", "name": "Other device",
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
fn rejected_buffer_request_keeps_the_device_preferences_and_transport() {
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

    Action::OutputBuffer { ms: Some(0) }.run(&mut app);
    assert!(app.error.as_deref().unwrap().contains("10 and 500"));
    assert_eq!(app.position_ms(), 350.0);
    assert_eq!((app.locator_left, app.locator_right, app.cycle), (200.0, 450.0, true));
    assert_eq!(app.song_offset, -17.0);
    assert_eq!(serde_json::to_vec(&app.preferences).unwrap(), saved);

    let restored: dawsu_application::Preferences = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored.audio_buffer_ms, Some(40));
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
        key: "Tab".into(), code: "Tab".into(), ctrl: true, shift: false, alt: false,
    };
    route.handle(tab, &mut app).unwrap().run(&mut app);
    assert!(app.help_requested);
    let open = Action::Key {
        key: "h".into(), code: "KeyH".into(), ctrl: false, shift: false, alt: false,
    };
    route.handle(open, &mut app).unwrap().run(&mut app);
    assert!(matches!(app.requests.last(), Some(dawsu_application::PlatformRequest::OpenMap)));
}