//! The drum detection window (v3 f12): its requests, answered on the editor thread. Every answer
//! is the window's fresh view, so the window never shows a state older than its own action.

use dawsu_application::{Application, Drum, DrumDetectState, DrumOptions, DrumRow, DrumSource, ObjectKind, RowLands, GRIDS, MAX_OBJECT_MS};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use super::scene_lane_key;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SourceJson {
    Song,
    Stem { path: PathBuf },
}

/// The review's options as the window sends them; lanes by their scene key.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionsJson {
    pub kick: Option<String>,
    pub snare: Option<String>,
    pub sensitivity: u8,
    pub object_ms: f64,
    pub grid: Option<u32>,
    pub stem_offset: f64,
}

#[derive(Debug)]
pub enum DrumRequest {
    /// The window's poll. `known` is the review stamp whose rows it already has.
    Context { known: Option<String> },
    Start { map: PathBuf, source: SourceJson },
    Review { map: PathBuf, options: OptionsJson },
    Listen { map: PathBuf, from: f64, ticked: Vec<String> },
    StopListen,
    Apply { map: PathBuf, review: String, ticked: Vec<String> },
}

/// Runs one request and answers with the window's view.
pub fn answer(app: &mut Application, request: DrumRequest) -> Result<Value, String> {
    let known = match request {
        DrumRequest::Context { known } => known,
        DrumRequest::Start { map, source } => {
            same_map(app, &map)?;
            app.start_drum_detection(match source {
                SourceJson::Song => DrumSource::Song,
                SourceJson::Stem { path } => DrumSource::Stem(path),
            })?;
            None
        }
        DrumRequest::Review { map, options } => {
            same_map(app, &map)?;
            let lane = |key: Option<String>| key.map(|key| lane_by_key(app, &key)).transpose();
            let options = DrumOptions {
                kick: lane(options.kick)?,
                snare: lane(options.snare)?,
                sensitivity: options.sensitivity,
                object_ms: options.object_ms,
                grid: options.grid,
                stem_offset: options.stem_offset,
            };
            app.set_drum_options(options)?;
            None
        }
        DrumRequest::Listen { map, from, ticked } => {
            same_map(app, &map)?;
            app.listen_drum_detection(from, &ticked)?;
            None
        }
        DrumRequest::StopListen => {
            app.stop_drum_listen();
            None
        }
        DrumRequest::Apply { map, review, ticked } => {
            same_map(app, &map).map_err(|_| "The open difficulty changed. Nothing was applied.".to_owned())?;
            let review = review.parse().map_err(|_| "Detect drum hits first.".to_owned())?;
            app.apply_drum_detection(review, &ticked)?;
            None
        }
    };
    app.poll();
    Ok(context(app, known.as_deref()))
}

/// The window chose for the difficulty it showed, not one opened since.
fn same_map(app: &Application, map: &Path) -> Result<(), String> {
    match app.document() {
        Some(document) if document.path == map => Ok(()),
        _ => Err("The open difficulty changed. Check the window again.".into()),
    }
}

/// A lane drum hit detection may fill: any lane of the difficulty, hidden ones too.
fn lane_by_key(app: &Application, key: &str) -> Result<dawsu_foundation::SampleIdentity, String> {
    app.document()
        .and_then(|document| document.composer.hitsounds().lanes().iter()
            .find(|lane| scene_lane_key(&lane.identity) == key))
        .map(|lane| lane.identity.clone())
        .ok_or_else(|| "A chosen lane no longer exists. Pick the lanes again.".into())
}

fn lands(lands: RowLands) -> &'static str {
    match lands {
        RowLands::Object(ObjectKind::Circle) => "circle",
        RowLands::Object(ObjectKind::SliderHead) => "sliderHead",
        RowLands::Object(ObjectKind::SliderRepeat) => "sliderRepeat",
        RowLands::Object(ObjectKind::SliderEnd) => "sliderEnd",
        RowLands::Object(ObjectKind::SpinnerEnd) => "spinnerEnd",
        RowLands::Grid => "grid",
        RowLands::Flagged => "flagged",
        RowLands::Duplicate => "duplicate",
        RowLands::NoLane => "noLane",
    }
}

fn row(row: &DrumRow) -> Value {
    json!({
        "id": row.id,
        "drum": if row.drum == Drum::Kick { "kick" } else { "snare" },
        "time": row.time,
        "strength": row.strength,
        "lands": lands(row.lands),
    })
}

/// The window's view. Rows come only when the review changed since `known`.
pub fn context(app: &Application, known: Option<&str>) -> Value {
    let view = app.drum_detection();
    let document = app.document();
    let options = &view.options;
    let mut result = json!({
        "map": document.map(|d| &d.path),
        "title": document.map(|d| &d.title),
        "hasSong": view.has_song,
        "duration": app.duration_ms(),
        "state": "idle",
        "error": view.error,
        "source": view.source.as_ref().map(|source| match source {
            DrumSource::Song => json!({ "kind": "song" }),
            DrumSource::Stem(path) => json!({
                "kind": "stem", "path": path,
                "name": path.file_name().map(|name| name.to_string_lossy()),
            }),
        }),
        "options": {
            "kick": options.kick.as_ref().map(scene_lane_key),
            "snare": options.snare.as_ref().map(scene_lane_key),
            "sensitivity": options.sensitivity,
            "objectMs": options.object_ms,
            "grid": options.grid,
            "stemOffset": options.stem_offset,
        },
        "editorGrid": view.editor_grid,
        "grids": GRIDS,
        "maxObjectMs": MAX_OBJECT_MS,
        "lanes": view.lanes.iter().map(|(lane, name, hidden)| json!({ "key": scene_lane_key(lane), "name": name, "hidden": hidden }))
            .collect::<Vec<_>>(),
        "listening": view.listening,
    });
    match view.state {
        DrumDetectState::Idle => {}
        DrumDetectState::Running { done, total } => {
            result["state"] = json!("running");
            result["done"] = json!(done);
            result["total"] = json!(total);
        }
        DrumDetectState::Ready { review, rows, counts, unhides } => {
            let review = review.to_string();
            result["state"] = json!("ready");
            result["counts"] = json!({
                "added": counts.added, "onObjects": counts.on_objects, "loose": counts.loose,
                "flagged": counts.flagged, "duplicates": counts.duplicates,
            });
            result["unhides"] = json!(unhides);
            if known != Some(review.as_str()) {
                result["rows"] = json!(rows.iter().map(row).collect::<Vec<_>>());
            }
            result["review"] = json!(review);
        }
    }
    result
}
