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
