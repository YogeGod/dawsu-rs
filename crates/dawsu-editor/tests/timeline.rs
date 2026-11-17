//! Timeline maths and snap (Responsibilities 4 and 4a); locators and bookmarks (Responsibility 5).

use dawsu_editor::tempo::{Snap, SnapMode, SnapSpacing, SnapSpacingError, TempoMap, TimingSection};
use dawsu_editor::time_axis::TimeAxis;
use dawsu_editor::transport::{BookmarkList, Locators};
use dawsu_foundation::SongTime;

fn t(ms: f64) -> SongTime {
    SongTime::from_ms(ms)
}

fn section(start: f64, beat: f64, per_bar: u32) -> TimingSection {
    TimingSection {
        start: t(start),
        beat_length_ms: beat,
        beats_per_bar: per_bar,
    }
}

/// Three red lines: 120 BPM from 0, 100 BPM (600 ms) from 2000, 3/4 at 150 BPM (400 ms) from 5000.
fn tempo() -> TempoMap {
    TempoMap::new(vec![
        section(2000.0, 600.0, 4),
        section(0.0, 500.0, 4),
        section(5000.0, 400.0, 3),
    ])
}

fn close(a: SongTime, b: f64) -> bool {
    (a.ms() - b).abs() < 1e-6
}

#[test]
fn r4_snap_spacing_is_whole_beats_or_one_nth_of_a_beat() {
    assert_eq!(SnapSpacing::new(0, 1), Err(SnapSpacingError::Zero));
    assert_eq!(
        SnapSpacing::new(2, 3),
        Err(SnapSpacingError::LongAndFraction)
    );
    assert_eq!(SnapSpacing::fraction(7).unwrap().step_ms(700.0), 100.0);
    assert_eq!(SnapSpacing::new(4, 1).unwrap().step_ms(500.0), 2000.0);
    let offered: Vec<String> = SnapSpacing::OPTIONS
        .iter()
        .map(|s| format!("{}/{}", s.beats(), s.divisions()))
        .collect();
    assert_eq!(
        offered,
        [
            "4/1", "3/1", "2/1", "1/1", "1/2", "1/3", "1/4", "1/5", "1/6", "1/7", "1/8", "1/9"
        ]
    );
    assert_eq!(
        SnapSpacing::fraction(4).unwrap().neighbour(1),
        SnapSpacing::fraction(5).unwrap()
    );
    assert_eq!(
        SnapSpacing::fraction(9).unwrap().neighbour(1),
        SnapSpacing::fraction(9).unwrap(),
        "clamped at the finest"
    );
    assert_eq!(
        SnapSpacing::new(4, 1).unwrap().neighbour(-1),
        SnapSpacing::new(4, 1).unwrap()
    );
}

#[test]
fn r4_snaps_to_one_fifth_seventh_and_ninth_within_each_red_line() {
    let map = tempo();
    // 1/5 at 120 BPM: 100 ms steps from 0.
    assert!(close(
        map.snap(t(1234.0), SnapSpacing::fraction(5).unwrap()),
        1200.0
    ));
    // 1/7 at 100 BPM from 2000: steps of 600/7 ms counted from the red line.
    let step7 = 600.0 / 7.0;
    assert!(close(
        map.snap(
            t(2000.0 + 3.0 * step7 + 10.0),
            SnapSpacing::fraction(7).unwrap()
        ),
        2000.0 + 3.0 * step7
    ));
    // 1/9 at 150 BPM from 5000.
    let step9 = 400.0 / 9.0;
    assert!(close(
        map.snap(
            t(5000.0 + 5.0 * step9 - 4.0),
            SnapSpacing::fraction(9).unwrap()
        ),
        5000.0 + 5.0 * step9
    ));
    // Long snaps: 2/1 at 120 BPM.
    assert!(close(
        map.snap(t(2600.0 - 700.0), SnapSpacing::new(2, 1).unwrap()),
        2000.0
    ));
}

#[test]
fn r4_the_next_red_line_wins_when_it_is_closer_than_the_grid_line_as_in_lazer() {
    // 120 BPM from 0, and a red line at 1100 that is off the first section's grid.
    let map = TempoMap::new(vec![section(0.0, 500.0, 4), section(1100.0, 500.0, 4)]);
    let beat = SnapSpacing::fraction(1).unwrap();
    // 1060: the grid line 1000 is 60 away, the red line 40: the red line wins.
    assert!(close(map.snap(t(1060.0), beat), 1100.0));
    // 1020: the grid line 1000 is 20 away, the red line 80: the grid wins.
    assert!(close(map.snap(t(1020.0), beat), 1000.0));
}

#[test]
fn r4_times_before_zero_snap_to_a_positive_grid_line_and_an_empty_map_uses_60_bpm() {
    let map = TempoMap::new(vec![section(250.0, 500.0, 4)]);
    assert!(close(
        map.snap(t(-400.0), SnapSpacing::fraction(1).unwrap()),
        250.0
    ));
    let empty = TempoMap::default();
    assert!(close(
        empty.snap(t(1600.0), SnapSpacing::fraction(1).unwrap()),
        2000.0
    ));
}

#[test]
fn r4_grid_steps_move_to_the_previous_and_next_grid_line() {
    let map = tempo();
    let quarter = SnapSpacing::fraction(4).unwrap();
    assert!(close(map.grid_step(t(1000.0), 1, quarter), 1125.0));
    assert!(close(map.grid_step(t(1000.0), -1, quarter), 875.0));
    assert!(close(map.grid_step(t(1010.0), 1, quarter), 1125.0));
    assert!(
        close(map.grid_step(t(1010.0), -1, quarter), 1000.0),
        "off the grid: the line just before"
    );
    assert!(
        close(map.grid_step(t(0.0), -1, quarter), 0.0),
        "never before 0"
    );
}

#[test]
fn r4_bar_and_beat_count_bars_across_red_lines() {
    let map = tempo();
    let at = |ms: f64| {
        let b = map.bar_beat_at(t(ms));
        (b.bar, b.beat)
    };
    assert_eq!(at(0.0), (1, 1));
    assert_eq!(at(1750.0), (1, 4));
    // The second red line (2000) starts bar 2: the first section is exactly one 4/4 bar long.
    assert_eq!(at(2000.0), (2, 1));
    assert_eq!(at(2000.0 + 4.0 * 600.0), (3, 1));
    // 2000..5000 is 3000 ms = 1.25 bars of 2400 ms, so the third red line starts bar 2 + 2 = 4.
    assert_eq!(at(5000.0), (4, 1));
    assert_eq!(
        at(5000.0 + 3.0 * 400.0 + 1.0),
        (5, 1),
        "3/4 after the third red line"
    );
}

#[test]
fn r4_time_axis_converts_both_ways_and_zooms_around_a_point() {
    let mut axis = TimeAxis::default();
    axis.end_ms = 120_000.0;
    axis.set_left(t(1000.0));
    axis.set_zoom(0.5);
    assert_eq!(axis.x(t(1200.0)), 100.0);
    assert!(close(axis.time(100.0), 1200.0));
    let anchor = axis.time(300.0);
    axis.zoom_at(2.0, 300.0);
    assert!(
        close(axis.time(300.0), anchor.ms()),
        "the time under the pointer stays put"
    );
    assert_eq!(axis.zoom(), 1.0);
    axis.set_zoom(50.0);
    assert_eq!(axis.zoom(), TimeAxis::MAX_ZOOM);
    axis.set_left(t(-9000.0));
    assert_eq!(axis.left().ms(), TimeAxis::MIN_LEFT_MS);
    axis.set_left(t(1e9));
    assert_eq!(axis.left().ms(), 120_000.0);
    assert!((TimeAxis::slider_from_zoom(TimeAxis::zoom_from_slider(0.3)) - 0.3).abs() < 1e-12);
    assert!((TimeAxis::zoom_from_slider(0.0) - TimeAxis::MIN_ZOOM).abs() < 1e-12);
}

#[test]
fn r4a_snap_switches_on_and_off_with_grid_grid_relative_and_objects_modes() {
    let map = tempo();
    let objects = [t(300.0), t(1100.0), t(1900.0)];
    let mut snap = Snap {
        on: false,
        mode: SnapMode::Grid,
        spacing: SnapSpacing::fraction(4).unwrap(),
    };
    assert_eq!(
        snap.time(&map, &objects, t(1010.0)),
        t(1010.0),
        "off: no snapping"
    );
    snap.on = true;
    assert!(close(snap.time(&map, &objects, t(1010.0)), 1000.0));
    // Grid: dragging an item at 1010 by +130 lands on the grid line 1125.
    assert!((snap.offset(&map, &objects, t(1010.0), 130.0) - 115.0).abs() < 1e-6);
    // Grid relative: by whole steps, keeping the 10 ms offset from its own line.
    snap.mode = SnapMode::GridRelative;
    assert!((snap.offset(&map, &objects, t(1010.0), 130.0) - 125.0).abs() < 1e-6);
    // Objects: to the closest object time the editor supplies.
    snap.mode = SnapMode::Objects;
    assert_eq!(snap.time(&map, &objects, t(1400.0)), t(1100.0));
    assert_eq!(snap.time(&map, &objects, t(1600.0)), t(1900.0));
    assert_eq!(
        snap.time(&map, &[], t(1600.0)),
        t(1600.0),
        "no objects: unchanged"
    );
}

#[test]
fn r5_locators_and_the_cycle_switch_are_plain_times() {
    let mut l = Locators::default();
    assert_eq!(l.left, None);
    l.set_left(t(1000.0));
    l.set_right(t(500.0));
    assert!(l.is_reversed());
    l.to_range(t(4000.0), t(2000.0));
    assert_eq!((l.left, l.right), (Some(t(2000.0)), Some(t(4000.0))));
    assert!(!l.cycle_on);
    l.toggle_cycle();
    assert!(l.cycle_on);
    l.toggle_cycle();
    assert_eq!(l.loop_range(t(100.0), t(900.0)), t(100.0));
    assert!(l.cycle_on);
}

#[test]
fn r5_bookmarks_stay_sorted_with_optional_labels() {
    let mut b = BookmarkList::new([t(3000.0), t(1000.0)]);
    assert!(b.add(t(2000.0), Some("drop".into())));
    assert!(!b.add(t(2000.4), None), "one bookmark per time");
    let times: Vec<f64> = b.items().iter().map(|x| x.time.ms()).collect();
    assert_eq!(times, [1000.0, 2000.0, 3000.0]);
    assert_eq!(b.items()[1].label.as_deref(), Some("drop"));
    assert_eq!(b.next(t(2000.0)), Some(t(3000.0)));
    assert_eq!(b.previous(t(2000.0)), Some(t(1000.0)));
    assert_eq!(b.previous(t(1000.0)), None);
    assert!(b.remove_at(t(2000.0)));
    assert!(!b.remove_at(t(2500.0)));
    assert_eq!(b.items().len(), 2);
}

#[test]
fn r4_grid_steps_back_across_a_red_line_land_on_the_earlier_sections_grid() {
    let map = TempoMap::new(vec![section(0.0, 1000.0, 4), section(4000.0, 200.0, 4)]);
    let whole = SnapSpacing::fraction(1).unwrap();
    assert!(close(map.grid_step(t(4000.0), -1, whole), 3000.0));
    assert!(close(map.grid_step(t(3000.0), 1, whole), 4000.0));
    assert!(
        close(map.grid_step(t(3500.0), 1, whole), 4000.0),
        "the red line comes first"
    );
    assert!(close(map.grid_step(t(4000.0), 1, whole), 4200.0));
}
