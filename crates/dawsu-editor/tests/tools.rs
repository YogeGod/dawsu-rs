//! The mouse tools driven by scripted pointer input against a fake editor (Responsibility 6).

use dawsu_editor::keys::Mods;
use dawsu_editor::tempo::{Snap, SnapSpacing, TempoMap, TimingSection};
use dawsu_editor::tools::{
    PointerButton, PointerInput, PointerSurface, TimeRange, ToolBox, ToolKind, ToolPreview,
    ToolTarget,
};
use dawsu_editor::{EditHistory, Selection};
use dawsu_foundation::SongTime;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
struct Hit {
    id: u32,
    time: f64,
    row: i32,
    value: i32,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Doc {
    hits: Vec<Hit>,
    next_id: u32,
}

/// A fake editor: 120 BPM, snap 1/4 (125 ms), hits 20 ms wide for hit testing.
struct Fake {
    history: EditHistory<Doc>,
    selection: Selection<u32>,
    tempo: TempoMap,
    snap: Snap,
    muted: HashSet<u32>,
    auditioned: Vec<u32>,
    pan: (f64, f64),
    range: Option<TimeRange>,
    pickers: u32,
    preview: ToolPreview,
}

impl Fake {
    fn new(hits: &[(f64, i32)]) -> Fake {
        let doc = Doc {
            hits: hits
                .iter()
                .enumerate()
                .map(|(i, (t, r))| Hit {
                    id: i as u32,
                    time: *t,
                    row: *r,
                    value: 50,
                })
                .collect(),
            next_id: hits.len() as u32,
        };
        Fake {
            history: EditHistory::new(doc),
            selection: Selection::new(),
            tempo: TempoMap::new(vec![TimingSection {
                start: SongTime::from_ms(0.0),
                beat_length_ms: 500.0,
                beats_per_bar: 4,
            }]),
            snap: Snap {
                spacing: SnapSpacing::fraction(4).unwrap(),
                ..Snap::default()
            },
            muted: HashSet::new(),
            auditioned: Vec::new(),
            pan: (0.0, 0.0),
            range: None,
            pickers: 0,
            preview: ToolPreview::None,
        }
    }

    fn hits(&self) -> Vec<(f64, i32)> {
        let mut v: Vec<(f64, i32)> = self
            .history
            .document()
            .hits
            .iter()
            .map(|h| (h.time, h.row))
            .collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    }

    fn change(&mut self, f: impl FnOnce(&mut Doc)) {
        self.history
            .edit(true, |d: &mut Doc| -> Result<(), ()> {
                f(d);
                Ok(())
            })
            .expect("a tool changes the document only inside a transaction");
    }
}

impl ToolTarget for Fake {
    type Item = u32;

    fn hit_test(&self, at: &PointerInput) -> Option<u32> {
        self.history
            .document()
            .hits
            .iter()
            .find(|h| h.row == at.row && (h.time - at.time.ms()).abs() <= 10.0)
            .map(|h| h.id)
    }

    fn in_range(&self, from: SongTime, to: SongTime, from_row: i32, to_row: i32) -> Vec<u32> {
        self.history
            .document()
            .hits
            .iter()
            .filter(|h| {
                h.time >= from.ms() && h.time <= to.ms() && h.row >= from_row && h.row <= to_row
            })
            .map(|h| h.id)
            .collect()
    }

    fn item_time(&self, item: &u32) -> SongTime {
        SongTime::from_ms(
            self.history
                .document()
                .hits
                .iter()
                .find(|h| h.id == *item)
                .unwrap()
                .time,
        )
    }

    fn selection(&mut self) -> &mut Selection<u32> {
        &mut self.selection
    }

    fn snap(&self, time: SongTime) -> SongTime {
        self.snap.time(&self.tempo, &[], time)
    }

    fn snap_offset(&self, from: SongTime, raw_ms: f64) -> f64 {
        self.snap.offset(&self.tempo, &[], from, raw_ms)
    }

    fn begin(&mut self, label: &str) {
        self.history.begin(label);
    }

    fn commit(&mut self) {
        self.history.commit();
    }

    fn cancel(&mut self) {
        self.history.cancel();
    }

    fn insert(&mut self, at: SongTime, row: i32) -> Option<u32> {
        let id = self.history.document().next_id;
        self.change(|d| {
            d.hits.push(Hit {
                id,
                time: at.ms(),
                row,
                value: 50,
            });
            d.next_id += 1;
        });
        Some(id)
    }

    fn delete(&mut self, items: &[u32]) {
        let items = items.to_vec();
        self.change(|d| d.hits.retain(|h| !items.contains(&h.id)));
    }

    fn move_by(&mut self, items: &[u32], delta_ms: f64, delta_rows: i32) {
        let items = items.to_vec();
        self.change(|d| {
            for h in d.hits.iter_mut().filter(|h| items.contains(&h.id)) {
                h.time += delta_ms;
                h.row += delta_rows;
            }
        });
    }

    fn set_values(&mut self, values: &[(u32, i32)]) {
        let values = values.to_vec();
        self.change(|d| {
            for (id, v) in &values {
                d.hits.iter_mut().find(|h| h.id == *id).unwrap().value = *v;
            }
        });
    }

    fn insert_value(&self) -> i32 {
        50
    }

    fn toggle_listen_mute(&mut self, item: &u32) {
        if !self.muted.remove(item) {
            self.muted.insert(*item);
        }
    }

    fn audition(&mut self, item: &u32) {
        self.auditioned.push(*item);
    }

    fn pan(&mut self, delta_ms: f64, delta_rows: f64) {
        self.pan.0 += delta_ms;
        self.pan.1 += delta_rows;
    }

    fn set_range(&mut self, range: Option<TimeRange>) {
        self.range = range;
    }

    fn open_tool_picker(&mut self, _at: &PointerInput) {
        self.pickers += 1;
    }

    fn show_preview(&mut self, preview: ToolPreview) {
        self.preview = preview;
    }
}

fn p(ms: f64, row: i32) -> PointerInput {
    PointerInput::at(ms, row)
}

fn with(mut e: PointerInput, f: impl FnOnce(&mut PointerInput)) -> PointerInput {
    f(&mut e);
    e
}

fn tool(kind: ToolKind, target: &mut Fake) -> ToolBox<u32> {
    let mut tb = ToolBox::new();
    tb.select_tool(kind, target);
    tb
}

#[test]
fn select_click_selects_and_ctrl_toggles_and_shift_adds() {
    let mut f = Fake::new(&[(500.0, 0), (1000.0, 1)]);
    let mut tb = tool(ToolKind::Select, &mut f);
    tb.down(&mut f, p(500.0, 0));
    tb.up(&mut f, p(500.0, 0));
    assert_eq!(f.selection.items(), &HashSet::from([0]));
    let shift = with(p(1000.0, 1), |e| e.mods = Mods::SHIFT);
    tb.down(&mut f, shift);
    tb.up(&mut f, shift);
    assert_eq!(f.selection.items(), &HashSet::from([0, 1]));
    assert_eq!(f.selection.primary(), Some(&0), "Shift keeps the anchor");
    let ctrl = with(p(500.0, 0), |e| e.mods = Mods::CTRL);
    tb.down(&mut f, ctrl);
    tb.up(&mut f, ctrl);
    assert_eq!(f.selection.items(), &HashSet::from([1]));
    assert!(f.history.steps().is_empty(), "selecting makes no undo step");
}

#[test]
fn select_drag_on_empty_space_selects_the_marquee_contents() {
    let mut f = Fake::new(&[(500.0, 0), (1000.0, 1), (2000.0, 1)]);
    let mut tb = tool(ToolKind::Select, &mut f);
    tb.down(&mut f, p(400.0, 0));
    tb.moved(&mut f, p(1200.0, 1));
    assert!(matches!(f.preview, ToolPreview::Marquee { .. }));
    tb.up(&mut f, p(1200.0, 1));
    assert_eq!(f.selection.items(), &HashSet::from([0, 1]));
    assert_eq!(f.preview, ToolPreview::None);
}

#[test]
fn select_drag_moves_the_selection_snapped_as_one_undo_step() {
    let mut f = Fake::new(&[(500.0, 0), (750.0, 0)]);
    let mut tb = tool(ToolKind::Select, &mut f);
    f.selection.replace([0, 1]);
    tb.down(&mut f, p(500.0, 0));
    tb.moved(&mut f, p(540.0, 0));
    assert_eq!(
        f.hits(),
        vec![(500.0, 0), (750.0, 0)],
        "less than half a step: no move yet"
    );
    tb.moved(&mut f, p(570.0, 0));
    assert_eq!(
        f.hits(),
        vec![(625.0, 0), (875.0, 0)],
        "visible while dragging, snapped to 1/4"
    );
    tb.moved(&mut f, p(760.0, 1));
    tb.up(&mut f, p(760.0, 1));
    assert_eq!(f.hits(), vec![(750.0, 1), (1000.0, 1)]);
    assert_eq!(f.history.steps(), vec!["Move"]);
    f.history.undo();
    assert_eq!(f.hits(), vec![(500.0, 0), (750.0, 0)]);
}

#[test]
fn esc_during_a_select_drag_restores_the_positions() {
    let mut f = Fake::new(&[(500.0, 0)]);
    let mut tb = tool(ToolKind::Select, &mut f);
    tb.down(&mut f, p(500.0, 0));
    tb.moved(&mut f, p(1000.0, 2));
    assert_eq!(f.hits(), vec![(1000.0, 2)]);
    tb.cancel(&mut f);
    assert_eq!(f.hits(), vec![(500.0, 0)]);
    assert!(f.history.steps().is_empty());
    assert!(!f.history.is_dirty());
    assert!(!tb.is_stroke_open());
}

#[test]
fn select_double_click_previews_a_hit_and_inserts_on_empty_grid() {
    let mut f = Fake::new(&[(500.0, 0)]);
    let mut tb = tool(ToolKind::Select, &mut f);
    let double = |ms, row| with(p(ms, row), |e| e.click_count = 2);
    tb.down(&mut f, double(500.0, 0));
    tb.up(&mut f, double(500.0, 0));
    assert_eq!(f.auditioned, vec![0]);
    assert!(f.history.steps().is_empty());
    tb.down(&mut f, double(1010.0, 3));
    tb.up(&mut f, double(1010.0, 3));
    assert_eq!(f.hits(), vec![(500.0, 0), (1000.0, 3)]);
    assert_eq!(f.history.steps(), vec!["Insert"]);
}

#[test]
fn grab_pans_time_and_rows_and_makes_no_step() {
    let mut f = Fake::new(&[]);
    let mut tb = tool(ToolKind::Grab, &mut f);
    tb.down(&mut f, p(1000.0, 5));
    tb.moved(&mut f, p(900.0, 4));
    tb.moved(&mut f, p(800.0, 4));
    tb.up(&mut f, p(800.0, 4));
    assert_eq!(f.pan, (200.0, 1.0));
    assert!(f.history.steps().is_empty());
}

#[test]
fn range_drag_marks_a_range_and_a_click_clears_it() {
    let mut f = Fake::new(&[(500.0, 0)]);
    let mut tb = tool(ToolKind::Range, &mut f);
    tb.down(&mut f, p(2000.0, 3));
    tb.moved(&mut f, p(1000.0, 1));
    tb.up(&mut f, p(1000.0, 1));
    assert_eq!(
        f.range,
        Some(TimeRange {
            from: SongTime::from_ms(1000.0),
            to: SongTime::from_ms(2000.0),
            from_row: 1,
            to_row: 3
        })
    );
    tb.down(&mut f, p(1500.0, 2));
    tb.up(&mut f, p(1500.0, 2));
    assert_eq!(f.range, None);
    assert!(f.history.steps().is_empty());
}

#[test]
fn a_draw_stroke_is_one_undo_step() {
    let mut f = Fake::new(&[]);
    let mut tb = tool(ToolKind::Draw, &mut f);
    tb.down(&mut f, p(10.0, 2));
    tb.moved(&mut f, p(130.0, 2));
    tb.moved(&mut f, p(140.0, 5)); // same grid line: no second hit; stays on the stroke's row
    tb.moved(&mut f, p(250.0, 2));
    tb.up(&mut f, p(250.0, 2));
    assert_eq!(f.hits(), vec![(0.0, 2), (125.0, 2), (250.0, 2)]);
    assert_eq!(f.history.steps(), vec!["Draw"]);
}

#[test]
fn fast_draw_drag_fills_every_intermediate_snap_line_without_skipping() {
    let mut f = Fake::new(&[]);
    let mut tb = tool(ToolKind::Draw, &mut f);
    // Single fast move jumping across 4 quarter-beat grid lines (0 -> 500 ms at 125 ms spacing).
    tb.down(&mut f, p(10.0, 1));
    tb.moved(&mut f, p(505.0, 1));
    tb.up(&mut f, p(505.0, 1));
    assert_eq!(
        f.hits(),
        vec![(0.0, 1), (125.0, 1), (250.0, 1), (375.0, 1), (500.0, 1)],
        "every intermediate 1/4 snap line must be filled"
    );
    assert_eq!(f.history.steps(), vec!["Draw"]);

    // Backward fast drag across 3 snap lines on another row (1000 -> 625 ms).
    tb.down(&mut f, p(1000.0, 2));
    tb.moved(&mut f, p(620.0, 2));
    tb.up(&mut f, p(620.0, 2));
    assert_eq!(
        f.hits(),
        vec![
            (0.0, 1),
            (125.0, 1),
            (250.0, 1),
            (375.0, 1),
            (500.0, 1),
            (625.0, 2),
            (750.0, 2),
            (875.0, 2),
            (1000.0, 2),
        ]
    );
    assert_eq!(f.history.steps(), vec!["Draw", "Draw"]);
}

#[test]
fn fast_draw_drag_with_snap_off_does_not_spam_intermediate_hits() {
    let mut f = Fake::new(&[]);
    f.snap.on = false;
    let mut tb = tool(ToolKind::Draw, &mut f);
    tb.down(&mut f, p(100.0, 0));
    tb.moved(&mut f, p(600.0, 0));
    tb.up(&mut f, p(600.0, 0));
    assert_eq!(f.hits(), vec![(100.0, 0), (600.0, 0)]);
    assert_eq!(f.history.steps(), vec!["Draw"]);
}

#[test]
fn a_draw_stroke_starting_on_a_hit_erases() {
    let mut f = Fake::new(&[(500.0, 0), (625.0, 0), (2000.0, 0)]);
    let mut tb = tool(ToolKind::Draw, &mut f);
    tb.down(&mut f, p(505.0, 0));
    tb.moved(&mut f, p(620.0, 0));
    tb.up(&mut f, p(620.0, 0));
    assert_eq!(f.hits(), vec![(2000.0, 0)]);
    assert_eq!(f.history.steps(), vec!["Erase"]);
}

#[test]
fn an_erase_stroke_deletes_every_hit_crossed_in_one_step_and_drops_them_from_the_selection() {
    let mut f = Fake::new(&[(500.0, 0), (700.0, 1), (900.0, 1), (3000.0, 1)]);
    f.selection.replace([1, 3]);
    let mut tb = tool(ToolKind::Erase, &mut f);
    tb.down(&mut f, p(400.0, 0));
    tb.moved(&mut f, p(1000.0, 1));
    tb.up(&mut f, p(1000.0, 1));
    assert_eq!(f.hits(), vec![(3000.0, 1)]);
    assert_eq!(f.history.steps(), vec!["Erase"]);
    assert_eq!(f.selection.items(), &HashSet::from([3]));
}

#[test]
fn mute_is_listening_only_never_an_undo_step_or_a_saved_change() {
    let mut f = Fake::new(&[(500.0, 0)]);
    let mut tb = tool(ToolKind::Mute, &mut f);
    tb.down(&mut f, p(500.0, 0));
    tb.up(&mut f, p(500.0, 0));
    assert!(f.muted.contains(&0));
    assert!(f.history.steps().is_empty());
    assert!(!f.history.is_dirty());
    tb.down(&mut f, p(500.0, 0));
    tb.up(&mut f, p(500.0, 0));
    assert!(f.muted.is_empty());
}

#[test]
fn ramp_sets_values_along_the_ramp_as_one_step_and_acts_as_select_in_the_grid() {
    let mut f = Fake::new(&[(0.0, 0), (500.0, 1), (1000.0, 2), (3000.0, 0)]);
    let mut tb = tool(ToolKind::Ramp, &mut f);
    let lane = |ms, v| {
        with(p(ms, 0), |e| {
            e.surface = PointerSurface::ValueLane;
            e.row_value = Some(v);
        })
    };
    tb.down(&mut f, lane(0.0, 20.0));
    tb.moved(&mut f, lane(1000.0, 80.0));
    assert!(matches!(f.preview, ToolPreview::RampLine { .. }));
    tb.up(&mut f, lane(1000.0, 80.0));
    let values: Vec<i32> = f.history.document().hits.iter().map(|h| h.value).collect();
    assert_eq!(values, vec![20, 50, 80, 50]);
    assert_eq!(f.history.steps(), vec!["Ramp"]);
    // In the grid, the Ramp tool selects.
    tb.down(&mut f, p(3000.0, 0));
    tb.up(&mut f, p(3000.0, 0));
    assert_eq!(f.selection.items(), &HashSet::from([3]));
}

#[test]
fn right_click_opens_the_tool_picker_on_other_tools_and_empty_select() {
    for kind in [
        ToolKind::Grab,
        ToolKind::Range,
        ToolKind::Erase,
        ToolKind::Mute,
    ] {
        let mut f = Fake::new(&[(500.0, 0)]);
        let mut tb = tool(kind, &mut f);
        let right = with(p(500.0, 0), |e| e.button = PointerButton::Right);
        tb.down(&mut f, right);
        tb.up(&mut f, right);
        assert_eq!(f.pickers, 1, "{kind:?}");
        assert_eq!(
            f.hits(),
            vec![(500.0, 0)],
            "{kind:?} must not delete on right-click"
        );
        assert!(f.history.steps().is_empty());
    }
    // Select (and Ramp in the grid, which acts as Select) opens the tool picker on empty space.
    for kind in [ToolKind::Select, ToolKind::Ramp] {
        let mut f = Fake::new(&[(500.0, 0)]);
        let mut tb = tool(kind, &mut f);
        let right = with(p(1000.0, 0), |e| e.button = PointerButton::Right);
        tb.down(&mut f, right);
        tb.up(&mut f, right);
        assert_eq!(f.pickers, 1, "{kind:?} on empty space");
        assert_eq!(f.hits(), vec![(500.0, 0)]);
        assert!(f.history.steps().is_empty());
    }
}

#[test]
fn draw_delete_drag_erases_zoomed_out_and_fast_crossed_hits_in_own_row_as_one_step() {
    // Hits at 500, 625, 750, 875 on row 0, plus a hit at 750 on row 1 and 2000 on row 0.
    let mut f = Fake::new(&[
        (500.0, 0),
        (625.0, 0),
        (750.0, 0),
        (875.0, 0),
        (750.0, 1),
        (2000.0, 0),
    ]);
    f.selection.replace([0, 1, 4]);
    let mut tb = tool(ToolKind::Draw, &mut f);
    // Click 35 ms away from 500 ms (beyond 10 ms hit_test, but snaps to 500 ms on 1/4 grid).
    tb.down(&mut f, p(535.0, 0));
    // Fast drag straight to 870 ms (snaps to 875 ms) while drifting to row 1.
    tb.moved(&mut f, p(870.0, 1));
    tb.up(&mut f, p(870.0, 1));
    assert_eq!(
        f.hits(),
        vec![(750.0, 1), (2000.0, 0)],
        "erases every crossed hit on the starting row, leaves other rows untouched"
    );
    assert_eq!(f.history.steps(), vec!["Erase"]);
    assert_eq!(f.selection.items(), &HashSet::from([4]));
    assert_eq!(f.preview, ToolPreview::None);

    // Undo restores all four deleted hits at once.
    f.history.undo();
    assert_eq!(f.hits().len(), 6);
}

#[test]
fn draw_vertical_drag_adjusts_volume_for_stroke_notes_and_stops_on_release() {
    let mut f = Fake::new(&[(2000.0, 0)]);
    // Even if something was previously selected, Draw clears selection and never selects drawn notes.
    f.selection.replace([0]);
    let mut tb = tool(ToolKind::Draw, &mut f);
    let at_y = |ms, row, y| with(p(ms, row), |e| e.y = y);
    // Press at y = 100 -> creates hit at 0 ms with base volume 50.
    tb.down(&mut f, at_y(0.0, 0, 100.0));
    assert!(f.selection.is_empty(), "Draw must not leave selection active");
    // Drag across 250 ms while drifting vertically -> re-anchors without changing volume while drawing.
    tb.moved(&mut f, at_y(250.0, 0, 80.0));
    let drawn_values = |f: &Fake| -> Vec<(f64, i32)> {
        let mut v: Vec<(f64, i32)> = f
            .history
            .document()
            .hits
            .iter()
            .filter(|h| h.time < 1000.0)
            .map(|h| (h.time, h.value))
            .collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    };
    assert_eq!(drawn_values(&f), vec![(0.0, 50), (125.0, 50), (250.0, 50)]);
    // Stop horizontal drawing and deliberately drag UP by 78 px (60 px past 18 px deadzone -> +21 volume = 71).
    tb.moved(&mut f, at_y(250.0, 0, 2.0));
    assert_eq!(drawn_values(&f), vec![(0.0, 71), (125.0, 71), (250.0, 71)]);
    // Drag far DOWN -> clamps to 5, and existing untouched hit at 2000 ms keeps 50.
    tb.moved(&mut f, at_y(250.0, 0, 500.0));
    assert_eq!(drawn_values(&f), vec![(0.0, 5), (125.0, 5), (250.0, 5)]);
    assert_eq!(f.history.document().hits[0].value, 50);
    // Drag far UP -> clamps to 100.
    tb.moved(&mut f, at_y(250.0, 0, -400.0));
    assert_eq!(drawn_values(&f), vec![(0.0, 100), (125.0, 100), (250.0, 100)]);
    tb.up(&mut f, at_y(250.0, 0, -400.0));
    assert!(f.selection.is_empty());
    assert_eq!(f.history.steps(), vec!["Draw"]);
    // Moving after release must not modify drawn notes.
    tb.moved(&mut f, at_y(250.0, 0, 500.0));
    assert_eq!(drawn_values(&f), vec![(0.0, 100), (125.0, 100), (250.0, 100)]);
}

#[test]
fn alt_draw_from_select_adjusts_volume_and_never_selects_drawn_notes() {
    let mut f = Fake::new(&[]);
    let mut tb = tool(ToolKind::Select, &mut f);
    let alt_y = |ms, row, y| {
        with(p(ms, row), |e| {
            e.mods = Mods::ALT;
            e.y = y;
        })
    };
    tb.down(&mut f, alt_y(0.0, 1, 100.0));
    tb.moved(&mut f, alt_y(250.0, 1, 100.0));
    // After finishing horizontal stroke, drag DOWN by 78 px (60 px past 18 px deadzone -> 50 - 21 = 29).
    tb.moved(&mut f, alt_y(250.0, 1, 178.0));
    tb.up(&mut f, alt_y(250.0, 1, 178.0));
    let mut values: Vec<(f64, i32)> = f
        .history
        .document()
        .hits
        .iter()
        .map(|h| (h.time, h.value))
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(values, vec![(0.0, 29), (125.0, 29), (250.0, 29)]);
    assert!(f.selection.is_empty());
    assert_eq!(tb.active(), ToolKind::Select);
}

#[test]
fn draw_never_selects_or_starts_marquee_or_moves_hits() {
    let mut f = Fake::new(&[(500.0, 0), (1000.0, 0)]);
    let mut tb = tool(ToolKind::Draw, &mut f);
    // Paint drag over empty space: no marquee, no selection change.
    tb.down(&mut f, p(0.0, 0));
    tb.moved(&mut f, p(250.0, 0));
    assert_eq!(f.preview, ToolPreview::None);
    assert!(f.selection.is_empty());
    tb.up(&mut f, p(250.0, 0));
    assert!(f.selection.is_empty());
    assert_eq!(f.preview, ToolPreview::None);

    // Click on an existing hit with Draw: erases instead of selecting/moving.
    tb.down(&mut f, p(500.0, 0));
    tb.moved(&mut f, p(625.0, 0));
    tb.up(&mut f, p(625.0, 0));
    assert!(f.selection.is_empty());
    assert!(!f.hits().contains(&(500.0, 0)));
    assert!(!f.hits().contains(&(625.0, 0)));
}

#[test]
fn draw_right_click_drag_erases_crossed_hits_in_one_step() {
    let mut f = Fake::new(&[(500.0, 0), (750.0, 1), (1000.0, 1), (2000.0, 0)]);
    let mut tb = tool(ToolKind::Draw, &mut f);
    let right = |ms, row| with(p(ms, row), |e| e.button = PointerButton::Right);
    tb.down(&mut f, right(400.0, 0));
    tb.moved(&mut f, right(1050.0, 1));
    tb.up(&mut f, right(1050.0, 1));
    assert_eq!(f.hits(), vec![(2000.0, 0)]);
    assert_eq!(f.history.steps(), vec!["Erase"]);
    assert_eq!(f.pickers, 0);
}

#[test]
fn select_right_click_on_hit_deletes_immediately_and_drag_erases_in_one_step() {
    let mut f = Fake::new(&[(500.0, 0), (750.0, 0), (1000.0, 1), (2000.0, 0)]);
    f.selection.replace([0, 3]);
    let mut tb = tool(ToolKind::Select, &mut f);
    let right = |ms, row| with(p(ms, row), |e| e.button = PointerButton::Right);
    tb.down(&mut f, right(500.0, 0));
    assert_eq!(
        f.hits(),
        vec![(750.0, 0), (1000.0, 1), (2000.0, 0)],
        "right-clicking a hit in Select deletes it immediately"
    );
    tb.moved(&mut f, right(1000.0, 1));
    tb.up(&mut f, right(1000.0, 1));
    assert_eq!(f.hits(), vec![(2000.0, 0)]);
    assert_eq!(f.history.steps(), vec!["Erase"]);
    assert_eq!(f.selection.items(), &HashSet::from([3]));
    assert_eq!(f.pickers, 0);

    // Undo restores all three erased hits in one step.
    f.history.undo();
    assert_eq!(f.hits().len(), 4);
}

#[test]
fn alt_draws_with_any_tool() {
    let mut f = Fake::new(&[]);
    let mut tb = tool(ToolKind::Select, &mut f);
    let alt = with(p(250.0, 1), |e| e.mods = Mods::ALT);
    tb.down(&mut f, alt);
    tb.up(&mut f, alt);
    assert_eq!(f.hits(), vec![(250.0, 1)]);
    assert_eq!(tb.active(), ToolKind::Select);
}

#[test]
fn switching_tool_mid_stroke_rolls_the_stroke_back() {
    let mut f = Fake::new(&[]);
    let mut tb = tool(ToolKind::Draw, &mut f);
    tb.down(&mut f, p(0.0, 0));
    tb.moved(&mut f, p(125.0, 0));
    tb.select_tool(ToolKind::Erase, &mut f);
    assert!(f.hits().is_empty());
    assert!(f.history.steps().is_empty());
}

#[test]
fn every_tool_has_its_3_1_x_command_and_mouse_actions() {
    let ids: Vec<String> = ToolKind::ALL.iter().map(|k| k.command().id).collect();
    assert_eq!(
        ids,
        [
            "toolSelect",
            "toolGrab",
            "toolRange",
            "toolDraw",
            "toolErase",
            "toolMute",
            "toolRamp"
        ]
    );
    assert!(ToolKind::ALL.iter().all(|k| !k.mouse_actions().is_empty()));
}

#[test]
fn shift_click_selects_the_range_from_the_primary_on_the_clicked_row_only() {
    let mut f = Fake::new(&[
        (1000.0, 0),
        (1500.0, 1),
        (2000.0, 2),
        (2500.0, 0),
        (3000.0, 0),
        (4000.0, 0),
    ]);
    let mut tb = tool(ToolKind::Select, &mut f);
    tb.down(&mut f, p(1000.0, 0));
    tb.up(&mut f, p(1000.0, 0));
    let shift = with(p(3000.0, 0), |e| e.mods = Mods::SHIFT);
    tb.down(&mut f, shift);
    tb.up(&mut f, shift);
    assert_eq!(
        f.selection.items(),
        &HashSet::from([0, 3, 4]),
        "hits on other rows in between stay unselected"
    );
    assert_eq!(
        f.selection.primary(),
        Some(&0),
        "the anchor stays for the next Shift+click"
    );
}

#[test]
fn shift_click_on_another_row_selects_that_row_from_the_anchor_time() {
    let mut f = Fake::new(&[(1000.0, 0), (1500.0, 0), (1200.0, 1), (1800.0, 1), (2500.0, 1)]);
    let mut tb = tool(ToolKind::Select, &mut f);
    tb.down(&mut f, p(1000.0, 0));
    tb.up(&mut f, p(1000.0, 0));
    let shift = with(p(1800.0, 1), |e| e.mods = Mods::SHIFT);
    tb.down(&mut f, shift);
    tb.up(&mut f, shift);
    assert_eq!(f.selection.items(), &HashSet::from([0, 2, 3]));
    assert_eq!(f.selection.primary(), Some(&0));
}

#[test]
fn shift_click_keeps_hits_selected_on_other_rows_and_ctrl_shift_click_toggles_one() {
    // 0 and 3 on row 0, 1 and 2 on row 1, 4 on row 2.
    let mut f = Fake::new(&[(1000.0, 0), (1000.0, 1), (2000.0, 1), (2000.0, 0), (1500.0, 2)]);
    let mut tb = tool(ToolKind::Select, &mut f);
    f.selection.replace([4]);
    let ctrl = with(p(1000.0, 0), |e| e.mods = Mods::CTRL);
    tb.down(&mut f, ctrl);
    tb.up(&mut f, ctrl);
    let shift = with(p(2000.0, 0), |e| e.mods = Mods::SHIFT);
    tb.down(&mut f, shift);
    tb.up(&mut f, shift);
    assert_eq!(
        f.selection.items(),
        &HashSet::from([4, 0, 3]),
        "Shift adds the row's range and keeps what was selected"
    );
    let ctrl_shift = with(p(2000.0, 1), |e| e.mods = Mods::CTRL | Mods::SHIFT);
    tb.down(&mut f, ctrl_shift);
    tb.up(&mut f, ctrl_shift);
    assert_eq!(
        f.selection.items(),
        &HashSet::from([4, 0, 3, 2]),
        "Ctrl+Shift toggles the clicked hit only, as before"
    );
}

#[test]
fn removing_the_primary_passes_it_to_the_last_selected_item() {
    let mut f = Fake::new(&[(500.0, 0), (1000.0, 0), (1500.0, 0)]);
    f.selection.replace([0, 1, 2]);
    f.selection.remove(&0);
    assert_eq!(f.selection.primary(), Some(&2));
}

#[test]
fn esc_on_an_erase_stroke_or_a_marquee_puts_the_selection_back() {
    let mut f = Fake::new(&[(500.0, 0), (1000.0, 0)]);
    f.selection.replace([0, 1]);
    let mut tb = tool(ToolKind::Erase, &mut f);
    tb.down(&mut f, p(500.0, 0));
    tb.cancel(&mut f);
    assert_eq!(f.hits().len(), 2);
    assert_eq!(f.selection.items(), &HashSet::from([0, 1]));
    assert_eq!(f.selection.primary(), Some(&0));

    let mut tb = tool(ToolKind::Select, &mut f);
    tb.down(&mut f, p(3000.0, 0));
    tb.cancel(&mut f);
    assert_eq!(f.selection.items(), &HashSet::from([0, 1]));
}
