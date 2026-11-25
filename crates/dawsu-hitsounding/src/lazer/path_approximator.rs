// Ported from osu!framework (ppy/osu-framework) at 06e28ecb30f9136304a4cd8889550884bf05a346, the
// framework release 2026.1019.0 that the vault's lazer pin builds on:
//   osu.Framework/Utils/PathApproximator.cs (BSplineToPiecewiseLinear, BezierToPiecewiseLinear,
//   CatmullToPiecewiseLinear, CircularArcToPiecewiseLinear, LinearToPiecewiseLinear and helpers)
//   osu.Framework/Utils/CircularArcProperties.cs
//   osu.Framework/Utils/Precision.cs (AlmostEquals)
// Copyright (c) ppy Pty Ltd <contact@ppy.sh>. Licensed under the MIT Licence (ADR 0027).
//
// Arithmetic is in single precision wherever the original uses float or osuTK's Vector2, so path
// lengths match lazer's.

use super::vector::Vector2;

pub const BEZIER_TOLERANCE: f32 = 0.25;
const CATMULL_DETAIL: usize = 50;
const CIRCULAR_ARC_TOLERANCE: f32 = 0.1;

/// `Precision.FLOAT_EPSILON` and `Precision.AlmostEquals(float, float)`.
pub const FLOAT_EPSILON: f32 = 1e-3;
pub fn almost_equals_f32(a: f32, b: f32) -> bool {
    (a - b).abs() <= FLOAT_EPSILON
}

/// `Precision.DOUBLE_EPSILON` and `Precision.AlmostEquals(double, double)`.
pub const DOUBLE_EPSILON: f64 = 1e-7;
pub fn almost_equals_f64(a: f64, b: f64) -> bool {
    (a - b).abs() <= DOUBLE_EPSILON
}

pub fn bezier_to_piecewise_linear(control_points: &[Vector2]) -> Vec<Vector2> {
    b_spline_to_piecewise_linear(control_points, (control_points.len() as i64 - 1).max(1) as usize)
}

pub fn b_spline_to_piecewise_linear(control_points: &[Vector2], degree: usize) -> Vec<Vector2> {
    assert!(degree >= 1, "a B-spline needs degree 1 or more");
    if control_points.len() < 2 {
        return control_points.first().map(|p| vec![*p]).unwrap_or_default();
    }
    let mut degree = degree.min(control_points.len() - 1);
    let mut output = Vec::new();
    let point_count = control_points.len() - 1;

    let mut to_flatten = b_spline_to_bezier_internal(control_points, &mut degree);
    let mut free_buffers: Vec<Vec<Vector2>> = Vec::new();

    let mut subdivision_buffer1 = vec![Vector2::ZERO; degree + 1];
    let mut subdivision_buffer2 = vec![Vector2::ZERO; degree * 2 + 1];

    while let Some(mut parent) = to_flatten.pop() {
        if bezier_is_flat_enough(&parent) {
            bezier_approximate(&parent, &mut output, &mut subdivision_buffer1, &mut subdivision_buffer2, degree + 1);
            free_buffers.push(parent);
            continue;
        }
        let mut right_child = free_buffers.pop().unwrap_or_else(|| vec![Vector2::ZERO; degree + 1]);
        // The left child is written into subdivision_buffer2, as in the original.
        let left_child = &mut subdivision_buffer2;
        bezier_subdivide(&parent, left_child, &mut right_child, &mut subdivision_buffer1, degree + 1);
        parent[..degree + 1].copy_from_slice(&left_child[..degree + 1]);
        to_flatten.push(right_child);
        to_flatten.push(parent);
    }

    output.push(control_points[point_count]);
    output
}

pub fn catmull_to_piecewise_linear(control_points: &[Vector2]) -> Vec<Vector2> {
    let n = control_points.len();
    let mut result = Vec::with_capacity(n.saturating_sub(1) * CATMULL_DETAIL * 2);
    for i in 0..n.saturating_sub(1) {
        let v1 = if i > 0 { control_points[i - 1] } else { control_points[i] };
        let v2 = control_points[i];
        let v3 = if i < n - 1 { control_points[i + 1] } else { v2 + v2 - v1 };
        let v4 = if i + 2 < n { control_points[i + 2] } else { v3 + v3 - v2 };
        for c in 0..CATMULL_DETAIL {
            result.push(catmull_find_point(v1, v2, v3, v4, c as f32 / CATMULL_DETAIL as f32));
            result.push(catmull_find_point(v1, v2, v3, v4, (c + 1) as f32 / CATMULL_DETAIL as f32));
        }
    }
    result
}

/// `CircularArcProperties`.
#[derive(Debug, Clone, Copy)]
pub struct CircularArcProperties {
    pub is_valid: bool,
    pub theta_start: f64,
    pub theta_range: f64,
    pub direction: f64,
    pub radius: f32,
    pub centre: Vector2,
}

impl CircularArcProperties {
    pub fn new(control_points: &[Vector2]) -> CircularArcProperties {
        let a = control_points[0];
        let b = control_points[1];
        let c = control_points[2];
        let invalid = CircularArcProperties { is_valid: false, theta_start: 0.0, theta_range: 0.0, direction: 0.0, radius: 0.0, centre: Vector2::ZERO };
        if almost_equals_f32(0.0, (b.y - a.y) * (c.x - a.x) - (b.x - a.x) * (c.y - a.y)) {
            return invalid;
        }
        let d = 2.0 * (a.x * (b - c).y + b.x * (c - a).y + c.x * (a - b).y);
        let a_sq = a.length_squared();
        let b_sq = b.length_squared();
        let c_sq = c.length_squared();
        let centre = Vector2::new(
            a_sq * (b - c).y + b_sq * (c - a).y + c_sq * (a - b).y,
            a_sq * (c - b).x + b_sq * (a - c).x + c_sq * (b - a).x,
        ) / d;
        let d_a = a - centre;
        let d_c = c - centre;
        let radius = d_a.length();
        let theta_start = f64::from(d_a.y).atan2(f64::from(d_a.x));
        let mut theta_end = f64::from(d_c.y).atan2(f64::from(d_c.x));
        while theta_end < theta_start {
            theta_end += 2.0 * std::f64::consts::PI;
        }
        let mut direction = 1.0;
        let mut theta_range = theta_end - theta_start;
        let ac = c - a;
        let ortho_a_to_c = Vector2::new(ac.y, -ac.x);
        if Vector2::dot(ortho_a_to_c, b - a) < 0.0 {
            direction = -direction;
            theta_range = 2.0 * std::f64::consts::PI - theta_range;
        }
        CircularArcProperties { is_valid: true, theta_start, theta_range, direction, radius, centre }
    }
}

pub fn circular_arc_to_piecewise_linear(control_points: &[Vector2]) -> Vec<Vector2> {
    let pr = CircularArcProperties::new(control_points);
    if !pr.is_valid {
        return bezier_to_piecewise_linear(control_points);
    }
    let amount_points = circular_arc_point_count(pr);
    let mut output = Vec::with_capacity(amount_points);
    for i in 0..amount_points {
        let fract = i as f64 / (amount_points - 1) as f64;
        let theta = pr.theta_start + pr.direction * fract * pr.theta_range;
        let o = Vector2::new(theta.cos() as f32, theta.sin() as f32) * pr.radius;
        output.push(pr.centre + o);
    }
    output
}

/// The number of points `CircularArcToPiecewiseLinear` uses for an arc.
pub fn circular_arc_point_count(pr: CircularArcProperties) -> usize {
    if 2.0 * pr.radius <= CIRCULAR_ARC_TOLERANCE {
        2
    } else {
        let step = 2.0 * (1.0 - f64::from(CIRCULAR_ARC_TOLERANCE) / f64::from(pr.radius)).acos();
        // C#'s (int) cast truncates toward zero and saturates; ceil of a finite positive value fits.
        let n = (pr.theta_range / step).ceil();
        let n = if n.is_nan() { 0.0 } else { n.min(i32::MAX as f64) };
        (n as i64).max(2) as usize
    }
}

pub fn linear_to_piecewise_linear(control_points: &[Vector2]) -> Vec<Vector2> {
    control_points.to_vec()
}

fn b_spline_to_bezier_internal(control_points: &[Vector2], degree: &mut usize) -> Vec<Vec<Vector2>> {
    let mut result: Vec<Vec<Vector2>> = Vec::new();
    *degree = (*degree).min(control_points.len() - 1);
    let degree = *degree;
    let point_count = control_points.len() - 1;
    let mut points = control_points.to_vec();

    if degree == point_count {
        result.push(points);
    } else {
        for i in 0..point_count - degree {
            let mut sub_bezier = vec![Vector2::ZERO; degree + 1];
            sub_bezier[0] = points[i];
            for j in 0..degree.saturating_sub(1) {
                sub_bezier[j + 1] = points[i + 1];
                for k in 1..degree - j {
                    let l = k.min(point_count - degree - i);
                    points[i + k] = (points[i + k] * l as f32 + points[i + k + 1]) / (l + 1) as f32;
                }
            }
            sub_bezier[degree] = points[i + 1];
            result.push(sub_bezier);
        }
        result.push(points[point_count - degree..].to_vec());
        // The original reverses its stack so that popping yields the segments in order.
        result.reverse();
    }
    result
}

fn bezier_is_flat_enough(control_points: &[Vector2]) -> bool {
    for i in 1..control_points.len().saturating_sub(1) {
        let v = control_points[i - 1] - control_points[i] * 2.0 + control_points[i + 1];
        if v.length_squared() > BEZIER_TOLERANCE * BEZIER_TOLERANCE * 4.0 {
            return false;
        }
    }
    true
}

fn bezier_subdivide(control_points: &[Vector2], l: &mut [Vector2], r: &mut [Vector2], subdivision_buffer: &mut [Vector2], count: usize) {
    let midpoints = subdivision_buffer;
    midpoints[..count].copy_from_slice(&control_points[..count]);
    for i in 0..count {
        l[i] = midpoints[0];
        r[count - i - 1] = midpoints[count - i - 1];
        for j in 0..count - i - 1 {
            midpoints[j] = (midpoints[j] + midpoints[j + 1]) / 2.0;
        }
    }
}

fn bezier_approximate(control_points: &[Vector2], output: &mut Vec<Vector2>, subdivision_buffer1: &mut [Vector2], subdivision_buffer2: &mut [Vector2], count: usize) {
    // In the original, l is subdivisionBuffer2 and r is subdivisionBuffer1, and subdivisionBuffer1 is
    // also passed as the midpoint buffer, so r and the midpoints are one array.
    let l = subdivision_buffer2;
    let r = subdivision_buffer1;
    r[..count].copy_from_slice(&control_points[..count]);
    for (i, slot) in l.iter_mut().enumerate().take(count) {
        *slot = r[0];
        // r[count - i - 1] = midpoints[count - i - 1] assigns the array to itself here.
        for j in 0..count - i - 1 {
            r[j] = (r[j] + r[j + 1]) / 2.0;
        }
    }
    l[count..count + (count - 1)].copy_from_slice(&r[1..count]);
    output.push(control_points[0]);
    for i in 1..count - 1 {
        let index = 2 * i;
        let p = (l[index - 1] + l[index] * 2.0 + l[index + 1]) * 0.25;
        output.push(p);
    }
}

fn catmull_find_point(v1: Vector2, v2: Vector2, v3: Vector2, v4: Vector2, t: f32) -> Vector2 {
    let t2 = t * t;
    let t3 = t * t2;
    Vector2::new(
        0.5 * (2.0 * v2.x + (-v1.x + v3.x) * t + (2.0 * v1.x - 5.0 * v2.x + 4.0 * v3.x - v4.x) * t2 + (-v1.x + 3.0 * v2.x - 3.0 * v3.x + v4.x) * t3),
        0.5 * (2.0 * v2.y + (-v1.y + v3.y) * t + (2.0 * v1.y - 5.0 * v2.y + 4.0 * v3.y - v4.y) * t2 + (-v1.y + 3.0 * v2.y - 3.0 * v3.y + v4.y) * t3),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_bezier_gives_its_end_points() {
        let out = bezier_to_piecewise_linear(&[Vector2::new(0.0, 0.0), Vector2::new(100.0, 0.0)]);
        assert_eq!(out.first(), Some(&Vector2::new(0.0, 0.0)));
        assert_eq!(out.last(), Some(&Vector2::new(100.0, 0.0)));
    }

    #[test]
    fn a_half_circle_arc_is_about_pi_r_long() {
        let out = circular_arc_to_piecewise_linear(&[Vector2::new(-100.0, 0.0), Vector2::new(0.0, 100.0), Vector2::new(100.0, 0.0)]);
        let len: f64 = out.windows(2).map(|w| f64::from((w[1] - w[0]).length())).sum();
        assert!((len - std::f64::consts::PI * 100.0).abs() < 0.5, "{len}");
    }

    #[test]
    fn colinear_arc_points_are_not_a_valid_arc() {
        assert!(!CircularArcProperties::new(&[Vector2::new(0.0, 0.0), Vector2::new(50.0, 0.0), Vector2::new(100.0, 0.0)]).is_valid);
    }
}
