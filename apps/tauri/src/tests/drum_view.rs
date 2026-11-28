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

#[test]
fn drums_view_merges_identical_audio_lanes_in_autodetect_and_presets_and_routes_vst_notes() {
    use dawsu_hitsounding::drums::{DrumMap, DrumRow, write_dawsudrum};

    let make_wav = |frames: usize, mut synth: Box<dyn FnMut(f32) -> f32>| -> Vec<u8> {
        let sr = 44_100u32;
        let data_bytes = (frames * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        for field in [16u32, 1 | (1 << 16), sr, sr * 2, 2 | (16 << 16)] {
            out.extend_from_slice(&field.to_le_bytes());
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        for i in 0..frames {
            let s = synth(i as f32 / sr as f32).clamp(-1.0, 1.0);
            out.extend_from_slice(&((s * 30_000.0) as i16).to_le_bytes());
        }
        out
    };
    let mut kick_phase = 0.0f32;
    let kick_wave = make_wav(11_025, Box::new(move |t| {
        let f = 55.0 + 110.0 * (-t / 0.030).exp();
        kick_phase += 2.0 * std::f32::consts::PI * f / 44_100.0;
        kick_phase.sin() * (-t / 0.065).exp()
    }));
    let mut rng = 1u64;
    let snare_wave = make_wav(11_025, Box::new(move |t| {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let n = ((rng >> 32) as i32) as f32 / i32::MAX as f32;
        (2.0 * std::f32::consts::PI * 200.0 * t).sin() * (-t / 0.050).exp() * 0.5
            + n * (-t / 0.080).exp() * 0.45
    }));

    let fixture = Fixture::new("drum-view-merge-siblings");
    let map = fixture.0.join("Map.osu");
    for name in ["drum-hitnormal.wav", "drum-hitnormal2.wav", "drum-hitnormal3.wav"] {
        fs::write(fixture.0.join(name), &kick_wave).unwrap();
    }
    fs::write(fixture.0.join("normal-hitclap.wav"), &snare_wave).unwrap();
    fs::write(
        &map,
        b"osu file format v14\n\n[General]\nMode:0\nSampleSet:Drum\n\n[TimingPoints]\n0,500,4,3,1,100,1,0\n0,500,4,3,2,100,0,0\n0,500,4,3,3,100,0,0\n\n[HitObjects]\n256,192,500,1,0,0:0:0:0:\n256,192,1000,1,0,3:0:2:0:\n256,192,1500,1,0,3:0:3:0:\n256,192,2000,1,8,1:0:1:0:\n",
    )
    .unwrap();

    let mut app = Application::headless();
    open(&mut app, &map);
    let doc_path = app.document().unwrap().path.clone();
    let mut drums = crate::drum_view::DrumView::for_editor();
    let mut cache = crate::ViewTickCache::default();

    let row_of_key = |app: &Application, key: &str| {
        let model = app.document().unwrap().composer.hitsounds();
        app.document()
            .unwrap()
            .composer
            .lanes
            .iter()
            .position(|id| {
                model
                    .lane(id)
                    .is_some_and(|lane| dawsu_hitsounding::drums::lane_key(lane) == key)
            })
            .unwrap()
    };
    let k1 = row_of_key(&app, "BeatmapFile|drum-hitnormal.wav");
    let k2 = row_of_key(&app, "BeatmapFile|drum-hitnormal2.wav");
    let k3 = row_of_key(&app, "BeatmapFile|drum-hitnormal3.wav");

    drums.open_or_toggle_and_sync(&mut app, &doc_path, None, true).unwrap();
    let mut shown = snapshot_view(&app, true);
    drums.decorate(&app, &mut shown);
    let shown = serde_json::to_value(shown).unwrap();
    let drum_rows = shown["scene"]["drumRows"].as_array().unwrap();
    let kick_entries: Vec<_> = drum_rows
        .iter()
        .filter(|r| {
            r["rows"].as_array().is_some_and(|arr| {
                arr.iter()
                    .any(|v| v == k1 || v == k2 || v == k3)
            })
        })
        .collect();
    assert_eq!(kick_entries.len(), 1, "3 identical kicks must collapse to 1 row: {drum_rows:?}");
    assert!(
        kick_entries[0]["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("Kick")),
        "expected merged row to classify as Kick: {:?}",
        kick_entries[0]
    );
    let members: Vec<usize> = kick_entries[0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(members, vec![k1, k2, k3]);
    assert!(
        !drum_rows.iter().any(|r| r["row"] == k2 || r["row"] == k3),
        "secondary identical kicks must not also emit standalone Other rows: {drum_rows:?}"
    );
    let setup_kick = shown["drumView"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["laneKey"] == "BeatmapFile|drum-hitnormal.wav")
        .unwrap();
    assert_eq!(setup_kick["mergedLaneKeys"].as_array().unwrap().len(), 3);

    // VST routing applies Kick's MIDI note to every merged identical sibling lane.
    let routed = drums.routed_midi_notes(&app);
    let lanes = app.document().unwrap().composer.lanes.clone();
    for r in [k1, k2, k3] {
        assert_eq!(routed.get(&lanes[r]).copied(), setup_kick["midiNote"].as_u64().map(|v| v as u8));
    }

    // Active DrumRowGroups maps every member back to the primary group.
    let groups = app.drum_row_groups().expect("drum_row_groups synced");
    assert_eq!(groups.members_for_row(k2), Some(&[k1, k2, k3][..]));

    // Explicitly claiming two identical-audio lanes under two different preset rows preserves both claims.
    let kit = DrumMap {
        id: "bbbbbbbb-0000-4000-8000-000000000009".into(),
        name: "Explicit Two Kicks".into(),
        rows: vec![
            DrumRow {
                id: "aaaaaaaa-0000-4000-8000-000000000001".into(),
                name: "Kick Main".into(),
                lane: Some("BeatmapFile|drum-hitnormal.wav".into()),
                slot: None,
                midi_note: Some(36),
                choke_group: None,
            },
            DrumRow {
                id: "aaaaaaaa-0000-4000-8000-000000000002".into(),
                name: "Kick Accent".into(),
                lane: Some("BeatmapFile|drum-hitnormal2.wav".into()),
                slot: None,
                midi_note: Some(35),
                choke_group: None,
            },
        ],
        audio: Vec::new(),
    };
    let preset = fixture.0.join("ExplicitTwo.dawsudrum");
    fs::write(&preset, write_dawsudrum(&kit).unwrap()).unwrap();
    drums.open_or_toggle_and_sync(&mut app, &doc_path, Some(&preset), true).unwrap();
    let mut explicit_view = snapshot_view(&app, true);
    drums.decorate(&app, &mut explicit_view);
    let explicit_view = serde_json::to_value(explicit_view).unwrap();
    let explicit_rows = explicit_view["scene"]["drumRows"].as_array().unwrap();
    let main_row = explicit_rows.iter().find(|r| r["name"] == "Kick Main").unwrap();
    let accent_row = explicit_rows.iter().find(|r| r["name"] == "Kick Accent").unwrap();
    assert_eq!(main_row["rows"], serde_json::json!([k1, k3]), "unclaimed k3 folds into k1 without stealing k2");
    assert_eq!(accent_row["rows"], serde_json::json!([k2]), "explicit preset claim on k2 stays separate");
    let explicit_routed = drums.routed_midi_notes(&app);
    assert_eq!(explicit_routed.get(&lanes[k1]).copied(), Some(36));
    assert_eq!(explicit_routed.get(&lanes[k3]).copied(), Some(36));
    assert_eq!(explicit_routed.get(&lanes[k2]).copied(), Some(35));

    // Deleting a secondary member lane & undoing refreshes drum_row_groups via sync_on_document_open.
    let rev = app.document().unwrap().composer.history.version();
    app.delete_lane(&doc_path, &lanes[k3], rev).unwrap();
    assert!(drums.sync_on_document_open(&mut app, &mut cache));
    assert_eq!(app.drum_row_groups().unwrap().members_for_row(k1), Some(&[k1][..]));
    app.command("undo");
    assert!(drums.sync_on_document_open(&mut app, &mut cache));
    assert_eq!(app.drum_row_groups().unwrap().members_for_row(k3), Some(&[k1, k3][..]));
}

#[test]
fn task3_drums_view_merged_editing_selection_step_pads_vst_and_single_undo_replace() {
    use dawsu_application::PointerPhase;
    use dawsu_editor::tools::PointerInput;
    use dawsu_hitsounding::drums::lane_key;

    struct NoopProcessor;
    impl dawsu_application::drum_vst::DrumBlockProcessor for NoopProcessor {
        fn process_block(&mut self, _: f64, _: &[dawsu_application::drum_vst::ShmEvent], l: &mut [f32], r: &mut [f32]) -> bool {
            l.fill(0.0);
            r.fill(0.0);
            true
        }
    }

    let make_wav = |frames: usize, freq_mul: f32| -> Vec<u8> {
        let sr = 44_100u32;
        let data_bytes = (frames * 2) as u32;
        let mut out = Vec::with_capacity(44 + data_bytes as usize);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        for field in [16u32, 1 | (1 << 16), sr, sr * 2, 2 | (16 << 16)] {
            out.extend_from_slice(&field.to_le_bytes());
        }
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_bytes.to_le_bytes());
        let mut phase = 0.0f32;
        for i in 0..frames {
            let t = i as f32 / sr as f32;
            let f = 55.0 * freq_mul + 110.0 * (-t / 0.030).exp();
            phase += 2.0 * std::f32::consts::PI * f / sr as f32;
            let s = (phase.sin() * (-t / 0.065).exp()).clamp(-1.0, 1.0);
            out.extend_from_slice(&((s * 30_000.0) as i16).to_le_bytes());
        }
        out
    };

    let fixture = Fixture::new("task3-merged-editing");
    let map = fixture.0.join("Map.osu");
    let kick_wav = make_wav(8820, 1.0);
    let alt_kick_wav = make_wav(8820, 1.6);
    for name in ["drum-hitnormal.wav", "drum-hitnormal2.wav", "drum-hitnormal3.wav"] {
        fs::write(fixture.0.join(name), &kick_wav).unwrap();
    }
    let ext_kick = fixture.0.join("replaced-kick.wav");
    fs::write(&ext_kick, &alt_kick_wav).unwrap();
    fs::write(
        &map,
        b"osu file format v14\n\n[General]\nMode:0\nSampleSet:Drum\n\n[TimingPoints]\n0,500,4,3,1,100,1,0\n1500,-100,4,3,2,100,0,0\n\n[HitObjects]\n256,192,500,1,0,0:0:0:0:\n256,192,1000,1,0,3:0:3:0:\n256,192,2000,1,8,1:0:1:0:\n",
    )
    .unwrap();

    let mut app = Application::headless();
    open(&mut app, &map);
    let doc_path = app.document().unwrap().path.clone();
    let mut drums = crate::drum_view::DrumView::for_editor();
    let mut cache = crate::ViewTickCache::default();
    drums.open_or_toggle_and_sync(&mut app, &doc_path, None, true).unwrap();

    let row_of_key = |app: &Application, key: &str| {
        let model = app.document().unwrap().composer.hitsounds();
        app.document()
            .unwrap()
            .composer
            .lanes
            .iter()
            .position(|id| model.lane(id).is_some_and(|lane| lane_key(lane) == key))
            .unwrap()
    };
    let k1 = row_of_key(&app, "BeatmapFile|drum-hitnormal.wav");
    let k2 = row_of_key(&app, "BeatmapFile|drum-hitnormal2.wav");
    let k3 = row_of_key(&app, "BeatmapFile|drum-hitnormal3.wav");
    let c1 = row_of_key(&app, "Skin|normal-hitclap");

    // 1. Clicking primary row k1 at 1000 ms (where hit lives on k3!) selects k3's hit and moves it horizontally onto the active timing point's matching member (k1); Undo restores k3 at 1000 ms.
    let k1_id_initial = app.document().unwrap().composer.lanes[k1].clone();
    Action::Tool { name: "Select".into() }.run(&mut app);
    let ptr_at = |ms: f64, row: usize| {
        let mut p = PointerInput::at(ms, row as i32);
        p.x = ms;
        p.y = 12.0;
        p
    };
    app.pointer(PointerPhase::Down, ptr_at(1000.0, k1));
    assert_eq!(app.document().unwrap().composer.selected.items().len(), 1);
    let selected_id = app.document().unwrap().composer.lanes[k3].clone();
    assert_eq!(
        app.document()
            .unwrap()
            .composer
            .selected
            .items()
            .iter()
            .next()
            .unwrap()
            .lane,
        selected_id
    );

    app.pointer(PointerPhase::Move, ptr_at(1250.0, k1));
    app.pointer(PointerPhase::Up, ptr_at(1250.0, k1));
    let hits_after_move = app.document().unwrap().composer.hitsounds();
    assert!(
        hits_after_move
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1250.0).abs() < 1.0 && m.has_hit(&k1_id_initial))
            && !hits_after_move
                .moments()
                .iter()
                .any(|m| (m.time().ms() - 1000.0).abs() < 1.0 && m.has_hit(&selected_id)),
        "horizontal move into (Drum, 1) timing point must place onto matching member k1 and vacate 1000 ms"
    );
    app.command("undo");
    assert!(
        app.document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1000.0).abs() < 1.0 && m.has_hit(&selected_id)),
        "undo after horizontal move must restore k3 at 1000 ms"
    );

    // 2. Shift+Up/Down from an anchor sitting on secondary member k3 finds neighbor across drum rows.
    app.pointer(PointerPhase::Down, PointerInput::at(1000.0, k1 as i32));
    app.pointer(PointerPhase::Up, PointerInput::at(1000.0, k1 as i32));
    let mut visible_primaries: Vec<usize> = app
        .drum_row_groups()
        .unwrap()
        .groups
        .iter()
        .filter_map(|g| g.first().copied())
        .collect();
    if !visible_primaries.contains(&c1) {
        visible_primaries.push(c1);
    }
    let nav_down = dawsu_editor::keys::KeyChord::new(
        dawsu_editor::keys::Key::Down,
        dawsu_editor::keys::Mods::SHIFT,
    );
    app.key_with_lane_view(nav_down, doc_path.clone(), visible_primaries, 5000.0);
    assert!(
        app.document().unwrap().composer.selected.items().len() >= 2,
        "Shift+Down from secondary member k3 should select neighbor hit across drum rows"
    );

    // 3. Draw click at 1000 ms on primary k1 erases k3 hit instead of stacking a duplicate; Undo restores.
    Action::Tool { name: "Draw".into() }.run(&mut app);
    app.pointer(PointerPhase::Down, PointerInput::at(1000.0, k1 as i32));
    app.pointer(PointerPhase::Up, PointerInput::at(1000.0, k1 as i32));
    assert!(
        !app.document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1000.0).abs() < 1.0 && m.has_hit(&selected_id)),
        "Draw on occupied secondary hit should wipe it"
    );
    app.command("undo");

    // 4. Double-click empty spot at 1500 ms on k1 (timing point index 2 active!) picks k2 cleanly; step pad at 1500 ms removes it instead of stacking.
    Action::Tool { name: "Select".into() }.run(&mut app);
    let mut dbl = PointerInput::at(1500.0, k1 as i32);
    dbl.click_count = 2;
    app.pointer(PointerPhase::Down, dbl);
    let k2_id = app.document().unwrap().composer.lanes[k2].clone();
    assert!(
        app.document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1500.0).abs() < 1.0 && m.has_hit(&k2_id)),
        "Double-click at 1500 ms must place on k2 matching timing point index 2"
    );
    assert!(app.toggle_step_hit(k1, 1500.0));
    assert!(
        !app.document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1500.0).abs() < 1.0 && m.has_hit(&k2_id)),
        "Step pad on primary row k1 must remove co-timed k2 hit"
    );

    // 5. Action::Lane mute/solo, LaneGain, and toggle_drum_vst_lane apply uniformly across k1, k2, k3.
    Action::Lane { row: k1, operation: "mute".into() }.run(&mut app);
    for r in [k1, k2, k3] {
        let lane = app.document().unwrap().composer.lanes[r].clone();
        assert!(app.is_muted(&lane), "all merged members must mute uniformly");
    }
    Action::LaneGain { row: k1, gain: 0.65 }.run(&mut app);
    for r in [k1, k2, k3] {
        let doc = app.document().unwrap();
        let lane = &doc.composer.lanes[r];
        let plays_as = doc
            .composer
            .hitsounds()
            .lane(lane)
            .map(|m| m.plays_as())
            .unwrap_or_else(|| lane.clone());
        assert!((app.lane_gain(&plays_as) - 0.65).abs() < 1e-4);
    }
    let (handle, _src) = dawsu_application::drum_vst::DrumVstLiveHandle::new(
        "vst-m".into(),
        "Kit".into(),
        "kit.vst3".into(),
        None,
        true,
        Box::new(NoopProcessor),
    );
    app.install_drum_vst_for_test(handle);
    app.set_drum_vst_routing(drums.routed_midi_notes(&app));
    assert!(!app.toggle_drum_vst_lane(k1).unwrap());
    for r in [k1, k2, k3] {
        let lane = app.document().unwrap().composer.lanes[r].clone();
        assert!(app.drum_vst().unwrap().is_lane_vst_muted(&lane));
    }

    // 6. Single-undo Sample Replacement consolidates k1 + k2 + k3 cleanly with zero phantom 0-hit rows, and Undo restores all!
    let k1_id = app.document().unwrap().composer.lanes[k1].clone();
    app.set_active_lane(k1);
    app.replace_lane_sample(&doc_path, &k1_id, &ext_kick).unwrap();
    drums.sync_on_document_open(&mut app, &mut cache);

    let replacement = app.document().unwrap().composer.lanes[app.active_lane()].clone();
    let hits_after = app.document().unwrap().composer.hitsounds();
    assert!(hits_after.is_lane_hidden(&k2_id));
    assert!(hits_after.is_lane_hidden(&selected_id));
    assert!(
        hits_after
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 500.0).abs() < 1.0 && m.has_hit(&replacement)),
        "hit from primary k1 must move to replacement"
    );
    assert!(
        hits_after
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1000.0).abs() < 1.0 && m.has_hit(&replacement)),
        "hit from secondary k3 must move to replacement in same transaction"
    );
    let mut after_view = snapshot_view(&app, true);
    drums.decorate(&app, &mut after_view);
    let after_json = serde_json::to_value(after_view).unwrap();
    let drum_rows_after = after_json["scene"]["drumRows"].as_array().unwrap();
    assert!(
        !drum_rows_after.iter().any(|r| r["row"] == k2 || r["row"] == k3),
        "no phantom 0-hit rows for emptied secondaries: {drum_rows_after:?}"
    );

    // One single Undo restores k1, k2, k3 and every original hit cleanly!
    app.command("undo");
    drums.sync_on_document_open(&mut app, &mut cache);
    let hits_undone = app.document().unwrap().composer.hitsounds();
    assert!(!hits_undone.is_lane_hidden(&k2_id));
    assert!(!hits_undone.is_lane_hidden(&selected_id));
    assert!(
        hits_undone
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1000.0).abs() < 1.0 && m.has_hit(&selected_id)),
        "single Undo must restore secondary k3 hit at 1000 ms"
    );
    assert_eq!(app.drum_row_groups().unwrap().members_for_row(k1), Some(&[k1, k2, k3][..]));

    // 7. Action::Pointer with visible_rows restricts vertical drag to visible rows & Erase removes secondary hit.
    let ptr = |app: &mut Application, phase: &str, ms: f64, row: usize| {
        Action::Pointer {
            phase: phase.into(),
            ms,
            row: row as i32,
            x: ms,
            y: 12.0,
            value: None,
            volume_lanes: None,
            visible_rows: Some(vec![k1, c1]),
            button: None,
            shift: false,
            ctrl: false,
            alt: false,
        }
        .run(app);
    };
    let c1_member_ids: Vec<_> = {
        let doc = app.document().unwrap();
        app.drum_row_groups()
            .and_then(|g| g.members_for_row(c1))
            .map(|rows| rows.iter().map(|&r| doc.composer.lanes[r].clone()).collect())
            .unwrap_or_else(|| vec![doc.composer.lanes[c1].clone()])
    };
    assert!(!c1_member_ids.is_empty() && !c1_member_ids.contains(&selected_id));
    Action::Tool { name: "Select".into() }.run(&mut app);
    ptr(&mut app, "down", 1000.0, k1);
    ptr(&mut app, "move", 1000.0, c1);
    ptr(&mut app, "up", 1000.0, c1);
    let hits_moved = app.document().unwrap().composer.hitsounds();
    assert!(
        hits_moved
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1000.0).abs() < 1.0
                && c1_member_ids.iter().any(|id| m.has_hit(id))
                && !m.has_hit(&selected_id)),
        "filtered visible_rows [k1, c1] must move hit directly from merged k1(k3) onto c1's drum group"
    );
    app.command("undo");

    Action::Tool { name: "Erase".into() }.run(&mut app);
    ptr(&mut app, "down", 1000.0, k1);
    ptr(&mut app, "up", 1000.0, k1);
    assert!(
        !app.document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1000.0).abs() < 1.0 && m.has_hit(&selected_id)),
        "Erase on primary row k1 with visible_rows must remove secondary k3 hit"
    );
    app.command("undo");

    // 8. paint_hits_on_lane skips occupied secondary member (1000 ms on k3) & uses choose_member_lane (k2 at 1500 ms).
    let placed = app.paint_hits_on_lane(k1, 500.0, 2000.0);
    assert!(placed > 0);
    let hits_painted = app.document().unwrap().composer.hitsounds();
    let m1000 = hits_painted
        .moments()
        .iter()
        .find(|m| (m.time().ms() - 1000.0).abs() < 1.0)
        .unwrap();
    let kick_count_at_1000 = [k1_id.clone(), k2_id.clone(), selected_id.clone()]
        .iter()
        .filter(|id| m1000.has_hit(id))
        .count();
    assert_eq!(kick_count_at_1000, 1, "must not stack duplicate kick onto already-occupied 1000 ms");
    assert!(
        hits_painted
            .moments()
            .iter()
            .any(|m| (m.time().ms() - 1500.0).abs() < 1.0 && m.has_hit(&k2_id)),
        "paint_hits_on_lane at 1500 ms must resolve to k2 via timing point index 2"
    );
}

#[test]
#[ignore = "real osu! mapset verification against local Caligula's Horse - Mute"]
fn read_only_caligulas_horse_mute_merges_identical_lanes_and_sorts_canonically() {
    use dawsu_hitsounding::drums::lane_key;

    let src_dir = std::path::Path::new(r"F:\osu!\Songs\2218070 Caligula's Horse - Mute");
    let src_osu = src_dir.join("Caligula's Horse - Mute (ravensong) [Quiescence of Vicissitude].osu");
    if !src_osu.exists() {
        return;
    }

    let fixture = Fixture::new("caligula-mute-readonly");
    let mapset = fixture.0.join("mapset");
    fs::create_dir_all(&mapset).unwrap();
    let dst_osu = mapset.join(src_osu.file_name().unwrap());
    fs::copy(&src_osu, &dst_osu).unwrap();
    for entry in fs::read_dir(src_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.eq_ignore_ascii_case("audio.ogg") {
            continue;
        }
        if path.extension().is_some_and(|e| {
            e.eq_ignore_ascii_case("wav") || e.eq_ignore_ascii_case("ogg") || e.eq_ignore_ascii_case("mp3")
        }) {
            fs::copy(&path, mapset.join(name)).unwrap();
        }
    }

    let mut app = Application::headless_with_projects(fixture.0.join("data"));
    open(&mut app, &dst_osu);
    let doc_path = app.document().unwrap().path.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        app.poll();
        let all_files_loaded = {
            let doc = app.document().unwrap();
            let model = doc.composer.hitsounds();
            model.lanes().iter().all(|lane| {
                !matches!(lane.source, dawsu_hitsounding::LaneSource::File(_))
                    || app.lane_clip(&lane.identity).is_some()
            })
        };
        if all_files_loaded {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let raw_lane_count = app.document().unwrap().composer.lanes.len();
    let mut drums = crate::drum_view::DrumView::for_editor();
    drums.open_or_toggle_and_sync(&mut app, &doc_path, None, true).unwrap();
    let mut view = snapshot_view(&app, true);
    drums.decorate(&app, &mut view);
    let view_json = serde_json::to_value(view).unwrap();
    let drum_rows = view_json["scene"]["drumRows"].as_array().unwrap();

    let model = app.document().unwrap().composer.hitsounds();
    let composer_lanes = &app.document().unwrap().composer.lanes;
    let key_of = |idx: usize| lane_key(model.lane(&composer_lanes[idx]).unwrap());

    println!("Caligula raw lanes: {raw_lane_count}, drum rows: {}", drum_rows.len());
    for row in drum_rows {
        let keys: Vec<String> = row["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| key_of(v.as_u64().unwrap() as usize))
            .collect();
        println!(
            "  row={:?} name={:?} members={keys:?}",
            row["row"], row["name"]
        );
    }

    let drum_normal_rows: Vec<_> = drum_rows
        .iter()
        .filter(|r| {
            r["rows"].as_array().is_some_and(|arr| {
                arr.iter()
                    .map(|v| key_of(v.as_u64().unwrap() as usize))
                    .any(|k| k.starts_with("BeatmapFile|drum-hitnormal"))
            })
        })
        .collect();
    assert_eq!(
        drum_normal_rows.len(),
        1,
        "all 5 identical drum-hitnormal*.wav kicks must collapse into one single row: {drum_rows:?}"
    );
    assert!(
        drum_normal_rows[0]["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("Kick")),
        "merged drum-hitnormal*.wav row must classify as Kick: {:?}",
        drum_normal_rows[0]
    );
    assert_eq!(
        drum_normal_rows[0]["rows"].as_array().map(Vec::len),
        Some(5),
        "single merged Kick row must fold all 5 identical drum-hitnormal*.wav lanes: {:?}",
        drum_normal_rows[0]
    );
    let kick_keys: Vec<String> = drum_normal_rows[0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| key_of(v.as_u64().unwrap() as usize))
        .collect();
    for expected in [
        "BeatmapFile|drum-hitnormal.wav",
        "BeatmapFile|drum-hitnormal2.wav",
        "BeatmapFile|drum-hitnormal3.wav",
        "BeatmapFile|drum-hitnormal4.wav",
        "BeatmapFile|drum-hitnormal5.wav",
    ] {
        assert!(
            kick_keys.iter().any(|k| k == expected),
            "expected {expected} inside merged Kick row {kick_keys:?}"
        );
    }

    // Check canonical category ordering across named instrument rows.
    let category_rank = |name: &str| -> usize {
        if name.starts_with("Kick") { 0 }
        else if name.starts_with("Snare") { 1 }
        else if name.starts_with("Clap") { 2 }
        else if name.contains("Tom") { 3 }
        else if name.contains("Hat") { 4 }
        else if name.starts_with("Ride") { 5 }
        else if name.starts_with("Crash") { 6 }
        else { 7 }
    };
    let named_ranks: Vec<(usize, &str)> = drum_rows
        .iter()
        .filter_map(|r| r["name"].as_str().map(|n| (category_rank(n), n)))
        .filter(|(rank, _)| *rank < 7)
        .collect();
    for pair in named_ranks.windows(2) {
        assert!(
            pair[0].0 <= pair[1].0,
            "expected canonical kit order Kick -> Snare/Clap -> Toms -> Hi-Hats -> Ride/Crash, got {named_ranks:?}"
        );
    }

    // Raw Hitsounds view remains unmerged when Drums view is toggled off.
    drums.open_or_toggle_and_sync(&mut app, &doc_path, None, false).unwrap();
    let mut off_view = snapshot_view(&app, true);
    drums.decorate(&app, &mut off_view);
    let off_json = serde_json::to_value(off_view).unwrap();
    assert_eq!(off_json["drumView"]["enabled"], false);
    assert_eq!(off_json["scene"]["lanes"].as_array().unwrap().len(), raw_lane_count);
}

#[test]
fn drum_view_draw_and_vertical_drag_on_existing_object_preserve_sibling_volumes() {
    let fixture = Fixture::new("drum-per-hit-volumes");
    let map = fixture.0.join("Map.osu");
    fs::write(
        &map,
        b"osu file format v14\n\n[General]\nMode:0\nSampleSet:Normal\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,0,0:0:0:0:\n",
    )
    .unwrap();

    let mut app = Application::headless();
    open(&mut app, &map);
    let doc_path = app.document().unwrap().path.clone();
    let mut drums = crate::drum_view::DrumView::for_editor();
    drums.open_or_toggle_and_sync(&mut app, &doc_path, None, true).unwrap();

    let view_json = {
        let mut view = snapshot_view(&app, true);
        drums.decorate(&app, &mut view);
        serde_json::to_value(view).unwrap()
    };
    let drum_rows = view_json["scene"]["drumRows"].as_array().unwrap();
    let clap_entry = drum_rows
        .iter()
        .find(|r| r["name"].as_str().is_some_and(|n| n.starts_with("Clap")))
        .expect("Clap drum row");
    let kick_entry = drum_rows
        .iter()
        .find(|r| r["name"].as_str().is_some_and(|n| n.starts_with("Kick")))
        .expect("Kick drum row");
    let kick_row = kick_entry["row"].as_u64().unwrap() as usize;
    let clap_row = clap_entry["row"].as_u64().unwrap() as usize;
    let visible = Some(vec![kick_row, clap_row]);

    let clap_members: Vec<_> = {
        let doc = app.document().unwrap();
        app.drum_row_groups()
            .and_then(|g| g.members_for_row(clap_row))
            .map(|rows| rows.iter().map(|&r| doc.composer.lanes[r].clone()).collect())
            .unwrap_or_else(|| vec![doc.composer.lanes[clap_row].clone()])
    };
    let kick_lane = app.document().unwrap().composer.hitsounds().moments()[0]
        .hits()
        .next()
        .unwrap()
        .clone();

    let ptr = |app: &mut Application, phase: &str, ms: f64, y: f64| {
        Action::Pointer {
            phase: phase.into(),
            ms,
            row: clap_row as i32,
            x: 0.0,
            y,
            value: None,
            volume_lanes: None,
            visible_rows: visible.clone(),
            button: None,
            shift: false,
            ctrl: false,
            alt: false,
        }
        .run(app);
    };

    Action::Tool { name: "Draw".into() }.run(&mut app);
    Action::InsertVolume { value: 60 }.run(&mut app);
    ptr(&mut app, "down", 1000.0, 200.0);
    ptr(&mut app, "up", 1000.0, 200.0);

    Action::InsertVolume { value: 80 }.run(&mut app);
    ptr(&mut app, "down", 1500.0, 200.0);
    ptr(&mut app, "move", 1500.0, 280.0);
    ptr(&mut app, "up", 1500.0, 280.0);

    let hits = app.document().unwrap().composer.hitsounds();
    let m0 = &hits.moments()[0];
    let m1 = &hits.moments()[1];
    let clap0 = clap_members.iter().find(|l| m0.has_hit(l)).expect("clap at 1000 ms");
    let clap1 = clap_members.iter().find(|l| m1.has_hit(l)).expect("clap at 1500 ms");
    assert_eq!(m0.hit_volume(&kick_lane), 100);
    assert_eq!(m0.hit_volume(clap0), 60);
    assert_eq!(m1.hit_volume(&kick_lane), 100);
    assert!(m1.hit_volume(clap1) < 80, "vertical drag in Drum View should lower only the new hit: {}", m1.hit_volume(clap1));
}
