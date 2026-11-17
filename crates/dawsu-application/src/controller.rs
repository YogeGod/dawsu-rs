use dawsu_editor::tools::{PointerButton, PointerInput};
use dawsu_editor::volume_ramp::{RampShape, ramp};
use dawsu_editor_composer::Composer;
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
            let originals = targets
                .into_iter()
                .filter_map(|id| moments.get(id).map(|moment| (MomentId(id), moment.volume())))
                .collect();
            ("Volume", Self::Drag { start: point.1, originals })
        };
        composer.history.begin(label);
        stroke.apply(composer, input);
        Some(stroke)
    }

    pub(super) fn apply(&mut self, composer: &mut Composer, input: PointerInput) {
        let current = point(input);
        let values: Vec<_> = match self {
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
