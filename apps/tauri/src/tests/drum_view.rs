use super::helpers::{Fixture, finish, open};
use crate::{Action, scene_lane_key, snapshot, snapshot_view};
use dawsu_application::Application;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
        choke_group: None,
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
    let mut drums = crate::drum_view::DrumView::default();
    open(&mut app, &path);
    let expected = app.document().unwrap().path.clone();
    let original = snapshot(&app, true);
    assert!(drums.load(&app, &sibling, &preset).is_err());
    drums.load(&app, &expected, &preset).unwrap();
    let read = |drums: &crate::drum_view::DrumView, app: &Application| {
        let mut scene = snapshot_view(app, true);
        drums.decorate(app, &mut scene);
        serde_json::to_value(scene).unwrap()
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
    let mut drums = crate::drum_view::DrumView::default();
    let load = |drums: &mut crate::drum_view::DrumView, app: &Application, rows: Vec<(&str, Option<String>)>| {
        let kit = DrumMap {
            id: "bbbbbbbb-0000-4000-8000-000000000002".into(),
            name: "Kit".into(),
            rows: rows.into_iter().enumerate().map(|(id, (name, lane))| DrumRow {
                id: format!("aaaaaaaa-0000-4000-8000-{id:012}"), name: name.into(), lane, slot: None, midi_note: None, choke_group: None,
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

    let lane_idx = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &clap).unwrap();
    app.set_active_lane(lane_idx);
    app.replace_lane_sample(&map, &clap, &external).unwrap();
    let replacement = app.document().unwrap().composer.lanes[app.active_lane()].clone();
    let revision = app.document().unwrap().composer.history.version();
    app.delete_lane(&map, &whistle, revision).unwrap();

    let plain = snapshot(&app, true);
    let order = plain["scene"]["laneOrder"].as_array().unwrap().clone();
    let read = |drums: &crate::drum_view::DrumView| {
        let mut shown = snapshot_view(&app, true);
        drums.decorate(&app, &mut shown);
        let shown = serde_json::to_value(shown).unwrap();
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
fn drum_view_rows_find_a_sample_under_another_spelling_and_keep_hidden_ones_hidden() {
    use dawsu_hitsounding::drums::{DrumMap, DrumRow, write_dawsudrum};

    let fixture = Fixture::new("drum-view-spelling");
    let path = fixture.0.join("Hard.osu");
    // Soft custom index 1: a clap and a whistle, from a pack saved with capital letters.
    let bytes = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,2,1,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n256,192,2000,1,2,0:0:0:0:\n";
    fs::write(&path, bytes).unwrap();
    let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    for name in ["Soft-HitClap.WAV", "Soft-HitWhistle.WAV"] {
        fs::write(fixture.0.join(name), wave).unwrap();
    }
    // A drum map written for the pack saved as .ogg files.
    let kit = DrumMap {
        id: "bbbbbbbb-0000-4000-8000-000000000003".into(),
        name: "Pack".into(),
        rows: [("Snare", "BeatmapFile|soft-hitclap.ogg"), ("Melody", "BeatmapFile|soft-hitwhistle.ogg")]
            .into_iter()
            .enumerate()
            .map(|(id, (name, lane))| DrumRow {
                id: format!("aaaaaaaa-0000-4000-8000-{id:012}"), name: name.into(), lane: Some(lane.into()), slot: None, midi_note: None, choke_group: None,
            })
            .collect(),
        audio: Vec::new(),
    };
    let preset = fixture.0.join("Pack.dawsudrum");
    fs::write(&preset, write_dawsudrum(&kit).unwrap()).unwrap();

    let mut app = Application::headless();
    open(&mut app, &path);
    let map = app.document().unwrap().path.clone();
    let mut drums = crate::drum_view::DrumView::default();
    drums.load(&app, &map, &preset).unwrap();
    let read = |drums: &crate::drum_view::DrumView, app: &Application| {
        let mut shown = snapshot_view(app, true);
        drums.decorate(app, &mut shown);
        serde_json::to_value(shown).unwrap()
    };
    let shown = read(&drums, &app);
    let rows = shown["scene"]["drumRows"].as_array().unwrap();
    for (name, file) in [("Snare", "Soft-HitClap.WAV"), ("Melody", "Soft-HitWhistle.WAV")] {
        let row = rows.iter().find(|row| row["name"] == name).and_then(|row| row["row"].as_u64());
        let row = row.unwrap_or_else(|| panic!("{name} shows no lane: {rows:?}"));
        assert_eq!(shown["scene"]["lanes"][row as usize]["sample"], file);
    }

    // Hiding the whistle's lane hides its row too, whatever the spelling.
    let lane = shown["scene"]["lanes"].as_array().unwrap().iter().position(|lane| lane["sample"] == "Soft-HitWhistle.WAV").unwrap();
    let whistle = app.document().unwrap().composer.lanes[lane].clone();
    let revision = app.document().unwrap().composer.history.version();
    app.delete_lane(&map, &whistle, revision).unwrap();
    let rows = read(&drums, &app)["scene"]["drumRows"].as_array().unwrap().clone();
    assert!(rows.iter().all(|row| row["name"] != "Melody"), "{rows:?}");
    assert!(rows.iter().any(|row| row["name"] == "Snare" && !row["row"].is_null()), "{rows:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn a_drum_view_row_on_a_hidden_lane_never_names_another_spelling_of_its_file() {
    use dawsu_hitsounding::drums::{DrumMap, DrumRow, write_dawsudrum};

    let fixture = Fixture::new("drum-view-hidden-spelling");
    let path = fixture.0.join("Hard.osu");
    // Two hits name one kick saved twice, as kick.wav and kick.ogg.
    let bytes = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,2,1,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:kick.wav\n256,192,2000,1,0,0:0:0:0:kick.ogg\n";
    fs::write(&path, bytes).unwrap();
    let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    for name in ["kick.wav", "kick.ogg"] {
        fs::write(fixture.0.join(name), wave).unwrap();
    }
    let kit = DrumMap {
        id: "bbbbbbbb-0000-4000-8000-000000000004".into(),
        name: "Kick".into(),
        rows: vec![DrumRow {
            id: "aaaaaaaa-0000-4000-8000-000000000000".into(), name: "Kick".into(), lane: Some("BeatmapFile|kick.wav".into()), slot: None, midi_note: Some(36), choke_group: None,
        }],
        audio: Vec::new(),
    };
    let preset = fixture.0.join("Kick.dawsudrum");
    fs::write(&preset, write_dawsudrum(&kit).unwrap()).unwrap();

    let mut app = Application::headless();
    open(&mut app, &path);
    let map = app.document().unwrap().path.clone();
    let mut drums = crate::drum_view::DrumView::default();
    drums.load(&app, &map, &preset).unwrap();
    let read = |drums: &crate::drum_view::DrumView, app: &Application| {
        let mut shown = snapshot_view(app, true);
        drums.decorate(app, &mut shown);
        serde_json::to_value(shown).unwrap()
    };
    let lane = |shown: &serde_json::Value, file: &str| {
        let lanes = shown["scene"]["lanes"].as_array().unwrap();
        lanes.iter().position(|lane| lane["sample"] == file).unwrap_or_else(|| panic!("no lane plays {file}: {lanes:?}"))
    };
    let shown = read(&drums, &app);
    let rows = shown["scene"]["drumRows"].as_array().unwrap();
    let wav = lane(&shown, "kick.wav");
    assert!(rows.iter().any(|row| row["name"] == "Kick" && row["row"] == wav), "the row names its own file: {rows:?}");

    // Hiding kick.wav's lane hides the row; kick.ogg stays a lane with no name.
    let kick = app.document().unwrap().composer.lanes[wav].clone();
    let revision = app.document().unwrap().composer.history.version();
    app.delete_lane(&map, &kick, revision).unwrap();
    let shown = read(&drums, &app);
    let rows = shown["scene"]["drumRows"].as_array().unwrap();
    let ogg = lane(&shown, "kick.ogg");
    assert!(rows.iter().all(|row| row["name"] != "Kick"), "{rows:?}");
    assert!(rows.iter().any(|row| row["row"] == ogg && row["name"].is_null()), "{rows:?}");
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn drum_view_loads_builtin_presets_and_assigns_rows() {
    let fixture = Fixture::new("drum-view-builtin");
    let path = fixture.0.join("Hard.osu");
    let bytes = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
    fs::write(&path, bytes).unwrap();

    let mut app = Application::headless();
    open(&mut app, &path);
    let map = app.document().unwrap().path.clone();
    let mut drums = crate::drum_view::DrumView::default();

    assert!(drums.load_builtin(&app, &map, "Does not exist").is_err());
    drums.load_builtin(&app, &map, "Two kicks").unwrap();

    let read = |drums: &crate::drum_view::DrumView, app: &Application| {
        let mut shown = snapshot_view(app, true);
        drums.decorate(app, &mut shown);
        serde_json::to_value(shown).unwrap()
    };
    let shown = read(&drums, &app);
    assert_eq!(shown["drumView"]["enabled"], true);
    assert_eq!(shown["drumView"]["name"], "Two kicks");
    assert_eq!(shown["drumView"]["presets"].as_array().unwrap().len(), 11);
    let setup_rows = shown["drumView"]["rows"].as_array().unwrap();
    assert!(!setup_rows.is_empty());

    // Assign row 0 to Skin|normal-hitclap, then unassign it.
    drums.assign_row(&app, &map, 0, Some("Skin|normal-hitclap")).unwrap();
    let assigned = read(&drums, &app);
    assert_eq!(assigned["drumView"]["rows"][0]["laneKey"], "Skin|normal-hitclap");
    assert_eq!(assigned["drumView"]["rows"][0]["assigned"], true);

    drums.assign_row(&app, &map, 0, None).unwrap();
    let cleared = read(&drums, &app);
    assert!(cleared["drumView"]["rows"][0]["laneKey"].is_null());
    assert_eq!(cleared["drumView"]["rows"][0]["assigned"], false);

    assert!(drums.assign_row(&app, &map, 999, None).is_err());
    assert!(!app.is_dirty());
}

#[test]
fn drum_view_for_editor_binds_builtins_toggles_auto_detect_hides_unassigned_and_reorders_rows() {
    let fixture = Fixture::new("drum-view-editor");
    let path = fixture.0.join("Hard.osu");
    let map_bytes = b"osu file format v14\n\n[General]\nMode:0\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n";
    fs::write(&path, map_bytes).unwrap();

    // Skin-only difficulty: built-in presets should bind automatically onto Skin|* lanes.
    let mut app = Application::headless();
    open(&mut app, &path);
    let map = app.document().unwrap().path.clone();
    let mut drums = crate::drum_view::DrumView::for_editor();

    let read = |drums: &crate::drum_view::DrumView, app: &Application| {
        let mut shown = snapshot_view(app, true);
        drums.decorate(app, &mut shown);
        serde_json::to_value(shown).unwrap()
    };

    // Turning drum view on uses auto_detect by default and hides unassigned/Other rows by default.
    drums.toggle(&app, &map, true).unwrap();
    let auto_shown = read(&drums, &app);
    assert_eq!(auto_shown["drumView"]["enabled"], true);
    assert_eq!(auto_shown["drumView"]["autoDetect"], true);
    assert_eq!(auto_shown["drumView"]["hideUnassigned"], true);
    let auto_setup = auto_shown["drumView"]["rows"].as_array().unwrap();
    assert!(auto_setup.iter().any(|row| row["assigned"] == false), "{auto_setup:?}");
    let auto_rows = auto_shown["scene"]["drumRows"].as_array().unwrap();
    assert!(auto_rows.iter().all(|row| row["row"].is_number()), "{auto_rows:?}");

    // Turning hide_unassigned off reveals the unassigned template rows in scene.drumRows.
    drums.set_hide_unassigned(&app, &map, false).unwrap();
    let expanded = read(&drums, &app);
    assert_eq!(expanded["drumView"]["hideUnassigned"], false);
    let expanded_rows = expanded["scene"]["drumRows"].as_array().unwrap();
    assert!(expanded_rows.len() > auto_rows.len(), "{expanded_rows:?}");
    assert!(expanded_rows.iter().any(|row| row["row"].is_null()), "{expanded_rows:?}");

    // Turning auto_detect off falls back from auto-detected kit to the first built-in preset.
    drums.set_auto_detect(&app, &map, false).unwrap();
    let fallback = read(&drums, &app);
    assert_eq!(fallback["drumView"]["autoDetect"], false);
    assert_ne!(fallback["drumView"]["name"], auto_shown["drumView"]["name"]);

    // Loading the 12-sample built-in preset binds its standard rows to Skin|* lanes.
    drums.load_builtin(&app, &map, "Full kit, all 12 samples").unwrap();
    drums.set_hide_unassigned(&app, &map, true).unwrap();
    let full = read(&drums, &app);
    let setup_rows = full["drumView"]["rows"].as_array().unwrap();
    assert_eq!(setup_rows.len(), 12);
    assert!(setup_rows.iter().all(|row| row["assigned"] == true), "{setup_rows:?}");
    let shown_rows = full["scene"]["drumRows"].as_array().unwrap();
    assert_eq!(shown_rows.len(), 12, "{shown_rows:?}");
    let first_before = setup_rows[0]["name"].as_str().unwrap().to_owned();
    let second_before = setup_rows[1]["name"].as_str().unwrap().to_owned();

    // Moving row 0 to index 1 reorders both drumView.rows and scene.drumRows.
    drums.move_row(&app, &map, 0, 1).unwrap();
    let moved = read(&drums, &app);
    assert_eq!(moved["drumView"]["rows"][0]["name"], second_before);
    assert_eq!(moved["drumView"]["rows"][1]["name"], first_before);
    assert_eq!(moved["scene"]["drumRows"][0]["name"], second_before);
    assert_eq!(moved["scene"]["drumRows"][1]["name"], first_before);
    assert!(drums.move_row(&app, &map, 0, 999).is_err());
    assert!(!app.is_dirty());
}

#[test]
fn drum_window_requests_answer_with_its_view_and_never_touch_main_errors() {
    use crate::drum_window::{DrumRequest, OptionsJson, SourceJson, answer};
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
    assert!(app.error().is_none(), "drum requests answer the drum window, not main");

    // A drum stem (the detector's test pattern at half level): the rows come once per review, and
    // Apply adds the checked ones.
    let pattern = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        .join("../../crates/dawsu-graph-audio/tests/fixtures/classic-3.1.4/pattern.f32");
    let pcm: Vec<u8> = fs::read(pattern).unwrap().as_chunks::<4>().0.iter()
        .flat_map(|b| (((f32::from_le_bytes(*b) * 0.5).clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
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
    assert!(app.status().starts_with("Added 1 drum hits: 1 on objects"), "{}", app.status());
}

#[test]
fn drum_window_lanes_carry_the_numbers_their_lane_headers_show() {
    use crate::drum_window::{DrumRequest, answer};
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
    // The header numbers: each row's place in the scene's lane order with layers under their main
    // lane, as app.js's orderedLaneRows shows them.
    let scene = &snapshot(&app, true)["scene"];
    let rows: Vec<usize> = scene["laneOrder"].as_array().unwrap().iter().map(|row| row.as_u64().unwrap() as usize).collect();
    let composer_lanes = app.document().unwrap().composer.lanes.clone();
    let shown = crate::family_order(rows, |row| &composer_lanes[row]);
    let headers: std::collections::HashMap<String, u64> = shown.iter().enumerate()
        .map(|(n, &row)| (scene["lanes"][row]["sourceKey"].as_str().unwrap().to_owned(), n as u64 + 1))
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
fn numbered_layers_sort_by_number_directly_under_their_main_lane() {
    use crate::family_order;
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
    use crate::drum_window::{DrumRequest, answer};
    let fixture = Fixture::new("layer-family-order");
    let map = fixture.0.join("map.osu");
    let tone = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
    fs::write(fixture.0.join("normal-hitfinish3.wav"), tone).unwrap();
    // A skin finish, a clap and a whistle, then a finish on custom index 3.
    fs::write(&map, "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,4,0:0:0:0:\n256,192,1000,1,8,0:0:0:0:\n256,192,1500,1,2,0:0:0:0:\n256,192,2000,1,4,1:0:3:0:\n").unwrap();
    let mut app = Application::headless();
    open(&mut app, &map);
    let scene = snapshot(&app, true)["scene"].clone();
    let numbers = |app: &mut Application| {
        let view = answer(app, DrumRequest::Context { known: None }).unwrap();
        let number = |name: &str| view["lanes"].as_array().unwrap().iter()
            .find(|lane| lane["name"] == name).and_then(|lane| lane["number"].as_u64()).unwrap();
        (number("normal-hitfinish"), number("normal-hitfinish3"))
    };
    // The drum window, as the headers, puts the layer directly under Finish.
    assert_eq!(numbers(&mut app), (4, 5));

    // The drum window numbers its lanes as the headers do, also after a saved order puts the layer first.
    let keys: Vec<String> = scene["laneOrder"].as_array().unwrap().iter().rev()
        .map(|row| scene["lanes"][row.as_u64().unwrap() as usize]["colourKey"].as_str().unwrap().to_owned()).collect();
    app.set_lane_layout(dawsu_application::LaneLayout { lane_order: keys, ..Default::default() }).unwrap();
    // Reversed, Finish keeps the place the saved order gives it (10th) and Finish 3 follows it.
    assert_eq!(numbers(&mut app), (10, 11));
}

#[test]
fn a_folded_family_survives_saving_and_reopening_the_difficulty() {
    let fixture = Fixture::new("layer-family-fold");
    let map = fixture.0.join("map.osu");
    fs::write(&map, "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,500,1,4,0:0:0:0:\n").unwrap();
    let mut app = Application::with_data_root(fixture.0.join("data"));
    app.clear_error();
    open(&mut app, &map);
    let path = app.document().unwrap().path.clone();
    serde_json::from_value::<Action>(serde_json::json!({
        "kind": "lane_layout", "path": path, "persist": true,
        "value": { "folder_order": ["normal", "soft", "drum"], "collapsed_folders": [],
            "collapsed_families": ["normal-hitfinish"], "lane_colours": {} },
    })).unwrap().run(&mut app);
    assert!(app.error().is_none(), "{:?}", app.error());
    open(&mut app, &map);
    let scene = snapshot(&app, true)["scene"].clone();
    assert_eq!(scene["laneLayout"]["collapsed_families"], serde_json::json!(["normal-hitfinish"]));
}

#[test]
fn toggle_step_hit_works_regardless_of_active_tool() {
    let fixture = Fixture::new("toggle-step-hit");
    let map = fixture.0.join("Map.osu");
    fs::write(
        &map,
        b"osu file format v14\n\n[Metadata]\nVersion:Hard\n\n[TimingPoints]\n0,500,4,2,0,80,1,0\n\n[HitObjects]\n256,192,0,1,0,0:0:0:0:\n",
    )
    .unwrap();

    let mut app = Application::headless();
    open(&mut app, &map);
    let clap_row = snapshot(&app, true)["scene"]["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|lane| lane["name"] == "soft-hitclap" && lane["index"] == 0)
        .unwrap();

    // Even with Select or Ramp tool active, toggle_step_hit adds and removes the hit cleanly.
    for tool in ["Select", "Ramp"] {
        Action::Tool { name: tool.into() }.run(&mut app);
        let add: Action = serde_json::from_value(serde_json::json!({
            "kind": "toggle_step_hit",
            "row": clap_row,
            "ms": 500.0
        }))
        .unwrap();
        add.run(&mut app);
        let after_add = snapshot(&app, true)["scene"]["moments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| (m[0].as_f64().unwrap() - 500.0).abs() < 1.0 && m[3].as_array().unwrap().iter().any(|h| h[0] == clap_row));
        assert!(after_add, "toggle_step_hit should add hit while {tool} tool is active");

        let remove: Action = serde_json::from_value(serde_json::json!({
            "kind": "toggle_step_hit",
            "row": clap_row,
            "ms": 500.0
        }))
        .unwrap();
        remove.run(&mut app);
        let after_remove = snapshot(&app, true)["scene"]["moments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| (m[0].as_f64().unwrap() - 500.0).abs() < 1.0 && m[3].as_array().unwrap().iter().any(|h| h[0] == clap_row));
        assert!(!after_remove, "toggle_step_hit should remove hit while {tool} tool is active");
    }
}

#[test]
fn task3_path2_auto_activates_default_osu_kit_shows_other_lanes_survives_replace_undo_and_never_bleeds() {
    let fixture = Fixture::new("task3-path2-drum-kit");
    let mapset = fixture.0.join("mapset");
    fs::create_dir_all(&mapset).unwrap();
    let insane = mapset.join("Artist - Song (Mapper) [Insane].osu");
    let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
    // Pre-existing normal-hitnormal.wav in mapset folder must not hijack Path 2's 12 Skin|* rows.
    fs::write(mapset.join("normal-hitnormal.wav"), wav).unwrap();
    fs::write(
        &insane,
        b"osu file format v14\n[General]\nMode: 0\n[Metadata]\nTitle:Song\nArtist:Artist\nCreator:Mapper\nVersion:Insane\n[TimingPoints]\n0,500,4,1,0,80,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
    )
    .unwrap();
    let ext_kick = fixture.0.join("custom-kick.wav");
    let ext_extra = fixture.0.join("extra-fx.wav");
    fs::write(&ext_kick, wav).unwrap();
    fs::write(&ext_extra, wav).unwrap();

    let mut app = Application::headless_with_projects(fixture.0.join("data"));
    let hs_path2 = app
        .create_hitsound_diff(&insane, false, Some(dawsu_application::HsStarterMode::Default12))
        .unwrap();

    let mut drums = crate::drum_view::DrumView::for_editor();
    let mut cache = crate::ViewTickCache::default();
    let read = |drums: &crate::drum_view::DrumView, app: &Application| {
        let mut shown = snapshot_view(app, true);
        drums.decorate(app, &mut shown);
        serde_json::to_value(shown).unwrap()
    };

    // Open Path 2 diff -> auto-activates Default osu! kit with all 12 Skin|* rows.
    super::helpers::open_raw(&mut app, &hs_path2);
    assert!(drums.sync_on_document_open(&mut app, &mut cache));
    let view = read(&drums, &app);
    assert_eq!(view["drumView"]["enabled"], true);
    assert_eq!(view["drumView"]["name"], crate::drum_view::DEFAULT_OSU_KIT_NAME);
    let drum_rows = view["scene"]["drumRows"].as_array().unwrap();
    assert_eq!(drum_rows.len(), 12, "All 12 Skin|* rows must be bound in Path 2");
    assert_eq!(drum_rows[0]["name"], "Drum Kick");
    assert!(drum_rows.iter().all(|r| r["row"].is_number()));

    // Toggle to raw folders and back to Drum Map cleanly.
    let doc_path = app.document().unwrap().path.clone();
    drums.toggle(&app, &doc_path, false).unwrap();
    assert_eq!(read(&drums, &app)["drumView"]["enabled"], false);
    drums.toggle(&app, &doc_path, true).unwrap();
    assert_eq!(read(&drums, &app)["drumView"]["enabled"], true);

    // Adding a custom lane (+ Add as lane) appears immediately below row 12 even with hide_unassigned=true.
    let extra_id = app.add_lane_from_file(&doc_path, &ext_extra).unwrap();
    let after_add = read(&drums, &app);
    let rows_after_add = after_add["scene"]["drumRows"].as_array().unwrap();
    assert_eq!(rows_after_add.len(), 13, "DrumViewRow::Other must be visible below row 12");
    assert!(rows_after_add[12]["drumRowIndex"].is_null());
    assert!(rows_after_add[12]["row"].is_number());
    let _ = extra_id;

    // Replacing a skin lane (Drum Kick = Skin|drum-hitnormal) stays bound across Replace AND Undo.
    let drum_normal = dawsu_foundation::SampleIdentity::Standard {
        set: dawsu_foundation::SampleSet::Drum,
        sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Normal),
        index: dawsu_foundation::CustomIndex::new(0),
    };
    let drum_row = app
        .document()
        .unwrap()
        .composer
        .lanes
        .iter()
        .position(|l| l == &drum_normal)
        .unwrap();
    app.set_active_lane(drum_row);
    app.replace_lane_sample(&doc_path, &drum_normal, &ext_kick).unwrap();
    let after_replace = read(&drums, &app);
    assert_eq!(after_replace["scene"]["drumRows"][0]["name"], "Drum Kick");
    assert!(after_replace["scene"]["drumRows"][0]["row"].is_number());

    app.command("undo");
    let after_undo = read(&drums, &app);
    assert_eq!(after_undo["scene"]["drumRows"][0]["name"], "Drum Kick");
    assert!(after_undo["scene"]["drumRows"][0]["row"].is_number());

    // Switching to sibling [Insane] diff must NOT inherit Path 2's auto-activated Default osu! kit.
    super::helpers::open_raw(&mut app, &insane);
    assert!(drums.sync_on_document_open(&mut app, &mut cache));
    let sibling_view = read(&drums, &app);
    assert!(
        sibling_view.get("drumView").is_none() || sibling_view["drumView"].is_null(),
        "Path 2 per-diff drum kit must not bleed into sibling gameplay diffs"
    );
}
