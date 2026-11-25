#![cfg_attr(windows, windows_subsystem = "windows")]

mod action;
mod commands;
mod crash;
mod drum_view;
mod drum_window;
mod folders;
mod home;
mod keyboard;
mod library;
mod osu_link;
mod scene;
mod selection;
mod skin_textures;
mod updater;

use action::*;
use commands::*;
use scene::*;

use dawsu_application::{Application, ExportOptions, ExportSettings, OutputBackend, lane_name};
use serde_json::{Value, json};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};
use tauri::{Emitter, Manager};

#[cfg(feature = "recovery-smoke")]
const RECOVERY_SMOKE_DATA: &str = "DAWSU_RECOVERY_SMOKE_DATA";

pub(crate) enum Request {
    /// Scene wanted, the action, `fresh` (the screen lacks the static map data), and `tick_rev`
    /// (the last full view's revision for slim playback polls).
    View(bool, Option<Action>, bool, Option<u64>, mpsc::Sender<ViewResponse>),
    Waveform(mpsc::Sender<Result<Vec<u8>, String>>),
    Help(mpsc::Sender<Result<Value, String>>),
    AudioOutputs(Option<OutputBackend>, mpsc::Sender<Value>),
    Shortcuts(mpsc::Sender<Value>),
    ExportSettings(mpsc::Sender<Option<(PathBuf, ExportSettings)>>),
    KeyFile(PathBuf, bool, mpsc::Sender<Result<(), String>>),
    CreateHitsoundDiff(
        PathBuf,
        bool,
        Option<dawsu_application::HsStarterMode>,
        mpsc::Sender<Result<PathBuf, String>>,
    ),
    CreateHitsoundDiffFromCurrent(mpsc::Sender<Result<PathBuf, String>>),
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
    ExportUnusedDryRun(mpsc::Sender<Result<Vec<dawsu_application::UnusedSampleCandidate>, String>>),
    ExportListenTarget(String, mpsc::Sender<Result<(), String>>),
    ExportStopListen(mpsc::Sender<Result<(), String>>),
    OpenMapsetRoot(mpsc::Sender<Option<PathBuf>>),
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
    SharedBaseApplyAll(
        f64,
        bool,
        mpsc::Sender<Result<dawsu_application::SharedBaseApplyAllResult, String>>,
    ),
    SharedBaseClearOverrides(mpsc::Sender<Result<dawsu_application::SharedBaseStatus, String>>),
    SharedBaseClear(mpsc::Sender<Result<dawsu_application::SharedBaseStatus, String>>),
    SharedBaseRules(Option<String>, mpsc::Sender<Result<Vec<dawsu_application::BaseRule>, String>>),
    SharedBaseAddRule(Option<String>, dawsu_application::BaseRule, mpsc::Sender<Result<Vec<dawsu_application::BaseRule>, String>>),
    SharedBaseRemoveRule(Option<String>, usize, mpsc::Sender<Result<Vec<dawsu_application::BaseRule>, String>>),
    SharedBaseDiffProjections(f64, mpsc::Sender<Result<Vec<dawsu_application::DiffProjectionTab>, String>>),
    SharedBaseVolumeEnvelope(Option<String>, mpsc::Sender<Result<dawsu_application::VolumeEnvelope, String>>),
    SharedBaseSetVolumeEnvelope(Option<String>, dawsu_application::VolumeEnvelope, mpsc::Sender<Result<dawsu_application::VolumeEnvelope, String>>),
    RelinkMovedMapset(PathBuf, Option<PathBuf>, mpsc::Sender<Result<dawsu_application::RelinkMapsetResult, String>>),
    StemSeparationStatus(mpsc::Sender<Result<dawsu_application::StemSeparationStatus, String>>),
    StemSeparationStart(
        dawsu_application::StemSeparationStartOptions,
        mpsc::Sender<Result<dawsu_application::StemSeparationStatus, String>>,
    ),
    StemSeparationCancel(mpsc::Sender<Result<dawsu_application::StemSeparationStatus, String>>),
    StemSeparationDeleteCache(
        mpsc::Sender<Result<dawsu_application::StemSeparationStatus, String>>,
    ),
}
pub(crate) struct Editor(pub(crate) mpsc::Sender<Request>, pub(crate) dawsu_application::bounce_plan::BounceCancelToken);

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
    let mut out = json!({
        "source": source,
        "targets": targets,
        "backups": app.backups_folder(),
        "busy": app.busy,
        "requested": status.requested,
        "previewing": status.running || status.waiting,
        "progress": status.progress.map(|(done, total)| json!({ "kind": "preview", "done": done, "total": total })),
        "choices": document.and_then(|d| d.export_choices()).map(|choices| {
            let mut c = json!({
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
            });
            c["autoFixStagedMixes"] = json!(choices.auto_fix_staged_mixes);
            c["writeMode"] = json!(choices.write_mode);
            c["singleFormat"] = json!(choices.single_format);
            c["additionOnlyNormal"] = json!(choices.addition_only_normal);
            c["tickPolicy"] = json!(choices.tick_policy);
            c["allowSchemaGrowth"] = json!(choices.allow_schema_growth);
            c["autoCleanupStale"] = json!(choices.auto_cleanup_stale);
            c["takeOverSamples"] = json!(choices.take_over_samples);
            c["applyVolumeEnvelope"] = json!(choices.apply_volume_envelope);
            c
        }),
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
    });
    out["playing"] = json!(app.is_playing());
    out
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
        Request::ExportUnusedDryRun(response) => {
            match guard(app, fault, |app| app.plan_sweep_snapshot()) {
                Ok(Some(snapshot)) => {
                    std::thread::spawn(move || {
                        let _ = response.send(dawsu_application::run_plan_sweep(snapshot));
                    });
                }
                Ok(None) => {
                    let _ = response.send(Ok(Vec::new()));
                }
                Err(error) => {
                    let _ = response.send(Err(error));
                }
            }
        }
        Request::ExportListenTarget(target_path, response) => {
            let result = guard(app, fault, |app| {
                if target_path.trim().is_empty() {
                    return Err("Choose a target difficulty to preview.".to_owned());
                }
                app.listen_export_target(Path::new(&target_path))
            })
            .and_then(|result| result);
            let _ = response.send(result);
        }
        Request::ExportStopListen(response) => {
            let result = guard(app, fault, Application::stop_export_listen);
            let _ = response.send(result);
        }
        _ => unreachable!("only Export requests reach answer_export"),
    }
}

fn editor_thread(initial: Option<PathBuf>) -> Editor {
    let (send, receive) = mpsc::channel();
    let bounce_cancel = dawsu_application::bounce_plan::BounceCancelToken::new();
    let thread_cancel = bounce_cancel.clone();
    std::thread::spawn(move || {
        let data_root = dawsu_application::data_folder();
        #[cfg(feature = "recovery-smoke")]
        let data_root = std::env::var_os(RECOVERY_SMOKE_DATA)
            .filter(|path| !path.is_empty())
            .map_or(data_root, PathBuf::from);
        let mut app = Application::for_desktop(data_root);
        app.bounce_cancel = thread_cancel;
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
                            &mut drums,
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
                Ok(Request::CreateHitsoundDiff(sibling_path, reuse_existing, starter_mode, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.create_hitsound_diff(&sibling_path, reuse_existing, starter_mode)
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
                }
                Ok(Request::CreateHitsoundDiffFromCurrent(response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.create_hitsound_diff_from_current()
                    })
                    .and_then(|result| result);
                    let _ = response.send(result);
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
                            app.set_drum_vst_choke_groups(drums.choke_groups(app));
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
                            app.set_drum_vst_choke_groups(drums.choke_groups(app));
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
                                if app.has_dirty_beatmap() {
                                    Err("Save or discard the current difficulty before quitting.".into())
                                } else {
                                    Err("Save or discard unsaved Keysound changes before quitting.".into())
                                }
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
                Ok(request @ (Request::ExportView(_) | Request::ExportPreview(..) | Request::ExportRun(..) | Request::ExportUnusedDryRun(_) | Request::ExportListenTarget(..) | Request::ExportStopListen(_))) => {
                    if matches!(request, Request::ExportRun(..)) {
                        tick_cache.invalidate();
                    }
                    answer_export(&mut app, &mut fault, &mut export_cache, request);
                }
                Ok(Request::ExportClose) => {
                    if fault.is_none() {
                        let _ = guard(&mut app, &mut fault, |app| {
                            app.stop_export_listen();
                            app.cancel_export_preview();
                        });
                    }
                }
                Ok(Request::OpenMapsetRoot(response)) => {
                    let root = app
                        .document()
                        .and_then(|d| d.path.parent().map(Path::to_path_buf));
                    let _ = response.send(root);
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
                Ok(Request::SharedBaseApplyAll(leniency_ms, overwrite_overrides, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_apply_to_all_diffs(leniency_ms, overwrite_overrides)
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
                Ok(Request::SharedBaseRules(diff_file, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_rules(diff_file.as_deref())
                    });
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseAddRule(diff_file, rule, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_add_rule(diff_file.as_deref(), rule)
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseRemoveRule(diff_file, index, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_remove_rule(diff_file.as_deref(), index)
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseDiffProjections(leniency_ms, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_diff_projections(leniency_ms)
                    });
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseVolumeEnvelope(diff_file, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_volume_envelope(diff_file.as_deref())
                    });
                    let _ = response.send(result);
                }
                Ok(Request::SharedBaseSetVolumeEnvelope(diff_file, envelope, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.shared_base_set_volume_envelope(diff_file.as_deref(), envelope)
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::RelinkMovedMapset(missing, explicit_dir, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.relink_moved_mapset(&missing, explicit_dir.as_deref())
                    })
                    .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::StemSeparationStatus(response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.stem_separation_status());
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::StemSeparationStart(options, response)) => {
                    let result =
                        guard(&mut app, &mut fault, |app| app.stem_separation_start(options))
                            .and_then(|r| r);
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::StemSeparationCancel(response)) => {
                    let result = guard(&mut app, &mut fault, |app| app.stem_separation_cancel());
                    let _ = response.send(result);
                }
                Ok(Request::StemSeparationDeleteCache(response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        app.stem_separation_delete_cache()
                    })
                    .and_then(|r| r);
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
                        handle_drum_vst_action(app, &mut drums, &payload)
                    })
                    .and_then(|r| r);
                    tick_cache.drums_rev += 1;
                    tick_cache.invalidate();
                    let _ = response.send(result);
                }
                Ok(Request::KeysoundsAction(payload, response)) => {
                    let result = guard(&mut app, &mut fault, |app| {
                        if payload.get("kind").and_then(Value::as_str) == Some("browse_dawsuks") {
                            let mut dialog = rfd::FileDialog::new()
                                .set_title("Open Keysound project")
                                .add_filter("dawsu! Keysound project", &["dawsuks"]);
                            if let Some(dir) = app
                                .keysound_history
                                .project_path
                                .as_ref()
                                .and_then(|p| p.parent().map(Path::to_path_buf))
                                .or_else(|| {
                                    let d = dawsu_application::data_folder().join("keysounds-projects");
                                    d.is_dir().then_some(d)
                                })
                                .or_else(|| app.document().and_then(|d| d.path.parent().map(Path::to_path_buf)))
                            {
                                dialog = dialog.set_directory(dir);
                            }
                            if let Some(path) = dialog.pick_file() {
                                let mut res = app.keysounds_action(&json!({
                                    "kind": "load_dawsuks",
                                    "path": path.to_string_lossy(),
                                }))?;
                                res["picked"] = json!(true);
                                res["loadedPath"] = json!(path.to_string_lossy());
                                Ok(res)
                            } else {
                                Ok(json!({ "picked": false }))
                            }
                        } else {
                            app.keysounds_action(&payload)
                        }
                    })
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
                            drums.sync_on_document_open(app, &mut tick_cache);
                        });
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Editor(send, bounce_cancel)
}

pub(crate) fn ask<T>(state: &Editor, request: impl FnOnce(mpsc::Sender<T>) -> Request) -> Result<T, String> {
    ask_within(&state.0, Duration::from_secs(7), request)
}

pub(crate) fn ask_within<T>(editor: &mpsc::Sender<Request>, wait: Duration, request: impl FnOnce(mpsc::Sender<T>) -> Request) -> Result<T, String> {
    let (send, receive) = mpsc::channel();
    editor
        .send(request(send))
        .map_err(|_| "The editor worker stopped".to_owned())?;
    receive
        .recv_timeout(wait)
        .map_err(|_| "The editor worker did not respond".into())
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
        .manage(updater::StagedUpdate::default())
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
            commands::editor::editor_view,
            commands::editor::editor_waveform,
            commands::editor::editor_help,
            commands::editor::editor_export_settings,
            commands::export::export_view,
            commands::export::export_preview,
            commands::export::export_run,
            commands::export::export_unused_dry_run,
            commands::export::export_listen_target,
            commands::export::export_stop_listen,
            commands::export::preview_diff_ribbon,
            commands::export::bounce_osz_budget,
            commands::midi_keysounds::keysounds_dedup_report,
            commands::windows::open_export_window,
            commands::windows::close_export_window,
            commands::windows::show_in_editor,
            commands::windows::drum_window_open,
            commands::windows::preview_window_open,
            commands::windows::close_preview_window,
            commands::windows::close_drum_window,
            commands::drum_and_graph::drum_context,
            commands::drum_and_graph::drum_detect_start,
            commands::drum_and_graph::drum_review,
            commands::drum_and_graph::drum_listen,
            commands::drum_and_graph::drum_listen_stop,
            commands::drum_and_graph::drum_apply,
            commands::drum_and_graph::drum_pick_stem,
            commands::windows::open_node_graph_window,
            commands::windows::close_node_graph_window,
            commands::drum_and_graph::node_graph_view,
            commands::drum_and_graph::node_graph_action,
            commands::drum_and_graph::node_graph_export_json,
            commands::drum_and_graph::node_graph_pick_open,
            commands::drum_and_graph::node_graph_pick_save,
            commands::drum_and_graph::node_graph_pick_audio,
            commands::export::pick_merge_preset,
            commands::export::save_merge_preset,
            commands::editor::editor_shortcuts,
            commands::editor::audio_outputs,
            commands::editor::import_key_file,
            commands::editor::export_key_file,
            commands::library_and_files::pick_map,
            commands::library_and_files::pick_map_folder,
            commands::library_and_files::pick_map_for_hitsound_diff,
            commands::library_and_files::inspect_mapset_folder,
            commands::library_and_files::create_hitsound_diff,
            commands::library_and_files::create_hitsound_diff_from_current,
            commands::library_and_files::pick_sample,
            commands::library_and_files::pick_lane_sample,
            commands::library_and_files::pick_drum_stem,
            commands::library_and_files::pick_sound_set,
            commands::library_and_files::sound_set_library,
            commands::library_and_files::sound_set_placement,
            commands::library_and_files::save_sound_set,
            commands::library_and_files::pick_songs_folder,
            commands::library_and_files::songs_index_search,
            commands::library_and_files::pick_hitsound_library,
            commands::library_and_files::list_library,
            commands::library_and_files::sample_preview,
            commands::library_and_files::lane_add_candidates,
            commands::library_and_files::map_preview_objects,
            commands::drum_and_graph::pick_drum_map,
            commands::drum_and_graph::set_drum_view,
            commands::drum_and_graph::drum_map_action,
            commands::editor::open_editor_folder,
            commands::editor::close_editor,
            commands::library_and_files::skin_textures,
            commands::library_and_files::preview_skins,
            commands::library_and_files::preview_skin_load,
            commands::library_and_files::pick_skin_folder,
            commands::editor::open_from_osu,
            commands::midi_keysounds::pick_midi_file,
            commands::midi_keysounds::inspect_midi_file,
            commands::midi_keysounds::apply_midi_import,
            commands::midi_keysounds::export_midi_file,
            commands::midi_keysounds::import_mania_hitsounds,
            commands::shared_base::shared_base_status,
            commands::shared_base::shared_base_set_enabled,
            commands::shared_base::shared_base_capture,
            commands::shared_base::shared_base_apply,
            commands::shared_base::shared_base_apply_all,
            commands::shared_base::shared_base_clear_overrides,
            commands::shared_base::shared_base_clear,
            commands::shared_base::shared_base_rules,
            commands::shared_base::shared_base_add_rule,
            commands::shared_base::shared_base_remove_rule,
            commands::shared_base::shared_base_diff_projections,
            commands::shared_base::shared_base_volume_envelope,
            commands::shared_base::shared_base_set_volume_envelope,
            commands::editor::relink_moved_mapset,
            commands::export::export_pick_hitsound_only_dir,
            commands::stem_and_updater::stem_separation_status,
            commands::stem_and_updater::stem_separation_start,
            commands::stem_and_updater::stem_separation_cancel,
            commands::stem_and_updater::stem_separation_delete_cache,
            commands::midi_keysounds::midi_input_ports,
            commands::midi_keysounds::midi_record_toggle,
            commands::midi_keysounds::vst3_scan,
            commands::drum_and_graph::drum_vst_action,
            commands::midi_keysounds::keysounds_action,
            commands::editor::preview_revert_mapset_to_original,
            commands::editor::revert_mapset_to_original,
            commands::editor::undo_mapset_revert,
            commands::stem_and_updater::check_for_updates,
            commands::stem_and_updater::download_update,
            commands::stem_and_updater::install_update_on_close,
            commands::stem_and_updater::cancel_staged_update
        ])
        .build(tauri::generate_context!())
        .expect("start the dawsu! Tauri editor")
        .run(|app_handle, event| {
            if let tauri::RunEvent::Exit = event {
                let data_root = dawsu_application::data_folder();
                app_handle
                    .state::<updater::StagedUpdate>()
                    .launch_if_staged(&data_root);
            }
        });
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod preset_close_test;
