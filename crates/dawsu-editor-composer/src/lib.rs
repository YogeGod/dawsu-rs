//! Manual edits on top of the shared editing kernel.

use dawsu_editor::tempo::{Snap, SnapMode, SnapSpacing, TempoMap};
use dawsu_editor::tools::{PointerInput, TimeRange, ToolPreview, ToolTarget};
use dawsu_editor::{Clipboard, EditHistory, Selection};
use dawsu_foundation::{Hitsound, SampleIdentity, SongTime, Sound};
use dawsu_hitsounding::{Hitsounds, MomentId};
use std::{collections::BTreeSet, convert::Infallible};

mod batch;
mod lane_edit;
mod sound_sets;
mod workflows;
pub use batch::{
    BatchCounts, BatchHit, BatchOptions, BatchPlan, BatchRefusal, BatchReport, BatchRow, Lands,
    PatternStampOptions, Policy, quantize_stamp_volume,
};
pub use sound_sets::{MOMENT_HITNORMAL, SetHitnormal};
pub use workflows::{RangeOperation, VolumeScope};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HitKey {
    pub moment: usize,
    pub lane: SampleIdentity,
}

pub struct Composer {
    pub history: EditHistory<Hitsounds>,
    pub selected: Selection<HitKey>,
    pub lanes: Vec<SampleIdentity>,
    pub tempo: TempoMap,
    pub spacing: SnapSpacing,
    pub snap_enabled: bool,
    pub snap_mode: SnapMode,
    object_times: Vec<SongTime>,
    pub insert_volume: i32,
    pub protect_normals: bool,
    pub range: Option<TimeRange>,
    pub view_start: f64,
    pub row_offset: f64,
    pub picker_requested: bool,
    pub audition_requests: Vec<SampleIdentity>,
    pub listen_muted: Vec<HitKey>,
    pub listen_version: u64,
    pub preview: ToolPreview,
    clipboard: Clipboard<(SampleIdentity, i32)>,
}

impl Composer {
    pub fn new(hitsounds: Hitsounds) -> Self {
        let mut object_times: Vec<_> = hitsounds
            .moments()
            .iter()
            .filter(|m| m.place() != dawsu_hitsounding::Place::NoObject)
            .map(|m| m.time())
            .collect();
        object_times.sort_by(|a, b| a.ms().total_cmp(&b.ms()));
        object_times.dedup();
        let lanes = hitsounds
            .lanes()
            .iter()
            .map(|l| l.identity.clone())
            .collect();
        Self {
            history: EditHistory::new(hitsounds),
            selected: Selection::new(),
            lanes,
            tempo: TempoMap::default(),
            spacing: SnapSpacing::fraction(4).unwrap(),
            snap_enabled: true,
            snap_mode: SnapMode::Grid,
            object_times,
            insert_volume: 100,
            protect_normals: false,
            range: None,
            view_start: 0.0,
            row_offset: 0.0,
            picker_requested: false,
            audition_requests: Vec::new(),
            listen_muted: Vec::new(),
            listen_version: 0,
            preview: ToolPreview::None,
            clipboard: Clipboard::new(),
        }
    }
    pub fn hitsounds(&self) -> &Hitsounds {
        self.history.document()
    }
    pub fn dirty(&self) -> bool {
        self.history.is_dirty()
    }
    pub fn mark_saved(&mut self) {
        self.history.mark_clean();
    }
    fn prune_selection(&mut self) {
        let kept: Vec<_> = {
            let moments = self.hitsounds().moments();
            self.selected_keys_in_order()
                .into_iter()
                .filter(|key| moments.get(key.moment).is_some_and(|m| m.has_hit(&key.lane)))
                .collect()
        };
        self.selected.replace(kept);
    }
    pub fn undo(&mut self) {
        self.history.undo();
        self.prune_selection();
    }
    pub fn redo(&mut self) {
        self.history.redo();
        self.prune_selection();
    }
    pub fn apply(&mut self, label: &str, edit: impl FnOnce(&mut Hitsounds)) {
        let _ = self.history.apply(label, true, |h| {
            edit(h);
            Ok::<_, Infallible>(())
        });
    }
    pub fn add(&mut self, time: f64, lane: SampleIdentity, volume: i32, tolerance: f64) {
        let mut id = 0;
        self.apply("Draw hit", |h| {
            let moment = destination_at(h, time, volume, tolerance);
            h.add_hit(moment, lane.clone());
            id = moment.0;
        });
        if !self.lanes.contains(&lane) {
            self.lanes.push(lane.clone());
        }
        self.selected.replace([HitKey { moment: id, lane }]);
    }
    pub fn remove_selected(&mut self) {
        let (removed, kept): (Vec<_>, Vec<_>) = self
            .selected_keys_in_order()
            .into_iter()
            .partition(|key| self.can_remove_hit(key));
        if removed.is_empty() {
            return;
        }
        self.apply("Delete hits", |hits| {
            for key in removed {
                hits.remove_hit(MomentId(key.moment), &key.lane);
            }
        });
        self.selected.replace(kept);
    }

    fn can_remove_hit(&self, key: &HitKey) -> bool {
        (!self.protect_normals || !normal(&key.lane))
            && self.hitsounds().moments().get(key.moment)
                .is_some_and(|moment| moment.has_hit(&key.lane))
    }
    pub fn move_selected(
        &mut self,
        delta: f64,
        destination: Option<SampleIdentity>,
        tolerance: f64,
    ) {
        self.move_to(destination, tolerance, |_, time| (time + delta).max(0.0));
    }

    pub fn nudge(&mut self, direction: i32) {
        let spacing = self.spacing;
        self.move_to(None, 5.0, |tempo, time| {
            tempo
                .nudge(SongTime::from_ms(time), direction, spacing)
                .ms()
        });
    }

    fn move_to(
        &mut self,
        destination: Option<SampleIdentity>,
        tolerance: f64,
        target: impl Fn(&TempoMap, f64) -> f64,
    ) {
        let keys: Vec<_> = self
            .selected_keys_in_order()
            .into_iter()
            .map(|key| {
                (
                    key.clone(),
                    target(
                        &self.tempo,
                        self.hitsounds().moments()[key.moment].time().ms(),
                    ),
                )
            })
            .collect();
        let mut moved = Vec::new();
        self.apply("Move hits", |h| {
            let values: Vec<_> = keys
                .iter()
                .map(|(k, time)| (k.clone(), *time, h.moments()[k.moment].hit_volume(&k.lane)))
                .collect();
            for (key, _, _) in &values {
                h.remove_hit(MomentId(key.moment), &key.lane);
            }
            for (key, time, volume) in values {
                let lane = destination.clone().unwrap_or(key.lane);
                let moment = destination_at(h, time, volume, tolerance);
                h.add_hit_with_volume(moment, lane.clone(), volume);
                moved.push(HitKey { moment: moment.0, lane });
            }
        });
        if let Some(lane) = destination {
            if !self.lanes.contains(&lane) {
                self.lanes.push(lane);
            }
        }
        self.selected.replace(moved);
    }
    pub fn move_rows(&mut self, delta: i32) {
        let items: Vec<_> = self.selected.items().iter().cloned().collect();
        if items.is_empty() {
            return;
        }
        self.history.begin("Move hits between lanes");
        ToolTarget::move_by(self, &items, 0.0, delta);
        self.history.commit();
    }
    pub fn select_neighbour(&mut self, direction: i32, add: bool) {
        let time = self
            .selected
            .primary()
            .map(|k| self.hitsounds().moments()[k.moment].time().ms());
        let mut keys: Vec<_> = self
            .hitsounds()
            .moments()
            .iter()
            .enumerate()
            .flat_map(|(i, m)| {
                m.hits().map(move |lane| HitKey {
                    moment: i,
                    lane: lane.clone(),
                })
            })
            .collect();
        keys.sort_by(|a, b| {
            self.hitsounds().moments()[a.moment]
                .time()
                .ms()
                .total_cmp(&self.hitsounds().moments()[b.moment].time().ms())
                .then_with(|| a.lane.cmp(&b.lane))
        });
        if direction < 0 {
            keys.reverse();
        }
        if let Some(key) = keys.into_iter().find(|k| {
            time.is_none_or(|t| {
                if direction < 0 {
                    self.hitsounds().moments()[k.moment].time().ms() < t
                } else {
                    self.hitsounds().moments()[k.moment].time().ms() > t
                }
            })
        }) {
            if add {
                self.selected.add(key);
            } else {
                self.selected.replace([key]);
            }
        }
    }
    pub fn volume(&mut self, value: i32, relative: bool) {
        self.volume_for_lanes(value, relative, None);
    }

    /// Applies volume at the selected hit moments. A lane scope uses logical slot identities.
    /// The result is kept between 5 and 100.
    pub fn volume_for_lanes(
        &mut self,
        value: i32,
        relative: bool,
        lanes: Option<&BTreeSet<SampleIdentity>>,
    ) -> bool {
        let hits = self.hitsounds();
        let moments: BTreeSet<_> = self.selected.items().iter().filter_map(|key| {
            hits.moments().get(key.moment)?.has_hit(&key.lane).then_some(key.moment)
        }).collect();
        let edit = |before: i32| {
            let desired = if relative { before.saturating_add(value) } else { value };
            // An edit never leaves a volume under 5%: osu! plays nothing quieter (R5) and Save
            // writes nothing lower (R15), so the screen would show a number nothing uses.
            desired.clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100)
        };
        let mut shared = Vec::new();
        let mut moved = Vec::new();
        let mut individual = Vec::new();
        for index in moments {
            let id = MomentId(index);
            let moment = hits.moment(id);
            if let Some(lanes) = lanes {
                for lane in moment.hits() {
                    let before = moment.hit_volume(lane);
                    let after = edit(before);
                    if lanes.contains(hits.lane_slot(lane).unwrap_or(lane)) && after != before {
                        individual.push((id, lane.clone(), after));
                    }
                }
            } else {
                let volume = edit(moment.volume());
                if !relative || !moment.has_hit_volume_overrides() {
                    if volume != moment.reference_volume() {
                        moved.push((id, volume));
                    }
                    continue;
                }
                let volumes: Vec<_> = moment.hits().map(|lane| {
                    let before = moment.hit_volume(lane);
                    (lane, before, edit(before))
                }).collect();
                if volumes.iter().any(|(_, before, after)| before != after) {
                    shared.push((id, volume));
                    individual.extend(volumes.into_iter()
                        .filter(|(_, _, after)| *after != volume)
                        .map(|(lane, _, after)| (id, lane.clone(), after)));
                }
            }
        }
        if shared.is_empty() && moved.is_empty() && individual.is_empty() {
            return false;
        }
        self.apply("Change volume", |hits| {
            for (id, volume) in shared {
                hits.set_volume(id, volume);
            }
            for (id, volume) in moved {
                hits.move_volume(id, volume);
                // Moving keeps each hit's difference, which can push a quieter hit under 5%;
                // the edit stops it at 5% like every other volume it sets.
                let floor = dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR;
                let quiet: Vec<_> = hits.moment(id).hits()
                    .filter(|lane| hits.moment(id).hit_volume(lane) < floor)
                    .cloned().collect();
                for lane in quiet {
                    hits.set_hit_volume(id, &lane, floor);
                }
            }
            for (id, lane, volume) in individual {
                hits.set_hit_volume(id, &lane, volume);
            }
        });
        true
    }
    pub fn select_all(&mut self) {
        self.selected.replace(
            self.hitsounds()
                .moments()
                .iter()
                .enumerate()
                .flat_map(|(i, m)| {
                    m.hits().map(move |lane| HitKey {
                        moment: i,
                        lane: lane.clone(),
                    })
                })
                .collect::<Vec<_>>(),
        );
    }
    pub fn copy(&mut self) {
        let items: Vec<_> = self
            .selected_keys_in_order()
            .into_iter()
            .map(|k| {
                let m = &self.hitsounds().moments()[k.moment];
                (m.time(), (k.lane.clone(), m.hit_volume(&k.lane)))
            })
            .collect();
        self.clipboard.copy(items);
    }
    pub fn paste(&mut self, at: f64) {
        if self.clipboard.is_empty() {
            return;
        }
        let clipboard = self.clipboard.paste_at(SongTime::from_ms(at));
        let mut added = Vec::new();
        self.apply("Paste hits", |h| {
            for (time, (lane, v)) in clipboard {
                let moment = destination_at(h, time.ms(), v, 5.0);
                h.add_hit_with_volume(moment, lane.clone(), v);
                added.push(HitKey { moment: moment.0, lane });
            }
        });
        self.selected.replace(added);
    }
}

fn normal(lane: &SampleIdentity) -> bool {
    matches!(
        lane,
        SampleIdentity::Standard {
            sound: Sound::Hit(Hitsound::Normal),
            ..
        }
    )
}
fn destination_at(h: &mut Hitsounds, time: f64, volume: i32, tolerance: f64) -> MomentId {
    nearest_moment(h, time, tolerance)
        .unwrap_or_else(|| h.add_moment(SongTime::from_ms(time), volume))
}

fn nearest_moment(h: &Hitsounds, time: f64, tolerance: f64) -> Option<MomentId> {
    h.moments()
        .iter()
        .enumerate()
        .filter(|(_, m)| (m.time().ms() - time).abs() <= tolerance)
        .min_by(|(_, a), (_, b)| {
            (a.time().ms() - time)
                .abs()
                .total_cmp(&(b.time().ms() - time).abs())
        })
        .map(|(i, _)| MomentId(i))
}

impl ToolTarget for Composer {
    type Item = HitKey;
    fn hit_test(&self, at: &PointerInput) -> Option<HitKey> {
        let lane = self.lanes.get(at.row.max(0) as usize)?;
        self.hitsounds()
            .moments()
            .iter()
            .enumerate()
            .filter(|(_, m)| m.has_hit(lane) && (m.time().ms() - at.time.ms()).abs() <= 10.0)
            .min_by(|(_, a), (_, b)| {
                (a.time().ms() - at.time.ms())
                    .abs()
                    .total_cmp(&(b.time().ms() - at.time.ms()).abs())
            })
            .map(|(moment, _)| HitKey {
                moment,
                lane: lane.clone(),
            })
    }
    fn in_range(&self, from: SongTime, to: SongTime, first: i32, last: i32) -> Vec<HitKey> {
        let mut keys = Vec::new();
        for (i, m) in self.hitsounds().moments().iter().enumerate() {
            if m.time().ms() < from.ms().min(to.ms()) || m.time().ms() > from.ms().max(to.ms()) {
                continue;
            }
            for lane in m.hits() {
                if let Some(row) = self.lanes.iter().position(|l| l == lane) {
                    if (first..=last).contains(&(row as i32)) {
                        keys.push(HitKey {
                            moment: i,
                            lane: lane.clone(),
                        });
                    }
                }
            }
        }
        keys
    }
    fn item_time(&self, item: &HitKey) -> SongTime {
        self.hitsounds().moments()[item.moment].time()
    }
    fn selection(&mut self) -> &mut Selection<HitKey> {
        &mut self.selected
    }
    fn snap(&self, time: SongTime) -> SongTime {
        Snap {
            on: self.snap_enabled,
            mode: self.snap_mode,
            spacing: self.spacing,
        }
        .time(&self.tempo, &self.object_times, time)
    }
    fn snap_steps(&self, from: SongTime, to: SongTime) -> Vec<SongTime> {
        let start = self.snap(from);
        let end = self.snap(to);
        if !self.snap_enabled || (end.ms() - start.ms()).abs() < 1e-6 {
            return vec![end];
        }
        let forward = end.ms() > start.ms();
        if self.snap_mode == SnapMode::Objects {
            let (lo, hi) = if forward { (start.ms(), end.ms()) } else { (end.ms(), start.ms()) };
            let mut steps: Vec<SongTime> = self
                .object_times
                .iter()
                .copied()
                .filter(|t| t.ms() >= lo - 1e-6 && t.ms() <= hi + 1e-6)
                .collect();
            if !forward {
                steps.reverse();
            }
            if steps.is_empty() {
                steps.push(end);
            }
            return steps;
        }
        let dir = if forward { 1 } else { -1 };
        let mut steps = Vec::new();
        let mut cur = start;
        for _ in 0..256 {
            let next = self.tempo.grid_step(cur, dir, self.spacing);
            if (next.ms() - cur.ms()).abs() < 1e-6 {
                break;
            }
            if (forward && next.ms() > end.ms() + 1e-6) || (!forward && next.ms() < end.ms() - 1e-6) {
                break;
            }
            steps.push(next);
            cur = next;
            if (cur.ms() - end.ms()).abs() < 1e-6 {
                break;
            }
        }
        if steps.last().is_none_or(|last| (last.ms() - end.ms()).abs() >= 1e-6) {
            steps.push(end);
        }
        steps
    }
    fn snap_offset(&self, from: SongTime, raw: f64) -> f64 {
        Snap {
            on: self.snap_enabled,
            mode: self.snap_mode,
            spacing: self.spacing,
        }
        .offset(&self.tempo, &self.object_times, from, raw)
    }
    fn begin(&mut self, label: &str) {
        self.history.begin(label);
    }
    fn commit(&mut self) {
        self.history.commit();
    }
    fn cancel(&mut self) {
        self.history.cancel();
    }
    fn insert(&mut self, at: SongTime, row: i32) -> Option<HitKey> {
        let lane = self.lanes.get(row.max(0) as usize)?.clone();
        let volume = self.insert_volume;
        let existing = nearest_moment(self.hitsounds(), at.ms(), 5.0);
        if let Some(id) = existing
            && self.hitsounds().moments()[id.0].has_hit(&lane)
        {
            return Some(HitKey { moment: id.0, lane });
        }
        let mut moment = 0;
        let _ = self.history.edit(true, |h| {
            let id = existing.unwrap_or_else(|| h.add_moment(at, volume));
            h.add_hit(id, lane.clone());
            moment = id.0;
            Ok::<_, Infallible>(())
        });
        self.audition_requests.push(lane.clone());
        Some(HitKey { moment, lane })
    }
    fn delete(&mut self, items: &[HitKey]) {
        let removable: Vec<_> = items.iter().filter(|key| self.can_remove_hit(key)).collect();
        if removable.is_empty() {
            return;
        }
        let _ = self.history.edit(true, |hits| {
            for key in removable {
                hits.remove_hit(MomentId(key.moment), &key.lane);
            }
            Ok::<_, Infallible>(())
        });
    }
    fn move_by(&mut self, items: &[HitKey], delta: f64, rows: i32) {
        let lanes = self.lanes.clone();
        let mut new = Vec::new();
        let primary = self.selected.primary().cloned();
        let _ = self.history.edit(true, |h| {
            let mut values: Vec<_> = items
                .iter()
                .map(|k| {
                    (
                        k.clone(),
                        h.moments()[k.moment].time().ms(),
                        h.moments()[k.moment].hit_volume(&k.lane),
                    )
                })
                .collect();
            values.sort_unstable_by(|(a, _, _), (b, _, _)| {
                (Some(a) != primary.as_ref(), a.moment, &a.lane)
                    .cmp(&(Some(b) != primary.as_ref(), b.moment, &b.lane))
            });
            for (k, _, _) in &values {
                h.remove_hit(MomentId(k.moment), &k.lane);
            }
            for (k, t, v) in values {
                let old = lanes.iter().position(|l| *l == k.lane).unwrap_or(0) as i32;
                let mut row = (old + rows).clamp(0, lanes.len() as i32 - 1);
                while rows != 0 && h.is_lane_hidden(&lanes[row as usize]) {
                    let next = row + rows.signum();
                    if !(0..lanes.len() as i32).contains(&next) {
                        row = old;
                        break;
                    }
                    row = next;
                }
                let lane = lanes[row as usize].clone();
                let moment = destination_at(h, (t + delta).max(0.0), v, 5.0);
                h.add_hit_with_volume(moment, lane.clone(), v);
                new.push(HitKey { moment: moment.0, lane });
            }
            Ok::<_, Infallible>(())
        });
        if rows != 0 {
            self.audition_requests
                .extend(new.iter().map(|k| k.lane.clone()));
        }
        self.selected.replace(new);
    }
    fn set_values(&mut self, values: &[(HitKey, i32)]) {
        let _ = self.history.edit(true, |h| {
            for (key, v) in values {
                // Each hit at a moment brings the same value, so moving to it again changes nothing.
                h.move_volume(MomentId(key.moment), (*v).clamp(0, 100));
            }
            Ok::<_, Infallible>(())
        });
    }
    fn toggle_listen_mute(&mut self, item: &HitKey) {
        if let Some(i) = self.listen_muted.iter().position(|k| k == item) {
            self.listen_muted.remove(i);
        } else {
            self.listen_muted.push(item.clone());
        }
        self.listen_version += 1;
    }
    fn audition(&mut self, item: &HitKey) {
        self.audition_requests.push(item.lane.clone());
    }
    fn pan(&mut self, ms: f64, rows: f64) {
        self.view_start = (self.view_start + ms).max(0.0);
        self.row_offset = (self.row_offset + rows).max(0.0);
    }
    fn set_range(&mut self, range: Option<TimeRange>) {
        self.range = range;
    }
    fn open_tool_picker(&mut self, _at: &PointerInput) {
        self.picker_requested = true;
    }
    fn show_preview(&mut self, preview: ToolPreview) {
        self.preview = preview;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_editor::keys::Mods;
    use dawsu_editor::tempo::TimingSection;
    use dawsu_editor::tools::{PointerButton, PointerSurface, ToolBox, ToolKind};
    use dawsu_foundation::{CustomIndex, SampleSet};
    use dawsu_hitsounding::{Decompiled, Place, decompile};
    use dawsu_osu_format::OsuFile;

    fn empty_composer() -> Composer {
        let f = OsuFile::read(b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n".to_vec()).file;
        let Decompiled::Done(h) = decompile(&f, &[], &|| false, &mut |_, _| {}) else {
            panic!("empty fixture")
        };
        Composer::new(h)
    }

    fn lane_normal() -> SampleIdentity {
        SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Normal),
            index: CustomIndex::new(0),
        }
    }

    fn lane_clap() -> SampleIdentity {
        SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Clap),
            index: CustomIndex::new(0),
        }
    }

    #[test]
    fn undo_and_redo_preserve_surviving_selection() {
        let mut composer = empty_composer();
        let normal = lane_normal();
        let clap = lane_clap();
        composer.add(100.0, normal.clone(), 80, 5.0);
        composer.add(200.0, clap.clone(), 80, 5.0);

        let k1 = HitKey { moment: 0, lane: normal.clone() };
        let k2 = HitKey { moment: 1, lane: clap.clone() };
        composer.selected.replace([k1.clone(), k2.clone()]);

        composer.undo();
        let after_undo = composer.selected_keys_in_order();
        assert_eq!(after_undo, vec![k1.clone()]);

        composer.redo();
        let after_redo = composer.selected_keys_in_order();
        assert_eq!(after_redo, vec![k1]);
    }

    #[test]
    fn volume_for_lanes_updates_no_object_moments() {
        let mut composer = empty_composer();
        let normal = lane_normal();
        let clap = lane_clap();
        composer.add(100.0, normal.clone(), 80, 5.0);
        composer.add(100.0, clap.clone(), 80, 5.0);
        composer.selected.replace([
            HitKey { moment: 0, lane: normal.clone() },
            HitKey { moment: 0, lane: clap.clone() },
        ]);

        let scope = BTreeSet::from([clap.clone()]);
        assert!(composer.volume_for_lanes(55, false, Some(&scope)));
        let m = composer.hitsounds().moment(MomentId(0));
        assert_eq!(m.place(), Place::NoObject);
        assert_eq!(m.hit_volume(&clap), 55);
        assert_eq!(m.hit_volume(&normal), 80);
    }

    #[test]
    fn draw_paint_inserts_dense_hits_under_10ms_apart() {
        let mut composer = empty_composer();
        let lane = lane_normal();
        composer.lanes = vec![lane];
        // Fast tempo (500 BPM => 120 ms/beat) with 1/16 snap => 7.5 ms step (< 10 ms).
        composer.tempo = TempoMap::new(vec![TimingSection {
            start: SongTime::from_ms(0.0),
            beat_length_ms: 120.0,
            beats_per_bar: 4,
        }]);
        composer.spacing = SnapSpacing::fraction(16).unwrap();

        let mut tools = ToolBox::new();
        tools.select_tool(ToolKind::Draw, &mut composer);

        let ev = |ms: f64| PointerInput {
            time: SongTime::from_ms(ms),
            row: 0,
            row_value: None,
            surface: PointerSurface::Grid,
            button: PointerButton::Left,
            mods: Mods::NONE,
            click_count: 1,
            x: 0.0,
            y: 0.0,
        };

        tools.down(&mut composer, ev(0.0));
        tools.moved(&mut composer, ev(15.0));
        tools.up(&mut composer, ev(15.0));

        assert_eq!(composer.hitsounds().moments().len(), 3);
    }
}
