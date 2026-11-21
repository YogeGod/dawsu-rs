//! Copy hitsounds (Generate and Copy plan, Task 3; design doc "Generate and Copy hitsounds",
//! Responsibilities 7-12, invariants 3, 5 and 6). Old test names, where a test follows one, are
//! from the old app's `HitsoundCopierTests.cs` and `SimultaneousCopyTests.cs` at c81774e.

use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_hitsounding::copy::{copy_hitsounds, CopyCollisionPolicy, CopyMode, CopyOptions, CopyOutcome, TargetOutcome, TargetPlan};
use dawsu_hitsounding::rules::{answer, Answer};
use dawsu_hitsounding::{decompile, Decompiled, Hitsounds, MomentId, ProblemKind};
use dawsu_osu_format::OsuFile;
use dawsu_test_support::{save_pack_bytes, save_pack_files};

fn read(bytes: &[u8]) -> OsuFile {
    OsuFile::read(bytes.to_vec()).file
}

fn map(objects: &str) -> OsuFile {
    map_with("osu file format v14", "0,500,4,2,1,60,1,0", objects)
}

fn map_with(header: &str, timing: &str, objects: &str) -> OsuFile {
    read(format!("{header}\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[Events]\r\n\r\n[TimingPoints]\r\n{timing}\r\n\r\n[HitObjects]\r\n{objects}\r\n").as_bytes())
}

fn dec(file: &OsuFile) -> Hitsounds {
    match decompile(file, &[], &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => panic!("{other:?}"),
    }
}

fn run(source: &OsuFile, targets: &[OsuFile], options: &CopyOptions) -> Vec<TargetOutcome> {
    match copy_hitsounds(&dec(source), source, targets, &[], options, &|| false, &mut |_, _| {}) {
        CopyOutcome::Targets(t) => t,
        other => panic!("{other:?}"),
    }
}

fn plan(source: &OsuFile, target: &OsuFile, options: &CopyOptions) -> TargetPlan {
    match run(source, std::slice::from_ref(target), options).remove(0) {
        TargetOutcome::Planned(p) => *p,
        other => panic!("{other:?}"),
    }
}

fn std(set: SampleSet, sound: Hitsound, index: u32) -> SampleIdentity {
    SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(index) }
}

/// The same hitsounds with each moment a bare hitnormal of its first standard hit's set and index
/// (a moment with only sample files keeps them), so the clear can be written on object lines.
fn cleared(h: &Hitsounds) -> Hitsounds {
    let mut out = h.clone();
    for i in 0..out.moments().len() {
        let Some((set, index)) = out.moments()[i].hits().find_map(|l| match l {
            SampleIdentity::Standard { set, index, .. } => Some((*set, *index)),
            _ => None,
        }) else {
            continue;
        };
        let hits: Vec<SampleIdentity> = out.moments()[i].hits().cloned().collect();
        for l in &hits {
            out.remove_hit(MomentId(i), l);
        }
        out.add_hit(MomentId(i), SampleIdentity::Standard { set, sound: Sound::Hit(Hitsound::Normal), index });
    }
    out
}

/// Each moment's (time, hits, volume) after reading the planned file back.
fn moments(file: &OsuFile) -> Vec<(f64, Vec<SampleIdentity>, i32)> {
    dec(&read(&file.to_bytes())).moments().iter().map(|m| (m.time().ms(), m.hits().cloned().collect(), m.volume())).collect()
}

fn with_clap(file: &OsuFile, at: usize) -> Hitsounds {
    let mut h = dec(file);
    h.add_hit(MomentId(at), std(SampleSet::Soft, Hitsound::Clap, 1));
    h
}

const ONLY_DEFINED: CopyOptions = CopyOptions {
    start_index: 100,
    never_silent_hitnormals: false,
    mode: CopyMode::OverwriteOnlyDefined,
    leniency: 5.0,
    collision_policy: CopyCollisionPolicy::Merge,
    copy_hitsounds: true,
    copy_body_hitsounds: true,
    copy_to_slider_ticks: false,
    copy_to_slider_slides: false,
    copy_sample_sets: true,
    copy_volumes: true,
    always_preserve_5_volume: true,
    copy_storyboarded_samples: false,
    ignore_hitsound_satisfied_samples: true,
    ignore_whenever_hitsound: false,
    mute_slider_ends: false,
    beat_divisors: Vec::new(),
    muted_divisors: Vec::new(),
    min_length: 0.5,
    muted_index: None,
    muted_sample_set: None,
};

// ---------------------------------------------------------------- the identity and the invariants

#[test]
fn copying_a_difficulty_onto_itself_changes_no_byte() {
    for (name, path) in save_pack_files() {
        let file = read(&std::fs::read(&path).unwrap());
        if !matches!(answer(&file, &[]), Answer::Objects(_)) {
            continue;
        }
        for options in [CopyOptions::default(), ONLY_DEFINED] {
            let p = plan(&file, &file, &options);
            assert_eq!(p.file.to_bytes(), file.original_bytes(), "{name} ({:?})", options.mode);
            assert!(p.problems.is_empty() && p.dropped.is_empty() && p.changed == 0, "{name}: {:?}", p.problems);
        }
    }
}

#[test]
fn a_copied_clap_reads_back_and_only_its_line_changes() {
    let source_file = map("256,192,1000,1,0,0:0:0:0:\r\n256,192,2000,1,0,0:0:0:0:");
    let target = map("100,100,1002,1,0,0:0:0:0:\r\n100,100,2000,1,0,0:0:0:0:");
    let source = with_clap(&source_file, 0);
    let p = match copy_hitsounds(&source, &source_file, std::slice::from_ref(&target), &[], &ONLY_DEFINED, &|| false, &mut |_, _| {}) {
        CopyOutcome::Targets(mut t) => match t.remove(0) {
            TargetOutcome::Planned(p) => *p,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    let summary = p.file.change_summary();
    assert_eq!(summary.lines.len(), 1, "{summary:?}");
    assert_eq!(summary.lines[0].2.as_deref(), Some("100,100,1002,1,8,0:0:0:0:"));
    assert!(moments(&p.file)[0].1.contains(&std(SampleSet::Soft, Hitsound::Clap, 1)));
    // Invariant 3, and compile's: no object, red line or green line removed or moved.
    assert_eq!((summary.hit_objects_added, summary.hit_objects_removed, summary.red_lines_changed, summary.green_lines_removed), (0, 0, 0, 0));
}

#[test]
fn invariant_6_the_same_inputs_give_the_same_plan() {
    let source = read(&save_pack_bytes("owner/catalyst-hitsound-diff.osu"));
    let target = read(&save_pack_bytes("owner/catalyst-impetus-std.osu"));
    let (a, b) = (plan(&source, &target, &CopyOptions::default()), plan(&source, &target, &CopyOptions::default()));
    assert_eq!(a.file.to_bytes(), b.file.to_bytes());
    assert_eq!((a.problems, a.dropped, a.matched, a.changed), (b.problems, b.dropped, b.matched, b.changed));
}

#[test]
fn invariant_5_the_change_summary_lists_exactly_the_lines_that_differ() {
    let source = read(&save_pack_bytes("owner/catalyst-hitsound-diff.osu"));
    let target = read(&save_pack_bytes("owner/catalyst-impetus-std.osu"));
    let p = plan(&source, &target, &CopyOptions::default());
    let (before, after) = (target.original_bytes(), p.file.to_bytes());
    let count = |b: &[u8]| b.split(|c| *c == b'\n').count();
    let changed_in_place = p.file.change_summary().lines.iter().filter(|l| l.1.is_some() && l.2.is_some()).count();
    let added = p.file.change_summary().lines.iter().filter(|l| l.1.is_none()).count();
    assert_eq!(count(&after), count(before) + added, "only additions change the line count");
    assert!(changed_in_place + added > 0, "the hitsound diff changes the map");
}

// ---------------------------------------------------------------- the options

#[test]
fn overwrite_everything_clears_unmatched_moments_and_only_defined_keeps_them() {
    let source = map("256,192,1000,1,0,0:0:0:0:");
    let target = map("100,100,1000,1,0,0:0:0:0:\r\n100,100,3000,1,8,0:0:0:0:");
    let cleared = moments(&plan(&source, &target, &CopyOptions::default()).file);
    assert_eq!(cleared[1].1, vec![std(SampleSet::Soft, Hitsound::Normal, 1)], "a hitnormal in the source's state there");
    let kept = moments(&plan(&source, &target, &ONLY_DEFINED).file);
    assert!(kept[1].1.contains(&std(SampleSet::Soft, Hitsound::Clap, 1)), "the target's clap stays");
}

#[test]
fn without_copying_sample_sets_the_target_s_set_and_index_are_kept() {
    let source = map_with("osu file format v14", "0,500,4,3,2,60,1,0", "256,192,1000,1,8,0:0:0:0:");
    let target = map("100,100,1000,1,0,0:0:0:0:");
    let options = CopyOptions { copy_sample_sets: false, ..ONLY_DEFINED };
    let got = moments(&plan(&source, &target, &options).file);
    assert_eq!(got[0].1, vec![std(SampleSet::Soft, Hitsound::Normal, 1), std(SampleSet::Soft, Hitsound::Clap, 1)], "the clap in the target's Soft 1, not the source's Drum 2");
}

#[test]
fn a_five_percent_target_volume_stays_only_when_asked() {
    // A_five_percent_target_volume_stays_only_when_asked (old test).
    let source = map("256,192,1000,1,0,0:0:0:80:");
    let target = map("100,100,1000,1,0,0:0:0:5:");
    assert_eq!(moments(&plan(&source, &target, &ONLY_DEFINED).file)[0].2, 5);
    let options = CopyOptions { always_preserve_5_volume: false, ..ONLY_DEFINED };
    assert_eq!(moments(&plan(&source, &target, &options).file)[0].2, 80);
    let options = CopyOptions { copy_volumes: false, ..ONLY_DEFINED };
    assert_eq!(moments(&plan(&source, &target, &options).file)[0].2, 5, "volumes not copied: the target's");
}

#[test]
fn a_source_moment_with_no_target_within_the_leniency_is_dropped_and_listed() {
    let source = map("256,192,1000,1,0,0:0:0:0:\r\n256,192,5000,1,8,0:0:0:0:");
    let target = map("100,100,1000,1,0,0:0:0:0:");
    let p = plan(&source, &target, &ONLY_DEFINED);
    assert_eq!(p.dropped.iter().map(|t| t.ms()).collect::<Vec<_>>(), vec![5000.0]);
    assert_eq!(p.matched, 1);
}

#[test]
fn simultaneous_source_moments_at_one_volume_are_written_together_and_at_two_need_a_mix() {
    // A circle with a whistle and, at the same time, a lone clap with no object, both at 60:
    // together one hitnormal, a whistle and a clap, which one object holds.
    let source_file = map("256,192,1000,1,2,0:0:0:0:");
    let mut source = dec(&source_file);
    let loose = source.add_moment(source.moments()[0].time(), 60);
    source.add_hit(loose, std(SampleSet::Soft, Hitsound::Clap, 1));
    let target = map("100,100,1000,1,0,0:0:0:0:");
    let p = match copy_hitsounds(&source, &source_file, std::slice::from_ref(&target), &[], &ONLY_DEFINED, &|| false, &mut |_, _| {}) {
        CopyOutcome::Targets(mut t) => match t.remove(0) {
            TargetOutcome::Planned(p) => *p,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert!(p.problems.is_empty(), "{:?}", p.problems);
    let hits = &moments(&p.file)[0].1;
    assert!(hits.contains(&std(SampleSet::Soft, Hitsound::Whistle, 1)) && hits.contains(&std(SampleSet::Soft, Hitsound::Clap, 1)), "{hits:?}");

    // Two circles: each has a hitnormal, so the pile-up repeats a lane, and at 40 and 80 it has
    // two volumes: a mix either way.
    let source = map("256,192,1000,1,2,0:0:0:40:\r\n300,192,1000,1,8,0:0:0:80:");
    let p = plan(&source, &target, &ONLY_DEFINED);
    let ProblemKind::NeedsSampleMix(layers) = &p.problems[0].kind else { panic!("{:?}", p.problems) };
    let volumes: Vec<i32> = layers.iter().map(|l| l.1).collect();
    assert!(volumes.contains(&40) && volumes.contains(&80), "each layer keeps its own volume: {layers:?}");
    assert_eq!(p.file.to_bytes(), target.original_bytes(), "the moment keeps its sound");

    // Keeping_target_indexes_reports... (old test): without copying sample sets the target is refused.
    let options = CopyOptions { copy_sample_sets: false, ..ONLY_DEFINED };
    assert!(matches!(run(&source, std::slice::from_ref(&target), &options)[0], TargetOutcome::Refused(_)));
}

#[test]
fn nearby_hits_in_leniency_window_merge_by_default_and_later_wins_is_optional() {
    // Kick (circle with finish) at 999 ms + loose clap at 1001 ms -> 1000 ms target circle.
    let source_file = map("256,192,999,1,4,0:0:0:0:");
    let mut source = dec(&source_file);
    let clap = source.add_moment(dawsu_foundation::SongTime::from_ms(1001.0), 60);
    source.add_hit(clap, std(SampleSet::Soft, Hitsound::Clap, 1));
    let target = map("100,100,1000,1,0,0:0:0:0:");

    // Default Merge/Union keeps both 999 ms and 1001 ms sounds together with zero collisions.
    let merged = match copy_hitsounds(&source, &source_file, std::slice::from_ref(&target), &[], &ONLY_DEFINED, &|| false, &mut |_, _| {}) {
        CopyOutcome::Targets(mut t) => match t.remove(0) {
            TargetOutcome::Planned(p) => *p,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert!(merged.problems.is_empty(), "{:?}", merged.problems);
    assert!(merged.collisions.is_empty(), "{:?}", merged.collisions);
    let hits = &moments(&merged.file)[0].1;
    assert!(hits.contains(&std(SampleSet::Soft, Hitsound::Finish, 1)) && hits.contains(&std(SampleSet::Soft, Hitsound::Clap, 1)), "{hits:?}");

    // Optional LaterWins replaces 999 ms with 1001 ms and records the collision.
    source.add_hit(clap, std(SampleSet::Soft, Hitsound::Normal, 1));
    let later_opts = CopyOptions { collision_policy: CopyCollisionPolicy::LaterWins, ..ONLY_DEFINED };
    let later = match copy_hitsounds(&source, &source_file, std::slice::from_ref(&target), &[], &later_opts, &|| false, &mut |_, _| {}) {
        CopyOutcome::Targets(mut t) => match t.remove(0) {
            TargetOutcome::Planned(p) => *p,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    assert_eq!(later.collisions.len(), 1);
    assert_eq!(later.merged, 0);
    let later_hits = &moments(&later.file)[0].1;
    assert!(later_hits.contains(&std(SampleSet::Soft, Hitsound::Clap, 1)) && !later_hits.contains(&std(SampleSet::Soft, Hitsound::Finish, 1)), "{later_hits:?}");
}

#[test]
fn nearby_ordinary_circles_merge_into_a_plain_union_even_without_copying_sample_sets() {
    // Two ordinary circles at 999 ms (hitnormal + finish) and 1001 ms (hitnormal + clap), both at 60%.
    let source = map("256,192,999,1,4,0:0:0:0:\r\n300,192,1001,1,8,0:0:0:0:");
    let target = map("100,100,1000,1,0,0:0:0:0:");

    // Identical hitnormals collapse to one instead of forcing a doubled sample mix.
    let merged = plan(&source, &target, &ONLY_DEFINED);
    assert!(merged.problems.is_empty(), "{:?}", merged.problems);
    assert!(merged.collisions.is_empty(), "{:?}", merged.collisions);
    assert_eq!(merged.merged, 1);
    let hits = &moments(&merged.file)[0].1;
    assert_eq!(
        hits,
        &vec![
            std(SampleSet::Soft, Hitsound::Normal, 1),
            std(SampleSet::Soft, Hitsound::Finish, 1),
            std(SampleSet::Soft, Hitsound::Clap, 1),
        ],
    );

    // With copy_sample_sets off, the plain union retargets to the target's sample set instead of refusing the difficulty.
    let no_sets = CopyOptions { copy_sample_sets: false, ..ONLY_DEFINED };
    let retargeted = plan(&source, &target, &no_sets);
    assert!(retargeted.problems.is_empty(), "{:?}", retargeted.problems);
    assert_eq!(retargeted.merged, 1);
    let retargeted_hits = &moments(&retargeted.file)[0].1;
    assert_eq!(retargeted_hits, hits);
}

#[test]
fn matched_slider_bodies_take_the_source_whistle_and_set_and_unmatched_ones_lose_the_whistle() {
    // A_matched_slider_takes_the_source_sliders_body_sounds (old test).
    let source = map("100,100,1000,2,2,L|300:100,1,200,0|0,0:0|0:0,3:0:0:0:");
    let target = map("100,100,1000,2,0,L|300:100,1,200,0|0,0:0|0:0,0:0:0:0:");
    let got = dec(&read(&plan(&source, &target, &ONLY_DEFINED).file.to_bytes()));
    assert!(got.bodies()[0].whistle());
    assert_eq!(got.bodies()[0].set(), Some(SampleSet::Drum));
    let nodes: Vec<Vec<SampleIdentity>> = got.moments().iter().map(|m| m.hits().cloned().collect()).collect();
    assert!(nodes.iter().all(|h| !h.iter().any(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Whistle), .. }))), "the nodes keep their own sounds: {nodes:?}");

    // Overwrite_everything_clears_the_body_of_a_slider_the_source_has_nothing_for (old test).
    let source = map("400,100,9000,1,0,0:0:0:0:");
    let target = map("100,100,1000,2,2,L|300:100,1,200,0|0,0:0|0:0,0:0:0:0:");
    let got = dec(&read(&plan(&source, &target, &CopyOptions::default()).file.to_bytes()));
    assert!(!got.bodies()[0].whistle());
}

#[test]
fn slider_ends_the_source_has_nothing_for_are_muted_on_a_muted_divisor() {
    // A slider of one span from 1000 ending on a 1/4 line (beat 500 ms): 1000 + 375 = 1375.
    let source = map("256,192,1000,1,0,0:0:0:0:");
    let length = 1.4 * 100.0 * 0.75; // 0.75 beats at slider velocity 1
    let target = map(&format!("100,100,1000,2,0,L|300:100,1,{length},0|0,0:0|0:0,0:0:0:0:"));
    let options = CopyOptions { mute_slider_ends: true, ..CopyOptions::default() };
    let p = plan(&source, &target, &options);
    assert_eq!(p.muted, 1, "{:?}", p.problems);
    let tail = moments(&p.file).into_iter().last().unwrap();
    assert_eq!(tail.2, 5);
}

#[test]
fn storyboard_samples_are_copied_once_and_overwrite_everything_clears_the_target_s_first() {
    let mut source = map("256,192,1000,1,0,0:0:0:0:");
    source.insert_storyboard_sample("Sample,3000,0,\"whoosh.wav\",70");
    let source = read(&source.to_bytes());
    let mut target = map("100,100,1000,1,0,0:0:0:0:");
    target.insert_storyboard_sample("Sample,4000,0,\"old.wav\",50");
    let target = read(&target.to_bytes());

    let options = CopyOptions { copy_storyboarded_samples: true, ..CopyOptions::default() };
    let got = plan(&source, &target, &options).file.storyboard_samples();
    assert_eq!(got.iter().map(|s| (s.time, s.path.as_str(), s.volume)).collect::<Vec<_>>(), vec![(3000.0, "whoosh.wav", 70.0)]);

    let options = CopyOptions { copy_storyboarded_samples: true, ..ONLY_DEFINED };
    let got = plan(&source, &target, &options).file.storyboard_samples();
    assert_eq!(got.len(), 2, "the target's own stays");
    let twice = plan(&source, &read(&plan(&source, &target, &options).file.to_bytes()), &options).file.storyboard_samples();
    assert_eq!(twice.len(), 2, "never doubled");
}

#[test]
fn file_times_match_across_format_versions() {
    // Plan decision D: a v4 source (24 ms added to its song times) and a v14 target with equal
    // file times match.
    let source = map_with("osu file format v4", "0,500,4,2,1,60,1,0", "256,192,1000,1,8,0:0:0:0:");
    let target = map("100,100,1000,1,0,0:0:0:0:");
    let p = plan(&source, &target, &ONLY_DEFINED);
    assert_eq!(p.matched, 1);
    assert!(moments(&p.file)[0].1.iter().any(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Clap), .. })));
}

// ---------------------------------------------------------------- outcomes

#[test]
fn a_mania_target_is_unsupported_and_bad_options_are_refused() {
    let source = read(&save_pack_bytes("owner/catalyst-impetus-std.osu"));
    let mania = read(&save_pack_bytes("synthetic/mania-4k-holds-keysounds.osu"));
    assert!(matches!(run(&source, &[mania], &CopyOptions::default())[0], TargetOutcome::UnsupportedMode { mode: 3 }));
    // A_negative_or_endless_leniency_is_refused (old test).
    for leniency in [-1.0, f64::INFINITY, f64::NAN] {
        let options = CopyOptions { leniency, ..CopyOptions::default() };
        assert!(matches!(copy_hitsounds(&dec(&source), &source, &[], &[], &options, &|| false, &mut |_, _| {}), CopyOutcome::InvalidOptions(_)));
    }
}

#[test]
fn progress_counts_objects_across_all_copy_targets() {
    let a = map("256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,0,0:0:0:0:");
    let b = map("256,192,2000,1,0,0:0:0:0:");
    let mut reports = Vec::new();
    let result = copy_hitsounds(&dec(&a), &a, &[a.clone(), b], &[], &CopyOptions::default(),
        &|| false, &mut |done, total| reports.push((done, total)));
    assert!(matches!(result, CopyOutcome::Targets(_)));
    assert_eq!(reports, [(1, 3), (2, 3), (3, 3)]);
}

#[test]
fn a_cancel_stops_the_whole_copy() {
    let source = read(&save_pack_bytes("owner/catalyst-impetus-std.osu"));
    let targets = vec![source.clone(), source.clone()];
    let checks = std::cell::Cell::new(0);
    let cancel = || {
        checks.set(checks.get() + 1);
        checks.get() > 3
    };
    let mut reports = Vec::new();
    let out = copy_hitsounds(&dec(&source), &source, &targets, &[], &CopyOptions::default(), &cancel, &mut |d, t| reports.push((d, t)));
    assert!(matches!(out, CopyOutcome::Cancelled));
}

// ---------------------------------------------------------------- timing (the plan's check before Task 4)

/// Set `DAWSU_TIME_MAP` to a map's path to time Copy and Generate on it (run in release).
#[test]
fn timing_on_one_map() {
    let Ok(path) = std::env::var("DAWSU_TIME_MAP") else {
        eprintln!("skipped: DAWSU_TIME_MAP is not set");
        return;
    };
    let file = read(&std::fs::read(&path).unwrap());
    let started = std::time::Instant::now();
    let h = dec(&file);
    eprintln!("decompile: {:?}, {} moments", started.elapsed(), h.moments().len());
    let started = std::time::Instant::now();
    let p = plan(&file, &file, &CopyOptions::default());
    eprintln!("copy onto itself: {:?} ({} changed)", started.elapsed(), p.changed);
    let cleared = cleared(&h);
    let started = std::time::Instant::now();
    let cleared_file = match dawsu_hitsounding::compile(&file, &cleared, dawsu_hitsounding::Placement::OnObjects, &|| false, &mut |_, _| {}) {
        dawsu_hitsounding::Compiled::Done { file, problems } => {
            eprintln!("cleared-copy problems: {}, first {:?}", problems.len(), problems.first());
            read(&file.to_bytes())
        }
        _ => unreachable!(),
    };
    eprintln!("compile of a cleared copy: {:?}", started.elapsed());
    let started = std::time::Instant::now();
    let p = plan(&file, &cleared_file, &CopyOptions::default());
    eprintln!("copy onto the cleared copy: {:?} ({} changed)", started.elapsed(), p.changed);
    let started = std::time::Instant::now();
    let g = dawsu_hitsounding::generate::generate(&h, &file, "map.osu", &[], &|| false, &mut |_, _| {});
    eprintln!("generate: {:?} ({})", started.elapsed(), matches!(g, dawsu_hitsounding::generate::GenerateOutcome::Planned { .. }));
}

// ---------------------------------------------------------------- the local corpus

/// The local corpus (not committed): set `DAWSU_CORPUS` to its folder. Every osu!-mode map copied
/// onto itself changes no byte, and copied onto a cleared copy of itself reads back with its hits,
/// except the moments listed as problems.
#[test]
fn the_local_corpus_copies_onto_itself_and_onto_a_cleared_copy() {
    let Ok(corpus) = std::env::var("DAWSU_CORPUS") else {
        eprintln!("skipped: DAWSU_CORPUS is not set");
        return;
    };
    let (mut maps, mut restored, mut problem_moments) = (0, 0usize, 0usize);
    let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
    let mut paths: Vec<_> = std::fs::read_dir(&corpus).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "osu")).collect();
    paths.sort();
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let file = read(&std::fs::read(&path).unwrap());
        if !matches!(answer(&file, &[]), Answer::Objects(_)) {
            continue;
        }
        let p = plan(&file, &file, &CopyOptions::default());
        assert_eq!(p.file.to_bytes(), file.original_bytes(), "{name}: self-copy");

        let cleared = cleared(&dec(&file));
        let cleared_file = match dawsu_hitsounding::compile(&file, &cleared, dawsu_hitsounding::Placement::OnObjects, &|| false, &mut |_, _| {}) {
            dawsu_hitsounding::Compiled::Done { file, .. } => read(&file.to_bytes()),
            _ => unreachable!(),
        };
        let p = plan(&file, &cleared_file, &CopyOptions::default());
        for pr in &p.problems {
            *kinds.entry(format!("{:?}", pr.kind).split('(').next().unwrap().to_string()).or_default() += 1;
        }
        problem_moments += p.problems.len();
        let (want, got) = (dec(&file), dec(&read(&p.file.to_bytes())));
        assert_eq!(want.moments().len(), got.moments().len(), "{name}");
        for (w, g) in want.moments().iter().zip(got.moments()) {
            if p.problems.iter().any(|pr| pr.time == g.time()) {
                continue;
            }
            assert_eq!(g.hits().collect::<Vec<_>>(), w.hits().collect::<Vec<_>>(), "{name} at {:?}", w.time());
            restored += 1;
        }
        maps += 1;
    }
    eprintln!("corpus: {maps} maps copied onto themselves unchanged; {restored} moments restored onto cleared copies, {problem_moments} problems {kinds:?}");
}

#[test]
fn unmatched_sources_take_each_tick_once_then_use_slide_time_and_keep_native_bytes() {
    use dawsu_foundation::SliderSound;

    let source = map_with("osu file format v4", "0,500,4,2,1,60,1,0", concat!(
        "256,192,1497,1,8,0:0:0:40:\r\n",
        "256,192,1503,1,0,0:0:0:55:\r\n",
        "256,192,1800,1,0,0:0:0:70:",
    ));
    let target = map(concat!(
        "100,100,1000,2,0,L|300:100,1,200,0|0,0:0|0:0,0:0:0:0:\r\n",
        "100,100,1000,2,0,L|300:100,1,200,0|0,0:0|0:0,0:0:0:0:",
    ));
    let options = CopyOptions { copy_to_slider_ticks: true, copy_to_slider_slides: true, ..ONLY_DEFINED };
    let result = plan(&source, &target, &options);
    assert_eq!(result.file.to_bytes(), target.to_bytes());
    assert_eq!(result.dropped.iter().map(|time| time.ms()).collect::<Vec<_>>(), [1824.0]);
    assert_eq!(result.slider_placements.len(), 2);
    let (tick, slide) = (&result.slider_placements[0], &result.slider_placements[1]);
    assert_eq!((tick.role, tick.time.ms(), tick.volume), (SliderSound::Tick, 1500.0, 40));
    assert_eq!((slide.role, slide.time.ms(), slide.volume), (SliderSound::Slide, 1503.0, 55));
    assert_eq!(tick.layers.len(), 2);
    assert!(tick.layers.iter().all(|(_, gain)| *gain == 100));
    assert!(plan(&source, &target, &CopyOptions { copy_sample_sets: false, ..options }).slider_placements.is_empty());
}

#[test]
fn simultaneous_tick_layers_keep_individual_gains_and_the_existing_five_percent_policy() {
    use dawsu_foundation::SliderSound;

    let source = map("256,192,1500,1,0,0:0:0:30:\r\n256,192,1500,1,8,0:0:0:80:");
    let target = map_with("osu file format v14", "0,500,4,2,1,5,1,0", "100,100,1000,2,0,L|300:100,1,200");
    let options = CopyOptions { copy_to_slider_ticks: true, ..ONLY_DEFINED };
    let result = plan(&source, &target, &options);
    let [placement] = result.slider_placements.as_slice() else { panic!("expected one tick event") };
    assert_eq!((placement.role, placement.time.ms(), placement.volume), (SliderSound::Tick, 1500.0, 5));
    assert_eq!(placement.layers.iter().map(|(_, gain)| *gain).collect::<Vec<_>>(), [30, 80, 80]);
    assert!(result.dropped.is_empty());
    let result = plan(&source, &target, &CopyOptions { copy_volumes: false, ..options });
    let placement = &result.slider_placements[0];
    assert_eq!(placement.volume, 5);
    assert!(placement.layers.iter().all(|(_, gain)| *gain == 100));
}

#[test]
fn copy_mixes_independent_gains_when_enabled_and_keeps_the_target_volume_policies() {
    let source = map("256,192,1000,1,8,0:0:0:80:");
    let mut edited = dec(&source);
    let clap = std(SampleSet::Soft, Hitsound::Clap, 1);
    edited.set_hit_volume(MomentId(0), &clap, 35);
    let target = map("256,192,1000,1,0,0:0:0:50:");
    let floor = map("256,192,1000,1,0,0:0:0:5:");
    let make_plan = |target: &OsuFile, options: &CopyOptions| {
        match copy_hitsounds(&edited, &source, &[target.clone()], &[], options, &|| false, &mut |_, _| {}) {
            CopyOutcome::Targets(mut targets) => match targets.remove(0) {
                TargetOutcome::Planned(p) => p,
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    };

    let copied = make_plan(&target, &ONLY_DEFINED);
    let [problem] = copied.problems.as_slice() else { panic!("expected one mix: {:?}", copied.problems) };
    let ProblemKind::NeedsSampleMix(layers) = &problem.kind else { panic!("{problem:?}") };
    assert!(layers.contains(&(clap, 35)));
    assert!(layers.contains(&(std(SampleSet::Soft, Hitsound::Normal, 1), 80)));

    let unscaled = make_plan(&target, &CopyOptions { copy_volumes: false, ..ONLY_DEFINED });
    assert!(unscaled.problems.is_empty());
    assert_eq!(moments(&unscaled.file)[0].2, 50);
    let preserved = make_plan(&floor, &ONLY_DEFINED);
    assert!(preserved.problems.is_empty());
    assert_eq!(moments(&preserved.file)[0].2, 5);

    let slider = map("100,100,500,2,0,L|300:100,1,200");
    let timed = make_plan(&slider, &CopyOptions { copy_to_slider_ticks: true, ..ONLY_DEFINED });
    let [tick] = timed.slider_placements.as_slice() else { panic!("expected one timed tick sample") };
    assert_eq!(tick.time.ms(), 1000.0);
    assert_eq!(tick.volume, 100);
    assert_eq!(tick.layers.iter().map(|(_, gain)| *gain).collect::<Vec<_>>(), [80, 35]);
}
