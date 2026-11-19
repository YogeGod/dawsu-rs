#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dawsu_application::{Application, PointerPhase, lane_name};
use dawsu_editor::{
    keys::Mods,
    tempo::SnapMode,
    tools::{PointerButton, PointerInput, PointerSurface, ToolKind},
};
use dawsu_foundation::SongTime;
use dawsu_hitsounding::{Place, copy::CopyOptions};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};
use tauri::{Emitter, Manager, State};

#[cfg(feature = "recovery-smoke")]
const RECOVERY_SMOKE_DATA: &str = "DAWSU_RECOVERY_SMOKE_DATA";

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Action {
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
    Save,
    ConfirmRevert,
    CancelRevert,
    Play,
    Stop,
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
    Volume {
        value: i32,
        relative: bool,
    },
    InsertVolume {
        value: i32,
    },
    Gains {
        song: f32,
        main: f32,
    },
    SongMuted {
        on: bool,
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
    View {
        start: f64,
        zoom: f64,
        row_height: f32,
    },
    KeyPreset {
        name: String,
    },
    Generate,
    NewDifficulty {
        name: String,
    },
    Copy {
        targets: Vec<PathBuf>,
        leniency: f64,
    },
    Output {
        apply: bool,
    },
    DismissError,
    CancelJob,
}

impl Action {
    fn run(self, app: &mut Application) {
        match self {
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
            Self::Pending { choice } => match choice.as_str() {
                "save" => app.save(),
                "discard" => app.discard_and_open(),
                _ => app.pending_open = None,
            },
            Self::Command { id } => app.command(&id),
            Self::Save => app.save(),
            Self::ConfirmRevert => app.confirm_revert(),
            Self::CancelRevert => app.revert_requested = false,
            Self::Play => app.play_pause(),
            Self::Stop => app.stop(),
            Self::Seek { ms } => app.seek(ms),
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
                        _ => {
                            app.active_lane = row;
                            app.audition(&lane);
                        }
                    }
                }
            }
            Self::Volume { value, relative } => app.selection_volume(value, relative),
            Self::InsertVolume { value } => app.set_insert_volume(value),
            Self::Gains { song, main } => {
                app.preferences.song_gain = song.clamp(0.0, 1.0);
                app.preferences.master_gain = main.clamp(0.2, 1.0);
                app.gains_changed();
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
            Self::Generate => app.generate(),
            Self::NewDifficulty { name } => app.new_diff(name),
            Self::Copy { targets, leniency } => {
                app.copy_hitsounds(
                    targets,
                    CopyOptions {
                        leniency: leniency.clamp(0.0, 1000.0),
                        ..Default::default()
                    },
                );
            }
            Self::Output { apply } => {
                if apply {
                    app.apply_output();
                } else {
                    app.output_review = None;
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

fn snapshot(app: &Application, include_scene: bool) -> Value {
    let document = app.document();
    let scene = if include_scene {
        document.map(|d| {
            let c = &d.composer;
            let lanes = c.lanes.iter().map(|lane| json!({
                "name": lane_name(lane), "muted": app.is_muted(lane), "solo": app.is_solo(lane)
            })).collect::<Vec<_>>();
            let row_by_lane: std::collections::HashMap<_, _> =
                c.lanes.iter().enumerate().map(|(row, lane)| (lane, row)).collect();
            let selected: std::collections::HashSet<_> = c.selected.items().iter()
                .map(|key| (key.moment, &key.lane)).collect();
            let moments = c.hitsounds().moments().iter().enumerate().map(|(i, moment)| {
                let hits = moment.hits().filter_map(|lane| row_by_lane.get(lane)
                    .map(|row| json!([row, selected.contains(&(i, lane))]))).collect::<Vec<_>>();
                json!([moment.time().ms(), moment.volume(), moment.place() != Place::NoObject, hits])
            }).collect::<Vec<_>>();
            let sections = c.tempo.sections().iter()
                .map(|s| json!([s.start.ms(), s.beat_length_ms, s.beats_per_bar]))
                .collect::<Vec<_>>();
            let waveform = app.waveform.iter()
                .map(|b| json!([b.peak, b.low, b.mid, b.high])).collect::<Vec<_>>();
            json!({
                "title": d.title, "path": d.path, "difficulties": d.difficulties,
                "lanes": lanes, "moments": moments, "sections": sections, "waveform": waveform,
                "selected": c.selected.len(), "history": c.history.steps(), "historyPosition": c.history.position(),
                "snap": c.snap_enabled, "snapMode": match c.snap_mode { SnapMode::Grid => "grid", SnapMode::GridRelative => "relative", SnapMode::Objects => "objects" },
                "beats": c.spacing.beats(), "divisor": c.spacing.divisions(),
                "insertVolume": c.insert_volume, "protect": c.protect_normals, "hs": d.hs_mode,
                "viewStart": c.view_start, "zoom": app.zoom, "rowHeight": app.row_height,
                "range": c.range.map(|r| [r.from.ms(), r.to.ms()]),
            })
        })
    } else {
        None
    };
    let review = app.output_review.as_ref().map(|p| {
        json!({
            "label": p.label, "details": p.details,
            "files": p.files.iter().map(|f| f.path.as_path()).collect::<Vec<_>>()
        })
    });
    let mut state = json!({
        "scene": scene, "hasDocument": document.is_some(), "busy": app.busy, "playing": app.is_playing(),
        "position": app.position_ms(), "duration": app.duration_ms(), "dirty": app.is_dirty(),
        "status": app.status, "error": app.error, "warnings": app.warnings, "speed": app.playback_speed,
        "song": app.preferences.song_gain, "main": app.preferences.master_gain,
        "songMuted": app.preferences.song_muted, "audition": app.preferences.audition, "offset": app.song_offset,
        "left": app.locator_left, "right": app.locator_right, "cycle": app.cycle,
        "tool": format!("{:?}", app.tool()), "pendingOpen": app.pending_open.is_some(),
        "revert": app.revert_requested, "review": review, "activeLane": app.active_lane,
        "recent": app.preferences.recent, "firstLaunch": app.first_launch,
    });
    if include_scene && document.is_none() {
        state["recovery"] = match app.recovery_offers() {
            Ok(offers) => json!({ "offers": offers }),
            Err(error) => json!({ "error": format!("Could not list recovery copies. {error}") }),
        };
    }
    state
}

enum Request {
    View(bool, Option<Action>, mpsc::Sender<Value>),
    Help(mpsc::Sender<Result<Value, String>>),
    Close(bool, mpsc::Sender<Result<(), String>>),
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
        if let Some(path) = initial.filter(|p| map_path(p)) {
            let _ = guard(&mut app, &mut fault, |app| app.request_open(path));
        }
        loop {
            match receive.recv_timeout(Duration::from_millis(25)) {
                Ok(Request::View(scene, action, response)) => {
                    let state = guard(&mut app, &mut fault, |app| {
                        if let Some(action) = action {
                            action.run(app);
                        }
                        app.poll();
                        snapshot(app, scene)
                    })
                    .unwrap_or_else(|message| json!({ "fault": message }));
                    let _ = response.send(state);
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
                Ok(Request::Close(discard, response)) => {
                    let result = if fault.is_some() {
                        Ok(())
                    } else {
                        guard(&mut app, &mut fault, |app| {
                            if app.busy {
                                Err("Wait for the current job to finish.".into())
                            } else if app.is_dirty() && !discard {
                                Err("Save or discard the current difficulty before quitting."
                                    .into())
                            } else {
                                app.save_session().map(|()| app.save_preferences())
                            }
                        })
                        .and_then(|result| result)
                    };
                    let _ = response.send(result);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if fault.is_none() {
                        let _ = guard(&mut app, &mut fault, Application::poll);
                    }
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
    ask(&state, Request::Help)?
}

#[tauri::command]
fn pick_map(window: tauri::WebviewWindow, state: State<'_, Editor>) -> Result<Value, String> {
    let selected = rfd::FileDialog::new()
        .set_parent(&window)
        .add_filter("osu! difficulty or mapset", &["osu", "osz"])
        .pick_file();
    ask(&state, |tx| {
        Request::View(true, selected.map(|path| Action::Open { path }), tx)
    })
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
            pick_map,
            close_editor
        ])
        .run(tauri::generate_context!())
        .expect("start the dawsu! Tauri editor");
}
