use dawsu_editor::tools::{PointerButton, PointerInput};
use dawsu_editor::volume_ramp::{RampShape, ramp};
use dawsu_editor_composer::Composer;
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::MomentId;
use std::convert::Infallible;

pub(super) enum ControllerStroke {
    Drag {
        start: i32,
        originals: Vec<(MomentId, i32)>,
    },
    Paint {
        last: (f64, i32),
    },
    Spread(VolumeSpread),
}

struct VolumeBar {
    moment: MomentId,
    shared: i32,
    gains: Vec<(SampleIdentity, i32)>,
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
            ("Paint volumes", Self::Paint { last: point })
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
                    .filter_map(|id| moments.get(id).map(|moment| (MomentId(id), moment.volume())))
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
        let values: Vec<_> = match self {
            Self::Spread(spread) => return spread.apply(composer, input.y),
            Self::Drag { start, originals } => originals
                .iter()
                .map(|&(id, original)| {
                    let value = if originals.len() > 1 {
                        (original + current.1 - *start).clamp(0, 100)
                    } else {
                        current.1
                    };
                    (id, value)
                })
                .collect(),
            Self::Paint { last } => {
                let from = last.0.min(current.0);
                let to = last.0.max(current.0);
                let moments: Vec<_> = composer.hitsounds().moments().iter().enumerate()
                    .map(|(id, moment)| (moment.time().ms(), MomentId(id)))
                    .filter(|(time, _)| (from..=to).contains(time))
                    .collect();
                let values = ramp(&moments, *last, current, RampShape::Linear);
                *last = current;
                values
            }
        };
        if values.iter().any(|(id, value)| composer.hitsounds().moments()[id.0].volume() != *value) {
            let _ = composer.history.edit(true, |hits| {
                for (id, value) in values {
                    hits.set_volume(id, value);
                }
                Ok::<_, Infallible>(())
            });
        }
    }
}

fn point(input: PointerInput) -> (f64, i32) {
    (input.time.ms(), input.row_value.unwrap_or(100.0).round().clamp(1.0, 100.0) as i32)
}
