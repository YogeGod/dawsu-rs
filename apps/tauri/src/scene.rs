use dawsu_application::{Application, EditorTimeSignature, LaneLayout, lane_name};
use dawsu_editor::{
    keys::{Key, KeyCombination, Mods},
    tempo::SnapMode,
    tools::ToolPreview,
    volume_ramp::ramp_at,
};
use dawsu_foundation::{CustomIndex, SampleIdentity, SongTime, Sound};
use dawsu_hitsounding::{LaneSource, Place};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use crate::action::{
    Action, PlaybackStart, lane_header_rows, map_path, sample_set_name, scene_lane_key,
};
use crate::commands::library_and_files::{cached_recent_meta, is_osu_hs_diff};
use crate::{drum_view, home, selection};

#[derive(serde::Serialize)]
pub(crate) struct GameplayOverlay {
    points: Vec<(&'static str, f64, usize)>,
    spans: Vec<(&'static str, f64, f64, usize)>,
}

pub(crate) fn gameplay_overlay(document: &dawsu_application::Document) -> GameplayOverlay {
    let model = document.composer.hitsounds();
    let objects = document.file.hit_objects();
    let index_by_line: std::collections::HashMap<_, _> = objects.iter().enumerate()
        .map(|(index, object)| (object.line(), index)).collect();
    let mut points = Vec::new();
    let mut spans: Vec<_> = model.bodies().iter().filter_map(|body| {
        let &index = index_by_line.get(&body.object().line())?;
        Some(("slider", body.start().ms(), body.end().ms(), index))
    }).collect();
    for moment in model.moments() {
        let time = moment.time().ms();
        match moment.place() {
            Place::Circle(object) | Place::SliderNode { object, .. } => {
                if let Some(&index) = index_by_line.get(&object.line()) {
                    let kind = if matches!(moment.place(), Place::Circle(_)) { "circle" } else { "slider" };
                    points.push((kind, time, index));
                }
            }
            Place::SpinnerEnd(object) => {
                if let Some(&index) = index_by_line.get(&object.line()) {
                    let raw = &objects[index];
                    let end = raw.spinner_end().or_else(|| raw.hold_end()).unwrap_or(raw.time());
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
    GameplayOverlay { points, spans }
}

/// Each object's combo number and colour, as osu!lazer works them out at 131fe72:
/// `ConvertHitObjectParser.cs:458-525` (an offset counts only with the new-combo bit and never on a
/// spinner), then `OsuHitObject.cs:193-196` on the objects sorted by time (a spinner never starts a
/// combo, even flagged or first; the first object and one after a spinner do), and the colour from the map's
/// `[Colours]` by the index with offsets (`LegacyBeatmapSkin.cs:90`), or else the default skin's four
/// by the plain index (`SkinConfiguration.cs:46-52`). `objects` is `[number, colour]` per object in
/// file order, the order of the gameplay overlay's object indices.
/// Each object's combo number, combo index and combo index with offsets (lazer's
/// `IHasComboInformation.UpdateComboInformation`), in file order, and the map's own combo colours.
pub(crate) fn combo_info(document: &dawsu_application::Document) -> (Vec<(u32, usize, usize)>, Vec<String>) {
    let objects = document.file.hit_objects();
    let map_colours: Vec<String> = document.file.section_values("Colours").into_iter()
        .filter(|(key, _)| key.strip_prefix("Combo").and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=8).contains(&n)))
        .filter_map(|(_, value)| {
            let parts: Vec<_> = value.split(',').map(str::trim).collect();
            if !(3..=4).contains(&parts.len()) {
                return None;
            }
            // Combo colours take no alpha; only the first three parts count.
            let rgb: Vec<u8> = parts[..3].iter().map(|part| part.parse::<u8>()).collect::<Result<_, _>>().ok()?;
            Some(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]))
        })
        .collect();
    let spinner = |index: usize| objects[index].spinner_end().is_some();
    // The parser's pass, in file order: the new-combo bit, and the offset that counts only with it.
    let mut starts = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        let bit = object.type_bits() & 4 != 0;
        let offset = if bit && !spinner(index) { (object.type_bits() >> 4) & 7 } else { 0 };
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

#[derive(serde::Serialize)]
pub(crate) struct ComboOverlay {
    colours: Vec<String>,
    objects: Vec<(u32, usize)>,
}

/// The timeline's combo overlay: `colours`, and `objects` as `[number, colour]` per object (see `combo_info`).
pub(crate) fn combo_overlay(document: &dawsu_application::Document) -> ComboOverlay {
    let (combos, map_colours) = combo_info(document);
    let objects: Vec<(u32, usize)> = combos.iter().map(|&(number, combo, with_offsets)| {
        (number, if map_colours.is_empty() { combo % 4 } else { with_offsets % map_colours.len() })
    }).collect();
    let colours = if map_colours.is_empty() {
        ["#ffc000", "#00ca00", "#127cff", "#f21839"].map(String::from).to_vec()
    } else {
        map_colours
    };
    ComboOverlay { colours, objects }
}

/// What the map preview draws for the open difficulty (`map`): circle size, approach time and each
/// object's stacked place, times, path and combo, as osu!lazer works them out at 131fe72
/// (`dawsu_hitsounding::playfield`). A taiko, catch or mania difficulty answers
/// `unsupported`; the editor refuses to open those today, so this is only a safeguard.
/// A slider's path is flat `[x0, y0, x1, y1, …]` on the playfield, stack offset included.
pub(crate) fn map_preview(app: &Application, map: &Path) -> Result<Value, String> {
    let document = app.document().ok_or("Open a difficulty first.")?;
    if document.path != map {
        return Err("The open difficulty changed.".into());
    }
    let key = app.document_generation();
    let Some(field) = dawsu_hitsounding::playfield::playfield(&document.file) else {
        return Ok(json!({ "path": document.path, "key": key, "unsupported": true }));
    };
    // With no colours of its own a map takes lazer's default skin's (Argon, ArgonSkin.cs:51-71) by the
    // plain combo index; its own go by the index with offsets, as in the timeline.
    let (combos, map_colours) = combo_info(document);
    let colours: Vec<String> = if map_colours.is_empty() {
        ["#f17400", "#00f135", "#0052f1", "#f10000", "#e8eb00", "#5c00f1"].map(String::from).to_vec()
    } else {
        map_colours.clone()
    };
    let colour_of = |(_, combo, with_offsets): (u32, usize, usize)| {
        if map_colours.is_empty() { combo % colours.len() } else { with_offsets % colours.len() }
    };
    let file_index: std::collections::HashMap<_, _> =
        document.file.hit_objects().iter().enumerate().map(|(index, object)| (object.line(), index)).collect();
    let round = |value: f32| (f64::from(value) * 100.0).round() / 100.0;
    let objects: Vec<Value> = field.objects.iter().map(|object| {
        let offset = object.stack_offset(field.scale);
        let at = object.position + offset;
        let combo = file_index.get(&object.line).and_then(|&index| combos.get(index).copied()).unwrap_or((0, 0, 0));
        let kind = match object.kind {
            dawsu_hitsounding::rules::Kind::Circle => "circle",
            dawsu_hitsounding::rules::Kind::Slider => "slider",
            dawsu_hitsounding::rules::Kind::Spinner => "spinner",
        };
        let mut value = json!({
            "kind": kind, "time": object.start_time, "end": object.end_time,
            "x": round(at.x), "y": round(at.y), "combo": combo.0, "colour": colour_of(combo),
        });
        if let Some(path) = &object.path {
            let points: Vec<f64> = path.points.iter().flat_map(|point| [round(at.x + point.x), round(at.y + point.y)]).collect();
            value["path"] = json!(points);
            value["lengths"] = json!(path.cumulative.iter().map(|length| (length * 100.0).round() / 100.0).collect::<Vec<_>>());
            value["spans"] = json!(object.span_count);
            value["ticks"] = json!(object.ticks);
        }
        value
    }).collect();
    Ok(json!({
        "path": document.path, "key": key, "radius": field.radius, "preempt": field.preempt, "fadeIn": field.fade_in,
        "colours": colours, "objects": objects,
    }))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ViewTickSnapshot {
    tick: bool,
    rev: u64,
    position: f64,
    playing: bool,
    busy: bool,
    peaks: PeaksSnapshot,
    musical_position: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(untagged)]
pub(crate) enum ViewResponse {
    View(Box<ViewSnapshot>),
    Tick(ViewTickSnapshot),
    Fault { fault: String },
    NameTaken { #[serde(rename = "nameTaken")] name_taken: String },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SceneLane {
    pub(crate) can_add_layer: bool,
    pub(crate) can_delete: bool,
    pub(crate) can_replace: bool,
    pub(crate) colour_key: String,
    pub(crate) family: Option<String>,
    pub(crate) gain: f64,
    pub(crate) group: Option<&'static str>,
    pub(crate) hit_count: usize,
    pub(crate) index: Option<u32>,
    pub(crate) key: String,
    pub(crate) muted: bool,
    pub(crate) name: String,
    pub(crate) sample: Option<String>,
    pub(crate) solo: bool,
    pub(crate) source_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) vst_bypassed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) vst_routed: Option<bool>,
}

pub(crate) struct SceneHit {
    row: usize,
    selected: bool,
    muted: bool,
    gain: Option<i32>,
}

impl serde::Serialize for SceneHit {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let len = if self.gain.is_some() { 4 } else if self.muted { 3 } else { 2 };
        let mut seq = serializer.serialize_seq(Some(len))?;
        seq.serialize_element(&self.row)?;
        seq.serialize_element(&self.selected)?;
        if let Some(gain) = self.gain {
            seq.serialize_element(&self.muted)?;
            seq.serialize_element(&gain)?;
        } else if self.muted {
            seq.serialize_element(&true)?;
        }
        seq.end()
    }
}

pub(crate) struct SceneMomentEntry {
    time: f64,
    volume: i32,
    on_object: bool,
    hits_start: usize,
    hits_end: usize,
}

pub(crate) struct SceneMoments {
    moments: Vec<SceneMomentEntry>,
    hits: Vec<SceneHit>,
}

impl serde::Serialize for SceneMoments {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        struct MomentTuple<'a> {
            entry: &'a SceneMomentEntry,
            hits: &'a [SceneHit],
        }
        impl serde::Serialize for MomentTuple<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut seq = serializer.serialize_seq(Some(4))?;
                seq.serialize_element(&self.entry.time)?;
                seq.serialize_element(&self.entry.volume)?;
                seq.serialize_element(&self.entry.on_object)?;
                seq.serialize_element(self.hits)?;
                seq.end()
            }
        }
        let mut seq = serializer.serialize_seq(Some(self.moments.len()))?;
        for entry in &self.moments {
            seq.serialize_element(&MomentTuple {
                entry,
                hits: &self.hits[entry.hits_start..entry.hits_end],
            })?;
        }
        seq.end()
    }
}

#[derive(serde::Serialize)]
pub(crate) struct SceneBookmark {
    label: Option<String>,
    time: f64,
}

#[derive(serde::Serialize)]
pub(crate) struct SceneTimeSignature {
    bar: u32,
    denominator: u32,
    numerator: u32,
}

impl From<&EditorTimeSignature> for SceneTimeSignature {
    fn from(s: &EditorTimeSignature) -> Self {
        Self { bar: s.bar, denominator: s.denominator, numerator: s.numerator }
    }
}

#[derive(serde::Serialize)]
pub(crate) struct SceneEditorBarSection {
    bar: i64,
    beat: f64,
    denominator: u32,
    numerator: u32,
    start: f64,
}

impl From<&dawsu_application::EditorBarSection> for SceneEditorBarSection {
    fn from(s: &dawsu_application::EditorBarSection) -> Self {
        Self { bar: s.bar, beat: s.beat, denominator: s.denominator, numerator: s.numerator, start: s.start }
    }
}

#[derive(serde::Serialize)]
pub(crate) struct SceneLaneLayout {
    collapsed_families: std::collections::BTreeSet<String>,
    collapsed_folders: std::collections::BTreeSet<String>,
    folder_order: Vec<String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    lane_colours: std::collections::BTreeMap<String, String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    lane_names: std::collections::BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lane_order: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    plain_list: bool,
}

impl From<&LaneLayout> for SceneLaneLayout {
    fn from(layout: &LaneLayout) -> Self {
        Self {
            collapsed_families: layout.collapsed_families.clone(),
            collapsed_folders: layout.collapsed_folders.clone(),
            folder_order: layout.folder_order.clone(),
            lane_colours: layout.lane_colours.clone(),
            lane_names: layout.lane_names.clone(),
            lane_order: layout.lane_order.clone(),
            plain_list: layout.plain_list,
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SceneSnapshot {
    pub(crate) beats: u32,
    pub(crate) bookmarks: Vec<SceneBookmark>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) combos: Option<ComboOverlay>,
    pub(crate) coverage: dawsu_application::CoverageSummary,
    pub(crate) difficulties: Vec<PathBuf>,
    pub(crate) divisor: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) drum_rows: Option<Vec<drum_view::DrumRowSnapshot>>,
    pub(crate) editor_bars: Vec<SceneEditorBarSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) gameplay: Option<GameplayOverlay>,
    pub(crate) history: Vec<String>,
    pub(crate) history_position: usize,
    pub(crate) hs: bool,
    pub(crate) hs_difficulties: Vec<PathBuf>,
    pub(crate) insert_volume: i32,
    pub(crate) lane_layout: Option<SceneLaneLayout>,
    pub(crate) lane_order: Vec<usize>,
    pub(crate) lane_revision: u64,
    pub(crate) lanes: Vec<SceneLane>,
    pub(crate) map_title: String,
    pub(crate) moments: SceneMoments,
    pub(crate) orphaned_hits: Vec<f64>,
    pub(crate) path: PathBuf,
    pub(crate) pattern_stamp_step: Option<f64>,
    pub(crate) placement: Option<u64>,
    pub(crate) preview_key: u64,
    pub(crate) protect: bool,
    pub(crate) ramp_preview: Option<Vec<(f64, i32)>>,
    pub(crate) range: Option<[f64; 2]>,
    pub(crate) repeat_gap: Option<u32>,
    pub(crate) row_height: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sections: Option<Vec<(f64, f64, u32)>>,
    pub(crate) selection: Option<selection::SelectionSummary>,
    pub(crate) snap: bool,
    pub(crate) snap_mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) starter_mode: Option<&'static str>,
    pub(crate) time_signatures: Vec<SceneTimeSignature>,
    pub(crate) title: String,
    pub(crate) version: String,
    pub(crate) view_start: f64,
    pub(crate) zoom: f64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DrumVstSnapshot {
    enabled: bool,
    instance_id: String,
    loaded: bool,
    mapped_lanes: usize,
    plugin_name: String,
    plugin_path: String,
    plugin_uid: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct OutputProblemSnapshot {
    message: String,
    target: PathBuf,
    time: f64,
    timeline: PathBuf,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PanelsSnapshot {
    browser: bool,
    controller: bool,
    inspector: bool,
    lane_names: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecentMetaSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) background: Option<String>,
    pub(crate) is_hs: bool,
    pub(crate) path: PathBuf,
}

#[derive(serde::Serialize)]
pub(crate) struct PeaksSnapshot {
    hitsounds: f64,
    master: f64,
    song: f64,
}

#[derive(serde::Serialize)]
pub(crate) struct PlannedFileSnapshot {
    added: usize,
    changed: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    diff: Option<dawsu_application::output::OsuDiffSummary>,
    path: PathBuf,
    removed: usize,
    replace: bool,
    timing: usize,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutputReviewSnapshot {
    combinations: Value,
    created_mixes: Vec<PathBuf>,
    details: Vec<String>,
    files: Vec<PlannedFileSnapshot>,
    include_slider_audio: bool,
    label: String,
    mixes: Vec<PathBuf>,
    reused_mixes: Vec<PathBuf>,
    sample_warnings: Vec<String>,
    volume_summary: Vec<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct RecoveryOfferSnapshot {
    created_ms: u64,
    difficulty: String,
    id: String,
    mapset: String,
}

#[derive(serde::Serialize)]
#[serde(untagged)]
pub(crate) enum RecoverySnapshot {
    Offers { offers: Vec<RecoveryOfferSnapshot> },
    Error { error: String },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SoundSetPromptSnapshot {
    edited_keep_files: usize,
    name: String,
    open: u64,
    placements: usize,
    set_id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ViewSnapshot {
    pub(crate) active_lane: usize,
    pub(crate) audition: bool,
    pub(crate) auto_check_updates: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bookmark_step: Option<Option<f64>>,
    pub(crate) busy: bool,
    pub(crate) can_undo_revert: bool,
    pub(crate) classic_browser: bool,
    pub(crate) clean_redundant_greenlines: bool,
    pub(crate) cycle: bool,
    pub(crate) dirty: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) drum_view: Option<drum_view::DrumViewSnapshot>,
    pub(crate) drum_vst: Option<DrumVstSnapshot>,
    pub(crate) duration: f64,
    pub(crate) edit_dialog: Option<&'static str>,
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) external_osu_changed: bool,
    pub(crate) first_launch: bool,
    pub(crate) follow: bool,
    pub(crate) has_document: bool,
    pub(crate) has_original_snapshot: bool,
    pub(crate) hitsound_library: Option<PathBuf>,
    pub(crate) hitsounds: f64,
    pub(crate) hitsounds_muted: bool,
    pub(crate) hitsounds_solo: bool,
    pub(crate) hs_mode: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) is_lazer_external_edit: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) is_practice_mapset: bool,
    pub(crate) keep_pitch: bool,
    pub(crate) key_preset: String,
    pub(crate) lane_count: usize,
    pub(crate) left: f64,
    pub(crate) main: f64,
    pub(crate) musical_position: Option<String>,
    pub(crate) object_lines: bool,
    pub(crate) offset: f64,
    pub(crate) output_device: String,
    pub(crate) output_problems: Vec<OutputProblemSnapshot>,
    pub(crate) panels: PanelsSnapshot,
    pub(crate) peaks: PeaksSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pending_home: Option<bool>,
    pub(crate) pending_open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) playback_start: Option<PlaybackStart>,
    pub(crate) playing: bool,
    pub(crate) position: f64,
    pub(crate) preview_window: bool,
    pub(crate) quantize_strength: f64,
    pub(crate) ramp_shape: String,
    pub(crate) recent: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recent_meta: Option<Vec<RecentMetaSnapshot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recovery: Option<RecoverySnapshot>,
    pub(crate) reload_osu_after_save: bool,
    pub(crate) return_on_stop: bool,
    pub(crate) revert: bool,
    pub(crate) review: Option<OutputReviewSnapshot>,
    pub(crate) right: f64,
    pub(crate) save_problem_review: bool,
    pub(crate) save_review: Option<Vec<(&'static str, PathBuf)>>,
    pub(crate) scene: Option<SceneSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) shortcut_action: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) shortcut_help: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) shortcut_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) show_home: Option<bool>,
    pub(crate) song: f64,
    pub(crate) song_loading: bool,
    pub(crate) song_muted: bool,
    pub(crate) song_solo: bool,
    pub(crate) songs_folder: Option<PathBuf>,
    pub(crate) sound_set_prompts: Vec<SoundSetPromptSnapshot>,
    pub(crate) speed: f64,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) stem_active: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) stem_loaded: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) stem_loading: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stem_name: Option<String>,
    pub(crate) step_pads_mode: bool,
    pub(crate) tab_routes: Vec<(bool, bool, bool, bool)>,
    pub(crate) tap_recorded_count: usize,
    pub(crate) tap_recording_armed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tick_rev: Option<u64>,
    pub(crate) tool: String,
    pub(crate) warnings: Vec<String>,
    pub(crate) waveform_revision: u64,
}

pub(crate) fn snapshot_view(app: &Application, include_scene: bool) -> ViewSnapshot {
    snapshot_with_static(app, include_scene, true)
}

#[cfg(test)]
pub(crate) fn snapshot(app: &Application, include_scene: bool) -> Value {
    serde_json::to_value(snapshot_view(app, include_scene)).unwrap()
}

pub(crate) fn musical_position_text(app: &Application, position: f64) -> Option<String> {
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
            beat.bar, beat.beat, tick.min(479), 60_000.0 / tempo.beat_length_ms,
        )
    })
}

pub(crate) fn snapshot_with_static(app: &Application, include_scene: bool, include_static: bool) -> ViewSnapshot {
    let document = app.document();
    let position = app.position_ms();
    let musical_position = musical_position_text(app, position);
    let scene = if include_scene {
        document.map(|d| {
            let c = &d.composer;
            let model = c.hitsounds();
            let row_by_lane: std::collections::HashMap<_, _> =
                c.lanes.iter().enumerate().map(|(row, lane)| (lane, row)).collect();
            let mut hit_counts = std::collections::HashMap::new();
            for lane in model.moments().iter().flat_map(|moment| moment.hits()) {
                *hit_counts.entry(lane).or_insert(0) += 1;
            }
            let (lane_order, backing_slots) = lane_header_rows(d, &|lane| hit_counts.contains_key(lane));
            let lanes = c.lanes.iter().enumerate().map(|(row, lane)| {
                let slot = backing_slots.get(&row).copied().unwrap_or(lane);
                let (family, index, group) = match slot {
                    SampleIdentity::Standard { set, sound, index } => {
                        let family = lane_name(&SampleIdentity::Standard { set: *set, sound: *sound, index: CustomIndex::new(0) });
                        (Some(family), Some(index.get()), Some(sample_set_name(*set)))
                    }
                    SampleIdentity::File(_) => (None, None, None),
                };
                let sample = model.lane(lane).and_then(|lane| match &lane.source {
                    LaneSource::File(name) => Some(name.clone()),
                    LaneSource::Skin => None,
                });
                let hit_count = hit_counts.get(lane).copied().unwrap_or(0);
                let read_only = matches!(slot, SampleIdentity::Standard { sound: Sound::Slider(_), .. });
                let protected = c.protect_normals && hit_count > 0
                    && matches!(slot, SampleIdentity::Standard { sound: Sound::Hit(dawsu_foundation::Hitsound::Normal), .. });
                let plays_as = model.lane(lane).map(|m| m.plays_as()).unwrap_or_else(|| lane.clone());
                let (vst_routed, vst_bypassed) = match app.drum_vst() {
                    Some(vst) if vst.is_enabled() => {
                        let has_note = vst.routed_notes().contains_key(lane);
                        let bypassed = has_note && vst.is_lane_vst_muted(lane);
                        (Some(has_note && !bypassed), Some(bypassed))
                    }
                    Some(_) => (Some(false), Some(false)),
                    None => (None, None),
                };
                SceneLane {
                    can_add_layer: index.is_some(),
                    can_delete: !read_only && !protected,
                    can_replace: !matches!(lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. }),
                    colour_key: dawsu_application::lane_layout_key(slot),
                    family,
                    gain: f64::from(app.lane_gain(&plays_as)),
                    group,
                    hit_count,
                    index,
                    key: scene_lane_key(slot),
                    muted: app.is_muted(lane),
                    name: lane_name(slot),
                    sample,
                    solo: app.is_solo(lane),
                    source_key: scene_lane_key(lane),
                    vst_bypassed,
                    vst_routed,
                }
            }).collect::<Vec<_>>();
            let has_selected = !c.selected.items().is_empty();
            let selected: std::collections::HashSet<_> = if has_selected {
                c.selected.items().iter().map(|key| (key.moment, &key.lane)).collect()
            } else {
                std::collections::HashSet::new()
            };
            let has_listen_muted = !c.listen_muted.is_empty();
            let listen_muted: std::collections::HashSet<_> = if has_listen_muted {
                c.listen_muted.iter().map(|key| (key.moment, &key.lane)).collect()
            } else {
                std::collections::HashSet::new()
            };
            let source_moments = c.hitsounds().moments();
            let mut moment_entries = Vec::with_capacity(source_moments.len());
            let mut hit_entries = Vec::with_capacity(source_moments.len());
            for (i, moment) in source_moments.iter().enumerate() {
                let hits_start = hit_entries.len();
                let moment_vol = moment.volume();
                for lane in moment.hits() {
                    let Some(&row) = row_by_lane.get(lane) else { continue };
                    let is_selected = has_selected && selected.contains(&(i, lane));
                    let muted = has_listen_muted && listen_muted.contains(&(i, lane));
                    let gain = moment.hit_volume(lane);
                    hit_entries.push(SceneHit {
                        row,
                        selected: is_selected,
                        muted,
                        gain: (gain != moment_vol).then_some(gain),
                    });
                }
                moment_entries.push(SceneMomentEntry {
                    time: moment.time().ms(),
                    volume: moment.reference_volume(),
                    on_object: moment.place() != Place::NoObject,
                    hits_start,
                    hits_end: hit_entries.len(),
                });
            }
            let moments = SceneMoments {
                moments: moment_entries,
                hits: hit_entries,
            };
            let sections = include_static.then(|| {
                c.tempo.sections().iter()
                    .map(|s| (s.start.ms(), s.beat_length_ms, s.beats_per_bar))
                    .collect::<Vec<_>>()
            });
            let ramp_preview = match c.preview {
                ToolPreview::RampLine { from, to, shape } => {
                    let (from, to) = if from.0 > to.0 { (to, from) } else { (from, to) };
                    Some((0..=16).map(|sample| {
                        let fraction = sample as f64 / 16.0;
                        let time = from.0.ms() + (to.0.ms() - from.0.ms()) * fraction;
                        (time, ramp_at(shape, fraction, from.1, to.1))
                    }).collect::<Vec<_>>())
                }
                _ => None,
            };
            let mut selection = selection::summary(model, selected.iter().copied());
            if let Some(selection) = selection.as_mut()
                && let Some(index) = d.selected_slider_body_index()
            {
                let body = &model.bodies()[index];
                selection.slider_body = Some(selection::SliderBodySelection {
                    index,
                    set: body.set().map(sample_set_name),
                    start: body.start().ms(),
                    volume: body.volume(),
                    whistle: body.whistle(),
                    whistle_set: body.whistle_set().map_or("auto", sample_set_name),
                });
            }
            SceneSnapshot {
                beats: c.spacing.beats(),
                bookmarks: d.bookmarks().iter().map(|b| SceneBookmark {
                    label: b.label.clone(),
                    time: b.time.ms(),
                }).collect(),
                combos: include_static.then(|| combo_overlay(d)),
                coverage: d.coverage_summary(),
                difficulties: d.difficulties.clone(),
                divisor: c.spacing.divisions(),
                drum_rows: None,
                editor_bars: d.editor_bars().iter().map(Into::into).collect(),
                gameplay: (include_static || d.hs_mode).then(|| gameplay_overlay(d)),
                history: c.history.steps().into_iter().map(str::to_owned).collect(),
                history_position: c.history.position(),
                hs: d.hs_mode,
                hs_difficulties: d.difficulties.iter().filter(|p| is_osu_hs_diff(p)).cloned().collect(),
                insert_volume: c.insert_volume,
                lane_layout: d.lane_layout().map(Into::into),
                lane_order,
                lane_revision: c.history.version(),
                lanes,
                map_title: d.map_title(),
                moments,
                orphaned_hits: dawsu_application::orphaned_hit_times(model, d.hs_mode),
                path: d.path.clone(),
                pattern_stamp_step: app.requested_edit().filter(|&id| id == "pattern_stamp").and_then(|_| c.pattern_stamp_step()),
                placement: c.selected_placement(),
                // The map preview asks for its objects again when this changes (map_preview).
                preview_key: app.document_generation(),
                protect: c.protect_normals,
                ramp_preview,
                range: c.range.map(|r| [r.from.ms(), r.to.ms()]),
                repeat_gap: app.requested_edit().filter(|&id| id == "repeat").and_then(|_| c.repeat_gap()),
                row_height: f64::from(app.row_height()),
                sections,
                selection,
                snap: c.snap_enabled,
                snap_mode: match c.snap_mode { SnapMode::Grid => "grid", SnapMode::GridRelative => "relative", SnapMode::Objects => "objects" },
                starter_mode: d.starter_mode.map(dawsu_application::HsStarterMode::as_str),
                time_signatures: d.time_signatures().iter().map(Into::into).collect(),
                title: d.title.clone(),
                version: d.version(),
                view_start: c.view_start,
                zoom: app.zoom(),
            }
        })
    } else {
        None
    };
    let review = app.output_review().map(|plan| {
        let files = plan.files.iter().map(|file| {
            let summary = file.file.change_summary();
            let diff = file.diff_summary();
            PlannedFileSnapshot {
                added: summary.hit_objects_added,
                changed: summary.hit_objects_changed,
                diff: (!diff.is_empty()).then(|| diff.clone()),
                path: file.path.clone(),
                removed: summary.hit_objects_removed,
                replace: file.expected.is_some(),
                timing: summary.green_lines_added + summary.green_lines_edited
                    + summary.green_lines_removed + summary.red_lines_changed,
            }
        }).collect::<Vec<_>>();
        let mut details = plan.details.clone();
        details.extend(plan.volume_clarity_lines());
        let sample_warnings = plan.sample_warning_lines();
        details.extend(sample_warnings.iter().cloned());
        OutputReviewSnapshot {
            combinations: serde_json::to_value(plan.export_combinations()).unwrap_or_default(),
            created_mixes: plan.created_mix_paths().map(Path::to_path_buf).collect(),
            details,
            files,
            include_slider_audio: plan.includes_slider_audio(),
            label: plan.label.clone(),
            mixes: plan.mix_paths().map(Path::to_path_buf).collect(),
            reused_mixes: plan.reused_mix_paths().map(Path::to_path_buf).collect(),
            sample_warnings,
            volume_summary: plan.volume_clarity_lines(),
        }
    });
    let output_problems = app.output_problems();
    let save_problem_review = app.save_problem_review_requested() && !output_problems.is_empty();
    let output_problems = output_problems.into_iter().map(|problem| OutputProblemSnapshot {
        message: problem.message,
        target: problem.target,
        time: problem.time,
        timeline: problem.timeline,
    }).collect();
    let (master_peak, song_peak, hitsound_peak) = app.audio_peaks();
    let tab_routes = [false, true].into_iter().flat_map(|ctrl| {
        [false, true].into_iter().flat_map(move |shift| {
            [false, true].into_iter().filter_map(move |alt| {
                let mut mods = Mods::NONE;
                for (pressed, flag) in [(ctrl, Mods::CTRL), (shift, Mods::SHIFT), (alt, Mods::ALT)] {
                    if pressed {
                        mods = mods.union(flag);
                    }
                }
                let id = app.keymap.command_for(KeyCombination::new(Key::Tab, mods))?;
                Some((ctrl, shift, alt, matches!(id, "open" | "help")))
            })
        })
    }).collect::<Vec<_>>();
    let recovery = if include_scene && document.is_none() {
        Some(match app.recovery_offers() {
            Ok(offers) => RecoverySnapshot::Offers {
                offers: offers.into_iter().map(|o| RecoveryOfferSnapshot {
                    created_ms: o.created_ms,
                    difficulty: o.difficulty,
                    id: o.id,
                    mapset: o.mapset,
                }).collect(),
            },
            Err(error) => RecoverySnapshot::Error {
                error: format!("Could not list recovery copies. {error}"),
            },
        })
    } else {
        None
    };
    let prefs = app.preferences();
    ViewSnapshot {
        active_lane: app.active_lane(),
        audition: prefs.audition,
        bookmark_step: None,
        busy: app.is_busy(),
        can_undo_revert: app.can_undo_revert(),
        classic_browser: prefs.classic_browser,
        cycle: app.cycle_enabled(),
        dirty: app.is_dirty(),
        drum_view: None,
        drum_vst: app.drum_vst().map(|v| DrumVstSnapshot {
            enabled: v.is_enabled(),
            instance_id: v.instance_id.clone(),
            loaded: true,
            mapped_lanes: v.routed_notes().len(),
            plugin_name: v.plugin_name.clone(),
            plugin_path: v.plugin_path.clone(),
            plugin_uid: v.plugin_uid.clone(),
        }),
        duration: app.duration_ms(),
        edit_dialog: app.requested_edit(),
        error: app.error().map(str::to_owned),
        external_osu_changed: app.external_osu_changed(),
        first_launch: app.first_launch(),
        follow: prefs.follow,
        has_document: document.is_some(),
        has_original_snapshot: app.has_original_snapshot(),
        hitsound_library: prefs.hitsound_library.clone(),
        hitsounds: f64::from(prefs.hitsound_gain),
        hitsounds_muted: prefs.hitsounds_muted,
        hitsounds_solo: app.hitsounds_solo(),
        hs_mode: document.is_some_and(|d| d.hs_mode),
        is_lazer_external_edit: document.is_some_and(|d| crate::osu_link::is_lazer_external_edit_path(&d.path)),
        is_practice_mapset: document.is_some_and(|d| dawsu_application::practice::is_practice_mapset_path(&d.path)),
        key_preset: prefs.key_preset.clone(),
        lane_count: document.map_or(0, |d| d.composer.lanes.len()),
        left: app.locator_left(),
        main: f64::from(prefs.master_gain),
        musical_position,
        object_lines: prefs.object_lines,
        offset: app.song_offset(),
        output_device: app.device_name().to_owned(),
        output_problems,
        panels: PanelsSnapshot {
            browser: prefs.show_browser,
            controller: prefs.show_controller,
            inspector: prefs.show_inspector,
            lane_names: prefs.lane_names,
        },
        peaks: PeaksSnapshot {
            hitsounds: f64::from(hitsound_peak),
            master: f64::from(master_peak),
            song: f64::from(song_peak),
        },
        pending_home: None,
        pending_open: app.has_pending_open(),
        playback_start: None,
        playing: app.is_playing(),
        position,
        // The map preview was in its own window when the editor last closed: main opens it again.
        preview_window: prefs.preview_window.is_some_and(|place| place.open),
        quantize_strength: app.quantize_strength(),
        ramp_shape: format!("{:?}", app.ramp_shape()),
        recent: prefs.recent.clone(),
        recent_meta: (document.is_none() || include_static).then(|| {
            prefs
                .recent
                .iter()
                .map(|path| cached_recent_meta(path))
                .collect()
        }),
        recovery,
        return_on_stop: prefs.return_on_stop,
        reload_osu_after_save: prefs.reload_osu_after_save,
        clean_redundant_greenlines: prefs.clean_redundant_greenlines,
        keep_pitch: prefs.keep_pitch,
        auto_check_updates: prefs.auto_check_updates,
        step_pads_mode: prefs.step_pads_mode,
        tap_recorded_count: app.tap_recorded_count(),
        tap_recording_armed: app.tap_recording_armed(),
        revert: app.revert_requested(),
        review,
        right: app.locator_right(),
        save_problem_review,
        save_review: app.save_review_files().map(|files| {
            files.into_iter().map(|(label, path)| (label, path.to_path_buf())).collect()
        }),
        scene,
        shortcut_action: None,
        shortcut_help: None,
        shortcut_open: None,
        show_home: None,
        song: f64::from(prefs.song_gain),
        song_loading: app.is_song_loading(),
        song_muted: prefs.song_muted,
        song_solo: app.song_solo(),
        songs_folder: (!prefs.songs_folder.as_os_str().is_empty())
            .then(|| prefs.songs_folder.clone()),
        sound_set_prompts: document.map(|_| app.sound_set_prompts()).unwrap_or_default().iter().map(|p| {
            SoundSetPromptSnapshot {
                edited_keep_files: p.edited_keep_files,
                name: p.name.clone(),
                open: app.sound_set_prompt_open(),
                placements: p.placements,
                set_id: p.set_id.clone(),
            }
        }).collect(),
        speed: app.playback_speed(),
        status: app.status().to_owned(),
        stem_active: app.is_monitoring_stem(),
        stem_loaded: app.has_drum_stem(),
        stem_loading: app.is_drum_stem_loading(),
        stem_name: app.drum_stem_name(),
        tab_routes,
        tick_rev: None,
        tool: format!("{:?}", app.tool()),
        warnings: app.warnings().to_vec(),
        waveform_revision: app.waveform_revision(),
    }
}

pub(crate) fn encode_waveform_bytes(bins: &[dawsu_application::WaveBin]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bins.len() * 6 * 4);
    for bin in bins {
        for value in [bin.peak, bin.low, bin.mid, bin.high, bin.amp_l, bin.amp_r] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

#[derive(Default)]
pub(crate) struct ViewTickCache {
    pub(crate) rev: u64,
    pub(crate) last_view_key: Option<(dawsu_application::ViewStateToken, bool, bool, PlaybackStart, u64)>,
    pub(crate) drums_rev: u64,
    pub(crate) static_scene_key: Option<(u64, u64, usize)>,
    pub(crate) last_hs_mode: bool,
}

impl ViewTickCache {
    pub(crate) fn invalidate(&mut self) {
        self.rev += 1;
        self.last_view_key = None;
    }
}

pub(crate) fn view_tick_key(
    app: &Application,
    home: &home::Route,
    playback_start: PlaybackStart,
    drums_rev: u64,
) -> (dawsu_application::ViewStateToken, bool, bool, PlaybackStart, u64) {
    (
        app.view_state_token(),
        home.visible(),
        home.pending(),
        playback_start,
        drums_rev,
    )
}

pub(crate) fn decorate_and_stamp_view(
    app: &Application,
    home: &home::Route,
    drums: &drum_view::DrumView,
    playback_start: PlaybackStart,
    cache: &mut ViewTickCache,
    want_scene: bool,
) -> ViewResponse {
    let mut view = snapshot_view(app, want_scene);
    home.add_status(&mut view);
    drums.decorate(app, &mut view);
    view.playback_start = Some(playback_start);
    cache.rev += 1;
    cache.last_view_key = Some(view_tick_key(app, home, playback_start, cache.drums_rev));
    view.tick_rev = Some(cache.rev);
    ViewResponse::View(Box::new(view))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn next_view_response(
    app: &mut Application,
    home: &mut home::Route,
    drums: &mut drum_view::DrumView,
    playback_start: &mut PlaybackStart,
    cache: &mut ViewTickCache,
    pending_initial_open: &mut Option<PathBuf>,
    scene: bool,
    action: Option<Action>,
    fresh: bool,
    tick_rev: Option<u64>,
) -> ViewResponse {
    let had_action = action.is_some();
    match action {
        Some(Action::Open { path }) if app.first_launch() => {
            if map_path(&path) {
                *pending_initial_open = Some(path);
            }
        }
        Some(Action::OpenPracticeMapset) if app.first_launch() => {}
        Some(action) => {
            if let Some(action) = home.handle(action, app) {
                action.run_with_start(app, playback_start);
            }
        }
        None => {}
    }
    if !app.first_launch()
        && let Some(path) = pending_initial_open.take()
    {
        app.request_open(path);
    }
    app.poll();
    home.settle(app);
    drums.sync_on_document_open(app, cache);

    if !had_action && !scene && !fresh && tick_rev == Some(cache.rev) {
        let key = view_tick_key(app, home, *playback_start, cache.drums_rev);
        if cache.last_view_key == Some(key) {
            let position = app.position_ms();
            let (master_peak, song_peak, hitsound_peak) = app.audio_peaks();
            return ViewResponse::Tick(ViewTickSnapshot {
                tick: true,
                rev: cache.rev,
                position,
                playing: app.is_playing(),
                busy: app.is_busy(),
                peaks: PeaksSnapshot {
                    hitsounds: f64::from(hitsound_peak),
                    master: f64::from(master_peak),
                    song: f64::from(song_peak),
                },
                musical_position: musical_position_text(app, position),
            });
        }
    }

    let want_scene = scene && !home.visible();
    let current_hs_mode = app.document().is_some_and(|d| d.hs_mode);
    let include_static = if want_scene && app.document().is_some() {
        let key = (app.document_generation(), app.waveform_revision(), app.active_waveform().len());
        let changed = fresh || cache.static_scene_key != Some(key) || cache.last_hs_mode != current_hs_mode;
        cache.static_scene_key = Some(key);
        cache.last_hs_mode = current_hs_mode;
        changed
    } else {
        if home.visible() || app.document().is_none() {
            cache.static_scene_key = None;
            cache.last_hs_mode = false;
        }
        false
    };
    let mut view = snapshot_with_static(app, want_scene, include_static);
    home.add_status(&mut view);
    drums.decorate(app, &mut view);
    view.playback_start = Some(*playback_start);
    if let Some(request @ ("filter" | "newDiff" | "openFolder" | "importOsz" | "openHitsoundFolder" | "reviewOutput")) = app.requested_edit() {
        view.shortcut_action = Some(request);
        view.edit_dialog = None;
        app.cancel_timeline_edit();
    }
    view.shortcut_help = Some(app.take_shortcut_help());
    // The Inspector's bookmark pick follows Previous and Next, from a button, menu or key.
    view.bookmark_step = Some(app.take_bookmark_step());
    view.shortcut_open = Some(app.take_shortcut_open_request());
    cache.rev += 1;
    cache.last_view_key = Some(view_tick_key(app, home, *playback_start, cache.drums_rev));
    view.tick_rev = Some(cache.rev);
    ViewResponse::View(Box::new(view))
}
