#![cfg_attr(windows, windows_subsystem = "windows")]

mod crash;
mod drum_view;
mod drum_window;
mod folders;
mod home;
mod keyboard;
mod library;
mod osu_link;
mod selection;
mod skin_textures;
mod updater;

use dawsu_application::{Application, OutputBackend, ExportOptions, ExportSettings, MixFormat, PlatformRequest, EditorTimeSignature, LaneLayout, PointerPhase, RangeOperation, TimelineEdit, VolumeScope, lane_name};
use dawsu_editor::{
    keys::{Key, KeyCombination, Mods},
    tempo::SnapMode,
    tools::{PointerButton, PointerInput, PointerSurface, ToolKind, ToolPreview, ToolTarget},
    volume_ramp::{RampShape, ramp_at},
};
use dawsu_foundation::{CustomIndex, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_hitsounding::{
    LaneSource, Place,
    copy::{CopyCollisionPolicy, CopyMode, CopyOptions, FallbackSound},
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

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, serde::Serialize)]
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
#[serde(remote = "CopyCollisionPolicy", rename_all = "snake_case")]
enum CopyCollisionPolicyJson {
    Merge,
    LaterWins,
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
    #[serde(default, with = "CopyCollisionPolicyJson")]
    collision_policy: CopyCollisionPolicy,
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
    #[serde(default)]
    slider_end_mute: dawsu_hitsounding::copy::SliderEndMuteOptions,
    #[serde(default)]
    fallback_sound: Option<FallbackSound>,
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
    Seek,
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
            // A click without a drag moves the playhead to the snapped time and leaves L, R and Cycle.
            Self::Seek => {
                app.seek(snap(to));
                return;
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
    ResetShortcuts,
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
    PatternStamp { count: usize, step_ms: f64, snap_to_objects: bool, quantize_volumes: bool },
    CloneSampleVariant { lane: usize },
    AlternateVariants { from_lane: usize, to_lane: usize },
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
    ReloadFromDisk,
    DismissExternalOsuChange,
    Play,
    PlaybackStart { value: PlaybackStart },
    ReturnOnStop { on: bool },
    CleanRedundantGreenlines { on: bool },
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
        #[serde(default)]
        button: Option<String>,
        shift: bool,
        ctrl: bool,
        alt: bool,
    },
    PaintDrawStroke {
        row: usize,
        start_ms: f64,
        end_ms: f64,
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
        #[serde(default)]
        hitsounds: Option<f32>,
    },
    HitsoundGain {
        gain: f32,
    },
    HitsoundsMuted {
        on: bool,
    },
    HitsoundsSolo {
        on: bool,
    },
    LaneGain {
        row: usize,
        gain: f32,
    },
    SongMuted {
        on: bool,
    },
    SongSolo {
        on: bool,
    },
    ToggleStemMonitor,
    LoadDrumStem {
        path: PathBuf,
    },
    ClearDrumStem,
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
            Self::ResetShortcuts => app.reset_shortcuts(),
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
            Self::PatternStamp {
                count,
                step_ms,
                snap_to_objects,
                quantize_volumes,
            } => {
                app.pattern_stamp(dawsu_application::PatternStampOptions {
                    repeat_count: count,
                    step_ms,
                    snap_to_objects,
                    quantize_volumes,
                    ..Default::default()
                });
            }
            Self::CloneSampleVariant { lane } => {
                if let Err(error) = app.clone_sample_variant(lane) {
                    app.error = Some(error);
                }
            }
            Self::AlternateVariants { from_lane, to_lane } => {
                if let Err(error) = app.alternate_variants(from_lane, to_lane) {
                    app.error = Some(error);
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
            Self::ReloadFromDisk => app.reload_from_disk(),
            Self::DismissExternalOsuChange => app.dismiss_external_osu_change(),
            Self::Play => app.play_pause_from(playback_start.position(app)),
            Self::PlaybackStart { value } => *playback_start = value,
            Self::ReturnOnStop { on } => {
                app.preferences.return_on_stop = on;
                app.save_preferences();
            }
            Self::CleanRedundantGreenlines { on } => {
                app.preferences.clean_redundant_greenlines = on;
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
                button,
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
                        button: if button.as_deref() == Some("right") {
                            PointerButton::Right
                        } else {
                            PointerButton::Left
                        },
                        mods,
                        click_count,
                        x,
                        y,
                    },
                );
            }
            Self::PaintDrawStroke {
                row,
                start_ms,
                end_ms,
            } => {
                app.paint_hits_on_lane(row, start_ms, end_ms);
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
            Self::Gains { song, main, hitsounds } => {
                app.preferences.song_gain = song.clamp(0.0, 1.0);
                app.preferences.master_gain = main.clamp(0.2, 1.0);
                if let Some(hs) = hitsounds {
                    app.preferences.hitsound_gain = hs.clamp(0.0, 2.0);
                }
                app.gains_changed();
            }
            Self::HitsoundGain { gain } => {
                app.preferences.hitsound_gain = gain.clamp(0.0, 2.0);
                app.gains_changed();
            }
            Self::HitsoundsMuted { on } => {
                app.preferences.hitsounds_muted = on;
                app.gains_changed();
            }
            Self::HitsoundsSolo { on } => app.set_hitsounds_solo(on),
            Self::LaneGain { row, gain } => {
                let target = app.document().and_then(|d| {
                    let lane = d.composer.lanes.get(row)?;
                    Some(
                        d.composer
                            .hitsounds()
                            .lane(lane)
                            .map(|m| m.plays_as())
                            .unwrap_or_else(|| lane.clone()),
                    )
                });
                if let Some(lane) = target {
                    app.set_lane_gain(lane, gain);
                }
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
            Self::ToggleStemMonitor => {
                app.toggle_monitor_stem();
            }
            Self::LoadDrumStem { path } => app.load_drum_stem(path),
            Self::ClearDrumStem => app.clear_drum_stem(),
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
                let bound = app.duration_ms().max(600_000.0) * 10.0;
                app.set_view_start(start.clamp(-bound, bound));
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


#[derive(serde::Serialize)]
struct GameplayOverlay {
    points: Vec<(&'static str, f64, usize)>,
    spans: Vec<(&'static str, f64, f64, usize)>,
}

fn gameplay_overlay(document: &dawsu_application::Document) -> GameplayOverlay {
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
fn combo_info(document: &dawsu_application::Document) -> (Vec<(u32, usize, usize)>, Vec<String>) {
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
struct ComboOverlay {
    colours: Vec<String>,
    objects: Vec<(u32, usize)>,
}

/// The timeline's combo overlay: `colours`, and `objects` as `[number, colour]` per object (see `combo_info`).
fn combo_overlay(document: &dawsu_application::Document) -> ComboOverlay {
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
fn map_preview(app: &Application, map: &Path) -> Result<Value, String> {
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
struct ViewTickSnapshot {
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
enum ViewResponse {
    View(Box<ViewSnapshot>),
    Tick(ViewTickSnapshot),
    Fault { fault: String },
    NameTaken { #[serde(rename = "nameTaken")] name_taken: String },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneLane {
    can_add_layer: bool,
    can_delete: bool,
    can_replace: bool,
    colour_key: String,
    family: Option<String>,
    gain: f64,
    group: Option<&'static str>,
    hit_count: usize,
    index: Option<u32>,
    key: String,
    muted: bool,
    name: String,
    sample: Option<String>,
    solo: bool,
    source_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    vst_bypassed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vst_routed: Option<bool>,
}

struct SceneHit {
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

struct SceneMomentEntry {
    time: f64,
    volume: i32,
    on_object: bool,
    hits_start: usize,
    hits_end: usize,
}

struct SceneMoments {
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
struct SceneBookmark {
    label: Option<String>,
    time: f64,
}

#[derive(serde::Serialize)]
struct SceneTimeSignature {
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
struct SceneEditorBarSection {
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
struct SceneLaneLayout {
    collapsed_families: std::collections::BTreeSet<String>,
    collapsed_folders: std::collections::BTreeSet<String>,
    folder_order: Vec<String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    lane_colours: std::collections::BTreeMap<String, String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    lane_names: std::collections::BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lane_order: Vec<String>,
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
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneSnapshot {
    beats: u32,
    bookmarks: Vec<SceneBookmark>,
    #[serde(skip_serializing_if = "Option::is_none")]
    combos: Option<ComboOverlay>,
    difficulties: Vec<PathBuf>,
    divisor: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    drum_rows: Option<Vec<drum_view::DrumRowSnapshot>>,
    editor_bars: Vec<SceneEditorBarSection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gameplay: Option<GameplayOverlay>,
    history: Vec<String>,
    history_position: usize,
    hs: bool,
    insert_volume: i32,
    lane_layout: Option<SceneLaneLayout>,
    lane_order: Vec<usize>,
    lane_revision: u64,
    lanes: Vec<SceneLane>,
    map_title: String,
    moments: SceneMoments,
    orphaned_hits: Vec<f64>,
    path: PathBuf,
    pattern_stamp_step: Option<f64>,
    placement: Option<u64>,
    preview_key: u64,
    protect: bool,
    ramp_preview: Option<Vec<(f64, i32)>>,
    range: Option<[f64; 2]>,
    repeat_gap: Option<u32>,
    row_height: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    sections: Option<Vec<(f64, f64, u32)>>,
    selection: Option<selection::SelectionSummary>,
    snap: bool,
    snap_mode: &'static str,
    time_signatures: Vec<SceneTimeSignature>,
    title: String,
    version: String,
    view_start: f64,
    zoom: f64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DrumVstSnapshot {
    enabled: bool,
    instance_id: String,
    loaded: bool,
    mapped_lanes: usize,
    plugin_name: String,
    plugin_path: String,
    plugin_uid: Option<String>,
}

#[derive(serde::Serialize)]
struct OutputProblemSnapshot {
    message: String,
    target: PathBuf,
    time: f64,
    timeline: PathBuf,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PanelsSnapshot {
    browser: bool,
    controller: bool,
    inspector: bool,
    lane_names: bool,
}

#[derive(serde::Serialize)]
struct PeaksSnapshot {
    hitsounds: f64,
    master: f64,
    song: f64,
}

#[derive(serde::Serialize)]
struct PlannedFileSnapshot {
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
struct OutputReviewSnapshot {
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
struct RecoveryOfferSnapshot {
    created_ms: u64,
    difficulty: String,
    id: String,
    mapset: String,
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum RecoverySnapshot {
    Offers { offers: Vec<RecoveryOfferSnapshot> },
    Error { error: String },
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SoundSetPromptSnapshot {
    edited_keep_files: usize,
    name: String,
    open: u64,
    placements: usize,
    set_id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewSnapshot {
    active_lane: usize,
    audition: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    bookmark_step: Option<Option<f64>>,
    busy: bool,
    can_undo_revert: bool,
    classic_browser: bool,
    clean_redundant_greenlines: bool,
    cycle: bool,
    dirty: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    drum_view: Option<drum_view::DrumViewSnapshot>,
    drum_vst: Option<DrumVstSnapshot>,
    duration: f64,
    edit_dialog: Option<&'static str>,
    error: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    external_osu_changed: bool,
    first_launch: bool,
    follow: bool,
    has_document: bool,
    has_original_snapshot: bool,
    hitsound_library: Option<PathBuf>,
    hitsounds: f64,
    hitsounds_muted: bool,
    hitsounds_solo: bool,
    hs_mode: bool,
    key_preset: String,
    lane_count: usize,
    left: f64,
    main: f64,
    musical_position: Option<String>,
    object_lines: bool,
    offset: f64,
    output_device: String,
    output_problems: Vec<OutputProblemSnapshot>,
    panels: PanelsSnapshot,
    peaks: PeaksSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_home: Option<bool>,
    pending_open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    playback_start: Option<PlaybackStart>,
    playing: bool,
    position: f64,
    preview_window: bool,
    quantize_strength: f64,
    ramp_shape: String,
    recent: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<RecoverySnapshot>,
    return_on_stop: bool,
    revert: bool,
    review: Option<OutputReviewSnapshot>,
    right: f64,
    save_problem_review: bool,
    save_review: Option<Vec<(&'static str, PathBuf)>>,
    scene: Option<SceneSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shortcut_action: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shortcut_help: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shortcut_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    show_home: Option<bool>,
    song: f64,
    song_loading: bool,
    song_muted: bool,
    song_solo: bool,
    songs_folder: Option<PathBuf>,
    sound_set_prompts: Vec<SoundSetPromptSnapshot>,
    speed: f64,
    status: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stem_active: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stem_loaded: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stem_loading: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    stem_name: Option<String>,
    tab_routes: Vec<(bool, bool, bool, bool)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tick_rev: Option<u64>,
    tool: String,
    warnings: Vec<String>,
    waveform_revision: u64,
}

fn snapshot_view(app: &Application, include_scene: bool) -> ViewSnapshot {
    snapshot_with_static(app, include_scene, true)
}

#[cfg(test)]
fn snapshot(app: &Application, include_scene: bool) -> Value {
    serde_json::to_value(snapshot_view(app, include_scene)).unwrap()
}

fn musical_position_text(app: &Application, position: f64) -> Option<String> {
    app.document().map(|document| {
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
    })
}

fn snapshot_with_static(app: &Application, include_scene: bool, include_static: bool) -> ViewSnapshot {
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
                let (vst_routed, vst_bypassed) = match &app.drum_vst {
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
            let selected: std::collections::HashSet<_> = c.selected.items().iter()
                .map(|key| (key.moment, &key.lane)).collect();
            let listen_muted: std::collections::HashSet<_> = c.listen_muted.iter()
                .map(|key| (key.moment, &key.lane)).collect();
            let source_moments = c.hitsounds().moments();
            let mut moment_entries = Vec::with_capacity(source_moments.len());
            let mut hit_entries = Vec::with_capacity(source_moments.len());
            for (i, moment) in source_moments.iter().enumerate() {
                let hits_start = hit_entries.len();
                let moment_vol = moment.volume();
                for lane in moment.hits() {
                    let Some(&row) = row_by_lane.get(lane) else { continue };
                    let is_selected = selected.contains(&(i, lane));
                    let muted = listen_muted.contains(&(i, lane));
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
                difficulties: d.difficulties.clone(),
                divisor: c.spacing.divisions(),
                drum_rows: None,
                editor_bars: d.editor_bars().iter().map(Into::into).collect(),
                gameplay: (include_static || d.hs_mode).then(|| gameplay_overlay(d)),
                history: c.history.steps().into_iter().map(str::to_owned).collect(),
                history_position: c.history.position(),
                hs: d.hs_mode,
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
                row_height: f64::from(app.row_height),
                sections,
                selection,
                snap: c.snap_enabled,
                snap_mode: match c.snap_mode { SnapMode::Grid => "grid", SnapMode::GridRelative => "relative", SnapMode::Objects => "objects" },
                time_signatures: d.time_signatures().iter().map(Into::into).collect(),
                title: d.title.clone(),
                version: d.version(),
                view_start: c.view_start,
                zoom: app.zoom,
            }
        })
    } else {
        None
    };
    let review = app.output_review.as_ref().map(|plan| {
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
    ViewSnapshot {
        active_lane: app.active_lane,
        audition: app.preferences.audition,
        bookmark_step: None,
        busy: app.busy,
        can_undo_revert: app.can_undo_revert(),
        classic_browser: app.preferences.classic_browser,
        cycle: app.cycle,
        dirty: app.is_dirty(),
        drum_view: None,
        drum_vst: app.drum_vst.as_ref().map(|v| DrumVstSnapshot {
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
        error: app.error.clone(),
        external_osu_changed: app.external_osu_changed,
        first_launch: app.first_launch,
        follow: app.preferences.follow,
        has_document: document.is_some(),
        has_original_snapshot: app.has_original_snapshot(),
        hitsound_library: app.preferences.hitsound_library.clone(),
        hitsounds: f64::from(app.preferences.hitsound_gain),
        hitsounds_muted: app.preferences.hitsounds_muted,
        hitsounds_solo: app.hitsounds_solo(),
        hs_mode: document.is_some_and(|d| d.hs_mode),
        key_preset: app.preferences.key_preset.clone(),
        lane_count: document.map_or(0, |d| d.composer.lanes.len()),
        left: app.locator_left,
        main: f64::from(app.preferences.master_gain),
        musical_position,
        object_lines: app.preferences.object_lines,
        offset: app.song_offset,
        output_device: app.device_name().to_owned(),
        output_problems,
        panels: PanelsSnapshot {
            browser: app.preferences.show_browser,
            controller: app.preferences.show_controller,
            inspector: app.preferences.show_inspector,
            lane_names: app.preferences.lane_names,
        },
        peaks: PeaksSnapshot {
            hitsounds: f64::from(hitsound_peak),
            master: f64::from(master_peak),
            song: f64::from(song_peak),
        },
        pending_home: None,
        pending_open: app.pending_open.is_some(),
        playback_start: None,
        playing: app.is_playing(),
        position,
        // The map preview was in its own window when the editor last closed: main opens it again.
        preview_window: app.preferences.preview_window.is_some_and(|place| place.open),
        quantize_strength: app.quantize_strength(),
        ramp_shape: format!("{:?}", app.ramp_shape()),
        recent: app.preferences.recent.clone(),
        recovery,
        return_on_stop: app.preferences.return_on_stop,
        clean_redundant_greenlines: app.preferences.clean_redundant_greenlines,
        revert: app.revert_requested,
        review,
        right: app.locator_right,
        save_problem_review,
        save_review: app.save_review_files().map(|files| {
            files.into_iter().map(|(label, path)| (label, path.to_path_buf())).collect()
        }),
        scene,
        shortcut_action: None,
        shortcut_help: None,
        shortcut_open: None,
        show_home: None,
        song: f64::from(app.preferences.song_gain),
        song_loading: app.song_loading,
        song_muted: app.preferences.song_muted,
        song_solo: app.song_solo(),
        songs_folder: (!app.preferences.songs_folder.as_os_str().is_empty())
            .then(|| app.preferences.songs_folder.clone()),
        sound_set_prompts: document.map(|_| app.sound_set_prompts()).unwrap_or_default().iter().map(|p| {
            SoundSetPromptSnapshot {
                edited_keep_files: p.edited_keep_files,
                name: p.name.clone(),
                open: app.sound_set_prompt_open(),
                placements: p.placements,
                set_id: p.set_id.clone(),
            }
        }).collect(),
        speed: app.playback_speed,
        status: app.status.clone(),
        stem_active: app.is_monitoring_stem(),
        stem_loaded: app.has_drum_stem(),
        stem_loading: app.drum_stem_loading,
        stem_name: app.drum_stem_name(),
        tab_routes,
        tick_rev: None,
        tool: format!("{:?}", app.tool()),
        warnings: app.warnings.clone(),
        waveform_revision: app.waveform_revision(),
    }
}

fn encode_waveform_bytes(bins: &[dawsu_application::WaveBin]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bins.len() * 6 * 4);
    for bin in bins {
        for value in [bin.peak, bin.low, bin.mid, bin.high, bin.amp_l, bin.amp_r] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

enum Request {
    /// Scene wanted, the action, `fresh` (the screen lacks the static map data), and `tick_rev`
    /// (the last full view's revision for slim playback polls).
    View(bool, Option<Action>, bool, Option<u64>, mpsc::Sender<ViewResponse>),
    Waveform(mpsc::Sender<Result<Vec<u8>, String>>),
    Help(mpsc::Sender<Result<Value, String>>),
    AudioOutputs(Option<OutputBackend>, mpsc::Sender<Value>),
    Shortcuts(mpsc::Sender<Value>),
    ExportSettings(mpsc::Sender<Option<(PathBuf, ExportSettings)>>),
    KeyFile(PathBuf, bool, mpsc::Sender<Result<(), String>>),
    SongsDirectory(mpsc::Sender<Option<PathBuf>>),
    SetSongsDirectory(PathBuf, mpsc::Sender<Result<ViewResponse, String>>),
    LibraryDirectory(mpsc::Sender<Option<PathBuf>>),
    SetLibraryDirectory(PathBuf, mpsc::Sender<Result<ViewResponse, String>>),
    LibraryEntries(library::Location, mpsc::Sender<Result<Vec<(String, bool)>, String>>),
    SamplePreview(PreviewSource, mpsc::Sender<Result<Value, String>>),
    SoundSetLibrary(mpsc::Sender<Result<Value, String>>),
    SoundSetPlacement(PathBuf, u64, mpsc::Sender<Result<Value, String>>),
    SaveSoundSet(PathBuf, String, mpsc::Sender<Result<ViewResponse, String>>),
    LaneAddCandidates(PathBuf, mpsc::Sender<Result<Value, String>>),
    MapPreview(PathBuf, mpsc::Sender<Result<Value, String>>),
    /// The map preview window's place, kept for the next open; `true` also saves the preferences.
    PreviewPlace(dawsu_application::WindowPlace, bool),
    PreviewPlaceGet(mpsc::Sender<Option<dawsu_application::WindowPlace>>),
    /// Only whether the preview window is open changed; `true` also saves the preferences.
    PreviewOpen(bool, bool),
    DrumView(PathBuf, Option<PathBuf>, bool, mpsc::Sender<Result<ViewResponse, String>>),
    DrumMapAction(PathBuf, drum_view::DrumMapAction, mpsc::Sender<Result<ViewResponse, String>>),
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
    /// M4 Node Graph queries and actions on the active editor session.
    NodeGraphView(mpsc::Sender<Result<Value, String>>),
    NodeGraphAction(dawsu_application::NodeGraphAction, mpsc::Sender<Result<Value, String>>),
    NodeGraphExportJson(mpsc::Sender<Result<String, String>>),
    NodeGraphClose,
    /// MIDI import, export, live MIDI recording, VST3 scan, and Keysounding actions.
    InspectMidiFile(String, mpsc::Sender<Result<dawsu_application::MidiInspectResult, String>>),
    ApplyMidiImport(
        dawsu_application::MidiImportOptions,
        mpsc::Sender<Result<dawsu_application::MidiImportOutcome, String>>,
    ),
    ExportMidiFile(PathBuf, mpsc::Sender<Result<usize, String>>),
    ImportManiaHitsounds(
        PathBuf,
        mpsc::Sender<Result<dawsu_application::ManiaImportReport, String>>,
    ),
    MidiRecordToggle(Option<usize>, Option<u32>, mpsc::Sender<Result<dawsu_application::MidiRecordStatus, String>>),
    Vst3Scan(mpsc::Sender<Result<dawsu_application::Vst3ScanReport, String>>),
    DrumVstAction(Value, mpsc::Sender<Result<Value, String>>),
    KeysoundsAction(Value, mpsc::Sender<Result<Value, String>>),
    PreviewRevertMapsetToOriginal(mpsc::Sender<Result<dawsu_application::SnapshotPreview, String>>),
    RevertMapsetToOriginal(Vec<String>, mpsc::Sender<Result<usize, String>>),
    UndoMapsetRevert(mpsc::Sender<Result<usize, String>>),
    SharedBaseStatus(mpsc::Sender<Result<dawsu_application::SharedBaseStatus, String>>),
    SharedBaseSetEnabled(bool, mpsc::Sender<Result<dawsu_application::SharedBaseStatus, String>>),
    SharedBaseCapture(mpsc::Sender<Result<dawsu_application::SharedBaseStatus, String>>),
    SharedBaseApply(
        f64,
        bool,
        mpsc::Sender<Result<dawsu_application::SharedBaseApplyResult, String>>,
    ),
    SharedBaseClearOverrides(mpsc::Sender<Result<dawsu_application::SharedBaseStatus, String>>),
    SharedBaseClear(mpsc::Sender<Result<dawsu_application::SharedBaseStatus, String>>),
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
            "laterWins": choices.later_wins,
            "mixFormat": choices.mix_format,
            "sliderEndMute": {
                "enabled": choices.slider_end_mute.enabled,
                "minBeats": choices.slider_end_mute.min_beats,
                "muteSpinnerEnds": choices.slider_end_mute.mute_spinner_ends,
                "onlyUnhit": choices.slider_end_mute.only_unhit,
                "divisors": choices.slider_end_mute.divisors,
            },
            "fallbackSound": choices.fallback_sound,
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
                .map(|file| {
                    let diff = file.diff_summary();
                    let mut entry = json!({ "path": file.path, "replaces": file.expected.is_some() });
                    if !diff.is_empty() {
                        entry["diff"] = json!(diff);
                    }
                    entry
                }),
            "mixes": preview.plan.mix_paths().collect::<Vec<_>>(),
            "createdMixes": preview.plan.created_mix_paths().collect::<Vec<_>>(),
            "reusedMixes": preview.plan.reused_mix_paths().collect::<Vec<_>>(),
            "sampleWarnings": preview.plan.sample_warning_lines(),
            "volumeSummary": preview.plan.volume_clarity_lines(),
            "targets": preview.plan.targets.iter().map(|counts| {
                let diff = preview.plan.files.iter()
                    .find(|file| file.path == counts.path)
                    .map(|file| file.diff_summary())
                    .filter(|diff| !diff.is_empty());
                let mut entry = json!({
                    "path": counts.path,
                    "matched": counts.matched,
                    "unmatched": counts.unmatched,
                    "changed": counts.changed,
                    "collisions": counts.collisions,
                    "keptFromSilent": counts.kept_from_silent,
                    "muted": counts.muted,
                    "fallbackApplied": counts.fallback_applied,
                });
                if let Some(diff) = diff {
                    entry["diff"] = json!(diff);
                }
                entry
            }).collect::<Vec<_>>(),
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

#[derive(Default)]
struct ViewTickCache {
    rev: u64,
    last_non_playback_sig: Option<u64>,
    drums_rev: u64,
    static_scene_key: Option<(u64, u64, usize)>,
    last_hs_mode: bool,
}

impl ViewTickCache {
    fn invalidate(&mut self) {
        self.rev += 1;
        self.last_non_playback_sig = None;
    }
}

fn non_playback_signature(
    app: &Application,
    home: &home::Route,
    playback_start: PlaybackStart,
    drums_rev: u64,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    app.status.hash(&mut hasher);
    app.error.hash(&mut hasher);
    app.warnings.hash(&mut hasher);
    app.busy.hash(&mut hasher);
    app.is_playing().hash(&mut hasher);
    let doc = app.document();
    doc.is_some().hash(&mut hasher);
    if let Some(d) = doc {
        app.document_generation().hash(&mut hasher);
        d.composer.history.version().hash(&mut hasher);
        d.composer.lanes.len().hash(&mut hasher);
        d.hs_mode.hash(&mut hasher);
        d.composer.snap_enabled.hash(&mut hasher);
        d.composer.spacing.beats().hash(&mut hasher);
        d.composer.spacing.divisions().hash(&mut hasher);
        d.path.hash(&mut hasher);
    }
    home.visible().hash(&mut hasher);
    home.pending().hash(&mut hasher);
    app.is_dirty().hash(&mut hasher);
    std::mem::discriminant(&app.tool()).hash(&mut hasher);
    std::mem::discriminant(&app.ramp_shape()).hash(&mut hasher);
    app.active_lane.hash(&mut hasher);
    app.locator_left.to_bits().hash(&mut hasher);
    app.locator_right.to_bits().hash(&mut hasher);
    app.cycle.hash(&mut hasher);
    app.preferences.follow.hash(&mut hasher);
    app.preferences.return_on_stop.hash(&mut hasher);
    app.preferences.clean_redundant_greenlines.hash(&mut hasher);
    app.preferences.audition.hash(&mut hasher);
    app.preferences.object_lines.hash(&mut hasher);
    app.preferences.classic_browser.hash(&mut hasher);
    app.song_offset.to_bits().hash(&mut hasher);
    app.playback_speed.to_bits().hash(&mut hasher);
    app.preferences.song_gain.to_bits().hash(&mut hasher);
    app.preferences.master_gain.to_bits().hash(&mut hasher);
    app.preferences.hitsound_gain.to_bits().hash(&mut hasher);
    app.preferences.song_muted.hash(&mut hasher);
    app.preferences.hitsounds_muted.hash(&mut hasher);
    app.song_solo().hash(&mut hasher);
    app.hitsounds_solo().hash(&mut hasher);
    app.duration_ms().to_bits().hash(&mut hasher);
    app.song_loading.hash(&mut hasher);
    app.drum_stem_loading.hash(&mut hasher);
    app.has_drum_stem().hash(&mut hasher);
    app.is_monitoring_stem().hash(&mut hasher);
    app.waveform_revision().hash(&mut hasher);
    app.active_waveform().len().hash(&mut hasher);
    let problems = app.output_problems();
    problems.len().hash(&mut hasher);
    for problem in &problems {
        problem.target.hash(&mut hasher);
        problem.timeline.hash(&mut hasher);
        problem.time.to_bits().hash(&mut hasher);
        problem.message.hash(&mut hasher);
    }
    (app.save_problem_review_requested() && !problems.is_empty()).hash(&mut hasher);
    app.save_review_files().as_ref().map(Vec::len).hash(&mut hasher);
    app.sound_set_prompt_open().hash(&mut hasher);
    let prompts = app.sound_set_prompts();
    prompts.len().hash(&mut hasher);
    for prompt in prompts {
        prompt.set_id.hash(&mut hasher);
        prompt.placements.hash(&mut hasher);
        prompt.edited_keep_files.hash(&mut hasher);
    }
    app.drum_vst
        .as_ref()
        .map(|v| (&v.instance_id, v.is_enabled(), v.routed_notes().len(), v.muted_vst_lanes().len()))
        .hash(&mut hasher);
    app.drum_vst_loading.hash(&mut hasher);
    app.revert_requested.hash(&mut hasher);
    app.external_osu_changed.hash(&mut hasher);
    app.output_review.as_ref().map(|p| (&p.label, p.files.len(), p.details.len())).hash(&mut hasher);
    app.requested_edit().hash(&mut hasher);
    app.bookmark_step.map(f64::to_bits).hash(&mut hasher);
    app.help_requested.hash(&mut hasher);
    app.requests.iter().any(|request| matches!(request, PlatformRequest::OpenMap)).hash(&mut hasher);
    app.pending_open.is_some().hash(&mut hasher);
    app.first_launch.hash(&mut hasher);
    app.has_original_snapshot().hash(&mut hasher);
    app.can_undo_revert().hash(&mut hasher);
    app.device_name().hash(&mut hasher);
    app.quantize_strength().to_bits().hash(&mut hasher);
    app.preferences.key_preset.hash(&mut hasher);
    app.preferences.hitsound_library.hash(&mut hasher);
    app.preferences.songs_folder.hash(&mut hasher);
    app.preferences.show_browser.hash(&mut hasher);
    app.preferences.show_inspector.hash(&mut hasher);
    app.preferences.show_controller.hash(&mut hasher);
    app.preferences.lane_names.hash(&mut hasher);
    app.preferences.recent.len().hash(&mut hasher);
    app.preferences.recent.first().hash(&mut hasher);
    app.preferences.preview_window.map(|p| p.open).hash(&mut hasher);
    playback_start.hash(&mut hasher);
    drums_rev.hash(&mut hasher);
    hasher.finish()
}

fn decorate_and_stamp_view(
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
    cache.last_non_playback_sig = Some(non_playback_signature(app, home, playback_start, cache.drums_rev));
    view.tick_rev = Some(cache.rev);
    ViewResponse::View(Box::new(view))
}

#[allow(clippy::too_many_arguments)]
fn next_view_response(
    app: &mut Application,
    home: &mut home::Route,
    drums: &drum_view::DrumView,
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
        Some(Action::Open { path }) if app.first_launch => {
            if map_path(&path) {
                *pending_initial_open = Some(path);
            }
        }
        Some(action) => {
            if let Some(action) = home.handle(action, app) {
                action.run_with_start(app, playback_start);
            }
        }
        None => {}
    }
    if !app.first_launch
        && let Some(path) = pending_initial_open.take()
    {
        app.request_open(path);
    }
    app.poll();
    home.settle(app);

    if !had_action && !scene && !fresh && tick_rev == Some(cache.rev) {
        let sig = non_playback_signature(app, home, *playback_start, cache.drums_rev);
        if cache.last_non_playback_sig == Some(sig) {
            let position = app.position_ms();
            let (master_peak, song_peak, hitsound_peak) = app.audio_peaks();
            return ViewResponse::Tick(ViewTickSnapshot {
                tick: true,
                rev: cache.rev,
                position,
                playing: app.is_playing(),
                busy: app.busy,
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
    view.shortcut_help = Some(std::mem::take(&mut app.help_requested));
    // The Inspector's bookmark pick follows Previous and Next, from a button, menu or key.
    view.bookmark_step = Some(app.bookmark_step.take());
    view.shortcut_open = Some(std::mem::take(&mut app.requests).iter()
        .any(|request| matches!(request, PlatformRequest::OpenMap)));
    cache.rev += 1;
    cache.last_non_playback_sig = Some(non_playback_signature(app, home, *playback_start, cache.drums_rev));
    view.tick_rev = Some(cache.rev);
    ViewResponse::View(Box::new(view))
}

fn editor_thread(initial: Option<PathBuf>) -> Editor {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let data_root = dawsu_application::data_folder();
        #[cfg(feature = "recovery-smoke")]
        let data_root = std::env::var_os(RECOVERY_SMOKE_DATA)
            .filter(|path| !path.is_empty())
            .map_or(data_root, PathBuf::from);
        let mut app = Application::for_desktop(data_root);
        let mut fault = None;
        let mut export_cache = ExportCache::default();
        let mut playback_start = PlaybackStart::default();
        let mut home = home::Route::default();
        let mut drums = drum_view::DrumView::for_editor();
        let mut tick_cache = ViewTickCache::default();
        let mut pending_initial_open = initial.filter(|path| map_path(path));
        if !app.first_launch && let Some(path) = pending_initial_open.take() {
            let _ = guard(&mut app, &mut fault, |app| app.request_open(path));
        }
        loop {
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(Request::View(scene, action, fresh, tick_rev, response)) => {
                    let view = guard(&mut app, &mut fault, |app| {
                        next_view_response(
                            app,
                            &mut home,
                            &drums,
                            &mut playback_start,
                            &mut tick_cache,
                            &mut pending_initial_open,
                            scene,
                            action,
                            fresh,
                            tick_rev,
                        )
                    })
                    .unwrap_or_else(|message| ViewResponse::Fault { fault: message });
                    let _ = response.send(view);
                }
                Ok(Request::Waveform(response)) => {
                    let result = guard(&mut app, &mut fault, |app| encode_waveform_bytes(app.active_waveform()));
                    let _ = response.send(result);
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
                    if import && result.is_ok() {
                        tick_cache.invalidate();
                    }
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
                            Ok(decorate_and_stamp_view(app, &home, &drums, playback_start, &mut tick_cache, false))
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
                        Ok(decorate_and_stamp_view(app, &home, &drums, playback_start, &mut tick_cache, false))
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
                Ok(Request::SamplePreview(source, response)) => {
                    let result = guard(&mut app, &mut fault, |app| sample_outline(app, source))
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
                        Ok(()) => Ok(decorate_and_stamp_view(app, &home, &drums, playback_start, &mut tick_cache, !home.visible())),
                        Err(dawsu_application::SaveSetError::NameTaken(message)) => {
                            Ok(ViewResponse::NameTaken { name_taken: message })
                        }
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
                Ok(Request::MapPreview(map, response)) => {
                    let result = guard(&mut app, &mut fault, |app| map_preview(app, &map)).and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::PreviewPlace(place, save)) => {
                    app.preferences.preview_window = Some(place);
                    if save && !app.first_launch {
                        app.save_preferences();
                    }
                }
                Ok(Request::PreviewOpen(open, save)) => {
                    if let Some(place) = app.preferences.preview_window.as_mut() {
                        place.open = open;
                    }
                    if save && !app.first_launch {
                        app.save_preferences();
                    }
                }
                Ok(Request::PreviewPlaceGet(response)) => {
                    let _ = response.send(app.preferences.preview_window);
                }
                Ok(Request::DrumView(map, preset, on, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        match preset {
                            Some(path) => drums.load(app, &map, &path),
                            None => drums.toggle(app, &map, on),
                        }.map(|()| {
                            app.set_drum_vst_routing(drums.routed_midi_notes(app));
                            tick_cache.drums_rev += 1;
                            decorate_and_stamp_view(app, &home, &drums, playback_start, &mut tick_cache, !home.visible())
                        })
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::DrumMapAction(map, action, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        match action {
                            drum_view::DrumMapAction::LoadBuiltin { name } => {
                                drums.load_builtin(app, &map, &name)
                            }
                            drum_view::DrumMapAction::AssignRow { row_index, lane_key } => {
                                drums.assign_row(app, &map, row_index, lane_key.as_deref())
                            }
                            drum_view::DrumMapAction::SetMidiNote { row_index, midi_note } => {
                                drums.set_midi_note(app, &map, row_index, midi_note)
                            }
                            drum_view::DrumMapAction::AutoDetect => {
                                drums.auto_detect_now(app, &map)
                            }
                            drum_view::DrumMapAction::SetAutoDetect { on } => {
                                drums.set_auto_detect(app, &map, on)
                            }
                            drum_view::DrumMapAction::SetHideUnassigned { on } => {
                                drums.set_hide_unassigned(app, &map, on)
                            }
                            drum_view::DrumMapAction::MoveRow { from_index, to_index } => {
                                drums.move_row(app, &map, from_index, to_index)
                            }
                        }.map(|()| {
                            app.set_drum_vst_routing(drums.routed_midi_notes(app));
                            tick_cache.drums_rev += 1;
                            decorate_and_stamp_view(app, &home, &drums, playback_start, &mut tick_cache, !home.visible())
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
                                    // Frees the numbers of new samples nothing keeps.
                                    app.close_document();
                                    if !app.first_launch {
                                        app.save_preferences();
                                    }
                                })
                            }
                        })
                        .and_then(|result| result)
                    };
                    if result.is_ok() {
                        tick_cache.invalidate();
                    }
                    let _ = response.send(result);
                }
                Ok(request @ (Request::ExportView(_) | Request::ExportPreview(..) | Request::ExportRun(..))) => {
                    if matches!(request, Request::ExportRun(..)) {
                        tick_cache.invalidate();
                    }
                    answer_export(&mut app, &mut fault, &mut export_cache, request);
                }
                Ok(Request::ExportClose) => {
                    if fault.is_none() {
                        let _ = guard(&mut app, &mut fault, Application::cancel_export_preview);
                    }
                }
                Ok(Request::Drum(request, response)) => {
                    let result = guard(&mut app, &mut fault, |app| drum_window::answer(app, request)).and_then(|result| result);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::DrumClose) => {
                    if fault.is_none() {
                        let _ = guard(&mut app, &mut fault, Application::cancel_drum_detection);
                    }
                }
                Ok(Request::InspectMidiFile(path, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        let document = app.document().ok_or("Open a difficulty first.")?;
                        let lanes = document.composer.hitsounds().lanes();
                        dawsu_application::inspect_midi_file(
                            Path::new(&path),
                            lanes,
                            drums.active_map(app),
                        )
                    })
                    .and_then(|r| r);
                    let _ = response.send(result);
                }
                Ok(Request::ApplyMidiImport(options, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        let drum_map = drums.active_map(app).cloned();
                        app.apply_midi_import(&options, drum_map.as_ref())
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::ExportMidiFile(path, response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.export_midi(&path))
                        .and_then(|r| r);
                    let _ = response.send(result);
                }
                Ok(Request::ImportManiaHitsounds(path, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.import_mania_hitsound_diff(&path)
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseStatus(response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.shared_base_status());
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseSetEnabled(enabled, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_set_enabled(enabled)
                    })
                    .and_then(|r| r);
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseCapture(response)) => {
                    let result =
                        guard(&mut app, &mut fault, |app| app.shared_base_capture_current())
                            .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseApply(leniency_ms, overwrite_overrides, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_apply_to_current(leniency_ms, overwrite_overrides)
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseClearOverrides(response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_clear_current_overrides()
                    })
                    .and_then(|r| r);
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseClear(response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.shared_base_clear())
                        .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::MidiRecordToggle(port_index, snap, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        let drum_map = drums.active_map(app).cloned();
                        app.toggle_midi_recording(port_index, drum_map.as_ref(), snap)
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::Vst3Scan(response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.scan_vst3_plugins(&[]))
                        .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::DrumVstAction(payload, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        handle_drum_vst_action(app, &drums, &payload)
                    })
                    .and_then(|r| r);
                    tick_cache.drums_rev += 1;
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::KeysoundsAction(payload, response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.keysounds_action(&payload))
                        .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::PreviewRevertMapsetToOriginal(response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.preview_revert_mapset_to_original()
                    })
                    .and_then(|r| r);
                    let _ = response.send(result);
                }
                Ok(Request::RevertMapsetToOriginal(include_deleted, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.revert_mapset_to_original(&include_deleted)
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::UndoMapsetRevert(response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.undo_mapset_revert())
                        .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::NodeGraphView(response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.poll_node_graph();
                        serde_json::to_value(app.node_graph_view()).map_err(|e| e.to_string())
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::NodeGraphAction(action, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.node_graph_action(action)?;
                        serde_json::to_value(app.node_graph_view()).map_err(|e| e.to_string())
                    })
                    .and_then(|result| result);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::NodeGraphExportJson(response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.node_graph_export_json())
                        .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::NodeGraphClose) => {
                    if fault.is_none() {
                        let _ = guard(&mut app, &mut fault, Application::cancel_node_graph_and_clear_viewer);
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
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
    scene: bool,
    action: Option<Action>,
    fresh: Option<bool>,
    tick_rev: Option<u64>,
) -> Result<ViewResponse, String> {
    let view = ask(&state, |tx| Request::View(scene, action, fresh.unwrap_or(false), tick_rev, tx))?;
    if let ViewResponse::View(snapshot) = &view {
        if let Some(scene_val) = &snapshot.scene {
            let map_title = scene_val.title.as_str();
            let version = scene_val.version.as_str();
            let file_stem = scene_val
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let title = if !version.is_empty() && !map_title.is_empty() {
                format!("{map_title} [{version}] — dawsu!")
            } else if !file_stem.is_empty() {
                format!("{file_stem} — dawsu!")
            } else {
                "dawsu!".to_string()
            };
            let _ = window.set_title(&title);
        } else if !snapshot.has_document {
            let _ = window.set_title("dawsu!");
        }
    }
    Ok(view)
}

#[tauri::command]
fn editor_waveform(state: State<'_, Editor>) -> Result<tauri::ipc::Response, String> {
    let bytes = ask(&state, Request::Waveform)??;
    Ok(tauri::ipc::Response::new(bytes))
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
    #[serde(default, rename = "autoFixStagedMixes")]
    auto_fix_staged_mixes: bool,
    #[serde(default, rename = "mixFormat")]
    mix_format: MixFormat,
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
        targets: options.targets,
        copy: options.options,
        base: options.base,
        slider_audio: options.slider_audio,
        auto_fix_staged_mixes: options.auto_fix_staged_mixes,
        mix_format: options.mix_format,
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

/// Opens the map preview in its own window, where it last was, or brings it forward when open.
/// Async, so building the window never runs on the main thread that drives it.
#[tauri::command]
async fn preview_window_open(app: tauri::AppHandle, state: State<'_, Editor>) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("preview") {
        let _ = window.unminimize();
        // A reloaded editor page learns again that the window is open.
        let _ = app.emit_to("main", "preview-window", true);
        return window.set_focus().map_err(|error| error.to_string());
    }
    let place = ask(&state, Request::PreviewPlaceGet)?;
    // dawsu! 3.1.4's map preview window: 480 × 420 at first, 300 × 240 at least (PlayfieldWindow.cs).
    let mut builder = tauri::WebviewWindowBuilder::new(&app, "preview", tauri::WebviewUrl::App("preview.html".into()))
        .title("Map preview")
        .inner_size(place.map_or(480.0, |p| p.width.max(300.0)), place.map_or(420.0, |p| p.height.max(240.0)))
        .min_inner_size(300.0, 240.0)
        .skip_taskbar(true)
        .disable_drag_drop_handler();
    // Owned by the editor, as 3.1.4's was, so a click on the editor never hides it behind
    // (owned on Windows, a child window on macOS, transient on Linux).
    if let Some(main) = app.get_webview_window("main") {
        builder = builder.parent(&main).map_err(|error| error.to_string())?;
    }
    match place.filter(|place| on_a_monitor(&app, place)) {
        Some(place) => builder = builder.position(place.x, place.y),
        // The first time, in the middle of the screen rather than its top-left corner.
        None => builder = builder.center(),
    }
    let built = builder.build();
    let result = match (built, app.get_webview_window("preview")) {
        (Ok(_), _) => Ok(()),
        (Err(_), Some(window)) => window.set_focus().map_err(|error| error.to_string()),
        (Err(error), None) => Err(error.to_string()),
    };
    if result.is_ok() {
        remember_preview_place(&app, &state, true, false);
        let _ = app.emit_to("main", "preview-window", true);
    }
    result
}

/// Whether a saved place still shows at least 40 × 40 px of the window on a monitor that is there.
/// (Windows puts a window snapped to the left edge a few pixels left of the screen.)
fn on_a_monitor(app: &tauri::AppHandle, place: &dawsu_application::WindowPlace) -> bool {
    app.available_monitors().unwrap_or_default().iter().any(|monitor| {
        let scale = monitor.scale_factor();
        let (at, size) = (monitor.position().to_logical::<f64>(scale), monitor.size().to_logical::<f64>(scale));
        let across = (place.x + place.width).min(at.x + size.width) - place.x.max(at.x);
        let down = (place.y + place.height).min(at.y + size.height) - place.y.max(at.y);
        across >= 40.0 && down >= 40.0
    })
}

/// Hands the preview window's place to the editor, if the window is there.
fn remember_preview_place(app: &tauri::AppHandle, editor: &Editor, open: bool, save: bool) {
    let Some(window) = app.get_webview_window("preview") else { return };
    // Minimised (Windows reports −32000 and 0 × 0) or maximised, the normal place is the last one kept;
    // only whether it is open changes.
    if window.is_minimized().unwrap_or(false) || window.is_maximized().unwrap_or(false) {
        let _ = editor.0.send(Request::PreviewOpen(open, save));
        return;
    }
    let (Ok(scale), Ok(at), Ok(size)) = (window.scale_factor(), window.outer_position(), window.inner_size()) else { return };
    let (at, size) = (at.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
    let place = dawsu_application::WindowPlace { x: at.x, y: at.y, width: size.width, height: size.height, open };
    let _ = editor.0.send(Request::PreviewPlace(place, save));
}

/// Closes the map preview window the way its title bar does.
#[tauri::command]
fn close_preview_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("preview") {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Ok(()),
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

/// Opens the node graph window beside the editor, or brings it forward when it is open.
#[tauri::command]
async fn open_node_graph_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("node-graph") {
        let _ = window.unminimize();
        return window.set_focus().map_err(|error| error.to_string());
    }
    let built = tauri::WebviewWindowBuilder::new(&app, "node-graph", tauri::WebviewUrl::App("node-graph.html".into()))
        .title("Node graph")
        .inner_size(1280.0, 800.0)
        .min_inner_size(960.0, 620.0)
        .disable_drag_drop_handler()
        .build();
    match (built, app.get_webview_window("node-graph")) {
        (Ok(_), _) => Ok(()),
        (Err(_), Some(window)) => window.set_focus().map_err(|error| error.to_string()),
        (Err(error), None) => Err(error.to_string()),
    }
}

#[tauri::command]
fn close_node_graph_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("node-graph") {
        Some(window) => window.close().map_err(|error| error.to_string()),
        None => Ok(()),
    }
}

fn emit_node_graph_sync(app: &tauri::AppHandle, view: &Value, refresh_timeline: bool) {
    let viewer = view.get("viewer").cloned().unwrap_or(Value::Null);
    let _ = app.emit_to("main", "node-graph-viewer", viewer);
    if refresh_timeline {
        let _ = app.emit_to("main", "node-graph-applied", true);
    }
}

#[tauri::command]
fn node_graph_view(app: tauri::AppHandle, state: State<'_, Editor>) -> Result<Value, String> {
    let view = ask(&state, Request::NodeGraphView)??;
    emit_node_graph_sync(&app, &view, false);
    Ok(view)
}

#[tauri::command]
fn node_graph_action(
    app: tauri::AppHandle,
    state: State<'_, Editor>,
    action: dawsu_application::NodeGraphAction,
) -> Result<Value, String> {
    let refresh_timeline = matches!(
        action,
        dawsu_application::NodeGraphAction::Apply | dawsu_application::NodeGraphAction::SelectLastHits
    );
    let view = ask(&state, |tx| Request::NodeGraphAction(action, tx))??;
    emit_node_graph_sync(&app, &view, refresh_timeline);
    Ok(view)
}

#[tauri::command]
fn node_graph_export_json(state: State<'_, Editor>) -> Result<String, String> {
    ask(&state, Request::NodeGraphExportJson)?
}

#[tauri::command]
fn node_graph_pick_open(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    state: State<'_, Editor>,
) -> Result<Option<Value>, String> {
    let Some(path) = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Load node graph recipe")
        .add_filter("Node graph", &["dawsugraph", "json"])
        .pick_file()
    else {
        return Ok(None);
    };
    let json = std::fs::read_to_string(&path).map_err(|e| format!("Could not read recipe: {e}"))?;
    let view = ask(&state, |tx| {
        Request::NodeGraphAction(dawsu_application::NodeGraphAction::ImportJson { json }, tx)
    })??;
    emit_node_graph_sync(&app, &view, false);
    Ok(Some(view))
}

#[tauri::command]
fn node_graph_pick_save(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<bool, String> {
    use dawsu_foundation::safe_write::{self, BackupPolicy, WriteOutcome};
    let json = ask(&state, Request::NodeGraphExportJson)??;
    let Some(mut path) = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Save node graph recipe")
        .add_filter("Node graph", &["dawsugraph"])
        .save_file()
    else {
        return Ok(false);
    };
    if path.extension().is_none() {
        path.set_extension("dawsugraph");
    }
    let bytes = json.as_bytes();
    match std::fs::read(&path) {
        Ok(expected) => {
            let backups = BackupPolicy::new(dawsu_application::data_folder().join("backups"), 5);
            match safe_write::replace(&path, &expected, bytes, &backups).map_err(|e| e.to_string())? {
                WriteOutcome::Saved { .. } => {}
                WriteOutcome::ChangedOnDisk => {
                    return Err("The recipe file changed while saving. Choose a new file or try again.".into());
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            safe_write::create(&path, bytes).map_err(|e| e.to_string())?;
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(true)
}

#[tauri::command]
fn node_graph_pick_audio(window: tauri::WebviewWindow) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Pick an audio or stem file")
        .add_filter("Audio", &["wav", "ogg", "mp3", "flac"])
        .pick_file()
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
) -> Result<ViewResponse, String> {
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
        Request::View(true, selected.map(|path| Action::Open { path }), false, None, tx)
    })
}

#[tauri::command]
fn pick_sample(window: tauri::WebviewWindow, state: State<'_, Editor>, target: SampleTarget) -> Result<ViewResponse, String> {
    let mut dialog = rfd::FileDialog::new().set_parent(&window)
        .set_title("Choose replacement sample")
        .add_filter("Audio sample", &["wav", "ogg", "mp3"]);
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let selected = dialog.pick_file();
    ask(&state, |tx| Request::View(true, selected.map(|path| Action::ReplaceSample { target, path, mapset: false }), false, None, tx))
}

#[tauri::command]
fn pick_drum_stem(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<ViewResponse, String> {
    let mut dialog = rfd::FileDialog::new().set_parent(&window)
        .set_title("Attach drum stem")
        .add_filter("Audio", &["wav", "ogg", "mp3", "flac"]);
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let selected = dialog.pick_file();
    ask(&state, |tx| Request::View(true, selected.map(|path| Action::LoadDrumStem { path }), false, None, tx))
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
async fn save_sound_set(state: State<'_, Editor>, map: PathBuf, name: String) -> Result<ViewResponse, String> {
    let editor = state.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        ask_within(&editor, Duration::from_secs(120), |tx| Request::SaveSoundSet(map, name, tx))
    }).await.map_err(|e| e.to_string())??
}

#[tauri::command]
fn pick_sound_set(window: tauri::WebviewWindow, state: State<'_, Editor>, map: PathBuf) -> Result<ViewResponse, String> {
    let selected = rfd::FileDialog::new().set_parent(&window)
        .set_title("Place sound set")
        .add_filter("dawsu! sound set", &["dawsuset"])
        .pick_file();
    ask(&state, |tx| Request::View(true, selected.map(|path| Action::PlaceSoundSet { map, path }), false, None, tx))
}

fn songs_dialog(window: &tauri::WebviewWindow, state: &Editor) -> Result<rfd::FileDialog, String> {
    let dialog = rfd::FileDialog::new().set_parent(window);
    Ok(match ask(state, Request::SongsDirectory)? {
        Some(folder) => dialog.set_directory(folder),
        None => dialog,
    })
}

struct SongsIndexCache(
    std::sync::Arc<std::sync::Mutex<Option<(PathBuf, &'static str, Vec<dawsu_application::songs_index::SongMapsetEntry>)>>>,
);

#[tauri::command]
async fn songs_index_search(
    query: String,
    refresh: bool,
    state: State<'_, Editor>,
    cache: State<'_, SongsIndexCache>,
) -> Result<dawsu_application::songs_index::SongsSearchResult, String> {
    let Some(songs_dir) = ask(&state, Request::SongsDirectory)? else {
        return Ok(dawsu_application::songs_index::SongsSearchResult {
            total_mapsets: 0,
            total_diffs: 0,
            source: "none",
            matches: Vec::new(),
        });
    };
    let cache_arc = std::sync::Arc::clone(&cache.0);
    tauri::async_runtime::spawn_blocking(move || {
        const SEARCH_LIMIT: usize = 50;
        if !refresh {
            if let Ok(guard) = cache_arc.lock() {
                if let Some((cached_dir, source, entries)) = guard.as_ref() {
                    if cached_dir == &songs_dir {
                        let total_diffs = entries.iter().map(|m| m.diffs.len()).sum();
                        let matches = dawsu_application::songs_index::search_songs_index(
                            entries,
                            &query,
                            SEARCH_LIMIT,
                        );
                        return Ok(dawsu_application::songs_index::SongsSearchResult {
                            total_mapsets: entries.len(),
                            total_diffs,
                            source,
                            matches,
                        });
                    }
                }
            }
        }
        let (entries, source) =
            dawsu_application::songs_index::build_songs_index_with_source(&songs_dir);
        let total_mapsets = entries.len();
        let total_diffs = entries.iter().map(|m| m.diffs.len()).sum();
        let matches =
            dawsu_application::songs_index::search_songs_index(&entries, &query, SEARCH_LIMIT);
        if let Ok(mut guard) = cache_arc.lock() {
            *guard = Some((songs_dir, source, entries));
        }
        Ok(dawsu_application::songs_index::SongsSearchResult {
            total_mapsets,
            total_diffs,
            source,
            matches,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn pick_songs_folder(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<ViewResponse>, String> {
    let Some(folder) = songs_dialog(&window, &state)?.set_title("Choose Songs folder").pick_folder() else {
        return Ok(None);
    };
    ask(&state, |tx| Request::SetSongsDirectory(folder, tx))?.map(Some)
}

#[tauri::command]
fn pick_hitsound_library(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<ViewResponse>, String> {
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

/// Bars in the Browser's sample preview outline.
const PREVIEW_BARS: usize = 40;

/// The sample a Browser row stands for: a map folder sample by the lane that plays it, or a
/// library file.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum PreviewSource {
    Lane { row: usize },
    Library { location: library::Location },
}

/// The Browser's sample preview: the sample's length in ms and its outline.
fn sample_outline(app: &mut Application, source: PreviewSource) -> Result<Value, String> {
    let (ms, peaks) = match source {
        PreviewSource::Lane { row } => {
            let lane = app.document().and_then(|d| d.composer.lanes.get(row)).ok_or("No such lane.")?;
            app.lane_sample_outline(lane, PREVIEW_BARS).ok_or("The sample isn't loaded.")?
        }
        PreviewSource::Library { location } => {
            let sample = location.sample(app.preferences.hitsound_library.as_deref())?;
            app.sample_file_outline(&sample, PREVIEW_BARS)?
        }
    };
    Ok(json!({ "ms": ms, "peaks": peaks }))
}

#[tauri::command]
fn sample_preview(source: PreviewSource, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::SamplePreview(source, tx))?
}

#[tauri::command]
fn lane_add_candidates(map: PathBuf, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::LaneAddCandidates(map, tx))?
}

#[tauri::command]
fn map_preview_objects(map: PathBuf, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::MapPreview(map, tx))?
}

#[tauri::command]
fn pick_drum_map(
    map: PathBuf,
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<ViewResponse>, String> {
    let Some(path) = rfd::FileDialog::new().set_parent(&window).set_title("Open drum map")
        .add_filter("Drum map", &["dawsudrum"]).pick_file() else { return Ok(None) };
    ask(&state, |tx| Request::DrumView(map, Some(path), true, tx))?.map(Some)
}

#[tauri::command]
fn set_drum_view(map: PathBuf, on: bool, state: State<'_, Editor>) -> Result<ViewResponse, String> {
    ask(&state, |tx| Request::DrumView(map, None, on, tx))?
}

#[tauri::command]
fn drum_map_action(
    map: PathBuf,
    action: drum_view::DrumMapAction,
    state: State<'_, Editor>,
) -> Result<ViewResponse, String> {
    ask(&state, |tx| Request::DrumMapAction(map, action, tx))?
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

#[tauri::command]
fn skin_textures() -> Value {
    skin_textures::load_skin_textures_json()
}

#[tauri::command]
fn preview_skins(state: State<'_, Editor>) -> Result<Value, String> {
    let songs = ask(&state, Request::SongsDirectory)?;
    Ok(skin_textures::list_available_skins(songs.as_deref()))
}

#[tauri::command]
fn preview_skin_load(skin_id: String) -> Result<Value, String> {
    if skin_id == "argon" || skin_id.is_empty() {
        return Ok(Value::Null);
    }
    if skin_id == "classic" {
        return Ok(skin_textures::load_skin_textures_json());
    }
    if let Some(dir) = skin_id.strip_prefix("dir:") {
        return skin_textures::load_skin_from_dir(Path::new(dir));
    }
    Err(format!("Unknown preview skin: {skin_id}"))
}

#[tauri::command]
fn pick_skin_folder(window: tauri::WebviewWindow) -> Option<String> {
    rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Choose osu! skin folder")
        .pick_folder()
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_from_osu(state: State<'_, Editor>) -> Result<String, String> {
    let songs = ask(&state, Request::SongsDirectory)?;
    let path = osu_link::find_open_osu_map(songs.as_deref())?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn pick_midi_file(window: tauri::WebviewWindow) -> Option<String> {
    rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Import MIDI file")
        .add_filter("MIDI file", &["mid", "midi"])
        .pick_file()
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
fn inspect_midi_file(
    path: String,
    state: State<'_, Editor>,
) -> Result<dawsu_application::MidiInspectResult, String> {
    ask(&state, |tx| Request::InspectMidiFile(path, tx))?
}

#[tauri::command]
fn apply_midi_import(
    path: String,
    tracks: Vec<usize>,
    use_beatmap_tempo: Option<bool>,
    offset_ms: Option<f64>,
    snap: Option<bool>,
    follow_volume: Option<bool>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::MidiImportOutcome, String> {
    let options = dawsu_application::MidiImportOptions {
        path,
        tracks,
        use_beatmap_tempo: use_beatmap_tempo.unwrap_or(true),
        offset_ms: offset_ms.unwrap_or(0.0),
        snap: snap.unwrap_or(true),
        follow_volume: follow_volume.unwrap_or(false),
    };
    ask(&state, |tx| Request::ApplyMidiImport(options, tx))?
}

#[tauri::command]
fn export_midi_file(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<usize>, String> {
    let mut dialog = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Export MIDI file")
        .add_filter("MIDI file", &["mid"])
        .set_file_name("hitsounds.mid");
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let Some(mut path) = dialog.save_file() else {
        return Ok(None);
    };
    if path.extension().is_none() {
        path.set_extension("mid");
    }
    ask(&state, |tx| Request::ExportMidiFile(path, tx))?.map(Some)
}

#[tauri::command]
fn import_mania_hitsounds(
    window: tauri::WebviewWindow,
    state: State<'_, Editor>,
) -> Result<Option<dawsu_application::ManiaImportReport>, String> {
    let mut dialog = rfd::FileDialog::new()
        .set_parent(&window)
        .set_title("Import mania hitsound difficulty")
        .add_filter("osu! beatmap", &["osu"]);
    if let Ok(Ok(folder)) = ask(&state, |tx| Request::Folder(folders::Folder::Map, tx)) {
        dialog = dialog.set_directory(folder);
    }
    let Some(path) = dialog.pick_file() else {
        return Ok(None);
    };
    ask(&state, |tx| Request::ImportManiaHitsounds(path, tx))?.map(Some)
}

#[tauri::command]
fn shared_base_status(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, Request::SharedBaseStatus)?
}

#[tauri::command]
fn shared_base_set_enabled(
    enabled: bool,
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, |tx| Request::SharedBaseSetEnabled(enabled, tx))?
}

#[tauri::command]
fn shared_base_capture(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, Request::SharedBaseCapture)?
}

#[tauri::command]
fn shared_base_apply(
    leniency_ms: Option<f64>,
    overwrite_overrides: Option<bool>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseApplyResult, String> {
    ask(&state, |tx| {
        Request::SharedBaseApply(
            leniency_ms.unwrap_or(5.0),
            overwrite_overrides.unwrap_or(false),
            tx,
        )
    })?
}

#[tauri::command]
fn shared_base_clear_overrides(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, Request::SharedBaseClearOverrides)?
}

#[tauri::command]
fn shared_base_clear(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SharedBaseStatus, String> {
    ask(&state, Request::SharedBaseClear)?
}

#[tauri::command]
fn midi_input_ports() -> Vec<String> {
    dawsu_application::list_midi_input_ports()
}

#[tauri::command]
fn midi_record_toggle(
    port_index: Option<usize>,
    snap_divisor: Option<u32>,
    state: State<'_, Editor>,
) -> Result<dawsu_application::MidiRecordStatus, String> {
    ask(&state, |tx| Request::MidiRecordToggle(port_index, snap_divisor, tx))?
}

#[tauri::command]
fn vst3_scan(state: State<'_, Editor>) -> Result<dawsu_application::Vst3ScanReport, String> {
    ask(&state, Request::Vst3Scan)?
}

#[tauri::command]
fn drum_vst_action(payload: Value, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::DrumVstAction(payload, tx))?
}

fn handle_drum_vst_action(
    app: &mut Application,
    drums: &drum_view::DrumView,
    payload: &Value,
) -> Result<Value, String> {
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("Missing drum_vst_action kind")?;
    match kind {
        "status" => Ok(drum_vst_status_json(app)),
        "list_installed" => {
            let plugins = dawsu_application::vst3::list_installed_vst3_in(app.vst3.search_dirs());
            let folders: Vec<String> = app
                .vst3
                .search_dirs()
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            Ok(json!({
                "plugins": plugins.into_iter().map(|p| json!({
                    "name": p.label,
                    "path": p.path.to_string_lossy(),
                })).collect::<Vec<_>>(),
                "folders": folders,
            }))
        }
        "browse" => {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Choose a VST3 instrument")
                .add_filter("VST3 plug-in", &["vst3"]);
            if let Some(dir) = app.vst3.search_dirs().iter().find(|d| d.is_dir()) {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.pick_file() {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("VST3")
                    .to_string();
                Ok(json!({
                    "picked": true,
                    "name": name,
                    "path": path.to_string_lossy(),
                }))
            } else {
                Ok(json!({ "picked": false }))
            }
        }
        "load" => {
            let path = payload
                .get("pluginPath")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or("Choose a VST3 plug-in file first.")?;
            let uid = payload
                .get("pluginUid")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            app.load_drum_vst(PathBuf::from(path), uid)
                .map_err(|e| e.to_string())?;
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            Ok(drum_vst_status_json(app))
        }
        "toggle" => {
            let on = payload.get("enabled").and_then(Value::as_bool).unwrap_or(false);
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            app.set_drum_vst_enabled(on);
            Ok(drum_vst_status_json(app))
        }
        "toggle_lane" => {
            let row = payload
                .get("row")
                .and_then(Value::as_u64)
                .ok_or("Missing lane row")? as usize;
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            app.toggle_drum_vst_lane(row)?;
            Ok(drum_vst_status_json(app))
        }
        "unload" => {
            app.unload_drum_vst();
            Ok(drum_vst_status_json(app))
        }
        "open_editor" => {
            let id = app
                .drum_vst
                .as_ref()
                .map(|v| v.instance_id.clone())
                .ok_or("Load a Drum VST3 instrument first.")?;
            app.vst3.open_editor(&id).map_err(|e| e.to_string())?;
            Ok(drum_vst_status_json(app))
        }
        "close_editor" => {
            let id = app
                .drum_vst
                .as_ref()
                .map(|v| v.instance_id.clone())
                .ok_or("Load a Drum VST3 instrument first.")?;
            app.vst3.close_editor(&id).map_err(|e| e.to_string())?;
            let _ = app.save_drum_vst_session_state();
            Ok(drum_vst_status_json(app))
        }
        "preview_note" => {
            let note = payload.get("note").and_then(Value::as_u64).unwrap_or(36).min(127) as u8;
            let velocity = payload.get("velocity").and_then(Value::as_u64).unwrap_or(100).clamp(1, 127) as u8;
            if let Some(vst) = &app.drum_vst {
                vst.preview(note, velocity);
            }
            Ok(json!({ "ok": true }))
        }
        "note_names" => {
            let id = app
                .drum_vst
                .as_ref()
                .map(|v| v.instance_id.clone())
                .ok_or("Load a Drum VST3 instrument first.")?;
            let names = app.vst3.note_names(&id).map_err(|e| e.to_string())?;
            Ok(json!({
                "notes": names.into_iter().map(|(note, name)| json!({ "note": note, "name": name })).collect::<Vec<_>>()
            }))
        }
        "sync_state" => {
            app.save_drum_vst_session_state().map_err(|e| e.to_string())?;
            Ok(drum_vst_status_json(app))
        }
        "save_state" => {
            let b64 = app.save_drum_vst_session_state().map_err(|e| e.to_string())?;
            let picked = rfd::FileDialog::new()
                .set_title("Save Drum VST3 state")
                .set_file_name("drums-state.bin")
                .add_filter("VST3 state", &["bin", "vststate"])
                .save_file();
            if let Some(path) = picked {
                std::fs::write(&path, b64.as_bytes()).map_err(|e| e.to_string())?;
                Ok(json!({ "saved": true, "path": path.to_string_lossy() }))
            } else {
                Ok(json!({ "saved": false }))
            }
        }
        "load_state" => {
            let picked = rfd::FileDialog::new()
                .set_title("Load Drum VST3 state")
                .add_filter("VST3 state", &["bin", "vststate"])
                .pick_file();
            if let Some(path) = picked {
                let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
                app.restore_drum_vst_state_b64(raw.trim()).map_err(|e| e.to_string())?;
                Ok(json!({ "loaded": true, "path": path.to_string_lossy() }))
            } else {
                Ok(json!({ "loaded": false }))
            }
        }
        "plan_render" => {
            let exact = payload.get("exact").and_then(Value::as_bool).unwrap_or(false);
            let mode = if exact {
                dawsu_application::VelocityExportMode::ExactPerVolume
            } else {
                dawsu_application::VelocityExportMode::Tiered { max_interval: 20 }
            };
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            let plan = app.plan_drum_vst_render(mode)?;
            Ok(json!({
                "exact": exact,
                "hitCount": plan.assignments.len(),
                "jobCount": plan.jobs.len(),
                "jobs": plan.jobs.iter().map(|j| json!({
                    "filename": j.filename,
                    "pitch": j.pitch,
                    "midiVelocity": j.midi_velocity,
                })).collect::<Vec<_>>(),
            }))
        }
        "render_wavs" => {
            let exact = payload.get("exact").and_then(Value::as_bool).unwrap_or(false);
            let mode = if exact {
                dawsu_application::VelocityExportMode::ExactPerVolume
            } else {
                dawsu_application::VelocityExportMode::Tiered { max_interval: 20 }
            };
            app.set_drum_vst_routing(drums.routed_midi_notes(app));
            let (plan, outputs) = app.render_drum_vst_to_wavs(mode)?;
            Ok(json!({
                "renderedCount": outputs.len(),
                "hitCount": plan.assignments.len(),
                "files": outputs.iter().map(|o| o.path.file_name().and_then(|n| n.to_str()).unwrap_or("")).collect::<Vec<_>>(),
            }))
        }
        _ => Err(format!("Unknown drum_vst_action: {kind}")),
    }
}

fn drum_vst_status_json(app: &Application) -> Value {
    match &app.drum_vst {
        Some(v) => json!({
            "loaded": true,
            "instanceId": v.instance_id,
            "pluginName": v.plugin_name,
            "pluginPath": v.plugin_path,
            "pluginUid": v.plugin_uid,
            "enabled": v.is_enabled(),
            "mappedLanes": v.routed_notes().len(),
        }),
        None => json!({
            "loaded": false,
            "enabled": false,
            "mappedLanes": 0,
        }),
    }
}

#[tauri::command]
fn keysounds_action(payload: Value, state: State<'_, Editor>) -> Result<Value, String> {
    ask(&state, |tx| Request::KeysoundsAction(payload, tx))?
}

#[tauri::command]
fn preview_revert_mapset_to_original(
    state: State<'_, Editor>,
) -> Result<dawsu_application::SnapshotPreview, String> {
    ask(&state, Request::PreviewRevertMapsetToOriginal)?
}

#[tauri::command]
fn revert_mapset_to_original(
    include_deleted: Option<Vec<String>>,
    state: State<'_, Editor>,
) -> Result<usize, String> {
    ask(&state, |tx| {
        Request::RevertMapsetToOriginal(include_deleted.unwrap_or_default(), tx)
    })?
}

#[tauri::command]
fn undo_mapset_revert(state: State<'_, Editor>) -> Result<usize, String> {
    ask(&state, Request::UndoMapsetRevert)?
}

#[tauri::command]
async fn check_for_updates() -> Result<updater::UpdateCheckResult, String> {
    tauri::async_runtime::spawn_blocking(updater::check_latest_release)
        .await
        .map_err(|e| format!("Update check task failed: {e}"))?
}

fn close_requested(label: &str, editor: &Editor) -> CloseDecision {
    // Closing Export or the drum detection window never waits on the editor.
    match label {
        "export" => { let _ = editor.0.send(Request::ExportClose); }
        "drum" => { let _ = editor.0.send(Request::DrumClose); }
        "node-graph" => { let _ = editor.0.send(Request::NodeGraphClose); }
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
    crash::install();
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    tauri::Builder::default()
        .manage(editor_thread(initial))
        .manage(SongsIndexCache(std::sync::Arc::new(std::sync::Mutex::new(None))))
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match (window.label(), event) {
                // The preview window's place, kept as it moves; saved when it closes by itself.
                ("preview", tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) => {
                    remember_preview_place(app, &window.state::<Editor>(), true, false);
                }
                ("preview", tauri::WindowEvent::CloseRequested { .. }) => {
                    remember_preview_place(app, &window.state::<Editor>(), false, true);
                }
                ("preview", tauri::WindowEvent::Destroyed) => {
                    let _ = app.emit_to("main", "preview-window", false);
                }
                // Main closing with the preview open: it opens again next time (saved with the session).
                ("main", tauri::WindowEvent::CloseRequested { .. }) => {
                    remember_preview_place(app, &window.state::<Editor>(), true, false);
                }
                _ => {}
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "node-graph" {
                    let _ = window.app_handle().emit_to("main", "node-graph-viewer", Value::Null);
                }
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
            editor_waveform,
            editor_help,
            editor_export_settings,
            export_view,
            export_preview,
            export_run,
            open_export_window,
            close_export_window,
            show_in_editor,
            drum_window_open,
            preview_window_open,
            close_preview_window,
            close_drum_window,
            drum_context,
            drum_detect_start,
            drum_review,
            drum_listen,
            drum_listen_stop,
            drum_apply,
            drum_pick_stem,
            open_node_graph_window,
            close_node_graph_window,
            node_graph_view,
            node_graph_action,
            node_graph_export_json,
            node_graph_pick_open,
            node_graph_pick_save,
            node_graph_pick_audio,
            pick_merge_preset,
            save_merge_preset,
            editor_shortcuts,
            audio_outputs,
            import_key_file,
            export_key_file,
            pick_map,
            pick_sample,
            pick_drum_stem,
            pick_sound_set,
            sound_set_library,
            sound_set_placement,
            save_sound_set,
            pick_songs_folder,
            songs_index_search,
            pick_hitsound_library,
            list_library,
            sample_preview,
            lane_add_candidates,
            map_preview_objects,
            pick_drum_map,
            set_drum_view,
            drum_map_action,
            open_editor_folder,
            close_editor,
            skin_textures,
            preview_skins,
            preview_skin_load,
            pick_skin_folder,
            open_from_osu,
            pick_midi_file,
            inspect_midi_file,
            apply_midi_import,
            export_midi_file,
            import_mania_hitsounds,
            shared_base_status,
            shared_base_set_enabled,
            shared_base_capture,
            shared_base_apply,
            shared_base_clear_overrides,
            shared_base_clear,
            midi_input_ports,
            midi_record_toggle,
            vst3_scan,
            drum_vst_action,
            keysounds_action,
            preview_revert_mapset_to_original,
            revert_mapset_to_original,
            undo_mapset_revert,
            check_for_updates
        ])
        .run(tauri::generate_context!())
        .expect("start the dawsu! Tauri editor");
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod preset_close_test;
