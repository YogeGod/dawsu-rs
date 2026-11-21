#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod drum_view;
mod drum_window;
mod folders;
mod home;
mod keyboard;
mod library;
mod selection;

use dawsu_application::{Application, OutputBackend, ExportOptions, ExportSettings, PlatformRequest, EditorTimeSignature, LaneLayout, PointerPhase, RangeOperation, TimelineEdit, VolumeScope, lane_name};
use dawsu_editor::{
    keys::{Key, KeyCombination, Mods},
    tempo::SnapMode,
    tools::{PointerButton, PointerInput, PointerSurface, ToolKind, ToolPreview, ToolTarget},
    volume_ramp::{RampShape, ramp_at},
};
use dawsu_foundation::{CustomIndex, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_hitsounding::{
    LaneSource, Place,
    copy::{CopyMode, CopyOptions},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};
use tauri::{Emitter, Manager, State};

#[cfg(feature = "recovery-smoke")]
const RECOVERY_SMOKE_DATA: &str = "DAWSU_RECOVERY_SMOKE_DATA";

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

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WhistleSetJson {
    Auto,
    Normal,
    Soft,
    Drum,
}

impl WhistleSetJson {
    fn sample_set(self) -> Option<SampleSet> {
        match self {
            Self::Auto => None,
            Self::Normal => Some(SampleSet::Normal),
            Self::Soft => Some(SampleSet::Soft),
            Self::Drum => Some(SampleSet::Drum),
        }
    }
}

/// Tauri JSON adapter for the application's existing Copy options.
#[derive(Deserialize)]
#[serde(remote = "CopyOptions")]
struct CopyOptionsJson {
    #[serde(with = "CopyModeJson")]
    mode: CopyMode,
    leniency: f64,
    #[serde(default = "default_copy_start_index")]
    start_index: u32,
    #[serde(default)]
    never_silent_hitnormals: bool,
    copy_hitsounds: bool,
    copy_body_hitsounds: bool,
    #[serde(default)]
    copy_to_slider_ticks: bool,
    #[serde(default)]
    copy_to_slider_slides: bool,
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

fn default_copy_start_index() -> u32 {
    CopyOptions::default().start_index
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
#[serde(rename_all = "snake_case")]
enum RulerGesture {
    Left,
    Right,
    Draw,
    Move,
    Toggle,
}

impl RulerGesture {
    fn apply(self, app: &mut Application, path: &Path, from: f64, to: f64) {
        if app.busy || !from.is_finite() || !to.is_finite() {
            return;
        }
        let Some(document) = app.document().filter(|document| document.path == path) else {
            return;
        };
        let duration = app.duration_ms();
        let snap = |ms: f64| {
            document.composer.snap(SongTime::from_ms(ms.clamp(0.0, duration)))
                .ms().clamp(0.0, duration)
        };
        let (mut left, mut right, mut enabled) = (app.locator_left, app.locator_right, app.cycle);
        // A drag stops on the other locator; a Ctrl- or Alt-click (from == to) sets it where it lands.
        let dragged = from != to;
        match self {
            Self::Left => left = if dragged && left <= right { snap(to).min(right) } else { snap(to) },
            Self::Right => right = if dragged && left <= right { snap(to).max(left) } else { snap(to) },
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
            Self::Toggle => {
                if left == right || to < left.min(right) || to > left.max(right) {
                    return;
                }
                enabled = !enabled;
            }
        }
        (app.locator_left, app.locator_right, app.cycle) = (left, right, enabled);
        app.cycle_changed();
    }
}

#[derive(Deserialize)]
struct KeyLaneView {
    path: PathBuf,
    rows: Vec<usize>,
    span: f64,
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
    PlaceSoundSet {
        map: PathBuf,
        path: PathBuf,
    },
    PlaceLibrarySoundSet {
        map: PathBuf,
        id: String,
    },
    /// The open-time question about placements on an older version of a set.
    SoundSetPrompt {
        set_id: String,
        update: bool,
    },
    /// The Sound set placement dialog: `op` is "gain", "reset", "unlink" or "update".
    SoundSetPlacement {
        map: PathBuf,
        id: u64,
        op: String,
        gain: Option<i32>,
    },
    Open {
        path: PathBuf,
    },
    RestoreRecovery {
        id: String,
    },
    #[cfg(feature = "recovery-smoke")]
    RecoverySmokePanic,
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
        lane_view: Option<KeyLaneView>,
    },
    Save,
    ReviewSave,
    AnswerSaveReview { apply: bool },
    BookmarkAdd { path: PathBuf, time: f64, label: String },
    BookmarkEdit { path: PathBuf, time: f64, label: Option<String> },
    BookmarkJump { time: f64 },
    WaveformObject { path: PathBuf, object: usize, time: f64, node: bool, preview: bool },
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
    ShowLane { target: SampleTarget },
    DeleteLane { target: SampleTarget, revision: u64 },
    LaneLayout {
        path: Option<PathBuf>,
        #[serde(default)]
        persist: bool,
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
    LaneSelectionMix {
        target: SampleTarget,
        operation: String,
        lanes: Vec<String>,
    },
    GroupMix {
        set: String,
        operation: String,
    },
    Volume {
        value: i32,
        relative: bool,
    },
    VolumeScope {
        path: PathBuf,
        lanes: Option<Vec<String>>,
    },
    InsertVolume {
        value: i32,
    },
    SliderBody {
        path: PathBuf,
        index: usize,
        #[serde(default, deserialize_with = "deserialize_sample_set")]
        set: Option<SampleSet>,
        whistle_set: Option<WhistleSetJson>,
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
    SongSolo {
        on: bool,
    },
    OutputSettings {
        backend: OutputBackend,
        device_id: Option<String>,
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
    /// Shift-click on a ruler flag: jump the playhead to that locator.
    Locator {
        end: String,
    },
    RulerLocators {
        path: PathBuf,
        gesture: RulerGesture,
        from: f64,
        to: f64,
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
            Self::RestoreRecovery { id } => {
                if let Err(error) = app.restore_recovery(&id) {
                    app.error = Some(error);
                }
            }
            #[cfg(feature = "recovery-smoke")]
            Self::RecoverySmokePanic => {
                if std::env::var_os(RECOVERY_SMOKE_DATA).is_some_and(|path| !path.is_empty()) {
                    panic!("Controlled crash recovery smoke test");
                }
                app.error = Some("The smoke test needs an isolated app data folder.".into());
            }
            Self::BrowserStyle { classic } => {
                app.preferences.classic_browser = classic;
                if classic && app.preferences.show_browser {
                    app.preferences.show_inspector = true;
                }
                app.save_preferences();
            }
            Self::PlaceSoundSet { map, path } => {
                if let Err(error) = app.place_sound_set_file(&map, &path) {
                    app.error = Some(error);
                }
            }
            Self::PlaceLibrarySoundSet { map, id } => {
                if let Err(error) = app.place_library_sound_set(&map, &id) {
                    app.error = Some(error);
                }
            }
            Self::SoundSetPrompt { set_id, update } => {
                if let Err(error) = app.answer_sound_set_prompt(&set_id, update) {
                    app.error = Some(error);
                }
            }
            Self::SoundSetPlacement { map, id, op, gain } => {
                if app.document().is_none_or(|d| d.path != map) {
                    return;
                }
                let result = match (op.as_str(), gain) {
                    ("gain", Some(gain)) => app.set_sound_set_placement_gain(id, gain),
                    ("reset", _) => app.reset_sound_set_placement(id),
                    ("unlink", _) => app.unlink_sound_set_placement(id),
                    ("update", _) => app.sound_set_placement(id).and_then(|_| {
                        let set_id = app.document().and_then(|d| d.composer.hitsounds().placements().iter().find(|p| p.id == id).map(|p| p.set_id.clone()));
                        set_id.map_or(Ok(()), |set_id| app.update_sound_set(&set_id))
                    }),
                    _ => Err("Unknown sound set action.".into()),
                };
                if let Err(error) = result {
                    app.error = Some(error);
                }
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
                lane_view,
            } => {
                if let Some(chord) = keyboard::chord(&key, &code, ctrl, shift, alt) {
                    if app.keymap.command_for(chord.combination()) == Some("play") {
                        app.play_pause_from(playback_start.position(app));
                    } else if let Some(view) = lane_view {
                        app.key_with_lane_view(chord, view.path, view.rows, view.span);
                    } else {
                        app.key(chord);
                    }
                }
            }
            Self::Save => app.save(),
            Self::ReviewSave => app.review_save(),
            Self::AnswerSaveReview { apply } => app.answer_save_review(apply),
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
            Self::Stop => app.command("stop"),
            Self::Start => app.command("start"),
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
            Self::WaveformObject { path, object, time, node, preview } => {
                app.select_waveform_object(&path, object, time, node, preview);
            }
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
                let (phase, click_count) = match phase.as_str() {
                    "down" => (PointerPhase::Down, 1),
                    "double_click" => (PointerPhase::Down, 2),
                    "move" => (PointerPhase::Move, 1),
                    "up" => (PointerPhase::Up, 1),
                    _ => (PointerPhase::Cancel, 1),
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
                        click_count,
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
                        "activate" if !app.busy => app.active_lane = row,
                        "activate" => {},
                        _ => {
                            app.active_lane = row;
                            app.audition(&lane);
                        }
                    }
                }
            }
            Self::ShowLane { target } => {
                if let Err(error) = lane_by_key(app, &target.lane)
                    .or_else(|_| {
                        app.scan_lane_add_candidates(&target.map)?.into_iter()
                            .find(|lane| scene_lane_key(lane) == target.lane)
                            .ok_or_else(|| "The selected sample is unavailable.".into())
                    })
                    .and_then(|lane| app.show_lane(&target.map, &lane))
                {
                    app.error = Some(error);
                }
            }
            Self::DeleteLane { target, revision } => {
                if let Err(error) = lane_by_key(app, &target.lane)
                    .and_then(|lane| app.delete_lane(&target.map, &lane, revision))
                {
                    app.error = Some(error);
                }
            }
            Self::LaneSelectionMix { target, operation, lanes } => {
                lane_selection_mix(app, &target, &operation, lanes);
            }
            Self::GroupMix { set, operation } => group_mix(app, &set, &operation),
            Self::LaneLayout { path, persist, value } => {
                if path.as_ref().is_some_and(|path| app.document().is_none_or(|doc| &doc.path != path)) {
                    app.error = Some("The difficulty changed. Edit the lane layout again.".into());
                    return;
                }
                if let Err(error) = app.set_lane_layout(value).and_then(|()| if persist { app.save_session() } else { Ok(()) }) {
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
            Self::VolumeScope { path, lanes } => {
                if let Err(error) = set_volume_lane_scope(app, &path, lanes) {
                    app.error = Some(error);
                }
            }
            Self::InsertVolume { value } => app.set_insert_volume(value),
            Self::SliderBody { path, index, set, whistle_set, whistle, volume } => {
                if app.document().is_some_and(|doc| doc.path == path)
                    && let Err(error) = app.edit_slider_body(index, set, whistle_set.map(WhistleSetJson::sample_set), whistle, volume)
                {
                    app.error = Some(error);
                }
            }
            Self::Gains { song, main } => {
                app.preferences.song_gain = song.clamp(0.0, 1.0);
                app.preferences.master_gain = main.clamp(0.2, 1.0);
                app.gains_changed();
            }
            Self::OutputSettings { backend, device_id, ms } => {
                if let Err(error) = app.set_output_settings(backend, device_id, ms) {
                    app.error = Some(error);
                }
            }
            Self::SongMuted { on } => {
                app.preferences.song_muted = on;
                app.gains_changed();
            }
            Self::SongSolo { on } => app.set_song_solo(on),
            Self::Audition { on } => app.preferences.audition = on,
            Self::Offset { ms } => {
                app.song_offset = ms.clamp(-60_000.0, 60_000.0);
                app.cycle_changed();
            }
            Self::Speed { value } => app.set_speed(value),
            Self::HsMode { on } => app.set_hs_mode(on),
            Self::Protect { on } => app.set_normal_protection(on),
            Self::Locator { end } => {
                app.seek(if end == "left" { app.locator_left } else { app.locator_right });
            }
            Self::RulerLocators { path, gesture, from, to } => gesture.apply(app, &path, from, to),
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
                app.zoom = zoom.clamp(dawsu_application::MIN_ZOOM, 3.0);
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

fn mix_lanes(app: &mut Application, lanes: Vec<SampleIdentity>, operation: &str, clicked: Option<SampleIdentity>) {
    let (enabled, toggle): (fn(&Application, &SampleIdentity) -> bool, fn(&mut Application, SampleIdentity)) =
        match operation {
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

fn group_mix(app: &mut Application, set: &str, operation: &str) {
    let Some(document) = app.document().filter(|_| !app.busy) else {
        return;
    };
    let lanes = document.composer.lanes.iter().filter(|lane| {
        matches!(lane, SampleIdentity::Standard { set: lane_set, .. } if sample_set_name(*lane_set) == set)
    }).cloned().collect();
    mix_lanes(app, lanes, operation, None);
}

fn lane_selection_mix(app: &mut Application, target: &SampleTarget, operation: &str, lanes: Vec<String>) {
    let Some(document) = app.document().filter(|doc| !app.busy && doc.path == target.map) else {
        return;
    };
    let mut keys: std::collections::HashSet<_> = lanes.into_iter().collect();
    let mut members = Vec::new();
    let mut clicked = None;
    for lane in &document.composer.lanes {
        let key = scene_lane_key(lane);
        if keys.remove(&key) {
            if key == target.lane {
                clicked = Some(lane.clone());
            }
            members.push(lane.clone());
        }
    }
    if let Some(clicked) = clicked.filter(|_| keys.is_empty()) {
        mix_lanes(app, members, operation, Some(clicked));
    }
}

/// The difficulty's lane rows (composer lanes) that get a lane header, in order, and the slots that rows stand in for:
/// an unused slot lane is shown on its source's row. Hidden lanes get no header. `used` says
/// whether a lane has hits. The scene's `laneOrder` and the drum window's lane numbers share it.
fn lane_header_rows<'a>(
    document: &'a dawsu_application::Document,
    used: &dyn Fn(&SampleIdentity) -> bool,
) -> (Vec<usize>, std::collections::HashMap<usize, &'a SampleIdentity>) {
    let c = &document.composer;
    let model = c.hitsounds();
    let row_by_lane: std::collections::HashMap<_, _> = c.lanes.iter().enumerate().map(|(row, lane)| (lane, row)).collect();
    let mut slot_rows = std::collections::HashMap::new();
    let mut backing_slots = std::collections::HashMap::new();
    for (source, slot) in model.lane_slots() {
        if !used(slot) && let (Some(&from), Some(&to)) = (row_by_lane.get(source), row_by_lane.get(slot))
            && let std::collections::hash_map::Entry::Vacant(entry) = slot_rows.entry(to)
        {
            entry.insert(from);
            backing_slots.insert(from, slot);
        }
    }
    let lane_order = (0..c.lanes.len()).filter(|row| !backing_slots.contains_key(row))
        .map(|row| slot_rows.get(&row).copied().unwrap_or(row))
        .filter(|&row| !model.is_lane_hidden(&c.lanes[row]))
        .collect();
    (lane_order, backing_slots)
}

/// Puts each numbered layer directly under its family's main lane, by number. A family is one set
/// and sound; its main lane is the shown lane at custom index 1, else the skin sample (index 0).
/// Without either, its layers stay ordinary lanes where they are. This order is for display only:
/// the scene's `laneOrder` and the saved lane order never take it. app.js's `familyOrder` applies it
/// after the saved order, as `header_numbers` does here.
fn family_order<'a>(rows: Vec<usize>, lane: impl Fn(usize) -> &'a SampleIdentity) -> Vec<usize> {
    let family = |row: usize| match lane(row) {
        SampleIdentity::Standard { set, sound, index } => Some(((*set, *sound), index.get())),
        SampleIdentity::File(_) => None,
    };
    let mut mains = std::collections::HashMap::new();
    for &row in &rows {
        if let Some((key, index @ (0 | 1))) = family(row) && (index == 1 || !mains.contains_key(&key)) {
            mains.insert(key, row);
        }
    }
    let mut layers: std::collections::HashMap<usize, Vec<(u32, usize)>> = std::collections::HashMap::new();
    for &row in &rows {
        if let Some((key, index)) = family(row) && let Some(&main) = mains.get(&key) && main != row {
            layers.entry(main).or_default().push((index, row));
        }
    }
    let children: std::collections::HashSet<usize> = layers.values().flatten().map(|&(_, row)| row).collect();
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

/// A lane's key in the scene: the text the front end holds for a lane and sends back to name it.
fn scene_lane_key(lane: &SampleIdentity) -> String {
    format!("{lane:?}")
}

fn lane_by_key(app: &Application, key: &str) -> Result<SampleIdentity, String> {
    app.document()
        .and_then(|document| document.composer.lanes.iter().find(|lane| scene_lane_key(lane) == key))
        .cloned()
        .ok_or_else(|| "The lane changed. Choose a lane again.".into())
}

fn set_volume_lane_scope(app: &mut Application, path: &Path, lanes: Option<Vec<String>>) -> Result<(), String> {
    let document = app.document().filter(|document| document.path == path)
        .ok_or("The open difficulty changed. Select the volume lanes again.")?;
    let model = document.composer.hitsounds();
    let scope = lanes.map(|keys| keys.into_iter().map(|key| {
        let lane = lane_by_key(app, &key)?;
        Ok(model.lane_slot(&lane).cloned().unwrap_or(lane))
    }).collect::<Result<BTreeSet<_>, String>>()).transpose()?;
    app.set_selection_lane_scope(scope)
}

fn lane_add_choices(app: &Application, map: &Path) -> Result<Value, String> {
    let candidates = app.scan_lane_add_candidates(map)?;
    let model = app.document().ok_or("Open a difficulty first.")?.composer.hitsounds();
    Ok(json!(candidates.iter().map(|lane| json!({
        "key": scene_lane_key(lane),
        "name": lane_name(model.lane_slot(lane).unwrap_or(lane)),
    })).collect::<Vec<_>>()))
}

fn assign_sample(app: &mut Application, target: SampleTarget, path: PathBuf, mapset: bool) -> Result<(), String> {
    let document = app.document().ok_or("Open a difficulty first.")?;
    let lane = document.composer.lanes.get(app.active_lane).ok_or("Choose an active lane.")?;
    if document.path != target.map || scene_lane_key(lane) != target.lane {
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


fn gameplay_overlay(document: &dawsu_application::Document) -> Value {
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
            Place::NoObject => {}
        }
    }
    json!({ "points": points, "spans": spans })
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
                    LaneSource::File(name) => Some(name),
                    LaneSource::Skin => None,
                });
                let hit_count = hit_counts.get(lane).copied().unwrap_or(0);
                let read_only = matches!(slot, SampleIdentity::Standard { sound: Sound::Slider(_), .. });
                let protected = c.protect_normals && hit_count > 0
                    && matches!(slot, SampleIdentity::Standard { sound: Sound::Hit(dawsu_foundation::Hitsound::Normal), .. });
                json!({
                    "family": family, "index": index, "group": group, "key": scene_lane_key(slot),
                    "sourceKey": scene_lane_key(lane), "sample": sample,
                    "colourKey": dawsu_application::lane_layout_key(slot),
                    "name": lane_name(slot), "muted": app.is_muted(lane), "solo": app.is_solo(lane),
                    "canAddLayer": index.is_some(),
                    "canReplace": !matches!(lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. }),
                    "hitCount": hit_count, "canDelete": !read_only && !protected,
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
                        let muted = listen_muted.contains(&(i, lane));
                        let gain = moment.hit_volume(lane);
                        if gain != moment.volume() {
                            json!([row, selected, muted, gain])
                        } else if muted {
                            json!([row, selected, true])
                        } else {
                            json!([row, selected])
                        }
                    })).collect::<Vec<_>>();
                json!([moment.time().ms(), moment.reference_volume(), moment.place() != Place::NoObject, hits])
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
                    "whistleSet": body.whistle_set().map_or("auto", sample_set_name),
                    "volume": body.volume(),
                });
            }
            json!({
                "title": d.title, "path": d.path, "difficulties": d.difficulties,
                "lanes": lanes, "laneOrder": lane_order, "moments": moments, "sections": sections, "waveform": waveform,
                "gameplay": gameplay_overlay(d),
                "laneRevision": c.history.version(),
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
                "placement": c.selected_placement(),
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
        "returnOnStop": app.preferences.return_on_stop, "songMuted": app.preferences.song_muted,
        "audition": app.preferences.audition, "offset": app.song_offset,
        "left": app.locator_left, "right": app.locator_right, "cycle": app.cycle, "follow": app.preferences.follow,
        "tool": format!("{:?}", app.tool()), "pendingOpen": app.pending_open.is_some(),
        "keyPreset": app.preferences.key_preset,
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
    result["saveReview"] = json!(app.save_review_files());
    result["soundSetPrompts"] = json!(document.map(|_| app.sound_set_prompts()).unwrap_or_default().iter().map(|p| json!({
        "setId": p.set_id, "name": p.name, "placements": p.placements, "editedKeepFiles": p.edited_keep_files,
        "open": app.sound_set_prompt_open(),
    })).collect::<Vec<_>>());
    result["songSolo"] = json!(app.song_solo());
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
    if include_scene && document.is_none() {
        result["recovery"] = match app.recovery_offers() {
            Ok(offers) => json!({ "offers": offers }),
            Err(error) => json!({ "error": format!("Could not list recovery copies. {error}") }),
        };
    }
    result
}

enum Request {
    View(bool, Option<Action>, mpsc::Sender<Value>),
    Help(mpsc::Sender<Result<Value, String>>),
    AudioOutputs(Option<OutputBackend>, mpsc::Sender<Value>),
    Shortcuts(mpsc::Sender<Value>),
    ExportSettings(mpsc::Sender<Option<(PathBuf, ExportSettings)>>),
    KeyFile(PathBuf, bool, mpsc::Sender<Result<(), String>>),
    SongsDirectory(mpsc::Sender<Option<PathBuf>>),
    SetSongsDirectory(PathBuf, mpsc::Sender<Result<Value, String>>),
    LibraryDirectory(mpsc::Sender<Option<PathBuf>>),
    SetLibraryDirectory(PathBuf, mpsc::Sender<Result<Value, String>>),
    LibraryEntries(library::Location, mpsc::Sender<Result<Vec<(String, bool)>, String>>),
    SoundSetLibrary(mpsc::Sender<Result<Value, String>>),
    SoundSetPlacement(PathBuf, u64, mpsc::Sender<Result<Value, String>>),
    SaveSoundSet(PathBuf, String, mpsc::Sender<Result<Value, String>>),
    LaneAddCandidates(PathBuf, mpsc::Sender<Result<Value, String>>),
    DrumView(PathBuf, Option<PathBuf>, bool, mpsc::Sender<Result<Value, String>>),
    Folder(folders::Folder, mpsc::Sender<Result<PathBuf, String>>),
    Close(bool, mpsc::Sender<Result<(), String>>),
    // The Export window's own requests. Each always answers, with an error after a fault.
    ExportView(mpsc::Sender<Result<Value, String>>),
    /// The difficulty the window showed, and its options.
    ExportPreview(PathBuf, ExportOptions, mpsc::Sender<Result<u64, String>>),
    ExportRun(u64, mpsc::Sender<Result<u64, String>>),
    /// The Export window closed: its preview stops and is forgotten. Needs no answer.
    ExportClose,
    /// The drum detection window's requests, answered with its view.
    Drum(drum_window::DrumRequest, mpsc::Sender<Result<Value, String>>),
    /// The drum detection window closed: its run, result and Listen go. Needs no answer.
    DrumClose,
}
struct Editor(mpsc::Sender<Request>);

fn guard<T>(
    app: &mut Application,
    fault: &mut Option<String>,
    operation: impl FnOnce(&mut Application) -> T,
) -> Result<T, String> {
    if let Some(message) = fault {
        return Err(message.clone());
    }
    // A faulted worker only answers status and close requests. Recovery reads its
    // owned checkpoint without using the editor state touched by the panic.
    catch_unwind(AssertUnwindSafe(|| operation(app))).map_err(|_| {
        let message = match app.write_recovery_copy() {
            Ok(Some(copy)) => format!(
                "Saved a recovery copy for {} [{}]. Reopen dawsu! to restore it.",
                copy.mapset, copy.difficulty
            ),
            Ok(None) => "No difficulty was open when the editor stopped.".into(),
            Err(error) => format!("Could not save a recovery copy. {error}"),
        };
        *fault = Some(message.clone());
        message
    })
}

/// What the Export view reads from disk or from every hit, kept until the map changes:
/// polls come every 100-250 ms and must not re-read the receipt or walk every hit.
#[derive(Default)]
struct ExportCache {
    /// Path, load or Save, and the mapset's difficulties.
    targets_for: Option<(PathBuf, u64, Vec<PathBuf>)>,
    targets: Vec<PathBuf>,
    /// Path, load or Save, history version and lane count.
    counts_for: Option<(PathBuf, u64, u64, usize)>,
    /// Hits per lane, in lane order.
    counts: Vec<usize>,
    /// How often either was read, for tests.
    reads: usize,
}

fn export_snapshot(app: &Application, cache: &mut ExportCache) -> Value {
    let status = app.export_status();
    let document = app.document();
    let generation = app.document_generation();
    let source = document.map(|d| {
        let key = (d.path.clone(), generation, d.composer.history.version(), d.composer.lanes.len());
        if cache.counts_for.as_ref() != Some(&key) {
            let mut hit_counts = std::collections::HashMap::new();
            for lane in d.composer.hitsounds().moments().iter().flat_map(|moment| moment.hits()) {
                *hit_counts.entry(lane).or_insert(0_usize) += 1;
            }
            cache.counts = d.composer.lanes.iter().map(|lane| hit_counts.get(lane).copied().unwrap_or(0)).collect();
            cache.counts_for = Some(key);
            cache.reads += 1;
        }
        let lanes = d.composer.lanes.iter().zip(&cache.counts).map(|(lane, count)| json!({
            "name": lane_name(lane),
            "colourKey": dawsu_application::lane_layout_key(lane),
            "hitCount": count,
        })).collect::<Vec<_>>();
        json!({
            "path": d.path, "title": d.title,
            "difficulty": d.file.value("Metadata", "Version"),
            "version": d.composer.history.version(),
            "hits": cache.counts.iter().sum::<usize>(),
            "lanes": lanes,
            "laneColours": d.lane_layout().map(|layout| json!(layout.lane_colours)).unwrap_or_else(|| json!({})),
        })
    });
    let targets = match document {
        Some(d) => {
            let fresh = cache.targets_for.as_ref()
                .is_some_and(|(path, read, list)| *path == d.path && *read == generation && *list == d.difficulties);
            if !fresh {
                cache.targets = app.export_targets();
                cache.targets_for = Some((d.path.clone(), generation, d.difficulties.clone()));
                cache.reads += 1;
            }
            cache.targets.clone()
        }
        None => Vec::new(),
    };
    json!({
        "source": source,
        "targets": targets,
        "backups": app.backups_folder(),
        "busy": app.busy,
        "requested": status.requested,
        "previewing": status.running || status.waiting,
        "progress": status.progress.map(|(done, total)| json!({ "kind": "preview", "done": done, "total": total })),
        "choices": document.and_then(|d| d.export_choices()).map(|choices| json!({
            "leftOut": choices.left_out,
            "matchWithin": choices.match_within,
            "copyVolumes": choices.copy_volumes,
            "base": choices.base,
            "sliderAudio": choices.slider_audio,
        })),
        "error": status.error,
        "writing": status.writing,
        "writeId": status.write_id,
        "written": status.written,
        "writeError": status.write_error,
        "problems": status.problems.iter().map(|problem| json!({
            "target": problem.target, "timeline": problem.timeline,
            "time": problem.time, "message": problem.message,
        })).collect::<Vec<_>>(),
        "preview": status.preview.map(|preview| json!({
            "id": preview.id,
            "origin": "export",
            "stale": status.stale,
            "staleReason": status.stale_reason.map(|reason| match reason {
                dawsu_application::Stale::Map => "map",
                dawsu_application::Stale::MergeSettings => "mergeSettings",
            }),
            "source": preview.plan.source,
            "version": preview.plan.version,
            "label": preview.plan.label,
            "details": preview.plan.details,
            "files": preview.plan.files.iter().map(|file| &file.path).collect::<Vec<_>>(),
            // The base hitsound difficulty, and whether it replaces the one Generate made before.
            "base": preview.plan.generated_file().and_then(|path| preview.plan.files.iter().find(|file| file.path == path))
                .map(|file| json!({ "path": file.path, "replaces": file.expected.is_some() })),
            "mixes": preview.plan.mix_paths().collect::<Vec<_>>(),
            "targets": preview.plan.targets.iter().map(|counts| json!({
                "path": counts.path,
                "matched": counts.matched,
                "unmatched": counts.unmatched,
                "changed": counts.changed,
                "collisions": counts.collisions,
                "keptFromSilent": counts.kept_from_silent,
                "muted": counts.muted,
            })).collect::<Vec<_>>(),
        })),
    })
}

/// Answers the Export window. Never touches main's one-shot view state.
fn answer_export(app: &mut Application, fault: &mut Option<String>, cache: &mut ExportCache, request: Request) {
    match request {
        Request::ExportView(response) => {
            let _ = response.send(guard(app, fault, |app| export_snapshot(app, cache)));
        }
        Request::ExportPreview(source, options, response) => {
            let result = guard(app, fault, |app| {
                // The window chose these options for the difficulty it showed, not one opened since.
                if app.document().is_none_or(|d| d.path != source) {
                    return Err("Another difficulty was opened. Preview again.".to_owned());
                }
                app.request_export_preview(options)
            }).and_then(|result| result);
            let _ = response.send(result);
        }
        Request::ExportRun(preview_id, response) => {
            let result = guard(app, fault, |app| app.run_export(preview_id)).and_then(|result| result);
            let _ = response.send(result);
        }
        _ => unreachable!("only Export requests reach answer_export"),
    }
}

fn editor_thread(initial: Option<PathBuf>) -> Editor {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let data_root = dawsu_application::data_folder();
        #[cfg(feature = "recovery-smoke")]
        let data_root = std::env::var_os(RECOVERY_SMOKE_DATA)
            .filter(|path| !path.is_empty())
            .map_or(data_root, PathBuf::from);
        let mut app = Application::with_data_root(data_root);
        let mut fault = None;
        let mut export_cache = ExportCache::default();
        let mut playback_start = PlaybackStart::default();
        let mut home = home::Route::default();
        let mut drums = drum_view::DrumView::default();
        let mut pending_initial_open = initial.filter(|path| map_path(path));
        if !app.first_launch && let Some(path) = pending_initial_open.take() {
            let _ = guard(&mut app, &mut fault, |app| app.request_open(path));
        }
        loop {
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(Request::View(scene, action, response)) => {
                    let view = guard(&mut app, &mut fault, |app| {
                        match action {
                            Some(Action::Open { path }) if app.first_launch => {
                                if map_path(&path) {
                                    pending_initial_open = Some(path);
                                }
                            }
                            Some(action) => {
                                if let Some(action) = home.handle(action, app) {
                                    action.run_with_start(app, &mut playback_start);
                                }
                            }
                            None => {}
                        }
                        if !app.first_launch && let Some(path) = pending_initial_open.take() {
                            app.request_open(path);
                        }
                        app.poll();
                        home.settle(app);
                        let mut view = snapshot(app, scene && !home.visible());
                        home.add_status(&mut view);
                        drums.decorate(app, &mut view);
                        view["playbackStart"] = json!(playback_start);
                        if let Some(request @ ("filter" | "newDiff" | "openFolder" | "importOsz" | "openHitsoundFolder" | "reviewOutput")) = app.requested_edit() {
                            view["shortcutAction"] = json!(request);
                            view["editDialog"] = Value::Null;
                            app.cancel_timeline_edit();
                        }
                        view["shortcutHelp"] = json!(std::mem::take(&mut app.help_requested));
                        // The Inspector's bookmark pick follows Previous and Next, from a button, menu or key.
                        view["bookmarkStep"] = json!(app.bookmark_step.take());
                        view["shortcutOpen"] = json!(std::mem::take(&mut app.requests).iter()
                            .any(|request| matches!(request, PlatformRequest::OpenMap)));
                        view
                    })
                    .unwrap_or_else(|message| json!({ "fault": message }));
                    let _ = response.send(view);
                }
                // After a fault, a request without an error channel gets no answer.
                Ok(Request::Shortcuts(response)) => {
                    if let Ok(value) = guard(&mut app, &mut fault, |app| json!({
                        "preset": app.preferences.key_preset, "entries": app.shortcuts(),
                    })) {
                        let _ = response.send(value);
                    }
                }
                Ok(Request::KeyFile(path, import, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        if import { app.import_shortcuts(&path) } else { app.export_shortcuts(&path) }
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::AudioOutputs(requested, response)) => {
                    if let Ok(value) = guard(&mut app, &mut fault, |app| audio_output_settings(app, requested)) {
                        let _ = response.send(value);
                    }
                }
                Ok(Request::Help(response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        json!(
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
                        )
                    });
                    let _ = response.send(result);
                }
                Ok(Request::ExportSettings(response)) => {
                    if let Ok(value) = guard(&mut app, &mut fault, |app| {
                        app.document().map(|document| {
                            (document.path.clone(), document.export_settings().cloned().unwrap_or_default())
                        })
                    }) {
                        let _ = response.send(value);
                    }
                }
                Ok(Request::SongsDirectory(response)) => {
                    if let Ok(value) = guard(&mut app, &mut fault, |app| {
                        let folder = &app.preferences.songs_folder;
                        folder.is_dir().then(|| folder.clone())
                    }) {
                        let _ = response.send(value);
                    }
                }
                Ok(Request::SetSongsDirectory(folder, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        if app.first_launch {
                            Err("Choose a keyboard preset first.".into())
                        } else if !folder.is_dir() {
                            Err("The selected Songs folder is unavailable.".into())
                        } else {
                            app.preferences.songs_folder = folder;
                            app.save_preferences();
                            let mut view = snapshot(app, false);
                            home.add_status(&mut view);
                            drums.decorate(app, &mut view);
                            view["playbackStart"] = json!(playback_start);
                            Ok(view)
                        }
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::LibraryDirectory(response)) => {
                    if let Ok(value) = guard(&mut app, &mut fault, |app| {
                        app.preferences.hitsound_library.as_ref().filter(|path| path.is_dir()).cloned()
                    }) {
                        let _ = response.send(value);
                    }
                }
                Ok(Request::SetLibraryDirectory(folder, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        if app.first_launch {
                            return Err("Choose a keyboard preset first.".into());
                        }
                        let folder = folder.canonicalize().map_err(|e| format!("Opening hitsound library: {e}"))?;
                        if !folder.is_dir() {
                            return Err("The selected hitsound library folder is unavailable.".into());
                        }
                        app.preferences.hitsound_library = Some(folder);
                        app.save_preferences();
                        let mut view = snapshot(app, false);
                        home.add_status(&mut view);
                        drums.decorate(app, &mut view);
                        view["playbackStart"] = json!(playback_start);
                        Ok(view)
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::LibraryEntries(location, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        location.entries(app.preferences.hitsound_library.as_deref())
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::SoundSetLibrary(response)) => {
                    let result = guard(&mut app, &mut fault, |app| sound_set_library_view(app));
                    let _ = response.send(result);
                }
                Ok(Request::SoundSetPlacement(map, id, response)) => {
                    let result = guard(&mut app, &mut fault, |app| sound_set_placement_view(app, &map, id))
                        .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::SaveSoundSet(map, name, response)) => {
                    let result = guard(&mut app, &mut fault, |app| match app.save_sound_set_from_selection(&map, &name) {
                        Ok(()) => {
                            let mut view = snapshot(app, !home.visible());
                            home.add_status(&mut view);
                            drums.decorate(app, &mut view);
                            view["playbackStart"] = json!(playback_start);
                            Ok(view)
                        }
                        Err(dawsu_application::SaveSetError::NameTaken(message)) => Ok(json!({ "nameTaken": message })),
                        Err(dawsu_application::SaveSetError::Refused(message)) => Err(message),
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::LaneAddCandidates(map, response)) => {
                    let result = guard(&mut app, &mut fault, |app| lane_add_choices(app, &map))
                        .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::DrumView(map, preset, on, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        match preset {
                            Some(path) => drums.load(app, &map, &path),
                            None => drums.toggle(app, &map, on),
                        }.map(|()| {
                            let mut view = snapshot(app, !home.visible());
                            home.add_status(&mut view);
                            drums.decorate(app, &mut view);
                            view["playbackStart"] = json!(playback_start);
                            view
                        })
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::Folder(folder, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        folder.resolve(
                            app.document().map(|d| d.path.as_path()),
                            app.preferences.hitsound_library.as_deref(),
                            &dawsu_application::data_folder(),
                        )
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::Close(discard, response)) => {
                    let result = if fault.is_some() {
                        Ok(())
                    } else {
                        guard(&mut app, &mut fault, |app| {
                            if app.busy {
                                Err("Wait for the current job to finish.".into())
                            } else if app.is_dirty() && !discard {
                                Err("Save or discard the current difficulty before quitting.".into())
                            } else {
                                app.save_session().map(|()| {
                                    app.cancel_drum_detection();
                                    app.cancel_export_preview();
                                    if !app.first_launch {
                                        app.save_preferences();
                                    }
                                })
                            }
                        })
                        .and_then(|result| result)
                    };
                    let _ = response.send(result);
                }
                Ok(request @ (Request::ExportView(_) | Request::ExportPreview(..) | Request::ExportRun(..))) => {
                    answer_export(&mut app, &mut fault, &mut export_cache, request);
                }
                Ok(Request::ExportClose) => {
                    if fault.is_none() {
                        let _ = guard(&mut app, &mut fault, Application::cancel_export_preview);
                    }
                }
                Ok(Request::Drum(request, response)) => {
                    let result = guard(&mut app, &mut fault, |app| drum_window::answer(app, request)).and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::DrumClose) => {
                    if fault.is_none() {
                        let _ = guard(&mut app, &mut fault, Application::cancel_drum_detection);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if fault.is_none() {
                        let _ = guard(&mut app, &mut fault, |app| {
                            app.poll();
                            home.settle(app);
                        });
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Editor(send)
}

fn ask<T>(state: &Editor, request: impl FnOnce(mpsc::Sender<T>) -> Request) -> Result<T, String> {
    ask_within(&state.0, Duration::from_secs(7), request)
}

fn ask_within<T>(editor: &mpsc::Sender<Request>, wait: Duration, request: impl FnOnce(mpsc::Sender<T>) -> Request) -> Result<T, String> {
    let (send, receive) = mpsc::channel();
    editor
        .send(request(send))
        .map_err(|_| "The editor worker stopped".to_owned())?;
    receive
        .recv_timeout(wait)
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
    ask(&state, Request::Help)?
}

#[tauri::command]
fn editor_export_settings(state: State<'_, Editor>) -> Result<Option<(PathBuf, ExportSettings)>, String> {
    ask(&state, Request::ExportSettings)
}

#[derive(Deserialize)]
struct ExportOptionsJson {
    source: PathBuf,
    targets: Vec<PathBuf>,
    #[serde(with = "CopyOptionsJson")]
    options: CopyOptions,
    /// Also write the base hitsound difficulty (Generate), with or without its slider audio.
    #[serde(default)]
    base: bool,
    #[serde(default, rename = "sliderAudio")]
    slider_audio: bool,
}

/// "Export · <map title> [<difficulty>]", or "Export" with no map open.
fn export_title(view: &Value) -> String {
    match (view["source"]["title"].as_str(), view["source"]["difficulty"].as_str()) {
        (Some(title), Some(difficulty)) if !title.ends_with(&format!("[{difficulty}]")) => {
            format!("Export · {title} [{difficulty}]")
        }
        (Some(title), _) => format!("Export · {title}"),
        _ => "Export".into(),
    }
}

#[tauri::command]
fn export_view(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<Value, String> {
    let view = ask(&state, Request::ExportView)??;
    // The title follows the map the editor has open, also after a switch while Export is open.
    let title = export_title(&view);
    if window.title().ok().as_deref() != Some(title.as_str()) {
        let _ = window.set_title(&title);
    }
    Ok(view)
}

#[tauri::command]
fn export_preview(state: State<'_, Editor>, options: ExportOptionsJson) -> Result<u64, String> {
    let source = options.source;
    let options = ExportOptions {
        targets: options.targets, copy: options.options, base: options.base, slider_audio: options.slider_audio,
    };
    ask(&state, |tx| Request::ExportPreview(source, options, tx))?
}

#[tauri::command]
fn export_run(state: State<'_, Editor>, preview_id: u64) -> Result<u64, String> {
    ask(&state, |tx| Request::ExportRun(preview_id, tx))?
}

/// Opens the Export window beside the editor, or brings it forward when it is open.
/// Async, so building the window never runs on the main thread that drives it.
#[tauri::command]
async fn open_export_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("export") {
        let _ = window.unminimize();
        return window.set_focus().map_err(|error| error.to_string());
    }
    // A plain top-level window, not a child of main, so Show in editor can bring main forward.
    // Its own drop handler is off: a file dropped on Export never reaches main's drop listener.
    tauri::WebviewWindowBuilder::new(&app, "export", tauri::WebviewUrl::App("export.html".into()))
        .title("Export")
        .inner_size(1440.0, 880.0)
        .min_inner_size(1100.0, 720.0)
        .disable_drag_drop_handler()
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Closes the Export window the way its title bar does, so its preview is cancelled too.
#[tauri::command]
fn close_export_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("export") {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

/// Brings the editor window forward.
#[tauri::command]
fn show_in_editor(app: tauri::AppHandle) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("The editor window is closed.")?;
    let _ = main.unminimize();
    main.set_focus().map_err(|error| error.to_string())
}

/// Opens the drum detection window beside the editor, or brings it forward when it is open.
/// Async, so building the window never runs on the main thread that drives it.
#[tauri::command]
async fn drum_window_open(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("drum") {
        let _ = window.unminimize();
        return window.set_focus().map_err(|error| error.to_string());
    }
    let built = tauri::WebviewWindowBuilder::new(&app, "drum", tauri::WebviewUrl::App("drum.html".into()))
        .title("Drum hit detection")
        .inner_size(1200.0, 760.0)
        .min_inner_size(960.0, 600.0)
        .disable_drag_drop_handler()
        .build();
    match (built, app.get_webview_window("drum")) {
        (Ok(_), _) => Ok(()),
        // A second open raced the first one: the window exists now, so bring it forward.
        (Err(_), Some(window)) => window.set_focus().map_err(|error| error.to_string()),
        (Err(error), None) => Err(error.to_string()),
    }
}

/// Closes the drum detection window the way its title bar does, so its run and Listen stop too.
#[tauri::command]
fn close_drum_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("drum") {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

fn ask_drum(state: &Editor, request: drum_window::DrumRequest) -> Result<Value, String> {
    ask(state, |tx| Request::Drum(request, tx))?
}

#[tauri::command]
fn drum_context(state: State<'_, Editor>, known: Option<String>) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Context { known })
}

#[tauri::command]
fn drum_detect_start(state: State<'_, Editor>, map: PathBuf, source: drum_window::SourceJson) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Start { map, source })
}

#[tauri::command]
fn drum_review(state: State<'_, Editor>, map: PathBuf, options: drum_window::OptionsJson) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Review { map, options })
}

#[tauri::command]
fn drum_listen(state: State<'_, Editor>, map: PathBuf, from: f64, ticked: Vec<String>) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Listen { map, from, ticked })
}

#[tauri::command]
fn drum_listen_stop(state: State<'_, Editor>) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::StopListen)
}

#[tauri::command]
fn drum_apply(state: State<'_, Editor>, map: PathBuf, review: String, ticked: Vec<String>) -> Result<Value, String> {
    ask_drum(&state, drum_window::DrumRequest::Apply { map, review, ticked })
}

/// A drum stem file, picked in a dialog over the drum detection window.
#[tauri::command]
fn drum_pick_stem(window: tauri::WebviewWindow) -> Option<PathBuf> {
    rfd::FileDialog::new().set_parent(&window).set_title("Pick a drum stem")
        .add_filter("Audio", &["wav", "ogg", "mp3", "flac"]).pick_file()
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

fn audio_output_settings(app: &Application, requested: Option<OutputBackend>) -> Value {
    let available = OutputBackend::available();
    let preferred = app.preferences.audio_backend;
    let backend = requested.unwrap_or_else(|| {
        if available.contains(&preferred) { preferred } else { OutputBackend::System }
    });
    let (mut devices, error) = match app.output_devices(backend) {
        Ok(devices) => (devices, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    let qualified_id = (preferred == backend).then_some(app.preferences.audio_device_id.as_deref()).flatten();
    let device = if let Some(id) = qualified_id {
        devices.iter().find(|device| device.id == id)
    } else if backend == OutputBackend::System {
        app.preferences.device.as_ref().and_then(|name| devices.iter().find(|device| &device.name == name))
    } else {
        None
    };

    json!({
        "backend": backend,
        "available": available.iter().map(|choice| json!({"id": choice, "label": choice.label()})).collect::<Vec<_>>(),
        "current": app.device_name(),
        "currentBackend": app.active_output_backend().map(OutputBackend::label),
        "devices": devices,
        "device": device.map(|device| &device.id),
        "bufferMs": app.preferences.audio_buffer_ms,
        "effectiveBufferMs": app.effective_output_buffer_ms(),
        "error": error,
    })
}

#[tauri::command]
fn audio_outputs(state: State<'_, Editor>, backend: Option<OutputBackend>) -> Result<Value, String> {
    ask(&state, |response| Request::AudioOutputs(backend, response))
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
fn pick_map(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
    archive_only: Option<bool>,
) -> Result<Value, String> {
    let dialog = songs_dialog(&window, &state)?;
    let selected = if archive_only == Some(true) {
        dialog.set_title("Import an .osz")
            .add_filter("osu! beatmap archive", &["osz"])
            .pick_file()
            .filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("osz")))
    } else {
        dialog.add_filter("osu! difficulty or mapset", &["osu", "osz"]).pick_file()
    };
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

/// The library list for Edit > Place sound set: names, ids and counts only.
fn sound_set_library_view(app: &Application) -> Value {
    let library = match app.sound_set_library() {
        Ok(library) => library,
        Err(error) => return json!({ "configured": true, "error": error, "sets": [], "unreadable": [], "reading": false }),
    };
    json!({
        "configured": library.configured,
        "sets": library.sets.iter().map(|set| json!({
            "id": set.id, "name": set.name, "sounds": set.sounds, "inMap": set.in_map, "duplicate": set.duplicate,
            "missing": set.missing,
        })).collect::<Vec<_>>(),
        "unreadable": library.unreadable,
        "reading": library.reading,
    })
}

#[tauri::command]
fn sound_set_library(state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, Request::SoundSetLibrary)?
}

/// What Edit > Sound set placement shows of one placement. Reads the library, so it is asked
/// for when the dialog opens, never on the poll.
fn sound_set_placement_view(app: &Application, map: &Path, id: u64) -> Result<Value, String> {
    if app.document().is_none_or(|d| d.path != map) {
        return Err("The difficulty changed.".into());
    }
    let view = app.sound_set_placement(id)?;
    Ok(json!({
        "id": view.id, "setId": view.set_id, "name": view.name, "edited": view.edited, "sounds": view.sounds,
        "gain": view.gain, "frozen": view.frozen, "newer": view.newer,
    }))
}

#[tauri::command]
fn sound_set_placement(state: State<'_, Editor>, map: PathBuf, id: u64) -> Result<Value, String> {
    ask(&state, |tx| Request::SoundSetPlacement(map, id, tx))?
}

/// Saving reads every selected sample and the library, so it gets its own longer wait, off the
/// window's thread; a 7 s timeout while the editor still saves would make a retry look like a
/// name clash.
#[tauri::command]
async fn save_sound_set(state: State<'_, Editor>, map: PathBuf, name: String) -> Result<Value, String> {
    let editor = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ask_within(&editor, Duration::from_secs(120), |tx| Request::SaveSoundSet(map, name, tx))
    }).await.map_err(|e| e.to_string())??
}

#[tauri::command]
fn pick_sound_set(window: tauri::WebviewWindow, state: State<'_, Editor>, map: PathBuf) -> Result<Value, String> {
    let selected = rfd::FileDialog::new().set_parent(&window)
        .set_title("Place sound set")
        .add_filter("dawsu! sound set", &["dawsuset"])
        .pick_file();
    ask(&state, |tx| Request::View(true, selected.map(|path| Action::PlaceSoundSet { map, path }), tx))
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
fn lane_add_candidates(map: PathBuf, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::LaneAddCandidates(map, tx))?
}

#[tauri::command]
fn pick_drum_map(
    map: PathBuf,
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<Value>, String> {
    let Some(path) = rfd::FileDialog::new().set_parent(&window).set_title("Open drum map")
        .add_filter("Drum map", &["dawsudrum"]).pick_file() else { return Ok(None) };
    ask(&state, |tx| Request::DrumView(map, Some(path), true, tx))?.map(Some)
}

#[tauri::command]
fn set_drum_view(map: PathBuf, on: bool, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::DrumView(map, None, on, tx))?
}

#[tauri::command]
fn open_editor_folder(folder: folders::Folder, state: State<'_, Editor>) -> Result<(), String> {
    let path = ask(&state, |tx| Request::Folder(folder, tx))??;
    folder.open(&path)
}

#[tauri::command]
fn close_editor(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
    discard: bool,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("Only the editor window can quit dawsu!.".into());
    }
    ask(&state, |tx| Request::Close(discard, tx))??;
    app.exit(0);
    Ok(())
}

/// What a window's close request does to the editor. Only the main window owns the session.
#[derive(Debug, PartialEq)]
enum CloseDecision {
    /// Another window closes; the editor and its session stay as they are.
    Allow,
    /// Main closes after the session was saved; every other window closes with it.
    CloseAll,
    /// Main stays open; the message says why.
    Keep(String),
}

/// The windows that close with main.
fn closed_with_main<'a>(labels: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    labels.into_iter().filter(|label| *label != "main").collect()
}

fn close_requested(label: &str, editor: &Editor) -> CloseDecision {
    // Closing Export or the drum detection window never waits on the editor.
    match label {
        "export" => { let _ = editor.0.send(Request::ExportClose); }
        "drum" => { let _ = editor.0.send(Request::DrumClose); }
        _ => {}
    }
    if label != "main" {
        return CloseDecision::Allow;
    }
    match ask(editor, |tx| Request::Close(false, tx)).and_then(|r| r) {
        Ok(()) => CloseDecision::CloseAll,
        Err(error) => CloseDecision::Keep(error),
    }
}

fn main() {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    tauri::Builder::default()
        .manage(editor_thread(initial))
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                match close_requested(window.label(), &window.state::<Editor>()) {
                    CloseDecision::Allow => {}
                    CloseDecision::CloseAll => {
                        let others = window.app_handle().webview_windows();
                        for label in closed_with_main(others.keys().map(String::as_str)) {
                            let _ = others[label].destroy();
                        }
                    }
                    CloseDecision::Keep(error) => {
                        api.prevent_close();
                        let _ = window.emit_to("main", "editor-close-requested", error);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            editor_view,
            editor_help,
            editor_export_settings,
            export_view,
            export_preview,
            export_run,
            open_export_window,
            close_export_window,
            show_in_editor,
            drum_window_open,
            close_drum_window,
            drum_context,
            drum_detect_start,
            drum_review,
            drum_listen,
            drum_listen_stop,
            drum_apply,
            drum_pick_stem,
            pick_merge_preset,
            save_merge_preset,
            editor_shortcuts,
            audio_outputs,
            import_key_file,
            export_key_file,
            pick_map,
            pick_sample,
            pick_sound_set,
            sound_set_library,
            sound_set_placement,
            save_sound_set,
            pick_songs_folder,
            pick_hitsound_library,
            list_library,
            lane_add_candidates,
            pick_drum_map,
            set_drum_view,
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
