//! The radix-2 FFT (3.1.4's `DrumOnsets.Fft`). Bit equality with 3.1.4 is checked in
//! `dawsu-graph-audio`'s parity tests, which hold the fixture.

use dawsu_foundation::fft::{fft, twiddles};
use std::f64::consts::PI;

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn an_impulse_has_a_flat_spectrum() {
    let mut re = vec![0.0; 64];
    let mut im = vec![0.0; 64];
    re[0] = 1.0;
    fft(&mut re, &mut im);
    assert!(re.iter().all(|&v| v == 1.0));
    assert!(im.iter().all(|&v| v == 0.0));
}

#[test]
fn a_cosine_lands_in_its_bin() {
    let n = 256;
    let k = 9;
    let mut re: Vec<f64> = (0..n).map(|i| (2.0 * PI * k as f64 * i as f64 / n as f64).cos()).collect();
    let mut im = vec![0.0; n];
    fft(&mut re, &mut im);
    for bin in 0..n {
        let magnitude = re[bin].hypot(im[bin]);
        let expected = if bin == k || bin == n - k { n as f64 / 2.0 } else { 0.0 };
        assert!(close(magnitude, expected, 1e-9), "bin {bin}: {magnitude}");
    }
}

#[test]
fn matches_a_naive_dft() {
    let n = 128;
    let input: Vec<(f64, f64)> = (0..n)
        .map(|i| (((i * 7919) % 101) as f64 / 50.0 - 1.0, ((i * 104_729) % 37) as f64 / 18.0 - 1.0))
        .collect();
    let mut re: Vec<f64> = input.iter().map(|v| v.0).collect();
    let mut im: Vec<f64> = input.iter().map(|v| v.1).collect();
    fft(&mut re, &mut im);
    for k in 0..n {
        let (mut sr, mut si) = (0.0, 0.0);
        for (t, (xr, xi)) in input.iter().enumerate() {
            let angle = -2.0 * PI * (k * t) as f64 / n as f64;
            sr += xr * angle.cos() - xi * angle.sin();
            si += xr * angle.sin() + xi * angle.cos();
        }
        assert!(close(re[k], sr, 1e-9) && close(im[k], si, 1e-9), "bin {k}");
    }
}

#[test]
fn keeps_energy_parseval() {
    let n = 2048;
    let mut re: Vec<f64> = (0..n).map(|i| ((i as f64) * 0.37).sin() * ((i % 13) as f64 - 6.0)).collect();
    let mut im = vec![0.0; n];
    let time: f64 = re.iter().map(|v| v * v).sum();
    fft(&mut re, &mut im);
    let frequency: f64 = re.iter().zip(&im).map(|(r, i)| r * r + i * i).sum::<f64>() / n as f64;
    assert!(close(time, frequency, time * 1e-12));
}

#[test]
fn twiddles_follow_the_recurrence_from_one() {
    let steps: Vec<_> = twiddles(8).collect();
    assert_eq!(steps.len(), 4);
    assert_eq!(steps[0], (1.0, 0.0));
    let (wr, wi) = ((-2.0 * PI / 8.0).cos(), (-2.0 * PI / 8.0).sin());
    assert_eq!(steps[1], (wr, wi));
    assert_eq!(steps[2], (wr * wr - wi * wi, wr * wi + wi * wr));
}

#[test]
#[should_panic]
fn refuses_a_length_that_is_not_a_power_of_two() {
    fft(&mut [0.0; 6], &mut [0.0; 6]);
}
