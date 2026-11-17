//! The mouse tools as state machines over pointer input, testable without the UI (Responsibility 6).
//!
//! The tool box drives one stroke at a time against a `ToolTarget` the composer implements. Every
//! change a stroke makes runs in one edit transaction (begin on Down, commit on Up, cancel on Esc),
//! so a drag is one undo step that is visible while it runs and fully rolled back when cancelled.
//! Right-click opens the tool picker and never deletes. Mute is listening-only: no transaction.
//! Behaviour follows the 3.1.x arrange view and piano roll (the tool table in
//! `later/design-details.md`).

use crate::commands::{CommandDefinition, MouseAction};
use crate::keys::{Key, KeyCombination as K, Mods};
use crate::selection::Selection;
use crate::volume_ramp::{RampShape, ramp};
use dawsu_foundation::SongTime;
use std::hash::Hash;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

/// Where the pointer is: the grid of rows, or the value lane under it (volume or velocity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerSurface {
    #[default]
    Grid,
    ValueLane,
}

/// A pointer event in the kernel's own values. `row_value` is the value under the pointer in the
/// value lane (volume 1-100, velocity).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerInput {
    pub time: SongTime,
    pub row: i32,
    pub row_value: Option<f64>,
    pub surface: PointerSurface,
    pub button: PointerButton,
    pub mods: Mods,
    pub click_count: u32,
    pub x: f64,
    pub y: f64,
}

impl PointerInput {
    /// A plain left press in the grid.
    pub fn at(time_ms: f64, row: i32) -> PointerInput {
        PointerInput {
            time: SongTime::from_ms(time_ms),
            row,
            row_value: None,
            surface: PointerSurface::Grid,
            button: PointerButton::Left,
            mods: Mods::NONE,
            click_count: 1,
            x: 0.0,
            y: 0.0,
        }
    }
}

/// What a tool asks the view to draw while a stroke runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToolPreview {
    None,
    Marquee {
        from: SongTime,
        to: SongTime,
        from_row: i32,
        to_row: i32,
    },
    RangeBand {
        from: SongTime,
        to: SongTime,
        from_row: i32,
        to_row: i32,
    },
    RampLine {
        from: (SongTime, i32),
        to: (SongTime, i32),
        shape: RampShape,
    },
}

/// A time range across rows, made by the Range tool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeRange {
    pub from: SongTime,
    pub to: SongTime,
    pub from_row: i32,
    pub to_row: i32,
}

/// What an editor gives the tools: hit testing, its selection, snapping, its edit history's
/// transaction calls and the document operations a tool may ask for. Implemented by the composers.
pub trait ToolTarget {
    type Item: Clone + Eq + Hash;

    fn hit_test(&self, at: &PointerInput) -> Option<Self::Item>;
    /// Items from `from` to `to` (inclusive) in rows `from_row..=to_row`.
    fn in_range(&self, from: SongTime, to: SongTime, from_row: i32, to_row: i32)
    -> Vec<Self::Item>;
    fn item_time(&self, item: &Self::Item) -> SongTime;
    fn selection(&mut self) -> &mut Selection<Self::Item>;
    /// The editor's snap applied to a time (off, Grid, Grid relative or Objects).
    fn snap(&self, time: SongTime) -> SongTime;
    /// The snapped offset for dragging an item at `from` by `raw_ms`.
    fn snap_offset(&self, from: SongTime, raw_ms: f64) -> f64;

    /// The edit history's transaction calls.
    fn begin(&mut self, label: &str);
    fn commit(&mut self);
    fn cancel(&mut self);

    /// Document operations, inside the open transaction.
    fn insert(&mut self, at: SongTime, row: i32) -> Option<Self::Item>;
    fn delete(&mut self, items: &[Self::Item]);
    fn move_by(&mut self, items: &[Self::Item], delta_ms: f64, delta_rows: i32);
    fn set_values(&mut self, values: &[(Self::Item, i32)]);

    /// Listening and view actions: never undo steps.
    fn toggle_listen_mute(&mut self, item: &Self::Item);
    /// Playhead to the item and preview it (Select's double-click).
    fn audition(&mut self, item: &Self::Item);
    fn pan(&mut self, delta_ms: f64, delta_rows: f64);
    fn set_range(&mut self, range: Option<TimeRange>);
    fn open_tool_picker(&mut self, at: &PointerInput);
    fn show_preview(&mut self, preview: ToolPreview);
}

/// The kernel's tools. Keysound's Split and Glue live in the composer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolKind {
    #[default]
    Select,
    Grab,
    Range,
    Draw,
    Erase,
    Mute,
    Ramp,
}

impl ToolKind {
    pub const ALL: [ToolKind; 7] = [
        ToolKind::Select,
        ToolKind::Grab,
        ToolKind::Range,
        ToolKind::Draw,
        ToolKind::Erase,
        ToolKind::Mute,
        ToolKind::Ramp,
    ];

    /// The command that selects this tool.
    pub fn command(self) -> CommandDefinition {
        let (id, description, key) = match self {
            ToolKind::Select => ("toolSelect", "Select and move", Some(Key::D1)),
            ToolKind::Grab => ("toolGrab", "Grab: pan time and lanes", Some(Key::D2)),
            ToolKind::Range => (
                "toolRange",
                "Range: drag a time range across lanes, then Edit range",
                None,
            ),
            ToolKind::Draw => (
                "toolDraw",
                "Draw (hold Alt to draw with any tool)",
                Some(Key::D3),
            ),
            ToolKind::Erase => ("toolErase", "Erase", Some(Key::D4)),
            ToolKind::Mute => (
                "toolMute",
                "Mute hits while listening (never saved)",
                Some(Key::D5),
            ),
            ToolKind::Ramp => ("toolRamp", "Ramp volumes in the volume lane", Some(Key::D6)),
        };
        let def = CommandDefinition::new(id, description)
            .keys(&key.map(K::plain).into_iter().collect::<Vec<_>>());
        // The osu! preset: 2 is the placing tool (Draw), so Grab has no key there (old `Keymap.OsuKeys`).
        match self {
            ToolKind::Draw => def.osu(&[K::plain(Key::D2)]),
            ToolKind::Grab => def.osu(&[]),
            _ => def,
        }
    }

    /// The mouse actions this tool owns, for help.
    pub fn mouse_actions(self) -> &'static [MouseAction] {
        macro_rules! m {
            ($g:expr, $d:expr) => {
                MouseAction {
                    gesture: $g,
                    description: $d,
                }
            };
        }
        match self {
            ToolKind::Select => &[
                m!(
                    "Click / drag (Select)",
                    "Select items, drag a box, or drag items to move them"
                ),
                m!(
                    "Ctrl+click / Shift+click",
                    "Toggle / add an item to the selection"
                ),
                m!(
                    "Double-click an item (Select tool)",
                    "Move the playhead to it and hear it"
                ),
                m!(
                    "Double-click empty space (Select tool)",
                    "Insert an item there, as Draw would"
                ),
                m!("Right-click in the grid", "Pick a tool"),
            ],
            ToolKind::Grab => &[m!("Drag (Grab)", "Pan time and rows")],
            ToolKind::Range => &[m!(
                "Drag (Range)",
                "Mark a time range over the rows dragged across; a click clears it"
            )],
            ToolKind::Draw => &[
                m!(
                    "Click / drag (Draw)",
                    "Paint items; a stroke starting on an item erases"
                ),
                m!("Alt+drag", "Draw with any tool"),
            ],
            ToolKind::Erase => &[m!(
                "Click / drag (Erase)",
                "Delete every item the pointer crosses"
            )],
            ToolKind::Mute => &[m!(
                "Click an item (Mute)",
                "Silence it while listening (never saved)"
            )],
            ToolKind::Ramp => &[m!("Drag in the value lane (Ramp)", "Draw a ramp")],
        }
    }
}

/// The open stroke: what the tool box is doing between Down and Up.
#[derive(Debug, Clone, PartialEq)]
enum Stroke<I> {
    /// Select pressed on an item: moves the selection once dragged.
    Move {
        start: PointerInput,
        anchor: SongTime,
        applied_ms: f64,
        applied_rows: i32,
        moved: bool,
        select_only: Option<I>,
    },
    Marquee {
        start: PointerInput,
        additive: bool,
        last: PointerInput,
    },
    Pan {
        last: PointerInput,
    },
    Range {
        start: PointerInput,
        dragged: bool,
    },
    Paint {
        row: i32,
    },
    Wipe,
    Erase {
        last: PointerInput,
    },
    Ramp {
        start: (SongTime, i32),
        last: (SongTime, i32),
    },
}

/// One editor's active tool and its open stroke.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolBox<I> {
    active: ToolKind,
    stroke: Option<Stroke<I>>,
    /// The selection before the open stroke changed it, put back on cancel.
    saved_selection: Option<(Vec<I>, Option<I>)>,
    /// The Ramp tool's curve.
    pub ramp_shape: RampShape,
}

impl<I: Clone + Eq + Hash> ToolBox<I> {
    pub fn new() -> Self {
        ToolBox {
            active: ToolKind::Select,
            stroke: None,
            saved_selection: None,
            ramp_shape: RampShape::Linear,
        }
    }

    pub fn active(&self) -> ToolKind {
        self.active
    }

    /// Switches tool; a stroke in progress is cancelled first.
    pub fn select_tool<T: ToolTarget<Item = I>>(&mut self, tool: ToolKind, target: &mut T) {
        self.cancel(target);
        self.active = tool;
    }

    pub fn is_stroke_open(&self) -> bool {
        self.stroke.is_some()
    }

    pub fn down<T: ToolTarget<Item = I>>(&mut self, target: &mut T, e: PointerInput) {
        if self.stroke.is_some() {
            return;
        }
        if e.button == PointerButton::Right {
            target.open_tool_picker(&e);
            return;
        }
        if e.button != PointerButton::Left {
            return;
        }
        self.saved_selection = Some(target.selection().snapshot());
        // Alt draws with any tool, in the grid.
        let tool = if e.mods.alt() && e.surface == PointerSurface::Grid {
            ToolKind::Draw
        } else {
            self.active
        };
        // The Ramp tool acts as Select in the grid.
        let tool = if tool == ToolKind::Ramp && e.surface == PointerSurface::Grid {
            ToolKind::Select
        } else {
            tool
        };
        match tool {
            ToolKind::Select => self.select_down(target, e),
            ToolKind::Grab => self.stroke = Some(Stroke::Pan { last: e }),
            ToolKind::Range => {
                target.show_preview(ToolPreview::RangeBand {
                    from: e.time,
                    to: e.time,
                    from_row: e.row,
                    to_row: e.row,
                });
                self.stroke = Some(Stroke::Range {
                    start: e,
                    dragged: false,
                });
            }
            ToolKind::Draw => {
                if let Some(hit) = target.hit_test(&e) {
                    target.begin("Erase");
                    target.selection().remove(&hit);
                    target.delete(&[hit]);
                    self.stroke = Some(Stroke::Wipe);
                } else {
                    target.begin("Draw");
                    let at = target.snap(e.time);
                    target.insert(at, e.row);
                    self.stroke = Some(Stroke::Paint { row: e.row });
                }
            }
            ToolKind::Erase => {
                target.begin("Erase");
                if let Some(hit) = target.hit_test(&e) {
                    target.selection().remove(&hit);
                    target.delete(&[hit]);
                }
                self.stroke = Some(Stroke::Erase { last: e });
            }
            ToolKind::Mute => {
                if let Some(hit) = target.hit_test(&e) {
                    target.toggle_listen_mute(&hit);
                }
            }
            ToolKind::Ramp => {
                let value = e.row_value.unwrap_or(100.0).round() as i32;
                let start = (target.snap(e.time), value);
                target.show_preview(ToolPreview::RampLine {
                    from: start,
                    to: start,
                    shape: self.ramp_shape,
                });
                self.stroke = Some(Stroke::Ramp { start, last: start });
            }
        }
    }

    fn select_down<T: ToolTarget<Item = I>>(&mut self, target: &mut T, e: PointerInput) {
        let hit = target.hit_test(&e);
        if e.click_count >= 2 {
            match hit {
                Some(item) => target.audition(&item),
                None => {
                    target.begin("Insert");
                    let at = target.snap(e.time);
                    let inserted = target.insert(at, e.row);
                    target.commit();
                    if let Some(item) = inserted {
                        target.selection().replace([item]);
                    }
                }
            }
            return;
        }
        match hit {
            Some(item) => {
                let mut select_only = None;
                if e.mods.ctrl() {
                    target.selection().toggle(item.clone());
                } else if e.mods.shift() {
                    // shortcut: the range spans every row, as items carry no row here; narrow it
                    // when ToolTarget gets an item row.
                    let anchor = target.selection().primary().cloned();
                    if let Some(anchor) = anchor.filter(|a| *a != item) {
                        let (from, to) =
                            ordered(target.item_time(&anchor), target.item_time(&item));
                        for i in target.in_range(from, to, i32::MIN, i32::MAX) {
                            if i != anchor {
                                target.selection().add(i);
                            }
                        }
                        // Keep the anchor for the next Shift+click.
                        target.selection().add(anchor);
                    }
                    if !target.selection().contains(&item) {
                        target.selection().add(item.clone());
                    }
                } else if !target.selection().contains(&item) {
                    target.selection().replace([item.clone()]);
                } else {
                    // A plain click on a selected item keeps the selection for a drag; without a
                    // drag, Up selects only that item.
                    select_only = Some(item.clone());
                }
                if !target.selection().contains(&item) {
                    return; // Ctrl toggled it off: nothing to drag.
                }
                let anchor = target.item_time(&item);
                self.stroke = Some(Stroke::Move {
                    start: e,
                    anchor,
                    applied_ms: 0.0,
                    applied_rows: 0,
                    moved: false,
                    select_only,
                });
            }
            None => {
                let additive = e.mods.ctrl() || e.mods.shift();
                if !additive {
                    target.selection().clear();
                }
                self.stroke = Some(Stroke::Marquee {
                    start: e,
                    additive,
                    last: e,
                });
            }
        }
    }

    pub fn moved<T: ToolTarget<Item = I>>(&mut self, target: &mut T, e: PointerInput) {
        let Some(stroke) = self.stroke.as_mut() else {
            return;
        };
        match stroke {
            Stroke::Move {
                start,
                anchor,
                applied_ms,
                applied_rows,
                moved,
                ..
            } => {
                let delta_ms = target.snap_offset(*anchor, e.time.ms() - start.time.ms());
                let delta_rows = e.row - start.row;
                if !*moved {
                    if delta_ms == 0.0 && delta_rows == 0 {
                        return;
                    }
                    target.begin("Move");
                    *moved = true;
                }
                if delta_ms != *applied_ms || delta_rows != *applied_rows {
                    let items: Vec<I> = target.selection().items().iter().cloned().collect();
                    target.move_by(&items, delta_ms - *applied_ms, delta_rows - *applied_rows);
                    *applied_ms = delta_ms;
                    *applied_rows = delta_rows;
                }
            }
            Stroke::Marquee { start, last, .. } => {
                *last = e;
                target.show_preview(ToolPreview::Marquee {
                    from: start.time,
                    to: e.time,
                    from_row: start.row.min(e.row),
                    to_row: start.row.max(e.row),
                });
            }
            Stroke::Pan { last } => {
                target.pan(last.time.ms() - e.time.ms(), (last.row - e.row) as f64);
                *last = e;
            }
            Stroke::Range { start, dragged } => {
                *dragged = true;
                target.show_preview(ToolPreview::RangeBand {
                    from: start.time,
                    to: e.time,
                    from_row: start.row.min(e.row),
                    to_row: start.row.max(e.row),
                });
            }
            Stroke::Paint { row } => {
                let at = target.snap(e.time);
                let mut probe = e;
                probe.time = at;
                probe.row = *row;
                if target.hit_test(&probe).is_none() {
                    target.insert(at, *row);
                }
            }
            Stroke::Wipe => {
                if let Some(hit) = target.hit_test(&e) {
                    target.selection().remove(&hit);
                    target.delete(&[hit]);
                }
            }
            Stroke::Erase { last } => {
                let (from, to) = ordered(last.time, e.time);
                let crossed = target.in_range(from, to, last.row.min(e.row), last.row.max(e.row));
                if !crossed.is_empty() {
                    for item in &crossed {
                        target.selection().remove(item);
                    }
                    target.delete(&crossed);
                }
                *last = e;
            }
            Stroke::Ramp { start, last } => {
                let value = e.row_value.unwrap_or(last.1 as f64).round() as i32;
                *last = (target.snap(e.time), value);
                target.show_preview(ToolPreview::RampLine {
                    from: *start,
                    to: *last,
                    shape: self.ramp_shape,
                });
            }
        }
    }

    pub fn up<T: ToolTarget<Item = I>>(&mut self, target: &mut T, e: PointerInput) {
        self.saved_selection = None;
        let Some(stroke) = self.stroke.take() else {
            return;
        };
        match stroke {
            Stroke::Move {
                moved, select_only, ..
            } => {
                if moved {
                    target.commit();
                } else if let Some(item) = select_only {
                    target.selection().replace([item]);
                }
            }
            Stroke::Marquee {
                start, additive, ..
            } => {
                target.show_preview(ToolPreview::None);
                let (from, to) = ordered(start.time, e.time);
                let inside = target.in_range(from, to, start.row.min(e.row), start.row.max(e.row));
                if additive {
                    for item in inside {
                        target.selection().add(item);
                    }
                } else {
                    target.selection().replace(inside);
                }
            }
            Stroke::Pan { .. } => {}
            Stroke::Range { start, dragged } => {
                target.show_preview(ToolPreview::None);
                if dragged {
                    let (from, to) = ordered(start.time, e.time);
                    target.set_range(Some(TimeRange {
                        from,
                        to,
                        from_row: start.row.min(e.row),
                        to_row: start.row.max(e.row),
                    }));
                } else {
                    target.set_range(None);
                }
            }
            Stroke::Paint { .. } | Stroke::Wipe | Stroke::Erase { .. } => target.commit(),
            Stroke::Ramp { start, last } => {
                target.show_preview(ToolPreview::None);
                let (from, to) = ordered(start.0, last.0);
                let items: Vec<(f64, I)> = target
                    .in_range(from, to, i32::MIN, i32::MAX)
                    .into_iter()
                    .map(|i| (target.item_time(&i).ms(), i))
                    .collect();
                let values = ramp(
                    &items,
                    (start.0.ms(), start.1),
                    (last.0.ms(), last.1),
                    self.ramp_shape,
                );
                if !values.is_empty() {
                    target.begin("Ramp");
                    target.set_values(&values);
                    target.commit();
                }
            }
        }
    }

    /// Esc, focus lost or capture lost: rolls back the open stroke's transaction.
    pub fn cancel<T: ToolTarget<Item = I>>(&mut self, target: &mut T) {
        let saved = self.saved_selection.take();
        let Some(stroke) = self.stroke.take() else {
            return;
        };
        // Only strokes that changed the selection as a side effect put it back.
        if matches!(
            stroke,
            Stroke::Wipe
                | Stroke::Erase { .. }
                | Stroke::Marquee {
                    additive: false,
                    ..
                }
        ) && let Some(saved) = saved
        {
            target.selection().restore(saved);
        }
        match stroke {
            Stroke::Move { moved: true, .. }
            | Stroke::Paint { .. }
            | Stroke::Wipe
            | Stroke::Erase { .. } => target.cancel(),
            Stroke::Marquee { .. } | Stroke::Range { .. } | Stroke::Ramp { .. } => {
                target.show_preview(ToolPreview::None)
            }
            Stroke::Move { .. } | Stroke::Pan { .. } => {}
        }
    }
}

fn ordered(a: SongTime, b: SongTime) -> (SongTime, SongTime) {
    if b.ms() < a.ms() { (b, a) } else { (a, b) }
}
