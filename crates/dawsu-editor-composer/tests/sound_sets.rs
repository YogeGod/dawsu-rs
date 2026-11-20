//! Placing and unlinking a sound set: each is one undo step, and the link lives in the document.

use dawsu_editor_composer::{Composer, SetHitnormal};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_hitsounding::{Decompiled, decompile};
use dawsu_osu_format::OsuFile;

fn composer() -> Composer {
    let f = OsuFile::read(b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n".to_vec()).file;
    let Decompiled::Done(h) = decompile(&f, &[], &|| false, &mut |_, _| {}) else { panic!("fixture") };
    Composer::new(h)
}

fn set_hits() -> Vec<(String, SampleIdentity, i32)> {
    ["drum-hitnormal2.wav", "drum-hitclap2.wav"].iter().enumerate()
        .map(|(i, name)| (format!("m{i}"), read_sample_file_name(name).unwrap(), 60)).collect()
}

#[test]
fn placing_a_set_at_a_time_without_an_object_adds_a_moment_and_is_one_undo_step() {
    let mut c = composer();
    let id = c.place_sound_set(1250.0, "set", "d1", 100, &set_hits(), SetHitnormal::Keep, 5.0).expect("placed");
    let h = c.hitsounds();
    let moment = h.placements()[0].moment;
    assert_eq!((h.placements()[0].id, h.moment(moment).time().ms()), (id, 1250.0));
    assert_eq!(h.moment(moment).hits().count(), 2);
    assert!(c.lanes.contains(&set_hits()[1].1), "the set's lanes are rows");
    assert_eq!(c.selected.items().len(), 2, "the placed hits are selected");
    c.undo();
    assert!(c.hitsounds().placements().is_empty());
    assert!(c.hitsounds().moments().iter().all(|m| m.hits().next().is_none() || m.time().ms() == 1000.0));
    c.redo();
    assert_eq!(c.hitsounds().placements().len(), 1);
    c.unlink_placement(id);
    assert!(c.hitsounds().placements().is_empty());
    assert_eq!(c.hitsounds().moment(moment).hits().count(), 2, "the hits stay");
    c.undo();
    assert_eq!(c.hitsounds().placements().len(), 1, "unlink is one undo step");
}

#[test]
fn placing_on_an_object_snaps_to_it_and_a_second_placement_of_the_set_there_changes_nothing() {
    let mut c = composer();
    c.place_sound_set(1002.0, "set", "d1", 100, &set_hits(), SetHitnormal::Keep, 5.0).unwrap();
    assert_eq!(c.hitsounds().placements()[0].moment.0, 0, "on the circle");
    let version = c.history.version();
    assert!(c.place_sound_set(1000.0, "set", "d1", 100, &set_hits(), SetHitnormal::Keep, 5.0).is_none());
    assert_eq!(c.history.version(), version, "no undo step");
}

fn hitnormals(c: &Composer, moment: usize) -> Vec<(String, i32)> {
    let m = &c.hitsounds().moments()[moment];
    m.hits().filter(|l| matches!(l, SampleIdentity::Standard { sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Normal), .. }))
        .map(|l| (format!("{l:?}"), m.hit_volume(l))).collect()
}

#[test]
fn d6_a_set_with_a_hitnormal_replaces_the_moments_hitnormal_in_the_same_undo_step() {
    let mut c = composer();
    let before = hitnormals(&c, 0);
    assert_eq!(before.len(), 1, "the circle's own hitnormal");
    assert_eq!(c.moment_at(1001.0, 5.0).map(|m| m.0), Some(0));
    c.place_sound_set(1000.0, "set", "d1", 100, &set_hits(), SetHitnormal::ReplaceWithSet, 5.0).unwrap();
    let after = hitnormals(&c, 0);
    assert_eq!(after.len(), 1, "only the set's hitnormal: {after:?}");
    assert!(after[0].0.contains("Drum"));
    c.undo();
    assert_eq!(hitnormals(&c, 0), before);
    assert!(c.hitsounds().placements().is_empty());
}

#[test]
fn d6b_an_additions_only_set_moves_the_moments_hitnormal_to_the_sets_index_at_its_volume() {
    let mut c = composer();
    let volume = hitnormals(&c, 0)[0].1;
    let clap = vec![("m0".to_string(), read_sample_file_name("drum-hitclap2.wav").unwrap(), 60)];
    let moved = read_sample_file_name("normal-hitnormal2.wav").unwrap();
    c.place_sound_set(1000.0, "set", "d1", 100, &clap, SetHitnormal::Reindex(moved.clone()), 5.0).unwrap();
    let m = &c.hitsounds().moments()[0];
    assert_eq!(hitnormals(&c, 0).len(), 1);
    assert_eq!(m.hit_volume(&moved), volume);
    assert!(c.lanes.contains(&moved));
    c.undo();
    assert!(!c.hitsounds().moments()[0].has_hit(&moved), "one undo step");
}

#[test]
fn a_hitnormal_another_placement_added_stays_when_a_second_set_takes_the_moment() {
    let mut c = composer();
    c.place_sound_set(1000.0, "first", "d1", 100, &set_hits(), SetHitnormal::ReplaceWithSet, 5.0).unwrap();
    let soft = vec![("s0".to_string(), read_sample_file_name("soft-hitnormal3.wav").unwrap(), 60)];
    c.place_sound_set(1000.0, "second", "d1", 100, &soft, SetHitnormal::ReplaceWithSet, 5.0).unwrap();
    assert!(c.hitsounds().moments()[0].has_hit(&set_hits()[0].1), "the first set keeps its hitnormal");
    let clap_only = vec![("c0".to_string(), read_sample_file_name("normal-hitclap4.wav").unwrap(), 60)];
    c.place_sound_set(1000.0, "third", "d1", 100, &clap_only, SetHitnormal::Reindex(read_sample_file_name("normal-hitnormal4.wav").unwrap()), 5.0).unwrap();
    assert!(c.hitsounds().moments()[0].has_hit(&set_hits()[0].1) && c.hitsounds().moments()[0].has_hit(&soft[0].1), "placed hitnormals are never moved");
}

#[test]
fn a_placement_whose_hits_were_all_deleted_no_longer_blocks_placing_the_set_again() {
    let mut c = composer();
    c.place_sound_set(1250.0, "set", "d1", 100, &set_hits(), SetHitnormal::Keep, 5.0).unwrap();
    c.remove_selected();
    let id = c.place_sound_set(1250.0, "set", "d1", 100, &set_hits(), SetHitnormal::Keep, 5.0).expect("placed again");
    assert_eq!(c.hitsounds().placements().iter().map(|p| p.id).collect::<Vec<_>>(), [id], "the dead placement is replaced");
}
