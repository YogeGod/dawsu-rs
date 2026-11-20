//! In-place iterative radix-2 FFT, a port of 3.1.4's `DrumOnsets.Fft` (ADR 0056).
//!
//! The arithmetic order and the twiddle recurrence are 3.1.4's, so the same input gives the same
//! bits (classic drum detection parity). f64 throughout.

use std::f64::consts::PI;

/// The twiddle factors of one butterfly stage of length `len`, in the order the FFT uses them:
/// starting at 1 and multiplied by `e^(-2πi/len)` each step, as 3.1.4 does (no fresh cos/sin per step).
pub fn twiddles(len: usize) -> impl Iterator<Item = (f64, f64)> {
    recurrence(stage_step(len), len / 2)
}

fn stage_step(len: usize) -> (f64, f64) {
    let angle = -2.0 * PI / len as f64;
    (angle.cos(), angle.sin())
}

fn recurrence((wr, wi): (f64, f64), count: usize) -> impl Iterator<Item = (f64, f64)> {
    let mut current = (1.0, 0.0);
    (0..count).map(move |_| {
        let (cr, ci) = current;
        current = (cr * wr - ci * wi, cr * wi + ci * wr);
        (cr, ci)
    })
}

/// Transforms `re` + i·`im` in place (forward, unscaled).
///
/// # Panics
/// When the lengths differ or are not a power of two.
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(n == im.len() && n.is_power_of_two(), "the FFT needs two equal power-of-two lengths");
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let half = len / 2;
        let step = stage_step(len);
        for i in (0..n).step_by(len) {
            for (k, (cr, ci)) in recurrence(step, half).enumerate() {
                let (a, b) = (i + k, i + k + half);
                let xr = re[b] * cr - im[b] * ci;
                let xi = re[b] * ci + im[b] * cr;
                re[b] = re[a] - xr;
                im[b] = im[a] - xi;
                re[a] += xr;
                im[a] += xi;
            }
        }
        len <<= 1;
    }
}
