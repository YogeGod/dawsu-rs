use super::{Composer, HitKey, normal};
use dawsu_editor::tools::ToolTarget;
use dawsu_foundation::{SampleIdentity, SongTime};
use dawsu_hitsounding::{Hitsounds, MomentId, Place};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
};

#[derive(Clone, Copy)]
pub enum RangeOperation {
    Copy,
    Move,
    Delete,
}

#[derive(Clone, Copy)]
pub enum VolumeScope {
    Selected,
    SelectedLanes,
    All,
}

impl Composer {
    pub(super) fn selected_keys_in_order(&self) -> Vec<HitKey> {
        let mut keys: Vec<_> = self.selected.items().iter().cloned().collect();
        let primary = self.selected.primary();
        keys.sort_unstable_by(|a, b| {
            (Some(a) != primary, a.moment, &a.lane).cmp(&(Some(b) != primary, b.moment, &b.lane))
        });
        keys
    }

    pub fn quantize(&mut self) {
        self.quantize_with_strength(100.0);
    }

    pub fn quantize_with_strength(&mut self, strength: f64) -> bool {
        if !strength.is_finite() {
            return false;
        }
        let strength = strength.clamp(1.0, 100.0) / 100.0;
        let hits = self.hitsounds();
        let mut values: Vec<_> = self
            .selected_keys_in_order()
            .into_iter()
            .filter_map(|key| {
                let moment = hits
                    .moments()
                    .get(key.moment)
                    .filter(|moment| moment.has_hit(&key.lane))?;
                let time = moment.time();
                let target = self.tempo.snap(time, self.spacing);
                let time_shift = (target.ms() - time.ms()) * strength;
                let moved = (moment.place() == Place::NoObject && time_shift.abs() > 1e-6)
                    .then(|| (time.ms() + time_shift, moment.hit_volume(&key.lane)));
                Some((key, moved))
            })
            .collect();
        if !values.iter().any(|(_, moved)| moved.is_some()) {
            return false;
        }
        self.apply("Quantize hits", |hits| {
            for (key, moved) in &values {
                if moved.is_some() {
                    hits.remove_hit(MomentId(key.moment), &key.lane);
                }
            }
            let mut destinations = Destinations::new(hits);
            for (key, moved) in &mut values {
                if let Some((time, volume)) = moved {
                    key.moment = destinations
                        .place(hits, *time, key.lane.clone(), *volume, 0.0)
                        .0;
                }
            }
        });
        self.selected
            .replace(values.into_iter().map(|(key, _)| key));
        true
    }

    pub fn move_selection_start(&mut self, position: f64) -> bool {
        if !position.is_finite() {
            return false;
        }
        let keys = self.selected_keys_in_order();
        let Some(start) = keys
            .iter()
            .filter_map(|key| {
                self.hitsounds()
                    .moments()
                    .get(key.moment)
                    .filter(|moment| moment.has_hit(&key.lane))
                    .map(|moment| moment.time().ms())
            })
            .reduce(f64::min)
        else {
            return false;
        };
        let offset = position.max(0.0) - start;
        offset.abs() >= 0.5 && self.offset_hits(&keys, &[offset], true, "Move selection start")
    }

    pub fn select_across_lanes(&mut self, rows: &[usize], direction: isize, max_distance: f64) {
        let keys = self.selected_keys_in_order();
        let moments = self.hitsounds().moments();
        let Some((anchor, time)) = keys
            .iter()
            .filter_map(|key| {
                let moment = moments.get(key.moment)?;
                moment.has_hit(&key.lane).then(|| (key, moment.time().ms()))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
        else {
            return;
        };
        let Some(start) = rows
            .iter()
            .position(|&row| self.lanes.get(row) == Some(&anchor.lane))
        else {
            return;
        };
        let mut rank = HashMap::new();
        for offset in 1..rows.len() {
            let Some(row) = start
                .checked_add_signed(offset as isize * direction)
                .and_then(|index| rows.get(index))
            else {
                break;
            };
            if let Some(lane) = self.lanes.get(*row).filter(|lane| *lane != &anchor.lane) {
                rank.entry(lane).or_insert(offset);
            }
        }

        let mut closest = None;
        for (index, moment) in moments.iter().enumerate() {
            let distance = (moment.time().ms() - time).abs();
            if distance > max_distance {
                continue;
            }
            for lane in moment.hits() {
                let Some(&lane_rank) = rank.get(lane) else {
                    continue;
                };
                let score = (lane_rank, distance, index);
                if closest.as_ref().is_none_or(|(best, _)| score < *best) {
                    closest = Some((score, lane));
                }
            }
        }
        if let Some(((_, _, moment), lane)) = closest {
            let hit = HitKey {
                moment,
                lane: lane.clone(),
            };
            if !self.selected.contains(&hit) {
                self.selected.replace(keys.into_iter().chain([hit]));
            }
        }
    }

    pub fn duplicate(&mut self) {
        let Some((start, step, gap)) = self.selection_gap() else {
            return;
        };
        let shift = if self.spacing.is_long() {
            self.step_forward(start, gap).ms() - start.ms()
        } else {
            step * f64::from(gap)
        };
        let keys = self.selected_keys_in_order();
        self.offset_hits(&keys, &[shift], false, "Duplicate hits");
    }

    pub fn repeat_gap(&self) -> Option<u32> {
        self.selection_gap().map(|(_, _, gap)| gap.min(1024))
    }

    fn selection_gap(&self) -> Option<(SongTime, f64, u32)> {
        let times = self
            .selected
            .items()
            .iter()
            .filter_map(|key| self.hitsounds().moments().get(key.moment))
            .map(|moment| moment.time().ms());
        let first = times.clone().reduce(f64::min)?;
        let last = times.reduce(f64::max)?;
        let start = SongTime::from_ms(first);
        let step = self.spacing.step_ms(self.tempo.beat_length_at(start));
        let gap = (((last - first) / step + 0.5).ceil() as u32).max(1);
        Some((start, step, gap))
    }

    fn step_forward(&self, mut cursor: SongTime, count: u32) -> SongTime {
        for _ in 0..count {
            cursor = self.tempo.grid_step(cursor, 1, self.spacing);
        }
        cursor
    }

    pub fn repeat_selected(&mut self, copies: u32, gap_steps: u32) -> bool {
        if !(1..=64).contains(&copies) || !(1..=1024).contains(&gap_steps) {
            return false;
        }
        let originals = self.selected_keys_in_order();
        let Some(first) = originals
            .iter()
            .filter_map(|key| self.hitsounds().moments().get(key.moment))
            .map(|moment| moment.time().ms())
            .reduce(f64::min)
        else {
            return false;
        };
        let mut cursor = SongTime::from_ms(first);
        let shifts: Vec<_> = (0..copies)
            .map(|_| {
                cursor = self.step_forward(cursor, gap_steps);
                cursor.ms() - first
            })
            .collect();
        self.offset_hits(&originals, &shifts, false, "Repeat hits")
    }

    pub fn edit_marked_range(
        &mut self,
        operation: RangeOperation,
        offset: f64,
        rows: Option<&[usize]>,
    ) -> bool {
        let Some(range) = self.range else {
            return false;
        };
        if !offset.is_finite() {
            return false;
        }
        let originals = if let Some(rows) = rows {
            let lanes: HashSet<_> = rows.iter().filter_map(|row| self.lanes.get(*row)).collect();
            self.in_range(range.from, range.to, i32::MIN, i32::MAX)
                .into_iter()
                .filter(|key| lanes.contains(&key.lane))
                .collect()
        } else {
            self.in_range(range.from, range.to, range.from_row, range.to_row)
        };
        let originals: Vec<_> = originals
            .into_iter()
            .filter(|key| self.hitsounds().moments()[key.moment].time().ms() < range.to.ms())
            .collect();
        match operation {
            RangeOperation::Copy => self.offset_hits(&originals, &[offset], false, "Copy range"),
            RangeOperation::Move => self.offset_hits(&originals, &[offset], true, "Move range"),
            RangeOperation::Delete => {
                let remove: Vec<_> = originals
                    .into_iter()
                    .filter(|key| !self.protect_normals || !normal(&key.lane))
                    .collect();
                if remove.is_empty() {
                    return false;
                }
                self.apply("Delete range", |hits| {
                    for key in remove {
                        hits.remove_hit(MomentId(key.moment), &key.lane);
                    }
                });
                self.selected.clear();
                true
            }
        }
    }

    fn offset_hits(&mut self, keys: &[HitKey], shifts: &[f64], remove: bool, label: &str) -> bool {
        let originals: Vec<_> = keys
            .iter()
            .filter_map(|key| {
                let moment = self.hitsounds().moments().get(key.moment)?;
                moment
                    .has_hit(&key.lane)
                    .then(|| (key, moment.time().ms(), moment.hit_volume(&key.lane)))
            })
            .collect();
        if originals.is_empty() || shifts.iter().all(|shift| shift.abs() < 1e-6) {
            return false;
        }
        let mut selected = Vec::new();
        self.apply(label, |hits| {
            if remove {
                for (key, _, _) in &originals {
                    hits.remove_hit(MomentId(key.moment), &key.lane);
                }
            }
            let mut destinations = Destinations::new(hits);
            for shift in shifts {
                for &(key, time, volume) in &originals {
                    let lane = key.lane.clone();
                    let moment = destinations
                        .place(hits, (time + shift).max(0.0), lane.clone(), volume, 5.0)
                        .0;
                    selected.push(HitKey { moment, lane });
                }
            }
        });
        self.selected.replace(selected);
        true
    }

    pub fn scale_by_volume(
        &mut self,
        scope: VolumeScope,
        min: i32,
        max: i32,
        percent: f64,
    ) -> bool {
        let (min, max) = (min.min(max), min.max(max));
        if !(0..=100).contains(&min)
            || !(min..=100).contains(&max)
            || !percent.is_finite()
            || percent < 0.0
        {
            return false;
        }
        let selected_moments: HashSet<_> =
            self.selected.items().iter().map(|hit| hit.moment).collect();
        let selected_lanes: HashSet<_> =
            self.selected.items().iter().map(|hit| &hit.lane).collect();
        // A silent sound stays silent; the 1% floor is for sounds above 0.
        let scale = |volume: i32| {
            if volume == 0 {
                return 0;
            }
            (f64::from(volume) * percent / 100.0)
                .round()
                .clamp(1.0, 100.0) as i32
        };
        // "Lanes of selected hits" changes only those lanes, each picked by its own volume.
        let lanes_only = matches!(scope, VolumeScope::SelectedLanes);
        let changes: Vec<_> = self
            .hitsounds()
            .moments()
            .iter()
            .enumerate()
            .filter_map(|(index, moment)| {
                let before = moment.volume();
                let volume = scale(before);
                // Every sound is multiplied by the same factor, so their balance stays.
                let gains: Vec<_> = moment
                    .hits()
                    .filter(|lane| {
                        !lanes_only
                            || selected_lanes.contains(lane)
                                && (min..=max).contains(&moment.hit_volume(lane))
                    })
                    .map(|lane| (lane.clone(), scale(moment.hit_volume(lane))))
                    .collect();
                let in_scope = match scope {
                    VolumeScope::Selected => selected_moments.contains(&index),
                    VolumeScope::SelectedLanes => {
                        moment.hits().any(|lane| selected_lanes.contains(lane))
                    }
                    VolumeScope::All => moment.hits().next().is_some(),
                };
                (moment.place() != Place::NoObject
                    && (lanes_only || (min..=max).contains(&before))
                    && in_scope
                    && (!lanes_only && volume != before
                        || gains.iter().any(|(lane, gain)| *gain != moment.hit_volume(lane))))
                    .then_some((MomentId(index), (!lanes_only).then_some(volume), gains))
            })
            .collect();
        if changes.is_empty() {
            return false;
        }
        self.apply("Scale by volume", |hits| {
            for (id, volume, gains) in changes {
                if let Some(volume) = volume {
                    hits.set_volume(id, volume);
                }
                for (lane, gain) in gains {
                    hits.set_hit_volume(id, &lane, gain);
                }
            }
        });
        true
    }
}

/// An ordered index lets large repeats reuse nearby objects without rescanning every moment for each hit.
struct Destinations {
    exact: BTreeMap<EditTime, MomentId>,
    objects: BTreeMap<EditTime, MomentId>,
}

impl Destinations {
    fn new(hits: &Hitsounds) -> Self {
        let mut exact = BTreeMap::new();
        let mut objects = BTreeMap::new();
        for (index, moment) in hits.moments().iter().enumerate() {
            let key = EditTime(moment.time().ms());
            let id = MomentId(index);
            if moment.place() != Place::NoObject {
                objects.entry(key).or_insert(id);
            }
            if moment.place() != Place::NoObject || moment.hits().next().is_some() {
                exact.entry(key).or_insert(id);
            }
        }
        Self { exact, objects }
    }

    fn place(
        &mut self,
        hits: &mut Hitsounds,
        time: f64,
        lane: SampleIdentity,
        volume: i32,
        tolerance: f64,
    ) -> MomentId {
        let key = EditTime(time);
        let id = self
            .exact
            .get(&key)
            .copied()
            .or_else(|| {
                self.objects
                    .range(EditTime(time - tolerance)..=EditTime(time + tolerance))
                    .min_by(|(a, ai), (b, bi)| {
                        (a.0 - time)
                            .abs()
                            .total_cmp(&(b.0 - time).abs())
                            .then(ai.0.cmp(&bi.0))
                    })
                    .map(|(_, &id)| id)
            })
            .unwrap_or_else(|| {
                let id = hits.add_moment(SongTime::from_ms(time), volume);
                self.exact.insert(key, id);
                id
            });
        hits.add_hit_with_volume(id, lane, volume);
        id
    }
}

#[derive(Clone, Copy)]
struct EditTime(f64);

impl PartialEq for EditTime {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl Eq for EditTime {}

impl PartialOrd for EditTime {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EditTime {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}
