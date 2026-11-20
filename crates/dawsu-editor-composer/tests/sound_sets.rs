//! Placing and unlinking a sound set: each is one undo step, and the link lives in the document.

use dawsu_editor_composer::Composer;
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
    let id = c.place_sound_set(1250.0, "set", "d1", 100, &set_hits(), 5.0).expect("placed");
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
    c.place_sound_set(1002.0, "set", "d1", 100, &set_hits(), 5.0).unwrap();
    assert_eq!(c.hitsounds().placements()[0].moment.0, 0, "on the circle");
    let version = c.history.version();
    assert!(c.place_sound_set(1000.0, "set", "d1", 100, &set_hits(), 5.0).is_none());
    assert_eq!(c.history.version(), version, "no undo step");
}
