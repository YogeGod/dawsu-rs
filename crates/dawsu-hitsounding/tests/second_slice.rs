//! The second M2 slice (roadmap, Yoge 2026-11-15): the crate's error type, changing a slider
//! body's sample set and volume, and compiling hold notes and zero-length sliders.

use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, Sound, SongTime};
use dawsu_hitsounding::rules::{answer, Answer};
use dawsu_hitsounding::{compile, cue_list, decompile, Compiled, Decompiled, Error, Hitsounds, MomentId, Place, Placement, ProblemKind, SaveProblem};
use dawsu_osu_format::OsuFile;
use dawsu_test_support::save_pack_files;

fn read(bytes: &[u8]) -> OsuFile {
    OsuFile::read(bytes.to_vec()).file
}

fn map(objects: &str) -> OsuFile {
    read(format!("osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,2,1,60,1,0\r\n\r\n[HitObjects]\r\n{objects}\r\n").as_bytes())
}

fn dec(file: &OsuFile) -> Hitsounds {
    match decompile(file, &[], &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => panic!("{other:?}"),
    }
}

fn comp(file: &OsuFile, h: &Hitsounds, placement: Placement) -> (OsuFile, Vec<SaveProblem>) {
    match compile(file, h, placement, &|| false, &mut |_, _| {}) {
        Compiled::Done { file, problems } => (file, problems),
        Compiled::Cancelled => panic!("cancelled"),
    }
}

fn reopen(file: &OsuFile) -> OsuFile {
    read(&file.to_bytes())
}

fn set_of(identity: &SampleIdentity) -> SampleSet {
    match identity {
        SampleIdentity::Standard { set, .. } => *set,
        other => panic!("{other:?}"),
    }
}

fn hits(h: &Hitsounds) -> Vec<Vec<SampleIdentity>> {
    h.moments().iter().map(|m| m.hits().cloned().collect()).collect()
}

fn std(set: SampleSet, sound: Hitsound, index: u32) -> SampleIdentity {
    SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(index) }
}

// ---------------------------------------------------------------- the error type

#[test]
fn the_crate_error_holds_the_format_crate_s_error_unchanged() {
    let format = dawsu_osu_format::Error::Io { step: "reading the map", path: "map.osu".into(), source: std::io::Error::new(std::io::ErrorKind::NotFound, "gone") };
    let e: Error = format.into();
    assert!(matches!(&e, Error::Format(dawsu_osu_format::Error::Io { step: "reading the map", .. })));
    assert_eq!(e.to_string(), "reading the map failed for map.osu: gone");
    assert!(std::error::Error::source(&e).is_some());
}

// ---------------------------------------------------------------- slider bodies

#[test]
fn the_slider_whistle_plays_from_the_slider_s_addition_set() {
    // Own banks normal 1 (Normal), addition 2 (Soft): lazer renames the hitwhistle, whose bank is
    // the addition bank, to sliderwhistle.
    let file = map("100,100,1000,2,2,L|300:100,1,200,0|0,0:0|0:0,1:2:0:0:");
    let h = dec(&file);
    let body = &h.bodies()[0];
    assert!(body.whistle());
    assert_eq!(set_of(body.slide()), SampleSet::Normal);
    assert_eq!(set_of(&body.whistle_lane()), SampleSet::Soft);
    let Answer::Objects(objects) = answer(&file, &[]) else { unreachable!() };
    let lazer_whistle = objects[0].slider.as_ref().unwrap().sliding_samples.iter().find(|s| s.lazer.name == "sliderwhistle").unwrap();
    assert_eq!(lazer_whistle.lazer.bank, "soft");
}

#[test]
fn changing_a_slider_body_s_sample_set_reads_back_and_leaves_the_nodes_alone() {
    for object in [
        "100,100,1000,2,2,L|300:100,2,200,0|8|0,0:0|2:0|0:0,1:2:0:0:",
        "100,100,1000,2,0,L|300:100,2,200",
        "100,100,1000,2,0,L|300:100,2,200,0|0|0,1:0|1:0|1:0,0:0:0:0:",
    ] {
        let file = map(object);
        let before = dec(&file);
        let mut h = before.clone();
        h.set_body_set(0, SampleSet::Drum);
        assert_eq!(set_of(h.bodies()[0].slide()), SampleSet::Drum, "the model follows the edit");
        let (out, problems) = comp(&file, &h, Placement::GreenLines);
        assert!(problems.is_empty(), "{object}: {problems:?}");
        let summary = out.change_summary();
        assert_eq!((summary.lines.len(), summary.hit_objects_changed, summary.green_lines_added), (1, 1, 0), "{object}: {summary:?}");
        let after = dec(&reopen(&out));
        let (b, a) = (&before.bodies()[0], &after.bodies()[0]);
        assert_eq!(set_of(a.slide()), SampleSet::Drum, "{object}");
        assert_eq!(set_of(a.tick()), SampleSet::Drum, "{object}: ticks follow the body");
        assert_eq!(set_of(&a.whistle_lane()), set_of(&b.whistle_lane()), "{object}: the whistle keeps its set");
        assert_eq!((a.volume(), a.whistle(), a.ticks()), (b.volume(), b.whistle(), b.ticks()), "{object}");
        assert_eq!(hits(&after), hits(&before), "{object}: every node keeps its sounds");
    }
}

fn volumes(h: &Hitsounds) -> Vec<i32> {
    h.moments().iter().map(|m| m.volume()).collect()
}

#[test]
fn changing_a_slider_body_s_volume_moves_every_sound_at_its_head_together() {
    let file = map("100,100,1000,2,0,L|300:100,2,200,8|0|0,0:0|0:0|0:0,0:0:0:0:");
    let mut h = dec(&file);
    let head = MomentId(h.moments().iter().position(|m| matches!(m.place(), Place::SliderNode { node: 0, .. })).unwrap());
    let lane = |h: &Hitsounds, sound: Hitsound| {
        h.moment(head).hits().find(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(s), .. } if *s == sound)).unwrap().clone()
    };
    let (normal, clap) = (lane(&h, Hitsound::Normal), lane(&h, Hitsound::Clap));
    h.set_hit_volume(head, &clap, 30);
    assert_eq!((h.moment(head).volume(), h.moment(head).hit_volume(&normal), h.moment(head).hit_volume(&clap)), (60, 60, 30));
    h.set_body_volume(0, 80);
    let moment = h.moment(head);
    assert_eq!((moment.volume(), moment.hit_volume(&normal), moment.hit_volume(&clap)), (80, 80, 50), "the clap stays 30 below");

    // The body plays at the head's own volume, so the head's hits move by the body's change even
    // when no hit at the head plays at that volume.
    h.remove_hit(head, &normal);
    h.set_body_volume(0, 60);
    let moment = h.moment(head);
    assert_eq!((moment.volume(), moment.hit_volume(&clap)), (60, 30), "the clap stays 30 below the body");
}

#[test]
fn changing_a_slider_body_s_volume_changes_its_head_too() {
    // A body reads its volume at start + 6 ms and the head at start + 5 ms, so one green line at
    // the slider's start sets both, with the old values back 7 ms later (Yoge, 2026-11-15).
    let file = map("100,100,1000,2,0,L|300:100,2,200,0|0|0,0:0|0:0|0:0,0:0:0:0:");
    let before = dec(&file);
    let mut h = before.clone();
    h.set_body_volume(0, 80);
    let head = h.moments().iter().position(|m| matches!(m.place(), Place::SliderNode { node: 0, .. })).unwrap();
    assert_eq!(h.moments()[head].volume(), 80, "the head follows the body in the model");
    assert!(cue_list(&h, &[], &[]).iter().any(|c| c.loop_end().is_some() && c.gain() == 0.8), "the cue list plays the unsaved volume");

    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert!(problems.is_empty(), "{problems:?}");
    let summary = out.change_summary();
    assert_eq!((summary.green_lines_added, summary.hit_objects_changed, summary.red_lines_changed), (2, 0, 0), "{summary:?}");
    let added: Vec<&str> = summary.lines.iter().filter_map(|l| l.2.as_deref()).collect();
    assert_eq!(added, vec!["1000,-100,4,2,1,80,0,0", "1007,-100,4,2,1,60,0,0"]);
    let after = dec(&reopen(&out));
    assert_eq!(after.bodies()[0].volume(), 80);
    assert_eq!(volumes(&after), volumes(&h), "the head reads 80, the repeat and the tail keep 60");
    assert_eq!(hits(&after), hits(&before));

    let (_, problems) = comp(&file, &h, Placement::OnObjects);
    assert!(problems.iter().all(|p| p.kind == ProblemKind::NeedsGreenLine) && !problems.is_empty(), "{problems:?}");
}

#[test]
fn a_body_volume_change_is_refused_when_another_part_reads_inside_its_window() {
    // A circle at 1001 ms looks up at 1006 ms, inside the window from 1000 to the restoring line
    // at 1007, so it would read the new volume: the change is refused.
    let file = map("100,100,1000,2,0,L|300:100,1,200,0|0,0:0|0:0,0:0:0:0:\r\n256,192,1001,1,0,0:0:0:0:");
    let mut h = dec(&file);
    h.set_body_volume(0, 80);
    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert!(!problems.is_empty() && problems.iter().all(|p| p.kind == ProblemKind::LookupWindowTaken), "{problems:?}");
    assert!(out.change_summary().is_empty());
}

#[test]
fn slider_body_sets_read_back_over_the_save_pack() {
    let maps = save_pack_files().into_iter().map(|(name, path)| (name, read(&std::fs::read(path).unwrap()))).collect::<Vec<_>>();
    let edited = body_set_round_trip(&maps);
    assert!(edited > 100, "only {edited} bodies edited");
}

/// The local corpus (not committed): set `DAWSU_CORPUS` to its folder.
#[test]
fn slider_body_sets_read_back_over_the_local_corpus() {
    let Ok(corpus) = std::env::var("DAWSU_CORPUS") else {
        eprintln!("skipped: DAWSU_CORPUS is not set");
        return;
    };
    let maps: Vec<(String, OsuFile)> = std::fs::read_dir(&corpus)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "osu"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), read(&std::fs::read(&p).unwrap())))
        .collect();
    let edited = body_set_round_trip(&maps);
    eprintln!("corpus: {edited} slider bodies changed set and read back");
}

#[test]
fn slider_body_volumes_read_back_over_the_save_pack() {
    let maps = save_pack_files().into_iter().map(|(name, path)| (name, read(&std::fs::read(path).unwrap()))).collect::<Vec<_>>();
    let (edited, refused) = body_volume_round_trip(&maps);
    assert!(edited > 100, "only {edited} bodies read back ({refused} refused)");
}

#[test]
fn slider_body_volumes_read_back_over_the_local_corpus() {
    let Ok(corpus) = std::env::var("DAWSU_CORPUS") else {
        eprintln!("skipped: DAWSU_CORPUS is not set");
        return;
    };
    let maps: Vec<(String, OsuFile)> = std::fs::read_dir(&corpus)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "osu"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), read(&std::fs::read(&p).unwrap())))
        .collect();
    let (edited, refused) = body_volume_round_trip(&maps);
    eprintln!("corpus: {edited} slider body volumes read back with their heads, {refused} refused");
}

/// Sets every third slider body's volume to 77: each body that is not a save problem reads back
/// at 77 with its head, and every other moment and body as before. Returns (read back, refused).
fn body_volume_round_trip(maps: &[(String, OsuFile)]) -> (usize, usize) {
    let (mut edited, mut refused) = (0, 0);
    for (name, file) in maps {
        if !matches!(answer(file, &[]), Answer::Objects(_)) {
            continue;
        }
        let before = dec(file);
        let mut h = before.clone();
        let wanted: Vec<usize> = (0..before.bodies().len()).filter(|i| i % 3 == 0).collect();
        for &i in &wanted {
            h.set_body_volume(i, 77);
        }
        let (out, problems) = comp(file, &h, Placement::GreenLines);
        assert!(problems.iter().all(|p| matches!(p.kind, ProblemKind::LookupWindowTaken | ProblemKind::BodyVolumeNeedsHead)), "{name}: {problems:?}");
        let summary = out.change_summary();
        assert_eq!((summary.red_lines_changed, summary.green_lines_removed, summary.hit_objects_changed), (0, 0, 0), "{name}");
        let after = dec(&reopen(&out));
        assert_eq!(hits(&after), hits(&before), "{name}");
        let saved = |b: &dawsu_hitsounding::SliderBody| !problems.iter().any(|p| p.place == Place::SliderNode { object: b.object(), node: 0 } && p.time == b.start());
        for (i, (b, a)) in before.bodies().iter().zip(after.bodies()).enumerate() {
            let want = if wanted.contains(&i) && saved(b) { 77 } else { b.volume() };
            assert_eq!(a.volume(), want, "{name}: body {i}");
        }
        for (i, (m, a)) in h.moments().iter().zip(after.moments()).enumerate() {
            let refused_here = problems.iter().any(|p| p.place == m.place() && p.time == m.time());
            let want = if refused_here { before.moments()[i].volume() } else { m.volume() };
            assert_eq!(a.volume(), want, "{name}: moment {i}");
        }
        let n_refused = wanted.iter().filter(|&&i| !saved(&before.bodies()[i])).count();
        refused += n_refused;
        edited += wanted.len() - n_refused;
    }
    (edited, refused)
}

/// Changes the set of every third slider body of every osu! map, compiles, reopens and
/// decompiles: the edited bodies read back with the new set and everything else as before.
fn body_set_round_trip(maps: &[(String, OsuFile)]) -> usize {
    let mut edited = 0;
    for (name, file) in maps {
        if !matches!(answer(file, &[]), Answer::Objects(_)) {
            continue;
        }
        let before = dec(&file);
        let mut h = before.clone();
        let mut wanted = Vec::new();
        for (i, b) in before.bodies().iter().enumerate().filter(|(i, _)| i % 3 == 0) {
            let next = match set_of(b.slide()) {
                SampleSet::Normal => SampleSet::Soft,
                SampleSet::Soft => SampleSet::Drum,
                SampleSet::Drum => SampleSet::Normal,
            };
            h.set_body_set(i, next);
            wanted.push((i, next));
        }
        let (out, problems) = comp(file, &h, Placement::GreenLines);
        assert!(problems.is_empty(), "{name}: {problems:?}");
        let summary = out.change_summary();
        assert_eq!((summary.red_lines_changed, summary.green_lines_added, summary.hit_objects_added), (0, 0, 0), "{name}");
        let after = dec(&reopen(&out));
        assert_eq!(hits(&after), hits(&before), "{name}: every moment keeps its sounds");
        for (i, (b, a)) in before.bodies().iter().zip(after.bodies()).enumerate() {
            let want = wanted.iter().find(|(j, _)| *j == i).map_or(set_of(b.slide()), |(_, s)| *s);
            assert_eq!(set_of(a.slide()), want, "{name}: body {i}");
            assert_eq!(set_of(&a.whistle_lane()), set_of(&b.whistle_lane()), "{name}: body {i} whistle");
            assert_eq!((a.volume(), a.whistle()), (b.volume(), b.whistle()), "{name}: body {i}");
        }
        edited += wanted.len();
    }
    edited
}

// ---------------------------------------------------------------- holds and zero-length sliders

#[test]
fn a_hold_note_in_an_osu_map_compiles_like_a_spinner() {
    // lazer converts anything with a duration in an osu! map to a spinner; a hold keeps its
    // samples after its end time in field 5.
    let file = map("256,192,1000,128,0,1500:0:0:0:0:");
    let mut h = dec(&file);
    assert_eq!(h.moments()[0].time(), SongTime::from_ms(1500.0));
    assert!(matches!(h.moments()[0].place(), Place::SpinnerEnd(_)));
    h.add_hit(MomentId(0), std(SampleSet::Soft, Hitsound::Clap, 1));
    h.set_volume(MomentId(0), 70);
    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert!(problems.is_empty(), "{problems:?}");
    let summary = out.change_summary();
    assert_eq!(summary.lines.len(), 1);
    assert_eq!(summary.lines[0].2.as_deref(), Some("256,192,1000,128,8,1500:0:0:0:70:"));
    let after = dec(&reopen(&out));
    assert_eq!(after.moments()[0].hits().collect::<Vec<_>>(), h.moments()[0].hits().collect::<Vec<_>>());
    assert_eq!(after.moments()[0].volume(), 70);
}

#[test]
fn a_zero_length_slider_compiles_its_first_and_last_node() {
    // lazer reads a slider of length 0 as one span, keeping only the first and the last node's
    // sounds; the last of four nodes in the file is the tail.
    let file = map("100,100,1000,2,0,L|100:100,3,0,0|0|0|0,0:0|0:0|0:0|0:0,0:0:0:0:");
    let mut h = dec(&file);
    let tail = h.moments().iter().position(|m| matches!(m.place(), Place::SliderNode { node: 1, .. })).expect("the tail");
    let set = set_of(h.moments()[tail].hits().next().unwrap());
    h.add_hit(MomentId(tail), std(set, Hitsound::Clap, 1));
    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert!(problems.is_empty(), "{problems:?}");
    let summary = out.change_summary();
    assert_eq!(summary.lines[0].2.as_deref(), Some("100,100,1000,2,0,L|100:100,3,0,0|0|0|8,0:0|0:0|0:0|0:0,0:0:0:0:"));
    let after = dec(&reopen(&out));
    assert_eq!(hits(&after), hits(&h));
}

#[test]
fn a_long_node_entry_keeps_its_own_index_and_volume_through_compile() {
    // Node 0's entry holds index 3 and volume 40 of its own (lazer reads them: the oracle case
    // slider-node-long-entries).
    let file = map("100,100,1000,2,0,L|300:100,2,200,0|0|0,2:0:3:40|0:0|0:0,0:0:0:0:");
    let mut h = dec(&file);
    let head = h.moments().iter().position(|m| matches!(m.place(), Place::SliderNode { node: 0, .. })).unwrap();
    assert_eq!(h.moments()[head].volume(), 40);
    assert!(h.moments()[head].hits().all(|l| matches!(l, SampleIdentity::Standard { index, .. } if index.get() == 3)));

    // A clap at the same index and volume changes the sounds only; the entry keeps its parts.
    let mut clap = h.clone();
    clap.add_hit(MomentId(head), std(SampleSet::Soft, Hitsound::Clap, 3));
    let (out, problems) = comp(&file, &clap, Placement::GreenLines);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(out.change_summary().lines[0].2.as_deref(), Some("100,100,1000,2,0,L|300:100,2,200,8|0|0,2:0:3:40|0:0|0:0,0:0:0:0:"));
    assert_eq!(hits(&dec(&reopen(&out))), hits(&clap));

    // A new volume would need a green line the entry overrides: a save problem, line untouched.
    h.set_volume(MomentId(head), 90);
    let (out, problems) = comp(&file, &h, Placement::GreenLines);
    assert_eq!(problems.iter().map(|p| &p.kind).collect::<Vec<_>>(), vec![&ProblemKind::LineRefused]);
    assert!(out.change_summary().is_empty());
}

#[test]
fn slider_bodies_are_their_own_lanes_with_the_slider_sounds() {
    let file = map("100,100,1000,2,2,L|300:100,1,200,0|0,0:0|0:0,0:0:0:0:");
    let h = dec(&file);
    let body = &h.bodies()[0];
    assert!(matches!(body.slide(), SampleIdentity::Standard { sound: Sound::Slider(SliderSound::Slide), .. }));
    assert!(matches!(body.whistle_lane(), SampleIdentity::Standard { sound: Sound::Slider(SliderSound::Whistle), .. }));
    assert!(h.lane(&body.whistle_lane()).is_some());
}

#[test]
fn independent_whistle_set_and_auto_keep_other_slider_sounds() {
    for (object, initial) in [
        ("100,100,1000,2,2,L|300:100,2,200", None),
        ("100,100,1000,2,2,L|300:100,2,200,0|2|0,0:0:1:77:|2:0|0:0,2:2:0:0:", Some(SampleSet::Soft)),
    ] {
        let file = map(object);
        let before = dec(&file);
        assert_eq!(before.bodies()[0].whistle_set(), initial);
        let mut edited = before.clone();
        for choice in [Some(SampleSet::Drum), Some(SampleSet::Soft), None] {
            edited.set_body_whistle_set(0, choice);
            let (out, problems) = comp(&file, &edited, Placement::GreenLines);
            assert!(problems.is_empty(), "{object}: {problems:?}");
            let after = dec(&reopen(&out));
            let (a, b) = (&after.bodies()[0], &before.bodies()[0]);
            assert_eq!(a.whistle_set(), choice);
            assert_eq!(set_of(&a.whistle_lane()), choice.unwrap_or(SampleSet::Soft));
            assert_eq!((a.slide(), a.tick(), a.volume()), (b.slide(), b.tick(), b.volume()));
            assert_eq!(hits(&after), hits(&before), "every node keeps its sounds");
            assert_eq!(volumes(&after), volumes(&before), "every node keeps its volume");
            assert_eq!(out.timing_points().iter().map(|p| out.line_text(p.line())).collect::<Vec<_>>(),
                       file.timing_points().iter().map(|p| file.line_text(p.line())).collect::<Vec<_>>());
            let line = out.line_text(out.hit_objects()[0].line()).unwrap();
            if object.contains("0:0:1:77:") {
                assert!(line.contains(",0|2|0,0:0:1:77:|2:0|0:0,"), "{line}");
            }
        }
    }
}

#[test]
fn compile_keeps_equal_effective_gains_native_and_plans_a_mix_for_distinct_layers() {
    let source = map("256,192,1000,1,8,0:0:0:60:\r\n256,192,1500,1,8,0:0:0:60:");
    let mut edited = dec(&source);
    let clap = |moment: usize, hits: &Hitsounds| {
        hits.moments()[moment].hits().find(|lane| matches!(lane,
            SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Clap), .. }
        )).unwrap().clone()
    };
    let first_clap = clap(0, &edited);
    let second_clap = clap(1, &edited);
    edited.set_hit_volume(MomentId(0), &first_clap, 35);
    edited.set_volume(MomentId(1), 2);
    edited.set_hit_volume(MomentId(1), &second_clap, 5);

    let (encoded, problems) = comp(&source, &edited, Placement::OnObjects);
    let [SaveProblem { kind: ProblemKind::NeedsSampleMix(layers), .. }] = problems.as_slice() else {
        panic!("only the first object should require a mix: {problems:?}");
    };
    assert!(layers.contains(&(first_clap, 35)));
    assert_eq!(layers.iter().map(|(_, volume)| *volume).sum::<i32>(), 95);
    let decoded = dec(&reopen(&encoded));
    assert_eq!(decoded.moments()[1].volume(), 5);
    assert_eq!(decoded.moments()[1].hits().count(), 2);
}

// ---------------------------------------------------------------- Save's merge per role (Yoge 2026-11-20)

use dawsu_hitsounding::compile::{merge_roles, RoleFile};

fn role(set: SampleSet, role: Hitsound, layers: &[(SampleIdentity, i32)]) -> RoleFile {
    RoleFile { set, role, layers: layers.to_vec() }
}

#[test]
fn a_hitnormal_with_two_finishes_keeps_the_hitnormal_and_merges_the_finishes() {
    let normal = std(SampleSet::Normal, Hitsound::Normal, 0);
    let drum = std(SampleSet::Drum, Hitsound::Finish, 0);
    let soft = std(SampleSet::Soft, Hitsound::Finish, 0);
    let merge = merge_roles(&[(normal.clone(), 80), (drum.clone(), 80), (soft.clone(), 40)]);
    assert_eq!(merge.volume, 80);
    // Each set is used once; the drum finish is the loudest, so it names the finish file.
    assert_eq!(merge.files, [
        role(SampleSet::Normal, Hitsound::Normal, &[(normal, 80)]),
        role(SampleSet::Drum, Hitsound::Finish, &[(drum, 80), (soft, 40)]),
    ]);
    assert_eq!(merge.hits(5, SampleSet::Soft), [std(SampleSet::Normal, Hitsound::Normal, 5), std(SampleSet::Drum, Hitsound::Finish, 5)]);
}

#[test]
fn the_addition_set_is_the_one_most_additions_use_then_the_loudest_then_soft_drum_normal() {
    let add = |set, sound, volume| (std(set, sound, 0), volume);
    let normal = add(SampleSet::Normal, Hitsound::Normal, 100);
    let pick = |layers: &[(SampleIdentity, i32)]| merge_roles(layers).files.iter().skip(1).map(|f| f.set).collect::<Vec<_>>();
    // Two drum sounds win over one louder soft one.
    assert_eq!(pick(&[normal.clone(), add(SampleSet::Drum, Hitsound::Clap, 30), add(SampleSet::Drum, Hitsound::Whistle, 30), add(SampleSet::Soft, Hitsound::Clap, 90)]),
        [SampleSet::Drum, SampleSet::Drum]);
    // One each and the same volume: soft, then drum, then normal.
    assert_eq!(pick(&[normal.clone(), add(SampleSet::Normal, Hitsound::Clap, 50), add(SampleSet::Drum, Hitsound::Clap, 50)]), [SampleSet::Drum]);
    assert_eq!(pick(&[normal.clone(), add(SampleSet::Drum, Hitsound::Clap, 50), add(SampleSet::Soft, Hitsound::Clap, 50)]), [SampleSet::Soft]);
}

#[test]
fn two_hitnormals_are_mixed_into_one_hitnormal_file() {
    let normal = std(SampleSet::Normal, Hitsound::Normal, 2);
    let soft = std(SampleSet::Soft, Hitsound::Normal, 3);
    let merge = merge_roles(&[(normal.clone(), 80), (soft.clone(), 40)]);
    assert_eq!(merge.files, [role(SampleSet::Normal, Hitsound::Normal, &[(normal, 80), (soft, 40)])]);
    assert_eq!(merge.hits(4, SampleSet::Drum), [std(SampleSet::Normal, Hitsound::Normal, 4)]);
}

#[test]
fn a_moment_without_a_hitnormal_gets_no_hitnormal_file_and_sample_files_join_the_hitnormal() {
    let clap = std(SampleSet::Drum, Hitsound::Clap, 2);
    let whistle = std(SampleSet::Drum, Hitsound::Whistle, 3);
    let merge = merge_roles(&[(clap.clone(), 60), (whistle.clone(), 60)]);
    // Yoge 2026-11-20: the player's skin plays the hitnormal, in the set the object has today.
    assert_eq!(merge.files, [
        role(SampleSet::Drum, Hitsound::Whistle, &[(whistle, 60)]),
        role(SampleSet::Drum, Hitsound::Clap, &[(clap.clone(), 60)]),
    ]);
    assert!(!merge.has_hitnormal());
    assert_eq!(merge.hits(6, SampleSet::Soft), [
        std(SampleSet::Soft, Hitsound::Normal, 6), std(SampleSet::Drum, Hitsound::Whistle, 6), std(SampleSet::Drum, Hitsound::Clap, 6),
    ]);
    let kick = SampleIdentity::File("kick.wav".into());
    let merge = merge_roles(&[(kick.clone(), 70), (clap.clone(), 30)]);
    assert_eq!(merge.files, [
        role(SampleSet::Drum, Hitsound::Normal, &[(kick, 70)]),
        role(SampleSet::Drum, Hitsound::Clap, &[(clap, 30)]),
    ]);
}

#[test]
fn merged_volumes_never_go_below_five_percent() {
    let normal = std(SampleSet::Normal, Hitsound::Normal, 0);
    let clap = std(SampleSet::Normal, Hitsound::Clap, 0);
    let merge = merge_roles(&[(normal.clone(), 3), (clap.clone(), 1)]);
    assert_eq!(merge.volume, 5);
    assert_eq!(merge.files[1], role(SampleSet::Normal, Hitsound::Clap, &[(clap, 5)]));
}

#[test]
fn a_merged_moment_compiles_on_a_circle_with_one_index_and_reads_back() {
    let source = map("256,192,1000,1,0,1:0:2:80:");
    let mut edited = dec(&source);
    let merge = merge_roles(&[
        (std(SampleSet::Normal, Hitsound::Normal, 2), 80),
        (std(SampleSet::Drum, Hitsound::Finish, 3), 80),
        (std(SampleSet::Soft, Hitsound::Finish, 4), 40),
    ]);
    let hits: Vec<_> = merge.hits(5, SampleSet::Normal).into_iter().map(|lane| (lane, merge.volume)).collect();
    edited.set_moment_state(MomentId(0), merge.volume, hits);
    let (encoded, problems) = comp(&source, &edited, Placement::OnObjects);
    assert!(problems.is_empty(), "{problems:?}");
    let text = String::from_utf8(encoded.to_bytes()).unwrap();
    assert!(text.contains("256,192,1000,1,4,1:3:5:80:\r\n"), "{text}");
    let decoded = dec(&reopen(&encoded));
    let mut back: Vec<_> = decoded.moments()[0].hits().cloned().collect();
    back.sort();
    assert_eq!(back, [std(SampleSet::Normal, Hitsound::Normal, 5), std(SampleSet::Drum, Hitsound::Finish, 5)]);
    assert_eq!(decoded.moments()[0].volume(), 80);
}
