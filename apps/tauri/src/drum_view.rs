use dawsu_application::Application;
use dawsu_hitsounding::drums::{DrumMap, DrumViewRow, MAX_DAWSUDRUM_BYTES, drum_rows, lane_key, read_dawsudrum};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct DrumView {
    mapsets: HashMap<PathBuf, Kit>,
}

struct Kit {
    preset: DrumMap,
    enabled: bool,
}

impl DrumView {
    pub(super) fn load(
        &mut self, app: &Application, expected_map: &Path, path: &Path,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?;
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("dawsudrum")) {
            return Err("Choose a .dawsudrum file.".into());
        }
        let file = File::open(path).map_err(|error| format!("Opening drum map: {error}"))?;
        let mut limited = file.take(MAX_DAWSUDRUM_BYTES as u64 + 1);
        let mut text = String::new();
        limited.read_to_string(&mut text).map_err(|error| format!("Reading drum map: {error}"))?;
        let mut preset = read_dawsudrum(&text).map_err(|error| error.to_string())?;
        // This view uses only the instrument order and settings. Embedded audio stays in the selected file.
        preset.audio = Vec::new();
        self.mapsets.insert(folder.into(), Kit { preset, enabled: true });
        Ok(())
    }

    pub(super) fn toggle(
        &mut self, app: &Application, expected_map: &Path, on: bool,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?;
        if let Some(kit) = self.mapsets.get_mut(folder) {
            kit.enabled = on;
        } else if on {
            return Err("Open a drum map first.".into());
        }
        Ok(())
    }

    pub(super) fn decorate(&self, app: &Application, view: &mut Value) {
        let Some(document) = app.document() else { return };
        let Some(kit) = document.path.parent().and_then(|mapset| self.mapsets.get(mapset)) else {
            return;
        };
        view["drumView"] = json!({"enabled": kit.enabled, "name": kit.preset.name});
        let Some(order) = view["scene"]["laneOrder"].as_array().filter(|_| kit.enabled) else {
            return;
        };
        let composer = &document.composer;
        let model = composer.hitsounds();
        let definitions: HashMap<_, _> = model.lanes().iter().map(|lane| (&lane.identity, lane)).collect();
        // A replaced lane shows in the place of the lane it fills (the scene's `key`). A drum-map row
        // may name either lane; both stand for the one shown row.
        let filled = |row: usize| {
            let shown = &view["scene"]["lanes"][row];
            let key = shown["key"].as_str().filter(|key| Some(*key) != shown["sourceKey"].as_str())?;
            model.lanes().iter().find(|lane| super::scene_lane_key(&lane.identity) == key)
        };
        let available: Vec<_> = order.iter().filter_map(|value| {
            let row = value.as_u64()? as usize;
            let identity = composer.lanes.get(row)?;
            Some((row, *definitions.get(identity)?))
        }).flat_map(|(row, lane)| std::iter::once((row, lane)).chain(filled(row).map(|slot| (row, slot)))).collect();
        let lanes = available.iter().map(|(_, lane)| (*lane).clone()).collect::<Vec<_>>();
        let rows: HashMap<_, _> = available.iter().map(|(row, lane)| (&lane.identity, *row)).collect();
        // A row on a hidden lane stays hidden, as the lane does with the drum map off.
        let hidden: HashSet<_> = composer.lanes.iter().filter(|lane| model.is_lane_hidden(lane))
            .flat_map(|lane| [Some(lane), model.lane_slot(lane)].into_iter().flatten())
            .filter_map(|lane| definitions.get(lane).map(|lane| lane_key(lane)))
            .collect();
        let entries = drum_rows(Some(&kit.preset), &lanes);
        // Several drum-map rows on one lane show as one row, at the first of them and under all
        // their names, so the view never shows a lane twice.
        let mut names: HashMap<usize, Vec<&str>> = HashMap::new();
        for entry in &entries {
            if let DrumViewRow::Instrument { row, lane } = entry {
                names.entry(rows[&lane.identity]).or_default().push(&row.name);
            }
        }
        let mut shown_rows = HashSet::new();
        let shown = entries.iter().filter_map(|entry| match entry {
            DrumViewRow::Instrument { lane, .. } => {
                let row = rows[&lane.identity];
                shown_rows.insert(row).then(|| json!({"row": row, "name": names[&row].join(" / ")}))
            }
            DrumViewRow::Unassigned(row) if row.lane.as_ref().is_some_and(|key| hidden.contains(key)) => None,
            DrumViewRow::Unassigned(row) => Some(json!({"row": null, "name": row.name})),
            DrumViewRow::Other(lane) => {
                let row = rows[&lane.identity];
                shown_rows.insert(row).then(|| json!({"row": row, "name": null}))
            }
        }).collect::<Vec<_>>();
        view["scene"]["drumRows"] = json!(shown);
    }
}

fn current_mapset<'a>(app: &'a Application, expected_map: &Path) -> Result<&'a Path, String> {
    if app.busy {
        return Err("Wait for the current job before changing the drum view.".into());
    }
    let document = app.document().ok_or("Open a difficulty first.")?;
    if document.path != expected_map {
        return Err("The difficulty changed while choosing a drum map.".into());
    }
    document.path.parent().ok_or_else(|| "The map folder is unavailable.".into())
}
