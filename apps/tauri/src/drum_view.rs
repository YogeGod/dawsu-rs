use dawsu_application::Application;
pub(super) use dawsu_application::drum_view::{DrumMapAction, DrumRowSnapshot, DrumViewSnapshot};
use dawsu_hitsounding::drums::DrumMap;
#[cfg(test)]
pub(super) use dawsu_hitsounding::drums::DEFAULT_OSU_KIT_NAME;
#[cfg(test)]
use std::collections::HashMap;
use std::path::Path;

#[derive(Default)]
pub(super) struct DrumView(pub(crate) dawsu_application::drum_view::DrumView);

impl DrumView {
    pub(super) fn for_editor() -> Self {
        Self(dawsu_application::drum_view::DrumView::for_editor())
    }

    /// Runs whenever the open document generation changes: auto-activates Path 2's per-diff
    /// `Default osu! kit (12 skin sounds)` without bleeding into sibling diffs, syncs Drum VST
    /// routing, and bumps `tick_cache.drums_rev`.
    pub(super) fn sync_on_document_open(
        &mut self,
        app: &mut Application,
        tick_cache: &mut super::ViewTickCache,
    ) -> bool {
        if self.0.sync_on_document_open(app) {
            tick_cache.drums_rev += 1;
            tick_cache.invalidate();
            true
        } else {
            false
        }
    }

    pub(super) fn active_map(&self, app: &Application) -> Option<&DrumMap> {
        self.0.active_map(app)
    }

    pub(super) fn load(
        &mut self,
        app: &Application,
        expected_map: &Path,
        path: &Path,
    ) -> Result<(), String> {
        self.0.load(app, expected_map, path)
    }

    pub(super) fn load_builtin(
        &mut self,
        app: &Application,
        expected_map: &Path,
        name: &str,
    ) -> Result<(), String> {
        self.0.load_builtin(app, expected_map, name)
    }

    pub(super) fn auto_detect_now(
        &mut self,
        app: &Application,
        expected_map: &Path,
    ) -> Result<(), String> {
        self.0.auto_detect_now(app, expected_map)
    }

    pub(super) fn set_auto_detect(
        &mut self,
        app: &Application,
        expected_map: &Path,
        on: bool,
    ) -> Result<(), String> {
        self.0.set_auto_detect(app, expected_map, on)
    }

    pub(super) fn set_hide_unassigned(
        &mut self,
        app: &Application,
        expected_map: &Path,
        on: bool,
    ) -> Result<(), String> {
        self.0.set_hide_unassigned(app, expected_map, on)
    }

    pub(super) fn move_row(
        &mut self,
        app: &Application,
        expected_map: &Path,
        from_index: usize,
        to_index: usize,
    ) -> Result<(), String> {
        self.0.move_row(app, expected_map, from_index, to_index)
    }

    pub(super) fn assign_row(
        &mut self,
        app: &Application,
        expected_map: &Path,
        row_index: usize,
        target_lane_key: Option<&str>,
    ) -> Result<(), String> {
        self.0
            .assign_row(app, expected_map, row_index, target_lane_key)
    }

    pub(super) fn set_midi_note(
        &mut self,
        app: &Application,
        expected_map: &Path,
        row_index: usize,
        midi_note: Option<u8>,
    ) -> Result<(), String> {
        self.0
            .set_midi_note(app, expected_map, row_index, midi_note)
    }

    #[cfg(test)]
    pub(super) fn routed_midi_notes(
        &self,
        app: &Application,
    ) -> HashMap<dawsu_foundation::SampleIdentity, u8> {
        self.0.routed_midi_notes(app)
    }

    pub(super) fn toggle(
        &mut self,
        app: &Application,
        expected_map: &Path,
        on: bool,
    ) -> Result<(), String> {
        self.0.toggle(app, expected_map, on)
    }

    pub(super) fn sync_app(&mut self, app: &mut Application) {
        self.0.sync_app(app);
    }

    #[cfg(test)]
    pub(super) fn open_or_toggle_and_sync(
        &mut self,
        app: &mut Application,
        expected_map: &Path,
        preset: Option<&Path>,
        on: bool,
    ) -> Result<(), String> {
        self.0.open_or_toggle_and_sync(app, expected_map, preset, on)
    }

    pub(super) fn decorate(&self, app: &Application, view: &mut super::ViewSnapshot) {
        let lane_keys: Vec<(String, String)> = view
            .scene
            .as_ref()
            .map(|scene| {
                scene
                    .lanes
                    .iter()
                    .map(|l| (l.key.clone(), l.source_key.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let scene_lanes = view
            .scene
            .as_ref()
            .map(|scene| (scene.lane_order.as_slice(), lane_keys.as_slice()));
        let Some(decorated) = self.0.build_decorated_view(app, scene_lanes) else {
            return;
        };
        view.drum_view = Some(decorated.drum_view);
        if let (Some(scene), Some(rows)) = (view.scene.as_mut(), decorated.drum_rows) {
            scene.drum_rows = Some(rows);
        }
    }
}
