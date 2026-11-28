//! Whole-crate tests for dawsu-osu-format (Task 5 of the first M2 slice).

use dawsu_foundation::safe_write::BackupPolicy;
use dawsu_osu_format::{
    check_map_path, find_case_insensitive, list_folder, round_trip_check, FolderIndex, LineEnding, OsuFile, PathRefusal, Refusal, SaveOutcome,
    SetOutcome,
};
use dawsu_test_support::{save_pack_bytes, save_pack_files, Scratch};
use std::fs;

fn read(name: &str) -> OsuFile {
    OsuFile::read(save_pack_bytes(name)).file
}

// ---- invariant 1 and Responsibility 2: every save-pack file round-trips, one test per file

macro_rules! round_trip {
    ($($test:ident => $name:literal),* $(,)?) => {
        $(
            #[test]
            fn $test() {
                let bytes = save_pack_bytes($name);
                let file = OsuFile::read(bytes.clone()).file;
                assert_eq!(file.to_bytes(), bytes, "{} must write back byte for byte", $name);
                let report = round_trip_check(&bytes);
                assert!(report.same_bytes, "{}: {report:?}", $name);
                assert!(report.same_value_write_changes_nothing, "{}: {report:?}", $name);
            }
        )*
    };
}

round_trip! {
    round_trip_catalyst_hitsound_diff => "owner/catalyst-hitsound-diff.osu",
    round_trip_catalyst_impetus_std => "owner/catalyst-impetus-std.osu",
    round_trip_mukduk_lazer_v128_bom => "owner/mukduk-lazer-v128-bom.osu",
    round_trip_satellites_mania_18k => "owner/satellites-mania-18k-keysounds.osu",
    round_trip_bom_crlf_v14 => "synthetic/bom-crlf-v14.osu",
    round_trip_catch_fruits => "synthetic/catch-fruits-juice-bananas.osu",
    round_trip_custom_samples_and_edges => "synthetic/custom-samples-and-edges.osu",
    round_trip_invalid_utf8_lines => "synthetic/invalid-utf8-lines.osu",
    round_trip_lf_no_final_newline => "synthetic/lf-no-final-newline.osu",
    round_trip_mania_4k_holds => "synthetic/mania-4k-holds-keysounds.osu",
    round_trip_mixed_line_endings => "synthetic/mixed-line-endings.osu",
    round_trip_storyboard_and_colours => "synthetic/storyboard-and-colours.osu",
    round_trip_taiko_don_kat => "synthetic/taiko-don-kat-finishers.osu",
    round_trip_unknown_sections_and_comments => "synthetic/unknown-sections-and-comments.osu",
    round_trip_v128_lazer_style => "synthetic/v128-lazer-style.osu",
    round_trip_v3_short_fields => "synthetic/v3-short-fields.osu",
    round_trip_v7_old_hitsample => "synthetic/v7-old-hitsample.osu",
}

#[test]
fn every_save_pack_file_has_a_round_trip_test() {
    // 17 files, 17 named tests above.
    assert_eq!(save_pack_files().len(), 17);
}

#[test]
fn every_hitsound_field_of_the_save_pack_parses() {
    for (name, path) in save_pack_files() {
        let read = OsuFile::read(fs::read(&path).unwrap());
        assert!(read.warnings.is_empty(), "{name}: {:?}", read.warnings);
    }
}

// ---- Responsibility 1: lines, endings, BOM and sections

#[test]
fn a_file_remembers_its_bom_its_line_endings_and_sections() {
    let file = read("owner/mukduk-lazer-v128-bom.osu");
    assert!(file.has_bom());
    let mixed = read("synthetic/mixed-line-endings.osu");
    let endings: Vec<LineEnding> = mixed.lines().map(|l| l.ending()).take(4).collect();
    assert_eq!(endings, vec![LineEnding::CrLf, LineEnding::Lf, LineEnding::Cr, LineEnding::CrLf]);
    let no_final = read("synthetic/lf-no-final-newline.osu");
    assert_eq!(no_final.lines().last().unwrap().ending(), LineEnding::None);
    let hit_object_line = read("synthetic/v3-short-fields.osu").hit_objects()[0].line();
    assert_eq!(read("synthetic/v3-short-fields.osu").section_of(hit_object_line).as_deref(), Some("HitObjects"));
}

// ---- Responsibility 3: typed fields

#[test]
fn typed_timing_point_and_hit_object_fields() {
    let file = read("synthetic/custom-samples-and-edges.osu");
    assert_eq!(file.format_version(), Some(14));
    assert_eq!(file.general("SampleSet").as_deref(), Some("Soft"));

    let tps = file.timing_points();
    assert_eq!(tps.len(), 4);
    assert!(tps[0].uninherited());
    assert_eq!(tps[0].beat_length(), 500.0);
    assert_eq!(tps[1].sample_set(), Some(2));
    assert_eq!(tps[1].sample_index(), Some(1));
    assert_eq!(tps[1].volume(), Some(60));
    assert!(!tps[1].uninherited());
    assert_eq!(tps[1].effects(), Some(1));

    let objects = file.hit_objects();
    assert_eq!(objects.len(), 5);
    let kick = &objects[1];
    assert_eq!(kick.time(), 500.0);
    assert_eq!(kick.hitsound(), 14);
    let sample = kick.hit_sample().unwrap();
    assert_eq!((sample.normal_set, sample.addition_set, sample.index, sample.volume), (2, 3, 5, 85));
    assert_eq!(sample.filename.as_deref(), Some("kick-layer.wav"), "the file name is kept as text");

    let slider = objects[2].slider().unwrap();
    assert_eq!(slider.curve, "P|350:150|400:200");
    assert_eq!(slider.slides, 3);
    assert_eq!(slider.length, Some(100.0));
    assert_eq!(slider.edge_sounds.as_deref(), Some(&["2", "4", "8", "0"].map(String::from)[..]));
    let banks: Vec<Option<(i32, i32)>> = slider.edge_sets.as_ref().unwrap().iter().map(|s| s.as_ref().map(|h| (h.normal_set, h.addition_set))).collect();
    assert_eq!(banks, vec![Some((1, 2)), Some((2, 1)), Some((0, 0)), Some((3, 3))]);

    assert_eq!(objects[3].spinner_end(), Some(3500.0));
    assert_eq!(objects[4].hit_sample().unwrap().filename.as_deref(), Some("dir/sub sample.ogg"));
}

#[test]
fn a_slider_node_entry_is_read_in_full() {
    // lazer reads a node entry's third, fourth and fifth parts as its own index, volume and file
    // name (ConvertHitObjectParser.cs:127 and :231-237 at 131fe72).
    let read = OsuFile::read(b"osu file format v14\r\n\r\n[HitObjects]\r\n100,100,1000,2,0,L|300:100,2,200,0|8|0,2:0:3:40|0:0:0:-5|1:0:2:0:kick.wav,0:0:0:0:\r\n".to_vec());
    let objects = read.file.hit_objects();
    let sets = objects[0].slider().unwrap().edge_sets.clone().unwrap();
    let parts: Vec<(i32, i32, i32, i32, Option<String>)> = sets.into_iter().map(|s| s.map(|h| (h.normal_set, h.addition_set, h.index, h.volume, h.filename)).unwrap()).collect();
    assert_eq!(parts, vec![(2, 0, 3, 40, None), (0, 0, 0, 0, None), (1, 0, 2, 0, Some("kick.wav".into()))], "a negative volume reads as 0");
}

// ---- Responsibility 8: numbers read the way lazer reads them

#[test]
fn a_decimal_in_a_whole_number_field_skips_the_line_with_a_warning() {
    let text = "osu file format v14\r\n\r\n[TimingPoints]\r\n0,500,4,2,0,70,1,0\r\n1000,-100,4,2.5,0,70,0,0\r\n\r\n[HitObjects]\r\n256,192,0,1,0,0:0:0:0:\r\n256,192,500,1.0,0,0:0:0:0:\r\n256,192,600,1,0,0:0:0:0:\r\n";
    let read = OsuFile::read(text.as_bytes().to_vec());
    assert_eq!(read.file.timing_points().len(), 1);
    assert_eq!(read.file.hit_objects().len(), 2);
    assert_eq!(read.warnings.len(), 2);
    assert_eq!(read.warnings[0].line_number, 5);
    assert_eq!(read.warnings[1].line_number, 9);
    assert_eq!(read.file.to_bytes(), text.as_bytes(), "the skipped lines keep their bytes");
}

#[test]
fn decimals_are_read_where_lazer_reads_decimals() {
    let text = "osu file format v14\r\n\r\n[TimingPoints]\r\n10.5,333.33,4,2,0,70,1,0\r\n\r\n[HitObjects]\r\n256.7,192.2,100.25,1,0,0:0:0:0:\r\n";
    let read = OsuFile::read(text.as_bytes().to_vec());
    assert!(read.warnings.is_empty());
    assert_eq!(read.file.timing_points()[0].time(), 10.5);
    assert_eq!(read.file.hit_objects()[0].time(), 100.25);
}

#[test]
fn a_lone_slider_node_hitsound_decimal_is_not_a_failure_as_in_lazer() {
    let text = "osu file format v14\r\n\r\n[HitObjects]\r\n300,200,1000,2,0,B|400:200,1,140,2.5|8,0:0|0:0,0:0:0:0:\r\n";
    let read = OsuFile::read(text.as_bytes().to_vec());
    assert!(read.warnings.is_empty(), "lazer reads per-node hitsounds with TryParse");
}

// ---- Responsibility 4 and invariants 3, 4, 5, 6

#[test]
fn setting_a_field_to_its_own_value_changes_nothing() {
    let bytes = save_pack_bytes("synthetic/custom-samples-and-edges.osu");
    let mut file = OsuFile::read(bytes.clone()).file;
    let line = file.hit_objects()[1].line();
    assert_eq!(file.set_field(line, 4, "14"), Ok(SetOutcome::Unchanged));
    assert_eq!(file.to_bytes(), bytes);
    assert!(file.change_summary().is_empty());
}

#[test]
fn setting_one_field_changes_only_that_field() {
    let mut file = read("synthetic/custom-samples-and-edges.osu");
    let line = file.hit_objects()[1].line();
    assert_eq!(file.set_field(line, 4, "10"), Ok(SetOutcome::Changed));
    assert_eq!(file.line_text(line).unwrap(), "100,100,500,1,10,2:3:5:85:kick-layer.wav");
}

#[test]
fn a_line_that_is_not_utf8_cannot_be_edited() {
    let bytes = save_pack_bytes("synthetic/invalid-utf8-lines.osu");
    let mut file = OsuFile::read(bytes.clone()).file;
    let line = file.hit_objects().last().unwrap().line();
    assert_eq!(file.set_field(line, 4, "8"), Err(Refusal::NotUtf8));
    assert_eq!(file.to_bytes(), bytes);
}

#[test]
fn missing_fields_are_filled_with_what_osu_played() {
    let mut file = read("synthetic/v3-short-fields.osu");
    // A circle with no hit-sample field gets the default one before its new volume.
    let circle = file.hit_objects()[0].line();
    file.set_field(circle, 5, "0:0:0:40:").unwrap();
    assert_eq!(file.line_text(circle).unwrap(), "256,192,0,1,0,0:0:0:40:");

    // A slider's missing edge sounds are its own hitsound on every node, never 0; its edge sets inherit.
    let mut file = read("synthetic/v3-short-fields.osu");
    let slider = file.hit_objects()[2].line();
    file.set_field(slider, 10, "1:0:0:0:").unwrap();
    assert_eq!(file.line_text(slider).unwrap(), "300,200,1000,2,0,B|400:200|400:300,1,140,0|0,0:0|0:0,1:0:0:0:");

    // A timing line with only time and beat length gets meter 4, the map's sample set, index 0, volume 100.
    let mut file = read("synthetic/v3-short-fields.osu");
    let green = file.timing_points()[1].line();
    file.set_field(green, 5, "60").unwrap();
    assert_eq!(file.line_text(green).unwrap(), "1000,-100,4,0,0,60");
}

// ---- Responsibility 5 and invariants 7, 8

#[test]
fn a_green_line_is_inserted_after_the_last_point_at_or_before_its_time() {
    let mut file = read("synthetic/custom-samples-and-edges.osu");
    let id = file.insert_green_line("3000,-100,4,2,0,50,0,0");
    let order: Vec<f64> = file.timing_points().iter().map(|t| t.time()).collect();
    assert_eq!(order, vec![0.0, 2000.0, 3000.0, 4000.0, 6000.0]);
    assert_eq!(file.timing_points()[2].line(), id);
}

#[test]
fn a_hit_circle_is_inserted_in_time_order() {
    let mut file = read("synthetic/custom-samples-and-edges.osu");
    file.insert_hit_object("128,128,750,1,0,0:0:0:0:");
    let times: Vec<f64> = file.hit_objects().iter().map(|o| o.time()).collect();
    assert_eq!(times, vec![0.0, 500.0, 750.0, 1000.0, 2500.0, 4000.0]);
}

#[test]
fn a_new_line_copies_its_neighbours_line_ending() {
    let mut file = read("synthetic/mixed-line-endings.osu");
    let id = file.insert_green_line("3000,-100,4,2,0,50,0,0");
    // The new line follows "2000,-100,4,2,1,60,0,1", whose ending is CRLF.
    assert_eq!(file.line_ending(id), Some(LineEnding::CrLf));

    let mut lf = read("synthetic/lf-no-final-newline.osu");
    let id = lf.insert_hit_object("128,128,999999,1,0,0:0:0:0:");
    // After a last line with no ending, the old last line gets the file's first ending and the new line has none.
    assert_eq!(lf.line_ending(id), Some(LineEnding::None));
    assert!(lf.to_bytes().ends_with(b"\n128,128,999999,1,0,0:0:0:0:"));
}

#[test]
fn a_red_line_is_never_removed_as_a_green_line() {
    let mut file = read("synthetic/custom-samples-and-edges.osu");
    let red = file.timing_points()[0].line();
    let green = file.timing_points()[1].line();
    assert_eq!(file.remove_green_line(red), Err(Refusal::RedLine));
    assert_eq!(file.remove_green_line(green), Ok(()));
    assert_eq!(file.timing_points().len(), 3);
}

#[test]
fn bookmarks_go_into_the_editor_section_and_a_missing_section_is_added() {
    let mut file = read("synthetic/v3-short-fields.osu");
    file.set_bookmarks(&[1000, 2000]);
    let text = String::from_utf8(file.to_bytes()).unwrap();
    assert!(text.contains("[Editor]\r\nBookmarks: 1000,2000\r\n"), "{text}");
}

// ---- Responsibility 6: the change summary

#[test]
fn the_change_summary_names_every_changed_line() {
    let mut file = read("synthetic/custom-samples-and-edges.osu");
    let circle = file.hit_objects()[0].line();
    file.set_field(circle, 4, "8").unwrap();
    let green = file.timing_points()[1].line();
    file.set_field(green, 5, "40").unwrap();
    file.insert_green_line("5000,-100,4,2,0,30,0,0");
    let summary = file.change_summary();
    assert_eq!(summary.hit_objects_changed, 1);
    assert_eq!(summary.green_lines_edited, 1);
    assert_eq!(summary.green_lines_added, 1);
    assert_eq!(summary.other_lines_changed, 0);
    assert!(!summary.touches_more_than_hitsounds());
}

// ---- Responsibility 9: folder listing and path checks

#[test]
fn a_folder_is_listed_and_found_case_insensitively() {
    let s = Scratch::new("folder");
    fs::write(s.path("Soft-HitClap2.wav"), b"x").unwrap();
    fs::create_dir_all(s.path("sb")).unwrap();
    fs::write(s.path("sb/a.png"), b"x").unwrap();
    let listing = list_folder(s.dir()).unwrap();
    assert_eq!(listing, vec!["Soft-HitClap2.wav".to_string(), "sb/a.png".to_string()]);
    assert_eq!(find_case_insensitive(&listing, "soft-hitclap2.wav"), Some("Soft-HitClap2.wav"));
    assert_eq!(find_case_insensitive(&listing, "SB/A.PNG"), Some("sb/a.png"));
    assert_eq!(find_case_insensitive(&listing, "missing.wav"), None);
}

#[test]
fn a_folder_index_finds_what_the_listing_search_finds() {
    // Two names that differ only in case (possible on Linux): the first listed wins, as before.
    let listing: Vec<String> = ["A.wav", "Soft-HitClap2.wav", "a.wav", "sb/A.png", "sb/a.png"]
        .map(String::from).to_vec();
    let index = FolderIndex::new(&listing);
    for name in ["a.wav", "A.WAV", "soft-hitclap2.wav", "SB/a.PNG", r"sb\a.png", "missing.wav", ""] {
        assert_eq!(index.find(name), find_case_insensitive(&listing, name), "{name}");
    }
    assert_eq!(index.find("a.wav"), Some("A.wav"));
    assert_eq!(index.find(r"sb\A.PNG"), Some("sb/A.png"));
}

#[test]
fn paths_that_would_leave_the_folder_are_refused() {
    assert_eq!(check_map_path("dir/sub sample.ogg"), Ok(()));
    assert_eq!(check_map_path("a/../b.wav"), Ok(()), "climbing that stays inside is allowed");
    for bad in ["../x.wav", "a/../../x.wav", "/etc/x", "C:\\x.wav", "c:x.wav", "\\\\server\\share\\x.wav", "x.wav:stream"] {
        assert!(matches!(check_map_path(bad), Err(PathRefusal { .. })), "{bad} must be refused");
    }
}

// ---- Responsibility 14: one write

#[test]
fn the_one_write_saves_and_keeps_a_backup() {
    let s = Scratch::new("save");
    let bytes = save_pack_bytes("synthetic/custom-samples-and-edges.osu");
    let target = s.path("map.osu");
    fs::write(&target, &bytes).unwrap();
    let mut file = OsuFile::read(fs::read(&target).unwrap()).file;
    let line = file.hit_objects()[0].line();
    file.set_field(line, 4, "8").unwrap();

    let outcome = file.save_over(&target, &BackupPolicy::new(s.path("backups"), 20)).unwrap();

    assert_eq!(outcome, SaveOutcome::Saved { warnings: vec![] });
    assert_eq!(fs::read(&target).unwrap(), file.to_bytes());
    let reopened = OsuFile::read(fs::read(&target).unwrap()).file;
    assert_eq!(reopened.hit_objects()[0].hitsound(), 8);
}

#[test]
fn the_one_write_answers_changed_on_disk_and_leaves_the_file() {
    let s = Scratch::new("save-changed");
    let target = s.path("map.osu");
    fs::write(&target, save_pack_bytes("synthetic/custom-samples-and-edges.osu")).unwrap();
    let mut file = OsuFile::read(fs::read(&target).unwrap()).file;
    let line = file.hit_objects()[0].line();
    file.set_field(line, 4, "8").unwrap();
    fs::write(&target, b"osu! saved this meanwhile").unwrap();

    let outcome = file.save_over(&target, &BackupPolicy::new(s.path("backups"), 20)).unwrap();

    assert_eq!(outcome, SaveOutcome::ChangedOnDisk);
    assert_eq!(fs::read(&target).unwrap(), b"osu! saved this meanwhile");
}

#[test]
fn a_save_with_no_changes_writes_nothing_and_keeps_no_backup() {
    let s = Scratch::new("save-nothing");
    let target = s.path("map.osu");
    fs::write(&target, save_pack_bytes("synthetic/custom-samples-and-edges.osu")).unwrap();
    let file = OsuFile::read(fs::read(&target).unwrap()).file;
    let outcome = file.save_over(&target, &BackupPolicy::new(s.path("backups"), 20)).unwrap();
    assert_eq!(outcome, SaveOutcome::NothingToSave);
    assert!(!s.path("backups").exists());
}

// ---- after a save: the written file keeps the ids an editor took from it

#[test]
fn as_written_reads_back_like_a_fresh_read_but_keeps_line_ids() {
    let mut file = read("synthetic/custom-samples-and-edges.osu");
    let objects: Vec<_> = file.hit_objects().iter().map(|o| (o.line(), o.time())).collect();
    assert!(!objects.is_empty());
    file.insert_green_line("0,-100,4,2,1,60,0,0");
    let fresh = OsuFile::read(file.to_bytes()).file;
    let written = file.as_written();
    assert_eq!(written.to_bytes(), fresh.to_bytes());
    assert_eq!(written.original_bytes(), fresh.original_bytes(), "the written bytes are now the original");
    assert!(written.change_summary().lines.is_empty(), "nothing is left to write");
    // A fresh read numbers lines by position, so every object below the new green line gets another id.
    assert_ne!(fresh.hit_objects().iter().map(|o| o.line()).collect::<Vec<_>>(), objects.iter().map(|o| o.0).collect::<Vec<_>>());
    assert_eq!(written.hit_objects().iter().map(|o| (o.line(), o.time())).collect::<Vec<_>>(), objects);
    // A line inserted later still gets an id no earlier line has.
    let mut more = written.clone();
    let id = more.insert_green_line("1,-100,4,2,1,70,0,0");
    assert!(written.hit_objects().iter().all(|o| o.line() != id));
}
