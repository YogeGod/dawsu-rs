//! The batch step (composer TDD R23): many generated hits added as one undo step. Drum hit
//! detection uses it today; Record and the node graph can reuse it.
//!
//! [`Composer::plan_batch`] and [`Composer::add_batch`] share one planner, so the counts a review
//! shows are the counts Apply gives. Grid lines arrive with each hit ([`BatchHit::grid`]): the
//! caller measures them against the map's grid, so this step does no grid maths of its own.

use crate::{Composer, HitKey};
use dawsu_editor::tempo::SnapSpacing;
use dawsu_foundation::{SampleIdentity, SongTime};
use dawsu_hitsounding::{MomentId, Place};
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
        let objects: Vec<(MomentId, f64)> = model.moments().iter().enumerate()
            .filter(|(_, m)| m.place() != Place::NoObject && inside(m.time().ms()))
            .map(|(i, m)| (MomentId(i), m.time().ms())).collect();
        let loose_existing: Vec<(MomentId, f64)> = model.moments().iter().enumerate()
            .filter(|(_, m)| m.place() == Place::NoObject)
            .map(|(i, m)| (MomentId(i), m.time().ms())).collect();
        // Every hit already there and every hit this batch places, per lane, for duplicates.
        let mut taken: Vec<(SampleIdentity, f64)> = model.moments().iter()
            .flat_map(|m| m.hits().map(move |lane| (lane.clone(), m.time().ms()))).collect();
        let mut new_moments: Vec<f64> = Vec::new();
        let mut rows: Vec<BatchRow> = hits.iter().enumerate()
            .map(|(hit, h)| BatchRow { hit, time: h.time, lands: Lands::Skipped }).collect();
        let mut targets = Vec::new();
        let mut counts = BatchCounts::default();
        let mut order: Vec<usize> = (0..hits.len()).collect();
        order.sort_by(|&a, &b| hits[b].strength.total_cmp(&hits[a].strength).then(hits[a].time.total_cmp(&hits[b].time)));
        let nearest_object = |time: f64| objects.iter()
            .map(|&(id, t)| (id, t, (t - time).abs()))
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .filter(|o| o.2 <= options.object_ms);
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
                        match hit.grid {
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
            if taken.iter().any(|(lane, t)| *lane == hit.lane && (t - time).abs() <= options.dedupe_ms) {
                counts.duplicates += 1;
                rows[index].lands = Lands::Duplicate;
                continue;
            }
            taken.push((hit.lane.clone(), time));
            let target = match object {
                Some(id) => {
                    counts.on_objects += 1;
                    rows[index].lands = Lands::Object(id);
                    Target::Existing(id)
                }
                None => {
                    counts.loose += 1;
                    rows[index].lands = Lands::Loose;
                    let near = |t: f64| (t - time).abs() <= options.dedupe_ms;
                    if let Some(&(id, t)) = loose_existing.iter().find(|(_, t)| near(*t)) {
                        rows[index].time = t;
                        Target::Existing(id)
                    } else if let Some(n) = new_moments.iter().position(|t| near(*t)) {
                        rows[index].time = new_moments[n];
                        Target::New(n)
                    } else {
                        new_moments.push(time);
                        Target::New(new_moments.len() - 1)
                    }
                }
            };
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
        let volume = self.insert_volume;
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
}
