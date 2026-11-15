// Ported from osu!lazer (ppy/osu) at 131fe72a15581049401a588a13684938d4d0e0c7:
//   osu.Game/Rulesets/Objects/Legacy/ConvertHitObjectParser.cs (convertPathType, convertPathString,
//   convertPoints, mergeControlPointsLists, readPoint)
//   osu.Game/Rulesets/Objects/SliderPath.cs (calculatePath, calculateSubPath, calculateLength, Distance)
// Copyright (c) ppy Pty Ltd <contact@ppy.sh>. Licensed under the MIT Licence (ADR 0027).
//
// Only what slider times need is ported: the path's distance. Positions along the path are not.

use super::path_approximator::{self as pa, almost_equals_f32, CircularArcProperties};
use super::vector::Vector2;
use dawsu_osu_format::number::{parse_float, MAX_COORDINATE_VALUE};

/// `LegacyBeatmapEncoder.FIRST_LAZER_VERSION`.
pub const FIRST_LAZER_VERSION: u32 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplineType {
    Catmull,
    Bezier,
    BSpline(usize),
    Linear,
    PerfectCurve,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlPoint {
    pub position: Vector2,
    pub kind: Option<SplineType>,
}

fn convert_path_type(input: &str) -> SplineType {
    match input.chars().next() {
        Some('B') => {
            let rest = &input[1..];
            match rest.parse::<i32>() {
                Ok(degree) if !rest.is_empty() && degree > 0 => SplineType::BSpline(degree as usize),
                _ => SplineType::Bezier,
            }
        }
        Some('L') => SplineType::Linear,
        Some('P') => SplineType::PerfectCurve,
        _ => SplineType::Catmull,
    }
}

/// Reads a curve field (`B|400:200|400:300`) into control points relative to `offset`, the
/// object's position. Errors are the ones lazer throws, which make it skip the object.
pub fn parse_curve(point_string: &str, offset: Vector2, format_version: u32) -> Result<Vec<ControlPoint>, String> {
    let mut points: Vec<Vector2> = Vec::new();
    let mut segments: Vec<(SplineType, usize)> = Vec::new();
    for s in point_string.split('|') {
        let first = s.chars().next().ok_or("an empty curve point")?;
        if first.is_alphabetic() {
            segments.push((convert_path_type(s), points.len()));
            if points.is_empty() {
                points.push(Vector2::ZERO);
            }
        } else {
            points.push(read_point(s, offset, format_version)?);
        }
    }

    let mut control_points = Vec::new();
    for i in 0..segments.len() {
        let (kind, start) = segments[i];
        if i + 1 < segments.len() {
            let end = segments[i + 1].1;
            let end_point = *points.get(end).ok_or("a curve segment has no end point")?;
            control_points.extend(convert_points(kind, &points[start..end], Some(end_point), format_version));
        } else {
            control_points.extend(convert_points(kind, &points[start..], None, format_version));
        }
    }
    Ok(control_points)
}

fn read_point(value: &str, start: Vector2, format_version: u32) -> Result<Vector2, String> {
    let split: Vec<&str> = value.split(':').collect();
    let x = parse_float(split[0], MAX_COORDINATE_VALUE)?;
    let y = parse_float(split.get(1).ok_or("a curve point has no y")?, MAX_COORDINATE_VALUE)?;
    let pos = if format_version >= FIRST_LAZER_VERSION { Vector2::new(x, y) } else { Vector2::new(x.trunc(), y.trunc()) };
    Ok(pos - start)
}

fn is_linear(p0: Vector2, p1: Vector2, p2: Vector2) -> bool {
    almost_equals_f32(0.0, (p1.y - p0.y) * (p2.x - p0.x) - (p1.x - p0.x) * (p2.y - p0.y))
}

fn convert_points(kind: SplineType, points: &[Vector2], end_point: Option<Vector2>, format_version: u32) -> Vec<ControlPoint> {
    let mut kind = kind;
    let mut vertices: Vec<ControlPoint> = points.iter().map(|p| ControlPoint { position: *p, kind: None }).collect();
    if vertices.is_empty() {
        return Vec::new();
    }

    if kind == SplineType::PerfectCurve {
        let end_point_length = usize::from(end_point.is_some());
        if format_version < FIRST_LAZER_VERSION {
            if vertices.len() + end_point_length != 3 {
                kind = SplineType::Bezier;
            } else if is_linear(points[0], points[1], end_point.unwrap_or_else(|| points[2])) {
                kind = SplineType::Linear;
            }
        } else if vertices.len() + end_point_length > 3 {
            kind = SplineType::Bezier;
        }
    }

    vertices[0].kind = Some(kind);

    let mut out = Vec::new();
    let mut start_index = 0;
    let mut end_index = 0;
    loop {
        end_index += 1;
        if end_index >= vertices.len() {
            break;
        }
        if vertices[end_index].position != vertices[end_index - 1].position {
            continue;
        }
        if kind == SplineType::Catmull && end_index > 1 && format_version < FIRST_LAZER_VERSION {
            continue;
        }
        if end_index == vertices.len() - 1 {
            continue;
        }
        vertices[end_index - 1].kind = Some(kind);
        out.extend_from_slice(&vertices[start_index..end_index]);
        start_index = end_index + 1;
    }
    if start_index < end_index {
        out.extend_from_slice(&vertices[start_index..end_index]);
    }
    out
}

/// The result of a path calculation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathLength {
    /// `SliderPath.Distance`: the length after lengthening or shortening to the expected distance.
    pub distance: f64,
    /// `SliderPath.CalculatedDistance`: the length before that.
    pub calculated: f64,
}

/// `SliderPath` with `controlPoints` and `expectedDistance`, as far as its distance goes.
/// `optimise_catmull` is set on the osu! ruleset's own slider path, not on the decoder's.
pub fn path_length(control_points: &[ControlPoint], expected_distance: Option<f64>, optimise_catmull: bool) -> PathLength {
    let (path, optimised_length) = calculate_path(control_points, optimise_catmull);
    calculate_length(path, optimised_length, expected_distance)
}

fn calculate_path(control_points: &[ControlPoint], optimise_catmull: bool) -> (Vec<Vector2>, f64) {
    let mut calculated: Vec<Vector2> = Vec::new();
    let mut optimised_length = 0.0;
    if control_points.is_empty() {
        return (calculated, optimised_length);
    }
    let vertices: Vec<Vector2> = control_points.iter().map(|c| c.position).collect();
    let mut start = 0;
    for i in 0..control_points.len() {
        if control_points[i].kind.is_none() && i < control_points.len() - 1 {
            continue;
        }
        let segment = &vertices[start..=i];
        let kind = control_points[start].kind.unwrap_or(SplineType::Linear);
        if segment.len() == 1 {
            calculated.push(segment[0]);
        } else if segment.len() > 1 {
            let sub = calculate_sub_path(segment, kind, optimise_catmull, &mut optimised_length);
            let skip_first = !calculated.is_empty() && !sub.is_empty() && *calculated.last().unwrap() == sub[0];
            calculated.extend_from_slice(&sub[usize::from(skip_first)..]);
        }
        start = i;
    }
    (calculated, optimised_length)
}

fn calculate_sub_path(sub: &[Vector2], kind: SplineType, optimise_catmull: bool, optimised_length: &mut f64) -> Vec<Vector2> {
    match kind {
        SplineType::Linear => return pa::linear_to_piecewise_linear(sub),
        SplineType::PerfectCurve => {
            if sub.len() == 3 {
                let pr = CircularArcProperties::new(sub);
                if pr.is_valid {
                    let sub_points = if 2.0 * pr.radius <= 0.1 {
                        2
                    } else {
                        let n = (pr.theta_range / (2.0 * (1.0 - f64::from(0.1f32 / pr.radius)).acos())).ceil();
                        if n.is_nan() { 0 } else { (n.min(i32::MAX as f64) as i64).max(2) }
                    };
                    if sub_points < 1000 {
                        let path = pa::circular_arc_to_piecewise_linear(sub);
                        if !path.is_empty() {
                            return path;
                        }
                    }
                }
            }
        }
        SplineType::Catmull => {
            let path = pa::catmull_to_piecewise_linear(sub);
            if !optimise_catmull {
                return path;
            }
            const CATMULL_SEGMENT_LENGTH: usize = 50 * 2;
            let mut optimised = Vec::with_capacity(path.len());
            let mut last_start: Option<Vector2> = None;
            let mut length_removed_since_start = 0.0f64;
            for i in 0..path.len() {
                let Some(start) = last_start else {
                    optimised.push(path[i]);
                    last_start = Some(path[i]);
                    continue;
                };
                let dist_from_start = f64::from((start - path[i]).length());
                length_removed_since_start += f64::from((path[i - 1] - path[i]).length());
                if dist_from_start > 6.0 || (i + 1) % CATMULL_SEGMENT_LENGTH == 0 || i == path.len() - 1 {
                    optimised.push(path[i]);
                    *optimised_length += length_removed_since_start - dist_from_start;
                    last_start = None;
                    length_removed_since_start = 0.0;
                }
            }
            return optimised;
        }
        _ => {}
    }
    let degree = match kind {
        SplineType::BSpline(d) => d,
        _ => sub.len(),
    };
    pa::b_spline_to_piecewise_linear(sub, degree)
}

fn calculate_length(mut path: Vec<Vector2>, optimised_length: f64, expected_distance: Option<f64>) -> PathLength {
    let mut calculated_length = optimised_length;
    let mut cumulative = vec![0.0f64];
    for i in 0..path.len().saturating_sub(1) {
        let diff = path[i + 1] - path[i];
        calculated_length += f64::from(diff.length());
        cumulative.push(calculated_length);
    }
    let done = |cumulative: &Vec<f64>| PathLength { distance: cumulative.last().copied().unwrap_or(0.0), calculated: calculated_length };

    let Some(expected) = expected_distance else { return done(&cumulative) };
    if calculated_length == expected {
        return done(&cumulative);
    }
    // In osu-stable, if the last two path points of a slider are equal, extension is not performed.
    if path.len() >= 2 && path[path.len() - 1] == path[path.len() - 2] && expected > calculated_length {
        cumulative.push(calculated_length);
        return done(&cumulative);
    }
    cumulative.pop();
    let mut path_end_index = path.len() as i64 - 1;
    if calculated_length > expected {
        while cumulative.last().is_some_and(|l| *l >= expected) {
            cumulative.pop();
            path.remove(path_end_index as usize);
            path_end_index -= 1;
        }
    }
    if path_end_index <= 0 {
        cumulative.push(0.0);
        return done(&cumulative);
    }
    cumulative.push(expected);
    done(&cumulative)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn length_of(curve: &str, expected: Option<f64>, version: u32) -> PathLength {
        let points = parse_curve(curve, Vector2::new(300.0, 200.0), version).unwrap();
        path_length(&points, expected, true)
    }

    #[test]
    fn a_linear_slider_is_cut_to_its_stored_length() {
        let l = length_of("L|400:200", Some(70.0), 14);
        assert_eq!(l.calculated, 100.0);
        assert_eq!(l.distance, 70.0);
    }

    #[test]
    fn a_path_is_extended_to_its_stored_length() {
        assert_eq!(length_of("L|400:200", Some(140.0), 14).distance, 140.0);
    }

    #[test]
    fn equal_last_points_stop_the_extension() {
        // A linear path with a repeated end point keeps both points, so its last two are equal.
        let l = length_of("L|400:200|400:200", Some(140.0), 14);
        assert_eq!(l.distance, l.calculated);
        assert_eq!(l.distance, 100.0);
    }

    #[test]
    fn no_stored_length_keeps_the_calculated_one() {
        let l = length_of("L|400:200", None, 14);
        assert_eq!(l.distance, 100.0);
    }

    #[test]
    fn path_types_are_read_like_lazer() {
        assert_eq!(convert_path_type("B"), SplineType::Bezier);
        assert_eq!(convert_path_type("B3"), SplineType::BSpline(3));
        assert_eq!(convert_path_type("B0"), SplineType::Bezier);
        assert_eq!(convert_path_type("X"), SplineType::Catmull);
    }
}
