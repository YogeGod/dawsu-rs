//! New hitsound diff and Generate hitsound diff (Generate and Copy plan, Tasks 2 and 4; design
//! doc "Generate and Copy hitsounds").

use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_hitsounding::generate::{file_name_for, generate, new_hitsound_diff, GenerateOutcome, NewDiff, HITSOUNDS_VERSION};
use dawsu_hitsounding::lazer::control_points::offset_for;
use dawsu_hitsounding::rules::{answer, Answer};
use dawsu_hitsounding::{decompile, Decompiled, Hitsounds, MomentId, ProblemKind, SaveProblem};
use dawsu_osu_format::{OsuFile, SaveOutcome};
use dawsu_test_support::{save_pack_bytes, save_pack_files, Scratch};
use std::collections::{BTreeMap, BTreeSet};

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

// ---------------------------------------------------------------- Generate hitsound diff (Task 4)

fn dec(file: &OsuFile) -> Hitsounds {
    match decompile(file, &[], &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => panic!("{other:?}"),
    }
}

fn map(timing: &str, objects: &str) -> OsuFile {
    read(format!("osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Metadata]\r\nTitle:t\r\nArtist:a\r\nCreator:c\r\nVersion:Hard\r\nBeatmapID:5\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n{timing}\r\n\r\n[HitObjects]\r\n{objects}\r\n").as_bytes())
}

fn std(set: SampleSet, sound: Hitsound, index: u32) -> SampleIdentity {
    SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(index) }
}

fn generated(h: &Hitsounds, source: &OsuFile) -> (OsuFile, Vec<SaveProblem>) {
    match generate(h, source, "map.osu", &[], &|| false, &mut |_, _| {}) {
        GenerateOutcome::Planned { file, problems, .. } => (file, problems),
        other => panic!("{other:?}"),
    }
}

/// Generates from `h`, checks invariant 10 and that every group is read back from the plan at its
/// rounded time or is a save problem of a named kind, and returns (groups read back, problems by kind).
fn check(name: &str, source: &OsuFile, h: &Hitsounds) -> (usize, BTreeMap<String, usize>) {
    let (file, problems) = generated(h, source);
    assert_eq!(kept_lines(&file), kept_lines(source), "{name}: invariant 10");
    assert_eq!(file.has_bom(), source.has_bom(), "{name}");
    assert_eq!(file.value("Metadata", "Version").as_deref(), Some(HITSOUNDS_VERSION), "{name}");
    assert_eq!(file.value("Metadata", "BeatmapID").as_deref(), Some("0"), "{name}");

    let offset = offset_for(source.format_version());
    let key = |ms: f64| (ms - offset).round_ties_even() as i64;
    let mut groups: BTreeMap<i64, Vec<(SampleIdentity, i32)>> = BTreeMap::new();
    for m in h.moments().iter().filter(|m| m.hits().next().is_some()) {
        groups.entry(key(m.time().ms())).or_default().extend(m.hits().map(|l| (l.clone(), m.volume())));
    }
    let back = dec(&read(&file.to_bytes()));
    assert_eq!(back.moments().len(), groups.len(), "{name}: one circle per group");
    let (mut native, mut kinds) = (0, BTreeMap::new());
    for ((t, layers), m) in groups.iter().zip(back.moments()) {
        assert_eq!(key(m.time().ms()), *t, "{name}: the circle is at the group's rounded time");
        match problems.iter().find(|p| key(p.time.ms()) == *t) {
            Some(p) => {
                assert!(matches!(p.kind, ProblemKind::NeedsSampleMix(_) | ProblemKind::NeedsGreenLine), "{name} at {t}: {:?}", p.kind);
                *kinds.entry(format!("{:?}", p.kind).split('(').next().unwrap().to_string()).or_insert(0) += 1;
            }
            None => {
                let want: BTreeSet<_> = layers.iter().map(|l| l.0.clone()).collect();
                assert_eq!(m.hits().cloned().collect::<BTreeSet<_>>(), want, "{name} at {t}");
                assert_eq!(m.volume(), layers[0].1, "{name} at {t}");
                native += 1;
            }
        }
    }
    assert!(problems.iter().all(|p| groups.contains_key(&key(p.time.ms()))), "{name}: no problem outside a group");
    (native, kinds)
}

#[test]
fn invariant_10_and_every_group_read_back_or_named_on_the_save_pack() {
    for (name, path) in save_pack_files() {
        let source = read(&std::fs::read(&path).unwrap());
        if !matches!(answer(&source, &[]), Answer::Objects(_)) {
            continue;
        }
        check(&name, &source, &dec(&source));
    }
}

#[test]
fn an_added_clap_is_generated() {
    let source = map("0,500,4,1,0,70,1,0", "256,192,1000,1,0,0:0:0:0:\r\n256,192,1500,1,0,0:0:0:0:");
    let mut h = dec(&source);
    h.add_hit(MomentId(1), std(SampleSet::Normal, Hitsound::Clap, 0));
    let (native, kinds) = check("added clap", &source, &h);
    assert_eq!((native, kinds.len()), (2, 0));
    let back = dec(&read(&generated(&h, &source).0.to_bytes()));
    assert!(back.moments()[1].hits().any(|l| *l == std(SampleSet::Normal, Hitsound::Clap, 0)));
}

#[test]
fn an_index_the_green_line_does_not_have_needs_a_green_line() {
    let source = map("0,500,4,1,2,70,1,0", "256,192,1000,1,0,0:0:0:0:");
    // Index 0 on an object line means "the timing line's index", so index 0 under a line at
    // index 2 can only come from a green line.
    let mut h = dec(&source);
    h.remove_hit(MomentId(0), &std(SampleSet::Normal, Hitsound::Normal, 2));
    h.add_hit(MomentId(0), std(SampleSet::Normal, Hitsound::Normal, 0));
    h.add_hit(MomentId(0), std(SampleSet::Normal, Hitsound::Clap, 0));
    let (_, problems) = generated(&h, &source);
    assert!(matches!(problems.as_slice(), [p] if p.kind == ProblemKind::NeedsGreenLine), "{problems:?}");
}

#[test]
fn two_volumes_at_one_rounded_time_need_a_mix_and_the_circle_stays_bare() {
    let source = map("0,500,4,1,0,70,1,0", "256,192,1000,1,0,0:0:0:60:\r\n100,100,1000.4,1,2,0:0:0:40:");
    let (file, problems) = generated(&dec(&source), &source);
    let [p] = problems.as_slice() else { panic!("{problems:?}") };
    let ProblemKind::NeedsSampleMix(layers) = &p.kind else { panic!("{p:?}") };
    let volumes: BTreeSet<i32> = layers.iter().map(|l| l.1).collect();
    assert_eq!(volumes, BTreeSet::from([40, 60]));
    let circle = file.hit_objects()[0].line();
    assert_eq!(file.line_text(circle).as_deref(), Some("256,192,1000,1,0,0:0:0:0:"));
}

#[test]
fn a_taken_hitsounds_name_or_version_is_refused_and_a_cancel_stops() {
    let source = map("0,500,4,1,0,70,1,0", "256,192,1000,1,0,0:0:0:0:");
    let h = dec(&source);
    let name = file_name_for(&source, "map.osu", HITSOUNDS_VERSION);
    assert_eq!(name, "a - t (c) [Hitsounds].osu");
    let run = |folder: &[(String, Option<String>)], cancel: &dyn Fn() -> bool| generate(&h, &source, "map.osu", folder, cancel, &mut |_, _| {});
    assert!(matches!(run(&[(name.to_uppercase(), None)], &|| false), GenerateOutcome::NameTaken));
    assert!(matches!(run(&[("x.osu".into(), Some("hitsounds".into()))], &|| false), GenerateOutcome::VersionTaken));
    assert!(matches!(run(&[], &|| true), GenerateOutcome::Cancelled));
}

/// The local corpus (not committed): set `DAWSU_CORPUS` to its folder. Every osu!-mode map's
/// groups read back from its plan or are save problems of a named kind.
#[test]
fn the_local_corpus_generates_every_group_or_names_its_problem() {
    let Ok(corpus) = std::env::var("DAWSU_CORPUS") else {
        eprintln!("skipped: DAWSU_CORPUS is not set");
        return;
    };
    let (mut maps, mut native) = (0, 0);
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut paths: Vec<_> = std::fs::read_dir(&corpus).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "osu")).collect();
    paths.sort();
    for path in paths {
        let source = read(&std::fs::read(&path).unwrap());
        if !matches!(answer(&source, &[]), Answer::Objects(_)) {
            continue;
        }
        let (n, k) = check(&path.file_name().unwrap().to_string_lossy(), &source, &dec(&source));
        native += n;
        for (kind, count) in k {
            *kinds.entry(kind).or_insert(0) += count;
        }
        maps += 1;
    }
    eprintln!("corpus: {maps} maps generated; {native} groups read back, problems {kinds:?}");
}
#[test]
fn generate_offers_a_mix_for_two_lanes_with_independent_gains() {
    let source = map("0,500,4,2,1,60,1,0", "256,192,1000,1,8,0:0:0:60:");
    let mut edited = dec(&source);
    let clap = std(SampleSet::Soft, Hitsound::Clap, 1);
    edited.set_hit_volume(MomentId(0), &clap, 35);

    let (_, problems) = generated(&edited, &source);
    let [SaveProblem { kind: ProblemKind::NeedsSampleMix(layers), .. }] = problems.as_slice() else {
        panic!("expected a generated mix: {problems:?}");
    };
    assert!(layers.contains(&(clap, 35)));
    assert!(layers.contains(&(std(SampleSet::Soft, Hitsound::Normal, 1), 60)));
}
