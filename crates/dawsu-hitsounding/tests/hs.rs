//! HS mode in compile (hitsound domain design doc Responsibility 14, invariant 6; the
//! last-four-parts plan, part B). Old cases are from the old app's `HsModeTests.cs` and
//! `HsSaveTests.cs` at c81774e.

use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_hitsounding::hs::{compile_hs, CompiledHs};
use dawsu_hitsounding::{compile, decompile, Compiled, Decompiled, Hitsounds, MomentId, Placement, ProblemKind};
use dawsu_osu_format::OsuFile;

fn read(bytes: &[u8]) -> OsuFile {
    OsuFile::read(bytes.to_vec()).file
}

fn map(version: u32) -> OsuFile {
    read(format!("osu file format v{version}\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n").as_bytes())
}

fn dec(file: &OsuFile) -> Hitsounds {
    match decompile(file, &[], &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => panic!("{other:?}"),
    }
}

fn hit(sound: Hitsound) -> SampleIdentity {
    SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(sound), index: CustomIndex::new(0) }
}

fn done(c: CompiledHs) -> (OsuFile, Vec<dawsu_hitsounding::SaveProblem>, usize) {
    match c {
        CompiledHs::Done { file, problems, created } => (file, problems, created.len()),
        CompiledHs::Cancelled => panic!("cancelled"),
    }
}

/// Each moment's (time, sorted hits, volume).
fn moments(h: &Hitsounds) -> Vec<(f64, Vec<SampleIdentity>, i32)> {
    h.moments()
        .iter()
        .map(|m| {
            let mut hits: Vec<SampleIdentity> = m.hits().cloned().collect();
            hits.sort();
            (m.time().ms(), hits, m.volume())
        })
        .collect()
}

#[test]
fn moments_without_an_object_get_centred_circles_and_other_edits_still_land() {
    let file = map(14);
    let mut h = dec(&file);
    let before = moments(&h);
    h.add_hit(MomentId(0), hit(Hitsound::Clap));
    h.set_body_whistle(0, true);
    let a = h.add_moment(SongTime::from_ms(3000.4), 80);
    h.add_hit(a, hit(Hitsound::Normal));
    h.add_hit(a, hit(Hitsound::Finish));
    let b = h.add_moment(SongTime::from_ms(4000.0), 60);
    h.add_hit(b, hit(Hitsound::Normal));

    let (out, problems, created) = done(compile_hs(&file, &h, Placement::GreenLines, &|| false, &mut |_, _| {}));
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(created, 2);
    let texts: Vec<String> = out.hit_objects().iter().map(|o| out.line_text(o.line()).unwrap()).collect();
    assert_eq!(texts[2..].iter().map(|t| t.split(',').take(4).collect::<Vec<_>>().join(",")).collect::<Vec<_>>(), ["256,192,3000,1", "256,192,4000,1"], "centred circles in time order");

    let back = dec(&read(&out.to_bytes()));
    let got = moments(&back);
    assert_eq!(got[0].1, vec![hit(Hitsound::Normal), hit(Hitsound::Clap)], "the circle's edit");
    assert!(back.bodies()[0].whistle(), "the body's edit");
    assert_eq!(&got[1..before.len()], &before[1..], "every other moment as it was");
    assert_eq!(got[before.len()..], [(3000.0, vec![hit(Hitsound::Normal), hit(Hitsound::Finish)], 80), (4000.0, vec![hit(Hitsound::Normal)], 60)]);
}

#[test]
fn outside_hs_mode_compile_makes_no_object() {
    let file = map(14);
    let mut h = dec(&file);
    let a = h.add_moment(SongTime::from_ms(3000.0), 80);
    h.add_hit(a, hit(Hitsound::Normal));
    let Compiled::Done { file: out, problems } = compile(&file, &h, Placement::GreenLines, &|| false, &mut |_, _| {}) else { panic!() };
    assert_eq!(out.hit_objects().len(), 2, "invariant 6");
    assert!(problems.iter().any(|p| p.kind == ProblemKind::NoObject));
}

#[test]
fn moments_within_one_millisecond_share_a_circle_and_two_volumes_need_a_mix() {
    let file = map(14);
    let mut h = dec(&file);
    let a = h.add_moment(SongTime::from_ms(2999.9), 80);
    h.add_hit(a, hit(Hitsound::Normal));
    let b = h.add_moment(SongTime::from_ms(3000.2), 80);
    h.add_hit(b, hit(Hitsound::Clap));
    let (out, problems, created) = done(compile_hs(&file, &h, Placement::GreenLines, &|| false, &mut |_, _| {}));
    assert!(problems.is_empty());
    assert_eq!((created, out.hit_objects().len()), (1, 3), "one circle for both");

    h.set_volume(b, 40);
    let (out, problems, _) = done(compile_hs(&file, &h, Placement::GreenLines, &|| false, &mut |_, _| {}));
    assert!(matches!(problems.as_slice(), [p] if matches!(p.kind, ProblemKind::NeedsSampleMix(_))), "{problems:?}");
    assert_eq!(out.hit_objects().len(), 3, "the circle stays, bare");
}

#[test]
fn a_moment_at_an_existing_object_s_time_gets_its_own_circle() {
    let file = map(14);
    let mut h = dec(&file);
    let a = h.add_moment(SongTime::from_ms(1000.0), 80);
    h.add_hit(a, hit(Hitsound::Finish));
    let (out, _, created) = done(compile_hs(&file, &h, Placement::GreenLines, &|| false, &mut |_, _| {}));
    assert_eq!((created, out.hit_objects().len()), (1, 3));
}

#[test]
fn early_format_circles_are_written_in_file_time() {
    // Before v5 lazer adds 24 ms; a moment at song time 3024 is written at 3000.
    let file = map(4);
    let mut h = dec(&file);
    let a = h.add_moment(SongTime::from_ms(3024.0), 80);
    h.add_hit(a, hit(Hitsound::Normal));
    let (out, _, _) = done(compile_hs(&file, &h, Placement::OnObjects, &|| false, &mut |_, _| {}));
    assert!(out.hit_objects().iter().any(|o| o.time() == 3000.0));
}

#[test]
fn with_no_moment_to_place_it_is_plain_compile_and_a_stop_stops() {
    let file = map(14);
    let h = dec(&file);
    let (out, problems, created) = done(compile_hs(&file, &h, Placement::GreenLines, &|| false, &mut |_, _| {}));
    assert_eq!((out.to_bytes(), problems.len(), created), (file.to_bytes(), 0, 0));
    let mut h = h;
    let a = h.add_moment(SongTime::from_ms(3000.0), 80);
    h.add_hit(a, hit(Hitsound::Normal));
    assert!(matches!(compile_hs(&file, &h, Placement::GreenLines, &|| true, &mut |_, _| {}), CompiledHs::Cancelled));
}
