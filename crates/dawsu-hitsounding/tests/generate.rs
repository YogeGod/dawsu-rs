//! New hitsound diff and Generate hitsound diff (Generate and Copy plan, Tasks 2 and 4; design
//! doc "Generate and Copy hitsounds").

use dawsu_hitsounding::generate::{new_hitsound_diff, NewDiff};
use dawsu_osu_format::{OsuFile, SaveOutcome};
use dawsu_test_support::{save_pack_bytes, save_pack_files, Scratch};

fn read(bytes: &[u8]) -> OsuFile {
    OsuFile::read(bytes.to_vec()).file
}

/// The lines outside `[HitObjects]` records and the `Version:` and `BeatmapID:` lines, with
/// their sections and bytes (endings included).
fn kept_lines(file: &OsuFile) -> Vec<(Option<String>, Vec<u8>)> {
    file.lines()
        .filter_map(|l| {
            let section = file.section_of(l.id());
            let text = String::from_utf8_lossy(l.content()).to_string();
            let t = text.trim();
            // The whole [HitObjects] section may change: removing its last object passes "no final
            // newline" on to the line before it.
            let in_objects = section.as_deref() == Some("HitObjects");
            let is_named = section.as_deref() == Some("Metadata") && (t.starts_with("Version") || t.starts_with("BeatmapID"));
            (!in_objects && !is_named).then(|| (section, [l.content(), ending(l.ending())].concat()))
        })
        .collect()
}

fn ending(e: dawsu_osu_format::LineEnding) -> &'static [u8] {
    match e {
        dawsu_osu_format::LineEnding::CrLf => b"\r\n",
        dawsu_osu_format::LineEnding::Lf => b"\n",
        dawsu_osu_format::LineEnding::Cr => b"\r",
        dawsu_osu_format::LineEnding::None => b"",
    }
}

fn hit_object_records(file: &OsuFile) -> usize {
    file.lines()
        .filter(|l| file.section_of(l.id()).as_deref() == Some("HitObjects"))
        .filter(|l| {
            let t = String::from_utf8_lossy(l.content()).trim().to_string();
            !t.is_empty() && !t.starts_with("//") && !t.starts_with('[')
        })
        .count()
}

fn planned(outcome: NewDiff) -> (OsuFile, String) {
    match outcome {
        NewDiff::Planned { file, file_name } => (file, file_name),
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------- New hitsound diff (Task 2)

#[test]
fn invariant_2_a_new_diff_differs_only_in_hit_objects_version_and_beatmap_id() {
    for (name, path) in save_pack_files() {
        let bytes = std::fs::read(&path).unwrap();
        let source = read(&bytes);
        let (file, _) = planned(new_hitsound_diff(&source, path.file_name().unwrap().to_str().unwrap(), "Timing", &[]));
        assert_eq!(kept_lines(&file), kept_lines(&source), "{name}: every other line, BOM and ending kept");
        assert_eq!(file.has_bom(), source.has_bom(), "{name}");
        assert_eq!(hit_object_records(&file), 0, "{name}: no object left");
        assert_eq!(file.value("Metadata", "Version").as_deref(), Some("Timing"), "{name}");
        assert_eq!(file.value("Metadata", "BeatmapID").as_deref(), Some("0"), "{name}");
    }
}

#[test]
fn object_lines_lazer_cannot_read_are_removed_too() {
    // A decimal in the type field: lazer skips the line (format Responsibility 8).
    let source = read(b"osu file format v14\r\n\r\n[Metadata]\r\nVersion:x\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n256,192,2000,1.5,0,0:0:0:0:\r\n");
    assert_eq!((hit_object_records(&source), source.hit_objects().len()), (2, 1), "one object line lazer skips");
    let (file, _) = planned(new_hitsound_diff(&source, "invalid.osu", "Timing", &[]));
    assert_eq!(hit_object_records(&file), 0);
}

#[test]
fn a_new_diff_is_named_from_the_metadata_with_a_fixed_set_of_characters_replaced() {
    let source = read(&save_pack_bytes("owner/catalyst-impetus-std.osu"));
    let (_, file_name) = planned(new_hitsound_diff(&source, "Arch Echo - Catalyst (Yoge God) [Impetus].osu", "  Timing ", &[]));
    assert_eq!(file_name, "Arch Echo - Catalyst (Yoge God) [Timing].osu", "the name is trimmed");

    let odd = read(b"osu file format v14\r\n\r\n[Metadata]\r\nTitle:What? \"Now\"\r\nArtist:AC/DC\r\nCreator:a|b\r\nVersion:x\r\n\r\n[HitObjects]\r\n");
    let (_, file_name) = planned(new_hitsound_diff(&odd, "any.osu", "v<1>", &[]));
    assert_eq!(file_name, "AC_DC - What_ _Now_ (a_b) [v_1_].osu", "the same on every system");

    let no_artist = read(b"osu file format v14\r\n\r\n[Metadata]\r\nTitle:t\r\nVersion:Hard\r\n\r\n[HitObjects]\r\n");
    let (_, file_name) = planned(new_hitsound_diff(&no_artist, "song [Hard].osu", "Timing", &[]));
    assert_eq!(file_name, "song [Timing].osu", "without an artist the source's file name is used");
}

#[test]
fn a_blank_name_a_taken_version_or_a_taken_file_name_is_refused() {
    let source = read(&save_pack_bytes("owner/catalyst-impetus-std.osu"));
    let source_name = "Arch Echo - Catalyst (Yoge God) [Impetus].osu";
    assert!(matches!(new_hitsound_diff(&source, source_name, "   ", &[]), NewDiff::BlankName));
    let folder = vec![(source_name.to_string(), Some("Impetus".to_string())), ("x.osu".to_string(), Some("TIMING".to_string()))];
    assert!(matches!(new_hitsound_diff(&source, source_name, "timing", &folder), NewDiff::VersionTaken));
    let folder = vec![("arch echo - catalyst (yoge god) [timing].osu".to_string(), None)];
    assert!(matches!(new_hitsound_diff(&source, source_name, "Timing", &folder), NewDiff::NameTaken));
}

#[test]
fn planning_writes_nothing_and_the_plan_never_replaces_a_file() {
    let scratch = Scratch::new("new-diff");
    let source_path = scratch.path("source.osu");
    std::fs::write(&source_path, save_pack_bytes("owner/catalyst-impetus-std.osu")).unwrap();
    let before: Vec<_> = std::fs::read_dir(scratch.dir()).unwrap().map(|e| e.unwrap().file_name()).collect();
    let (file, file_name) = planned(new_hitsound_diff(&read(&std::fs::read(&source_path).unwrap()), "source.osu", "Timing", &[]));
    let after: Vec<_> = std::fs::read_dir(scratch.dir()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after, "invariant 1");

    // The plan starts from the source's bytes, so it is written only as a new file.
    assert_eq!(file.save_new(&source_path).unwrap(), SaveOutcome::AlreadyExists);
    assert_eq!(file.save_new(&scratch.path(&file_name)).unwrap(), SaveOutcome::Saved { warnings: vec![] });
    assert_eq!(std::fs::read(&source_path).unwrap(), save_pack_bytes("owner/catalyst-impetus-std.osu"));
}
