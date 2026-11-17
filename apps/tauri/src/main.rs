#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod folders;
mod home;
mod keyboard;
mod library;
mod selection;

use dawsu_application::{Application, ExportSettings, PlatformRequest, EditorTimeSignature, LaneLayout, PointerPhase, RangeOperation, TimelineEdit, VolumeScope, lane_name};
use dawsu_editor::{
    keys::{Key, KeyCombination, Mods},
    tempo::SnapMode,
    tools::{PointerButton, PointerInput, PointerSurface, ToolKind, ToolPreview},
    volume_ramp::{RampShape, ramp_at},
};
use dawsu_foundation::{CustomIndex, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_hitsounding::{
    LaneSource, Place,
    copy::{CopyMode, CopyOptions},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::mpsc, time::Duration};
use tauri::{Emitter, Manager, State};

#[derive(Clone, Copy, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum PlaybackStart {
    #[default]
    Cursor,
    Left,
    Selection,
    SelectionOrLeft,
}

impl PlaybackStart {
    fn position(self, app: &Application) -> Option<f64> {
        let start = || {
            let composer = &app.document()?.composer;
            let moments = composer.hitsounds().moments();
            composer.selected.items().iter()
                .filter_map(|hit| moments.get(hit.moment))
                .map(|moment| moment.time().ms())
                .min_by(f64::total_cmp)
        };
        match self {
            Self::Cursor => None,
            Self::Left => Some(app.locator_left),
            Self::Selection => start(),
            Self::SelectionOrLeft => start().or(Some(app.locator_left)),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum EditorPanel {
    Browser,
    Inspector,
    Controller,
    LaneNames,
}

#[derive(Deserialize)]
#[serde(remote = "CopyMode", rename_all = "snake_case")]
enum CopyModeJson {
    OverwriteEverything,
    OverwriteOnlyDefined,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SampleSetJson {
    Normal,
    Soft,
    Drum,
}

fn deserialize_sample_set<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<SampleSet>, D::Error> {
    Ok(Option::<SampleSetJson>::deserialize(input)?.map(|set| match set {
        SampleSetJson::Normal => SampleSet::Normal,
        SampleSetJson::Soft => SampleSet::Soft,
        SampleSetJson::Drum => SampleSet::Drum,
    }))
}

/// Tauri JSON adapter for the application's existing Copy options.
#[derive(Deserialize)]
#[serde(remote = "CopyOptions")]
struct CopyOptionsJson {
    #[serde(with = "CopyModeJson")]
    mode: CopyMode,
    leniency: f64,
    copy_hitsounds: bool,
    copy_body_hitsounds: bool,
    copy_sample_sets: bool,
    copy_volumes: bool,
    always_preserve_5_volume: bool,
    copy_storyboarded_samples: bool,
    ignore_hitsound_satisfied_samples: bool,
    ignore_whenever_hitsound: bool,
    mute_slider_ends: bool,
    beat_divisors: Vec<u32>,
    muted_divisors: Vec<u32>,
    min_length: f64,
    muted_index: Option<u32>,
    #[serde(deserialize_with = "deserialize_sample_set")]
    muted_sample_set: Option<SampleSet>,
}

#[derive(Deserialize)]
struct ShortcutInput {
    key: String,
    code: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TimelineEditRequest {
    Quantize { strength: f64 },
    SelectionStart { position: f64 },
    Repeat { copies: u32, gap_steps: u32 },
    Range { operation: String, offset: f64, rows: Option<Vec<usize>> },
    Scale { scope: String, low: i32, high: i32, percent: f64 },
}

impl TimelineEditRequest {
    fn parse(self) -> Option<TimelineEdit> {
        Some(match self {
            Self::Quantize { strength } => TimelineEdit::Quantize(strength),
            Self::SelectionStart { position } => TimelineEdit::MoveSelectionStart(position),
            Self::Repeat { copies, gap_steps } => TimelineEdit::Repeat { copies, gap_steps },
            Self::Range { operation, offset, rows } => {
                let operation = match operation.as_str() {
                    "copy" => RangeOperation::Copy,
                    "move" => RangeOperation::Move,
                    "delete" => RangeOperation::Delete,
                    _ => return None,
                };
                TimelineEdit::Range { operation, offset, rows }
            }
            Self::Scale { scope, low, high, percent } => {
                let scope = match scope.as_str() {
                    "selected" => VolumeScope::Selected,
                    "selected_lanes" => VolumeScope::SelectedLanes,
                    "all" => VolumeScope::All,
                    _ => return None,
                };
                TimelineEdit::Scale { scope, low, high, percent }
            }
        })
    }
}

#[derive(Deserialize)]
struct SampleTarget {
    map: PathBuf,
    lane: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Action {
    Home,
    HomeChoice { choice: String },
    ChangeShortcut { id: String, chord: Option<ShortcutInput> },
    Resume,
    BrowserStyle { classic: bool },
    ReplaceSample {
        target: SampleTarget,
        path: PathBuf,
        mapset: bool,
    },
    PreviewLibrarySample {
        location: library::Location,
    },
    ReplaceLibrarySample {
        target: SampleTarget,
        location: library::Location,
    },
    Open {
        path: PathBuf,
    },
    Pending {
        choice: String,
    },
    Command {
        id: String,
    },
    TimelineEdit { path: PathBuf, edit: TimelineEditRequest },
    CancelTimelineEdit,
    Key {
        key: String,
        code: String,
        ctrl: bool,
        shift: bool,
        alt: bool,
    },
    Save,
    ReviewSave,
    BookmarkAdd { path: PathBuf, time: f64, label: String },
    BookmarkEdit { path: PathBuf, time: f64, label: Option<String> },
    BookmarkJump { time: f64 },
    Panel {
        panel: EditorPanel,
        on: bool,
    },
    ObjectLines {
        on: bool,
    },
    DismissSaveProblemReview,
    ConfirmRevert,
    CancelRevert,
    Play,
    PlaybackStart { value: PlaybackStart },
    ReturnOnStop { on: bool },
    Stop,
    Start,
    Seek {
        ms: f64,
    },
    Snap {
        on: bool,
        mode: String,
        beats: u32,
        divisor: u32,
    },
    Tool {
        name: String,
    },
    RampShape {
        value: String,
    },
    Pointer {
        phase: String,
        ms: f64,
        row: i32,
        x: f64,
        y: f64,
        value: Option<f64>,
        shift: bool,
        ctrl: bool,
        alt: bool,
    },
    History {
        position: usize,
    },
    Lane {
        row: usize,
        operation: String,
    },
    LaneLayout {
        value: LaneLayout,
    },
    TimeSignatures {
        map: PathBuf,
        value: Vec<EditorTimeSignature>,
    },
    ExportSettings {
        map: PathBuf,
        value: ExportSettings,
    },
    GroupMix {
        set: String,
        operation: String,
    },
    Volume {
        value: i32,
        relative: bool,
    },
    InsertVolume {
        value: i32,
    },
    SliderBody {
        path: PathBuf,
        index: usize,
        #[serde(default, deserialize_with = "deserialize_sample_set")]
        set: Option<SampleSet>,
        whistle: bool,
        volume: i32,
    },
    Gains {
        song: f32,
        main: f32,
    },
    SongMuted {
        on: bool,
    },
    OutputDevice {
        name: String,
    },
    OutputBuffer {
        ms: Option<u32>,
    },
    Audition {
        on: bool,
    },
    Offset {
        ms: f64,
    },
    Speed {
        value: f64,
    },
    HsMode {
        on: bool,
    },
    Protect {
        on: bool,
    },
    Locator {
        end: String,
        jump: bool,
    },
    Cycle {
        on: bool,
    },
    Follow {
        on: bool,
    },
    View {
        start: f64,
        zoom: f64,
        row_height: f32,
    },
    KeyPreset {
        name: String,
    },
    Generate {
        #[serde(default)]
        include_slider_audio: bool,
    },
    NewDifficulty {
        name: String,
    },
    Copy {
        targets: Vec<PathBuf>,
        #[serde(with = "CopyOptionsJson")]
        options: CopyOptions,
    },
    Output {
        apply: bool,
    },
    AuditionOutputMix { index: usize },
    DismissError,
    CancelJob,
}

impl Action {
    #[cfg(test)]
    fn run(self, app: &mut Application) {
        self.run_with_start(app, &mut PlaybackStart::default());
    }

    fn run_with_start(self, app: &mut Application, playback_start: &mut PlaybackStart) {
        match self {
            Self::Home | Self::HomeChoice { .. } | Self::Resume => {}
            Self::ChangeShortcut { id, chord } => {
                let result = if let Some(input) = chord {
                    keyboard::chord(&input.key, &input.code, input.ctrl, input.shift, input.alt)
                        .ok_or_else(|| "This key is unavailable for shortcuts.".to_owned())
                        .and_then(|chord| app.change_shortcut(&id, Some(chord.combination())))
                } else {
                    app.change_shortcut(&id, None)
                };
                if let Err(error) = result {
                    app.error = Some(error);
                }
            }
            Self::Open { path } => {
                if map_path(&path) {
                    app.request_open(path);
                }
            }
            Self::BrowserStyle { classic } => {
                app.preferences.classic_browser = classic;
                if classic && app.preferences.show_browser {
                    app.preferences.show_inspector = true;
                }
                app.save_preferences();
            }
            Self::ReplaceSample { target, path, mapset } => {
                if let Err(error) = assign_sample(app, target, path, mapset) {
                    app.error = Some(error);
                }
            }
            Self::PreviewLibrarySample { location } => {
                if let Err(error) = location.sample(app.preferences.hitsound_library.as_deref())
                    .and_then(|sample| app.audition_sample_file(&sample))
                {
                    app.error = Some(error);
                }
            }
            Self::ReplaceLibrarySample { target, location } => {
                if let Err(error) = location.sample(app.preferences.hitsound_library.as_deref())
                    .and_then(|sample| assign_sample(app, target, sample, false))
                {
                    app.error = Some(error);
                }
            }
            Self::Pending { choice } => match choice.as_str() {
                "save" => app.save(),
                "discard" => app.discard_and_open(),
                _ => app.pending_open = None,
            },
            Self::Command { id } => app.command(&id),
            Self::TimelineEdit { path, edit } => {
                if let Some(edit) = edit.parse() {
                    app.apply_timeline_edit(&path, edit);
                }
            }
            Self::CancelTimelineEdit => app.cancel_timeline_edit(),
            Self::Key {
                key,
                code,
                ctrl,
                shift,
                alt,
            } => {
                if let Some(chord) = keyboard::chord(&key, &code, ctrl, shift, alt) {
                    if app.keymap.command_for(chord.combination()) == Some("play") {
                        app.play_pause_from(playback_start.position(app));
                    } else {
                        app.key(chord);
                    }
                }
            }
            Self::Save => app.save(),
            Self::ReviewSave => app.review_save(),
            Self::Panel { panel, on } => {
                match panel {
                    EditorPanel::Browser => {
                        app.preferences.show_browser = on;
                        if on && app.preferences.classic_browser {
                            app.preferences.show_inspector = true;
                        }
                    }
                    EditorPanel::Inspector => app.preferences.show_inspector = on,
                    EditorPanel::Controller => app.preferences.show_controller = on,
                    EditorPanel::LaneNames => app.preferences.lane_names = on,
                }
                app.save_preferences();
            }
            Self::ObjectLines { on } => {
                app.preferences.object_lines = on;
                app.save_preferences();
            }
            Self::DismissSaveProblemReview => app.dismiss_save_problem_review(),
            Self::ConfirmRevert => app.confirm_revert(),
            Self::CancelRevert => app.revert_requested = false,
            Self::Play => app.play_pause_from(playback_start.position(app)),
            Self::PlaybackStart { value } => *playback_start = value,
            Self::ReturnOnStop { on } => {
                app.preferences.return_on_stop = on;
                app.save_preferences();
            }
            Self::Stop => {
                if app.is_playing() {
                    app.stop();
                } else {
                    app.seek(0.0);
                }
            }
            Self::Start => app.seek(0.0),
            Self::Seek { ms } => app.seek(ms),
            Self::BookmarkAdd { path, time, label } => {
                if app.document().is_some_and(|doc| doc.path == path)
                    && let Err(error) = app.add_bookmark_at(time, Some(label))
                {
                    app.error = Some(error);
                }
            }
            Self::BookmarkEdit { path, time, label } => {
                if app.document().is_some_and(|doc| doc.path == path) {
                    let result = match label {
                        Some(label) => app.rename_bookmark(time, label),
                        None => app.remove_bookmark(time),
                    };
                    if let Err(error) = result {
                        app.error = Some(error);
                    }
                }
            }
            Self::BookmarkJump { time } => app.jump_to_bookmark(time),
            Self::Snap {
                on,
                mode,
                beats,
                divisor,
            } => {
                let mode = match mode.as_str() {
                    "relative" => SnapMode::GridRelative,
                    "objects" => SnapMode::Objects,
                    _ => SnapMode::Grid,
                };
                app.set_snap_options(on, beats, divisor, mode);
            }
            Self::Tool { name } => {
                if let Some(kind) = ToolKind::ALL
                    .into_iter()
                    .find(|kind| format!("{kind:?}") == name)
                {
                    app.set_tool(kind);
                }
            }
            Self::RampShape { value } => {
                let shape = match value.as_str() {
                    "Linear" => RampShape::Linear,
                    "EaseIn" => RampShape::EaseIn,
                    "EaseOut" => RampShape::EaseOut,
                    "SCurve" => RampShape::SCurve,
                    _ => return,
                };
                app.set_ramp_shape(shape);
            }
            Self::Pointer {
                phase,
                ms,
                row,
                x,
                y,
                value,
                shift,
                ctrl,
                alt,
            } => {
                let phase = match phase.as_str() {
                    "down" => PointerPhase::Down,
                    "move" => PointerPhase::Move,
                    "up" => PointerPhase::Up,
                    _ => PointerPhase::Cancel,
                };
                let mut mods = Mods::NONE;
                for (on, flag) in [(shift, Mods::SHIFT), (ctrl, Mods::CTRL), (alt, Mods::ALT)] {
                    if on {
                        mods = mods.union(flag);
                    }
                }
                app.pointer(
                    phase,
                    PointerInput {
                        time: SongTime::from_ms(ms.max(0.0)),
                        row,
                        row_value: value,
                        surface: if value.is_some() {
                            PointerSurface::ValueLane
                        } else {
                            PointerSurface::Grid
                        },
                        button: PointerButton::Left,
                        mods,
                        click_count: 1,
                        x,
                        y,
                    },
                );
            }
            Self::History { position } => app.go_to_history(position),
            Self::Lane { row, operation } => {
                if let Some(lane) = app
                    .document()
                    .and_then(|d| d.composer.lanes.get(row))
                    .cloned()
                {
                    match operation.as_str() {
                        "mute" => app.mute(lane),
                        "solo" => app.solo(lane),
                        "add_layer" if !app.busy => {
                            app.active_lane = row;
                            app.command("addNumberedLayer");
                        }
                        "add_layer" => {}
                        "preview_sample" => app.audition(&lane),
                        _ => {
                            app.active_lane = row;
                            app.audition(&lane);
                        }
                    }
                }
            }
            Self::GroupMix { set, operation } => group_mix(app, &set, &operation),
            Self::LaneLayout { value } => {
                if let Err(error) = app.set_lane_layout(value) {
                    app.error = Some(error);
                }
            }
            Self::TimeSignatures { map, value } => {
                if !app.document().is_some_and(|document| document.path == map) {
                    app.error = Some("The open difficulty changed. Reopen Time signatures to try again.".into());
                } else if let Err(error) = app.set_time_signatures(value) {
                    app.error = Some(error);
                }
            }
            Self::ExportSettings { map, value } => {
                if !app.document().is_some_and(|document| document.path == map) {
                    app.error = Some("The open difficulty changed. Reopen Merge settings to try again.".into());
                } else if let Err(error) = app.set_export_settings(value) {
                    app.error = Some(error);
                }
            }
            Self::Volume { value, relative } => app.selection_volume(value, relative),
            Self::InsertVolume { value } => app.set_insert_volume(value),
            Self::SliderBody { path, index, set, whistle, volume } => {
                if app.document().is_some_and(|doc| doc.path == path)
                    && let Err(error) = app.edit_slider_body(index, set, whistle, volume)
                {
                    app.error = Some(error);
                }
            }
            Self::Gains { song, main } => {
                app.preferences.song_gain = song.clamp(0.0, 1.0);
                app.preferences.master_gain = main.clamp(0.2, 1.0);
                app.gains_changed();
            }
            Self::OutputDevice { name } => app.select_device(name),
            Self::OutputBuffer { ms } => {
                if let Err(error) = app.set_output_buffer(ms) {
                    app.error = Some(error);
                }
            }
            Self::SongMuted { on } => {
                app.preferences.song_muted = on;
                app.gains_changed();
            }
            Self::Audition { on } => app.preferences.audition = on,
            Self::Offset { ms } => {
                app.song_offset = ms.clamp(-60_000.0, 60_000.0);
                app.cycle_changed();
            }
            Self::Speed { value } => app.set_speed(value),
            Self::HsMode { on } => app.set_hs_mode(on),
            Self::Protect { on } => app.set_normal_protection(on),
            Self::Locator { end, jump } => {
                if jump {
                    app.seek(if end == "left" {
                        app.locator_left
                    } else {
                        app.locator_right
                    });
                } else {
                    let position = app.position_ms();
                    if end == "left" {
                        app.locator_left = position;
                    } else {
                        app.locator_right = position;
                    }
                    app.cycle_changed();
                }
            }
            Self::Cycle { on } => {
                app.cycle = on;
                app.cycle_changed();
            }
            Self::Follow { on } => {
                app.preferences.follow = on;
                app.save_preferences();
            }
            Self::View {
                start,
                zoom,
                row_height,
            } => {
                app.set_view_start(start.clamp(0.0, app.duration_ms()));
                app.zoom = zoom.clamp(0.002, 3.0);
                app.row_height = row_height.clamp(20.0, 65.0);
            }
            Self::KeyPreset { name } => {
                app.choose_key_preset(if name == "osu!" { "osu!" } else { "DAW" })
            }
            Self::Generate { include_slider_audio } => app.generate_with_slider_audio(include_slider_audio),
            Self::NewDifficulty { name } => app.new_diff(name),
            Self::Copy { targets, options } => app.copy_hitsounds(targets, options),
            Self::Output { apply } => {
                if apply {
                    app.apply_output();
                } else {
                    app.output_review = None;
                }
            }
            Self::AuditionOutputMix { index } => {
                if let Err(error) = app.audition_output_mix(index) {
                    app.error = Some(error);
                }
            }
            Self::DismissError => app.error = None,
            Self::CancelJob => app.cancel_job(),
        }
    }
}

fn map_path(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("osu") || ext.eq_ignore_ascii_case("osz"))
}

fn sample_set_name(set: SampleSet) -> &'static str {
    match set {
        SampleSet::Normal => "normal",
        SampleSet::Soft => "soft",
        SampleSet::Drum => "drum",
    }
}

fn group_mix(app: &mut Application, set: &str, operation: &str) {
    if app.busy {
        return;
    }
    let (enabled, toggle): (fn(&Application, &SampleIdentity) -> bool, fn(&mut Application, SampleIdentity)) =
        match operation {
            "mute" => (Application::is_muted, Application::mute),
            "solo" => (Application::is_solo, Application::solo),
            _ => return,
        };
    let Some(document) = app.document() else {
        return;
    };
    let lanes = document.composer.lanes.iter().filter(|lane| {
        matches!(lane, SampleIdentity::Standard { set: lane_set, .. } if sample_set_name(*lane_set) == set)
    }).cloned().collect::<Vec<_>>();
    let on = lanes.iter().any(|lane| !enabled(app, lane));
    for lane in lanes {
        if enabled(app, &lane) != on {
            toggle(app, lane);
        }
    }
}

fn assign_sample(app: &mut Application, target: SampleTarget, path: PathBuf, mapset: bool) -> Result<(), String> {
    let document = app.document().ok_or("Open a difficulty first.")?;
    let lane = document.composer.lanes.get(app.active_lane).ok_or("Choose an active lane.")?;
    if document.path != target.map || format!("{lane:?}") != target.lane {
        return Err("The active lane changed while choosing a sample.".into());
    }
    let lane = lane.clone();
    let path = if mapset {
        let name = path.to_str().ok_or("Invalid sample filename.")?;
        if !document.composer.hitsounds().lanes().iter().any(|lane| {
            matches!(&lane.source, LaneSource::File(sample) if sample == name)
        }) {
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
    app.replace_lane_sample(&target.map, &lane, &path)
}


fn snapshot(app: &Application, include_scene: bool) -> Value {
    let document = app.document();
    let position = app.position_ms();
    let musical_position = document.map(|document| {
        let composer = &document.composer;
        let time = SongTime::from_ms(position);
        let tempo = composer.tempo.section_at(time);
        let (beat, (numerator, denominator)) = document.editor_bar_at(time);
        let tick = (beat.beat_fraction * 480.0 + 1e-9).floor() as u32;
        let snap = if composer.snap_enabled {
            format!("{}/{}", composer.spacing.beats(), composer.spacing.divisions())
        } else {
            "Snap off".into()
        };
        format!(
            "{}.{}.{:03} · {:.1} BPM · {numerator}/{denominator} · {snap}",
            beat.bar, beat.beat, tick.min(479), 60_000.0 / tempo.beat_length_ms,
        )
    });
    let scene = if include_scene {
        document.map(|d| {
            let c = &d.composer;
            let model = c.hitsounds();
            let row_by_lane: std::collections::HashMap<_, _> =
                c.lanes.iter().enumerate().map(|(row, lane)| (lane, row)).collect();
            let used: std::collections::HashSet<_> = model.moments().iter().flat_map(|moment| moment.hits()).collect();
            let mut slot_rows = std::collections::HashMap::new();
            let mut backing_slots = std::collections::HashMap::new();
            for (source, slot) in model.lane_slots() {
                if !used.contains(slot) && let (Some(&from), Some(&to)) = (row_by_lane.get(source), row_by_lane.get(slot))
                    && let std::collections::hash_map::Entry::Vacant(entry) = slot_rows.entry(to)
                {
                    entry.insert(from);
                    backing_slots.insert(from, slot);
                }
            }
            let lane_order = (0..c.lanes.len()).filter(|row| !backing_slots.contains_key(row))
                .map(|row| slot_rows.get(&row).copied().unwrap_or(row)).collect::<Vec<_>>();
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
                    LaneSource::File(name) => Some(name),
                    LaneSource::Skin => None,
                });
                json!({
                    "family": family, "index": index, "group": group, "key": format!("{slot:?}"),
                    "sourceKey": format!("{lane:?}"), "sample": sample,
                    "name": lane_name(slot), "muted": app.is_muted(lane), "solo": app.is_solo(lane),
                    "canAddLayer": index.is_some(),
                    "canReplace": !matches!(lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. }),
                })
            }).collect::<Vec<_>>();
            let selected: std::collections::HashSet<_> = c.selected.items().iter()
                .map(|key| (key.moment, &key.lane)).collect();
            let listen_muted: std::collections::HashSet<_> = c.listen_muted.iter()
                .map(|key| (key.moment, &key.lane)).collect();
            let moments = c.hitsounds().moments().iter().enumerate().map(|(i, moment)| {
                let hits = moment.hits().filter_map(|lane| row_by_lane.get(lane)
                    .map(|row| {
                        let selected = selected.contains(&(i, lane));
                        if listen_muted.contains(&(i, lane)) {
                            json!([row, selected, true])
                        } else {
                            json!([row, selected])
                        }
                    })).collect::<Vec<_>>();
                json!([moment.time().ms(), moment.volume(), moment.place() != Place::NoObject, hits])
            }).collect::<Vec<_>>();
            let sections = c.tempo.sections().iter()
                .map(|s| json!([s.start.ms(), s.beat_length_ms, s.beats_per_bar]))
                .collect::<Vec<_>>();
            let waveform = app.waveform.iter()
                .map(|b| json!([b.peak, b.low, b.mid, b.high])).collect::<Vec<_>>();
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
                selection["sliderBody"] = json!({
                    "index": index,
                    "start": body.start().ms(),
                    "set": body.set().map(sample_set_name),
                    "whistle": body.whistle(),
                    "volume": body.volume(),
                });
            }
            json!({
                "title": d.title, "path": d.path, "difficulties": d.difficulties,
                "lanes": lanes, "laneOrder": lane_order, "moments": moments, "sections": sections, "waveform": waveform,
                "selection": selection, "history": c.history.steps(), "historyPosition": c.history.position(),
                "snap": c.snap_enabled, "snapMode": match c.snap_mode { SnapMode::Grid => "grid", SnapMode::GridRelative => "relative", SnapMode::Objects => "objects" },
                "beats": c.spacing.beats(), "divisor": c.spacing.divisions(),
                "insertVolume": c.insert_volume, "protect": c.protect_normals, "hs": d.hs_mode,
                "viewStart": c.view_start, "zoom": app.zoom, "rowHeight": app.row_height,
                "range": c.range.map(|r| [r.from.ms(), r.to.ms()]),
                "repeatGap": app.requested_edit().filter(|&id| id == "repeat").and_then(|_| c.repeat_gap()),
                "laneLayout": d.lane_layout(),
                "timeSignatures": d.time_signatures(), "editorBars": d.editor_bars(),
                "bookmarks": d.bookmarks().iter().map(|b| json!({"time": b.time.ms(), "label": b.label})).collect::<Vec<_>>(),
                "rampPreview": ramp_preview,
            })
        })
    } else {
        None
    };
    let review = app.output_review.as_ref().map(|plan| {
        let files = plan.files.iter().map(|file| {
            let summary = file.file.change_summary();
            json!({
                "path": file.path,
                "replace": file.expected.is_some(),
                "changed": summary.hit_objects_changed,
                "added": summary.hit_objects_added,
                "removed": summary.hit_objects_removed,
                "timing": summary.green_lines_added + summary.green_lines_edited
                    + summary.green_lines_removed + summary.red_lines_changed,
            })
        }).collect::<Vec<_>>();
        json!({
            "label": plan.label, "details": plan.details, "files": files,
            "mixes": plan.mix_paths().collect::<Vec<_>>(),
            "combinations": plan.export_combinations(),
            "includeSliderAudio": plan.includes_slider_audio(),
        })
    });
    let mut result = json!({
        "scene": scene, "hasDocument": document.is_some(), "busy": app.busy, "playing": app.is_playing(),
        "laneCount": document.map_or(0, |d| d.composer.lanes.len()),
        "position": position, "musicalPosition": musical_position, "duration": app.duration_ms(), "dirty": app.is_dirty(),
        "status": app.status, "error": app.error, "warnings": app.warnings, "speed": app.playback_speed,
        "song": app.preferences.song_gain, "main": app.preferences.master_gain,
        "returnOnStop": app.preferences.return_on_stop, "songMuted": app.preferences.song_muted, "audition": app.preferences.audition, "offset": app.song_offset,
        "left": app.locator_left, "right": app.locator_right, "cycle": app.cycle, "follow": app.preferences.follow,
        "tool": format!("{:?}", app.tool()), "pendingOpen": app.pending_open.is_some(),
        "rampShape": format!("{:?}", app.ramp_shape()),
        "revert": app.revert_requested, "review": review, "activeLane": app.active_lane,
        "saveProblemReview": app.save_problem_review(), "objectLines": app.preferences.object_lines,
        "classicBrowser": app.preferences.classic_browser,
        "hitsoundLibrary": app.preferences.hitsound_library,
        "panels": {
            "browser": app.preferences.show_browser,
            "inspector": app.preferences.show_inspector,
            "controller": app.preferences.show_controller,
            "laneNames": app.preferences.lane_names,
        },
        "outputProblems": app.output_problems().iter().map(|problem| json!({
            "target": problem.target, "timeline": problem.timeline,
            "time": problem.time, "message": problem.message,
        })).collect::<Vec<_>>(),
        "recent": app.preferences.recent, "firstLaunch": app.first_launch,
    });
    result["editDialog"] = json!(app.requested_edit());
    result["quantizeStrength"] = json!(app.quantize_strength());
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
    result["tabRoutes"] = json!(tab_routes);
    result
}

enum Request {
    View(bool, Option<Action>, mpsc::Sender<Value>),
    Help(mpsc::Sender<Value>),
    AudioOutputs(mpsc::Sender<Value>),
    Shortcuts(mpsc::Sender<Value>),
    ExportSettings(mpsc::Sender<Option<(PathBuf, ExportSettings)>>),
    KeyFile(PathBuf, bool, mpsc::Sender<Result<(), String>>),
    SongsDirectory(mpsc::Sender<Option<PathBuf>>),
    SetSongsDirectory(PathBuf, mpsc::Sender<Result<Value, String>>),
    LibraryDirectory(mpsc::Sender<Option<PathBuf>>),
    SetLibraryDirectory(PathBuf, mpsc::Sender<Result<Value, String>>),
    LibraryEntries(library::Location, mpsc::Sender<Result<Vec<(String, bool)>, String>>),
    Folder(folders::Folder, mpsc::Sender<Result<PathBuf, String>>),
    Close(bool, mpsc::Sender<Result<(), String>>),
}
struct Editor(mpsc::Sender<Request>);

fn editor_thread(initial: Option<PathBuf>) -> Editor {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut app = Application::new();
        let mut playback_start = PlaybackStart::default();
        let mut home = home::Route::default();
        let mut pending_initial_open = initial.filter(|path| map_path(path));
        if !app.first_launch && let Some(path) = pending_initial_open.take() {
            app.request_open(path);
        }
        loop {
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(Request::View(scene, action, response)) => {
                    match action {
                        Some(Action::Open { path }) if app.first_launch => {
                            if map_path(&path) {
                                pending_initial_open = Some(path);
                            }
                        }
                        Some(action) => {
                            if let Some(action) = home.handle(action, &mut app) {
                                action.run_with_start(&mut app, &mut playback_start);
                            }
                        }
                        None => {}
                    }
                    if !app.first_launch && let Some(path) = pending_initial_open.take() {
                        app.request_open(path);
                    }
                    app.poll();
                    home.settle(&mut app);
                    let mut view = snapshot(&app, scene && !home.visible());
                    home.add_status(&mut view);
                    view["playbackStart"] = json!(playback_start);
                    view["shortcutHelp"] = json!(std::mem::take(&mut app.help_requested));
                    view["shortcutOpen"] = json!(std::mem::take(&mut app.requests).iter()
                        .any(|request| matches!(request, PlatformRequest::OpenMap)));
                    let _ = response.send(view);
                }
                Ok(Request::Shortcuts(response)) => {
                    let _ = response.send(json!({
                        "preset": app.preferences.key_preset, "entries": app.shortcuts(),
                    }));
                }
                Ok(Request::KeyFile(path, import, response)) => {
                    let result = if import { app.import_shortcuts(&path) } else { app.export_shortcuts(&path) };
                    let _ = response.send(result);
                }
                Ok(Request::AudioOutputs(response)) => {
                    let mut devices = app.devices();
                    devices.sort();
                    devices.dedup();
                    let _ = response.send(json!({
                        "current": app.device_name(), "devices": devices,
                        "bufferMs": app.preferences.audio_buffer_ms,
                    }));
                }
                Ok(Request::Help(response)) => {
                    let _ = response.send(json!(
                        app.help()
                            .iter()
                            .map(|group| json!([
                                group.title,
                                group
                                    .entries
                                    .iter()
                                    .map(|e| json!([e.keys, e.description]))
                                    .collect::<Vec<_>>()
                            ]))
                            .collect::<Vec<_>>()
                    ));
                }
                Ok(Request::ExportSettings(response)) => {
                    let value = app.document().map(|document| {
                        (document.path.clone(), document.export_settings().cloned().unwrap_or_default())
                    });
                    let _ = response.send(value);
                }
                Ok(Request::SongsDirectory(response)) => {
                    let folder = &app.preferences.songs_folder;
                    let _ = response.send(folder.is_dir().then(|| folder.clone()));
                }
                Ok(Request::SetSongsDirectory(folder, response)) => {
                    let result = if app.first_launch {
                        Err("Choose a keyboard preset first.".into())
                    } else if !folder.is_dir() {
                        Err("The selected Songs folder is unavailable.".into())
                    } else {
                        app.preferences.songs_folder = folder;
                        app.save_preferences();
                        let mut view = snapshot(&app, false);
                        home.add_status(&mut view);
                        view["playbackStart"] = json!(playback_start);
                        Ok(view)
                    };
                    let _ = response.send(result);
                }
                Ok(Request::LibraryDirectory(response)) => {
                    let folder = app.preferences.hitsound_library.as_ref().filter(|path| path.is_dir());
                    let _ = response.send(folder.cloned());
                }
                Ok(Request::SetLibraryDirectory(folder, response)) => {
                    let result = (|| {
                        if app.first_launch {
                            return Err("Choose a keyboard preset first.".into());
                        }
                        let folder = folder.canonicalize().map_err(|e| format!("Opening hitsound library: {e}"))?;
                        if !folder.is_dir() {
                            return Err("The selected hitsound library folder is unavailable.".into());
                        }
                        app.preferences.hitsound_library = Some(folder);
                        app.save_preferences();
                        let mut view = snapshot(&app, false);
                        home.add_status(&mut view);
                        view["playbackStart"] = json!(playback_start);
                        Ok(view)
                    })();
                    let _ = response.send(result);
                }
                Ok(Request::LibraryEntries(location, response)) => {
                    let _ = response.send(location.entries(app.preferences.hitsound_library.as_deref()));
                }
                Ok(Request::Folder(folder, response)) => {
                    let path = folder.resolve(
                        app.document().map(|d| d.path.as_path()),
                        app.preferences.hitsound_library.as_deref(),
                        &dawsu_application::data_folder(),
                    );
                    let _ = response.send(path);
                }
                Ok(Request::Close(discard, response)) => {
                    let result = if app.busy {
                        Err("Wait for the current job to finish.".into())
                    } else if app.is_dirty() && !discard {
                        Err("Save or discard the current difficulty before quitting.".into())
                    } else {
                        app.save_session().map(|()| {
                            if !app.first_launch {
                                app.save_preferences();
                            }
                        })
                    };
                    let _ = response.send(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    app.poll();
                    home.settle(&mut app);
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Editor(send)
}

fn ask<T>(state: &Editor, request: impl FnOnce(mpsc::Sender<T>) -> Request) -> Result<T, String> {
    let (send, receive) = mpsc::channel();
    state
        .0
        .send(request(send))
        .map_err(|_| "The editor worker stopped".to_owned())?;
    receive
        .recv_timeout(Duration::from_secs(7))
        .map_err(|_| "The editor worker did not respond".into())
}

#[tauri::command]
fn editor_view(
    state: State<'_, Editor>,
    scene: bool,
    action: Option<Action>,
) -> Result<Value, String> {
    ask(&state, |tx| Request::View(scene, action, tx))
}

#[tauri::command]
fn editor_help(state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, Request::Help)
}

#[tauri::command]
fn editor_export_settings(state: State<'_, Editor>) -> Result<Option<(PathBuf, ExportSettings)>, String> {
    ask(&state, Request::ExportSettings)
}

#[tauri::command]
fn pick_merge_preset(window: tauri::WebviewWindow) -> Result<Option<ExportSettings>, String> {
    rfd::FileDialog::new().set_parent(&window).set_title("Load merge preset")
        .add_filter("Merge preset", &["json"]).pick_file()
        .map(|path| ExportSettings::read_file(&path)).transpose()
}

#[tauri::command]
fn save_merge_preset(window: tauri::WebviewWindow, value: ExportSettings) -> Result<bool, String> {
    use dawsu_foundation::safe_write::{self, BackupPolicy, WriteOutcome};
    let bytes = value.checked_bytes()?;
    let Some(mut path) = rfd::FileDialog::new().set_parent(&window).set_title("Save merge preset")
        .add_filter("Merge preset", &["json"]).set_file_name("hitsound-merge.json").save_file()
        else { return Ok(false) };
    if path.extension().is_none() {
        path.set_extension("json");
    }
    if !path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("json")) {
        return Err("Choose a .json file for the merge preset.".into());
    }
    match std::fs::read(&path) {
        Ok(expected) => {
            let backups = BackupPolicy::new(dawsu_application::data_folder().join("backups"), 5);
            match safe_write::replace(&path, &expected, &bytes, &backups).map_err(|error| error.to_string())? {
                WriteOutcome::Saved { .. } => {}
                WriteOutcome::ChangedOnDisk => return Err("The preset file changed while saving. Choose a new file or try again.".into()),
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            safe_write::create(&path, &bytes).map_err(|error| error.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(true)
}

#[tauri::command]
fn audio_outputs(state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, Request::AudioOutputs)
}

#[tauri::command]
fn editor_shortcuts(state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, Request::Shortcuts)
}

#[tauri::command]
fn import_key_file(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<bool, String> {
    let selected = rfd::FileDialog::new().set_parent(&window)
        .set_title("Import shortcuts").add_filter("dawsu! shortcuts", &["dawsukeys"]).pick_file();
    let Some(path) = selected else { return Ok(false); };
    ask(&state, |tx| Request::KeyFile(path, true, tx))??;
    Ok(true)
}

#[tauri::command]
fn export_key_file(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<bool, String> {
    let selected = rfd::FileDialog::new().set_parent(&window)
        .set_title("Export shortcuts").add_filter("dawsu! shortcuts", &["dawsukeys"])
        .set_file_name("shortcuts.dawsukeys").save_file();
    let Some(mut path) = selected else { return Ok(false); };
    if path.extension().is_none() {
        path.set_extension("dawsukeys");
    }
    ask(&state, |tx| Request::KeyFile(path, false, tx))??;
    Ok(true)
}

#[tauri::command]
fn pick_map(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<Value, String> {
    let selected = songs_dialog(&window, &state)?
        .add_filter("osu! difficulty or mapset", &["osu", "osz"])
        .pick_file();
    ask(&state, |tx| {
        Request::View(true, selected.map(|path| Action::Open { path }), tx)
    })
}

#[tauri::command]
fn pick_sample(window: tauri::WebviewWindow, state: State<'_, Editor>, target: SampleTarget) -> Result<Value, String> {
    let selected = rfd::FileDialog::new().set_parent(&window)
        .set_title("Choose replacement sample")
        .add_filter("Audio sample", &["wav", "ogg", "mp3"])
        .pick_file();
    ask(&state, |tx| Request::View(true, selected.map(|path| Action::ReplaceSample { target, path, mapset: false }), tx))
}

fn songs_dialog(window: &tauri::WebviewWindow, state: &Editor) -> Result<rfd::FileDialog, String> {
    let dialog = rfd::FileDialog::new().set_parent(window);
    Ok(match ask(state, Request::SongsDirectory)? {
        Some(folder) => dialog.set_directory(folder),
        None => dialog,
    })
}

#[tauri::command]
fn pick_songs_folder(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<Value>, String> {
    let Some(folder) = songs_dialog(&window, &state)?.set_title("Choose Songs folder").pick_folder() else {
        return Ok(None);
    };
    ask(&state, |tx| Request::SetSongsDirectory(folder, tx))?.map(Some)
}

#[tauri::command]
fn pick_hitsound_library(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<Value>, String> {
    let mut dialog = rfd::FileDialog::new().set_parent(&window).set_title("Choose hitsound library folder");
    if let Some(folder) = ask(&state, Request::LibraryDirectory)? {
        dialog = dialog.set_directory(folder);
    }
    let Some(folder) = dialog.pick_folder() else { return Ok(None) };
    ask(&state, |tx| Request::SetLibraryDirectory(folder, tx))?.map(Some)
}

#[tauri::command]
fn list_library(location: library::Location, state: State<'_, Editor>) -> Result<Vec<(String, bool)>, String> {
    ask(&state, |tx| Request::LibraryEntries(location, tx))?
}

#[tauri::command]
fn open_editor_folder(folder: folders::Folder, state: State<'_, Editor>) -> Result<(), String> {
    let path = ask(&state, |tx| Request::Folder(folder, tx))??;
    folder.open(&path)
}

#[tauri::command]
fn close_editor(
    app: tauri::AppHandle,
    state: State<'_, Editor>,
    discard: bool,
) -> Result<(), String> {
    ask(&state, |tx| Request::Close(discard, tx))??;
    app.exit(0);
    Ok(())
}

fn main() {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    tauri::Builder::default()
        .manage(editor_thread(initial))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let editor = window.state::<Editor>();
                if let Err(error) = ask(&editor, |tx| Request::Close(false, tx)).and_then(|r| r) {
                    api.prevent_close();
                    let _ = window.emit("editor-close-requested", error);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            editor_view,
            editor_help,
            editor_export_settings,
            pick_merge_preset,
            save_merge_preset,
            editor_shortcuts,
            audio_outputs,
            import_key_file,
            export_key_file,
            pick_map,
            pick_sample,
            pick_songs_folder,
            pick_hitsound_library,
            list_library,
            open_editor_folder,
            close_editor
        ])
        .run(tauri::generate_context!())
        .expect("start the dawsu! Tauri editor");
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod preset_close_test;
