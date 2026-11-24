//! Value ramps for the Ramp tool, ported from the old app's `OsuHitsound.Lanes/VolumeRamp.cs` at
//! `c81774e` (dawsu!'s own code). Values are osu! volumes 5-100 (saved-volume floor; Keysound: velocities).

/// The curve of a ramp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RampShape {
    #[default]
    Linear,
    EaseIn,
    EaseOut,
    SCurve,
}

/// Saved-volume floor (5%); matches `dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR`.
const SAVED_VOLUME_FLOOR: i32 = 5;

/// The value at `t` (0 to 1) of a ramp from `from` to `to`, rounded half away from zero and kept 5-100.
pub fn ramp_at(shape: RampShape, t: f64, from: i32, to: i32) -> i32 {
    let t = t.clamp(0.0, 1.0);
    let k = match shape {
        RampShape::EaseIn => t * t,
        RampShape::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        RampShape::SCurve => t * t * (3.0 - 2.0 * t),
        RampShape::Linear => t,
    };
    ((from as f64 + (to - from) as f64 * k).round() as i32).clamp(SAVED_VOLUME_FLOOR, 100)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_at_clamps_to_saved_volume_floor() {
        assert_eq!(ramp_at(RampShape::Linear, 0.0, 0, 100), 5);
        assert_eq!(ramp_at(RampShape::Linear, 0.0, 1, 100), 5);
        assert_eq!(ramp_at(RampShape::Linear, 0.0, 4, 100), 5);
        assert_eq!(ramp_at(RampShape::Linear, 1.0, 100, 0), 5);
        assert_eq!(ramp_at(RampShape::Linear, 1.0, 0, 120), 100);
    }
}
