//! Setting a `key:value` line and listing and removing storyboard sample lines (Generate and
//! Copy plan, Task 1; osu format Responsibilities 4, 5 and 11).

use dawsu_osu_format::{OsuFile, Refusal, SetOutcome};
use dawsu_test_support::{save_pack_bytes, save_pack_files};

fn read(bytes: &[u8]) -> OsuFile {
    OsuFile::read(bytes.to_vec()).file
}

/// The file's lines whose bytes differ, as (before, after) text.
fn differing_lines(before: &[u8], after: &[u8]) -> Vec<(String, String)> {
    let (a, b): (Vec<&[u8]>, Vec<&[u8]>) = (before.split_inclusive(|c| *c == b'\n').collect(), after.split_inclusive(|c| *c == b'\n').collect());
    assert_eq!(a.len(), b.len(), "the line count changed");
    a.iter().zip(&b).filter(|(x, y)| x != y).map(|(x, y)| (String::from_utf8_lossy(x).into_owned(), String::from_utf8_lossy(y).into_owned())).collect()
}

#[test]
fn setting_version_changes_only_that_value_on_every_save_pack_file() {
    for (name, path) in save_pack_files() {
        let bytes = std::fs::read(&path).unwrap();
        let mut file = read(&bytes);
        if file.value("Metadata", "Version").is_none() {
            continue;
        }
        assert_eq!(file.set_value("Metadata", "Version", "dawsu test"), Ok(SetOutcome::Changed), "{name}");
        assert_eq!(file.value("Metadata", "Version").as_deref(), Some("dawsu test"), "{name}");
        let changed = differing_lines(&bytes, &file.to_bytes());
        assert_eq!(changed.len(), 1, "{name}: {changed:?}");
        assert!(changed[0].0.trim_start().starts_with("Version") && changed[0].1.contains("dawsu test"), "{name}: {changed:?}");
        let ending = |l: &str| l.len() - l.trim_end_matches(['\r', '\n']).len();
        assert_eq!(ending(&changed[0].0), ending(&changed[0].1), "{name}: the line ending is kept");
    }
}

#[test]
fn setting_the_value_a_line_holds_changes_nothing() {
    let bytes = save_pack_bytes("owner/catalyst-impetus-std.osu");
    let mut file = read(&bytes);
    assert_eq!(file.set_value("Metadata", "Version", "Impetus"), Ok(SetOutcome::Unchanged));
    assert_eq!(file.to_bytes(), bytes);
    assert!(file.change_summary().is_empty());
}

#[test]
fn a_missing_key_is_added_to_its_section_and_a_missing_section_in_osu_s_order() {
    let mut file = read(b"osu file format v14\r\n\r\n[General]\r\nMode: 0\r\n\r\n[Metadata]\r\nTitle:x\r\n\r\n[HitObjects]\r\n");
    assert_eq!(file.set_value("Metadata", "BeatmapID", "0"), Ok(SetOutcome::Changed));
    assert_eq!(file.value("Metadata", "BeatmapID").as_deref(), Some("0"));
    assert_eq!(file.set_value("Difficulty", "SliderMultiplier", "1.4"), Ok(SetOutcome::Changed));
    let text = String::from_utf8(file.to_bytes()).unwrap();
    assert_eq!(text, "osu file format v14\r\n\r\n[General]\r\nMode: 0\r\n\r\n[Metadata]\r\nTitle:x\r\nBeatmapID:0\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\n\r\n[HitObjects]\r\n");
}

#[test]
fn of_two_lines_for_one_key_the_last_is_read_and_set_as_lazer_reads_the_last() {
    // LegacyBeatmapDecoder.handleMetadata assigns the value for every line (131fe72, :351-380).
    let mut file = read(b"[Metadata]\r\nVersion:Old\r\nVersion:New\r\n");
    assert_eq!(file.value("Metadata", "Version").as_deref(), Some("New"));
    file.set_value("Metadata", "Version", "Hitsounds").unwrap();
    assert_eq!(String::from_utf8(file.to_bytes()).unwrap(), "[Metadata]\r\nVersion:Old\r\nVersion:Hitsounds\r\n");
}

#[test]
fn a_value_with_a_line_break_is_refused() {
    let mut file = read(b"[Metadata]\r\nVersion:Old\r\n");
    assert_eq!(file.set_value("Metadata", "Version", "a\r\nb"), Err(Refusal::LineBreak));
    assert_eq!(file.set_value("Metadata", "Version", "a\nb"), Err(Refusal::LineBreak));
    assert_eq!(file.to_bytes(), b"[Metadata]\r\nVersion:Old\r\n");
}

#[test]
fn a_line_that_is_not_utf8_is_refused() {
    let mut file = read(b"[Metadata]\r\nVersion:Caf\xE9\r\n");
    assert_eq!(file.set_value("Metadata", "Version", "x"), Err(Refusal::NotUtf8));
}

#[test]
fn storyboard_samples_are_listed_in_both_forms_and_removed_one_by_one() {
    let bytes = save_pack_bytes("synthetic/storyboard-and-colours.osu");
    let file = read(&bytes);
    let samples = file.storyboard_samples();
    assert_eq!(samples.len(), 1);
    let s = &samples[0];
    assert_eq!((s.time, s.layer.as_str(), s.path.as_str(), s.volume), (1500.0, "0", "sb/whoosh.wav", 80.0));

    let mut other = read(b"[Events]\r\n5,250,1,\"kick.ogg\"\r\nSample,300,0,\"a b.wav\",40\r\n");
    let samples = other.storyboard_samples();
    assert_eq!(samples.iter().map(|s| (s.time, s.path.as_str(), s.volume)).collect::<Vec<_>>(), vec![(250.0, "kick.ogg", 100.0), (300.0, "a b.wav", 40.0)], "a missing volume is 100");
    other.remove_storyboard_sample(samples[0].line).unwrap();
    assert_eq!(other.to_bytes(), b"[Events]\r\nSample,300,0,\"a b.wav\",40\r\n");

    let mut whole = read(&bytes);
    let line = whole.storyboard_samples()[0].line;
    whole.remove_storyboard_sample(line).unwrap();
    let after = whole.to_bytes();
    let removed: Vec<&[u8]> = bytes.split_inclusive(|c| *c == b'\n').filter(|l| !l.starts_with(b"Sample,")).collect();
    assert_eq!(after, removed.concat(), "every other byte is kept");
    assert!(whole.storyboard_samples().is_empty());
    assert_eq!(whole.remove_storyboard_sample(line), Err(Refusal::NotFound));
}
