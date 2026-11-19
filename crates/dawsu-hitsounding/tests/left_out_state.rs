//! Save's roll-back (F052) puts a moment, a slider body or an HS group back to an earlier state
//! without touching what decompile found, so a part that differs from the file still counts as
//! changed, and a lane's hidden state and a head's volume stay as they are.

use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_hitsounding::hs::{compile_hs, hs_group_time, hs_groups, CompiledHs};
use dawsu_hitsounding::{decompile, Decompiled, Hitsounds, MomentId, Place, Placement};
use dawsu_osu_format::OsuFile;

fn read(bytes: &[u8]) -> OsuFile {
    OsuFile::read(bytes.to_vec()).file
}

fn map() -> OsuFile {
    read(b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n")
}

fn dec(file: &OsuFile) -> Hitsounds {
    match decompile(file, &[], &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => panic!("{other:?}"),
    }
}

fn hit(set: SampleSet, sound: Hitsound) -> SampleIdentity {
    SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: CustomIndex::new(0) }
}

fn state(h: &Hitsounds, id: MomentId) -> (i32, Vec<(SampleIdentity, i32)>) {
    let m = h.moment(id);
    (m.volume(), m.hits().map(|lane| (lane.clone(), m.hit_volume(lane))).collect())
}

#[test]
fn a_moment_set_to_another_state_keeps_its_baseline_and_hidden_lanes() {
    let file = map();
    let mut h = dec(&file);
    let decoded = state(&h, MomentId(0));
    let clap = hit(SampleSet::Soft, Hitsound::Clap);
    h.set_lane_hidden(&clap, true);
    h.set_moment_state(MomentId(0), 80, [(hit(SampleSet::Normal, Hitsound::Normal), 80), (clap.clone(), 40)]);
    assert!(h.moment(MomentId(0)).is_changed());
    assert_eq!(h.moment(MomentId(0)).hit_volume(&clap), 40);
    assert!(h.lane(&clap).is_some(), "the lane exists for the hit");
    assert!(h.is_lane_hidden(&clap), "unlike add_hit, the lane stays hidden");
    h.set_moment_state(MomentId(0), decoded.0, decoded.1.clone());
    assert!(!h.moment(MomentId(0)).is_changed(), "the decoded state is the baseline again");
}

#[test]
fn reset_puts_a_moment_back_to_what_decompile_found_and_empties_a_loose_one() {
    let file = map();
    let mut h = dec(&file);
    let decoded = state(&h, MomentId(0));
    h.add_hit(MomentId(0), hit(SampleSet::Drum, Hitsound::Whistle));
    h.set_hit_volume(MomentId(0), &hit(SampleSet::Drum, Hitsound::Whistle), 30);
    h.reset_moment(MomentId(0));
    assert_eq!(state(&h, MomentId(0)), decoded);
    assert!(!h.moment(MomentId(0)).is_changed());
    let loose = h.add_moment(SongTime::from_ms(5000.0), 60);
    h.add_hit(loose, hit(SampleSet::Normal, Hitsound::Normal));
    h.reset_moment(loose);
    assert_eq!(h.moment(loose).hits().count(), 0);
    assert!(!h.moment(loose).is_changed());
}

#[test]
fn a_body_state_is_set_without_moving_its_head_volume() {
    let file = map();
    let mut h = dec(&file);
    let head = h.moments().iter().position(|m| matches!(m.place(), Place::SliderNode { node: 0, .. })).unwrap();
    let head_volume = h.moment(MomentId(head)).volume();
    let decompiled = h.bodies()[0].decompiled_state();
    assert_eq!(h.bodies()[0].state(), decompiled);
    let mut changed = decompiled.clone();
    changed.volume = 30;
    changed.whistle = true;
    changed.set = Some(SampleSet::Soft);
    h.set_body_state(0, changed.clone());
    assert_eq!(h.bodies()[0].state(), changed);
    assert!(h.bodies()[0].is_changed());
    assert_eq!(h.moment(MomentId(head)).volume(), head_volume, "set_body_volume would move the head");
    assert!(h.lane(&h.bodies()[0].whistle_lane()).is_some());
    h.set_body_state(0, decompiled);
    assert!(!h.bodies()[0].is_changed());
}

#[test]
fn hs_groups_name_the_moments_of_each_created_circle_in_creation_order() {
    let file = map();
    let mut h = dec(&file);
    let late = h.add_moment(SongTime::from_ms(4000.0), 60);
    h.add_hit(late, hit(SampleSet::Normal, Hitsound::Normal));
    let a = h.add_moment(SongTime::from_ms(3000.4), 80);
    h.add_hit(a, hit(SampleSet::Normal, Hitsound::Normal));
    let b = h.add_moment(SongTime::from_ms(3000.2), 80);
    h.add_hit(b, hit(SampleSet::Normal, Hitsound::Finish));
    let empty = h.add_moment(SongTime::from_ms(6000.0), 80);
    let groups = hs_groups(&file, &h);
    assert_eq!(groups.into_iter().collect::<Vec<_>>(), [(3000, vec![a, b]), (4000, vec![late])]);
    assert_eq!(hs_group_time(&file, h.moment(a).time()), 3000);
    assert_eq!(hs_group_time(&file, h.moment(empty).time()), 6000);
    let CompiledHs::Done { file: out, created, .. } = compile_hs(&file, &h, Placement::OnObjects, &|| false, &mut |_, _| {}) else {
        panic!("cancelled")
    };
    let times: Vec<_> = created.iter().map(|id| out.hit_objects().into_iter().find(|o| o.line() == id.line()).unwrap().time()).collect();
    assert_eq!(times, [3000.0, 4000.0]);
}
