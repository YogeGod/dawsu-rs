use crate::editor_meter::EditorTimeSignature;
use crate::{
    Application, LaneLayout, PointerPhase, controller, layers, refusals, safe_child, shortcuts,
};
use dawsu_editor::Selection;
use dawsu_editor::commands::{EditorScope, HelpGroup, KeyOutcome, RunOutcome};
use dawsu_editor::keys::{KeyChord, Mods};
use dawsu_editor::tempo::SnapSpacing;
use dawsu_editor::tools::{
    PointerButton, PointerInput, PointerSurface, TimeRange, ToolKind, ToolPreview, ToolTarget,
};
use dawsu_editor::volume_ramp::RampShape;
use dawsu_editor_composer::{Composer, HitKey};
use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_hitsounding::LaneSource;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

struct SamplesRowTarget<'a> {
    composer: &'a mut Composer,
    vis: Vec<usize>,
    row_to_coord: HashMap<usize, usize>,
}

impl<'a> SamplesRowTarget<'a> {
    fn new(composer: &'a mut Composer, visible_rows: &[usize]) -> Option<Self> {
        let mut seen = HashSet::new();
        let vis: Vec<usize> = visible_rows
            .iter()
            .copied()
            .filter(|&r| r < composer.lanes.len() && seen.insert(r))
            .collect();
        if vis.is_empty() {
            return None;
        }
        let row_to_coord = vis
            .iter()
            .enumerate()
            .map(|(coord, &raw)| (raw, coord))
            .collect();
        Some(Self {
            composer,
            vis,
            row_to_coord,
        })
    }

    fn translate_pointer(&self, mut input: PointerInput) -> PointerInput {
        if input.surface == PointerSurface::Grid {
            input.row = usize::try_from(input.row)
                .ok()
                .and_then(|r| self.row_to_coord.get(&r).copied())
                .map_or(-1, |c| c as i32);
        }
        input
    }
}

impl ToolTarget for SamplesRowTarget<'_> {
    type Item = HitKey;

    fn hit_test(&self, at: &PointerInput) -> Option<HitKey> {
        let coord = usize::try_from(at.row).ok()?;
        let raw = *self.vis.get(coord)?;
        let mut mapped = *at;
        mapped.row = raw as i32;
        ToolTarget::hit_test(self.composer, &mapped)
    }

    fn in_range(&self, from: SongTime, to: SongTime, first: i32, last: i32) -> Vec<HitKey> {
        let lo = from.ms().min(to.ms());
        let hi = from.ms().max(to.ms());
        let mut keys = Vec::new();
        for (i, m) in self.composer.hitsounds().moments().iter().enumerate() {
            if m.time().ms() < lo || m.time().ms() > hi {
                continue;
            }
            for lane in m.hits() {
                if first == i32::MIN && last == i32::MAX {
                    keys.push(HitKey {
                        moment: i,
                        lane: lane.clone(),
                    });
                } else if let Some(raw) = self.composer.lanes.iter().position(|l| l == lane)
                    && let Some(&coord) = self.row_to_coord.get(&raw)
                    && (first..=last).contains(&(coord as i32))
                {
                    keys.push(HitKey {
                        moment: i,
                        lane: lane.clone(),
                    });
                }
            }
        }
        keys
    }

    fn item_time(&self, item: &HitKey) -> SongTime {
        ToolTarget::item_time(self.composer, item)
    }

    fn selection(&mut self) -> &mut Selection<HitKey> {
        ToolTarget::selection(self.composer)
    }

    fn snap(&self, time: SongTime) -> SongTime {
        ToolTarget::snap(self.composer, time)
    }

    fn snap_steps(&self, from: SongTime, to: SongTime) -> Vec<SongTime> {
        ToolTarget::snap_steps(self.composer, from, to)
    }

    fn snap_offset(&self, from: SongTime, raw_ms: f64) -> f64 {
        ToolTarget::snap_offset(self.composer, from, raw_ms)
    }

    fn snap_offset_relative(&self, from: SongTime, raw_ms: f64) -> f64 {
        ToolTarget::snap_offset_relative(self.composer, from, raw_ms)
    }

    fn begin(&mut self, label: &str) {
        ToolTarget::begin(self.composer, label);
    }

    fn commit(&mut self) {
        ToolTarget::commit(self.composer);
    }

    fn cancel(&mut self) {
        ToolTarget::cancel(self.composer);
    }

    fn insert(&mut self, at: SongTime, row: i32) -> Option<HitKey> {
        let coord = usize::try_from(row).ok()?;
        let raw = *self.vis.get(coord)?;
        ToolTarget::insert(self.composer, at, raw as i32)
    }

    fn delete(&mut self, items: &[HitKey]) {
        ToolTarget::delete(self.composer, items);
    }

    fn move_by(&mut self, items: &[HitKey], delta_ms: f64, delta_rows: i32) {
        self.composer
            .move_by_in_view(items, delta_ms, delta_rows, Some(&self.vis));
    }

    fn set_values(&mut self, values: &[(HitKey, i32)]) {
        ToolTarget::set_values(self.composer, values);
    }

    fn set_hit_values(&mut self, values: &[(HitKey, i32)]) {
        ToolTarget::set_hit_values(self.composer, values);
    }

    fn insert_value(&self) -> i32 {
        ToolTarget::insert_value(self.composer)
    }

    fn toggle_listen_mute(&mut self, item: &HitKey) {
        ToolTarget::toggle_listen_mute(self.composer, item);
    }

    fn audition(&mut self, item: &HitKey) {
        ToolTarget::audition(self.composer, item);
    }

    fn pan(&mut self, delta_ms: f64, delta_rows: f64) {
        ToolTarget::pan(self.composer, delta_ms, delta_rows);
    }

    fn set_range(&mut self, range: Option<TimeRange>) {
        ToolTarget::set_range(self.composer, range);
    }

    fn open_tool_picker(&mut self, at: &PointerInput) {
        ToolTarget::open_tool_picker(self.composer, at);
    }

    fn show_preview(&mut self, preview: ToolPreview) {
        ToolTarget::show_preview(self.composer, preview);
    }
}

fn resolve_volume_lanes(
    composer: &dawsu_editor_composer::Composer,
    rows: &[usize],
) -> BTreeSet<SampleIdentity> {
    let hits = composer.hitsounds();
    rows.iter()
        .filter_map(|&row| composer.lanes.get(row))
        .map(|lane| hits.lane_slot(lane).unwrap_or(lane).clone())
        .collect()
}

impl Application {
    pub fn set_lane_layout(&mut self, mut layout: LaneLayout) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current edit before changing the lane layout".into());
        }
        let document = self.document.as_mut().ok_or("No difficulty is open")?;
        layout.normalise();
        document.lane_layout = Some(layout);
        self.bump_view_revision();
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
        self.bump_view_revision();
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
        self.drum_move_stroke = None;
        self.cancel_controller_stroke();
        if let Some(d) = self.document.as_mut() {
            self.tools.select_tool(tool, &mut d.composer);
        }
        self.refresh_recovery_edit();
        self.bump_view_revision();
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
            self.bump_view_revision();
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
        self.pointer_scoped(phase, input, None);
    }

    pub fn pointer_scoped(&mut self, phase: PointerPhase, input: PointerInput, volume_lanes: Option<&[usize]>) {
        self.pointer_in_view(phase, input, volume_lanes, None);
    }

    pub fn pointer_in_view(
        &mut self,
        phase: PointerPhase,
        input: PointerInput,
        volume_lanes: Option<&[usize]>,
        visible_rows: Option<&[usize]>,
    ) {
        self.pointer_input(phase, input, volume_lanes, visible_rows);
        self.refresh_recovery_edit();
        self.bump_view_revision();
    }
    /// Replaces or extends selection with every hit in `from_ms..=to_ms` on the explicit
    /// visible data-lane indices in `rows`, so visual folder/custom lane ordering matches
    /// the on-screen marquee box.
    pub fn select_box(
        &mut self,
        from_ms: f64,
        to_ms: f64,
        rows: &[usize],
        additive: bool,
        active_lane: Option<usize>,
    ) {
        if self.busy {
            return;
        }
        self.cancel_controller_stroke();
        let Some(d) = self.document.as_mut() else {
            return;
        };
        if self.tools.is_stroke_open() {
            self.drum_move_stroke = None;
            self.tools.cancel(&mut d.composer);
        }
        if let Some(lane) = active_lane.filter(|&row| row < d.composer.lanes.len()) {
            self.active_lane = lane;
        }
        let (lo, hi) = if to_ms < from_ms { (to_ms, from_ms) } else { (from_ms, to_ms) };
        let lane_set: std::collections::HashSet<&SampleIdentity> = rows
            .iter()
            .filter_map(|&row| d.composer.lanes.get(row))
            .collect();
        let mut inside = Vec::new();
        if !lane_set.is_empty() && lo.is_finite() && hi.is_finite() {
            for (i, moment) in d.composer.hitsounds().moments().iter().enumerate() {
                let t = moment.time().ms();
                if t < lo || t > hi {
                    continue;
                }
                for lane in moment.hits() {
                    if lane_set.contains(lane) {
                        inside.push(HitKey {
                            moment: i,
                            lane: lane.clone(),
                        });
                    }
                }
            }
        }
        if additive {
            for item in inside {
                d.composer.selected.add(item);
            }
        } else {
            d.composer.selected.replace(inside);
        }
        self.bump_view_revision();
    }
    pub(crate) fn pointer_input(
        &mut self,
        phase: PointerPhase,
        input: PointerInput,
        volume_lanes: Option<&[usize]>,
        visible_rows: Option<&[usize]>,
    ) {
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
                self.select_double_click(input, visible_rows);
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
                let raw_row = input.row.max(0) as usize;
                self.active_lane = self
                    .drum_row_groups
                    .as_ref()
                    .and_then(|g| g.members_for_row(raw_row))
                    .and_then(|m| m.first().copied())
                    .unwrap_or(raw_row);
            } else if !input.mods.shift() && self.tools.active() != ToolKind::Ramp {
                if !self.tools.is_stroke_open() {
                    let scope = volume_lanes
                        .filter(|rows| !rows.is_empty())
                        .map(|rows| resolve_volume_lanes(&d.composer, rows))
                        .or_else(|| self.selection_volume_scope.lane_scope().cloned());
                    let active_slot = d.composer.lanes.get(self.active_lane).map(|lane| {
                        d.composer.hitsounds().lane_slot(lane).unwrap_or(lane).clone()
                    });
                    self.controller_stroke = controller::ControllerStroke::start(
                        &mut d.composer,
                        input,
                        self.zoom,
                        scope.as_ref(),
                        active_slot.as_ref(),
                    );
                }
                return;
            }
        }
        if let Some(groups) = self.drum_row_groups.as_ref() {
            let ctx = crate::drum_editing::DrumGroupContext::new(
                groups,
                &d.composer.lanes,
                visible_rows,
            );
            let translated = ctx.translate_pointer(input);
            let mut target = crate::drum_editing::DrumRowTarget {
                composer: &mut d.composer,
                file: &d.file,
                ctx,
                move_stroke: &mut self.drum_move_stroke,
            };
            match phase {
                PointerPhase::Down => self.tools.down(&mut target, translated),
                PointerPhase::Move => self.tools.moved(&mut target, translated),
                PointerPhase::Up => self.tools.up(&mut target, translated),
                PointerPhase::Cancel => self.tools.cancel(&mut target),
            }
        } else if let Some(mut target) = visible_rows.and_then(|rows| SamplesRowTarget::new(&mut d.composer, rows)) {
            let translated = target.translate_pointer(input);
            match phase {
                PointerPhase::Down => self.tools.down(&mut target, translated),
                PointerPhase::Move => self.tools.moved(&mut target, translated),
                PointerPhase::Up => self.tools.up(&mut target, translated),
                PointerPhase::Cancel => self.tools.cancel(&mut target),
            }
        } else {
            match phase {
                PointerPhase::Down => self.tools.down(&mut d.composer, input),
                PointerPhase::Move => self.tools.moved(&mut d.composer, input),
                PointerPhase::Up => self.tools.up(&mut d.composer, input),
                PointerPhase::Cancel => self.tools.cancel(&mut d.composer),
            }
        }
    }
    pub(crate) fn select_double_click(&mut self, input: PointerInput, visible_rows: Option<&[usize]>) {
        let Some(document) = self.document.as_mut() else { return };
        let Ok(row) = usize::try_from(input.row) else { return };
        if row >= document.composer.lanes.len() {
            return;
        }
        let primary_row = self
            .drum_row_groups
            .as_ref()
            .and_then(|g| g.members_for_row(row))
            .and_then(|m| m.first().copied())
            .unwrap_or(row);
        self.active_lane = primary_row;

        if let Some(groups) = self.drum_row_groups.as_ref() {
            let ctx = crate::drum_editing::DrumGroupContext::new(
                groups,
                &document.composer.lanes,
                visible_rows,
            );
            let translated = ctx.translate_pointer(input);
            let mut target = crate::drum_editing::DrumRowTarget {
                composer: &mut document.composer,
                file: &document.file,
                ctx,
                move_stroke: &mut self.drum_move_stroke,
            };
            if let Some(hit) = target.hit_test(&translated) {
                let moment = &document.composer.hitsounds().moments()[hit.moment];
                let time = moment.time().ms();
                let gain = moment.hit_volume(&hit.lane) as f32 / 100.0;
                let lane = hit.lane.clone();
                document.composer.selected.replace([hit]);
                self.seek(time);
                self.audition_lane(&lane, gain);
            } else {
                self.tools.down(&mut target, translated);
                self.tools.up(&mut target, translated);
            }
        } else {
            let composer = &mut document.composer;
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
        self.bump_view_revision();
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
        self.bump_view_revision();
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
        self.bump_view_revision();
    }

    pub fn set_view_start(&mut self, ms: f64) {
        if let Some(d) = self.document.as_mut()
            && ms.is_finite() {
                d.composer.view_start = ms;
                self.bump_view_revision();
            }
    }
    pub fn set_snap(&mut self, enabled: bool, division: u32) {
        if let Some(d) = self.document.as_mut() {
            d.composer.snap_enabled = enabled;
            d.composer.spacing = SnapSpacing::fraction(division.max(1)).unwrap();
            self.bump_view_revision();
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
            self.bump_view_revision();
        }
    }
    pub fn set_insert_volume(&mut self, v: i32) {
        if let Some(d) = self.document.as_mut() {
            // New hits are never drawn under 5% (see `Composer::volume_for_lanes`).
            d.composer.insert_volume = v.clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100);
            self.bump_view_revision();
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
        let (target_row, placed) = if let Some(members) = self
            .drum_row_groups
            .as_ref()
            .and_then(|g| g.members_for_row(row))
            .filter(|m| m.len() > 1)
        {
            (
                members[0],
                crate::drum_editing::paint_hits_on_group(
                    &mut d.composer,
                    &d.file,
                    members,
                    start_ms,
                    end_ms,
                ),
            )
        } else {
            (row, d.composer.paint_hits_on_lane(row, start_ms, end_ms))
        };
        if placed > 0 {
            self.active_lane = target_row;
            self.refresh_recovery_edit();
            self.bump_view_revision();
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
        let (target_row, changed) = if let Some(members) = self
            .drum_row_groups
            .as_ref()
            .and_then(|g| g.members_for_row(row))
            .filter(|m| m.len() > 1)
        {
            (
                members[0],
                crate::drum_editing::toggle_step_hit_group(&mut d.composer, &d.file, members, ms),
            )
        } else {
            (row, d.composer.toggle_step_hit(row, ms))
        };
        if !changed {
            return false;
        }
        self.active_lane = target_row;
        self.record_shared_base_overrides_if_needed();
        self.sync_audio();
        self.refresh_recovery_edit();
        self.bump_view_revision();
        true
    }
    pub fn selection_volume_scope_str(&self) -> &'static str {
        self.selection_volume_scope.as_str()
    }

    pub fn set_selection_volume_scope(&mut self, scope: crate::SelectionVolumeScope) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current edit before changing volume scope".into());
        }
        self.document.as_ref().ok_or("Open a difficulty first")?;
        self.selection_volume_scope = scope;
        self.bump_view_revision();
        Ok(())
    }

    pub fn set_selection_lane_scope(&mut self, lanes: Option<BTreeSet<SampleIdentity>>) -> Result<(), String> {
        self.set_selection_volume_scope(match lanes {
            Some(lanes) => crate::SelectionVolumeScope::SelectedLanes(lanes),
            None => crate::SelectionVolumeScope::AllLanes,
        })
    }

    pub fn selection_volume(&mut self, v: i32, relative: bool) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        let scope = self.selection_volume_scope.clone();
        let changed = self.document.as_mut().is_some_and(|d| match &scope {
            crate::SelectionVolumeScope::SelectedHits => d.composer.volume_for_selected_hits(v, relative),
            crate::SelectionVolumeScope::AllLanes => d.composer.volume_for_lanes(v, relative, None),
            crate::SelectionVolumeScope::SelectedLanes(lanes) => {
                d.composer.volume_for_lanes(v, relative, Some(lanes))
            }
        });
        if changed {
            self.bump_view_revision();
            self.refresh_recovery_edit();
        }
    }

    pub fn set_moment_volume_at_scoped(
        &mut self,
        ms: f64,
        value: i32,
        volume_lanes: Option<&[usize]>,
    ) -> bool {
        if self.busy || self.is_stroke_open() {
            return false;
        }
        let explicit_scope = self.selection_volume_scope.lane_scope().cloned();
        let changed = self.document.as_mut().is_some_and(|d| {
            let scope = volume_lanes
                .filter(|rows| !rows.is_empty())
                .map(|rows| resolve_volume_lanes(&d.composer, rows))
                .or(explicit_scope);
            d.composer.set_moment_volume_at(ms, value, scope.as_ref())
        });
        if changed {
            self.bump_view_revision();
            self.refresh_recovery_edit();
        }
        changed
    }

    pub fn set_moment_volume_at(&mut self, ms: f64, value: i32) -> bool {
        self.set_moment_volume_at_scoped(ms, value, None)
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
            self.bump_view_revision();
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
        self.bump_view_revision();
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
        self.bump_view_revision();
    }

    pub fn select_hit(&mut self, key: HitKey, add: bool) {
        if let Some(d) = self.document.as_mut() {
            if add {
                d.composer.selected.add(key);
            } else {
                d.composer.selected.replace([key]);
            }
            self.bump_view_revision();
        }
    }

    pub fn command(&mut self, id: &str) {
        let catalogue = self.catalogue.clone();
        if let RunOutcome::Failed { message, .. } = catalogue.run(self, id, EditorScope::Hitsound) {
            self.error = Some(message);
        }
        self.refresh_recovery_edit();
        self.bump_view_revision();
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
        self.bump_view_revision();
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
        self.bump_view_revision();
    }
    /// Replaces the active lane's editable hits, retaining its logical slot.
    /// In Drums view, consolidates all lanes merged into that drum row in one undoable step.
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
        let member_lanes: Vec<SampleIdentity> = self
            .drum_row_groups
            .as_ref()
            .and_then(|g| g.members_for_row(self.active_lane))
            .map(|rows| {
                rows.iter()
                    .filter_map(|&r| d.composer.lanes.get(r).cloned())
                    .collect::<Vec<_>>()
            })
            .filter(|v| v.len() > 1 && v.contains(expected_lane))
            .unwrap_or_else(|| vec![expected_lane.clone()]);
        if member_lanes
            .iter()
            .any(|l| matches!(l, SampleIdentity::Standard { sound: Sound::Slider(_), .. }))
        {
            return Err("Slider body samples are read-only here.".into());
        }
        let primary_lane = member_lanes[0].clone();
        let secondaries: Vec<SampleIdentity> = member_lanes
            .iter()
            .filter(|l| *l != &primary_lane)
            .cloned()
            .collect();

        let root = d.path.parent().ok_or("Missing mapset folder")?;
        let source = source.canonicalize().map_err(|e| format!("Opening {}: {e}", source.display()))?;
        let extension = source.extension().and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase).filter(|ext| matches!(ext.as_str(), "wav" | "ogg" | "mp3"))
            .ok_or("Choose a WAV, OGG, or MP3 sample.")?;
        let hits = d.composer.hitsounds();
        let logical = hits.lane_slot(&primary_lane).unwrap_or(&primary_lane);
        let file_path = |lane: &SampleIdentity| match &hits.lane(lane)?.source {
            LaneSource::File(name) => safe_child(root, name).ok(),
            LaneSource::Skin => None,
        };
        if secondaries.is_empty() && file_path(&primary_lane).as_ref() == Some(&source) {
            return Ok(());
        }
        let existing = source.starts_with(root).then(|| hits.lanes().iter().find(|candidate| {
            let lane = &candidate.identity;
            let compatible = match (&primary_lane, lane) {
                (SampleIdentity::Standard { set, sound, .. }, SampleIdentity::Standard { set: other_set, sound: other_sound, .. }) =>
                    set == other_set && sound == other_sound,
                (SampleIdentity::File(_), SampleIdentity::File(_)) => true,
                _ => false,
            };
            compatible
                && !member_lanes.contains(lane)
                && self.clip_for_lane(lane).is_some()
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
                primary_lane,
                SampleIdentity::Standard { index, .. } if index.get() == 0
            );
            let name = layers::create_backing(d, &primary_lane, &extension, &bytes, allow_index_one)?;
            let lane = dawsu_hitsounding::rules::read_sample_file_name(&name)
                .unwrap_or_else(|| SampleIdentity::File(name.clone()));
            self.document.as_mut().expect("open document").pending.add(&name, &bytes)?;
            self.register_samples(vec![(name, clip)], false);
            lane
        };
        let composer = &mut self.document.as_mut().expect("open document").composer;
        let replaced_set: BTreeSet<SampleIdentity> = member_lanes.iter().cloned().collect();
        let mut seen_selected = BTreeSet::new();
        let selected = composer
            .selected
            .items()
            .iter()
            .cloned()
            .filter_map(|mut hit| {
                if replaced_set.contains(&hit.lane) {
                    hit.lane = replacement.clone();
                }
                seen_selected.insert((hit.moment, hit.lane.clone())).then_some(hit)
            })
            .collect::<Vec<_>>();
        let tx_label = if secondaries.is_empty() {
            "Replace lane sample"
        } else {
            "Replace drum sample"
        };
        composer.apply(tx_label, |hits| {
            hits.replace_lane(&primary_lane, replacement.clone());
            for sec in &secondaries {
                let moves: Vec<(dawsu_hitsounding::MomentId, i32)> = hits
                    .moments()
                    .iter()
                    .enumerate()
                    .filter(|(_, moment)| moment.has_hit(sec))
                    .map(|(i, moment)| (dawsu_hitsounding::MomentId(i), moment.hit_volume(sec)))
                    .collect();
                for (moment_id, vol) in moves {
                    hits.remove_hit(moment_id, sec);
                    if !hits.moments()[moment_id.0].has_hit(&replacement) {
                        hits.add_hit_with_volume(moment_id, replacement.clone(), vol);
                    }
                }
                hits.set_lane_hidden(sec, true);
            }
        });
        composer.selected.replace(selected);
        self.active_lane = composer.lanes.iter().position(|lane| lane == &replacement)
            .expect("the backing lane is registered before replacing hits");
        self.status = format!("Assigned {}", source.file_name().unwrap_or_default().to_string_lossy());
        self.sync_audio();
        self.bump_view_revision();
        Ok(())
    }

    pub fn add_numbered_layer(&mut self) -> Result<(), String> {
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
        self.bump_view_revision();
        Ok(())
    }
}
