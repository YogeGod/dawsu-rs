use dawsu_application::Application;
use dawsu_hitsounding::{
    Lane,
    drums::{
        DrumMap, DrumViewRow, MAX_DAWSUDRUM_BYTES, drum_rows, lane_for_key, lane_key,
        read_dawsudrum,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum DrumMapAction {
    LoadBuiltin { name: String },
    AssignRow { row_index: usize, lane_key: Option<String> },
    SetMidiNote { row_index: usize, midi_note: Option<u8> },
    AutoDetect,
    SetAutoDetect { on: bool },
    SetHideUnassigned { on: bool },
    MoveRow { from_index: usize, to_index: usize },
}

/// The 10 built-in drum map presets bundled into every release.
pub(super) const BUILTIN_DRUM_MAPS: &[(&str, &str)] = &[
    (
        "Full kit, all 12 samples",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Full kit, all 12 samples.dawsudrum"),
    ),
    (
        "Drum kit, kick on drum-hitnormal",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Drum kit, kick on drum-hitnormal.dawsudrum"),
    ),
    (
        "Drum kit, kick on normal-hitnormal",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Drum kit, kick on normal-hitnormal.dawsudrum"),
    ),
    (
        "Kick and snare on claps",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Kick and snare on claps.dawsudrum"),
    ),
    (
        "Kick and snare on hitnormals",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Kick and snare on hitnormals.dawsudrum"),
    ),
    (
        "Kick on drum-hitnormal",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Kick on drum-hitnormal.dawsudrum"),
    ),
    (
        "Kick on normal-hitnormal",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Kick on normal-hitnormal.dawsudrum"),
    ),
    (
        "Kick on whistle (older maps)",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Kick on whistle (older maps).dawsudrum"),
    ),
    (
        "Net0 hitsound kit",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Net0 hitsound kit.dawsudrum"),
    ),
    (
        "Two kicks",
        include_str!("../../../crates/dawsu-hitsounding/drum-map-presets/Two kicks.dawsudrum"),
    ),
];

pub(super) struct DrumView {
    mapsets: HashMap<PathBuf, Kit>,
    auto_detect: bool,
    hide_unassigned: bool,
}

impl Default for DrumView {
    fn default() -> Self {
        Self {
            mapsets: HashMap::new(),
            auto_detect: true,
            hide_unassigned: false,
        }
    }
}

struct Kit {
    preset: DrumMap,
    enabled: bool,
    auto_detected: bool,
}

impl DrumView {
    /// Editor startup configuration: automatically detects kits from sounds and hides unassigned lanes.
    pub(super) fn active_map(&self, app: &Application) -> Option<&DrumMap> {
        let document = app.document()?;
        let folder = document.path.parent()?;
        let kit = self.mapsets.get(folder)?;
        kit.enabled.then_some(&kit.preset)
    }

    pub(super) fn for_editor() -> Self {
        Self {
            mapsets: HashMap::new(),
            auto_detect: true,
            hide_unassigned: true,
        }
    }

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
        self.mapsets.insert(folder.into(), Kit { preset, enabled: true, auto_detected: false });
        Ok(())
    }

    pub(super) fn load_builtin(
        &mut self, app: &Application, expected_map: &Path, name: &str,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?;
        let lanes = visible_lanes(app);
        let preset = read_and_bind_builtin(name, &lanes)?;
        self.mapsets.insert(folder.into(), Kit { preset, enabled: true, auto_detected: false });
        Ok(())
    }

    pub(super) fn auto_detect_now(
        &mut self, app: &Application, expected_map: &Path,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?;
        let lanes = visible_lanes(app);
        let preset = dawsu_application::drum_classify::auto_detect_drum_map(&lanes, |id| app.lane_clip(id));
        self.mapsets.insert(folder.into(), Kit { preset, enabled: true, auto_detected: true });
        Ok(())
    }

    pub(super) fn set_auto_detect(
        &mut self, app: &Application, expected_map: &Path, on: bool,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        self.auto_detect = on;
        if on {
            self.auto_detect_now(app, expected_map)?;
        } else if self.mapsets.get(&folder).is_some_and(|kit| kit.auto_detected) {
            self.load_builtin(app, expected_map, BUILTIN_DRUM_MAPS[0].0)?;
        }
        Ok(())
    }

    pub(super) fn set_hide_unassigned(
        &mut self, app: &Application, expected_map: &Path, on: bool,
    ) -> Result<(), String> {
        current_mapset(app, expected_map)?;
        self.hide_unassigned = on;
        Ok(())
    }

    pub(super) fn move_row(
        &mut self, app: &Application, expected_map: &Path, from_index: usize, to_index: usize,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?;
        let kit = self.mapsets.get_mut(folder).ok_or("Open a drum map first.")?;
        let len = kit.preset.rows.len();
        if from_index >= len || to_index >= len {
            return Err("That drum row does not exist.".into());
        }
        if from_index != to_index {
            let row = kit.preset.rows.remove(from_index);
            kit.preset.rows.insert(to_index, row);
        }
        Ok(())
    }

    pub(super) fn assign_row(
        &mut self,
        app: &Application,
        expected_map: &Path,
        row_index: usize,
        target_lane_key: Option<&str>,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?;
        let document = app.document().ok_or("Open a difficulty first.")?;
        let model = document.composer.hitsounds();
        let resolved = match target_lane_key.map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(key) => {
                if let Some(lane) = model.lanes().iter().find(|lane| {
                    lane_key(lane) == key || super::scene_lane_key(&lane.identity) == key
                }) {
                    Some(lane_key(lane))
                } else {
                    Some(key.to_owned())
                }
            }
        };
        let kit = self.mapsets.get_mut(folder).ok_or("Open a drum map first.")?;
        let row = kit.preset.rows.get_mut(row_index).ok_or("That drum row does not exist.")?;
        let previous = row.lane.clone();
        row.lane = resolved;
        if let Err(error) = kit.preset.validate() {
            kit.preset.rows[row_index].lane = previous;
            return Err(error.to_string());
        }
        Ok(())
    }

    pub(super) fn set_midi_note(
        &mut self,
        app: &Application,
        expected_map: &Path,
        row_index: usize,
        midi_note: Option<u8>,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?;
        let kit = self.mapsets.get_mut(folder).ok_or("Open a drum map first.")?;
        let row = kit.preset.rows.get_mut(row_index).ok_or("That drum row does not exist.")?;
        row.midi_note = midi_note.map(|n| n.min(127));
        Ok(())
    }

    pub(super) fn set_row_choke_group(
        &mut self,
        app: &Application,
        expected_map: &Path,
        row_index: usize,
        choke_group: Option<u8>,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?;
        let kit = self.mapsets.get_mut(folder).ok_or("Open a drum map first.")?;
        let row = kit.preset.rows.get_mut(row_index).ok_or("That drum row does not exist.")?;
        row.choke_group = choke_group.filter(|&g| g > 0);
        Ok(())
    }

    /// Collects `midi_note -> choke_group` from the active drum map's rows for live VST3 choking.
    pub(super) fn choke_groups(&self, app: &Application) -> HashMap<u8, u8> {
        let mut groups = HashMap::new();
        let Some(document) = app.document() else {
            return groups;
        };
        let Some(folder) = document.path.parent() else {
            return groups;
        };
        let Some(kit) = self.mapsets.get(folder) else {
            return groups;
        };
        for row in &kit.preset.rows {
            if let (Some(note), Some(gid)) = (row.midi_note, row.choke_group)
                && gid > 0
            {
                groups.insert(note.min(127), gid);
            }
        }
        groups
    }

    /// Computes the lane -> MIDI note routing table matching v3.1.4 `MainWindow.DrumVst.cs:35-54`:
    /// skin defaults (`hitnormal=36, hitclap=38, hitwhistle=42, hitfinish=49`) overridden by the
    /// active drum map's assigned rows (`Some(n)` maps, `None` explicitly keeps sample playback).
    pub(super) fn routed_midi_notes(
        &self,
        app: &Application,
    ) -> HashMap<dawsu_foundation::SampleIdentity, u8> {
        let mut notes = HashMap::new();
        let Some(document) = app.document() else {
            return notes;
        };
        for lane in &document.composer.lanes {
            if let Some(note) = dawsu_application::drum_vst::default_skin_midi_note(lane) {
                notes.insert(lane.clone(), note);
            }
        }
        let Some(folder) = document.path.parent() else {
            return notes;
        };
        let Some(kit) = self.mapsets.get(folder) else {
            return notes;
        };
        let lanes = visible_lanes(app);
        let mut seen = HashSet::new();
        for row in &kit.preset.rows {
            let Some(key) = row.lane.as_deref() else {
                continue;
            };
            let Some(idx) = lane_for_key(key, &lanes) else {
                continue;
            };
            let id = lanes[idx].identity.clone();
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(midi_note) = row.midi_note {
                notes.insert(id, midi_note.min(127));
            } else {
                notes.remove(&id);
            }
        }
        notes
    }

    pub(super) fn toggle(
        &mut self, app: &Application, expected_map: &Path, on: bool,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        if let Some(kit) = self.mapsets.get_mut(&folder) {
            kit.enabled = on;
        } else if on {
            if self.auto_detect {
                self.auto_detect_now(app, expected_map)?;
            } else {
                let lanes = visible_lanes(app);
                let preset = read_and_bind_builtin(BUILTIN_DRUM_MAPS[0].0, &lanes)?;
                self.mapsets.insert(folder, Kit { preset, enabled: true, auto_detected: false });
            }
        }
        Ok(())
    }

    pub(super) fn decorate(&self, app: &Application, view: &mut super::ViewSnapshot) {
        let Some(document) = app.document() else { return };
        let Some(kit) = document.path.parent().and_then(|mapset| self.mapsets.get(mapset)) else {
            return;
        };
        let composer = &document.composer;
        let model = composer.hitsounds();
        let definitions: HashMap<_, _> = model.lanes().iter().map(|lane| (&lane.identity, lane)).collect();
        let presets: Vec<&'static str> = BUILTIN_DRUM_MAPS.iter().map(|(name, _)| *name).collect();
        let visible_lanes = visible_lanes(app);
        let assignable_lanes: Vec<AssignableLane> = visible_lanes
            .iter()
            .map(|lane| AssignableLane {
                key: lane_key(lane),
                name: super::lane_name(&lane.identity),
                scene_key: super::scene_lane_key(&lane.identity),
            })
            .collect();

        let Some(scene) = view.scene.as_ref().filter(|_| kit.enabled) else {
            let setup_rows: Vec<DrumSetupRow> = kit
                .preset
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let matched = row
                        .lane
                        .as_deref()
                        .and_then(|key| lane_for_key(key, &visible_lanes))
                        .map(|i| lane_key(&visible_lanes[i]));
                    DrumSetupRow {
                        assigned: matched.is_some(),
                        choke_group: row.choke_group,
                        index,
                        lane_key: matched.or_else(|| row.lane.clone()),
                        midi_note: row.midi_note,
                        name: row.name.clone(),
                    }
                })
                .collect();
            view.drum_view = Some(DrumViewSnapshot {
                auto_detect: self.auto_detect,
                enabled: kit.enabled,
                hide_unassigned: self.hide_unassigned,
                lanes: assignable_lanes,
                name: kit.preset.name.clone(),
                presets,
                rows: setup_rows,
            });
            return;
        };
        // A replaced lane shows in the place of the lane it fills (the scene's `key`). A drum-map row
        // may name either lane; both stand for the one shown row.
        let filled = |row: usize| {
            let shown = scene.lanes.get(row)?;
            let key = (shown.key != shown.source_key).then_some(shown.key.as_str())?;
            model.lanes().iter().find(|lane| super::scene_lane_key(&lane.identity) == key)
        };
        let available: Vec<_> = scene.lane_order.iter().filter_map(|&row| {
            let identity = composer.lanes.get(row)?;
            Some((row, *definitions.get(identity)?))
        }).flat_map(|(row, lane)| std::iter::once((row, lane)).chain(filled(row).map(|slot| (row, slot)))).collect();
        let rows: HashMap<_, _> = available.iter().map(|(row, lane)| (&lane.identity, *row)).collect();
        // A row on a hidden lane stays hidden, as the lane does with the drum map off. The rows
        // look among the hidden lanes too, so a row whose own lane is hidden never names another
        // spelling of its file that is shown.
        let hidden = composer.lanes.iter().filter(|lane| model.is_lane_hidden(lane))
            .flat_map(|lane| [Some(lane), model.lane_slot(lane)].into_iter().flatten())
            .filter(|lane| !rows.contains_key(lane))
            .filter_map(|lane| definitions.get(lane).copied());
        let lanes = available.iter().map(|(_, lane)| *lane).chain(hidden).cloned().collect::<Vec<_>>();
        let entries = drum_rows(Some(&kit.preset), &lanes);
        let setup_rows: Vec<DrumSetupRow> = entries
            .iter()
            .take(kit.preset.rows.len())
            .enumerate()
            .map(|(index, entry)| match entry {
                DrumViewRow::Instrument { row, lane } if rows.contains_key(&lane.identity) => {
                    DrumSetupRow {
                        assigned: true,
                        choke_group: row.choke_group,
                        index,
                        lane_key: Some(lane_key(lane)),
                        midi_note: row.midi_note,
                        name: row.name.clone(),
                    }
                }
                DrumViewRow::Instrument { row, .. } | DrumViewRow::Unassigned(row) => {
                    DrumSetupRow {
                        assigned: false,
                        choke_group: row.choke_group,
                        index,
                        lane_key: row.lane.clone(),
                        midi_note: row.midi_note,
                        name: row.name.clone(),
                    }
                }
                DrumViewRow::Other(_) => unreachable!("first kit.preset.rows.len() entries are preset rows"),
            })
            .collect();
        // Several drum-map rows on one lane show as one row, at the first of them and under all
        // their names, so the view never shows a lane twice.
        let mut names: HashMap<usize, Vec<&str>> = HashMap::new();
        for entry in &entries {
            if let DrumViewRow::Instrument { row, lane } = entry
                && let Some(&shown) = rows.get(&lane.identity)
            {
                names.entry(shown).or_default().push(&row.name);
            }
        }
        let mut shown_rows = HashSet::new();
        let shown = entries.iter().enumerate().filter_map(|(index, entry)| match entry {
            DrumViewRow::Instrument { row: drum_row, lane } => {
                let row = *rows.get(&lane.identity)?;
                shown_rows.insert(row).then(|| DrumRowSnapshot {
                    drum_row_index: Some(index),
                    midi_note: drum_row.midi_note,
                    name: Some(names[&row].join(" / ")),
                    row: Some(row),
                })
            }
            DrumViewRow::Unassigned(row) => (!self.hide_unassigned).then(|| DrumRowSnapshot {
                drum_row_index: Some(index),
                midi_note: row.midi_note,
                name: Some(row.name.clone()),
                row: None,
            }),
            DrumViewRow::Other(lane) => {
                if self.hide_unassigned {
                    return None;
                }
                let row = *rows.get(&lane.identity)?;
                shown_rows.insert(row).then_some(DrumRowSnapshot {
                    drum_row_index: None,
                    midi_note: None,
                    name: None,
                    row: Some(row),
                })
            }
        }).collect::<Vec<_>>();
        view.drum_view = Some(DrumViewSnapshot {
            auto_detect: self.auto_detect,
            enabled: kit.enabled,
            hide_unassigned: self.hide_unassigned,
            lanes: assignable_lanes,
            name: kit.preset.name.clone(),
            presets,
            rows: setup_rows,
        });
        if let Some(scene) = view.scene.as_mut() {
            scene.drum_rows = Some(shown);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AssignableLane {
    key: String,
    name: String,
    scene_key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DrumSetupRow {
    assigned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    choke_group: Option<u8>,
    index: usize,
    lane_key: Option<String>,
    midi_note: Option<u8>,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DrumViewSnapshot {
    auto_detect: bool,
    enabled: bool,
    hide_unassigned: bool,
    lanes: Vec<AssignableLane>,
    name: String,
    presets: Vec<&'static str>,
    rows: Vec<DrumSetupRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DrumRowSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    drum_row_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    midi_note: Option<u8>,
    name: Option<String>,
    row: Option<usize>,
}

fn visible_lanes(app: &Application) -> Vec<Lane> {
    let Some(document) = app.document() else {
        return Vec::new();
    };
    let composer = &document.composer;
    let model = composer.hitsounds();
    let definitions: HashMap<_, _> = model.lanes().iter().map(|lane| (&lane.identity, lane)).collect();
    composer
        .lanes
        .iter()
        .filter(|id| !model.is_lane_hidden(id))
        .filter_map(|id| definitions.get(id).copied())
        .cloned()
        .collect()
}

fn read_and_bind_builtin(name: &str, lanes: &[Lane]) -> Result<DrumMap, String> {
    let (_, json_text) = BUILTIN_DRUM_MAPS
        .iter()
        .find(|(preset_name, _)| *preset_name == name)
        .ok_or_else(|| format!("Unknown drum map preset: {name}"))?;
    let mut preset = read_dawsudrum(json_text).map_err(|error| error.to_string())?;
    preset.audio = Vec::new();
    bind_builtin_preset(&mut preset, lanes);
    Ok(preset)
}

/// Built-in presets name standard mapset files like `BeatmapFile|drum-hitnormal.wav`. When a
/// difficulty uses the skin's default sound or a numbered variant (`Skin|drum-hitnormal`), bind it
/// automatically instead of leaving the preset's rows unassigned.
fn bind_builtin_preset(preset: &mut DrumMap, lanes: &[Lane]) {
    for row in &mut preset.rows {
        let Some(key) = row.lane.as_deref() else { continue };
        if lane_for_key(key, lanes).is_some() {
            continue;
        }
        let Some(file) = key.strip_prefix("BeatmapFile|") else { continue };
        let stem = file
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(file)
            .split('.')
            .next()
            .unwrap_or(file)
            .to_ascii_lowercase();
        let skin_target = format!("Skin|{stem}");
        if let Some(idx) = lane_for_key(&skin_target, lanes) {
            row.lane = Some(lane_key(&lanes[idx]));
            continue;
        }
        // Fallback: match the standard set-sound family (e.g. a custom numbered index `drum-hitnormal2`).
        if let Some(matched) = lanes.iter().find(|lane| {
            let name = super::lane_name(&lane.identity).to_ascii_lowercase();
            name == stem
                || name
                    .strip_prefix(&stem)
                    .is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()))
        }) {
            row.lane = Some(lane_key(matched));
        }
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
