//! Drum maps (the drum-map plan, Tasks 2-5; hitsound domain design doc Responsibilities 17-20a,
//! invariants 9 and 10). Old test names, where a test follows one, are from the old app at
//! c81774e: `DrumMapTests.cs`, `GgdMapFileTests.cs`, `DrumKitMappingTests.cs`, `DrumRenderTests.cs`
//! and `DrumMapModelTests.cs`.

use dawsu_hitsounding::drums::*;

fn ids() -> impl FnMut() -> String {
    let mut n = 0u32;
    move || {
        n += 1;
        format!("00000000-0000-4000-8000-{n:012}")
    }
}

const ID: &str = "11111111-2222-4333-8444-555555555555";

fn row(id: u32, name: &str, lane: Option<&str>, note: Option<u8>) -> DrumRow {
    DrumRow { id: format!("aaaaaaaa-0000-4000-8000-{id:012}"), name: name.into(), lane: lane.map(String::from), slot: lane.map(String::from), midi_note: note }
}

fn map(rows: Vec<DrumRow>) -> DrumMap {
    DrumMap { id: ID.into(), name: "Kit".into(), rows, audio: vec![] }
}

fn refused<T: std::fmt::Debug>(r: Result<T, DrumFileRefusal>) -> DrumFileRefusal {
    r.expect_err("refused")
}

// ---------------------------------------------------------------- Task 2: the .dawsudrum file

#[test]
fn midi_assignments_round_trip_and_legacy_defaults_can_be_cleared() {
    let m = map(vec![row(1, "Kick", Some("Skin|drum-hitnormal"), None), row(2, "Snare", None, Some(40))]);
    let back = read_dawsudrum(&write_dawsudrum(&m).unwrap()).unwrap();
    assert_eq!(back, m, "a cleared note on a row named Kick stays cleared");

    // An old file with no MidiNote field: the row takes its General MIDI note.
    let legacy = format!(r#"{{"Id":"{ID}","Name":"Kit","Rows":[{{"Id":"{ID}","Name":"Kick","LaneKey":null,"SlotKey":null}},{{"Id":"aaaaaaaa-0000-4000-8000-000000000002","Name":"Cowbell","LaneKey":null,"SlotKey":null}}]}}"#);
    let read = read_dawsudrum(&legacy).unwrap();
    assert_eq!(read.rows.iter().map(|r| r.midi_note).collect::<Vec<_>>(), [Some(36), None]);
    assert!(read.audio.is_empty(), "a missing Audio reads as none");
}

#[test]
fn invalid_midi_notes_are_rejected() {
    for note in ["-1", "128"] {
        let text = format!(r#"{{"Id":"{ID}","Name":"Kit","Rows":[{{"Id":"{ID}","Name":"Kick","LaneKey":null,"SlotKey":null,"MidiNote":{note}}}]}}"#);
        refused(read_dawsudrum(&text));
    }
}

#[test]
fn portable_preset_round_trips_audio_and_aliases() {
    let mut m = map(vec![row(1, "Kick", Some("Skin|drum-hitnormal"), Some(36)), row(2, "Subkick", Some("Skin|drum-hitnormal"), Some(35))]);
    m.audio.push(DrumAudio { lane: "Skin|drum-hitnormal".into(), slot: "Skin|drum-hitnormal".into(), bytes: (0..=255u8).collect() });
    let text = write_dawsudrum(&m).unwrap();
    assert!(text.contains("\"Bytes\": \"AAECAwQF"), "base64 as System.Text.Json writes it");
    assert_eq!(read_dawsudrum(&text).unwrap(), m);
}

#[test]
fn invalid_assignments_are_rejected() {
    for key in ["BeatmapFile|../outside.wav", "BeatmapFile|C:/outside.wav", "StoryboardFile|boom.wav", "999|soft-hitclap.wav", "Skin|soft-sliderslide", "BeatmapFile|a/../../x.wav", "BeatmapFile|..\\x.wav", "BeatmapFile|/x.wav", "nobar"] {
        let m = map(vec![row(1, "Kick", Some(key), Some(36))]);
        assert!(write_dawsudrum(&m).is_err(), "{key}");
    }
    for key in ["Skin|drum-hitnormal", "BeatmapFile|sub/kick.wav", "BeatmapFile|drum-hitclap2.wav", "BeatmapFile|kick 2.ogg"] {
        let m = map(vec![row(1, "Kick", Some(key), Some(36))]);
        assert!(write_dawsudrum(&m).is_ok(), "{key}");
    }
}

#[test]
fn duplicate_row_ids_and_oversized_audio_are_rejected() {
    let mut m = map(vec![row(1, "Kick", None, None), row(1, "Snare", None, None)]);
    assert!(write_dawsudrum(&m).is_err(), "duplicate row id");
    m.rows.pop();
    m.audio.push(DrumAudio { lane: "Skin|drum-hitnormal".into(), slot: "Skin|drum-hitnormal".into(), bytes: vec![] });
    assert!(write_dawsudrum(&m).is_err(), "empty audio");
    m.audio[0].bytes = vec![0; 32 * 1024 * 1024 + 1];
    assert!(write_dawsudrum(&m).is_err(), "audio over 32 MiB");
}

#[test]
fn names_ids_and_text_are_checked_as_the_old_app_checks_them() {
    let nil = DrumMap { id: "00000000-0000-0000-0000-000000000000".into(), ..map(vec![]) };
    assert!(write_dawsudrum(&nil).is_err(), "the nil id");
    let not_guid = DrumMap { id: "kit-1".into(), ..map(vec![]) };
    assert!(write_dawsudrum(&not_guid).is_err(), "an id the old app cannot read");
    assert!(write_dawsudrum(&DrumMap { name: "  ".into(), ..map(vec![]) }).is_err(), "a blank name");
    // 128 UTF-16 units is the limit: 64 surrogate pairs pass, 65 do not.
    assert!(write_dawsudrum(&DrumMap { name: "𝄞".repeat(64), ..map(vec![]) }).is_ok());
    assert!(write_dawsudrum(&DrumMap { name: "𝄞".repeat(65), ..map(vec![]) }).is_err());

    // A byte order mark, escapes and non-ASCII names read.
    let text = format!("\u{feff}{{\"Id\":\"{ID}\",\"Name\":\"Kit \\u00e9\",\"Rows\":[{{\"Id\":\"{ID}\",\"Name\":\"Becken \\\"China\\\" ü\",\"LaneKey\":\"BeatmapFile|sub/x.wav\",\"SlotKey\":null,\"MidiNote\":null}}],\"Audio\":[]}}");
    let m = read_dawsudrum(&text).unwrap();
    assert_eq!((m.name.as_str(), m.rows[0].name.as_str(), m.rows[0].slot.as_deref()), ("Kit é", "Becken \"China\" ü", None));
    assert_eq!(read_dawsudrum(&write_dawsudrum(&m).unwrap()).unwrap(), m);

    refused(read_dawsudrum("not json"));
    refused(read_dawsudrum(&format!(r#"{{"Id":"{ID}","Name":"Kit","Rows":null}}"#)));
    refused(read_dawsudrum(&format!(r#"{{"Id":"{ID}","Name":"Kit","Rows":[],"Audio":null}}"#)));
}

#[test]
fn base64_round_trips_and_refuses_bad_text() {
    for len in 0..8usize {
        let bytes: Vec<u8> = (0..len as u8).map(|b| b.wrapping_mul(37)).collect();
        assert_eq!(base64_decode(&base64_encode(&bytes)), Some(bytes));
    }
    assert_eq!(base64_encode(b"Man"), "TWFu");
    assert_eq!(base64_encode(b"Ma"), "TWE=");
    for bad in ["TWE", "TW=u", "T@Fu", "TWFu="] {
        assert_eq!(base64_decode(bad), None, "{bad}");
    }
}

#[test]
fn every_starter_row_has_a_gm_note_with_articulation_fallbacks() {
    let maps = default_drum_maps(&mut ids());
    for m in &maps.maps {
        for r in &m.rows {
            assert!(r.midi_note.is_some(), "{}", r.name);
        }
    }
    assert_eq!(default_note("Ghost Notes"), Some(38));
    assert_eq!(default_note("Mini Stack Right"), Some(55));
    assert_eq!(default_note("Cowbell"), None);
}

#[test]
fn user_starter_kits_preserve_exact_names_order_and_unassigned_samples() {
    let maps = default_drum_maps(&mut ids());
    assert_eq!(maps.maps.iter().map(|m| (m.name.as_str(), m.rows.len())).collect::<Vec<_>>(), [("Yoge Expanded Kit", 25), ("Yoge Simple Kit", 18)]);
    assert_eq!(maps.active.as_deref(), Some(maps.maps[0].id.as_str()));
    assert!(!maps.enabled);
    assert_eq!(maps.maps[1].rows.iter().map(|r| r.name.as_str()).take(3).collect::<Vec<_>>(), ["Kick", "Snare", "T1"]);
    assert!(maps.maps.iter().flat_map(|m| &m.rows).all(|r| r.lane.is_none() && r.slot.is_none()));
    maps.validate().unwrap();
}

#[test]
fn a_mapset_s_drum_maps_are_checked_together() {
    let mut maps = default_drum_maps(&mut ids());
    maps.active = Some("aaaaaaaa-0000-4000-8000-999999999999".into());
    assert!(maps.validate().is_err(), "active names no map");
    let mut maps = default_drum_maps(&mut ids());
    let again = maps.maps[0].clone();
    maps.maps.push(again);
    assert!(maps.validate().is_err(), "a map id twice");
}

/// The local drum corpus (Yoge's files; not committed): set `DAWSU_DRUM_CORPUS` to its folder.
/// Its acceptance run needs the variable and checks every file.
#[test]
fn the_local_corpus_dawsudrum_files_read_and_round_trip() {
    let Some(files) = corpus("dawsudrum") else { return };
    assert_eq!(files.len(), 10, "the corpus's .dawsudrum files");
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let started = std::time::Instant::now();
        let m = read_dawsudrum(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let took = started.elapsed();
        let written = write_dawsudrum(&m).unwrap();
        assert_eq!(read_dawsudrum(&written).unwrap(), m, "{}: invariant 10", path.display());
        // Compared as JSON values: equal, except the MidiNote a legacy row gains.
        let (mut original, mut ours): (serde_json::Value, serde_json::Value) = (serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap(), serde_json::from_str(&written).unwrap());
        let mut defaulted = 0;
        for (o, w) in original["Rows"].as_array_mut().unwrap().iter_mut().zip(ours["Rows"].as_array_mut().unwrap()) {
            if o.get("MidiNote").is_none() {
                defaulted += 1;
                w.as_object_mut().unwrap().remove("MidiNote");
            }
        }
        assert_eq!(original, ours, "{}", path.display());
        eprintln!("{}: {} rows ({defaulted} took a default note), {} audio, read in {took:?}", path.file_name().unwrap().to_string_lossy(), m.rows.len(), m.audio.len());
    }
}

// ---------------------------------------------------------------- Task 3: rows from a kit

/// GGD P4 Kit, mapping preset "Invasion", exported with EXPORT MAP (old `GgdMapFileTests`).
fn p4_invasion() -> String {
    "%ART__articulation_map\n-1\n24\n30\n27\n26\n28\n29\n33\n34\n35\n36\n-1\n44\n42\n45\n46\n47\n48\n43\n41\n17\n61\n62\n53\n52\n57\n56\n55\n54\n66\n65\n74\n73\n76\n75\n77\n78\n79\n".to_string() + &"-1\n".repeat(90)
}

#[test]
fn an_export_reads_as_the_notes_its_mappings_page_shows() {
    let kit: std::collections::HashMap<String, Option<u8>> = ggd_kit(&p4_invasion()).unwrap().iter().map(|a| (label(a), a.note)).collect();
    for (name, note) in [
        ("Kick", 24), ("Snare", 26), ("Flam", 27), ("Wires Off", 29), ("Ruff", 28), ("Sidestick", 30), ("Rack Tom 1", 33), ("Floor Tom 2", 36), ("Stick Click", 79),
        ("Tip Tight", 41), ("Edge Tight", 42), ("Tip Closed", 43), ("Edge Closed", 44), ("Open 3", 47), ("Pedal", 48), ("CC", 17), ("Bell", 61), ("Ride", 62),
        ("China", 65), ("China Choke", 66), ("Main Crash L", 52), ("Main Crash L Choke", 53), ("Wide Crash L", 56), ("Main Crash R", 54), ("Main Crash R Choke", 55),
        ("Splash L", 73), ("Splash C", 75), ("Splash C Choke", 76), ("Mini Hats", 77), ("Stack", 78),
    ] {
        assert_eq!(kit[name], Some(note), "{name}");
    }
}

#[test]
fn an_exported_map_fills_the_starter_drum_map() {
    let rows = kit_rows(&ggd_kit(&p4_invasion()).unwrap(), false);
    for (row, note) in [("Kick", 24), ("Closed Edge", 44), ("Left Crash", 52), ("Stack", 78)] {
        assert_eq!(match_row(row, &rows).map(|k| k.note), Some(note), "{row}");
    }
}

#[test]
fn other_files_are_refused() {
    for text in ["%ArticulationMap\n-1\n24\n", "hello"] {
        refused(ggd_kit(text));
    }
    refused(ggd_kit(&p4_invasion().replace("\n24\n", "\n128\n")));
    refused(ggd_kit(&p4_invasion().replace("\n24\n", "\nC0\n")));
    refused(ggd_kit(&("%ART__articulation_map\n".to_string() + &"-1\n".repeat(70_000))));
}

#[test]
fn a_map_too_short_for_every_articulation_is_refused_not_misread() {
    let text = "%ART__articulation_map\n".to_string() + &"24\n".repeat(37);
    refused(ggd_kit(&text));
    assert!(!ggd_kit(&(text + "24\n")).unwrap().is_empty());
    // Blank lines and CRLF are skipped, as the old reader skips them.
    assert!(ggd_kit(&p4_invasion().replace('\n', "\r\n\r\n")).is_ok());
}

#[test]
fn hits_are_named_after_their_drum_and_other_articulations_keep_their_own_name() {
    for (group, item, name) in [
        ("KICK", "Hit", "Kick"), ("SNARE", "Hit", "Snare"), ("SNARE", "Flam", "Flam"), ("SNARE", "Wires Off", "Wires Off"), ("TOMS", "Rack Tom 1: Hit", "Rack Tom 1"),
        ("CRASHES", "Main Crash L: Choke", "Main Crash L Choke"), ("CHINA", "Choke", "China Choke"), ("MINI HATS", "Hit", "Mini Hats"), ("RIDE", "Bell", "Bell"),
    ] {
        assert_eq!(label(&Articulation { group: group.into(), item: item.into(), note: Some(1) }), name);
    }
}

#[test]
fn the_ggd_matt_garstka_kit_maps_its_mapped_notes_and_leaves_chokes_out() {
    let rows = kit_rows(&ggd_matt_garstka(), false);
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["Kick", "Snare", "Flam", "Wires Off", "Ruff", "Sidestick", "Rack Tom 1", "Rack Tom 2", "Floor Tom 1", "Floor Tom 2", "Tip Tight", "Edge Tight", "Tip Closed", "Open 1", "Open 2", "Open 3", "Pedal", "Bell", "Ride", "China", "Main Crash L", "Main Crash R", "Splash L"]
    );
    assert_eq!(rows[0].note, 36);
    assert_eq!(rows.iter().find(|r| r.name == "Snare").unwrap().note, 37);
    assert_eq!(rows.iter().find(|r| r.name == "Bell").unwrap().note, 74);
}

#[test]
fn chokes_are_added_when_asked_for() {
    let rows = kit_rows(&ggd_matt_garstka(), true);
    assert_eq!(rows.iter().filter(|r| r.name.ends_with("Choke")).map(|r| r.name.as_str()).collect::<Vec<_>>(), ["China Choke", "Main Crash L Choke", "Main Crash R Choke", "Splash L Choke"]);
    assert_eq!(rows.iter().find(|r| r.name == "Main Crash L Choke").unwrap().note, 59);
}

#[test]
fn each_kind_of_drum_goes_to_the_usual_osu_sound() {
    use dawsu_foundation::Hitsound::*;
    let rows = kit_rows(&ggd_matt_garstka(), false);
    let sound = |n: &str| rows.iter().find(|r| r.name == n).unwrap().sound;
    for (name, want) in [("Kick", Normal), ("Floor Tom 2", Normal), ("Snare", Clap), ("Sidestick", Clap), ("Tip Tight", Whistle), ("Pedal", Whistle), ("Bell", Whistle), ("China", Finish), ("Splash L", Finish)] {
        assert_eq!(sound(name), want, "{name}");
    }
}

#[test]
fn the_starter_drum_map_rows_find_their_kit_instrument() {
    let rows = kit_rows(&ggd_matt_garstka(), false);
    for (row, kit) in [
        ("Kick", "Kick"), ("Snare", "Snare"), ("Ghost Notes", "Snare"), ("T1", "Rack Tom 1"), ("T4", "Floor Tom 2"), ("Tight Tip", "Tip Tight"), ("Tight Edge", "Edge Tight"), ("Closed Tip", "Tip Closed"),
        ("Hi-Hat Open 2", "Open 2"), ("Ride Bell", "Bell"), ("Bow Ride (Low)", "Ride"), ("Right Crash", "Main Crash R"), ("Left Crash", "Main Crash L"), ("China", "China"), ("Splash", "Splash L"),
    ] {
        assert_eq!(match_row(row, &rows).map(|k| k.name.as_str()), Some(kit), "{row}");
    }
}

#[test]
fn rows_the_kit_has_nothing_for_stay_unmatched() {
    let rows = kit_rows(&ggd_matt_garstka(), false);
    for name in ["Closed Edge", "Stack", "Ride Crash", "Mini Stack Left"] {
        assert!(match_row(name, &rows).is_none(), "{name}");
    }
}

#[test]
fn reported_note_names_become_rows_in_note_order_and_unknown_kits_have_no_table() {
    let names = std::collections::BTreeMap::from([(38, "Snare".to_string()), (36, "Kick".to_string()), (40, " ".to_string()), (200, "Bad".to_string()), (-1, "Low".to_string())]);
    let rows = kit_rows(&kit_from_note_names(&names), false);
    assert_eq!(rows.iter().map(|r| (r.name.as_str(), r.note, r.sound)).collect::<Vec<_>>(), [("Kick", 36, dawsu_foundation::Hitsound::Normal), ("Snare", 38, dawsu_foundation::Hitsound::Clap)]);
}

#[test]
fn the_local_corpus_ggd_exports_read_or_are_refused_by_their_header() {
    let Some(files) = corpus("nka") else { return };
    assert_eq!(files.len(), 3, "the corpus's .nka files");
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let result = ggd_kit(&std::fs::read_to_string(&path).unwrap());
        if name == "martin_mapping.nka" {
            assert!(result.is_err(), "{name}: its header is %ArticulationMap");
        } else {
            let rows = kit_rows(&result.unwrap_or_else(|e| panic!("{name}: {e}")), true);
            eprintln!("{name}: {} rows with chokes", rows.len());
        }
    }
}
/// The corpus files with one extension, or `None` (skipped, with a note) when the variable is unset.
fn corpus(extension: &str) -> Option<Vec<std::path::PathBuf>> {
    let Ok(dir) = std::env::var("DAWSU_DRUM_CORPUS") else {
        eprintln!("skipped: DAWSU_DRUM_CORPUS is not set");
        return None;
    };
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(extension))).collect();
    files.sort();
    Some(files)
}
