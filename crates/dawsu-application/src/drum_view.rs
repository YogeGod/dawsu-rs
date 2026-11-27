use crate::presentation::{lane_header_rows, scene_lane_key};
use crate::{Application, Document, HsStarterMode, lane_name};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::{
    Lane,
    drums::{
        BUILTIN_DRUM_MAPS, DEFAULT_OSU_KIT_NAME, DrumMap, DrumViewRow, MAX_DAWSUDRUM_BYTES,
        drum_rows, lane_for_key, lane_key, read_dawsudrum,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DrumMapAction {
    LoadBuiltin {
        name: String,
    },
    AssignRow {
        row_index: usize,
        lane_key: Option<String>,
    },
    SetMidiNote {
        row_index: usize,
        midi_note: Option<u8>,
    },
    AutoDetect,
    SetAutoDetect {
        on: bool,
    },
    SetHideUnassigned {
        on: bool,
    },
    MoveRow {
        from_index: usize,
        to_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignableLane {
    pub key: String,
    pub name: String,
    pub scene_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrumSetupRow {
    pub assigned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choke_group: Option<u8>,
    pub index: usize,
    pub lane_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merged_lane_keys: Vec<String>,
    pub midi_note: Option<u8>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrumViewSnapshot {
    pub auto_detect: bool,
    pub enabled: bool,
    pub hide_unassigned: bool,
    pub lanes: Vec<AssignableLane>,
    pub name: String,
    pub presets: Vec<&'static str>,
    pub rows: Vec<DrumSetupRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DrumRowSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drum_row_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub midi_note: Option<u8>,
    pub name: Option<String>,
    pub row: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DrumRowGroups {
    pub groups: Vec<Vec<usize>>,
    pub group_of_lane: HashMap<usize, usize>,
}

impl DrumRowGroups {
    pub fn from_rows<'a>(snapshots: impl IntoIterator<Item = &'a DrumRowSnapshot>) -> Self {
        let mut groups = Vec::new();
        let mut group_of_lane = HashMap::new();
        for snap in snapshots {
            let members: Vec<usize> = if !snap.rows.is_empty() {
                snap.rows.clone()
            } else if let Some(row) = snap.row {
                vec![row]
            } else {
                continue;
            };
            let g = groups.len();
            for &row in &members {
                group_of_lane.entry(row).or_insert(g);
            }
            groups.push(members);
        }
        Self {
            groups,
            group_of_lane,
        }
    }

    pub fn members_for_row(&self, row: usize) -> Option<&[usize]> {
        let &g = self.group_of_lane.get(&row)?;
        Some(self.groups[g].as_slice())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoratedDrumView {
    pub drum_view: DrumViewSnapshot,
    pub drum_rows: Option<Vec<DrumRowSnapshot>>,
}

pub type SceneLaneKeys<'a> = (&'a [usize], &'a [(String, String)]);

struct Kit {
    preset: DrumMap,
    enabled: bool,
    auto_detected: bool,
}

pub struct DrumView {
    mapsets: HashMap<PathBuf, Kit>,
    /// Per-difficulty auto-activated kits (e.g. Path 2 `Default osu! kit`), kept separate from
    /// mapset-wide user presets so Path 2 never bleeds into sibling gameplay or Path 1 diffs.
    per_diff: HashMap<PathBuf, Kit>,
    last_generation: u64,
    last_history_version: u64,
    auto_detect: bool,
    hide_unassigned: bool,
    revision: u64,
}

impl Default for DrumView {
    fn default() -> Self {
        Self {
            mapsets: HashMap::new(),
            per_diff: HashMap::new(),
            last_generation: 0,
            last_history_version: 0,
            auto_detect: true,
            hide_unassigned: false,
            revision: 0,
        }
    }
}

impl DrumView {
    /// Editor startup configuration: automatically detects kits from sounds and hides unassigned lanes.
    pub fn for_editor() -> Self {
        Self {
            mapsets: HashMap::new(),
            per_diff: HashMap::new(),
            last_generation: 0,
            last_history_version: 0,
            auto_detect: true,
            hide_unassigned: true,
            revision: 0,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn kit_for(&self, document: &Document) -> Option<&Kit> {
        self.per_diff
            .get(&document.path)
            .or_else(|| document.path.parent().and_then(|folder| self.mapsets.get(folder)))
    }

    fn kit_for_mut(&mut self, expected_map: &Path, folder: &Path) -> Option<&mut Kit> {
        if self.per_diff.contains_key(expected_map) {
            self.per_diff.get_mut(expected_map)
        } else {
            self.mapsets.get_mut(folder)
        }
    }

    /// Syncs Drum VST3 routing and choke groups from the active kit onto `app`.
    pub fn sync_vst(&self, app: &mut Application) {
        app.set_drum_vst_routing(self.routed_midi_notes(app));
        app.set_drum_vst_choke_groups(self.choke_groups(app));
    }

    /// Syncs Drum VST3 routing, choke groups, and `Application::drum_row_groups` from the active kit.
    pub fn sync_app(&mut self, app: &mut Application) {
        self.sync_vst(app);
        let groups = if self.active_map(app).is_some() {
            self.build_view(app, true)
                .and_then(|view| view.drum_rows)
                .map(|rows| DrumRowGroups::from_rows(&rows))
        } else {
            None
        };
        app.set_drum_row_groups(groups);
        if let Some(doc) = app.document() {
            self.last_generation = app.document_generation();
            self.last_history_version = doc.composer.history.version();
        }
    }

    /// Runs whenever the open document generation or edit history changes: auto-activates Path 2's
    /// per-diff `Default osu! kit (12 skin sounds)` without bleeding into sibling diffs, syncs Drum
    /// VST routing and merged row groups, and bumps [`DrumView::revision`].
    pub fn sync_on_document_open(&mut self, app: &mut Application) -> bool {
        let generation = app.document_generation();
        let history_version = app
            .document()
            .map_or(0, |doc| doc.composer.history.version());
        if generation == 0 || app.document().is_none() {
            if app.drum_row_groups().is_some() {
                app.set_drum_row_groups(None);
            }
            return false;
        }
        if generation == self.last_generation {
            if history_version != self.last_history_version {
                self.last_history_version = history_version;
                if self.active_map(app).is_some() {
                    self.sync_app(app);
                    self.bump_revision();
                    return true;
                }
            }
            return false;
        }
        self.last_generation = generation;
        self.last_history_version = history_version;
        if let Some(doc) = app.document() {
            let path = doc.path.clone();
            let has_mapset_preset = doc
                .path
                .parent()
                .is_some_and(|folder| self.mapsets.contains_key(folder));
            if doc.starter_mode == Some(HsStarterMode::Default12)
                && !self.per_diff.contains_key(&path)
                && !has_mapset_preset
                && let Some((_, text)) = BUILTIN_DRUM_MAPS
                    .iter()
                    .find(|(name, _)| *name == DEFAULT_OSU_KIT_NAME)
                && let Ok(mut preset) = read_dawsudrum(text)
            {
                preset.audio = Vec::new();
                self.per_diff.insert(
                    path,
                    Kit {
                        preset,
                        enabled: true,
                        auto_detected: false,
                    },
                );
            }
            self.sync_app(app);
            self.bump_revision();
            return true;
        }
        false
    }

    pub fn active_map(&self, app: &Application) -> Option<&DrumMap> {
        let document = app.document()?;
        let kit = self.kit_for(document)?;
        kit.enabled.then_some(&kit.preset)
    }

    pub fn load(&mut self, app: &Application, expected_map: &Path, path: &Path) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        if !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dawsudrum"))
        {
            return Err("Choose a .dawsudrum file.".into());
        }
        let file = File::open(path).map_err(|error| format!("Opening drum map: {error}"))?;
        let mut limited = file.take(MAX_DAWSUDRUM_BYTES as u64 + 1);
        let mut text = String::new();
        limited
            .read_to_string(&mut text)
            .map_err(|error| format!("Reading drum map: {error}"))?;
        let mut preset = read_dawsudrum(&text).map_err(|error| error.to_string())?;
        // This view uses only the instrument order and settings. Embedded audio stays in the selected file.
        preset.audio = Vec::new();
        self.per_diff.remove(expected_map);
        self.mapsets.insert(
            folder,
            Kit {
                preset,
                enabled: true,
                auto_detected: false,
            },
        );
        self.bump_revision();
        Ok(())
    }

    pub fn load_and_sync(
        &mut self,
        app: &mut Application,
        expected_map: &Path,
        path: &Path,
    ) -> Result<(), String> {
        self.load(app, expected_map, path)?;
        self.sync_app(app);
        Ok(())
    }

    pub fn load_builtin(
        &mut self,
        app: &Application,
        expected_map: &Path,
        name: &str,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        let lanes = visible_lanes(app);
        let preset = read_and_bind_builtin(name, &lanes)?;
        self.per_diff.remove(expected_map);
        self.mapsets.insert(
            folder,
            Kit {
                preset,
                enabled: true,
                auto_detected: false,
            },
        );
        self.bump_revision();
        Ok(())
    }

    pub fn auto_detect_now(&mut self, app: &Application, expected_map: &Path) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        let lanes = visible_lanes(app);
        let hit_counts = app
            .document()
            .map(hit_counts_for)
            .unwrap_or_default();
        let preset = crate::drum_classify::auto_detect_drum_map_with_counts(
            &lanes,
            |id| app.lane_clip(id),
            &hit_counts,
        );
        self.per_diff.remove(expected_map);
        self.mapsets.insert(
            folder,
            Kit {
                preset,
                enabled: true,
                auto_detected: true,
            },
        );
        self.bump_revision();
        Ok(())
    }

    pub fn set_auto_detect(
        &mut self,
        app: &Application,
        expected_map: &Path,
        on: bool,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        self.auto_detect = on;
        if on {
            self.auto_detect_now(app, expected_map)?;
        } else if self.mapsets.get(&folder).is_some_and(|kit| kit.auto_detected) {
            self.load_builtin(app, expected_map, BUILTIN_DRUM_MAPS[0].0)?;
        } else {
            self.bump_revision();
        }
        Ok(())
    }

    pub fn set_hide_unassigned(
        &mut self,
        app: &Application,
        expected_map: &Path,
        on: bool,
    ) -> Result<(), String> {
        current_mapset(app, expected_map)?;
        self.hide_unassigned = on;
        self.bump_revision();
        Ok(())
    }

    pub fn move_row(
        &mut self,
        app: &Application,
        expected_map: &Path,
        from_index: usize,
        to_index: usize,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        let kit = self
            .kit_for_mut(expected_map, &folder)
            .ok_or("Open a drum map first.")?;
        let len = kit.preset.rows.len();
        if from_index >= len || to_index >= len {
            return Err("That drum row does not exist.".into());
        }
        if from_index != to_index {
            let row = kit.preset.rows.remove(from_index);
            kit.preset.rows.insert(to_index, row);
            self.bump_revision();
        }
        Ok(())
    }

    pub fn assign_row(
        &mut self,
        app: &Application,
        expected_map: &Path,
        row_index: usize,
        target_lane_key: Option<&str>,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        let document = app.document().ok_or("Open a difficulty first.")?;
        let model = document.composer.hitsounds();
        let resolved = match target_lane_key.map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(key) => {
                if let Some(lane) = model
                    .lanes()
                    .iter()
                    .find(|lane| lane_key(lane) == key || scene_lane_key(&lane.identity) == key)
                {
                    Some(lane_key(lane))
                } else {
                    Some(key.to_owned())
                }
            }
        };
        let kit = self
            .kit_for_mut(expected_map, &folder)
            .ok_or("Open a drum map first.")?;
        let row = kit
            .preset
            .rows
            .get_mut(row_index)
            .ok_or("That drum row does not exist.")?;
        let previous = row.lane.clone();
        row.lane = resolved;
        if let Err(error) = kit.preset.validate() {
            kit.preset.rows[row_index].lane = previous;
            return Err(error.to_string());
        }
        self.bump_revision();
        Ok(())
    }

    pub fn set_midi_note(
        &mut self,
        app: &Application,
        expected_map: &Path,
        row_index: usize,
        midi_note: Option<u8>,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        let kit = self
            .kit_for_mut(expected_map, &folder)
            .ok_or("Open a drum map first.")?;
        let row = kit
            .preset
            .rows
            .get_mut(row_index)
            .ok_or("That drum row does not exist.")?;
        row.midi_note = midi_note.map(|n| n.min(127));
        self.bump_revision();
        Ok(())
    }

    pub fn set_row_choke_group(
        &mut self,
        app: &Application,
        expected_map: &Path,
        row_index: usize,
        choke_group: Option<u8>,
    ) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        let kit = self
            .kit_for_mut(expected_map, &folder)
            .ok_or("Open a drum map first.")?;
        let row = kit
            .preset
            .rows
            .get_mut(row_index)
            .ok_or("That drum row does not exist.")?;
        row.choke_group = choke_group.filter(|&g| g > 0);
        self.bump_revision();
        Ok(())
    }

    pub fn set_row_choke_group_and_sync(
        &mut self,
        app: &mut Application,
        expected_map: &Path,
        row_index: usize,
        choke_group: Option<u8>,
    ) -> Result<(), String> {
        self.set_row_choke_group(app, expected_map, row_index, choke_group)?;
        app.set_drum_vst_choke_groups(self.choke_groups(app));
        Ok(())
    }

    /// Collects `midi_note -> choke_group` from the active drum map's rows for live VST3 choking.
    pub fn choke_groups(&self, app: &Application) -> HashMap<u8, u8> {
        let mut groups = HashMap::new();
        let Some(document) = app.document() else {
            return groups;
        };
        let Some(kit) = self.kit_for(document) else {
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

    /// Computes the lane -> MIDI note routing table.
    pub fn routed_midi_notes(&self, app: &Application) -> HashMap<SampleIdentity, u8> {
        let mut notes = HashMap::new();
        let Some(document) = app.document() else {
            return notes;
        };
        for lane in &document.composer.lanes {
            if let Some(note) = crate::drum_vst::default_skin_midi_note(lane) {
                notes.insert(lane.clone(), note);
            }
        }
        let Some(kit) = self.kit_for(document) else {
            return notes;
        };
        let lanes = visible_lanes(app);
        let model = document.composer.hitsounds();
        let hit_counts = hit_counts_for(document);
        let lane_refs: Vec<&Lane> = lanes.iter().collect();
        let (groups, group_of) = crate::sound_groups::group_lanes_by_sound(
            &lane_refs,
            &|id| app.lane_clip(id),
            &hit_counts,
            true,
        );
        let resolve_lane_idx = |key: &str| {
            lane_for_key(key, &lanes).or_else(|| {
                lanes.iter().position(|lane| {
                    model
                        .lane_slot(&lane.identity)
                        .and_then(|slot| model.lane(slot))
                        .is_some_and(|slot_lane| lane_key(slot_lane) == key)
                })
            })
        };
        let explicit_idxs: HashSet<usize> = kit
            .preset
            .rows
            .iter()
            .filter_map(|row| row.lane.as_deref().and_then(resolve_lane_idx))
            .collect();
        let mut seen = HashSet::new();
        for row in &kit.preset.rows {
            let Some(key) = row.lane.as_deref() else {
                continue;
            };
            let Some(idx) = resolve_lane_idx(key) else {
                continue;
            };
            if !seen.insert(idx) {
                continue;
            }
            let apply_note = |id: SampleIdentity, notes: &mut HashMap<SampleIdentity, u8>| {
                if let Some(midi_note) = row.midi_note {
                    notes.insert(id, midi_note.min(127));
                } else {
                    notes.remove(&id);
                }
            };
            apply_note(lanes[idx].identity.clone(), &mut notes);
            if let Some(g) = group_of[idx] {
                for &sibling_idx in &groups[g].member_indices {
                    if sibling_idx == idx || explicit_idxs.contains(&sibling_idx) {
                        continue;
                    }
                    if seen.insert(sibling_idx) {
                        apply_note(lanes[sibling_idx].identity.clone(), &mut notes);
                    }
                }
            }
        }
        notes
    }

    pub fn toggle(&mut self, app: &Application, expected_map: &Path, on: bool) -> Result<(), String> {
        let folder = current_mapset(app, expected_map)?.to_path_buf();
        if let Some(kit) = self.kit_for_mut(expected_map, &folder) {
            kit.enabled = on;
            self.bump_revision();
        } else if on {
            if self.auto_detect {
                self.auto_detect_now(app, expected_map)?;
            } else {
                let lanes = visible_lanes(app);
                let preset = read_and_bind_builtin(BUILTIN_DRUM_MAPS[0].0, &lanes)?;
                self.mapsets.insert(
                    folder,
                    Kit {
                        preset,
                        enabled: true,
                        auto_detected: false,
                    },
                );
                self.bump_revision();
            }
        }
        Ok(())
    }

    pub fn open_or_toggle_and_sync(
        &mut self,
        app: &mut Application,
        expected_map: &Path,
        preset: Option<&Path>,
        on: bool,
    ) -> Result<(), String> {
        match preset {
            Some(path) => self.load(app, expected_map, path)?,
            None => self.toggle(app, expected_map, on)?,
        }
        self.sync_app(app);
        Ok(())
    }

    pub fn apply_action_and_sync(
        &mut self,
        app: &mut Application,
        expected_map: &Path,
        action: DrumMapAction,
    ) -> Result<(), String> {
        match action {
            DrumMapAction::LoadBuiltin { name } => self.load_builtin(app, expected_map, &name)?,
            DrumMapAction::AssignRow { row_index, lane_key } => {
                self.assign_row(app, expected_map, row_index, lane_key.as_deref())?
            }
            DrumMapAction::SetMidiNote {
                row_index,
                midi_note,
            } => self.set_midi_note(app, expected_map, row_index, midi_note)?,
            DrumMapAction::AutoDetect => self.auto_detect_now(app, expected_map)?,
            DrumMapAction::SetAutoDetect { on } => self.set_auto_detect(app, expected_map, on)?,
            DrumMapAction::SetHideUnassigned { on } => {
                self.set_hide_unassigned(app, expected_map, on)?
            }
            DrumMapAction::MoveRow {
                from_index,
                to_index,
            } => self.move_row(app, expected_map, from_index, to_index)?,
        }
        self.sync_app(app);
        Ok(())
    }

    /// Computes both the DrumView metadata snapshot and (when active and `want_scene`) shown drum
    /// rows directly from `Application`, or using precomputed `scene_lanes` (`(lane_order, [(key, source_key)])`).
    pub fn build_decorated_view(
        &self,
        app: &Application,
        scene_lanes: Option<SceneLaneKeys<'_>>,
    ) -> Option<DecoratedDrumView> {
        let document = app.document()?;
        let kit = self.kit_for(document)?;
        let composer = &document.composer;
        let model = composer.hitsounds();
        let definitions: HashMap<_, _> = model
            .lanes()
            .iter()
            .map(|lane| (&lane.identity, lane))
            .collect();
        let hit_counts = hit_counts_for(document);
        let presets: Vec<&'static str> = BUILTIN_DRUM_MAPS.iter().map(|(name, _)| *name).collect();
        let visible_lanes = visible_lanes(app);
        let assignable_lanes: Vec<AssignableLane> = visible_lanes
            .iter()
            .map(|lane| AssignableLane {
                key: lane_key(lane),
                name: lane_name(&lane.identity),
                scene_key: scene_lane_key(&lane.identity),
            })
            .collect();

        let Some((lane_order, lane_keys)) = scene_lanes.filter(|_| kit.enabled) else {
            let lane_refs: Vec<&Lane> = visible_lanes.iter().collect();
            let (groups, group_of) = crate::sound_groups::group_lanes_by_sound(
                &lane_refs,
                &|id| app.lane_clip(id),
                &hit_counts,
                true,
            );
            let explicit_idxs: HashSet<usize> = kit
                .preset
                .rows
                .iter()
                .filter_map(|row| row.lane.as_deref().and_then(|key| lane_for_key(key, &visible_lanes)))
                .collect();
            let mut claimed = explicit_idxs;
            let setup_rows: Vec<DrumSetupRow> = kit
                .preset
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let matched_idx = row
                        .lane
                        .as_deref()
                        .and_then(|key| lane_for_key(key, &visible_lanes));
                    let matched = matched_idx.map(|i| lane_key(&visible_lanes[i]));
                    let mut merged_lane_keys = Vec::new();
                    if let Some(primary) = matched_idx
                        && let Some(g) = group_of[primary]
                    {
                        merged_lane_keys.push(lane_key(&visible_lanes[primary]));
                        for &sibling in &groups[g].member_indices {
                            if sibling != primary && claimed.insert(sibling) {
                                merged_lane_keys.push(lane_key(&visible_lanes[sibling]));
                            }
                        }
                    }
                    DrumSetupRow {
                        assigned: matched.is_some(),
                        choke_group: row.choke_group,
                        index,
                        lane_key: matched.or_else(|| row.lane.clone()),
                        merged_lane_keys: if merged_lane_keys.len() > 1 {
                            merged_lane_keys
                        } else {
                            Vec::new()
                        },
                        midi_note: row.midi_note,
                        name: row.name.clone(),
                    }
                })
                .collect();
            return Some(DecoratedDrumView {
                drum_view: DrumViewSnapshot {
                    auto_detect: self.auto_detect,
                    enabled: kit.enabled,
                    hide_unassigned: self.hide_unassigned,
                    lanes: assignable_lanes,
                    name: kit.preset.name.clone(),
                    presets,
                    rows: setup_rows,
                },
                drum_rows: None,
            });
        };

        let filled = |row: usize| {
            let (shown_key, shown_source_key) = lane_keys.get(row)?;
            let key = (shown_key != shown_source_key).then_some(shown_key.as_str())?;
            model
                .lanes()
                .iter()
                .find(|lane| scene_lane_key(&lane.identity) == key)
        };
        let visible_composer_rows: Vec<(usize, &Lane)> = lane_order
            .iter()
            .filter_map(|&row| {
                let identity = composer.lanes.get(row)?;
                Some((row, *definitions.get(identity)?))
            })
            .collect();
        let playing_refs: Vec<&Lane> = visible_composer_rows.iter().map(|(_, lane)| *lane).collect();
        let (audio_groups, audio_group_of) = crate::sound_groups::group_lanes_by_sound(
            &playing_refs,
            &|id| app.lane_clip(id),
            &hit_counts,
            true,
        );
        let row_to_group: HashMap<usize, usize> = visible_composer_rows
            .iter()
            .enumerate()
            .filter_map(|(i, &(row, _))| audio_group_of[i].map(|g| (row, g)))
            .collect();
        let group_rows: Vec<Vec<usize>> = audio_groups
            .iter()
            .map(|group| {
                group
                    .member_indices
                    .iter()
                    .map(|&idx| visible_composer_rows[idx].0)
                    .collect()
            })
            .collect();

        let available: Vec<_> = visible_composer_rows
            .iter()
            .copied()
            .flat_map(|(row, lane)| std::iter::once((row, lane)).chain(filled(row).map(|slot| (row, slot))))
            .collect();
        let rows: HashMap<_, _> = available.iter().map(|(row, lane)| (&lane.identity, *row)).collect();
        let hidden = composer
            .lanes
            .iter()
            .filter(|lane| model.is_lane_hidden(lane))
            .flat_map(|lane| [Some(lane), model.lane_slot(lane)].into_iter().flatten())
            .filter(|lane| !rows.contains_key(lane))
            .filter_map(|lane| definitions.get(lane).copied());
        let lanes = available
            .iter()
            .map(|(_, lane)| *lane)
            .chain(hidden)
            .cloned()
            .collect::<Vec<_>>();
        let entries = drum_rows(Some(&kit.preset), &lanes);

        let explicit_rows: HashSet<usize> = entries
            .iter()
            .filter_map(|entry| match entry {
                DrumViewRow::Instrument { lane, .. } => rows.get(&lane.identity).copied(),
                _ => None,
            })
            .collect();
        let mut claimed_for_presets = explicit_rows.clone();
        let mut preset_members_by_index: Vec<Vec<usize>> = vec![Vec::new(); kit.preset.rows.len()];
        let mut merged_by_primary: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, entry) in entries.iter().take(kit.preset.rows.len()).enumerate() {
            if let DrumViewRow::Instrument { lane, .. } = entry
                && let Some(&primary) = rows.get(&lane.identity)
            {
                let mut members = vec![primary];
                if let Some(&g) = row_to_group.get(&primary) {
                    for &sibling in &group_rows[g] {
                        if sibling != primary && claimed_for_presets.insert(sibling) {
                            members.push(sibling);
                        }
                    }
                }
                merged_by_primary.entry(primary).or_insert_with(|| members.clone());
                preset_members_by_index[index] = members;
            }
        }

        let playing_lane_key = |composer_row: usize| -> Option<String> {
            let id = composer.lanes.get(composer_row)?;
            definitions.get(id).map(|l| lane_key(l))
        };
        let setup_rows: Vec<DrumSetupRow> = entries
            .iter()
            .take(kit.preset.rows.len())
            .enumerate()
            .map(|(index, entry)| match entry {
                DrumViewRow::Instrument { row, lane } if rows.contains_key(&lane.identity) => {
                    let primary = rows[&lane.identity];
                    let merged = merged_by_primary
                        .get(&primary)
                        .filter(|m| m.len() > 1)
                        .map(|m| m.iter().filter_map(|&r| playing_lane_key(r)).collect())
                        .unwrap_or_default();
                    DrumSetupRow {
                        assigned: true,
                        choke_group: row.choke_group,
                        index,
                        lane_key: Some(lane_key(lane)),
                        merged_lane_keys: merged,
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
                        merged_lane_keys: Vec::new(),
                        midi_note: row.midi_note,
                        name: row.name.clone(),
                    }
                }
                DrumViewRow::Other(_) => {
                    unreachable!("first kit.preset.rows.len() entries are preset rows")
                }
            })
            .collect();
        let mut names: HashMap<usize, Vec<&str>> = HashMap::new();
        for entry in &entries {
            if let DrumViewRow::Instrument { row, lane } = entry
                && let Some(&shown) = rows.get(&lane.identity)
            {
                names.entry(shown).or_default().push(&row.name);
            }
        }
        let mut shown_rows = HashSet::new();
        let mut shown = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            match entry {
                DrumViewRow::Instrument { row: drum_row, lane } => {
                    let Some(&primary) = rows.get(&lane.identity) else {
                        continue;
                    };
                    if !shown_rows.insert(primary) {
                        continue;
                    }
                    let members = preset_members_by_index[index].clone();
                    for &m in &members {
                        shown_rows.insert(m);
                    }
                    shown.push(DrumRowSnapshot {
                        drum_row_index: Some(index),
                        midi_note: drum_row.midi_note,
                        name: Some(names[&primary].join(" / ")),
                        row: Some(primary),
                        rows: members,
                    });
                }
                DrumViewRow::Unassigned(row) => {
                    if !self.hide_unassigned {
                        shown.push(DrumRowSnapshot {
                            drum_row_index: Some(index),
                            midi_note: row.midi_note,
                            name: Some(row.name.clone()),
                            row: None,
                            rows: Vec::new(),
                        });
                    }
                }
                DrumViewRow::Other(lane) => {
                    let Some(&row) = rows.get(&lane.identity) else {
                        continue;
                    };
                    if shown_rows.contains(&row) {
                        continue;
                    }
                    let members: Vec<usize> = row_to_group
                        .get(&row)
                        .map(|&g| {
                            group_rows[g]
                                .iter()
                                .copied()
                                .filter(|m| !shown_rows.contains(m) && !explicit_rows.contains(m))
                                .collect::<Vec<_>>()
                        })
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| vec![row]);
                    let primary = members[0];
                    for &m in &members {
                        shown_rows.insert(m);
                    }
                    shown.push(DrumRowSnapshot {
                        drum_row_index: None,
                        midi_note: None,
                        name: None,
                        row: Some(primary),
                        rows: members,
                    });
                }
            }
        }
        Some(DecoratedDrumView {
            drum_view: DrumViewSnapshot {
                auto_detect: self.auto_detect,
                enabled: kit.enabled,
                hide_unassigned: self.hide_unassigned,
                lanes: assignable_lanes,
                name: kit.preset.name.clone(),
                presets,
                rows: setup_rows,
            },
            drum_rows: Some(shown),
        })
    }

    /// Convenience helper for UI shells (like `apps/native`) that render directly from `Application`
    /// without building a full JSON scene first.
    pub fn build_view(&self, app: &Application, include_scene: bool) -> Option<DecoratedDrumView> {
        let document = app.document()?;
        if !include_scene {
            return self.build_decorated_view(app, None);
        }
        let c = &document.composer;
        let model = c.hitsounds();
        let used: HashSet<_> = model.moments().iter().flat_map(|m| m.hits()).collect();
        let (lane_order, backing_slots) = lane_header_rows(document, &|lane| used.contains(lane));
        let lane_keys: Vec<(String, String)> = c
            .lanes
            .iter()
            .enumerate()
            .map(|(row, lane)| {
                let slot = backing_slots.get(&row).copied().unwrap_or(lane);
                (scene_lane_key(slot), scene_lane_key(lane))
            })
            .collect();
        self.build_decorated_view(app, Some((&lane_order, &lane_keys)))
    }
}

fn hit_counts_for(document: &Document) -> HashMap<SampleIdentity, usize> {
    let mut counts = HashMap::new();
    for id in document
        .composer
        .hitsounds()
        .moments()
        .iter()
        .flat_map(|m| m.hits())
    {
        *counts.entry(id.clone()).or_insert(0) += 1;
    }
    counts
}

fn visible_lanes(app: &Application) -> Vec<Lane> {
    let Some(document) = app.document() else {
        return Vec::new();
    };
    let composer = &document.composer;
    let model = composer.hitsounds();
    let definitions: HashMap<_, _> = model
        .lanes()
        .iter()
        .map(|lane| (&lane.identity, lane))
        .collect();
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

fn bind_builtin_preset(preset: &mut DrumMap, lanes: &[Lane]) {
    for row in &mut preset.rows {
        let Some(key) = row.lane.as_deref() else {
            continue;
        };
        if lane_for_key(key, lanes).is_some() {
            continue;
        }
        let Some(file) = key.strip_prefix("BeatmapFile|") else {
            continue;
        };
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
        if let Some(matched) = lanes.iter().find(|lane| {
            let name = lane_name(&lane.identity).to_ascii_lowercase();
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
    if app.is_busy() {
        return Err("Wait for the current job before changing the drum view.".into());
    }
    let document = app.document().ok_or("Open a difficulty first.")?;
    if document.path != expected_map {
        return Err("The difficulty changed while choosing a drum map.".into());
    }
    document
        .path
        .parent()
        .ok_or_else(|| "The map folder is unavailable.".into())
}
