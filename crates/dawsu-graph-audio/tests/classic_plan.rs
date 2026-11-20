//! 3.1.4's grid helpers (`DrumAutoFillPlan.GridShift` and `OnGrid`).

use dawsu_graph_audio::classic::{grid_shift, on_grid};

/// 120 BPM from 0 ms: a 1/4 step is 125 ms, a 1/3 step 166.7 ms.
fn grid(time: f64, divisor: u32) -> (f64, f64) {
    let step = 500.0 / f64::from(divisor);
    ((time / step).round_ties_even() * step, step)
}

#[test]
fn grid_tolerance_narrows_for_fine_divisions_and_ignores_unmapped_time() {
    assert_eq!(on_grid(1015.0, &grid, &[16], 0.0), None);
    assert_eq!(on_grid(1008.0, &grid, &[16], 0.0), Some(1000.0));
    assert_eq!(on_grid(777.0, &|t, _| (t, f64::INFINITY), &[4], 0.0), Some(777.0));
}

#[test]
fn the_nearest_line_of_any_allowed_division_wins() {
    assert_eq!(on_grid(1170.0, &grid, &[4], 0.0), None);
    let triplet = on_grid(1170.0, &grid, &[4, 3], 0.0).unwrap();
    assert!((triplet - 1166.667).abs() < 0.001);
    assert_eq!(on_grid(1135.0, &grid, &[4], 0.0), Some(1125.0));
}

#[test]
fn a_constant_lag_is_measured_from_enough_hits() {
    let late: Vec<f64> = (0..12).map(|i| 2000.0 + f64::from(i) * 125.0 + 28.0).chain([4088.0]).collect();
    assert!((grid_shift(late.iter().copied(), &grid, 4) - 28.0).abs() < 1e-9);
    assert_eq!(grid_shift(late.iter().copied().take(3), &grid, 4), 0.0);
    assert_eq!(on_grid(2028.0, &grid, &[4], 28.0), Some(2000.0));
}

#[test]
fn a_lag_beyond_50_ms_is_not_compensated() {
    let late: Vec<f64> = (0..12).map(|i| 2000.0 + f64::from(i) * 500.0 + 60.0).collect();
    assert_eq!(grid_shift(late.iter().copied(), &grid, 1), 0.0);
}
