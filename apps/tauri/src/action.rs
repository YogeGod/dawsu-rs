use dawsu_application::{
    Application, EditorTimeSignature, ExportSettings, LaneLayout, OutputBackend, PointerPhase,
    RangeOperation, TimelineEdit, VolumeScope, lane_name,
};
use dawsu_editor::{
    keys::Mods,
    tempo::SnapMode,
    tools::{PointerButton, PointerInput, PointerSurface, ToolKind, ToolTarget},
    volume_ramp::RampShape,
};
use dawsu_foundation::{SampleIdentity, SampleSet, SongTime};
use dawsu_hitsounding::{
    LaneSource,
    copy::{CopyCollisionPolicy, CopyMode, CopyOptions, FallbackSound},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::{keyboard, library};

#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlaybackStart {
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
pub(crate) enum EditorPanel {
    Browser,
    Inspector,
    Controller,
    LaneNames,
}

#[derive(Deserialize)]
#[serde(remote = "CopyMode", rename_all = "snake_case")]
pub(crate) enum CopyModeJson {
    OverwriteEverything,
    OverwriteOnlyDefined,
}

#[derive(Deserialize)]
#[serde(remote = "CopyCollisionPolicy", rename_all = "snake_case")]
pub(crate) enum CopyCollisionPolicyJson {
    Merge,
    LaterWins,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SampleSetJson {
    Normal,
    Soft,
    Drum,
}

pub(crate) fn deserialize_sample_set<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<SampleSet>, D::Error> {
    Ok(Option::<SampleSetJson>::deserialize(input)?.map(|set| match set {
        SampleSetJson::Normal => SampleSet::Normal,
        SampleSetJson::Soft => SampleSet::Soft,
        SampleSetJson::Drum => SampleSet::Drum,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WhistleSetJson {
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
pub(crate) struct CopyOptionsJson {
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
    #[serde(default)]
    addition_only_normal: dawsu_hitsounding::index_packing::AdditionOnlyNormal,
    #[serde(default)]
    silent_normal_index: Option<u32>,
}

pub(crate) fn default_copy_start_index() -> u32 {
    CopyOptions::default().start_index
}

#[derive(Deserialize)]
pub(crate) struct ShortcutInput {
    key: String,
    code: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TimelineEditRequest {
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
pub(crate) struct SampleTarget {
    map: PathBuf,
    lane: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RulerGesture {
    Left,
    Right,
    Draw,
    Move,
    Seek,
}

impl RulerGesture {
    pub(crate) fn apply(self, app: &mut Application, path: &Path, from: f64, to: f64) {
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
pub(crate) struct KeyLaneView {
    path: PathBuf,
    rows: Vec<usize>,
    span: f64,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Action {
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
    AddLaneFromFile {
        map: PathBuf,
        path: PathBuf,
    },
    AddLaneFromLibrary {
        map: PathBuf,
        location: library::Location,
    },
    ShowDefaultSkinLanes {
        map: PathBuf,
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
    KeepPitch { on: bool },
    AutoCheckUpdates { on: bool },
    StepPadsMode { on: bool },
    ReloadOsuAfterSave { on: bool },
    TapLane { row: usize },
    TapRecordToggle,
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
    ToggleStepHit {
        row: usize,
        ms: f64,
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
    MomentVolume {
        ms: f64,
        value: i32,
    },
    SetSelectionSampleSet {
        set: String,
    },
    ToggleSelectionAddition {
        addition: String,
    },
    AuditionSelection,
    AutoFixProblem {
        time_ms: f64,
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
    pub(crate) fn run(self, app: &mut Application) {
        self.run_with_start(app, &mut PlaybackStart::default());
    }

    pub(crate) fn run_with_start(self, app: &mut Application, playback_start: &mut PlaybackStart) {
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
            Self::AddLaneFromFile { map, path } => {
                if let Err(error) = app.add_lane_from_file(&map, &path) {
                    app.error = Some(error);
                }
            }
            Self::AddLaneFromLibrary { map, location } => {
                if let Err(error) = location
                    .sample(app.preferences.hitsound_library.as_deref())
                    .and_then(|sample| app.add_lane_from_file(&map, &sample))
                {
                    app.error = Some(error);
                }
            }
            Self::ShowDefaultSkinLanes { map } => {
                if let Err(error) = app.show_default_skin_lanes(&map) {
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
            Self::KeepPitch { on } => app.set_keep_pitch(on),
            Self::AutoCheckUpdates { on } => app.set_auto_check_updates(on),
            Self::StepPadsMode { on } => app.set_step_pads_mode(on),
            Self::ReloadOsuAfterSave { on } => app.set_reload_osu_after_save(on),
            Self::TapLane { row } => {
                app.tap_lane(row);
            }
            Self::TapRecordToggle => {
                if let Err(error) = app.toggle_tap_recording() {
                    app.error = Some(error);
                }
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
            Self::ToggleStepHit { row, ms } => {
                app.toggle_step_hit(row, ms);
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
            Self::MomentVolume { ms, value } => {
                app.set_moment_volume_at(ms, value);
            }
            Self::SetSelectionSampleSet { set } => {
                let parsed = match set.to_ascii_lowercase().as_str() {
                    "normal" => Some(SampleSet::Normal),
                    "soft" => Some(SampleSet::Soft),
                    "drum" => Some(SampleSet::Drum),
                    _ => None,
                };
                if let Some(sample_set) = parsed {
                    app.set_selection_sample_set(sample_set);
                }
            }
            Self::ToggleSelectionAddition { addition } => {
                let sound = match addition.to_ascii_lowercase().as_str() {
                    "whistle" => Some(dawsu_foundation::Hitsound::Whistle),
                    "finish" => Some(dawsu_foundation::Hitsound::Finish),
                    "clap" => Some(dawsu_foundation::Hitsound::Clap),
                    _ => None,
                };
                if let Some(sound) = sound {
                    app.toggle_selection_addition(sound);
                }
            }
            Self::AuditionSelection => app.audition_selection(),
            Self::AutoFixProblem { time_ms } => {
                app.auto_fix_problem(time_ms);
            }
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

pub(crate) fn map_path(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("osu") || ext.eq_ignore_ascii_case("osz"))
}

pub(crate) fn sample_set_name(set: SampleSet) -> &'static str {
    match set {
        SampleSet::Normal => "normal",
        SampleSet::Soft => "soft",
        SampleSet::Drum => "drum",
    }
}

pub(crate) fn mix_lanes(app: &mut Application, lanes: Vec<SampleIdentity>, operation: &str, clicked: Option<SampleIdentity>) {
    type LanePredicate = fn(&Application, &SampleIdentity) -> bool;
    type LaneToggle = fn(&mut Application, SampleIdentity);
    let (enabled, toggle): (LanePredicate, LaneToggle) =
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

pub(crate) fn group_mix(app: &mut Application, set: &str, operation: &str) {
    let Some(document) = app.document().filter(|_| !app.busy) else {
        return;
    };
    let lanes = document.composer.lanes.iter().filter(|lane| {
        matches!(lane, SampleIdentity::Standard { set: lane_set, .. } if sample_set_name(*lane_set) == set)
    }).cloned().collect();
    mix_lanes(app, lanes, operation, None);
}

pub(crate) fn lane_selection_mix(app: &mut Application, target: &SampleTarget, operation: &str, lanes: Vec<String>) {
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
pub(crate) fn lane_header_rows<'a>(
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
pub(crate) fn family_order<'a>(rows: Vec<usize>, lane: impl Fn(usize) -> &'a SampleIdentity) -> Vec<usize> {
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
pub(crate) fn scene_lane_key(lane: &SampleIdentity) -> String {
    format!("{lane:?}")
}

pub(crate) fn lane_by_key(app: &Application, key: &str) -> Result<SampleIdentity, String> {
    app.document()
        .and_then(|document| document.composer.lanes.iter().find(|lane| scene_lane_key(lane) == key))
        .cloned()
        .ok_or_else(|| "The lane changed. Choose a lane again.".into())
}

pub(crate) fn set_volume_lane_scope(app: &mut Application, path: &Path, lanes: Option<Vec<String>>) -> Result<(), String> {
    let document = app.document().filter(|document| document.path == path)
        .ok_or("The open difficulty changed. Select the volume lanes again.")?;
    let model = document.composer.hitsounds();
    let scope = lanes.map(|keys| keys.into_iter().map(|key| {
        let lane = lane_by_key(app, &key)?;
        Ok(model.lane_slot(&lane).cloned().unwrap_or(lane))
    }).collect::<Result<BTreeSet<_>, String>>()).transpose()?;
    app.set_selection_lane_scope(scope)
}

pub(crate) fn lane_add_choices(app: &Application, map: &Path) -> Result<Value, String> {
    let candidates = app.scan_lane_add_candidates(map)?;
    let model = app.document().ok_or("Open a difficulty first.")?.composer.hitsounds();
    Ok(json!(candidates.iter().map(|lane| json!({
        "key": scene_lane_key(lane),
        "name": lane_name(model.lane_slot(lane).unwrap_or(lane)),
    })).collect::<Vec<_>>()))
}

pub(crate) fn assign_sample(app: &mut Application, target: SampleTarget, path: PathBuf, mapset: bool) -> Result<(), String> {
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
