//! Decompile, compile and the cue list (Task 7; hitsound domain Responsibilities 1-13 and 23,
//! invariants 1-8; ADRs 0035 and 0037).

use dawsu_foundation::safe_write::BackupPolicy;
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound, SongTime};
use dawsu_hitsounding::rules::{answer, saved_volume, Answer};
use dawsu_hitsounding::{compile, cue_list, decompile, Compiled, Decompiled, Hitsounds, LaneSource, MomentId, Place, Placement, ProblemKind, SaveProblem};
use dawsu_osu_format::{OsuFile, SaveOutcome};
use dawsu_test_support::serde_json::Value;
use dawsu_test_support::{oracle_result, oracle_results_dir, save_pack_bytes, save_pack_files, Scratch};
use std::cell::Cell;

fn dec_with(file: &OsuFile, folder: &[String]) -> Hitsounds {
    match decompile(file, folder, &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => panic!("expected hitsounds, got {other:?}"),
    }
}

fn dec(file: &OsuFile) -> Hitsounds {
    dec_with(file, &[])
}

fn comp(file: &OsuFile, h: &Hitsounds, placement: Placement) -> (OsuFile, Vec<SaveProblem>) {
    match compile(file, h, placement, &|| false, &mut |_, _| {}) {
        Compiled::Done { file, problems } => (file, problems),
        Compiled::Cancelled => panic!("cancelled"),
    }
}

fn read(bytes: &[u8]) -> OsuFile {
    OsuFile::read(bytes.to_vec()).file
}

fn reopen(file: &OsuFile) -> OsuFile {
    read(&file.to_bytes())
}

/// Every osu!-mode save-pack file, read.
fn osu_pack() -> Vec<(String, OsuFile)> {
    save_pack_files()
        .into_iter()
        .map(|(name, path)| (name, read(&std::fs::read(path).unwrap())))
        .filter(|(_, f)| matches!(answer(f, &[]), Answer::Objects(_)))
        .collect()
}

fn std(set: SampleSet, sound: Hitsound, index: u32) -> SampleIdentity {
    SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(index) }
}

const CIRCLES: &str = "osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,2,1,60,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:3:\r\n256,192,2000,1,0,0:0:0:3:\r\n";

fn slider_map(object: &str) -> OsuFile {
    read(format!("osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,2,1,60,1,0\r\n\r\n[HitObjects]\r\n{object}\r\n").as_bytes())
}

// ---------------------------------------------------------------- decompile

#[test]
fn invariant_1_decompile_then_compile_with_no_edits_changes_no_byte() {
    let pack = osu_pack();
    assert_eq!(pack.len(), 13, "the save pack's osu!-mode files");
    for (name, file) in &pack {
        let h = dec(file);
        assert!(!h.moments().is_empty(), "{name}");
        for placement in [Placement::GreenLines, Placement::OnObjects] {
            let (out, problems) = comp(file, &h, placement);
            assert!(problems.is_empty(), "{name}: {problems:?}");
            assert_eq!(out.to_bytes(), file.original_bytes(), "{name}");
            assert!(out.change_summary().is_empty(), "{name}");
        }
    }
}

#[test]
fn taiko_catch_and_mania_maps_give_unsupported_mode() {
    for (name, mode) in [
        ("owner/satellites-mania-18k-keysounds.osu", 3),
        ("synthetic/catch-fruits-juice-bananas.osu", 2),
        ("synthetic/mania-4k-holds-keysounds.osu", 3),
        ("synthetic/taiko-don-kat-finishers.osu", 1),
    ] {
        let file = read(&save_pack_bytes(name));
        assert_eq!(decompile(&file, &[], &|| false, &mut |_, _| {}), Decompiled::UnsupportedMode { mode }, "{name}");
    }
}

#[test]
fn every_sample_set_has_its_four_skin_lanes_default_first_and_folder_files_are_lanes() {
    let file = read(format!("{}\r\n", CIRCLES.replace("[General]\r\n", "[General]\r\nAudioFilename: song.mp3\r\n")).as_bytes());
    let folder = vec!["song.mp3".to_string(), "soft-hitclap3.wav".to_string(), "boom.ogg".to_string(), "bg.jpg".to_string()];
    let h = dec_with(&file, &folder);
    assert_eq!(h.lanes()[0].identity, std(SampleSet::Normal, Hitsound::Normal, 0), "the default sound first");
    for set in [SampleSet::Normal, SampleSet::Soft, SampleSet::Drum] {
        for sound in [Hitsound::Normal, Hitsound::Whistle, Hitsound::Finish, Hitsound::Clap] {
            assert!(h.lane(&std(set, sound, 0)).is_some(), "{set:?} {sound:?}");
        }
    }
    assert_eq!(h.lane(&std(SampleSet::Soft, Hitsound::Clap, 3)).unwrap().source, LaneSource::File("soft-hitclap3.wav".into()), "a file no object uses yet");
    assert_eq!(h.lane(&SampleIdentity::File("boom.ogg".into())).unwrap().source, LaneSource::File("boom.ogg".into()));
    assert!(h.lane(&SampleIdentity::File("song.mp3".into())).is_none(), "the song is not a sample");
    assert!(h.lane(&SampleIdentity::File("bg.jpg".into())).is_none());
}

#[test]
fn a_hit_whose_custom_index_file_is_missing_stays_on_its_numbered_lane() {
    // The save pack holds no sample files, so every numbered lane of the slice's map plays the skin.
    let file = read(&save_pack_bytes("owner/catalyst-impetus-std.osu"));
    let h = dec(&file);
    let numbered: Vec<_> = h.moments().iter().flat_map(|m| m.hits()).filter(|l| matches!(l, SampleIdentity::Standard { index, .. } if index.get() >= 1)).collect();
    assert!(!numbered.is_empty());
    for lane in numbered {
        assert_eq!(h.lane(lane).unwrap().source, LaneSource::Skin, "{lane:?}");
    }
}

#[test]
fn invariant_2_the_samples_at_each_part_share_one_volume() {
    for (name, file) in osu_pack() {
        let Answer::Objects(objects) = answer(&file, &[]) else { unreachable!() };
        for o in &objects {
            let mut groups = vec![o.samples.clone()];
            if let Some(s) = &o.slider {
                groups.extend(s.parts.iter().map(|p| p.samples.clone()));
                groups.push(s.tail_samples.clone());
            }
            for g in groups {
                assert!(g.windows(2).all(|w| w[0].lazer.volume == w[1].lazer.volume), "{name} at {}", o.start_time);
            }
        }
        let h = dec(&file);
        assert_eq!(h.moments().iter().filter(|m| m.hits().count() == 0).count(), 0, "{name}: every decompiled moment sounds");
    }
}

// ---------------------------------------------------------------- the cue list

fn number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap(),
        other => panic!("not a number: {other}"),
    }
}

#[test]
fn cue_times_equal_the_oracle_s_object_and_nested_object_times() {
    for (name, path) in save_pack_files() {
        let bytes = std::fs::read(&path).unwrap();
        let file = read(&bytes);
        let Decompiled::Done(h) = decompile(&file, &[], &|| false, &mut |_, _| {}) else { continue };
        let lazer = oracle_result(&oracle_results_dir(), "save-pack", &name, &bytes);
        let mut expected = Vec::new();
        for o in lazer["hit_objects"].as_array().unwrap() {
            match o["type"].as_str().unwrap() {
                "HitCircle" => expected.push(number(&o["start_time"])),
                // Spinner ticks are spin rewards, not parts a map's hitsounds set (as in oracle.rs).
                "Spinner" => expected.push(number(&o["end_time"])),
                "Slider" => expected.extend(o["nested"].as_array().unwrap().iter().map(|n| number(&n["start_time"]))),
                other => panic!("{name}: {other}"),
            }
        }
        let mut ours: Vec<f64> = cue_list(&h, &[], &[]).iter().map(|c| c.time().ms()).collect();
        for v in [&mut ours, &mut expected] {
            v.sort_by(f64::total_cmp);
            v.dedup();
        }
        assert_eq!(ours.len(), expected.len(), "{name}");
        for (a, b) in ours.iter().zip(&expected) {
            assert!((a - b).abs() <= 0.01, "{name}: cue at {a}, lazer {b}");
        }
    }
}

#[test]
fn the_cue_list_applies_the_played_volume_and_leaves_out_muted_and_unsoloed_lanes() {
    let file = read(CIRCLES.as_bytes());
    let mut h = dec(&file);
    let clap = std(SampleSet::Soft, Hitsound::Clap, 1);
    h.add_hit(MomentId(0), clap.clone());
    let cues = cue_list(&h, &[], &[]);
    assert_eq!(cues.len(), 3, "two hitnormals and the unsaved clap");
    assert!(cues.iter().all(|c| c.gain() == 0.05), "volume 3 plays at 5%");
    let normal = std(SampleSet::Soft, Hitsound::Normal, 1);
    assert_eq!(cue_list(&h, &[normal.clone()], &[]).len(), 1, "muted hitnormals");
    let soloed = cue_list(&h, &[], &[clap.clone()]);
    assert_eq!(soloed.len(), 1);
    assert_eq!(soloed[0].time(), SongTime::from_ms(1000.0));
    assert_eq!(soloed[0].sample(), &std(SampleSet::Soft, Hitsound::Clap, 0), "no file in the folder: the skin's clap plays");
}

// ---------------------------------------------------------------- compile

/// For every fifth moment of every osu! save-pack map, adds a clap or removes the clap it has,
/// compiles, reopens and decompiles: each edited moment reads back as edited at the saved volume
/// floor or above, unless it is listed as a save problem, and every other moment as before.
fn clap_round_trip(placement: Placement) {
    let (edited, _, _) = round_trip(placement, |m, h, i| {
        if i % 5 != 0 {
            return false;
        }
        let claps: Vec<_> = m.hits().filter(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Clap), .. })).cloned().collect();
        if !claps.is_empty() {
            claps.iter().for_each(|c| h.remove_hit(MomentId(i), c));
        } else {
            let base = m.hits().find(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Whistle | Hitsound::Finish), .. })).or_else(|| m.hits().find(|l| matches!(l, SampleIdentity::Standard { .. })));
            let Some(SampleIdentity::Standard { set, index, .. }) = base else { return false };
            h.add_hit(MomentId(i), SampleIdentity::Standard { set: *set, sound: Sound::Hit(Hitsound::Clap), index: *index });
        }
        true
    });
    assert!(edited > 1000, "only {edited} edits read back");
}

#[test]
fn slider_node_volumes_read_back_through_green_lines() {
    let (edited, green_lines, problems) = round_trip(Placement::GreenLines, |m, h, i| {
        if i % 4 != 0 || !matches!(m.place(), Place::SliderNode { .. }) {
            return false;
        }
        h.set_volume(MomentId(i), 77);
        true
    });
    assert!(edited > 100 && green_lines > 100, "{edited} edits, {green_lines} green lines");
    assert!(problems < edited, "{problems} problems");
}

/// Applies `edit` (given the decompiled moment, the hitsounds to edit and the moment's place in
/// the list; true when it edited) to every osu! save-pack map, compiles, reopens and decompiles.
/// Returns (edits read back, green lines added, save problems).
fn round_trip(placement: Placement, edit: impl Fn(&dawsu_hitsounding::Moment, &mut Hitsounds, usize) -> bool) -> (usize, usize, usize) {
    let (mut edited_total, mut green_lines, mut problem_total) = (0, 0, 0);
    for (name, file) in osu_pack() {
        let before = dec(&file);
        let mut h = before.clone();
        let mut edited = Vec::new();
        for (i, m) in before.moments().iter().enumerate() {
            if edit(m, &mut h, i) {
                edited.push(i);
            }
        }
        let (out, problems) = comp(&file, &h, placement);
        let summary = out.change_summary();
        assert_eq!(summary.red_lines_changed, 0, "{name}: invariant 4");
        assert_eq!(summary.green_lines_removed, 0, "{name}: invariant 5");
        assert_eq!((summary.hit_objects_added, summary.hit_objects_removed), (0, 0), "{name}: invariant 6");
        if placement == Placement::OnObjects {
            assert_eq!(summary.green_lines_added, 0, "{name}");
        }
        let after = dec(&reopen(&out));
        assert_eq!(after.moments().len(), before.moments().len(), "{name}");
        for (i, (b, a)) in before.moments().iter().zip(after.moments()).enumerate() {
            assert_eq!((spot(a.place()), a.time()), (spot(b.place()), b.time()), "{name}: moment {i}");
            let problem = problems.iter().any(|p| p.place == b.place() && p.time == b.time());
            if edited.contains(&i) && !problem {
                let want = h.moment(MomentId(i));
                assert_eq!(a.hits().collect::<Vec<_>>(), want.hits().collect::<Vec<_>>(), "{name}: edited moment {i} at {:?}", b.time());
                assert_eq!(a.volume(), saved_volume(want.volume()), "{name}: invariant 7 at moment {i}");
                edited_total += 1;
            } else {
                assert_eq!(a.hits().collect::<Vec<_>>(), b.hits().collect::<Vec<_>>(), "{name}: moment {i}");
                assert_eq!(a.volume(), b.volume(), "{name}: invariant 8 at moment {i}");
            }
        }
        for (b, a) in before.bodies().iter().zip(after.bodies()) {
            assert_eq!(body_key(a), body_key(b), "{name}: slider bodies are untouched");
        }
        green_lines += summary.green_lines_added;
        problem_total += problems.len();
    }
    eprintln!("{placement:?}: {edited_total} edits read back, {green_lines} green lines added, {problem_total} save problems");
    (edited_total, green_lines, problem_total)
}

#[test]
fn edited_moments_read_back_as_edited_with_green_line_placement() {
    clap_round_trip(Placement::GreenLines);
}

#[test]
fn edited_moments_read_back_as_edited_with_on_object_placement() {
    clap_round_trip(Placement::OnObjects);
}

#[test]
fn invariants_3_7_and_8_one_edit_changes_one_line_at_the_floor_and_leaves_the_quiet_moment_alone() {
    let file = read(CIRCLES.as_bytes());
    let mut h = dec(&file);
    assert_eq!(h.moment(MomentId(0)).volume(), 3);
    h.add_hit(MomentId(0), std(SampleSet::Soft, Hitsound::Clap, 1));
    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert!(problems.is_empty(), "{problems:?}");
    let summary = out.change_summary();
    assert_eq!(summary.lines.len(), 1, "{summary:?}");
    assert_eq!(summary.lines[0].1.as_deref(), Some("256,192,1000,1,0,0:0:0:3:"));
    assert_eq!(summary.lines[0].2.as_deref(), Some("256,192,1000,1,8,0:0:0:5:"));
    let after = dec(&reopen(&out));
    assert_eq!(after.moment(MomentId(0)).volume(), 5, "invariant 7 (R15)");
    assert_eq!(after.moment(MomentId(1)).volume(), 3, "invariant 8, rules invariant 3");
}

#[test]
fn a_moment_that_does_not_fit_the_native_combination_gets_a_mix_plan_and_keeps_its_line() {
    let file = read(CIRCLES.as_bytes());
    let mut h = dec(&file);
    h.add_hit(MomentId(0), std(SampleSet::Soft, Hitsound::Clap, 2));
    h.add_hit(MomentId(1), std(SampleSet::Soft, Hitsound::Whistle, 1));
    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].time, SongTime::from_ms(1000.0));
    let ProblemKind::NeedsSampleMix(plan) = &problems[0].kind else { panic!("{problems:?}") };
    assert_eq!(plan, &vec![(std(SampleSet::Soft, Hitsound::Normal, 1), 5), (std(SampleSet::Soft, Hitsound::Clap, 2), 5)]);
    let summary = out.change_summary();
    assert_eq!(summary.lines.len(), 1, "the rest is saved");
    assert_eq!(summary.lines[0].2.as_deref(), Some("256,192,2000,1,2,0:0:0:5:"));
}

#[test]
fn hits_with_no_object_are_kept_but_never_create_an_object() {
    let file = read(CIRCLES.as_bytes());
    let mut h = dec(&file);
    let m = h.add_moment(SongTime::from_ms(1500.0), 60);
    h.add_hit(m, std(SampleSet::Soft, Hitsound::Normal, 0));
    assert_eq!(cue_list(&h, &[], &[]).len(), 3, "the hit plays");
    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert_eq!(problems.iter().map(|p| &p.kind).collect::<Vec<_>>(), vec![&ProblemKind::NoObject]);
    assert!(out.change_summary().is_empty(), "invariant 6");
}

#[test]
fn a_slider_node_s_volume_goes_on_a_green_line_with_a_restoring_line() {
    let file = slider_map("100,100,1000,2,0,L|300:100,2,200,0|0|0,0:0|0:0|0:0,0:0:0:0:");
    let mut h = dec(&file);
    let tail = h.moments().iter().position(|m| m.place() == Place::SliderNode { object: m_object(&h), node: 2 }).unwrap();
    h.set_volume(MomentId(tail), 80);

    let (unchanged, problems) = comp(&file, &h, Placement::OnObjects);
    assert_eq!(problems.iter().map(|p| &p.kind).collect::<Vec<_>>(), vec![&ProblemKind::NeedsGreenLine]);
    assert!(unchanged.change_summary().is_empty());

    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert!(problems.is_empty(), "{problems:?}");
    let summary = out.change_summary();
    assert_eq!((summary.green_lines_added, summary.red_lines_changed, summary.hit_objects_changed), (2, 0, 0), "{summary:?}");
    let after = dec(&reopen(&out));
    for (b, a) in h.moments().iter().zip(after.moments()) {
        assert_eq!((a.time(), a.volume()), (b.time(), b.volume()));
    }
    assert_eq!(after.bodies().iter().map(body_key).collect::<Vec<_>>(), dec(&file).bodies().iter().map(body_key).collect::<Vec<_>>(), "the body keeps its volume and times");
}

/// A body without its object id, which changes when lines are added before it.
fn body_key(b: &dawsu_hitsounding::SliderBody) -> String {
    format!("{:?} {:?} {} {:?} {} {:?} {:?}", b.start(), b.end(), b.volume(), b.slide(), b.whistle(), b.ticks(), b.tick())
}

/// A place without its object id.
fn spot(p: Place) -> (u8, usize) {
    match p {
        Place::Circle(_) => (0, 0),
        Place::SliderNode { node, .. } => (1, node),
        Place::SpinnerEnd(_) => (2, 0),
        Place::NoObject => (3, 0),
    }
}

fn m_object(h: &Hitsounds) -> dawsu_hitsounding::ObjectId {
    match h.moments()[0].place() {
        Place::SliderNode { object, .. } => object,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_slider_body_whistle_compiles_back_and_leaves_the_nodes_alone() {
    for object in ["100,100,1000,2,0,L|300:100,2,200,0|0|0,0:0|0:0|0:0,0:0:0:0:", "100,100,1000,2,0,L|300:100,2,200"] {
        let file = slider_map(object);
        let mut h = dec(&file);
        assert!(!h.bodies()[0].whistle());
        h.set_body_whistle(0, true);
        let cues = cue_list(&h, &[], &[]);
        assert!(cues.iter().any(|c| c.sample() == &h.bodies()[0].whistle_lane() || matches!(c.sample(), SampleIdentity::Standard { sound: Sound::Slider(dawsu_foundation::SliderSound::Whistle), .. })));
        let (out, problems) = comp(&file, &h, Placement::GreenLines);
        assert!(problems.is_empty());
        let after = dec(&reopen(&out));
        assert!(after.bodies()[0].whistle(), "{object}");
        let before = dec(&file);
        for (b, a) in before.moments().iter().zip(after.moments()) {
            assert_eq!(a.hits().collect::<Vec<_>>(), b.hits().collect::<Vec<_>>(), "{object}: the nodes keep their sounds");
        }
        assert_eq!(out.change_summary().lines.len(), 1);
    }
}

// ---------------------------------------------------------------- cancel, progress and saving

#[test]
fn a_cancelled_decompile_or_compile_returns_cancelled_and_leaves_nothing_behind() {
    let scratch = Scratch::new("cancel");
    let path = scratch.path("map.osu");
    std::fs::write(&path, save_pack_bytes("owner/catalyst-impetus-std.osu")).unwrap();
    let file = read(&std::fs::read(&path).unwrap());

    let checks = Cell::new(0);
    let cancel_soon = || {
        checks.set(checks.get() + 1);
        checks.get() > 10
    };
    let mut reports = Vec::new();
    assert_eq!(decompile(&file, &[], &cancel_soon, &mut |done, total| reports.push((done, total))), Decompiled::Cancelled);
    assert_eq!(reports.len(), 9, "it stopped between hit objects");
    assert!(reports.iter().all(|(_, total)| *total == 1531));

    let mut h = dec(&file);
    h.add_hit(MomentId(0), std(SampleSet::Drum, Hitsound::Finish, 0));
    assert!(matches!(compile(&file, &h, Placement::GreenLines, &|| true, &mut |_, _| {}), Compiled::Cancelled));
    let left: Vec<_> = std::fs::read_dir(scratch.dir()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, vec!["map.osu"]);
    assert_eq!(std::fs::read(&path).unwrap(), save_pack_bytes("owner/catalyst-impetus-std.osu"));
}

#[test]
fn decompile_reports_progress_up_to_every_hit_object() {
    let file = read(&save_pack_bytes("owner/catalyst-impetus-std.osu"));
    let mut last = (0, 0);
    let _ = decompile(&file, &[], &|| false, &mut |done, total| last = (done, total));
    assert_eq!(last, (1531, 1531));
}

#[test]
fn a_save_with_no_changes_writes_nothing_and_makes_no_backup() {
    let scratch = Scratch::new("no-change-save");
    let path = scratch.path("map.osu");
    let original = save_pack_bytes("owner/catalyst-impetus-std.osu");
    std::fs::write(&path, &original).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let file = read(&std::fs::read(&path).unwrap());
    let (out, problems) = comp(&file, &dec(&file), Placement::GreenLines);
    assert!(problems.is_empty());
    let backups = scratch.path("backups");
    assert_eq!(out.save_over(&path, &BackupPolicy::new(backups.clone(), 5)).unwrap(), SaveOutcome::NothingToSave);
    assert!(!backups.exists(), "no backup folder");
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified);
}
