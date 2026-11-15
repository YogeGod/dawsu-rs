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
// ---------------------------------------------------------------- Task 4: Cubase .drm

/// A `.drm` file's XML as Cubase writes it: one Map item per (I-Note, O-Note, name), then the
/// Order list when given.
fn drm(name: &str, items: &[(i32, i32, &str)], order: Option<&[i32]>) -> String {
    let mut x = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<DrumMap>\n   <string name=\"Name\" value=\"{name}\" wide=\"true\"/>\n   <list name=\"Quantize\" type=\"list\">\n      <item>\n         <int name=\"Grid\" value=\"4\"/>\n      </item>\n   </list>\n   <list name=\"Map\" type=\"list\">\n");
    for (i, o, n) in items {
        x += &format!("      <item>\n         <int name=\"INote\" value=\"{i}\"/>\n         <int name=\"ONote\" value=\"{o}\"/>\n         <int name=\"Channel\" value=\"9\"/>\n         <float name=\"Length\" value=\"200\"/>\n         <int name=\"DisplayNote\" value=\"{i}\"/>\n         <string name=\"Name\" value=\"{n}\" wide=\"true\"/>\n      </item>\n");
    }
    x += "   </list>\n";
    if let Some(order) = order {
        x += "   <list name=\"Order\" type=\"int\">\n";
        for o in order {
            x += &format!("      <item value=\"{o}\"/>\n");
        }
        x += "   </list>\n";
    }
    x + "</DrumMap>\n"
}

fn rows_of(m: &DrumMap) -> Vec<(String, Option<u8>)> {
    m.rows.iter().map(|r| (r.name.clone(), r.midi_note)).collect()
}

#[test]
fn a_drm_file_gives_its_named_rows_in_order_with_their_o_notes() {
    // Position 2 has I-Note 3: reading Order values as I-Notes would put Kick (I-Note 2) first.
    let items = [(0, 0, ""), (1, 1, "Sound 2"), (3, 40, "Snare "), (2, 36, "Kick"), (4, 4, "Sound 3"), (5, 50, "Snare")];
    let m = read_drm(&drm("My Kit", &items, Some(&[2, 3, 1, 0, 4, 5])), "file", &mut ids()).unwrap();
    assert_eq!(m.name, "My Kit");
    assert_eq!(rows_of(&m), [("Snare".into(), Some(40)), ("Kick".into(), Some(36)), ("Sound 3".into(), Some(4)), ("Snare".into(), Some(50))], "blank and placeholder rows dropped, names trimmed, a name twice kept");
    assert!(m.rows.iter().all(|r| r.lane.is_none() && r.slot.is_none()));
    m.validate().unwrap();
}

#[test]
fn a_drm_file_without_an_order_list_or_a_name_still_reads() {
    let m = read_drm(&drm("  ", &[(0, 0, "Kick"), (1, 38, "Snare")], None), "GGD Map", &mut ids()).unwrap();
    assert_eq!(m.name, "GGD Map", "a blank map name takes the file's name");
    assert_eq!(rows_of(&m), [("Kick".into(), Some(0)), ("Snare".into(), Some(38))], "the Map's own order");
}

#[test]
fn a_broken_drm_file_is_refused() {
    let items = [(0, 0, "Kick"), (1, 1, "Snare")];
    refused(read_drm(&drm("Kit", &items, Some(&[0, 128])), "f", &mut ids()));
    refused(read_drm(&drm("Kit", &items, Some(&[0, 0])), "f", &mut ids()));
    refused(read_drm(&drm("Kit", &items, Some(&[0, 5])), "f", &mut ids()));
    refused(read_drm(&drm("Kit", &[(0, 200, "Kick")], None), "f", &mut ids()));
    refused(read_drm("<KeyCommands/>", "f", &mut ids()));
    refused(read_drm("not xml", "f", &mut ids()));
    refused(read_drm("<!DOCTYPE d [<!ENTITY a \"aaaa\">]><DrumMap/>", "f", &mut ids()));
    // A dropped row may hold anything; only kept rows are checked.
    assert!(read_drm(&drm("Kit", &[(0, 200, ""), (1, 1, "Snare")], None), "f", &mut ids()).is_ok());
}

#[test]
fn the_local_corpus_drm_files_read() {
    let Some(files) = corpus("drm") else { return };
    assert_eq!(files.len(), 5, "the corpus's .drm files");
    for path in files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let m = read_drm(&std::fs::read_to_string(&path).unwrap(), &stem, &mut ids()).unwrap_or_else(|e| panic!("{name}: {e}"));
        if name == "p2-GGD Invasion Cubase Map.drm" {
            assert_eq!(m.rows.iter().take(6).map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Kick", "Snare", "Flam", "Wires Off", "Ruff", "Sidestick"]);
        }
        eprintln!("{name}: map \"{}\", {} rows kept: {}", m.name, m.rows.len(), m.rows.iter().take(8).map(|r| format!("{} {}", r.name, r.midi_note.unwrap())).collect::<Vec<_>>().join(", "));
    }
}
// ---------------------------------------------------------------- Task 5: view rows and the render plan

use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_hitsounding::{decompile, Decompiled, Hitsounds, Lane, LaneSource, MomentId};
use dawsu_osu_format::OsuFile;

fn dec(text: &[u8]) -> Hitsounds {
    match decompile(&OsuFile::read(text.to_vec()).file, &[], &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => panic!("{other:?}"),
    }
}

fn small_map() -> Hitsounds {
    dec(b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Drum\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,3,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n256,192,1500,1,8,0:0:0:0:\r\n256,192,2000,1,2,0:0:0:0:\r\n256,192,2500,1,4,0:0:0:0:\r\n")
}

fn skin(set: SampleSet, sound: Hitsound, index: u32) -> Lane {
    Lane { identity: SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(index) }, source: LaneSource::Skin }
}

/// Invariant 9: the view's distinct lanes are exactly the given lanes, unchanged; a lane rows
/// name shows once per row (aliases), a lane no row names shows once after the rows in its own
/// order, and each unassigned row shows once.
fn check_view(map: &DrumMap, lanes: &[Lane]) {
    let view = drum_rows(Some(map), lanes);
    let shown: Vec<&Lane> = view.iter().filter_map(|v| match v { DrumViewRow::Instrument { lane, .. } | DrumViewRow::Other(lane) => Some(*lane), DrumViewRow::Unassigned(_) => None }).collect();
    for lane in lanes {
        assert!(shown.iter().any(|l| *l == lane), "every lane stays reachable");
    }
    assert!(shown.iter().all(|l| lanes.contains(l)), "no lane is made up");
    let keys: Vec<String> = lanes.iter().map(lane_key).collect();
    for (r, v) in map.rows.iter().zip(view.iter()) {
        match v {
            DrumViewRow::Instrument { row, lane } => assert!(std::ptr::eq(*row, r) && r.lane.as_deref() == Some(lane_key(lane).as_str())),
            DrumViewRow::Unassigned(row) => assert!(std::ptr::eq(*row, r) && !r.lane.as_ref().is_some_and(|k| keys.contains(k))),
            DrumViewRow::Other(_) => panic!("rows come first, one each"),
        }
    }
    let others: Vec<&Lane> = view[map.rows.len()..].iter().map(|v| match v { DrumViewRow::Other(l) => *l, _ => panic!("only other lanes after the rows") }).collect();
    let unnamed: Vec<&Lane> = lanes.iter().filter(|l| !map.rows.iter().any(|r| r.lane.as_deref() == Some(lane_key(l).as_str()))).collect();
    assert_eq!(others, unnamed, "every other lane once, in its own order");
}

#[test]
fn missing_assignments_are_explicit_and_never_create_fake_note_lanes() {
    let h = small_map();
    let m = map(vec![row(1, "Kick", Some("Skin|drum-hitnormal"), Some(36)), row(2, "Boom", Some("BeatmapFile|boom.wav"), Some(35)), row(3, "Ghost Notes", None, Some(38))]);
    let view = drum_rows(Some(&m), h.lanes());
    assert!(matches!(view[0], DrumViewRow::Instrument { lane, .. } if *lane == skin(SampleSet::Drum, Hitsound::Normal, 0)));
    assert!(matches!(view[1], DrumViewRow::Unassigned(r) if r.name == "Boom"));
    assert!(matches!(view[2], DrumViewRow::Unassigned(r) if r.name == "Ghost Notes"));
    check_view(&m, h.lanes());
    assert_eq!(drum_rows(None, h.lanes()).len(), h.lanes().len(), "with no map, the lanes as they are");
}

#[test]
fn invariant_9_on_every_save_pack_map_and_the_corpus_drum_maps() {
    let mut maps = default_drum_maps(&mut ids()).maps;
    if let Some(files) = corpus("dawsudrum") {
        maps.extend(files.iter().map(|p| read_dawsudrum(&std::fs::read_to_string(p).unwrap()).unwrap()));
    }
    for (name, path) in dawsu_test_support::save_pack_files() {
        let file = OsuFile::read(std::fs::read(&path).unwrap()).file;
        let Decompiled::Done(h) = decompile(&file, &[], &|| false, &mut |_, _| {}) else { continue };
        let lanes = h.lanes();
        // A map that names every lane, the first one twice (an alias), plus an unassigned row.
        let mut every = map(lanes.iter().enumerate().map(|(i, l)| row(i as u32 + 10, &format!("Row {i}"), Some(&lane_key(l)), None)).collect());
        if let Some(first) = lanes.first() {
            every.rows.insert(1, row(1, "Alias", Some(&lane_key(first)), None));
        }
        every.rows.push(row(2, "Nothing", Some("BeatmapFile|nothing.wav"), None));
        for m in maps.iter().chain([&every]) {
            check_view(m, lanes);
        }
        let _ = name;
    }
}

#[test]
fn lane_keys_are_the_old_app_s_keys() {
    assert_eq!(lane_key(&skin(SampleSet::Soft, Hitsound::Clap, 0)), "Skin|soft-hitclap");
    assert_eq!(lane_key(&Lane { identity: SampleIdentity::File("kick.wav".into()), source: LaneSource::File("kick.wav".into()) }), "BeatmapFile|kick.wav");
    assert_eq!(lane_key(&Lane { identity: SampleIdentity::File("gone.wav".into()), source: LaneSource::Skin }), "BeatmapFile|gone.wav", "a named file missing from the folder");
    assert_eq!(lane_key(&skin(SampleSet::Drum, Hitsound::Clap, 2)), "Skin|drum-hitclap2", "a numbered file missing from the folder");
}

#[test]
fn render_notes_take_skin_fallbacks_the_first_row_per_lane_and_drop_unnoted_lanes() {
    let lanes = vec![
        skin(SampleSet::Drum, Hitsound::Normal, 0), skin(SampleSet::Drum, Hitsound::Clap, 0), skin(SampleSet::Drum, Hitsound::Whistle, 0), skin(SampleSet::Drum, Hitsound::Finish, 0),
        skin(SampleSet::Soft, Hitsound::Clap, 3),
        Lane { identity: SampleIdentity::File("gone.wav".into()), source: LaneSource::Skin },
        Lane { identity: SampleIdentity::File("kick.wav".into()), source: LaneSource::File("kick.wav".into()) },
    ];
    let notes = render_notes(None, &lanes);
    assert_eq!(notes.values().copied().collect::<Vec<_>>().len(), 5, "skin lanes with a standard sound only");
    assert_eq!(notes[&lanes[0].identity], 36);
    assert_eq!(notes[&lanes[1].identity], 38);
    assert_eq!(notes[&lanes[2].identity], 42);
    assert_eq!(notes[&lanes[3].identity], 49);
    assert_eq!(notes[&lanes[4].identity], 38, "a numbered missing file plays as the skin's sound");
    assert!(!notes.contains_key(&lanes[5].identity) && !notes.contains_key(&lanes[6].identity));

    let m = map(vec![row(1, "Kick", Some("Skin|drum-hitnormal"), Some(35)), row(2, "Subkick", Some("Skin|drum-hitnormal"), Some(24)), row(3, "Snare", Some("Skin|drum-hitclap"), None), row(4, "Boom", Some("BeatmapFile|kick.wav"), Some(41))]);
    let notes = render_notes(Some(&m), &lanes);
    assert_eq!(notes[&lanes[0].identity], 35, "the first row on a lane wins");
    assert!(!notes.contains_key(&lanes[1].identity), "a row with no note takes its lane out");
    assert_eq!(notes[&lanes[6].identity], 41);
}

#[test]
fn render_plan_reuses_note_velocity_and_preserves_unmapped_hits() {
    let mut h = small_map();
    for (i, v) in [5, 50, 70, 100].into_iter().enumerate() {
        h.set_volume(MomentId(i), v);
    }
    let m = map(vec![row(1, "Kick", Some("Skin|drum-hitnormal"), Some(36)), row(2, "Clap", Some("Skin|drum-hitclap"), None)]);
    let plan = drum_render_plan(&h, &render_notes(Some(&m), h.lanes()));
    let kicks: Vec<(f64, u8, u8)> = plan.iter().filter(|p| p.note == 36).map(|p| (p.time.ms(), p.note, p.velocity)).collect();
    assert_eq!(kicks, [(1000.0, 36, 6), (1500.0, 36, 64), (2000.0, 36, 89), (2500.0, 36, 127)], "velocity from the played volume");
    let clap = SampleIdentity::Standard { set: SampleSet::Drum, sound: Sound::Hit(Hitsound::Clap), index: CustomIndex::new(0) };
    assert!(h.moments()[1].has_hit(&clap) && plan.iter().all(|p| p.lane != clap), "a lane with no note is not rendered");
    assert!(plan.iter().any(|p| p.note == 42), "the whistle keeps its skin fallback");
    assert_eq!(render_keys(&plan), {
        let mut k: Vec<(u8, u8)> = plan.iter().map(|p| (p.note, p.velocity)).collect();
        k.sort();
        k.dedup();
        k
    });
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
