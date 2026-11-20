//! Sound sets and their snapshot in a mapset (hitsound domain design doc Responsibilities 15 and
//! 16, invariant 11; the last-four-parts plan, part D). Old cases are from the old app's
//! `SoundSetTests.cs` at c81774e.

use dawsu_hitsounding::sound_sets::*;
use std::collections::BTreeSet;

const ID: &str = "9e7b1d18-3186-4778-9a6a-b6baef95cbe7";

fn member(n: u32, source: SetSource, name: &str, slot: &str, volume: i32, audio: &[u8]) -> SetMember {
    SetMember {
        id: format!("aaaaaaaa-0000-4000-8000-{n:012}"),
        original_key: format!("BeatmapFile|{name}"),
        lane: SetLane { source, name: name.into(), missing: false, kept: Kept::default() },
        slot: slot.into(),
        volume,
        audio: audio.to_vec(),
        kept: Kept::default(),
    }
}

fn set(members: Vec<SetMember>) -> SoundSet {
    SoundSet { id: ID.into(), name: "Cymbal stack".into(), members, library_id: None, kept: Kept::default() }
}

fn snapshot(s: &SoundSet, folder: &[&str], used: &[u32], previous: Option<&SoundSet>, existing: &dyn Fn(&str) -> Option<Vec<u8>>) -> Snapshot {
    let folder: Vec<String> = folder.iter().map(|f| f.to_string()).collect();
    plan_snapshot(s, &folder, &used.iter().copied().collect::<BTreeSet<u32>>(), previous, existing).unwrap()
}

#[test]
fn a_sound_set_file_round_trips_and_writing_refuses_what_reading_would() {
    let s = set(vec![member(1, SetSource::File, "drum-hitnormal.wav", "drum-hitnormal", 20, b"kick"), member(2, SetSource::Skin, "soft-hitfinish", "soft-hitfinish", 100, b"crash")]);
    let text = write_dawsuset(&s).unwrap();
    assert!(text.contains("\"Source\": 1") && text.contains("\"Audio\": \"Y3Jhc2g=\""));
    assert_eq!(read_dawsuset(&text).unwrap(), s);
    for bad in [
        set(vec![]),
        set(vec![member(1, SetSource::File, "drum-hitnormal.wav", "soft-hitnormal", 20, b"k")]),
        set(vec![member(1, SetSource::File, "kick.wav", "drum-hitnormal", 20, b"k")]),
        set(vec![member(1, SetSource::File, "kick.wav", "normal-hitnormal", 101, b"k")]),
        set(vec![member(1, SetSource::File, "kick.wav", "normal-hitnormal", 20, b"")]),
        set(vec![member(1, SetSource::File, "../kick.wav", "normal-hitnormal", 20, b"k")]),
        set(vec![member(1, SetSource::Storyboard, "kick.wav", "normal-hitnormal", 20, b"k")]),
        set(vec![member(1, SetSource::File, "soft-sliderslide.wav", "soft-hitnormal", 20, b"k")]),
        set(vec![member(1, SetSource::File, "a.wav", "normal-hitnormal", 20, b"k"), member(1, SetSource::File, "b.wav", "normal-hitnormal", 20, b"k")]),
    ] {
        assert!(write_dawsuset(&bad).is_err(), "{bad:?}");
    }
    let mut missing = set(vec![member(1, SetSource::File, "kick.wav", "normal-hitnormal", 20, b"k")]);
    missing.members[0].lane.missing = true;
    assert!(write_dawsuset(&missing).is_err(), "a missing lane");
    assert!(read_dawsuset("not json").is_err());
}

#[test]
fn placing_a_set_plays_each_member_at_its_volume_times_the_gain() {
    let s = set(vec![member(1, SetSource::File, "drum-hitnormal.wav", "drum-hitnormal", 25, b"k"), member(2, SetSource::Skin, "soft-hitclap", "soft-hitclap", 80, b"c")]);
    assert_eq!(placement_hits(&s, 100), [("BeatmapFile|drum-hitnormal.wav".to_string(), 25), ("Skin|soft-hitclap".to_string(), 80)]);
    assert_eq!(placement_hits(&s, 50).iter().map(|h| h.1).collect::<Vec<_>>(), [12, 40], "12.5 rounds half to even");
    assert_eq!(placement_hits(&s, 200).iter().map(|h| h.1).collect::<Vec<_>>(), [50, 100], "clamped to 100");
}

#[test]
fn a_snapshot_names_each_sample_by_its_role_at_the_lowest_free_index() {
    let s = set(vec![member(1, SetSource::File, "drum-hitnormal.OGG", "drum-hitnormal", 20, b"kick"), member(2, SetSource::Skin, "soft-hitclap", "soft-hitclap", 80, b"clap")]);
    // Index 2 is taken by a file (any extension) and 3 by a difficulty, so the first free is 4,
    // and every member shares it: one object plays one custom index.
    let snap = snapshot(&s, &["soft-hitclap2.ogg", "map.osu"], &[3], None, &|_| None);
    assert_eq!(snap.files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["drum-hitnormal4.ogg", "soft-hitclap4.wav"]);
    assert_eq!(snap.files[0].1, b"kick");
    assert!(snap.set.members.iter().all(|m| m.lane.source == SetSource::File));
    assert_eq!(snap.set.members[1].lane.name, "soft-hitclap4.wav");
    assert_eq!(snap.set.members[1].original_key, "BeatmapFile|soft-hitclap", "the original key stays");
}

#[test]
fn invariant_11_a_set_placed_again_reuses_its_snapshot() {
    let s = set(vec![member(1, SetSource::File, "drum-hitnormal.wav", "drum-hitnormal", 20, b"kick")]);
    let first = snapshot(&s, &[], &[], None, &|_| None);
    let on_disk = |name: &str| (name == "drum-hitnormal2.wav").then(|| b"kick".to_vec());
    let again = snapshot(&first.set, &["drum-hitnormal2.wav"], &[], Some(&first.set), &on_disk);
    assert!(again.files.is_empty(), "no new file");
    assert_eq!(again.set, first.set);
    // Changed on disk: a new file, never the changed one.
    let changed = |name: &str| (name == "drum-hitnormal2.wav").then(|| b"other".to_vec());
    assert_eq!(snapshot(&first.set, &["drum-hitnormal2.wav"], &[], Some(&first.set), &changed).files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["drum-hitnormal3.wav"]);
}

#[test]
fn a_repeated_role_takes_the_next_free_index_after_the_shared_one() {
    let s = set(vec![
        member(1, SetSource::File, "drum-hitnormal.wav", "drum-hitnormal", 20, b"kick"),
        member(2, SetSource::File, "drum-hitclap.wav", "drum-hitclap", 50, b"clap"),
        member(3, SetSource::File, "drum-hitclap2.wav", "drum-hitclap", 50, b"clap2"),
    ]);
    let snap = snapshot(&s, &["normal-hitnormal3.wav"], &[], None, &|_| None);
    assert_eq!(snap.files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["drum-hitnormal2.wav", "drum-hitclap2.wav", "drum-hitclap4.wav"]);
}

#[test]
fn a_snapshot_is_reused_whole_or_copied_again_whole() {
    let s = set(vec![
        member(1, SetSource::File, "drum-hitnormal.wav", "drum-hitnormal", 20, b"kick"),
        member(2, SetSource::File, "drum-hitclap.wav", "drum-hitclap", 50, b"clap"),
    ]);
    let first = snapshot(&s, &[], &[], None, &|_| None);
    let folder = ["drum-hitnormal2.wav", "drum-hitclap2.wav"];
    // The library set, not the snapshot, is planned again: the record supplies the files.
    let both = |name: &str| match name { "drum-hitnormal2.wav" => Some(b"kick".to_vec()), "drum-hitclap2.wav" => Some(b"clap".to_vec()), _ => None };
    let again = snapshot(&s, &folder, &[], Some(&first.set), &both);
    assert!(again.files.is_empty());
    assert_eq!(again.set, first.set);
    // One member's file changed: the whole set is copied again at one new index.
    let one = |name: &str| (name == "drum-hitnormal2.wav").then(|| b"kick".to_vec());
    let fresh = snapshot(&s, &folder, &[], Some(&first.set), &one);
    assert_eq!(fresh.files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["drum-hitnormal3.wav", "drum-hitclap3.wav"]);
    // A member's audio changed in the library: copied again too.
    let mut edited = s.clone();
    edited.members[1].audio = b"clap!".to_vec();
    assert_eq!(snapshot(&edited, &folder, &[], Some(&first.set), &both).files.len(), 2);
}

#[test]
fn a_file_with_the_same_bytes_is_never_taken_as_the_snapshot() {
    let s = set(vec![member(1, SetSource::File, "drum-hitnormal.wav", "drum-hitnormal", 20, b"kick")]);
    let same = |_: &str| Some(b"kick".to_vec());
    let snap = snapshot(&s, &["drum-hitnormal2.wav"], &[], None, &same);
    assert_eq!(snap.files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), ["drum-hitnormal3.wav"]);
}

#[test]
fn an_unsupported_extension_is_refused() {
    let s = set(vec![member(1, SetSource::File, "kick.flac", "normal-hitnormal", 20, b"k")]);
    assert!(plan_snapshot(&s, &[], &BTreeSet::new(), None, &|_| None).is_err());
}

/// The local corpus (Yoge's files, not committed): `DAWSU_DRUM_CORPUS` holds the old app's
/// `.dawsuset` files as `set-*.dawsuset`. Each reads and writes back as equal JSON, no key ignored.
#[test]
fn the_local_corpus_sound_set_files_round_trip() {
    let Ok(dir) = std::env::var("DAWSU_DRUM_CORPUS") else {
        eprintln!("skipped: DAWSU_DRUM_CORPUS is not set");
        return;
    };
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "dawsuset")).collect();
    files.sort();
    assert_eq!(files.len(), 10, "the corpus's .dawsuset files");
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let s = read_dawsuset(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let written = write_dawsuset(&s).unwrap();
        let (a, b): (serde_json::Value, serde_json::Value) = (serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap(), serde_json::from_str(&written).unwrap());
        assert_eq!(a, b, "{}", path.display());
        assert_eq!(read_dawsuset(&written).unwrap(), s);
        eprintln!("{}: \"{}\", {} members", path.file_name().unwrap().to_string_lossy(), s.name, s.members.len());
    }
}
