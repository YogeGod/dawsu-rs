//! Manual edits on top of the shared editing kernel.

use dawsu_editor::tempo::{Snap, SnapMode, SnapSpacing, TempoMap};
use dawsu_editor::tools::{PointerInput, TimeRange, ToolPreview, ToolTarget};
use dawsu_editor::{Clipboard, EditHistory, Selection};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SongTime, Sound};
use dawsu_hitsounding::{Hitsounds, MomentId, Place};
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
    move_stroke: Option<MoveStroke>,
}

type MoveStroke = (Hitsounds, Vec<(HitKey, f64, i32)>, f64, i32);

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
            spacing: SnapSpacing::fraction(4).expect("1/4 beat is a valid snap spacing"),
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
            move_stroke: None,
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
            .filter_map(|key| {
                let moment = self.hitsounds().moments().get(key.moment)?;
                if !moment.has_hit(&key.lane) {
                    return None;
                }
                let old_ms = moment.time().ms();
                let new_ms = target(&self.tempo, old_ms);
                Some((key, old_ms, new_ms))
            })
            .collect();
        if keys.is_empty() {
            return;
        }
        let any_changed = keys.iter().any(|(key, old_ms, new_ms)| {
            (new_ms - old_ms).abs() > 1e-6
                || destination.as_ref().is_some_and(|dest| dest != &key.lane)
        });
        if !any_changed {
            return;
        }
        let mut moved = Vec::new();
        self.apply("Move hits", |h| {
            let values: Vec<_> = keys
                .iter()
                .map(|(k, _, time)| (k.clone(), *time, h.moments()[k.moment].hit_volume(&k.lane)))
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
        if let Some(lane) = destination
            && !self.lanes.contains(&lane) {
                self.lanes.push(lane);
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

    /// Sets the volume of the single moment nearest `ms` (within 5 ms) directly,
    /// without requiring it to be selected first. Respects `lanes` when scoped.
    pub fn set_moment_volume_at(
        &mut self,
        ms: f64,
        value: i32,
        lanes: Option<&BTreeSet<SampleIdentity>>,
    ) -> bool {
        if !ms.is_finite() {
            return false;
        }
        let target = value.clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100);
        let hits = self.hitsounds();
        let best = hits
            .moments()
            .iter()
            .enumerate()
            .filter_map(|(idx, moment)| {
                let d = (moment.time().ms() - ms).abs();
                (d <= 5.0 && moment.hits().next().is_some()).then_some((d, MomentId(idx)))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id);
        let Some(id) = best else {
            return false;
        };
        let moment = hits.moment(id);
        if let Some(lanes) = lanes {
            let changes: Vec<_> = moment
                .hits()
                .filter(|lane| {
                    lanes.contains(hits.lane_slot(lane).unwrap_or(lane))
                        && moment.hit_volume(lane) != target
                })
                .cloned()
                .collect();
            if changes.is_empty() {
                return false;
            }
            self.apply("Change volume", |hits| {
                for lane in changes {
                    hits.set_hit_volume(id, &lane, target);
                }
            });
            return true;
        }
        if moment.reference_volume() == target && !moment.has_hit_volume_overrides() {
            return false;
        }
        self.apply("Change volume", |hits| {
            hits.move_volume(id, target);
            let floor = dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR;
            let quiet: Vec<_> = hits
                .moment(id)
                .hits()
                .filter(|lane| hits.moment(id).hit_volume(lane) < floor)
                .cloned()
                .collect();
            for lane in quiet {
                hits.set_hit_volume(id, &lane, floor);
            }
        });
        true
    }

    /// Moves all selected standard hits to `target_set`, leaving custom file lanes untouched.
    pub fn set_selection_sample_set(&mut self, target_set: SampleSet) -> bool {
        let hits = self.hitsounds();
        let mut moves = Vec::new();
        for key in self.selected.items() {
            let Some(moment) = hits.moments().get(key.moment) else {
                continue;
            };
            if !moment.has_hit(&key.lane) {
                continue;
            }
            if let SampleIdentity::Standard { set, sound, index } = key.lane
                && set != target_set
            {
                let next_lane = SampleIdentity::Standard {
                    set: target_set,
                    sound,
                    index,
                };
                let existing_target = moment.has_hit(&next_lane);
                let vol = moment.hit_volume(&key.lane);
                moves.push((key.moment, key.lane.clone(), next_lane, vol, existing_target));
            }
        }
        if moves.is_empty() {
            return false;
        }
        let mut next_selected: Vec<HitKey> = self.selected.items().iter().cloned().collect();
        self.apply("Change sample set", |hits| {
            for (moment_idx, old_lane, new_lane, vol, existing_target) in &moves {
                let id = MomentId(*moment_idx);
                hits.remove_hit(id, old_lane);
                if !existing_target {
                    hits.add_hit_with_volume(id, new_lane.clone(), *vol);
                }
            }
        });
        for key in &mut next_selected {
            if let Some((_, _, new_lane, _, _)) = moves
                .iter()
                .find(|(m, old, _, _, _)| *m == key.moment && old == &key.lane)
            {
                key.lane = new_lane.clone();
            }
        }
        let mut deduped = Vec::with_capacity(next_selected.len());
        for key in next_selected {
            if !deduped.contains(&key) {
                deduped.push(key);
            }
        }
        self.selected.replace(deduped);
        true
    }

    /// Toggles an addition (`Whistle`, `Finish`, or `Clap`) across all selected moments.
    pub fn toggle_selection_addition(&mut self, addition: Hitsound) -> bool {
        if !matches!(addition, Hitsound::Whistle | Hitsound::Finish | Hitsound::Clap) {
            return false;
        }
        let hits = self.hitsounds();
        let moments: BTreeSet<usize> = self
            .selected
            .items()
            .iter()
            .filter_map(|key| {
                hits.moments()
                    .get(key.moment)?
                    .has_hit(&key.lane)
                    .then_some(key.moment)
            })
            .collect();
        if moments.is_empty() {
            return false;
        }
        let target_sound = Sound::Hit(addition);
        let all_have = moments.iter().all(|&idx| {
            hits.moments()[idx].hits().any(|lane| {
                matches!(lane, SampleIdentity::Standard { sound, .. } if *sound == target_sound)
            })
        });
        let mut removals = Vec::new();
        let mut additions_to_add = Vec::new();
        for &idx in &moments {
            let moment = &hits.moments()[idx];
            if all_have {
                for lane in moment.hits() {
                    if matches!(lane, SampleIdentity::Standard { sound, .. } if *sound == target_sound) {
                        removals.push((MomentId(idx), lane.clone()));
                    }
                }
            } else {
                let already = moment.hits().any(|lane| {
                    matches!(lane, SampleIdentity::Standard { sound, .. } if *sound == target_sound)
                });
                if !already {
                    let (chosen_set, chosen_index) = moment
                        .hits()
                        .find_map(|lane| match lane {
                            SampleIdentity::Standard {
                                set,
                                sound: Sound::Hit(Hitsound::Whistle | Hitsound::Finish | Hitsound::Clap),
                                index,
                            } => Some((*set, *index)),
                            _ => None,
                        })
                        .or_else(|| {
                            moment.hits().find_map(|lane| match lane {
                                SampleIdentity::Standard {
                                    set,
                                    sound: Sound::Hit(Hitsound::Normal),
                                    index,
                                } => Some((*set, *index)),
                                _ => None,
                            })
                        })
                        .or_else(|| {
                            moment.hits().find_map(|lane| match lane {
                                SampleIdentity::Standard { set, index, .. } => Some((*set, *index)),
                                _ => None,
                            })
                        })
                        .unwrap_or((SampleSet::Normal, CustomIndex::default()));
                    let new_lane = SampleIdentity::Standard {
                        set: chosen_set,
                        sound: target_sound,
                        index: chosen_index,
                    };
                    additions_to_add.push((MomentId(idx), new_lane, moment.volume()));
                }
            }
        }
        if removals.is_empty() && additions_to_add.is_empty() {
            return false;
        }
        self.apply("Toggle addition", |hits| {
            for (id, lane) in &removals {
                hits.remove_hit(*id, lane);
            }
            for (id, lane, vol) in &additions_to_add {
                hits.add_hit_with_volume(*id, lane.clone(), *vol);
            }
        });
        if !removals.is_empty() {
            let kept: Vec<HitKey> = self
                .selected
                .items()
                .iter()
                .filter(|key| {
                    !removals
                        .iter()
                        .any(|(id, lane)| id.0 == key.moment && lane == &key.lane)
                })
                .cloned()
                .collect();
            self.selected.replace(kept);
        }
        true
    }

    /// Plain-English single-click auto-fix for a coverage or orphan issue at `time_ms`.
    pub fn auto_fix_at(&mut self, time_ms: f64) -> Option<String> {
        if !time_ms.is_finite() {
            return None;
        }
        let hits = self.hitsounds();
        let is_fixable = |m: &dawsu_hitsounding::Moment| {
            (m.place() == Place::NoObject && m.hits().next().is_some())
                || (m.place() != Place::NoObject && m.hits().next().is_none())
        };
        let (target_idx, moment) = hits
            .moments()
            .iter()
            .enumerate()
            .filter(|(_, m)| (m.time().ms() - time_ms).abs() <= 2.0 && is_fixable(m))
            .min_by(|(_, a), (_, b)| {
                (a.time().ms() - time_ms)
                    .abs()
                    .total_cmp(&(b.time().ms() - time_ms).abs())
            })?;
        if moment.place() == Place::NoObject && moment.hits().next().is_some() {
            let orphaned: Vec<(SampleIdentity, i32)> = moment
                .hits()
                .map(|lane| (lane.clone(), moment.hit_volume(lane)))
                .collect();
            let nearest_obj = hits
                .moments()
                .iter()
                .enumerate()
                .filter(|(_, m)| m.place() != Place::NoObject)
                .map(|(i, m)| (i, (m.time().ms() - moment.time().ms()).abs()))
                .filter(|(_, dist)| *dist <= 50.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            if let Some(dest_idx) = nearest_obj {
                self.apply("Snap orphaned hits to object", |h| {
                    let src = MomentId(target_idx);
                    let dst = MomentId(dest_idx);
                    for (lane, vol) in &orphaned {
                        h.remove_hit(src, lane);
                        h.add_hit_with_volume(dst, lane.clone(), *vol);
                    }
                });
                return Some("Snapped orphaned hits to nearest object".into());
            }
            self.apply("Remove orphaned hits", |h| {
                let src = MomentId(target_idx);
                for (lane, _) in &orphaned {
                    h.remove_hit(src, lane);
                }
            });
            return Some("Removed orphaned hits".into());
        }
        if moment.place() != Place::NoObject && moment.hits().next().is_none() {
            let default_lane = SampleIdentity::Standard {
                set: SampleSet::Normal,
                sound: Sound::Hit(Hitsound::Normal),
                index: CustomIndex::default(),
            };
            let vol = self.insert_volume.clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100);
            self.apply("Add default hitnormal", |h| {
                h.add_hit_with_volume(MomentId(target_idx), default_lane.clone(), vol);
            });
            return Some("Added default hitnormal".into());
        }
        None
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

/// Maximum snapped steps a single Draw paint stroke can place at once, so an
/// accidental zoomed-out drag can never freeze the editor.
pub const MAX_PAINT_STROKE_STEPS: usize = 512;

impl Composer {
    /// Computes every snapped step between `start_ms` and `end_ms` (inclusive of
    /// both endpoints, in drag order), respecting snap on/off, `snap_mode`
    /// (`Grid`, `GridRelative`, `Objects`), timing sections, and beat divisor.
    /// Capped at [`MAX_PAINT_STROKE_STEPS`] steps.
    pub fn paint_stroke_steps(&self, start_ms: f64, end_ms: f64) -> Vec<SongTime> {
        if !start_ms.is_finite() || !end_ms.is_finite() {
            return Vec::new();
        }
        let from = SongTime::from_ms(start_ms.max(0.0));
        let to = SongTime::from_ms(end_ms.max(0.0));
        let start = self.snap(from);
        let end = self.snap(to);
        if !self.snap_enabled || (end.ms() - start.ms()).abs() < 1e-6 {
            return vec![end];
        }
        let forward = end.ms() > start.ms();
        if self.snap_mode == SnapMode::Objects {
            let (lo, hi) = if forward {
                (start.ms(), end.ms())
            } else {
                (end.ms(), start.ms())
            };
            let mut raw: Vec<SongTime> = self
                .object_times
                .iter()
                .copied()
                .filter(|t| t.ms() >= lo - 1e-6 && t.ms() <= hi + 1e-6)
                .collect();
            if !forward {
                raw.reverse();
            }
            let mut steps: Vec<SongTime> = Vec::with_capacity(raw.len().min(MAX_PAINT_STROKE_STEPS));
            for t in raw {
                if steps.len() >= MAX_PAINT_STROKE_STEPS {
                    break;
                }
                if steps.last().is_none_or(|prev| (t.ms() - prev.ms()).abs() >= 1.0) {
                    steps.push(t);
                }
            }
            if steps.is_empty() {
                steps.push(end);
            }
            return steps;
        }
        let dir = if forward { 1 } else { -1 };
        let mut steps = vec![start];
        let mut cur = start;
        while steps.len() < MAX_PAINT_STROKE_STEPS {
            let next = self.tempo.grid_step(cur, dir, self.spacing);
            if (next.ms() - cur.ms()).abs() < 1e-6 {
                break;
            }
            if (forward && next.ms() > end.ms() + 1e-6)
                || (!forward && next.ms() < end.ms() - 1e-6)
            {
                break;
            }
            steps.push(next);
            cur = next;
            if (cur.ms() - end.ms()).abs() < 1e-6 {
                break;
            }
        }
        if steps.len() < MAX_PAINT_STROKE_STEPS
            && steps.last().is_none_or(|last| (last.ms() - end.ms()).abs() >= 1e-6)
        {
            steps.push(end);
        }
        steps
    }

    /// Paints hits onto `row` at every snapped step between `start_ms` and
    /// `end_ms` that does not already have a hit on that lane, respecting
    /// `insert_volume` and recording a single Undo step. Returns the number of
    /// newly placed hits.
    pub fn paint_hits_on_lane(&mut self, row: usize, start_ms: f64, end_ms: f64) -> usize {
        let Some(lane) = self.lanes.get(row).cloned() else {
            return 0;
        };
        let steps = self.paint_stroke_steps(start_ms, end_ms);
        if steps.is_empty() {
            return 0;
        }
        let mut to_place: Vec<SongTime> = Vec::with_capacity(steps.len());
        for at in steps {
            if to_place.iter().any(|prev| (prev.ms() - at.ms()).abs() < 5.0) {
                continue;
            }
            if let Some(id) = nearest_moment(self.hitsounds(), at.ms(), 5.0)
                && self.hitsounds().moments()[id.0].has_hit(&lane)
            {
                continue;
            }
            to_place.push(at);
        }
        if to_place.is_empty() {
            return 0;
        }
        let volume = self.insert_volume;
        let mut placed = 0usize;
        self.history.begin("Paint draw stroke");
        let _ = self.history.edit(true, |h| {
            for at in &to_place {
                let existing = nearest_moment(h, at.ms(), 5.0);
                if let Some(id) = existing
                    && h.moments()[id.0].has_hit(&lane)
                {
                    continue;
                }
                let id = existing.unwrap_or_else(|| h.add_moment(*at, volume));
                h.add_hit_with_volume(id, lane.clone(), volume);
                placed += 1;
            }
            Ok::<_, Infallible>(())
        });
        self.history.commit();
        if placed > 0 {
            self.audition_requests.push(lane);
        }
        placed
    }

    /// Toggles a single hit on `row` near `ms` (within 4 ms) as one Undo step, regardless of which
    /// timeline tool is selected. Returns `true` if a hit was added or removed.
    pub fn toggle_step_hit(&mut self, row: usize, ms: f64) -> bool {
        if !ms.is_finite() || ms < 0.0 {
            return false;
        }
        let Some(lane) = self.lanes.get(row).cloned() else {
            return false;
        };
        let existing_id = nearest_moment(self.hitsounds(), ms, 4.0);
        if let Some(id) = existing_id
            && self.hitsounds().moments()[id.0].has_hit(&lane)
        {
            self.history.begin("Remove step hit");
            let _ = self.history.edit(true, |h| {
                h.remove_hit(id, &lane);
                Ok::<_, Infallible>(())
            });
            self.history.commit();
            return true;
        }
        let target = if self.snap_mode == SnapMode::Objects {
            SongTime::from_ms(ms)
        } else {
            self.snap(SongTime::from_ms(ms))
        };
        let volume = self
            .insert_volume
            .clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100);
        self.history.begin("Add step hit");
        let _ = self.history.edit(true, |h| {
            let id = nearest_moment(h, target.ms(), 4.0)
                .or_else(|| nearest_moment(h, ms, 4.0))
                .unwrap_or_else(|| h.add_moment(target, volume));
            h.add_hit_with_volume(id, lane.clone(), volume);
            Ok::<_, Infallible>(())
        });
        self.history.commit();
        self.audition_requests.push(lane);
        true
    }
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
                if let Some(row) = self.lanes.iter().position(|l| l == lane)
                    && (first..=last).contains(&(row as i32)) {
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
            steps.truncate(MAX_PAINT_STROKE_STEPS);
            if steps.is_empty() {
                steps.push(end);
            }
            return steps;
        }
        let dir = if forward { 1 } else { -1 };
        let mut steps = Vec::new();
        let mut cur = start;
        for _ in 0..MAX_PAINT_STROKE_STEPS {
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
        if steps.len() < MAX_PAINT_STROKE_STEPS
            && steps.last().is_none_or(|last| (last.ms() - end.ms()).abs() >= 1e-6)
        {
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
        self.move_stroke = None;
        self.history.begin(label);
    }
    fn commit(&mut self) {
        self.move_stroke = None;
        self.history.commit();
    }
    fn cancel(&mut self) {
        self.move_stroke = None;
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
        if items.is_empty() || lanes.is_empty() {
            return;
        }
        let primary = self.selected.primary().cloned();
        if self.move_stroke.is_none() {
            let base_hits = self.hitsounds().clone();
            let mut values: Vec<_> = items
                .iter()
                .filter_map(|k| {
                    let moment = base_hits.moments().get(k.moment)?;
                    if !moment.has_hit(&k.lane) {
                        return None;
                    }
                    Some((k.clone(), moment.time().ms(), moment.hit_volume(&k.lane)))
                })
                .collect();
            if values.is_empty() {
                return;
            }
            values.sort_unstable_by(|(a, _, _), (b, _, _)| {
                (Some(a) != primary.as_ref(), a.moment, &a.lane)
                    .cmp(&(Some(b) != primary.as_ref(), b.moment, &b.lane))
            });
            self.move_stroke = Some((base_hits, values, 0.0, 0));
        }
        let Some((base_hits, values, total_delta, total_rows)) = self.move_stroke.as_mut() else {
            return;
        };
        *total_delta += delta;
        *total_rows += rows;
        let base_snapshot = base_hits.clone();
        let snapshot_values = values.clone();
        let applied_delta = *total_delta;
        let applied_rows = *total_rows;
        let mut new = Vec::new();
        let _ = self.history.edit(true, |h| {
            *h = base_snapshot;
            for (k, _, _) in &snapshot_values {
                h.remove_hit(MomentId(k.moment), &k.lane);
            }
            for (k, t, v) in snapshot_values {
                let old = lanes.iter().position(|l| *l == k.lane).unwrap_or(0) as i32;
                let mut row = (old + applied_rows).clamp(0, lanes.len() as i32 - 1);
                while applied_rows != 0 && h.is_lane_hidden(&lanes[row as usize]) {
                    let next = row + applied_rows.signum();
                    if !(0..lanes.len() as i32).contains(&next) {
                        row = old;
                        break;
                    }
                    row = next;
                }
                let lane = lanes[row as usize].clone();
                let moment = destination_at(h, (t + applied_delta).max(0.0), v, 5.0);
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
    fn insert_value(&self) -> i32 {
        self.insert_volume
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

    #[test]
    fn set_moment_volume_at_updates_nearest_moment_and_supports_undo() {
        let mut composer = empty_composer();
        let normal = lane_normal();
        composer.add(1000.0, normal.clone(), 80, 5.0);
        composer.add(2000.0, normal, 80, 5.0);

        // Updates the nearest moment within 5 ms without requiring selection, clamping to 5..100.
        assert!(composer.set_moment_volume_at(1002.0, 42, None));
        assert_eq!(composer.hitsounds().moments()[0].reference_volume(), 42);
        assert_eq!(composer.hitsounds().moments()[1].reference_volume(), 80);

        assert!(composer.set_moment_volume_at(2000.0, 1, None));
        assert_eq!(composer.hitsounds().moments()[1].reference_volume(), 5);

        // Undo reverts the last inline volume edit cleanly.
        composer.undo();
        assert_eq!(composer.hitsounds().moments()[1].reference_volume(), 80);

        // Missing moment returns false cleanly.
        assert!(!composer.set_moment_volume_at(9999.0, 50, None));
    }
}
