use dawsu_editor::tools::{PointerButton, PointerInput};
use dawsu_editor::volume_ramp::{RampShape, ramp};
use dawsu_editor_composer::Composer;
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::{Hitsounds, Moment, MomentId};
use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;

pub(super) enum ControllerStroke {
    Drag {
        start: i32,
        /// Each bar, with where it stood when the stroke began (`VolumeBar::current_reference`).
        originals: Vec<(VolumeBar, i32)>,
    },
    Paint {
        last: (f64, i32),
        originals: HashMap<usize, VolumeBar>,
        scope: Option<BTreeSet<SampleIdentity>>,
        active_lane: Option<SampleIdentity>,
    },
    Spread(VolumeSpread),
}

fn moment_in_scope(
    hits: &Hitsounds,
    moment: &Moment,
    scope: Option<&BTreeSet<SampleIdentity>>,
) -> bool {
    match scope {
        None => moment.hits().next().is_some(),
        Some(lanes) => moment
            .hits()
            .any(|lane| lanes.contains(hits.lane_slot(lane).unwrap_or(lane))),
    }
}

pub(super) struct VolumeBar {
    moment: MomentId,
    shared: i32,
    gains: Vec<(SampleIdentity, i32)>,
    scoped_only: bool,
    primary_lane: Option<SampleIdentity>,
}

impl VolumeBar {
    fn new(
        id: MomentId,
        moment: &Moment,
        scope: Option<&BTreeSet<SampleIdentity>>,
        active_lane: Option<&SampleIdentity>,
        hits: &Hitsounds,
    ) -> Option<Self> {
        if let Some(lanes) = scope {
            let gains: Vec<_> = moment
                .hits()
                .filter(|lane| lanes.contains(hits.lane_slot(lane).unwrap_or(lane)))
                .map(|lane| (lane.clone(), moment.hit_volume(lane)))
                .collect();
            if gains.is_empty() {
                return None;
            }
            // Mirror canvas.js primary bar height: active lane first if present, else first scoped hit.
            let (primary_lane, shared) = active_lane
                .and_then(|active| {
                    gains
                        .iter()
                        .find(|(lane, _)| lane == active || hits.lane_slot(lane).unwrap_or(lane) == active)
                })
                .or_else(|| gains.first())
                .map(|(lane, gain)| (Some(lane.clone()), *gain))
                .unwrap_or_else(|| (None, moment.reference_volume()));
            Some(Self {
                moment: id,
                shared,
                gains,
                scoped_only: true,
                primary_lane,
            })
        } else {
            Some(Self {
                moment: id,
                shared: moment.volume(),
                gains: moment
                    .hits()
                    .map(|lane| (lane.clone(), moment.hit_volume(lane)))
                    .collect(),
                scoped_only: false,
                primary_lane: None,
            })
        }
    }

    fn current_reference(&self, moment: &Moment) -> i32 {
        if !self.scoped_only {
            return moment.reference_volume();
        }
        self.primary_lane
            .as_ref()
            .filter(|lane| moment.has_hit(lane))
            .map(|lane| moment.hit_volume(lane))
            .or_else(|| {
                self.gains
                    .iter()
                    .find(|(lane, _)| moment.has_hit(lane))
                    .map(|(lane, _)| moment.hit_volume(lane))
            })
            .unwrap_or(self.shared)
    }

    /// Moves the moment (or only its scoped lanes) to `volume` from where it stood when the stroke
    /// began, so a hit that stopped at 5 or 100 earlier in the stroke gets its difference back.
    fn move_to(&self, hits: &mut Hitsounds, volume: i32) {
        if !self.scoped_only {
            hits.set_volume(self.moment, self.shared);
            for (lane, gain) in &self.gains {
                hits.set_hit_volume(self.moment, lane, *gain);
            }
            hits.move_volume(self.moment, volume);
            return;
        }
        let floor = dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR;
        let delta = volume.clamp(floor, 100) - self.shared;
        for (lane, gain) in &self.gains {
            hits.set_hit_volume(self.moment, lane, (gain + delta).clamp(floor, 100));
        }
    }
}

pub(super) struct VolumeSpread {
    start_y: f64,
    mean: f64,
    bars: Vec<VolumeBar>,
}

impl VolumeSpread {
    fn new(
        composer: &Composer,
        targets: Vec<usize>,
        start_y: f64,
        scope: Option<&BTreeSet<SampleIdentity>>,
        active_lane: Option<&SampleIdentity>,
    ) -> Option<Self> {
        let hits = composer.hitsounds();
        let bars: Vec<_> = targets
            .into_iter()
            .filter_map(|index| {
                let moment = hits.moments().get(index)?;
                VolumeBar::new(MomentId(index), moment, scope, active_lane, hits)
            })
            .collect();
        let gains = bars.iter().flat_map(|bar| bar.gains.iter().map(|(_, gain)| *gain));
        let count = gains.clone().count();
        if count == 0 {
            return None;
        }
        let mean = gains.map(f64::from).sum::<f64>() / count as f64;
        Some(Self { start_y, mean, bars })
    }

    fn matches(&self, composer: &Composer, scale: impl Fn(i32) -> i32) -> bool {
        self.bars.iter().all(|bar| {
            let current = &composer.hitsounds().moments()[bar.moment.0];
            (bar.scoped_only || current.volume() == scale(bar.shared))
                && bar.gains.iter().all(|(lane, gain)| current.hit_volume(lane) == scale(*gain))
        })
    }

    fn apply(&self, composer: &mut Composer, y: f64) {
        let factor = (1.0 + (self.start_y - y) / 60.0).clamp(0.0, 4.0);
        // A spread never takes a sound under 5% (R15); one that was already quieter is not pushed lower.
        let scale = |gain: i32| {
            let floor = f64::from(gain.min(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR));
            (self.mean + (f64::from(gain) - self.mean) * factor)
                .round_ties_even().clamp(floor, 100.0) as i32
        };
        if self.matches(composer, scale) {
            return;
        }
        let _ = composer.history.edit(true, |hits| {
            for bar in &self.bars {
                if !bar.scoped_only {
                    hits.set_volume(bar.moment, scale(bar.shared));
                }
                for (lane, gain) in &bar.gains {
                    hits.set_hit_volume(bar.moment, lane, scale(*gain));
                }
            }
            Ok::<_, Infallible>(())
        });
    }
}

impl ControllerStroke {
    pub(super) fn start(
        composer: &mut Composer,
        input: PointerInput,
        zoom: f64,
        scope: Option<&BTreeSet<SampleIdentity>>,
        active_lane: Option<&SampleIdentity>,
    ) -> Option<Self> {
        if input.button != PointerButton::Left {
            return None;
        }
        let point = point(input);
        let hits = composer.hitsounds();
        let moments = hits.moments();
        let (label, mut stroke) = if input.mods.alt() {
            (
                "Paint volumes",
                Self::Paint {
                    last: point,
                    originals: HashMap::new(),
                    scope: scope.cloned(),
                    active_lane: active_lane.cloned(),
                },
            )
        } else if let Some(nearest) = moments
            .iter()
            .enumerate()
            .filter(|(_, moment)| scope.is_none() || moment_in_scope(hits, moment, scope))
            .map(|(id, moment)| (id, (moment.time().ms() - point.0).abs() * zoom))
            .filter(|(_, distance)| *distance <= 6.0)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(id, _)| id)
        {
            let mut targets: Vec<_> = composer.selected.items().iter().map(|key| key.moment).collect();
            targets.sort_unstable();
            targets.dedup();
            if !targets.contains(&nearest) {
                targets = vec![nearest];
            }
            if input.mods.ctrl() && targets.len() > 1 {
                (
                    "Spread volumes",
                    Self::Spread(VolumeSpread::new(composer, targets, input.y, scope, active_lane)?),
                )
            } else {
                let originals: Vec<_> = targets
                    .into_iter()
                    .filter_map(|id| {
                        let moment = moments.get(id)?;
                        let bar = VolumeBar::new(MomentId(id), moment, scope, active_lane, hits)?;
                        let reference = bar.current_reference(moment);
                        Some((bar, reference))
                    })
                    .collect();
                if originals.is_empty() {
                    return None;
                }
                ("Volume", Self::Drag { start: point.1, originals })
            }
        } else if !input.mods.ctrl() {
            // Direct volume segment-grab: clicking on or near the segment connecting two adjacent
            // visible bars grabs both endpoints together preserving their slope.
            const SEGMENT_HIT_TOLERANCE_VOL: f64 = 12.0;
            let visible: Vec<_> = moments
                .iter()
                .enumerate()
                .filter(|(_, moment)| moment_in_scope(hits, moment, scope))
                .collect();
            let pos = visible.partition_point(|(_, moment)| moment.time().ms() < point.0);
            let left = pos.checked_sub(1).and_then(|idx| visible.get(idx)).copied();
            let right = visible.get(pos).copied();
            let ((left_id, left_m), (right_id, right_m)) = (left?, right?);
            let left_t = left_m.time().ms();
            let right_t = right_m.time().ms();
            let span = right_t - left_t;
            if !span.is_finite() || span <= 0.0 {
                return None;
            }
            let left_bar = VolumeBar::new(MomentId(left_id), left_m, scope, active_lane, hits)?;
            let right_bar = VolumeBar::new(MomentId(right_id), right_m, scope, active_lane, hits)?;
            let left_v = left_bar.current_reference(left_m);
            let right_v = right_bar.current_reference(right_m);
            let frac = (point.0 - left_t) / span;
            let line_v = f64::from(left_v) + f64::from(right_v - left_v) * frac;
            if (f64::from(point.1) - line_v).abs() > SEGMENT_HIT_TOLERANCE_VOL {
                return None;
            }
            (
                "Volume",
                Self::Drag {
                    start: point.1,
                    originals: vec![(left_bar, left_v), (right_bar, right_v)],
                },
            )
        } else {
            return None;
        };
        composer.history.begin(label);
        stroke.apply(composer, input);
        Some(stroke)
    }

    pub(super) fn finish(&mut self, composer: &mut Composer, input: PointerInput) {
        self.apply(composer, input);
        if matches!(self, Self::Spread(spread) if spread.matches(composer, |gain| gain)) {
            composer.history.cancel();
        } else {
            composer.history.commit();
        }
    }

    pub(super) fn apply(&mut self, composer: &mut Composer, input: PointerInput) {
        let current = point(input);
        let floor = dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR;
        let values: Vec<(&VolumeBar, i32)> = match self {
            Self::Spread(spread) => return spread.apply(composer, input.y),
            Self::Drag { start, originals } => originals
                .iter()
                .map(|(bar, reference)| {
                    let value = if originals.len() > 1 {
                        (reference + current.1 - *start).clamp(floor, 100)
                    } else {
                        current.1
                    };
                    (bar, value)
                })
                .collect(),
            Self::Paint { last, originals, scope, active_lane } => {
                let from = last.0.min(current.0);
                let to = last.0.max(current.0);
                let hits = composer.hitsounds();
                let moments: Vec<_> = hits
                    .moments()
                    .iter()
                    .enumerate()
                    .filter(|(_, moment)| scope.is_none() || moment_in_scope(hits, moment, scope.as_ref()))
                    .map(|(id, moment)| (moment.time().ms(), MomentId(id)))
                    .filter(|(time, _)| (from..=to).contains(time))
                    .collect();
                let values = ramp(&moments, *last, current, RampShape::Linear);
                *last = current;
                for (id, _) in &values {
                    if !originals.contains_key(&id.0)
                        && let Some(bar) = VolumeBar::new(
                            *id,
                            &hits.moments()[id.0],
                            scope.as_ref(),
                            active_lane.as_ref(),
                            hits,
                        )
                    {
                        originals.insert(id.0, bar);
                    }
                }
                let originals = &*originals;
                values
                    .into_iter()
                    .filter_map(|(id, value)| originals.get(&id.0).map(|bar| (bar, value)))
                    .collect()
            }
        };
        let moments = composer.hitsounds().moments();
        if values
            .iter()
            .any(|(bar, value)| bar.current_reference(&moments[bar.moment.0]) != *value)
        {
            let _ = composer.history.edit(true, |hits| {
                for (bar, value) in values {
                    bar.move_to(hits, value);
                }
                Ok::<_, Infallible>(())
            });
        }
    }
}

fn point(input: PointerInput) -> (f64, i32) {
    let floor = f64::from(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR);
    (input.time.ms(), input.row_value.unwrap_or(100.0).round().clamp(floor, 100.0) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_editor::keys::Mods;
    use dawsu_editor::tools::PointerSurface;
    use dawsu_editor_composer::HitKey;
    use dawsu_foundation::{CustomIndex, Hitsound, SampleSet, Sound};
    use dawsu_hitsounding::{Decompiled, decompile};
    use dawsu_osu_format::OsuFile;
    use std::collections::BTreeSet;

    fn clap() -> SampleIdentity {
        SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Clap), index: CustomIndex::new(0) }
    }

    /// Two circles. At 1000: hitnormal 80 at the moment's own volume, clap 50 with its own volume.
    fn normal_80_clap_50() -> (Composer, SampleIdentity) {
        let map = OsuFile::read(b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,0,0:0:0:0:\n".to_vec()).file;
        let Decompiled::Done(hits) = decompile(&map, &[], &|| false, &mut |_, _| {}) else {
            panic!("fixture");
        };
        let mut c = Composer::new(hits);
        let normal = c.hitsounds().moments()[0].hits().next().unwrap().clone();
        c.add(1000.0, clap(), 80, 5.0);
        c.volume(80, false);
        assert!(c.volume_for_lanes(50, false, Some(&BTreeSet::from([clap()]))));
        (c, normal)
    }

    fn bar(time: f64, value: f64, mods: Mods) -> PointerInput {
        PointerInput { row_value: Some(value), surface: PointerSurface::ValueLane, mods, ..PointerInput::at(time, 0) }
    }

    fn normal_and_clap(c: &Composer, normal: &SampleIdentity) -> (i32, i32) {
        let moment = &c.hitsounds().moments()[0];
        (moment.hit_volume(normal), moment.hit_volume(&clap()))
    }

    #[test]
    fn dragging_a_bar_moves_every_lane_together_and_a_lane_stopped_at_5_stays_there_after_release() {
        let (mut c, normal) = normal_80_clap_50();
        let mut stroke = ControllerStroke::start(&mut c, bar(1000.0, 90.0, Mods::NONE), 1.0, None, None).unwrap();
        assert_eq!(normal_and_clap(&c, &normal), (90, 60));
        stroke.apply(&mut c, bar(1000.0, 100.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (100, 70));
        stroke.apply(&mut c, bar(1000.0, 20.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (20, 5), "the clap stops at 5");
        stroke.finish(&mut c, bar(1000.0, 90.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (90, 60), "within one drag the clap gets its difference back");

        // After the drag ends, a lane that stopped at 5 keeps 5 as its new volume.
        let mut down = ControllerStroke::start(&mut c, bar(1000.0, 20.0, Mods::NONE), 1.0, None, None).unwrap();
        down.finish(&mut c, bar(1000.0, 20.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (20, 5));
        let mut up = ControllerStroke::start(&mut c, bar(1000.0, 90.0, Mods::NONE), 1.0, None, None).unwrap();
        up.finish(&mut c, bar(1000.0, 90.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (90, 75));
        c.undo();
        c.undo();
        assert_eq!(normal_and_clap(&c, &normal), (90, 60));
    }

    #[test]
    fn ctrl_dragging_several_bars_spreads_every_sound_around_their_mean() {
        let (mut c, normal) = normal_80_clap_50();
        c.selected.replace([HitKey { moment: 0, lane: clap() }, HitKey { moment: 1, lane: normal.clone() }]);
        let second = |c: &Composer| c.hitsounds().moments()[1].hit_volume(&normal);
        let at = |y: f64| PointerInput { y, ..bar(1000.0, 80.0, Mods::CTRL) };
        // The mean of 80, 50 and 100 is 76.7; moving up 60 px doubles each sound's distance from it.
        let mut stroke = ControllerStroke::start(&mut c, at(0.0), 1.0, None, None).unwrap();
        stroke.apply(&mut c, at(-60.0));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((83, 23), 100));
        stroke.finish(&mut c, at(30.0));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((78, 63), 88));
        c.undo();
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((80, 50), 100));
    }

    #[test]
    fn ctrl_dragging_a_spread_never_takes_a_sound_under_5_percent() {
        let (mut c, normal) = normal_80_clap_50();
        c.selected.replace([HitKey { moment: 0, lane: clap() }, HitKey { moment: 1, lane: normal.clone() }]);
        let second = |c: &Composer| c.hitsounds().moments()[1].hit_volume(&normal);
        let at = |y: f64| PointerInput { y, ..bar(1000.0, 80.0, Mods::CTRL) };
        // The mean of 80, 50 and 100 is 76.7; moving up 180 px puts each sound 4 times as far from it,
        // which would take the clap to -30.
        let mut stroke = ControllerStroke::start(&mut c, at(0.0), 1.0, None, None).unwrap();
        stroke.finish(&mut c, at(-180.0));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((90, 5), 100));
    }

    #[test]
    fn dragging_several_bars_moves_each_moment_by_its_own_change() {
        let (mut c, normal) = normal_80_clap_50();
        c.selected.replace([HitKey { moment: 0, lane: clap() }, HitKey { moment: 1, lane: normal.clone() }]);
        let second = |c: &Composer| c.hitsounds().moments()[1].volume();
        assert_eq!(second(&c), 100);
        let mut stroke = ControllerStroke::start(&mut c, bar(1000.0, 80.0, Mods::NONE), 1.0, None, None).unwrap();
        stroke.apply(&mut c, bar(1000.0, 90.0, Mods::NONE));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((90, 60), 100));
        stroke.finish(&mut c, bar(1000.0, 20.0, Mods::NONE));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((20, 5), 40));
    }

    /// `normal_80_clap_50` after its hitnormal is deleted: the moment keeps 80, the clap plays at 50.
    fn clap_50_at_a_moment_of_80() -> Composer {
        let (mut c, normal) = normal_80_clap_50();
        c.selected.replace([HitKey { moment: 0, lane: normal }]);
        c.remove_selected();
        assert_eq!((c.hitsounds().moments()[0].volume(), c.hitsounds().moments()[0].hit_volume(&clap())), (80, 50));
        c
    }

    #[test]
    fn dragging_a_bar_no_hit_plays_at_moves_from_the_hit_that_plays() {
        let mut c = clap_50_at_a_moment_of_80();
        let clap_volume = |c: &Composer| c.hitsounds().moments()[0].hit_volume(&clap());
        let mut stroke = ControllerStroke::start(&mut c, bar(1000.0, 80.0, Mods::NONE), 1.0, None, None).unwrap();
        assert_eq!(clap_volume(&c), 80, "the moment's own 80 is not where the clap plays");
        stroke.finish(&mut c, bar(1000.0, 90.0, Mods::NONE));
        assert_eq!(clap_volume(&c), 90);
        c.undo();
        assert_eq!(clap_volume(&c), 50);

        // Several bars move by the pointer's change from where each one plays.
        let normal = c.hitsounds().moments()[1].hits().next().unwrap().clone();
        c.selected.replace([HitKey { moment: 0, lane: clap() }, HitKey { moment: 1, lane: normal }]);
        let second = |c: &Composer| c.hitsounds().moments()[1].volume();
        let mut stroke = ControllerStroke::start(&mut c, bar(1000.0, 80.0, Mods::NONE), 1.0, None, None).unwrap();
        assert_eq!((clap_volume(&c), second(&c)), (50, 100), "a click without a move changes nothing");
        stroke.finish(&mut c, bar(1000.0, 70.0, Mods::NONE));
        assert_eq!((clap_volume(&c), second(&c)), (40, 90));
    }

    #[test]
    fn painting_bars_moves_every_lane_together_and_keeps_their_difference() {
        let (mut c, normal) = normal_80_clap_50();
        let mut stroke = ControllerStroke::start(&mut c, bar(900.0, 20.0, Mods::ALT), 1.0, None, None).unwrap();
        stroke.apply(&mut c, bar(1100.0, 20.0, Mods::ALT));
        assert_eq!(normal_and_clap(&c, &normal), (20, 5), "the clap stops at 5");
        stroke.apply(&mut c, bar(1100.0, 90.0, Mods::ALT));
        stroke.finish(&mut c, bar(900.0, 90.0, Mods::ALT));
        assert_eq!(normal_and_clap(&c, &normal), (90, 60), "painting back over it in the same stroke");
        assert_eq!(c.hitsounds().moments()[1].volume(), 100, "the stroke never reached the second circle");
    }

    #[test]
    fn painting_two_moments_in_one_stroke_moves_each_from_its_own_start() {
        let (mut c, normal) = normal_80_clap_50();
        c.add(1500.0, clap(), 80, 5.0);
        assert!(c.volume_for_lanes(90, false, Some(&BTreeSet::from([clap()]))));
        let second = |c: &Composer| {
            let moment = &c.hitsounds().moments()[1];
            (moment.hit_volume(&normal), moment.hit_volume(&clap()))
        };
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((80, 50), (100, 90)));
        let mut stroke = ControllerStroke::start(&mut c, bar(900.0, 20.0, Mods::ALT), 1.0, None, None).unwrap();
        stroke.apply(&mut c, bar(1600.0, 20.0, Mods::ALT));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((20, 5), (20, 10)));
        stroke.apply(&mut c, bar(1600.0, 60.0, Mods::ALT));
        stroke.finish(&mut c, bar(900.0, 60.0, Mods::ALT));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((60, 30), (60, 50)));
        c.undo();
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((80, 50), (100, 90)), "one stroke is one undo step");
    }

    #[test]
    fn scoped_alt_paint_only_changes_lanes_in_scope() {
        let (mut c, normal) = normal_80_clap_50();
        let scope = BTreeSet::from([normal.clone()]);
        let mut stroke =
            ControllerStroke::start(&mut c, bar(900.0, 25.0, Mods::ALT), 1.0, Some(&scope), Some(&normal)).unwrap();
        stroke.finish(&mut c, bar(1100.0, 25.0, Mods::ALT));
        assert_eq!(normal_and_clap(&c, &normal), (25, 50), "unscoped clap stays at 50");
        c.undo();
        assert_eq!(normal_and_clap(&c, &normal), (80, 50));
    }

    #[test]
    fn direct_segment_grab_between_bars_drags_adjacent_endpoints_preserving_slope() {
        let (mut c, normal) = normal_80_clap_50();
        let scope = BTreeSet::from([normal.clone()]);
        // Moment 0 is at 1000 ms (80%), moment 1 is at 1500 ms (100%). Midpoint 1250 ms expects ~90%.
        // Clicking far outside the +/-12 tolerance does not grab the segment.
        assert!(ControllerStroke::start(&mut c, bar(1250.0, 40.0, Mods::NONE), 1.0, Some(&scope), Some(&normal)).is_none());
        let mut stroke =
            ControllerStroke::start(&mut c, bar(1250.0, 88.0, Mods::NONE), 1.0, Some(&scope), Some(&normal)).unwrap();
        assert_eq!((c.hitsounds().moments()[0].hit_volume(&normal), c.hitsounds().moments()[1].hit_volume(&normal)), (80, 100));
        stroke.finish(&mut c, bar(1250.0, 68.0, Mods::NONE));
        assert_eq!((c.hitsounds().moments()[0].hit_volume(&normal), c.hitsounds().moments()[1].hit_volume(&normal)), (60, 80));
        assert_eq!(c.hitsounds().moments()[0].hit_volume(&clap()), 50, "unscoped clap stays at 50");
        c.undo();
        assert_eq!((c.hitsounds().moments()[0].hit_volume(&normal), c.hitsounds().moments()[1].hit_volume(&normal)), (80, 100));
    }
}
