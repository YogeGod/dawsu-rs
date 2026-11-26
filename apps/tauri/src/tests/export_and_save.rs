use super::helpers::{Fixture, isolated_save_fixture, open, wait_for_job};
use crate::{Action, PlaybackStart, encode_waveform_bytes, snapshot, snapshot_view};
use dawsu_application::{Application, WaveBin};
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

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
                value: None, button: None, shift: false, ctrl: false, alt: false,
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
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error().is_some_and(|error| error.contains("changed on disk")));
    assert!(app.document().unwrap().title.contains("Expert"));
    assert!(app.is_dirty());
    assert_eq!(fs::read_to_string(&expert).unwrap(), external);
    assert_eq!(fs::read_to_string(&hard).unwrap(), saved);
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
            high: 0.1,
            amp_l: 0.8,
            amp_r: 0.8,
        };
        4_000
    ];

    let start = Instant::now();
    let full = snapshot(&app, true);
    let full_bytes = serde_json::to_vec(&full).unwrap().len();
    let elapsed = start.elapsed();
    assert_eq!(full["scene"]["moments"].as_array().unwrap().len(), 20_000);
    assert!(full["scene"].get("waveform").is_none());
    assert_eq!(encode_waveform_bytes(&app.waveform).len(), 4_000 * 24);
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
    app.set_locator_left(1000.0);
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
    let row_height = app.row_height();
    send_key(&mut app, "ArrowDown", "ArrowDown", true, false);
    assert!(app.row_height() > row_height);
    app.set_locator_right(1200.0);
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
            button: None,
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
            button: None,
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
fn copy_review_uses_the_exact_requested_options() {
    use dawsu_foundation::SampleSet;
    use dawsu_hitsounding::copy::{CopyCollisionPolicy, CopyMode, CopyOptions};
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
        collision_policy: CopyCollisionPolicy::Merge,
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
        slider_end_mute: Default::default(),
        fallback_sound: None,
        addition_only_normal: Default::default(),
        silent_normal_index: None,
    });

    let mut app = Application::headless();
    open(&mut app, &source);
    action.run(&mut app);
    wait_for_job(&mut app);
    assert!(app.output_review().is_some());
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
    use crate::home::Route;

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
            value: None, button: None, shift: false, ctrl: false, alt: false,
        }.run(&mut app);
    }
    assert!(app.is_dirty());

    let send = |route: &mut Route, app: &mut Application, action| {
        assert!(route.handle(action, app).is_none());
        let mut view = snapshot_view(app, false);
        route.add_status(&mut view);
        serde_json::to_value(view).unwrap()
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
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    route.settle(&mut app);
    assert!(app.error().is_some());
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
fn save_applies_volume_floor_only_to_the_edited_hit() {
    let Some(root) = isolated_save_fixture("volume-floor",
        "tests::export_and_save::save_applies_volume_floor_only_to_the_edited_hit") else { return };
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
            value: None, button: None, shift: false, ctrl: false, alt: false,
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
fn home_save_with_left_out_moments_goes_home_after_the_summarys_save() {
    use crate::home::Route;

    let fixture = Fixture::new("home-left-out");
    let map = fixture.0.join("a.osu");
    fs::write(&map, "osu file format v14\r\n\r\n[General]\r\nMode: 0\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,8,2:0:0:0:\r\n256,192,1500,1,8,3:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n").unwrap();
    // A data folder of its own gives the difficulty an editor project to keep left-out moments.
    let mut app = Application::with_data_root(fixture.0.join("data"));
    app.clear_error();
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
                    value: None, button: None, shift: false, ctrl: false, alt: false,
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
fn only_closing_the_main_window_closes_the_editor() {
    use crate::{CloseDecision, Editor, Request, close_requested, closed_with_main};
    let (send, receive) = std::sync::mpsc::channel();
    let editor = Editor(send, dawsu_application::bounce_plan::BounceCancelToken::new());
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
    use crate::{Request, answer_export};
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
    let mut cache = crate::ExportCache::default();

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
    assert_eq!(view["preview"]["verification"]["passed"], true);
    assert_eq!(view["preview"]["verification"]["checkedFiles"], 1);
    assert_eq!(view["preview"]["verification"]["failedFiles"], 0);
    assert_eq!(counts["verification"]["passed"], true);
    let checks = counts["verification"]["checks"].as_array().expect("4 checks");
    let check_ids: Vec<&str> = checks.iter().filter_map(|c| c["id"].as_str()).collect();
    assert_eq!(check_ids, vec!["parse", "structure", "hitsounds", "samples"]);
    assert!(counts["byteIdentical"].is_boolean());
    assert!(
        counts["byteIdentical"] == true || counts["diff"]["sections"].is_array(),
        "expected byteIdentical or diff.sections: {counts}"
    );
    assert_eq!(view["choices"], serde_json::json!({
        "leftOut": [], "matchWithin": 5.0, "copyVolumes": true, "base": false, "sliderAudio": false, "laterWins": false, "mixFormat": "wav16",
        "sliderEndMute": { "enabled": false, "minBeats": 0.5, "divisors": [], "onlyUnhit": true, "muteSpinnerEnds": false },
        "fallbackSound": null,
        "autoFixStagedMixes": false,
        "writeMode": "direct",
        "singleFormat": "source",
        "additionOnlyNormal": "difficulty",
        "tickPolicy": "ignore",
        "allowSchemaGrowth": true,
        "autoCleanupStale": true,
        "takeOverSamples": false,
        "applyVolumeEnvelope": true,
    }));
    assert!(view["preview"]["base"].is_null(), "no base difficulty without the base choice");
    assert_eq!((&view["writing"], &view["written"], &view["writeError"]), (&serde_json::json!(false), &serde_json::Value::Null, &serde_json::Value::Null));
    assert!(view.get("review").is_none() && view.get("shortcutAction").is_none());
    assert!(app.output_review().is_none());

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
            value: Some(30.0), button: None, shift: false, ctrl: false, alt: false,
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
    assert!(app.error().is_none(), "{:?}", app.error());
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
fn the_export_view_reads_targets_and_hit_counts_once_per_edit() {
    use crate::{ExportCache, export_snapshot};
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
    use crate::export_title;
    let view = serde_json::json!({ "source": { "title": "yuikonnu - Kokuhaku", "difficulty": "Hard" } });
    assert_eq!(export_title(&view), "Export · yuikonnu - Kokuhaku [Hard]");
    // The editor's title already names the difficulty: it is not named twice.
    let named = serde_json::json!({ "source": { "title": "yuikonnu - Kokuhaku [Hard]", "difficulty": "Hard" } });
    assert_eq!(export_title(&named), "Export · yuikonnu - Kokuhaku [Hard]");
    assert_eq!(export_title(&serde_json::json!({ "source": null })), "Export");
}
