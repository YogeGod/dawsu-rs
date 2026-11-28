//! The batch step (composer TDD R23): many generated hits added as one undo step. Drum hit
//! detection uses it today; Record and the node graph can reuse it.
//!
//! [`Composer::plan_batch`] and [`Composer::add_batch`] share one planner, so the counts a review
//! shows are the counts Apply gives. Grid lines arrive with each hit ([`BatchHit::grid`]): the
//! caller measures them against the map's grid, so this step does no grid maths of its own.

use crate::{Composer, HitKey, normal};
use dawsu_editor::tempo::SnapSpacing;
use dawsu_foundation::{SampleIdentity, SongTime};
use dawsu_hitsounding::{MomentId, Place};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;

/// One generated hit.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchHit {
    /// Song time in ms, as detected.
    pub time: f64,
    pub lane: SampleIdentity,
    /// Stronger hits are placed first and win a duplicate.
    pub strength: f64,
    /// The grid line this hit snaps to, already corrected for lag, or `None` when it is too far from
    /// every allowed line.
    pub grid: Option<f64>,
}

/// What happens to hits that land on no object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Yoge's default (2026-11-20): an object within `object_ms`, else the hit's grid line, else
    /// flagged and left out.
    Snap,
    /// 3.1.4's auto-fill (`DrumAutoFillPlan.Create`), kept for parity: with `filter_grid`, hits off
    /// the grid are dropped and the rest move to their line first; hits on no object are counted as
    /// unmatched and added only with `include_unmatched`.
    Classic { filter_grid: bool, include_unmatched: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchOptions {
    /// How far a hit may be from an object and still land on it.
    pub object_ms: f64,
    /// Hits on one lane this close count as the same hit; loose hits this close share a moment.
    pub dedupe_ms: f64,
    /// The detections' constant lag behind the grid, taken off before matching objects.
    pub shift_ms: f64,
    /// Hits and objects from the start (inclusive) to the end (exclusive) only.
    pub range: Option<(f64, f64)>,
    pub policy: Policy,
}

impl Default for BatchOptions {
    fn default() -> Self {
        Self { object_ms: 30.0, dedupe_ms: 2.0, shift_ms: 0.0, range: None, policy: Policy::Snap }
    }
}

/// Where one hit lands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lands {
    /// On this object moment.
    Object(MomentId),
    /// On a moment with no object, at the row's time.
    Loose,
    /// Too far from the grid; left out (Snap).
    Flagged,
    /// Off the grid; dropped (Classic).
    OffGrid,
    /// On no object; dropped (Classic without unmatched).
    Unmatched,
    /// The lane already has a hit here.
    Duplicate,
    /// Outside the range, or a time or strength that is not a number.
    Skipped,
}

/// One input hit's outcome; `hit` indexes the input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatchRow {
    pub hit: usize,
    pub time: f64,
    pub lands: Lands,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchCounts {
    pub added: usize,
    pub on_objects: usize,
    pub loose: usize,
    /// Hits that found no object (3.1.4's counter).
    pub unmatched: usize,
    pub duplicates: usize,
    pub flagged: usize,
    pub off_grid: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatchPlan {
    pub rows: Vec<BatchRow>,
    pub counts: BatchCounts,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatchReport {
    pub counts: BatchCounts,
    /// The added hits, now the selection.
    pub added: Vec<HitKey>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BatchRefusal {
    /// An undo step or a stroke is open; the batch would merge into it.
    StepOpen,
    /// A target lane no longer exists (the batch never adds lanes).
    MissingLane(SampleIdentity),
}

#[derive(Clone, Copy)]
enum Target {
    Existing(MomentId),
    New(usize),
}

struct Planned {
    plan: BatchPlan,
    targets: Vec<(usize, Target)>,
    new_moments: Vec<f64>,
}

impl Composer {
    /// The editor's grid for a batch: the nearest line at 1/`divisor` (or the long snap's beats when
    /// set) and that line spacing in ms. Before the first red line the spacing is infinite, so
    /// nothing there counts as off the grid (3.1.4).
    pub fn grid_line(&self, time: f64, divisor: u32) -> (f64, f64) {
        let spacing = SnapSpacing::new(self.spacing.beats(), divisor.max(1))
            .or_else(|_| SnapSpacing::fraction(divisor.max(1)))
            .expect("a divisor of at least 1");
        let snapped = self.tempo.snap(SongTime::from_ms(time), spacing).ms();
        let mapped = self.tempo.sections().first().is_some_and(|first| first.start.ms() <= time);
        let step = if mapped { spacing.step_ms(self.tempo.beat_length_at(SongTime::from_ms(time))) } else { f64::INFINITY };
        (snapped, step)
    }

    /// Where each hit would land, read-only.
    pub fn plan_batch(&self, hits: &[BatchHit], options: &BatchOptions) -> BatchPlan {
        self.planned(hits, options).plan
    }

    fn planned(&self, hits: &[BatchHit], options: &BatchOptions) -> Planned {
        let model = self.hitsounds();
        let inside = |time: f64| options.range.is_none_or(|(from, to)| time >= from && time < to);
        // Sorted by time, so each hit finds its nearest object by a binary search: the drum window
        // re-plans thousands of hits after every edit.
        let mut objects: Vec<(MomentId, f64)> = model.moments().iter().enumerate()
            .filter(|(_, m)| m.place() != Place::NoObject && inside(m.time().ms()))
            .map(|(i, m)| (MomentId(i), m.time().ms())).collect();
        objects.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.0.cmp(&b.0.0)));
        let loose_existing: Vec<(MomentId, f64)> = model.moments().iter().enumerate()
            .filter(|(_, m)| m.place() == Place::NoObject)
            .map(|(i, m)| (MomentId(i), m.time().ms())).collect();
        // Every hit already there and every hit this batch places: sorted times per lane, for duplicates.
        let mut taken: HashMap<&SampleIdentity, Vec<f64>> = HashMap::new();
        for m in model.moments() {
            for lane in m.hits() {
                taken.entry(lane).or_default().push(m.time().ms());
            }
        }
        for times in taken.values_mut() {
            times.sort_by(f64::total_cmp);
        }
        let near_any = |times: &[f64], time: f64| {
            let at = times.partition_point(|t| *t < time - options.dedupe_ms);
            times.get(at).is_some_and(|t| (t - time).abs() <= options.dedupe_ms)
        };
        let mut new_moments: Vec<f64> = Vec::new();
        let mut rows: Vec<BatchRow> = hits.iter().enumerate()
            .map(|(hit, h)| BatchRow { hit, time: h.time, lands: Lands::Skipped }).collect();
        let mut targets = Vec::new();
        let mut counts = BatchCounts::default();
        let mut order: Vec<usize> = (0..hits.len()).collect();
        order.sort_by(|&a, &b| hits[b].strength.total_cmp(&hits[a].strength).then(hits[a].time.total_cmp(&hits[b].time)));
        let nearest_object = |time: f64| {
            let at = objects.partition_point(|o| o.1 < time);
            // The last object before the time (the first at that time) and the first after it.
            let before = at.checked_sub(1).map(|i| objects.partition_point(|o| o.1 < objects[i].1));
            [before, Some(at)].into_iter().flatten().filter_map(|i| objects.get(i))
                .map(|&(id, t)| (id, t, (t - time).abs()))
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .filter(|o| o.2 <= options.object_ms)
        };
        for index in order {
            let hit = &hits[index];
            if !hit.time.is_finite() || !hit.strength.is_finite() || hit.time < 0.0 || !inside(hit.time) {
                continue;
            }
            let (time, object) = match options.policy {
                Policy::Snap => match nearest_object(hit.time - options.shift_ms) {
                    Some((id, t, _)) => (t, Some(id)),
                    None => {
                        counts.unmatched += 1;
                        // A grid line outside the range is as good as none: flagged, never placed there.
                        match hit.grid.filter(|line| inside(*line)) {
                            Some(line) => (line, None),
                            None => {
                                counts.flagged += 1;
                                rows[index].lands = Lands::Flagged;
                                continue;
                            }
                        }
                    }
                },
                Policy::Classic { filter_grid, include_unmatched } => {
                    let time = match (filter_grid, hit.grid) {
                        (false, _) => hit.time,
                        (true, Some(line)) => line,
                        (true, None) => {
                            counts.off_grid += 1;
                            rows[index].lands = Lands::OffGrid;
                            continue;
                        }
                    };
                    match nearest_object(time) {
                        Some((id, t, _)) => (t, Some(id)),
                        None => {
                            counts.unmatched += 1;
                            if !include_unmatched {
                                rows[index].time = time;
                                rows[index].lands = Lands::Unmatched;
                                continue;
                            }
                            (time, None)
                        }
                    }
                }
            };
            rows[index].time = time;
            if taken.get(&hit.lane).is_some_and(|times| near_any(times, time)) {
                counts.duplicates += 1;
                rows[index].lands = Lands::Duplicate;
                continue;
            }
            let target = match object {
                Some(id) => {
                    counts.on_objects += 1;
                    rows[index].lands = Lands::Object(id);
                    Target::Existing(id)
                }
                None => {
                    counts.loose += 1;
                    rows[index].lands = Lands::Loose;
                    // The nearest no-object moment within the dedupe distance, made earlier or in this batch.
                    let existing = loose_existing.iter().map(|&(id, t)| (Target::Existing(id), t));
                    let made = new_moments.iter().enumerate().map(|(n, &t)| (Target::New(n), t));
                    match existing.chain(made).filter(|(_, t)| (t - time).abs() <= options.dedupe_ms)
                        .min_by(|a, b| (a.1 - time).abs().total_cmp(&(b.1 - time).abs())) {
                        Some((target, t)) => {
                            rows[index].time = t;
                            target
                        }
                        None => {
                            new_moments.push(time);
                            Target::New(new_moments.len() - 1)
                        }
                    }
                }
            };
            let times = taken.entry(&hit.lane).or_default();
            let placed = rows[index].time;
            times.insert(times.partition_point(|t| *t < placed), placed);
            counts.added += 1;
            targets.push((index, target));
        }
        Planned { plan: BatchPlan { rows, counts }, targets, new_moments }
    }

    /// Adds the planned hits as one undo step labelled `label` and selects them. Adds no step when
    /// nothing lands. Refused while a step is open or when a target lane is gone.
    pub fn add_batch(&mut self, label: &str, hits: &[BatchHit], options: &BatchOptions)
        -> Result<BatchReport, BatchRefusal> {
        if self.history.is_transaction_open() {
            return Err(BatchRefusal::StepOpen);
        }
        if let Some(missing) = hits.iter().find(|h| self.hitsounds().lane(&h.lane).is_none()) {
            return Err(BatchRefusal::MissingLane(missing.lane.clone()));
        }
        let Planned { plan, targets, new_moments } = self.planned(hits, options);
        if targets.is_empty() {
            return Ok(BatchReport { counts: plan.counts, added: Vec::new() });
        }
        let volume = self
            .insert_volume
            .clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100);
        let mut added = Vec::new();
        let _ = self.history.apply(label, true, |h| {
            let created: Vec<MomentId> = new_moments.iter().map(|&t| h.add_moment(SongTime::from_ms(t), volume)).collect();
            for (index, target) in &targets {
                let id = match *target {
                    Target::Existing(id) => id,
                    Target::New(n) => created[n],
                };
                h.add_hit(id, hits[*index].lane.clone());
                added.push(HitKey { moment: id.0, lane: hits[*index].lane.clone() });
            }
            Ok::<_, Infallible>(())
        });
        for key in &added {
            if !self.lanes.contains(&key.lane) {
                self.lanes.push(key.lane.clone());
            }
        }
        self.selected.replace(added.clone());
        Ok(BatchReport { counts: plan.counts, added })
    }

    /// Default step in ms for [`Self::pattern_stamp`]: the active bar length at the start of the
    /// selection, scaled up to whole bars if the selection spans more than one bar.
    pub fn pattern_stamp_step(&self) -> Option<f64> {
        let times = self
            .selected
            .items()
            .iter()
            .filter_map(|key| {
                self.hitsounds()
                    .moments()
                    .get(key.moment)
                    .filter(|m| m.has_hit(&key.lane))
            })
            .map(|m| m.time().ms());
        let first = times.clone().reduce(f64::min)?;
        let last = times.reduce(f64::max)?;
        let section = self.tempo.section_at(SongTime::from_ms(first));
        let bar_ms = section.beat_length_ms * f64::from(section.beats_per_bar.max(1));
        if !bar_ms.is_finite() || bar_ms <= 0.0 {
            return None;
        }
        let bars = (((last - first) / bar_ms).floor().max(0.0) as u32) + 1;
        Some(bar_ms * f64::from(bars))
    }

    /// Stamps the currently selected hits as a relative-offset groove `options.repeat_count` times
    /// after the selection, optionally snapping to nearby gameplay objects and quantizing volumes
    /// to 3 clean levels (`35%`, `70%`, `100%`). Everything applies as one `"Pattern stamp"` undo
    /// step and selects the stamped hits. Returns the number of hits added.
    pub fn pattern_stamp(&mut self, options: &PatternStampOptions) -> usize {
        if self.history.is_transaction_open()
            || options.repeat_count == 0
            || options.repeat_count > 256
        {
            return 0;
        }
        let selected_hits: Vec<(f64, SampleIdentity, i32)> = self
            .selected_keys_in_order()
            .into_iter()
            .filter_map(|key| {
                let moment = self.hitsounds().moments().get(key.moment)?;
                moment.has_hit(&key.lane).then(|| {
                    let volume = moment.hit_volume(&key.lane);
                    (moment.time().ms(), key.lane, volume)
                })
            })
            .collect();
        if selected_hits.is_empty() {
            return 0;
        }
        let Some(first) = selected_hits.iter().map(|(t, _, _)| *t).reduce(f64::min) else {
            return 0;
        };
        let step_ms = if options.step_ms.is_finite() && options.step_ms > 0.0 {
            options.step_ms
        } else {
            let Some(inferred) = self.pattern_stamp_step() else {
                return 0;
            };
            inferred
        };
        if !step_ms.is_finite() || step_ms <= 0.0 {
            return 0;
        }
        let leniency = if options.object_leniency_ms.is_finite() && options.object_leniency_ms >= 0.0 {
            options.object_leniency_ms
        } else {
            15.0
        };
        let pattern: Vec<(f64, SampleIdentity, i32)> = selected_hits
            .into_iter()
            .map(|(time, lane, vol)| {
                let volume = if options.quantize_volumes {
                    quantize_stamp_volume(vol)
                } else {
                    vol
                };
                (time - first, lane, volume)
            })
            .collect();
        let pattern_lanes: HashSet<SampleIdentity> =
            pattern.iter().map(|(_, lane, _)| lane.clone()).collect();

        let model = self.hitsounds();
        let span_start = first + step_ms - 1.0;
        let span_end = first + (options.repeat_count as f64 + 1.0) * step_ms - 1.0;
        let mut clear_targets: Vec<(MomentId, SampleIdentity)> = Vec::new();
        let mut cleared_set: HashSet<(usize, SampleIdentity)> = HashSet::new();
        if options.clear_existing {
            for (index, moment) in model.moments().iter().enumerate() {
                let t = moment.time().ms();
                if t >= span_start && t < span_end {
                    for lane in moment.hits() {
                        if pattern_lanes.contains(lane) && (!self.protect_normals || !normal(lane)) {
                            clear_targets.push((MomentId(index), lane.clone()));
                            cleared_set.insert((index, lane.clone()));
                        }
                    }
                }
            }
        }

        let mut objects: Vec<(MomentId, f64)> = model
            .moments()
            .iter()
            .enumerate()
            .filter(|(_, m)| m.place() != Place::NoObject)
            .map(|(i, m)| (MomentId(i), m.time().ms()))
            .collect();
        objects.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.0.cmp(&b.0.0)));

        let all_existing: Vec<(MomentId, f64)> = model
            .moments()
            .iter()
            .enumerate()
            .filter(|(_, m)| m.place() != Place::NoObject || m.hits().next().is_some())
            .map(|(i, m)| (MomentId(i), m.time().ms()))
            .collect();
        let loose_existing: Vec<(MomentId, f64)> = model
            .moments()
            .iter()
            .enumerate()
            .filter(|(_, m)| m.place() == Place::NoObject)
            .map(|(i, m)| (MomentId(i), m.time().ms()))
            .collect();

        let mut taken: HashMap<SampleIdentity, Vec<f64>> = HashMap::new();
        for (index, moment) in model.moments().iter().enumerate() {
            for lane in moment.hits() {
                if !cleared_set.contains(&(index, lane.clone())) {
                    taken.entry(lane.clone()).or_default().push(moment.time().ms());
                }
            }
        }
        for times in taken.values_mut() {
            times.sort_by(f64::total_cmp);
        }

        let dedupe_ms = 1.0;
        let near_any = |times: &[f64], time: f64| {
            let at = times.partition_point(|t| *t < time - dedupe_ms);
            times.get(at).is_some_and(|t| (t - time).abs() <= dedupe_ms)
        };
        let nearest_object = |time: f64| {
            let at = objects.partition_point(|o| o.1 < time);
            let before = at
                .checked_sub(1)
                .map(|i| objects.partition_point(|o| o.1 < objects[i].1));
            [before, Some(at)]
                .into_iter()
                .flatten()
                .filter_map(|i| objects.get(i))
                .map(|&(id, t)| (id, t, (t - time).abs()))
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .filter(|o| o.2 <= leniency)
        };

        let mut new_moments: Vec<(f64, i32)> = Vec::new();
        let mut placements: Vec<(Target, SampleIdentity, i32)> = Vec::new();

        for copy in 1..=options.repeat_count {
            let base = first + (copy as f64) * step_ms;
            for (offset, lane, volume) in &pattern {
                let raw_time = base + offset;
                if !raw_time.is_finite() || raw_time < 0.0 {
                    continue;
                }
                let (target, placed_time) = if options.snap_to_objects {
                    match nearest_object(raw_time) {
                        Some((id, t, _)) => (Target::Existing(id), t),
                        None => {
                            let existing = loose_existing
                                .iter()
                                .map(|&(id, t)| (Target::Existing(id), t));
                            let made = new_moments
                                .iter()
                                .enumerate()
                                .map(|(n, &(t, _))| (Target::New(n), t));
                            match existing
                                .chain(made)
                                .filter(|(_, t)| (t - raw_time).abs() <= dedupe_ms)
                                .min_by(|a, b| (a.1 - raw_time).abs().total_cmp(&(b.1 - raw_time).abs()))
                            {
                                Some(found) => found,
                                None => {
                                    new_moments.push((raw_time, *volume));
                                    (Target::New(new_moments.len() - 1), raw_time)
                                }
                            }
                        }
                    }
                } else {
                    let existing = all_existing
                        .iter()
                        .map(|&(id, t)| (Target::Existing(id), t));
                    let made = new_moments
                        .iter()
                        .enumerate()
                        .map(|(n, &(t, _))| (Target::New(n), t));
                    match existing
                        .chain(made)
                        .filter(|(_, t)| (t - raw_time).abs() <= 0.5)
                        .min_by(|a, b| (a.1 - raw_time).abs().total_cmp(&(b.1 - raw_time).abs()))
                    {
                        Some(found) => found,
                        None => {
                            new_moments.push((raw_time, *volume));
                            (Target::New(new_moments.len() - 1), raw_time)
                        }
                    }
                };
                if taken.get(lane).is_some_and(|times| near_any(times, placed_time)) {
                    continue;
                }
                let times = taken.entry(lane.clone()).or_default();
                times.insert(times.partition_point(|t| *t < placed_time), placed_time);
                placements.push((target, lane.clone(), *volume));
            }
        }

        if placements.is_empty() && clear_targets.is_empty() {
            return 0;
        }

        let mut added = Vec::with_capacity(placements.len());
        let _ = self.history.apply("Pattern stamp", true, |h| {
            for (id, lane) in &clear_targets {
                h.remove_hit(*id, lane);
            }
            let created: Vec<MomentId> = new_moments
                .iter()
                .map(|&(t, vol)| h.add_moment(SongTime::from_ms(t), vol))
                .collect();
            for (target, lane, volume) in &placements {
                let id = match *target {
                    Target::Existing(id) => id,
                    Target::New(n) => created[n],
                };
                h.add_hit_with_volume(id, lane.clone(), *volume);
                added.push(HitKey { moment: id.0, lane: lane.clone() });
            }
            Ok::<_, Infallible>(())
        });
        for key in &added {
            if !self.lanes.contains(&key.lane) {
                self.lanes.push(key.lane.clone());
            }
        }
        if !added.is_empty() {
            self.selected.replace(added.clone());
        }
        added.len()
    }
}

/// Options for [`Composer::pattern_stamp`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatternStampOptions {
    /// How many times to stamp the selected pattern after the selection.
    pub repeat_count: usize,
    /// Step between consecutive stamps in ms. When `<= 0.0` (or non-finite), inferred from the
    /// active bar length at the start of the selection.
    pub step_ms: f64,
    /// Snap each stamped hit to the nearest gameplay object within `object_leniency_ms`.
    pub snap_to_objects: bool,
    /// Maximum distance in ms to snap a stamped hit onto a gameplay object (default 15 ms).
    pub object_leniency_ms: f64,
    /// Quantize stamped hit volumes to 3 clean levels (`35%`, `70%`, `100%`) so osu!stable green
    /// lines stay minimal.
    pub quantize_volumes: bool,
    /// Clear existing hits on the pattern's lanes across the stamped span before placing.
    pub clear_existing: bool,
}

impl Default for PatternStampOptions {
    fn default() -> Self {
        Self {
            repeat_count: 4,
            step_ms: 0.0,
            snap_to_objects: true,
            object_leniency_ms: 15.0,
            quantize_volumes: false,
            clear_existing: false,
        }
    }
}

/// Snaps a volume percentage to 3 clean levels (`35%`, `70%`, `100%`) to minimize green-line bloat.
pub fn quantize_stamp_volume(volume: i32) -> i32 {
    const LEVELS: [i32; 3] = [35, 70, 100];
    LEVELS
        .into_iter()
        .min_by_key(|&level| (level - volume).abs())
        .unwrap_or(100)
}
