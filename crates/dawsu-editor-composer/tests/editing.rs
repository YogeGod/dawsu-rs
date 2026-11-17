use dawsu_editor_composer::Composer;
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_hitsounding::{Decompiled, decompile};
use dawsu_osu_format::OsuFile;

#[test]
fn copying_an_empty_selection_keeps_the_clipboard() {
    let mut c = composer();
    c.add(1000.0, clap(), 80, 5.0);
    c.copy();
    c.selected.clear();
    c.copy();
    c.paste(1500.0);
    assert!(c.hitsounds().moments()[1].has_hit(&clap()));
}
#[test]
fn empty_edits_do_not_dirty_the_document() {
    let mut c = composer();
    c.remove_selected();
    c.volume(80, false);
    c.paste(1000.0);
    c.quantize();
    assert!(!c.dirty());
    assert!(!c.history.can_undo());
}

fn composer() -> Composer {
    let f = OsuFile::read(b"osu file format v14\n[General]\nMode:0\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,0,0:0:0:0:\n".to_vec()).file;
    let Decompiled::Done(h) = decompile(&f, &[], &|| false, &mut |_, _| {}) else {
        panic!("fixture")
    };
    Composer::new(h)
}
fn clap() -> SampleIdentity {
    SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Clap),
        index: CustomIndex::new(0),
    }
}
#[test]
fn add_at_an_object_undo_redo_and_clean_state_survive_save() {
    let mut c = composer();
    c.add(1000.0, clap(), 80, 5.0);
    assert!(c.hitsounds().moments()[0].has_hit(&clap()));
    assert!(c.dirty());
    c.mark_saved();
    assert!(!c.dirty());
    c.undo();
    assert!(!c.hitsounds().moments()[0].has_hit(&clap()));
    assert!(c.dirty());
    c.redo();
    assert!(c.hitsounds().moments()[0].has_hit(&clap()));
    assert!(!c.dirty());
}
#[test]
fn moving_a_hit_between_rows_keeps_the_source_and_destination_changes_in_one_step() {
    let mut c = composer();
    c.add(1000.0, clap(), 70, 5.0);
    let destination = SampleIdentity::Standard {
        set: SampleSet::Soft,
        sound: Sound::Hit(Hitsound::Finish),
        index: CustomIndex::new(0),
    };
    c.move_selected(500.0, Some(destination.clone()), 5.0);
    assert!(!c.hitsounds().moments()[0].has_hit(&clap()));
    assert!(c.hitsounds().moments()[1].has_hit(&destination));
    c.undo();
    assert!(c.hitsounds().moments()[0].has_hit(&clap()));
    assert!(!c.hitsounds().moments()[1].has_hit(&destination));
}
#[test]
fn keyboard_lane_move_requests_the_destination_sample_and_one_undo_restores_it() {
    let mut c = composer();
    c.add(1000.0, clap(), 80, 5.0);
    let source = c.lanes.iter().position(|l| *l == clap()).unwrap();
    let destination = c.lanes[source + 1].clone();
    c.move_rows(1);
    assert_eq!(c.audition_requests.last(), Some(&destination));
    assert!(c.hitsounds().moments()[0].has_hit(&destination));
    c.undo();
    assert!(c.hitsounds().moments()[0].has_hit(&clap()));
}
#[test]
fn listening_mute_does_not_dirty_the_map() {
    use dawsu_editor::tools::ToolTarget;
    let mut c = composer();
    c.add(1000.0, clap(), 80, 5.0);
    c.mark_saved();
    let key = c.selected.primary().unwrap().clone();
    c.toggle_listen_mute(&key);
    assert_eq!(c.listen_version, 1);
    assert!(c.listen_muted.contains(&key));
    assert!(!c.dirty());
}
#[test]
fn object_and_relative_snapping_use_the_kernel_settings() {
    use dawsu_editor::tempo::{SnapMode, TempoMap, TimingSection};
    use dawsu_editor::tools::ToolTarget;
    use dawsu_foundation::SongTime;
    let mut c = composer();
    c.tempo = TempoMap::new(vec![TimingSection {
        start: SongTime::from_ms(0.0),
        beat_length_ms: 500.0,
        beats_per_bar: 4,
    }]);
    c.snap_mode = SnapMode::GridRelative;
    assert_eq!(c.snap_offset(SongTime::from_ms(1050.0), 60.0), 125.0);
    c.snap_mode = SnapMode::Objects;
    assert_eq!(c.snap(SongTime::from_ms(1380.0)).ms(), 1500.0);
}
