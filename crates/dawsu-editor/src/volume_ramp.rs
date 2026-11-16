//! Value ramps for the Ramp tool, ported from the old app's `OsuHitsound.Lanes/VolumeRamp.cs` at
//! `c81774e` (dawsu!'s own code). Values are osu! volumes 1-100 (Keysound: velocities).

/// The curve of a ramp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RampShape {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    SCurve,
}

/// The value at `t` (0 to 1) of a ramp from `from` to `to`, rounded half away from zero and kept 1-100.
pub fn ramp_at(shape: RampShape, t: f64, from: i32, to: i32) -> i32 {
    let t = t.clamp(0.0, 1.0);
    let k = match shape {
        RampShape::EaseIn => t * t,
        RampShape::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        RampShape::SCurve => t * t * (3.0 - 2.0 * t),
        RampShape::Linear => t,
    };
    ((from as f64 + (to - from) as f64 * k).round() as i32).clamp(1, 100)
}

/// Values for items between two times: each gets the ramp value at its place between the ends.
/// Items outside the two times are left out. The ends may be given in either order.
pub fn ramp<K: Clone>(
    items: &[(f64, K)],
    start: (f64, i32),
    end: (f64, i32),
    shape: RampShape,
) -> Vec<(K, i32)> {
    let (start, end) = if end.0 < start.0 {
        (end, start)
    } else {
        (start, end)
    };
    let span = end.0 - start.0;
    items
        .iter()
        .filter(|(t, _)| *t >= start.0 && *t <= end.0)
        .map(|(t, k)| {
            (
                k.clone(),
                ramp_at(
                    shape,
                    if span <= 0.0 {
                        0.0
                    } else {
                        (t - start.0) / span
                    },
                    start.1,
                    end.1,
                ),
            )
        })
        .collect()
}
