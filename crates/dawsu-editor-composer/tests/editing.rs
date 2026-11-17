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

#[test]
fn nudge_and_ruler_follow_the_tempo_change() {
    use dawsu_editor::tempo::{TempoMap, TimingSection};
    use dawsu_foundation::SongTime;
    let mut c = composer();
    c.tempo = TempoMap::new(vec![
        TimingSection {
            start: SongTime::from_ms(0.0),
            beat_length_ms: 500.0,
            beats_per_bar: 4,
        },
        TimingSection {
            start: SongTime::from_ms(1100.0),
            beat_length_ms: 250.0,
            beats_per_bar: 3,
        },
    ]);
    c.add(1350.0, clap(), 80, 0.0);
    c.nudge(1);
    let key = c.selected.items().iter().next().unwrap();
    assert_eq!(c.hitsounds().moments()[key.moment].time().ms(), 1412.5);
    c.add(1360.0, clap(), 70, 0.0);
    let a = c.selected.primary().unwrap().clone();
    c.add(1370.0, clap(), 70, 0.0);
    c.selected.add(a);
    c.nudge(1);
    let mut times: Vec<_> = c
        .selected
        .items()
        .iter()
        .map(|k| c.hitsounds().moments()[k.moment].time().ms())
        .collect();
    times.sort_by(f64::total_cmp);
    assert_eq!(times, [1422.5, 1432.5]);
    let ticks: Vec<_> = c
        .tempo
        .ruler_ticks(950.0, 1650.0, 1.0)
        .map(|t| t.ms())
        .collect();
    assert_eq!(ticks, [1000.0, 1100.0, 1350.0, 1600.0]);
}

#[test]
fn quantize_moves_only_off_grid_loose_hits_and_keeps_them_selected() {
    use dawsu_editor::tempo::{TempoMap, TimingSection};
    use dawsu_foundation::SongTime;
    let map = OsuFile::read(b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1270,1,0,0:0:0:0:\n".to_vec()).file;
    let Decompiled::Done(hits) = decompile(&map, &[], &|| false, &mut |_, _| {}) else {
        panic!("fixture");
    };
    let mut c = Composer::new(hits);
    c.tempo = TempoMap::new(vec![TimingSection {
        start: SongTime::from_ms(0.0), beat_length_ms: 500.0, beats_per_bar: 4,
    }]);
    c.add(1160.0, clap(), 80, 0.0);
    c.mark_saved();
    c.select_all();
    c.quantize();
    assert_eq!(c.hitsounds().moments()[0].hits().count(), 1, "the off-grid circle keeps its hit");
    assert_eq!(c.hitsounds().moments()[0].time().ms(), 1270.0);

    let moved = c.hitsounds().moments().iter().position(|m| m.has_hit(&clap())).unwrap();
    assert_eq!(c.hitsounds().moments()[moved].time().ms(), 1125.0);
    assert!(c.selected.items().iter().any(|key| key.moment == moved));
    assert_eq!(c.selected.items().len(), 2);
    c.mark_saved();
    c.quantize();
    assert!(!c.dirty(), "another quantize of the same hits is a no-op");
}
