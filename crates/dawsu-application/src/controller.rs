use dawsu_editor::tools::{PointerButton, PointerInput};
use dawsu_editor::volume_ramp::{RampShape, ramp};
use dawsu_editor_composer::Composer;
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::{Hitsounds, Moment, MomentId};
use std::collections::HashMap;
use std::convert::Infallible;

pub(super) enum ControllerStroke {
    Drag {
        start: i32,
        /// Each bar, with where it stood when the stroke began (`Moment::reference_volume`).
        originals: Vec<(VolumeBar, i32)>,
    },
    Paint {
        last: (f64, i32),
        originals: HashMap<usize, VolumeBar>,
    },
    Spread(VolumeSpread),
}

pub(super) struct VolumeBar {
    moment: MomentId,
    shared: i32,
    gains: Vec<(SampleIdentity, i32)>,
}

impl VolumeBar {
    fn new(id: MomentId, moment: &Moment) -> Self {
        Self {
            moment: id,
            shared: moment.volume(),
            gains: moment.hits().map(|lane| (lane.clone(), moment.hit_volume(lane))).collect(),
        }
    }

    /// Moves the moment to `volume` from where it stood when the stroke began, so a hit that
    /// stopped at 5 or 100 earlier in the stroke gets its difference back.
    fn move_to(&self, hits: &mut Hitsounds, volume: i32) {
        hits.set_volume(self.moment, self.shared);
        for (lane, gain) in &self.gains {
            hits.set_hit_volume(self.moment, lane, *gain);
        }
        hits.move_volume(self.moment, volume);
    }
}

pub(super) struct VolumeSpread {
    start_y: f64,
    mean: f64,
    bars: Vec<VolumeBar>,
}

impl VolumeSpread {
    fn new(composer: &Composer, targets: Vec<usize>, start_y: f64) -> Option<Self> {
        let bars: Vec<_> = targets.into_iter().filter_map(|index| {
            let moment = composer.hitsounds().moments().get(index)?;
            Some(VolumeBar {
                moment: MomentId(index),
                shared: moment.volume(),
                gains: moment.hits().map(|lane| (lane.clone(), moment.hit_volume(lane))).collect(),
            })
        }).collect();
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
            current.volume() == scale(bar.shared)
                && bar.gains.iter().all(|(lane, gain)| current.hit_volume(lane) == scale(*gain))
        })
    }

    fn apply(&self, composer: &mut Composer, y: f64) {
        let factor = (1.0 + (self.start_y - y) / 60.0).clamp(0.0, 4.0);
        let scale = |gain: i32| {
            (self.mean + (f64::from(gain) - self.mean) * factor)
                .round_ties_even().clamp(0.0, 100.0) as i32
        };
        if self.matches(composer, scale) {
            return;
        }
        let _ = composer.history.edit(true, |hits| {
            for bar in &self.bars {
                hits.set_volume(bar.moment, scale(bar.shared));
                for (lane, gain) in &bar.gains {
                    hits.set_hit_volume(bar.moment, lane, scale(*gain));
                }
            }
            Ok::<_, Infallible>(())
        });
    }
}

impl ControllerStroke {
    pub(super) fn start(composer: &mut Composer, input: PointerInput, zoom: f64) -> Option<Self> {
        if input.button != PointerButton::Left {
            return None;
        }
        let point = point(input);
        let (label, mut stroke) = if input.mods.alt() {
            ("Paint volumes", Self::Paint { last: point, originals: HashMap::new() })
        } else {
            let moments = composer.hitsounds().moments();
            let nearest = moments
                .iter()
                .enumerate()
                .map(|(id, moment)| (id, (moment.time().ms() - point.0).abs() * zoom))
                .filter(|(_, distance)| *distance <= 6.0)
                .min_by(|(_, a), (_, b)| a.total_cmp(b))?
                .0;
            let mut targets: Vec<_> = composer.selected.items().iter().map(|key| key.moment).collect();
            targets.sort_unstable();
            targets.dedup();
            if !targets.contains(&nearest) {
                targets = vec![nearest];
            }
            if input.mods.ctrl() && targets.len() > 1 {
                ("Spread volumes", Self::Spread(VolumeSpread::new(composer, targets, input.y)?))
            } else {
                let originals = targets.into_iter()
                    .filter_map(|id| moments.get(id)
                        .map(|moment| (VolumeBar::new(MomentId(id), moment), moment.reference_volume())))
                    .collect();
                ("Volume", Self::Drag { start: point.1, originals })
            }
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
        let values: Vec<(&VolumeBar, i32)> = match self {
            Self::Spread(spread) => return spread.apply(composer, input.y),
            Self::Drag { start, originals } => originals
                .iter()
                .map(|(bar, reference)| {
                    let value = if originals.len() > 1 {
                        (reference + current.1 - *start).clamp(0, 100)
                    } else {
                        current.1
                    };
                    (bar, value)
                })
                .collect(),
            Self::Paint { last, originals } => {
                let from = last.0.min(current.0);
                let to = last.0.max(current.0);
                let moments: Vec<_> = composer.hitsounds().moments().iter().enumerate()
                    .map(|(id, moment)| (moment.time().ms(), MomentId(id)))
                    .filter(|(time, _)| (from..=to).contains(time))
                    .collect();
                let values = ramp(&moments, *last, current, RampShape::Linear);
                *last = current;
                for (id, _) in &values {
                    originals.entry(id.0)
                        .or_insert_with(|| VolumeBar::new(*id, &composer.hitsounds().moments()[id.0]));
                }
                let originals = &*originals;
                values.into_iter().map(|(id, value)| (&originals[&id.0], value)).collect()
            }
        };
        if values.iter().any(|(bar, value)| composer.hitsounds().moments()[bar.moment.0].reference_volume() != *value) {
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
    (input.time.ms(), input.row_value.unwrap_or(100.0).round().clamp(1.0, 100.0) as i32)
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
        let mut stroke = ControllerStroke::start(&mut c, bar(1000.0, 90.0, Mods::NONE), 1.0).unwrap();
        assert_eq!(normal_and_clap(&c, &normal), (90, 60));
        stroke.apply(&mut c, bar(1000.0, 100.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (100, 70));
        stroke.apply(&mut c, bar(1000.0, 20.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (20, 5), "the clap stops at 5");
        stroke.finish(&mut c, bar(1000.0, 90.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (90, 60), "within one drag the clap gets its difference back");

        // After the drag ends, a lane that stopped at 5 keeps 5 as its new volume.
        let mut down = ControllerStroke::start(&mut c, bar(1000.0, 20.0, Mods::NONE), 1.0).unwrap();
        down.finish(&mut c, bar(1000.0, 20.0, Mods::NONE));
        assert_eq!(normal_and_clap(&c, &normal), (20, 5));
        let mut up = ControllerStroke::start(&mut c, bar(1000.0, 90.0, Mods::NONE), 1.0).unwrap();
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
        let mut stroke = ControllerStroke::start(&mut c, at(0.0), 1.0).unwrap();
        stroke.apply(&mut c, at(-60.0));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((83, 23), 100));
        stroke.finish(&mut c, at(30.0));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((78, 63), 88));
        c.undo();
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((80, 50), 100));
    }

    #[test]
    fn dragging_several_bars_moves_each_moment_by_its_own_change() {
        let (mut c, normal) = normal_80_clap_50();
        c.selected.replace([HitKey { moment: 0, lane: clap() }, HitKey { moment: 1, lane: normal.clone() }]);
        let second = |c: &Composer| c.hitsounds().moments()[1].volume();
        assert_eq!(second(&c), 100);
        let mut stroke = ControllerStroke::start(&mut c, bar(1000.0, 80.0, Mods::NONE), 1.0).unwrap();
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
        let mut stroke = ControllerStroke::start(&mut c, bar(1000.0, 80.0, Mods::NONE), 1.0).unwrap();
        assert_eq!(clap_volume(&c), 80, "the moment's own 80 is not where the clap plays");
        stroke.finish(&mut c, bar(1000.0, 90.0, Mods::NONE));
        assert_eq!(clap_volume(&c), 90);
        c.undo();
        assert_eq!(clap_volume(&c), 50);

        // Several bars move by the pointer's change from where each one plays.
        let normal = c.hitsounds().moments()[1].hits().next().unwrap().clone();
        c.selected.replace([HitKey { moment: 0, lane: clap() }, HitKey { moment: 1, lane: normal }]);
        let second = |c: &Composer| c.hitsounds().moments()[1].volume();
        let mut stroke = ControllerStroke::start(&mut c, bar(1000.0, 80.0, Mods::NONE), 1.0).unwrap();
        assert_eq!((clap_volume(&c), second(&c)), (50, 100), "a click without a move changes nothing");
        stroke.finish(&mut c, bar(1000.0, 70.0, Mods::NONE));
        assert_eq!((clap_volume(&c), second(&c)), (40, 90));
    }

    #[test]
    fn painting_bars_moves_every_lane_together_and_keeps_their_difference() {
        let (mut c, normal) = normal_80_clap_50();
        let mut stroke = ControllerStroke::start(&mut c, bar(900.0, 20.0, Mods::ALT), 1.0).unwrap();
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
        let mut stroke = ControllerStroke::start(&mut c, bar(900.0, 20.0, Mods::ALT), 1.0).unwrap();
        stroke.apply(&mut c, bar(1600.0, 20.0, Mods::ALT));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((20, 5), (20, 10)));
        stroke.apply(&mut c, bar(1600.0, 60.0, Mods::ALT));
        stroke.finish(&mut c, bar(900.0, 60.0, Mods::ALT));
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((60, 30), (60, 50)));
        c.undo();
        assert_eq!((normal_and_clap(&c, &normal), second(&c)), ((80, 50), (100, 90)), "one stroke is one undo step");
    }
}
