use crate::{Application, Document, lane_layout_key, lane_name};
use dawsu_editor::tempo::SnapMode;
use dawsu_editor::tools::ToolTarget;
use dawsu_foundation::{SampleIdentity, SampleSet, SongTime};
use dawsu_hitsounding::{LaneSource, Place};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GameplayOverlay {
    pub points: Vec<(&'static str, f64, usize)>,
    pub spans: Vec<(&'static str, f64, f64, usize)>,
}

pub fn gameplay_overlay(document: &Document) -> GameplayOverlay {
    let model = document.composer.hitsounds();
    let objects = document.file.hit_objects();
    let index_by_line: HashMap<_, _> = objects
        .iter()
        .enumerate()
        .map(|(index, object)| (object.line(), index))
        .collect();
    let mut points = Vec::new();
    let mut spans: Vec<_> = model
        .bodies()
        .iter()
        .filter_map(|body| {
            let &index = index_by_line.get(&body.object().line())?;
            Some(("slider", body.start().ms(), body.end().ms(), index))
        })
        .collect();
    for moment in model.moments() {
        let time = moment.time().ms();
        match moment.place() {
            Place::Circle(object) | Place::SliderNode { object, .. } => {
                if let Some(&index) = index_by_line.get(&object.line()) {
                    let kind = if matches!(moment.place(), Place::Circle(_)) {
                        "circle"
                    } else {
                        "slider"
                    };
                    points.push((kind, time, index));
                }
            }
            Place::SpinnerEnd(object) => {
                if let Some(&index) = index_by_line.get(&object.line()) {
                    let raw = &objects[index];
                    let end = raw
                        .spinner_end()
                        .or_else(|| raw.hold_end())
                        .unwrap_or(raw.time());
                    // Keep the rendered start aligned with the decoded end.
                    spans.push(("spinner", time - (end - raw.time()).max(0.0), time, index));
                }
            }
            Place::NoObject => {
                // In HS mode (hitsound diff), every drawn hit moment immediately appears as a
                // circle on the Song · objects row so the blueprint is visible before Save.
                if document.hs_mode && moment.hits().next().is_some() {
                    points.push(("circle", time, objects.len() + points.len()));
                }
            }
        }
    }
    points.sort_by(|a, b| a.1.total_cmp(&b.1));
    GameplayOverlay { points, spans }
}

/// Each object's combo number, combo index, and combo index with offsets (osu!lazer's
/// `IHasComboInformation.UpdateComboInformation`), in file order, plus `[Colours]` overrides.
pub fn combo_info(document: &Document) -> (Vec<(u32, usize, usize)>, Vec<String>) {
    let objects = document.file.hit_objects();
    let map_colours: Vec<String> = document
        .file
        .section_values("Colours")
        .into_iter()
        .filter(|(key, _)| {
            key.strip_prefix("Combo")
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| (1..=8).contains(&n))
        })
        .filter_map(|(_, value)| {
            let parts: Vec<_> = value.split(',').map(str::trim).collect();
            if !(3..=4).contains(&parts.len()) {
                return None;
            }
            // Combo colours take no alpha; only the first three parts count.
            let rgb: Vec<u8> = parts[..3]
                .iter()
                .map(|part| part.parse::<u8>())
                .collect::<Result<_, _>>()
                .ok()?;
            Some(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
        })
        .collect();
    let spinner = |index: usize| objects[index].spinner_end().is_some();
    let mut starts = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        let bit = object.type_bits() & 4 != 0;
        let offset = if bit && !spinner(index) {
            (object.type_bits() >> 4) & 7
        } else {
            0
        };
        starts.push((bit, offset));
    }
    let mut order: Vec<usize> = (0..objects.len()).collect();
    order.sort_by(|&a, &b| objects[a].time().total_cmp(&objects[b].time()));
    let mut combos = vec![(0u32, 0usize, 0usize); objects.len()];
    let (mut combo, mut with_offsets, mut number) = (0usize, 0usize, 0u32);
    let mut last: Option<usize> = None;
    for &index in &order {
        let (new_combo, offset) = starts[index];
        if !spinner(index) && (new_combo || last.is_none_or(spinner)) {
            number = 0;
            combo += 1;
            with_offsets += offset as usize + 1;
        }
        number += 1;
        combos[index] = (number, combo, with_offsets);
        last = Some(index);
    }
    (combos, map_colours)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComboOverlay {
    pub colours: Vec<String>,
    pub objects: Vec<(u32, usize)>,
}

/// The timeline's combo overlay: `colours`, and `objects` as `[number, colour]` per object.
pub fn combo_overlay(document: &Document) -> ComboOverlay {
    let (combos, map_colours) = combo_info(document);
    let objects: Vec<(u32, usize)> = combos
        .iter()
        .map(|&(number, combo, with_offsets)| {
            (
                number,
                if map_colours.is_empty() {
                    combo % 4
                } else {
                    with_offsets % map_colours.len()
                },
            )
        })
        .collect();
    let colours = if map_colours.is_empty() {
        ["#ffc000", "#00ca00", "#127cff", "#f21839"]
            .map(String::from)
            .to_vec()
    } else {
        map_colours
    };
    ComboOverlay { colours, objects }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPreviewObject {
    pub kind: &'static str,
    pub time: f64,
    pub end: f64,
    pub x: f64,
    pub y: f64,
    pub combo: u32,
    pub colour: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lengths: Option<Vec<f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spans: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticks: Option<Vec<f64>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPreviewData {
    pub path: PathBuf,
    pub key: u64,
    pub radius: f32,
    pub preempt: f64,
    pub fade_in: f64,
    pub colours: Vec<String>,
    pub objects: Vec<MapPreviewObject>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MapPreviewResult {
    Unsupported {
        path: PathBuf,
        key: u64,
        unsupported: bool,
    },
    Supported(MapPreviewData),
}

/// Typed Map Preview payload for the active standard difficulty (`map`).
pub fn map_preview_data(app: &Application, map: &Path) -> Result<MapPreviewResult, String> {
    let document = app.document().ok_or("Open a difficulty first.")?;
    if document.path != map {
        return Err("The open difficulty changed.".into());
    }
    let key = app.document_generation();
    let Some(field) = dawsu_hitsounding::playfield::playfield(&document.file) else {
        return Ok(MapPreviewResult::Unsupported {
            path: document.path.clone(),
            key,
            unsupported: true,
        });
    };
    let (combos, map_colours) = combo_info(document);
    let colours: Vec<String> = if map_colours.is_empty() {
        ["#f17400", "#00f135", "#0052f1", "#f10000", "#e8eb00", "#5c00f1"]
            .map(String::from)
            .to_vec()
    } else {
        map_colours.clone()
    };
    let colour_of = |(_, combo, with_offsets): (u32, usize, usize)| {
        if map_colours.is_empty() {
            combo % colours.len()
        } else {
            with_offsets % colours.len()
        }
    };
    let file_index: HashMap<_, _> = document
        .file
        .hit_objects()
        .iter()
        .enumerate()
        .map(|(index, object)| (object.line(), index))
        .collect();
    let round = |value: f32| (f64::from(value) * 100.0).round() / 100.0;
    let objects = field
        .objects
        .iter()
        .map(|object| {
            let offset = object.stack_offset(field.scale);
            let at = object.position + offset;
            let combo = file_index
                .get(&object.line)
                .and_then(|&index| combos.get(index).copied())
                .unwrap_or((0, 0, 0));
            let kind = match object.kind {
                dawsu_hitsounding::rules::Kind::Circle => "circle",
                dawsu_hitsounding::rules::Kind::Slider => "slider",
                dawsu_hitsounding::rules::Kind::Spinner => "spinner",
            };
            let (path, lengths, spans, ticks) = if let Some(path) = &object.path {
                let points: Vec<f64> = path
                    .points
                    .iter()
                    .flat_map(|point| [round(at.x + point.x), round(at.y + point.y)])
                    .collect();
                let lens: Vec<f64> = path
                    .cumulative
                    .iter()
                    .map(|length| (length * 100.0).round() / 100.0)
                    .collect();
                (
                    Some(points),
                    Some(lens),
                    Some(object.span_count),
                    Some(object.ticks.clone()),
                )
            } else {
                (None, None, None, None)
            };
            MapPreviewObject {
                kind,
                time: object.start_time,
                end: object.end_time,
                x: round(at.x),
                y: round(at.y),
                combo: combo.0,
                colour: colour_of(combo),
                path,
                lengths,
                spans,
                ticks,
            }
        })
        .collect();
    Ok(MapPreviewResult::Supported(MapPreviewData {
        path: document.path.clone(),
        key,
        radius: field.radius,
        preempt: field.preempt,
        fade_in: field.fade_in,
        colours,
        objects,
    }))
}

pub fn musical_position_text(app: &Application, position: f64) -> Option<String> {
    app.document().map(|document| {
        let composer = &document.composer;
        let time = SongTime::from_ms(position);
        let tempo = composer.tempo.section_at(time);
        let (beat, (numerator, denominator)) = document.editor_bar_at(time);
        let tick = (beat.beat_fraction * 480.0 + 1e-9).floor() as u32;
        let snap = if composer.snap_enabled {
            if composer.snap_mode == SnapMode::Objects {
                "Objects".into()
            } else {
                format!("{}/{}", composer.spacing.beats(), composer.spacing.divisions())
            }
        } else {
            "Snap off".into()
        };
        format!(
            "{}.{}.{:03} · {:.1} BPM · {numerator}/{denominator} · {snap}",
            beat.bar,
            beat.beat,
            tick.min(479),
            60_000.0 / tempo.beat_length_ms,
        )
    })
}

pub fn format_editor_window_title(map_title: &str, version: &str, path: &Path) -> String {
    let file_stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if !version.is_empty() && !map_title.is_empty() {
        format!("{map_title} [{version}] — dawsu!")
    } else if !file_stem.is_empty() {
        format!("{file_stem} — dawsu!")
    } else {
        "dawsu!".to_string()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackStart {
    #[default]
    Cursor,
    Left,
    Selection,
    SelectionOrLeft,
}

impl PlaybackStart {
    pub fn position(self, app: &Application) -> Option<f64> {
        let start = || {
            let composer = &app.document()?.composer;
            let moments = composer.hitsounds().moments();
            composer
                .selected
                .items()
                .iter()
                .filter_map(|hit| moments.get(hit.moment))
                .map(|moment| moment.time().ms())
                .min_by(f64::total_cmp)
        };
        match self {
            Self::Cursor => None,
            Self::Left => Some(app.locator_left()),
            Self::Selection => start(),
            Self::SelectionOrLeft => start().or(Some(app.locator_left())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulerGesture {
    Left,
    Right,
    Draw,
    Move,
    Seek,
}

impl RulerGesture {
    pub fn apply(self, app: &mut Application, path: &Path, from: f64, to: f64) {
        if app.is_busy() || !from.is_finite() || !to.is_finite() {
            return;
        }
        let Some(document) = app.document().filter(|document| document.path == path) else {
            return;
        };
        let duration = app.duration_ms();
        let snap = |ms: f64| {
            document
                .composer
                .snap(SongTime::from_ms(ms.clamp(0.0, duration)))
                .ms()
                .clamp(0.0, duration)
        };
        let (mut left, mut right, mut enabled) =
            (app.locator_left(), app.locator_right(), app.cycle_enabled());
        // A drag stops on the other locator; a Ctrl- or Alt-click (from == to) sets it where it lands.
        let dragged = from != to;
        match self {
            Self::Left => {
                left = if dragged && left <= right {
                    snap(to).min(right)
                } else {
                    snap(to)
                }
            }
            Self::Right => {
                right = if dragged && left <= right {
                    snap(to).max(left)
                } else {
                    snap(to)
                }
            }
            Self::Draw => {
                let (first, last) = (snap(from), snap(to));
                if first == last {
                    return;
                }
                (left, right, enabled) = (first.min(last), first.max(last), true);
            }
            Self::Move => {
                let requested = snap(left + (to - from)) - left;
                let shift = requested.clamp(-left.min(right), (duration - left.max(right)).max(0.0));
                left += shift;
                right += shift;
            }
            Self::Seek => {
                app.seek(snap(to));
                return;
            }
        }
        app.set_locators(left, right, enabled);
    }
}

pub fn map_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("osu") || ext.eq_ignore_ascii_case("osz"))
}

pub fn sample_set_name(set: SampleSet) -> &'static str {
    match set {
        SampleSet::Normal => "normal",
        SampleSet::Soft => "soft",
        SampleSet::Drum => "drum",
    }
}

pub fn scene_lane_key(lane: &SampleIdentity) -> String {
    format!("{lane:?}")
}

pub fn lane_by_key(app: &Application, key: &str) -> Result<SampleIdentity, String> {
    app.document()
        .and_then(|document| {
            document
                .composer
                .lanes
                .iter()
                .find(|lane| scene_lane_key(lane) == key)
        })
        .cloned()
        .ok_or_else(|| "The lane changed. Choose a lane again.".into())
}

pub fn mix_lanes(
    app: &mut Application,
    lanes: Vec<SampleIdentity>,
    operation: &str,
    clicked: Option<SampleIdentity>,
) {
    type LanePredicate = fn(&Application, &SampleIdentity) -> bool;
    type LaneToggle = fn(&mut Application, SampleIdentity);
    let (enabled, toggle): (LanePredicate, LaneToggle) = match operation {
        "mute" => (Application::is_muted, Application::mute),
        "solo" => (Application::is_solo, Application::solo),
        _ => return,
    };
    let on = match &clicked {
        Some(lane) => !enabled(app, lane),
        None => lanes.iter().any(|lane| !enabled(app, lane)),
    };
    for lane in lanes {
        if enabled(app, &lane) != on {
            toggle(app, lane);
        }
    }
}

pub fn group_mix(app: &mut Application, set: &str, operation: &str) {
    let Some(document) = app.document().filter(|_| !app.is_busy()) else {
        return;
    };
    let lanes = document
        .composer
        .lanes
        .iter()
        .filter(|lane| {
            matches!(lane, SampleIdentity::Standard { set: lane_set, .. } if sample_set_name(*lane_set) == set)
        })
        .cloned()
        .collect();
    mix_lanes(app, lanes, operation, None);
}

/// The difficulty's lane rows (composer lanes) that get a lane header, in order, and the slots
/// that rows stand in for.
pub fn lane_header_rows<'a>(
    document: &'a Document,
    used: &dyn Fn(&SampleIdentity) -> bool,
) -> (Vec<usize>, HashMap<usize, &'a SampleIdentity>) {
    let c = &document.composer;
    let model = c.hitsounds();
    let row_by_lane: HashMap<_, _> = c.lanes.iter().enumerate().map(|(row, lane)| (lane, row)).collect();
    let mut slot_rows = HashMap::new();
    let mut backing_slots = HashMap::new();
    for (source, slot) in model.lane_slots() {
        if !used(slot)
            && let (Some(&from), Some(&to)) = (row_by_lane.get(source), row_by_lane.get(slot))
            && let std::collections::hash_map::Entry::Vacant(entry) = slot_rows.entry(to)
        {
            entry.insert(from);
            backing_slots.insert(from, slot);
        }
    }
    let lane_order = (0..c.lanes.len())
        .filter(|row| !backing_slots.contains_key(row))
        .map(|row| slot_rows.get(&row).copied().unwrap_or(row))
        .filter(|&row| !model.is_lane_hidden(&c.lanes[row]))
        .collect();
    (lane_order, backing_slots)
}

/// Puts each numbered layer directly under its family's main lane, by number.
pub fn family_order<'a>(rows: Vec<usize>, lane: impl Fn(usize) -> &'a SampleIdentity) -> Vec<usize> {
    let family = |row: usize| match lane(row) {
        SampleIdentity::Standard { set, sound, index } => Some(((*set, *sound), index.get())),
        SampleIdentity::File(_) => None,
    };
    let mut mains = HashMap::new();
    for &row in &rows {
        if let Some((key, index @ (0 | 1))) = family(row)
            && (index == 1 || !mains.contains_key(&key))
        {
            mains.insert(key, row);
        }
    }
    let mut layers: HashMap<usize, Vec<(u32, usize)>> = HashMap::new();
    for &row in &rows {
        if let Some((key, index)) = family(row)
            && let Some(&main) = mains.get(&key)
            && main != row
        {
            layers.entry(main).or_default().push((index, row));
        }
    }
    let children: HashSet<usize> = layers.values().flatten().map(|&(_, row)| row).collect();
    let mut ordered = Vec::with_capacity(rows.len());
    for row in rows {
        if children.contains(&row) {
            continue;
        }
        ordered.push(row);
        if let Some(mut family) = layers.remove(&row) {
            family.sort_unstable();
            ordered.extend(family.into_iter().map(|(_, row)| row));
        }
    }
    ordered
}

/// Each lane's 1-based display number matching its lane header row order.
pub fn header_numbers(app: &Application) -> HashMap<&SampleIdentity, usize> {
    let Some(document) = app.document() else {
        return HashMap::new();
    };
    let c = &document.composer;
    let used: HashSet<_> = c
        .hitsounds()
        .moments()
        .iter()
        .flat_map(|moment| moment.hits())
        .collect();
    let (mut rows, backing_slots) = lane_header_rows(document, &|lane| used.contains(lane));
    let saved = document
        .lane_layout()
        .map(|layout| layout.lane_order.as_slice())
        .unwrap_or_default();
    if !saved.is_empty() {
        let rank = |row: &usize| {
            let key = lane_layout_key(backing_slots.get(row).copied().unwrap_or(&c.lanes[*row]));
            saved
                .iter()
                .position(|saved| *saved == key)
                .unwrap_or(saved.len())
        };
        rows.sort_by_key(rank);
    }
    let rows = family_order(rows, |row| {
        backing_slots.get(&row).copied().unwrap_or(&c.lanes[row])
    });
    rows.into_iter()
        .enumerate()
        .map(|(n, row)| (&c.lanes[row], n + 1))
        .collect()
}

pub fn assign_sample(
    app: &mut Application,
    expected_map: &Path,
    expected_lane_key: &str,
    path: PathBuf,
    mapset: bool,
) -> Result<(), String> {
    let document = app.document().ok_or("Open a difficulty first.")?;
    let lane = document
        .composer
        .lanes
        .get(app.active_lane())
        .ok_or("Choose an active lane.")?;
    if document.path != expected_map || scene_lane_key(lane) != expected_lane_key {
        return Err("The active lane changed while choosing a sample.".into());
    }
    let lane = lane.clone();
    let path = if mapset {
        let name = path.to_str().ok_or("Invalid sample filename.")?;
        if !document
            .composer
            .hitsounds()
            .lanes()
            .iter()
            .any(|lane| matches!(&lane.source, LaneSource::File(sample) if sample == name))
        {
            return Err("The selected mapset sample is unavailable.".into());
        }
        let root = document.path.parent().ok_or("Missing mapset folder.")?;
        let file = root.join(&path).canonicalize().map_err(|e| e.to_string())?;
        if !file.starts_with(root) {
            return Err("The selected sample leaves the map folder.".into());
        }
        file
    } else {
        path
    };
    app.replace_lane_sample(expected_map, &lane, &path)
}

pub fn lane_add_candidate_list(
    app: &Application,
    map: &Path,
) -> Result<Vec<(SampleIdentity, String, String)>, String> {
    let candidates = app.scan_lane_add_candidates(map)?;
    let model = app
        .document()
        .ok_or("Open a difficulty first.")?
        .composer
        .hitsounds();
    Ok(candidates
        .into_iter()
        .map(|lane| {
            let name = lane_name(model.lane_slot(&lane).unwrap_or(&lane));
            let key = scene_lane_key(&lane);
            (lane, key, name)
        })
        .collect())
}
