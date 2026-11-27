use dawsu_editor::tempo::{SnapSpacing, TempoMap, TimingSection};
use dawsu_editor_composer::Composer;
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SongTime, Sound};
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
fn duplicate_uses_a_snapped_offset_and_preserves_the_clipboard() {
    let mut c = composer();
    c.tempo = TempoMap::new(vec![TimingSection {
        beat_length_ms: 500.0,
        ..TimingSection::DEFAULT
    }]);
    c.add(900.0, clap(), 80, 0.0);
    c.copy();

    let whistle = SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Whistle),
        index: CustomIndex::new(0),
    };
    c.add(1110.0, whistle.clone(), 35, 0.0);
    let first = c.selected.primary().unwrap().clone();
    c.add(1400.0, whistle.clone(), 60, 0.0);
    let second = c.selected.primary().unwrap().clone();
    c.selected.replace([first, second]);

    let whistle_hits = |c: &Composer| {
        let mut hits: Vec<_> = c
            .hitsounds()
            .moments()
            .iter()
            .filter(|moment| moment.has_hit(&whistle))
            .map(|moment| (moment.time().ms(), moment.hit_volume(&whistle)))
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        hits
    };
    c.duplicate();
    let duplicated = [(1110.0, 35), (1400.0, 60), (1485.0, 35), (1775.0, 60)];
    assert_eq!(whistle_hits(&c), duplicated);
    c.undo();
    assert_eq!(whistle_hits(&c), duplicated[..2]);
    c.redo();
    assert_eq!(whistle_hits(&c), duplicated);

    c.paste(2000.0);
    assert_eq!(whistle_hits(&c), duplicated);
    let pasted = c
        .hitsounds()
        .moments()
        .iter()
        .find(|moment| moment.time().ms() == 2000.0)
        .expect("pasted clipboard hit");
    assert!(pasted.has_hit(&clap()));
    assert_eq!(pasted.hit_volume(&clap()), 80);
}

#[test]
fn duplicate_with_long_snap_steps_across_tempo_changes() {
    let mut c = composer();
    c.spacing = SnapSpacing::new(2, 1).unwrap();
    c.tempo = TempoMap::new(vec![
        TimingSection {
            beat_length_ms: 500.0,
            ..TimingSection::DEFAULT
        },
        TimingSection {
            start: SongTime::from_ms(1400.0),
            beat_length_ms: 400.0,
            ..TimingSection::DEFAULT
        },
    ]);
    c.add(1000.0, clap(), 70, 0.0);
    c.add(1800.0, clap(), 70, 0.0);
    c.select_all();

    c.duplicate();
    let times: Vec<_> = c
        .hitsounds()
        .moments()
        .iter()
        .filter(|moment| moment.has_hit(&clap()))
        .map(|moment| moment.time().ms())
        .collect();
    assert_eq!(times, [1000.0, 1800.0, 2200.0, 3000.0]);
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
    let normal = c.hitsounds().moments()[0].hits().next().unwrap().clone();
    c.add(1160.0, clap(), 80, 0.0);
    c.add(1160.0, normal.clone(), 80, 0.0);
    c.apply("Change gain", |hits| {
        hits.set_hit_volume(dawsu_hitsounding::MomentId(1), &clap(), 30);
    });
    c.mark_saved();
    c.select_all();
    c.quantize();
    assert_eq!(c.hitsounds().moments()[0].hits().count(), 1, "the off-grid circle keeps its hit");
    assert_eq!(c.hitsounds().moments()[0].time().ms(), 1270.0);

    let moved = c.hitsounds().moments().iter().position(|m| m.has_hit(&clap())).unwrap();
    let quantized = &c.hitsounds().moments()[moved];
    assert_eq!(quantized.time().ms(), 1125.0);
    assert_eq!((quantized.hit_volume(&normal), quantized.hit_volume(&clap())), (80, 30));
    assert!(c.selected.items().iter().any(|key| key.moment == moved));
    assert_eq!(c.selected.items().len(), 3);
    c.mark_saved();
    c.quantize();
    assert!(!c.dirty(), "another quantize of the same hits is a no-op");
}

#[test]
fn selected_lane_gain_edits_other_hits_at_selected_moments_and_undoes_as_one_step() {
    use std::collections::BTreeSet;

    let mut c = composer();
    let normal = c.hitsounds().moments()[0].hits().next().unwrap().clone();
    c.add(1000.0, clap(), 80, 5.0);
    c.volume(70, false);
    c.mark_saved();
    let selected_lanes = BTreeSet::from([normal.clone()]);

    assert!(c.volume_for_lanes(30, false, Some(&selected_lanes)));
    let edited = &c.hitsounds().moments()[0];
    assert_eq!((edited.hit_volume(&normal), edited.hit_volume(&clap())), (30, 70));
    assert_eq!(c.hitsounds().moments()[1].hit_volume(&normal), 100);
    assert!(!c.volume_for_lanes(30, false, Some(&selected_lanes)));
    assert!(!c.volume_for_lanes(30, false, Some(&BTreeSet::new())));

    c.undo();
    assert_eq!(c.hitsounds().moments()[0].hit_volume(&normal), 70);
    assert!(!c.dirty());
    c.redo();
    assert_eq!(c.hitsounds().moments()[0].hit_volume(&normal), 30);
    c.add(1000.0, clap(), 80, 5.0);
    c.volume(5, true);
    let relative = &c.hitsounds().moments()[0];
    assert_eq!((relative.hit_volume(&normal), relative.hit_volume(&clap())), (35, 75));
    assert!(c.volume_for_lanes(0, false, Some(&selected_lanes)));
    let scoped = &c.hitsounds().moments()[0];
    assert_eq!((scoped.hit_volume(&normal), scoped.hit_volume(&clap())), (5, 75), "a lane's volume stops at 5%");
    c.volume(0, false);
    let shared = &c.hitsounds().moments()[0];
    assert_eq!((shared.hit_volume(&normal), shared.hit_volume(&clap())), (5, 5), "a moment's volume stops at 5%");
    assert!(!shared.has_hit_volume_overrides());
}

#[test]
fn a_volume_edit_never_goes_under_5_percent() {
    let mut c = composer();
    let normal = c.hitsounds().moments()[0].hits().next().unwrap().clone();
    c.select_all();
    c.volume(2, false);
    assert_eq!(c.hitsounds().moments()[0].hit_volume(&normal), 5, "typing 2 gives 5");
    c.volume(-100, true);
    assert_eq!(c.hitsounds().moments()[0].hit_volume(&normal), 5, "a step down stops at 5");
    c.volume(60, false);
    assert_eq!(c.hitsounds().moments()[0].hit_volume(&normal), 60);
    c.volume(-58, true);
    assert_eq!(c.hitsounds().moments()[0].hit_volume(&normal), 5, "60 minus 58 stops at 5");
}

#[test]
fn the_ramp_tool_and_volume_drawer_never_go_under_5_percent() {
    use dawsu_editor::tools::ToolTarget;
    use dawsu_editor_composer::HitKey;
    use dawsu_hitsounding::MomentId;

    let mut c = composer();
    let normal = c.hitsounds().moments()[0].hits().next().unwrap().clone();
    // The Ramp tool sets values through `set_values`.
    c.begin("Ramp");
    c.set_values(&[(HitKey { moment: 0, lane: normal.clone() }, 1)]);
    c.commit();
    assert_eq!(c.hitsounds().moments()[0].hit_volume(&normal), 5, "a ramp to 1 gives 5");
    // The Volume drawer moves a moment directly.
    let mut hits = c.hitsounds().clone();
    hits.move_volume(MomentId(0), 0);
    assert_eq!(hits.moments()[0].hit_volume(&normal), 5, "a drag to 0 gives 5");
    assert_eq!(hits.moments()[0].volume(), 5);
}

#[test]
fn copied_moved_and_replaced_hits_keep_their_gain() {
    use dawsu_hitsounding::MomentId;
    use std::collections::BTreeSet;

    let mut c = composer();
    let normal = c.hitsounds().moments()[0].hits().next().unwrap().clone();
    c.add(1000.0, clap(), 80, 5.0);
    assert!(c.volume_for_lanes(30, false, Some(&BTreeSet::from([clap()]))));
    c.copy();
    c.paste(1500.0);
    let pasted = &c.hitsounds().moments()[1];
    assert_eq!((pasted.hit_volume(&normal), pasted.hit_volume(&clap())), (100, 30));

    let destination = SampleIdentity::File("rim.wav".into());
    c.move_selected(0.0, Some(destination.clone()), 5.0);
    let moved = &c.hitsounds().moments()[1];
    assert!(!moved.has_hit(&clap()));
    assert_eq!((moved.hit_volume(&normal), moved.hit_volume(&destination)), (100, 30));

    let replacement = SampleIdentity::File("snare.wav".into());
    c.apply("Replace sample", |hits| {
        hits.replace_lane(&destination, replacement.clone());
    });
    let replaced = c.hitsounds().moment(MomentId(1));
    assert!(!replaced.has_hit(&destination));
    assert_eq!(replaced.hit_volume(&replacement), 30);
    c.undo();
    assert_eq!(c.hitsounds().moment(MomentId(1)).hit_volume(&destination), 30);
}

#[test]
fn restored_gains_are_a_baseline_and_lane_scope_follows_the_replacement_slot() {
    use dawsu_hitsounding::MomentId;
    use std::collections::BTreeSet;

    let mut c = composer();
    let id = MomentId(0);
    let normal = c.hitsounds().moment(id).hits().next().unwrap().clone();
    let rim = SampleIdentity::File("rim.wav".into());
    c.history = {
        let mut hits = c.hitsounds().clone();
        hits.restore_moment_hits(id, 70, [(normal.clone(), 70), (rim.clone(), 25)]);
        hits.bind_lane_slot(rim.clone(), clap());
        dawsu_editor::EditHistory::new(hits)
    };
    let restored = c.hitsounds().moment(id);
    assert!(!restored.is_changed());
    assert_eq!((restored.hit_volume(&normal), restored.hit_volume(&rim)), (70, 25));

    c.select_all();
    assert!(c.volume_for_lanes(5, true, Some(&BTreeSet::from([clap()]))));
    assert_eq!(c.hitsounds().moment(id).hit_volume(&rim), 30);
    assert_eq!(c.hitsounds().moment(id).hit_volume(&normal), 70);
    assert!(c.hitsounds().moment(id).is_changed());
    c.undo();
    assert_eq!(c.hitsounds().moment(id).hit_volume(&rim), 25);
    assert!(!c.hitsounds().moment(id).is_changed());
}

/// Hitnormal 80 at the moment's own volume, clap 50 with its own volume.
fn normal_80_clap_50() -> (Composer, SampleIdentity) {
    use std::collections::BTreeSet;

    let mut c = composer();
    let normal = c.hitsounds().moments()[0].hits().next().unwrap().clone();
    c.add(1000.0, clap(), 80, 5.0);
    c.volume(80, false);
    assert!(c.volume_for_lanes(50, false, Some(&BTreeSet::from([clap()]))));
    assert_eq!(normal_and_clap(&c, &normal), (80, 50));
    (c, normal)
}

fn normal_and_clap(c: &Composer, normal: &SampleIdentity) -> (i32, i32) {
    let moment = &c.hitsounds().moments()[0];
    (moment.hit_volume(normal), moment.hit_volume(&clap()))
}

#[test]
fn set_all_moves_every_lane_together_and_keeps_their_difference() {
    let (mut c, normal) = normal_80_clap_50();
    assert!(!c.volume_for_lanes(80, false, None), "the volume the moment already has changes nothing");
    c.volume(90, false);
    assert_eq!(normal_and_clap(&c, &normal), (90, 60));
    c.volume(100, false);
    assert_eq!(normal_and_clap(&c, &normal), (100, 70));
    c.volume(20, false);
    assert_eq!(normal_and_clap(&c, &normal), (20, 5), "the clap stops at 5%");
    c.undo();
    assert_eq!(normal_and_clap(&c, &normal), (100, 70));
}

#[test]
fn ramp_moves_every_lane_together_and_keeps_their_difference() {
    use dawsu_editor::tools::ToolTarget;
    use dawsu_editor_composer::HitKey;

    let (mut c, normal) = normal_80_clap_50();
    let ramp = |c: &mut Composer, value: i32| {
        // The Ramp tool gives every hit at the moment, so the moment comes twice.
        let values = [
            (HitKey { moment: 0, lane: normal.clone() }, value),
            (HitKey { moment: 0, lane: clap() }, value),
        ];
        c.begin("Ramp");
        c.set_values(&values);
        c.commit();
    };
    ramp(&mut c, 90);
    assert_eq!(normal_and_clap(&c, &normal), (90, 60));
    ramp(&mut c, 20);
    assert_eq!(normal_and_clap(&c, &normal), (20, 5));
}

#[test]
fn scale_by_volume_multiplies_every_lane_and_keeps_their_balance() {
    use dawsu_editor_composer::VolumeScope;

    let (mut c, normal) = normal_80_clap_50();
    assert!(c.scale_by_volume(VolumeScope::All, 0, 100, 50.0));
    assert_eq!(normal_and_clap(&c, &normal), (40, 25));
    c.undo();
    assert!(c.scale_by_volume(VolumeScope::All, 0, 100, 150.0));
    assert_eq!(normal_and_clap(&c, &normal), (100, 75), "the hitnormal stops at 100");
    c.undo();
    assert!(c.scale_by_volume(VolumeScope::All, 0, 100, 1.0));
    assert_eq!(normal_and_clap(&c, &normal), (5, 5), "no sound goes below 5%");
}

#[test]
fn scale_by_volume_on_lanes_of_selected_hits_changes_only_those_lanes() {
    use dawsu_editor_composer::VolumeScope;

    let (mut c, normal) = normal_80_clap_50();
    assert!(
        !c.scale_by_volume(VolumeScope::SelectedLanes, 60, 100, 50.0),
        "From and To pick lanes by their own volume: the clap is at 50"
    );
    assert!(c.scale_by_volume(VolumeScope::SelectedLanes, 0, 100, 50.0));
    assert_eq!(normal_and_clap(&c, &normal), (80, 25), "the hitnormal is not a selected lane");
    assert_eq!(c.hitsounds().moments()[0].volume(), 80);
    assert_eq!(c.hitsounds().moments()[1].hit_volume(&normal), 100);
}

#[test]
fn scale_by_volume_leaves_a_silent_sound_silent() {
    use dawsu_editor_composer::VolumeScope;

    let (mut c, normal) = normal_80_clap_50();
    c.apply("Mute the clap", |hits| {
        hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &clap(), 0);
    });
    assert_eq!(normal_and_clap(&c, &normal), (80, 0));
    assert!(c.scale_by_volume(VolumeScope::All, 0, 100, 50.0));
    assert_eq!(normal_and_clap(&c, &normal), (40, 0), "0 times any factor stays 0");
}

/// The `normal_80_clap_50` moment after its hitnormal is deleted: the moment keeps 80, but the
/// only hit left plays at 50.
fn clap_50_at_a_moment_of_80() -> Composer {
    use dawsu_editor_composer::HitKey;

    let (mut c, normal) = normal_80_clap_50();
    c.selected.replace([HitKey { moment: 0, lane: normal.clone() }]);
    c.remove_selected();
    let moment = &c.hitsounds().moments()[0];
    assert!(!moment.has_hit(&normal));
    assert_eq!((moment.volume(), moment.hit_volume(&clap())), (80, 50));
    c.selected.replace([HitKey { moment: 0, lane: clap() }]);
    c
}

#[test]
fn set_all_after_a_lane_scoped_set_moves_from_the_volume_the_hit_plays_at() {
    use dawsu_editor_composer::HitKey;
    use std::collections::BTreeSet;

    let mut c = composer();
    let normal = c.hitsounds().moments()[0].hits().next().unwrap().clone();
    // Undo clears the selection, so each Set selects the hitnormal again.
    let select = |c: &mut Composer| c.selected.replace([HitKey { moment: 0, lane: normal.clone() }]);
    select(&mut c);
    assert!(c.volume_for_lanes(50, false, Some(&BTreeSet::from([normal.clone()]))));
    let hitnormal = |c: &Composer| c.hitsounds().moments()[0].hit_volume(&normal);
    assert_eq!((c.hitsounds().moments()[0].volume(), hitnormal(&c)), (100, 50));

    assert!(c.volume_for_lanes(80, false, None));
    assert_eq!(hitnormal(&c), 80);
    c.undo();
    select(&mut c);
    assert!(c.volume_for_lanes(100, false, None), "no hit plays at the moment's own 100");
    assert_eq!(hitnormal(&c), 100);
    c.undo();
    select(&mut c);
    assert!(!c.volume_for_lanes(50, false, None), "the hitnormal already plays at 50");
    assert_eq!(hitnormal(&c), 50);
}

#[test]
fn set_all_after_deleting_the_hitnormal_moves_from_the_hit_left() {
    use dawsu_editor_composer::HitKey;

    let mut c = clap_50_at_a_moment_of_80();
    // Undo clears the selection, so each Set selects the clap again.
    let select = |c: &mut Composer| c.selected.replace([HitKey { moment: 0, lane: clap() }]);
    let clap_volume = |c: &Composer| c.hitsounds().moments()[0].hit_volume(&clap());
    assert!(c.volume_for_lanes(80, false, None), "no hit plays at the deleted hitnormal's 80");
    assert_eq!(clap_volume(&c), 80);
    c.undo();
    select(&mut c);
    c.volume(90, false);
    assert_eq!(clap_volume(&c), 90);
    c.undo();
    select(&mut c);
    assert!(!c.volume_for_lanes(50, false, None), "the clap already plays at 50");
    assert_eq!(clap_volume(&c), 50);
}

#[test]
fn set_all_puts_the_loudest_hit_at_the_value_and_keeps_every_gap() {
    use dawsu_editor_composer::HitKey;
    use std::collections::BTreeSet;

    let (mut c, normal) = normal_80_clap_50();
    let whistle = SampleIdentity::Standard {
        set: SampleSet::Normal,
        sound: Sound::Hit(Hitsound::Whistle),
        index: CustomIndex::new(0),
    };
    c.add(1000.0, whistle.clone(), 80, 5.0);
    assert!(c.volume_for_lanes(30, false, Some(&BTreeSet::from([whistle.clone()]))));
    c.selected.replace([HitKey { moment: 0, lane: normal.clone() }]);
    c.remove_selected();
    c.selected.replace([HitKey { moment: 0, lane: clap() }]);
    c.volume(90, false);
    let moment = &c.hitsounds().moments()[0];
    assert_eq!((moment.hit_volume(&clap()), moment.hit_volume(&whistle)), (90, 70));
}

#[test]
fn ramp_after_deleting_the_hitnormal_moves_from_the_hit_left() {
    use dawsu_editor::tools::ToolTarget;
    use dawsu_editor_composer::HitKey;

    let mut c = clap_50_at_a_moment_of_80();
    let before = c.hitsounds().moments()[0].clone();
    c.begin("Ramp");
    c.set_values(&[(HitKey { moment: 0, lane: clap() }, 50)]);
    c.commit();
    assert_eq!(c.hitsounds().moments()[0], before, "a Ramp to where the clap plays leaves the moment as it was");
    c.begin("Ramp");
    c.set_values(&[(HitKey { moment: 0, lane: clap() }, 90)]);
    c.commit();
    assert_eq!(c.hitsounds().moments()[0].hit_volume(&clap()), 90);
}

#[test]
fn scale_by_volume_picks_a_moment_by_the_volume_its_hits_play_at() {
    use dawsu_editor_composer::{HitKey, VolumeScope};

    let mut c = clap_50_at_a_moment_of_80();
    let clap_volume = |c: &Composer| c.hitsounds().moments()[0].hit_volume(&clap());
    assert!(
        !c.scale_by_volume(VolumeScope::All, 70, 90, 50.0),
        "no hit at the first circle plays at the deleted hitnormal's 80"
    );
    assert_eq!(clap_volume(&c), 50);
    assert!(c.scale_by_volume(VolumeScope::All, 40, 60, 50.0), "the clap plays at 50");
    assert_eq!(clap_volume(&c), 25);
    c.undo();
    c.selected.replace([HitKey { moment: 0, lane: clap() }]);
    assert!(!c.scale_by_volume(VolumeScope::Selected, 70, 90, 50.0));
    assert!(c.scale_by_volume(VolumeScope::Selected, 40, 60, 50.0));
    assert_eq!(clap_volume(&c), 25);
}

#[test]
fn bulk_lane_move_deduplicates_audition_requests_and_clears_move_stroke() {
    use dawsu_editor_composer::HitKey;

    let mut c = composer();
    let src_idx = c.lanes.iter().position(|l| *l == clap()).unwrap();
    let dst_lane = c.lanes[src_idx + 1].clone();
    let after_lane = c.lanes[src_idx + 2].clone();

    // 25 hits on one lane moved down together must queue only one audition request.
    let mut keys = Vec::new();
    for i in 0..25 {
        c.add(5000.0 + i as f64 * 100.0, clap(), 80, 0.0);
        keys.push(c.selected.primary().unwrap().clone());
    }
    c.selected.replace(keys);
    c.audition_requests.clear();
    c.move_rows(1);
    assert_eq!(c.audition_requests, vec![dst_lane.clone()]);

    // Multi-lane select_all + move_rows queues only unique destination lanes and does NOT leak
    // move_stroke into a subsequent move_rows on a smaller selection.
    c.select_all();
    c.audition_requests.clear();
    c.move_rows(1);
    let unique: std::collections::HashSet<_> = c.audition_requests.iter().collect();
    assert_eq!(unique.len(), c.audition_requests.len());

    // Now select ONLY 2 hits on `after_lane` and move back up: must NOT re-select the earlier Ctrl+A set.
    let two: Vec<HitKey> = c
        .selected
        .items()
        .iter()
        .filter(|k| k.lane == after_lane)
        .take(2)
        .cloned()
        .collect();
    assert_eq!(two.len(), 2);
    c.selected.replace(two);
    c.move_rows(-1);
    assert_eq!(c.selected.items().len(), 2);
    assert!(c.selected.items().iter().all(|k| k.lane == dst_lane));
}

#[test]
fn move_rows_in_view_steps_through_empty_visible_lanes_in_visual_order() {
    let mut c = composer();
    // Visual order puts lane 3 first, empty lane 1 second, then lanes 0 and 2.
    let lane3 = c.lanes[3].clone();
    let lane1 = c.lanes[1].clone();
    c.add(500.0, lane3.clone(), 80, 0.0);
    let k1 = c.selected.primary().unwrap().clone();
    c.add(1500.0, lane3.clone(), 80, 0.0);
    let k2 = c.selected.primary().unwrap().clone();
    c.selected.replace([k1.clone(), k2.clone()]);
    c.move_rows_in_view(&[3, 1, 0, 2], 1);
    assert_eq!(c.selected.items().len(), 2);
    assert!(c.selected.items().iter().all(|k| k.lane == lane1));
    assert!(c.hitsounds().moments()[k1.moment].has_hit(&lane1));
    assert!(c.hitsounds().moments()[k2.moment].has_hit(&lane1));
}
