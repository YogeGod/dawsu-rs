//! `.osz` import and export (osu format design doc Responsibilities 10 and 13, invariants 10, 13
//! and 14; the last-four-parts plan, part C). Old cases are from the old app's
//! `NewDiffAndOszTests.cs` at c81774e.

use dawsu_osu_format::{export_osz, import_osz, OszExport, OszImport, OszLimits};
use dawsu_test_support::{save_pack_dir, Scratch};
use std::io::Write;
use std::path::{Path, PathBuf};

/// A zip of `(name, bytes)` entries; a name ending in `/` is a folder entry.
fn zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in entries {
        if name.ends_with('/') {
            w.add_directory(*name, options).unwrap();
        } else {
            w.start_file(*name, options).unwrap();
            w.write_all(bytes).unwrap();
        }
    }
    w.finish().unwrap();
}

fn import(archive: &Path, songs: &Path) -> OszImport {
    import_osz(archive, songs, OszLimits::default(), &|| false, &mut |_, _| {}).unwrap()
}

fn imported(outcome: OszImport) -> PathBuf {
    match outcome {
        OszImport::Imported { folder } => folder,
        other => panic!("{other:?}"),
    }
}

/// Every file under `dir` as (relative path with `/`, bytes), sorted.
fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let rel = format!("{prefix}{}", e.file_name().to_string_lossy());
            if e.file_type().unwrap().is_dir() {
                walk(&e.path(), &format!("{rel}/"), out);
            } else {
                out.push((rel, std::fs::read(e.path()).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out.sort();
    out
}

fn songs_entries(songs: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(songs).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

#[test]
fn a_folder_is_named_after_the_archive_and_a_clash_gives_2() {
    let s = Scratch::new("osz-name");
    let songs = s.path("Songs");
    std::fs::create_dir(&songs).unwrap();
    let archive = s.path("Map Set.osz");
    zip(&archive, &[("map.osu", b"osu file format v14\r\n"), ("sb/", b""), ("sb\\bg.png", b"png")]);
    let first = imported(import(&archive, &songs));
    assert_eq!(first, songs.join("Map Set"));
    assert_eq!(files(&first), [("map.osu".to_string(), b"osu file format v14\r\n".to_vec()), ("sb/bg.png".to_string(), b"png".to_vec())], "folder entries skipped, backslashes are folders");
    assert_eq!(imported(import(&archive, &songs)), songs.join("Map Set (2)"), "never merged into an existing folder");
    assert_eq!(songs_entries(&songs), ["Map Set", "Map Set (2)"], "no staging folder left");
}

#[test]
fn the_archive_s_own_name_is_cleaned() {
    let s = Scratch::new("osz-archive-name");
    let songs = s.path("Songs");
    std::fs::create_dir(&songs).unwrap();
    for (archive, folder) in [("Hello..osz", "Hello"), ("CON.osz", "Imported beatmap"), ("a_b.osz", "a_b")] {
        let path = s.path(archive);
        zip(&path, &[("map.osu", b"x")]);
        assert_eq!(imported(import(&path, &songs)), songs.join(folder), "{archive}");
    }
}

#[test]
fn unsafe_entry_names_are_refused_and_nothing_is_written() {
    let s = Scratch::new("osz-unsafe");
    let songs = s.path("Songs");
    std::fs::create_dir(&songs).unwrap();
    for (i, bad) in ["../evil.txt", "sub/../../evil.txt", "/evil.txt", "C:/evil.txt", "map.osu:hidden", "sounds/NUL.wav", "COM1", "trail./x.wav", "space /x.wav", "a<b.wav", "sub//x.wav"].iter().enumerate() {
        let path = s.path(&format!("bad{i}.osz"));
        zip(&path, &[("map.osu", b"x"), (bad, b"evil")]);
        assert!(matches!(import(&path, &songs), OszImport::UnsafeName(_)), "{bad}");
    }
    let path = s.path("case.osz");
    zip(&path, &[("Audio.mp3", b"a"), ("audio.mp3", b"b")]);
    assert!(matches!(import(&path, &songs), OszImport::UnsafeName(_)), "two names that differ only in case");
    let path = s.path("link.osz");
    let mut w = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    w.add_symlink("link.wav", "../../outside.wav", zip::write::SimpleFileOptions::default()).unwrap();
    w.finish().unwrap();
    assert!(matches!(import(&path, &songs), OszImport::UnsafeName(_)), "a symlink entry");
    assert!(songs_entries(&songs).is_empty(), "the Songs folder is left empty");
}

#[test]
fn the_limits_and_a_stop_leave_nothing_behind() {
    let s = Scratch::new("osz-limits");
    let songs = s.path("Songs");
    std::fs::create_dir(&songs).unwrap();
    let path = s.path("big.osz");
    zip(&path, &[("a.wav", &[0u8; 600]), ("b.wav", &[1u8; 600])]);
    let small = OszLimits { max_bytes: 1000, max_entries: 20000 };
    assert!(matches!(import_osz(&path, &songs, small, &|| false, &mut |_, _| {}).unwrap(), OszImport::TooLarge));
    let few = OszLimits { max_bytes: 1 << 30, max_entries: 1 };
    assert!(matches!(import_osz(&path, &songs, few, &|| false, &mut |_, _| {}).unwrap(), OszImport::TooManyEntries));
    assert!(matches!(import_osz(&path, &songs, OszLimits::default(), &|| true, &mut |_, _| {}).unwrap(), OszImport::Cancelled));
    std::fs::write(s.path("not.osz"), b"not a zip").unwrap();
    assert!(matches!(import(&s.path("not.osz"), &songs), OszImport::NotAnArchive(_)));
    assert!(songs_entries(&songs).is_empty());
}

#[test]
fn invariant_13_export_then_import_gives_the_same_names_and_bytes() {
    let s = Scratch::new("osz-round-trip");
    let target = s.path("pack.osz");
    let mut steps = Vec::new();
    assert_eq!(export_osz(&save_pack_dir(), &target, &|| false, &mut |done, total| steps.push((done, total))).unwrap(), OszExport::Exported);
    assert_eq!(steps.last().map(|s| s.0 == s.1), Some(true), "progress reaches the total");
    let songs = s.path("Songs");
    std::fs::create_dir(&songs).unwrap();
    let folder = imported(import(&target, &songs));
    assert_eq!(files(&folder), files(&save_pack_dir()));

    let again = s.path("again.osz");
    export_osz(&save_pack_dir(), &again, &|| false, &mut |_, _| {}).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), std::fs::read(&again).unwrap(), "two exports give the same bytes");
}

#[test]
fn invariant_14_a_failed_or_stopped_export_leaves_the_target_as_it_was() {
    let s = Scratch::new("osz-export-fail");
    let target = s.path("pack.osz");
    std::fs::write(&target, b"the old archive").unwrap();
    assert_eq!(export_osz(&save_pack_dir(), &target, &|| true, &mut |_, _| {}).unwrap(), OszExport::Cancelled);
    assert_eq!(std::fs::read(&target).unwrap(), b"the old archive");
    assert_eq!(songs_entries(s.dir()), ["pack.osz"], "no temporary file left");

    let folder = s.path("Set");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("map.osu"), b"x").unwrap();
    assert_eq!(export_osz(&folder, &folder.join("Set.osz"), &|| false, &mut |_, _| {}).unwrap(), OszExport::TargetInsideFolder);
    assert!(export_osz(&s.path("missing"), &s.path("m.osz"), &|| false, &mut |_, _| {}).is_err(), "a missing folder is an environment failure");
}
