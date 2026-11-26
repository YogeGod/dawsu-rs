use super::helpers::{Fixture, open, wait_for_job};
use crate::{Action, snapshot, sound_set_library_view};
use dawsu_application::Application;
use std::fs;
use std::time::{Duration, Instant};

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
    let location = |path: &str| -> crate::library::Location {
        serde_json::from_value(serde_json::json!({ "root": library, "path": path })).unwrap()
    };
    assert_eq!(location("").entries(Some(&library)).unwrap(), [("drums".into(), true)]);
    assert_eq!(location("drums").entries(Some(&library)).unwrap(), [("snare.WAV".into(), false)]);
    assert!(location("drums").entries(Some(&mapset)).is_err());
    let path = mapset.join("Hard.osu");
    let original = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
    fs::write(&path, original).unwrap();
    let mut app = Application::headless();
    app.update_preferences(|p| p.hitsound_library = Some(library.clone()));
    open(&mut app, &path);
    let active = app.active_lane();
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
    assert!(app.error().is_none(), "{:?}", app.error());
    assert_eq!(app.active_lane(), active);
    for path in ["../outside.wav", "drums/text.txt"] {
        action("replace_library_sample", path, &target).run(&mut app);
        assert!(app.take_error().is_some());
    }
    let absolute = fixture.0.join("outside.wav");
    action("replace_library_sample", absolute.to_str().unwrap(), &target).run(&mut app);
    assert!(app.take_error().is_some());
    let wrong = serde_json::json!({"map": mapset.join("Other.osu"), "lane": target["lane"]});
    action("replace_library_sample", "drums/snare.WAV", &wrong).run(&mut app);
    assert!(app.take_error().is_some());
    assert_eq!(fs::read_dir(&mapset).unwrap().count(), 1);
    action("replace_library_sample", "drums/snare.WAV", &target).run(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    assert!(app.is_dirty());
    // The new backing sample waits in dawsu!'s data folder until a Save plays it.
    assert_eq!(fs::read_dir(&mapset).unwrap().count(), 1);
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read(library.join("drums/snare.WAV")).unwrap(), wave);
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
    app.clear_error();
    open(&mut app, &path);
    let map = snapshot(&app, true)["scene"]["path"].clone();
    let place = |app: &mut Application| serde_json::from_value::<Action>(serde_json::json!({
        "kind": "place_sound_set", "map": map, "path": file,
    })).unwrap().run(app);
    app.update_preferences(|p| p.hitsound_library = None);
    place(&mut app);
    assert!(app.take_error().is_some_and(|e| e.contains("hitsound library")));
    app.update_preferences(|p| p.hitsound_library = Some(fixture.0.join("library")));
    app.seek(2000.0);
    place(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    let hits = app.document().unwrap().composer.hitsounds();
    assert_eq!(hits.placements().len(), 1);
    // The set's copy waits in dawsu!'s data folder until a Save plays it.
    assert!(!fixture.0.join("soft-hitclap2.wav").is_file());
    assert!(fixture.0.join("library").join("Sound sets").join("Clap.dawsuset").is_file());
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
    app.clear_error();
    app.update_preferences(|p| p.hitsound_library = Some(fixture.0.join("library")));
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
    app.update_preferences(|p| p.hitsound_library = Some(fixture.0.join("gone")));
    assert!(sound_set_library_view(&app)["error"].as_str().is_some_and(|e| e.starts_with("Hitsound library folder not found")));
    app.update_preferences(|p| p.hitsound_library = Some(fixture.0.join("library")));
    let map = snapshot(&app, true)["scene"]["path"].clone();
    app.seek(2000.0);
    serde_json::from_value::<Action>(serde_json::json!({ "kind": "place_library_sound_set", "map": map, "id": set.id })).unwrap().run(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    assert_eq!(app.document().unwrap().composer.hitsounds().placements().len(), 1);
    assert_eq!(sound_set_library_view(&app)["sets"][0]["inMap"], true);
}

#[test]
fn the_sample_preview_gives_the_length_and_outline_of_a_library_sample_or_a_lane_sample() {
    let fixture = Fixture::new("sample-preview");
    let library = fixture.0.join("library");
    let mapset = fixture.0.join("mapset");
    fs::create_dir_all(library.join("drums")).unwrap();
    fs::create_dir(&mapset).unwrap();
    // 16-bit mono at 48 kHz: four frames, the second the loudest.
    let wave = b"RIFF\x2c\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x08\0\0\0\x00\x10\x00\x40\x00\x20\x00\x00";
    fs::write(library.join("drums/snare.wav"), wave).unwrap();
    fs::write(fixture.0.join("outside.wav"), wave).unwrap();
    let path = mapset.join("Hard.osu");
    fs::write(&path, b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n").unwrap();
    let mut app = Application::headless();
    app.update_preferences(|p| p.hitsound_library = Some(library.clone()));
    open(&mut app, &path);
    let source = |value: serde_json::Value| -> crate::PreviewSource { serde_json::from_value(value).unwrap() };
    let library_sample = |path: &str| source(serde_json::json!({ "kind": "library", "location": { "root": library, "path": path } }));
    let preview = crate::sample_outline(&mut app, library_sample("drums/snare.wav")).unwrap();
    assert!((preview["ms"].as_f64().unwrap() - 4000.0 / 48000.0).abs() < 1e-9, "{preview}");
    let peaks: Vec<f64> = serde_json::from_value(preview["peaks"].clone()).unwrap();
    assert_eq!(peaks.len(), crate::PREVIEW_BARS);
    assert_eq!(peaks.iter().copied().fold(0.0, f64::max), 1.0);
    assert!(crate::sample_outline(&mut app, library_sample("../outside.wav")).is_err());

    let active = app.active_lane();
    let target = serde_json::json!({
        "map": app.document().unwrap().path,
        "lane": format!("{:?}", app.document().unwrap().composer.lanes[active]),
    });
    let assign: Action = serde_json::from_value(serde_json::json!({
        "kind": "replace_library_sample", "location": { "root": library, "path": "drums/snare.wav" }, "target": target,
    })).unwrap();
    assign.run(&mut app);
    wait_for_job(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    let row = app.active_lane();
    let preview = crate::sample_outline(&mut app, source(serde_json::json!({ "kind": "lane", "row": row }))).unwrap();
    assert!((preview["ms"].as_f64().unwrap() - 4000.0 / 48000.0).abs() < 1e-9, "{preview}");
    assert!(crate::sample_outline(&mut app, source(serde_json::json!({ "kind": "lane", "row": 999 }))).is_err());
}

#[test]
fn scene_lane_exposes_vst_routed_and_bypassed_and_toggle_lane_switches_them() {
    struct NoopProcessor;
    impl dawsu_application::drum_vst::DrumBlockProcessor for NoopProcessor {
        fn process_block(
            &mut self,
            _sample_rate: f64,
            _events: &[dawsu_application::drum_vst::ShmEvent],
            left: &mut [f32],
            right: &mut [f32],
        ) -> bool {
            left.fill(0.0);
            right.fill(0.0);
            true
        }
    }

    let fixture = Fixture::new("scene-vst-routed");
    let map = fixture.0.join("map.osu");
    fs::write(
        &map,
        "osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let mut drums = crate::drum_view::DrumView::for_editor();

    // Before loading a Drum VST3, vstRouted/vstBypassed are omitted from SceneLane JSON.
    let without_vst = serde_json::to_value(crate::snapshot_view(&app, true)).unwrap();
    let lanes_before = without_vst["scene"]["lanes"].as_array().unwrap();
    assert!(lanes_before[0].get("vstRouted").is_none());
    assert!(lanes_before[0].get("vstBypassed").is_none());

    let (handle, _source) = dawsu_application::drum_vst::DrumVstLiveHandle::new(
        "vst-1".into(),
        "Test Kit".into(),
        "kit.vst3".into(),
        None,
        true,
        Box::new(NoopProcessor),
    );
    app.install_drum_vst_for_test(handle);
    app.set_drum_vst_routing(drums.routed_midi_notes(&app));

    let with_vst = serde_json::to_value(crate::snapshot_view(&app, true)).unwrap();
    let normal_row = with_vst["scene"]["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|lane| lane["name"] == "normal-hitnormal")
        .unwrap();
    assert_eq!(with_vst["scene"]["lanes"][normal_row]["vstRouted"], true);
    assert_eq!(with_vst["scene"]["lanes"][normal_row]["vstBypassed"], false);

    // Toggling the lane off sets vstRouted=false, vstBypassed=true.
    crate::handle_drum_vst_action(
        &mut app,
        &mut drums,
        &serde_json::json!({ "kind": "toggle_lane", "row": normal_row }),
    )
    .unwrap();
    let bypassed = serde_json::to_value(crate::snapshot_view(&app, true)).unwrap();
    assert_eq!(bypassed["scene"]["lanes"][normal_row]["vstRouted"], false);
    assert_eq!(bypassed["scene"]["lanes"][normal_row]["vstBypassed"], true);

    // Toggling it back on restores vstRouted=true, vstBypassed=false.
    crate::handle_drum_vst_action(
        &mut app,
        &mut drums,
        &serde_json::json!({ "kind": "toggle_lane", "row": normal_row }),
    )
    .unwrap();
    let restored = serde_json::to_value(crate::snapshot_view(&app, true)).unwrap();
    assert_eq!(restored["scene"]["lanes"][normal_row]["vstRouted"], true);
    assert_eq!(restored["scene"]["lanes"][normal_row]["vstBypassed"], false);
}
