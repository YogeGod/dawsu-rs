use crate::editor_meter::EditorTimeSignature;
use crate::{
    Application, LaneLayout, PointerPhase, controller, layers, refusals, safe_child, shortcuts,
};
use dawsu_editor::commands::{EditorScope, HelpGroup, KeyOutcome, RunOutcome};
use dawsu_editor::keys::{KeyChord, Mods};
use dawsu_editor::tempo::SnapSpacing;
use dawsu_editor::tools::{PointerButton, PointerInput, PointerSurface, ToolKind, ToolTarget};
use dawsu_editor::volume_ramp::RampShape;
use dawsu_editor_composer::HitKey;
use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, Sound};
use dawsu_hitsounding::LaneSource;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

impl Application {
    pub fn set_lane_layout(&mut self, mut layout: LaneLayout) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current edit before changing the lane layout".into());
        }
        let document = self.document.as_mut().ok_or("No difficulty is open")?;
        layout.normalise();
        document.lane_layout = Some(layout);
        Ok(())
    }

    pub fn set_time_signatures(&mut self, signatures: Vec<EditorTimeSignature>) -> Result<(), String> {
        if self.busy {
            return Err("Wait for the current job before changing time signatures".into());
        }
        if !signatures.iter().all(EditorTimeSignature::valid) {
            return Err("Each time signature needs a bar of 1 or later, 1–32 beats, and a beat unit of 1, 2, 4, 8 or 16.".into());
        }
        let document = self.document.as_mut().ok_or("Open a difficulty first")?;
        document.update_time_signatures(signatures);
        Ok(())
    }

    pub fn set_step_pads_mode(&mut self, enabled: bool) {
        if self.preferences.step_pads_mode == enabled {
            return;
        }
        self.preferences.step_pads_mode = enabled;
        self.save_preferences();
    }

    pub fn set_tool(&mut self, tool: ToolKind) {
        self.cancel_controller_stroke();
        if let Some(d) = self.document.as_mut() {
            self.tools.select_tool(tool, &mut d.composer);
        }
        self.refresh_recovery_edit();
    }
    pub fn tool(&self) -> ToolKind {
        self.tools.active()
    }
    pub fn ramp_shape(&self) -> RampShape {
        self.tools.ramp_shape
    }
    pub fn set_ramp_shape(&mut self, shape: RampShape) {
        if self.document.is_some() && !self.busy && !self.is_stroke_open() {
            self.tools.ramp_shape = shape;
        }
    }
    pub(crate) fn is_stroke_open(&self) -> bool {
        self.controller_stroke.is_some() || self.tools.is_stroke_open()
    }
    pub(crate) fn cancel_controller_stroke(&mut self) {
        if self.controller_stroke.take().is_some() && let Some(d) = self.document.as_mut() {
            d.composer.history.cancel();
        }
    }
    pub fn pointer(&mut self, phase: PointerPhase, input: PointerInput) {
        self.pointer_input(phase, input);
        self.refresh_recovery_edit();
    }
    pub(crate) fn pointer_input(&mut self, phase: PointerPhase, input: PointerInput) {
        if self.busy {
            return;
        }
        if input.click_count >= 2 {
            if matches!(phase, PointerPhase::Down)
                && input.button == PointerButton::Left
                && input.surface == PointerSurface::Grid
                && input.mods == Mods::NONE
                && self.tools.active() == ToolKind::Select
                && !self.is_stroke_open()
            {
                self.select_double_click(input);
            }
            return;
        }
        let Some(d) = self.document.as_mut() else { return };
        if let Some(stroke) = &mut self.controller_stroke {
            match phase {
                PointerPhase::Down => {}
                PointerPhase::Move => stroke.apply(&mut d.composer, input),
                PointerPhase::Up => {
                    stroke.finish(&mut d.composer, input);
                    self.controller_stroke = None;
                }
                PointerPhase::Cancel => self.cancel_controller_stroke(),
            }
            return;
        }
        if matches!(phase, PointerPhase::Down) {
            if input.surface == PointerSurface::Grid {
                self.active_lane = input.row.max(0) as usize;
            } else if !input.mods.shift() && self.tools.active() != ToolKind::Ramp {
                if !self.tools.is_stroke_open() {
                    self.controller_stroke = controller::ControllerStroke::start(&mut d.composer, input, self.zoom);
                }
                return;
            }
        }
        match phase {
            PointerPhase::Down => self.tools.down(&mut d.composer, input),
            PointerPhase::Move => self.tools.moved(&mut d.composer, input),
            PointerPhase::Up => self.tools.up(&mut d.composer, input),
            PointerPhase::Cancel => self.tools.cancel(&mut d.composer),
        }
    }
    pub(crate) fn select_double_click(&mut self, input: PointerInput) {
        let Some(document) = self.document.as_mut() else { return };
        let composer = &mut document.composer;
        let Ok(row) = usize::try_from(input.row) else { return };
        if row >= composer.lanes.len() {
            return;
        }
        self.active_lane = row;
        if let Some(hit) = composer.hit_test(&input) {
            let moment = &composer.hitsounds().moments()[hit.moment];
            let time = moment.time().ms();
            let gain = moment.hit_volume(&hit.lane) as f32 / 100.0;
            let lane = hit.lane.clone();
            composer.selected.replace([hit]);
            self.seek(time);
            self.audition_lane(&lane, gain);
        } else {
            self.tools.down(composer, input);
            self.tools.up(composer, input);
        }
    }

    pub fn go_to_history(&mut self, position: usize) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        let slot = self.active_lane_slot();
        if let Some(document) = self.document.as_mut() {
            document.composer.history.go_to(position);
            document.composer.selected.clear();
        }
        self.restore_active_lane(slot);
        self.refresh_recovery_edit();
    }

    pub(crate) fn active_lane_slot(&self) -> Option<SampleIdentity> {
        let composer = &self.document()?.composer;
        let lane = composer.lanes.get(self.active_lane)?;
        Some(composer.hitsounds().lane_slot(lane).unwrap_or(lane).clone())
    }

    pub(crate) fn restore_active_lane(&mut self, slot: Option<SampleIdentity>) {
        let Some(document) = &self.document else { return };
        let composer = &document.composer;
        let hits = composer.hitsounds();
        let visible = |lane: &SampleIdentity| {
            !hits.is_lane_hidden(lane) && !hits.lane_slots().any(|(_, logical)| logical == lane)
        };
        let wanted = slot.as_ref().map(|slot| {
            hits.lane_slots().find_map(|(source, logical)| (logical == slot).then_some(source)).unwrap_or(slot)
        });
        self.active_lane = wanted.and_then(|wanted| {
            composer.lanes.iter().position(|lane| lane == wanted && visible(lane))
        }).or_else(|| {
            composer.lanes.iter().enumerate().filter(|(_, lane)| visible(lane))
                .min_by_key(|(row, _)| row.abs_diff(self.active_lane)).map(|(row, _)| row)
        }).unwrap_or(composer.lanes.len());
    }

    pub fn set_view_start(&mut self, ms: f64) {
        if let Some(d) = self.document.as_mut()
            && ms.is_finite() {
                d.composer.view_start = ms;
            }
    }
    pub fn set_snap(&mut self, enabled: bool, division: u32) {
        if let Some(d) = self.document.as_mut() {
            d.composer.snap_enabled = enabled;
            d.composer.spacing = SnapSpacing::fraction(division.max(1)).unwrap();
        }
    }
    pub fn set_normal_protection(&mut self, on: bool) {
        self.preferences.protect_hitnormals = on;
        if let Some(d) = self.document.as_mut() {
            d.composer.protect_normals = on;
        }
        self.save_preferences();
    }
    pub fn set_snap_options(
        &mut self,
        enabled: bool,
        beats: u32,
        divisions: u32,
        mode: dawsu_editor::tempo::SnapMode,
    ) {
        if let Some(d) = self.document.as_mut() {
            match SnapSpacing::new(beats, divisions) {
                Ok(spacing) => {
                    d.composer.snap_enabled = enabled;
                    d.composer.spacing = spacing;
                    d.composer.snap_mode = mode;
                }
                Err(error) => self.error = Some(refusals::snap(beats, divisions, error)),
            }
        }
    }
    pub fn set_insert_volume(&mut self, v: i32) {
        if let Some(d) = self.document.as_mut() {
            // New hits are never drawn under 5% (see `Composer::volume_for_lanes`).
            d.composer.insert_volume = v.clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100);
        }
    }
    /// Paints hits onto `row` at every snapped step between `start_ms` and `end_ms`
    /// that does not already have a hit on that lane, respecting `insert_volume`,
    /// snap mode, and beat divisor, recorded as a single Undo step.
    pub fn paint_hits_on_lane(&mut self, row: usize, start_ms: f64, end_ms: f64) -> usize {
        if self.busy || self.is_stroke_open() {
            return 0;
        }
        let Some(d) = self.document.as_mut() else {
            return 0;
        };
        let placed = d.composer.paint_hits_on_lane(row, start_ms, end_ms);
        if placed > 0 {
            self.active_lane = row;
            self.refresh_recovery_edit();
        }
        placed
    }

    /// Toggles a hit on `row` at `ms` for beat step-sequencer pads regardless of active tool.
    pub fn toggle_step_hit(&mut self, row: usize, ms: f64) -> bool {
        if self.busy || self.is_stroke_open() {
            return false;
        }
        let Some(d) = self.document.as_mut() else {
            return false;
        };
        if !d.composer.toggle_step_hit(row, ms) {
            return false;
        }
        self.active_lane = row;
        self.record_shared_base_overrides_if_needed();
        self.sync_audio();
        self.refresh_recovery_edit();
        true
    }
    pub fn set_selection_lane_scope(&mut self, lanes: Option<BTreeSet<SampleIdentity>>) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current edit before changing volume scope".into());
        }
        self.document.as_ref().ok_or("Open a difficulty first")?;
        self.selection_lane_scope = lanes;
        Ok(())
    }

    pub fn selection_volume(&mut self, v: i32, relative: bool) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        if let Some(d) = self.document.as_mut() {
            d.composer.volume_for_lanes(v, relative, self.selection_lane_scope.as_ref());
        }
        self.refresh_recovery_edit();
    }

    pub fn set_moment_volume_at(&mut self, ms: f64, value: i32) -> bool {
        if self.busy || self.is_stroke_open() {
            return false;
        }
        let changed = self.document.as_mut().is_some_and(|d| {
            d.composer
                .set_moment_volume_at(ms, value, self.selection_lane_scope.as_ref())
        });
        if changed {
            self.refresh_recovery_edit();
        }
        changed
    }

    pub fn set_selection_sample_set(&mut self, set: SampleSet) -> bool {
        if self.busy || self.is_stroke_open() {
            return false;
        }
        let changed = self
            .document
            .as_mut()
            .is_some_and(|d| d.composer.set_selection_sample_set(set));
        if changed {
            self.refresh_recovery_edit();
        }
        changed
    }

    pub fn toggle_selection_addition(&mut self, addition: Hitsound) -> bool {
        if self.busy || self.is_stroke_open() {
            return false;
        }
        let changed = self
            .document
            .as_mut()
            .is_some_and(|d| d.composer.toggle_selection_addition(addition));
        if changed {
            self.refresh_recovery_edit();
        }
        changed
    }

    pub fn audition_selection(&mut self) {
        let hits: Vec<(SampleIdentity, f32)> = {
            let Some(d) = self.document.as_ref() else {
                return;
            };
            let model = d.composer.hitsounds();
            let Some(first_moment) = d
                .composer
                .selected
                .items()
                .iter()
                .filter_map(|key| {
                    model
                        .moments()
                        .get(key.moment)?
                        .has_hit(&key.lane)
                        .then_some(key.moment)
                })
                .min()
            else {
                return;
            };
            let moment = &model.moments()[first_moment];
            moment
                .hits()
                .map(|lane| (lane.clone(), moment.hit_volume(lane) as f32 / 100.0))
                .collect()
        };
        for (lane, gain) in hits {
            self.audition_lane(&lane, gain);
        }
    }

    pub fn auto_fix_problem(&mut self, time_ms: f64) -> bool {
        if self.busy || self.is_stroke_open() {
            return false;
        }
        let outcome = self
            .document
            .as_mut()
            .and_then(|d| d.composer.auto_fix_at(time_ms));
        if let Some(msg) = outcome {
            self.status = msg;
            self.refresh_recovery_edit();
            true
        } else {
            false
        }
    }
    pub fn set_hs_mode(&mut self, hs: bool) {
        if self.busy {
            return;
        }
        if let Some(d) = self.document.as_mut() {
            d.hs_mode = hs;
        }
        self.refresh_recovery_edit();
    }
    /// Selects the hits attached to one waveform object or one of its nodes.
    pub fn select_waveform_object(&mut self, map: &Path, object: usize, time: f64, node: bool, preview: bool) {
        if self.busy || self.is_stroke_open() || !time.is_finite() {
            return;
        }
        let Some(document) = self.document.as_mut().filter(|document| document.path == map) else {
            return;
        };
        let Some(object_line) = document.file.hit_objects().get(object).map(|object| object.line()) else {
            return;
        };
        let composer = &mut document.composer;
        let mut hits = Vec::new();
        let mut found = false;
        for (index, moment) in composer.hitsounds().moments().iter().enumerate() {
            let object = match moment.place() {
                dawsu_hitsounding::Place::Circle(object)
                | dawsu_hitsounding::Place::SliderNode { object, .. }
                | dawsu_hitsounding::Place::SpinnerEnd(object) => object,
                dawsu_hitsounding::Place::NoObject => continue,
            };
            if object.line() != object_line || (node && moment.time().ms() != time) {
                continue;
            }
            found = true;
            hits.extend(moment.hits().map(|lane| {
                let gain = moment.hit_volume(lane) as f32 / 100.0;
                (HitKey { moment: index, lane: lane.clone() }, gain)
            }));
            if node {
                break;
            }
        }
        if !found {
            return;
        }
        composer.selected.replace(hits.iter().map(|(hit, _)| hit.clone()));
        self.seek(time);
        if preview {
            for (hit, gain) in hits {
                self.audition_lane(&hit.lane, gain);
            }
        }
    }

    pub fn select_hit(&mut self, key: HitKey, add: bool) {
        if let Some(d) = self.document.as_mut() {
            if add {
                d.composer.selected.add(key);
            } else {
                d.composer.selected.replace([key]);
            }
        }
    }

    pub fn command(&mut self, id: &str) {
        let catalogue = self.catalogue.clone();
        if let RunOutcome::Failed { message, .. } = catalogue.run(self, id, EditorScope::Hitsound) {
            self.error = Some(message);
        }
        self.refresh_recovery_edit();
    }
    pub fn key(&mut self, chord: KeyChord) {
        let catalogue = self.catalogue.clone();
        let keymap = self.keymap.clone();
        if let KeyOutcome::Command {
            outcome: RunOutcome::Failed { message, .. },
            ..
        } = catalogue.handle_key(self, &keymap, chord, EditorScope::Hitsound)
        {
            self.error = Some(message);
        }
        self.refresh_recovery_edit();
    }
    pub fn key_with_lane_view(&mut self, chord: KeyChord, path: PathBuf, rows: Vec<usize>, span: f64) {
        self.key_lane_view = Some((path, rows, span));
        self.key(chord);
        self.key_lane_view = None;
    }

    pub fn help(&self) -> Vec<HelpGroup> {
        self.catalogue.help(&self.keymap)
    }
    pub fn choose_key_preset(&mut self, preset: &str) {
        self.preferences.key_preset = preset.into();
        self.keymap = shortcuts::keymap(self.catalogue.definitions(), &self.preferences);
        self.first_launch = false;
        self.save_preferences();
    }
    /// Replaces the active lane's editable hits, retaining its logical slot.
    pub fn replace_lane_sample(
        &mut self,
        expected_map: &Path,
        expected_lane: &SampleIdentity,
        source: &Path,
    ) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before replacing a sample.".into());
        }
        let d = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let current = d.composer.lanes.get(self.active_lane);
        if d.path != expected_map || current != Some(expected_lane) {
            return Err("The active lane changed while choosing a sample.".into());
        }
        if matches!(expected_lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. }) {
            return Err("Slider body samples are read-only here.".into());
        }

        let root = d.path.parent().ok_or("Missing mapset folder")?;
        let source = source.canonicalize().map_err(|e| format!("Opening {}: {e}", source.display()))?;
        let extension = source.extension().and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase).filter(|ext| matches!(ext.as_str(), "wav" | "ogg" | "mp3"))
            .ok_or("Choose a WAV, OGG, or MP3 sample.")?;
        let hits = d.composer.hitsounds();
        let logical = hits.lane_slot(expected_lane).unwrap_or(expected_lane);
        let file_path = |lane: &SampleIdentity| match &hits.lane(lane)?.source {
            LaneSource::File(name) => safe_child(root, name).ok(),
            LaneSource::Skin => None,
        };
        if file_path(expected_lane).as_ref() == Some(&source) {
            return Ok(());
        }
        let existing = source.starts_with(root).then(|| hits.lanes().iter().find(|candidate| {
            let lane = &candidate.identity;
            let compatible = match (expected_lane, lane) {
                (SampleIdentity::Standard { set, sound, .. }, SampleIdentity::Standard { set: other_set, sound: other_sound, .. }) =>
                    set == other_set && sound == other_sound,
                (SampleIdentity::File(_), SampleIdentity::File(_)) => true,
                _ => false,
            };
            compatible && self.clip_for_lane(lane).is_some()
                && (lane == logical || hits.lane_slot(lane).is_none())
                && !hits.lane_slots().any(|(_, slot)| slot == lane)
                && !hits.moments().iter().any(|moment| moment.hits().any(|used| used == lane))
                && file_path(lane).as_ref() == Some(&source)
        })).flatten().map(|lane| lane.identity.clone());
        let replacement = if let Some(lane) = existing {
            lane
        } else {
            let bytes = std::fs::read(&source).map_err(|e| format!("Reading sample: {e}"))?;
            let clip = dawsu_audio::decode(&source)?;
            if !std::fs::read(&source).is_ok_and(|current| current == bytes) {
                return Err("The sample changed while it was being read. Choose it again.".into());
            }
            let allow_index_one = matches!(
                expected_lane,
                SampleIdentity::Standard { index, .. } if index.get() == 0
            );
            let name = layers::create_backing(d, expected_lane, &extension, &bytes, allow_index_one)?;
            let lane = dawsu_hitsounding::rules::read_sample_file_name(&name)
                .unwrap_or_else(|| SampleIdentity::File(name.clone()));
            self.document.as_mut().expect("open document").pending.add(&name, &bytes)?;
            self.register_samples(vec![(name, clip)], false);
            lane
        };
        let composer = &mut self.document.as_mut().expect("open document").composer;
        let selected = composer.selected.items().iter().cloned().map(|mut hit| {
            if hit.lane == *expected_lane {
                hit.lane = replacement.clone();
            }
            hit
        }).collect::<Vec<_>>();
        composer.apply("Replace lane sample", |hits| hits.replace_lane(expected_lane, replacement.clone()));
        composer.selected.replace(selected);
        self.active_lane = composer.lanes.iter().position(|lane| lane == &replacement)
            .expect("the backing lane is registered before replacing hits");
        self.status = format!("Assigned {}", source.file_name().unwrap_or_default().to_string_lossy());
        self.sync_audio();
        Ok(())
    }

    pub(crate) fn add_numbered_layer(&mut self) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before adding a sample layer.".into());
        }
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let family = document.composer.lanes.get(self.active_lane).ok_or("Choose a sample family.")?;
        let (name, bytes, clip) = layers::create(document, &self.preferences.default_samples, family)?;
        let lane = dawsu_hitsounding::rules::read_sample_file_name(&name)
            .expect("numbered layer names are constructed from sample identities");
        self.document.as_mut().expect("open document").pending.add(&name, &bytes)?;
        self.register_samples(vec![(name.clone(), clip)], false);
        if let Some(document) = &self.document
            && let Some(row) = document.composer.lanes.iter().position(|identity| *identity == lane)
        {
            self.active_lane = row;
        }
        self.status = format!("Added {name} with the default family sound.");
        Ok(())
    }
}
