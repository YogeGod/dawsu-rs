//! What the map preview draws: each object's place on osu!'s 512 × 384 playfield after stacking,
//! its times, a slider's path, and the circle size and approach time the map sets.
//! Follows osu!lazer (ppy/osu) at 131fe72a15581049401a588a13684938d4d0e0c7; copyright (c) ppy Pty Ltd
//! <contact@ppy.sh>, MIT Licence (ADR 0027, THIRD-PARTY-NOTICES.md):
//!   osu.Game.Rulesets.Osu/Objects/OsuHitObject.cs (OBJECT_RADIUS, PREEMPT_RANGE, ApplyDefaultsToSelf, StackOffset)
//!   osu.Game/Rulesets/Objects/Legacy/LegacyRulesetExtensions.cs (CalculateScaleFromCircleSize)
//!   osu.Game/Beatmaps/IBeatmapDifficultyInfo.cs (DifficultyRange, DifficultyRangeInt)
//!   osu.Game/Beatmaps/Formats/LegacyBeatmapDecoder.cs (ApproachRate falls back to OverallDifficulty; clamps)
//!   osu.Game.Rulesets.Osu/Beatmaps/OsuBeatmapProcessor.cs (ApplyStacking, applyStacking, applyStackingOld)
//!   osu.Game/Rulesets/Objects/Types/IHasPathWithRepeats.cs (CurvePositionAt, ProgressAt, SpanAt)

use std::collections::HashMap;

use crate::lazer::slider_path::{self, SliderGeometry, FIRST_LAZER_VERSION};
use crate::lazer::vector::Vector2;
use crate::rules::{self, Answer, Kind};
use dawsu_osu_format::number::{parse_float, MAX_PARSE_VALUE};
use dawsu_osu_format::{LineId, ObjectKind, OsuFile};

/// `OsuHitObject.OBJECT_RADIUS`.
pub const OBJECT_RADIUS: f32 = 64.0;
/// `OsuBeatmapProcessor.STACK_DISTANCE`.
pub const STACK_DISTANCE: f32 = 3.0;
/// `LegacyDecoder.LATEST_VERSION`: a file with no version header counts as this.
const LATEST_VERSION: u32 = 14;

/// The playfield of one osu! difficulty.
#[derive(Debug, Clone, PartialEq)]
pub struct Playfield {
    /// `OsuHitObject.Scale` from the circle size.
    pub scale: f32,
    /// `OsuHitObject.Radius`, in playfield pixels.
    pub radius: f32,
    /// `TimePreempt`: how long before its time an object appears.
    pub preempt: f64,
    /// `TimeFadeIn`.
    pub fade_in: f64,
    /// Every object in lazer's order: sorted by start time, ties in file order.
    pub objects: Vec<PlayfieldObject>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayfieldObject {
    pub line: LineId,
    pub kind: Kind,
    pub start_time: f64,
    /// The start time for a circle.
    pub end_time: f64,
    /// `Position`, as the decoder reads it.
    pub position: Vector2,
    pub stack_height: i32,
    /// `RepeatCount + 1` for a slider, 1 otherwise.
    pub span_count: usize,
    /// A slider's path relative to its position; empty otherwise.
    pub path: Option<SliderGeometry>,
    /// A slider's tick times (the osu! ruleset's ticks, `SliderEventGenerator`); empty otherwise.
    pub ticks: Vec<f64>,
}

impl PlayfieldObject {
    /// `EndPosition`: a slider's position at its end time, else its position.
    pub fn end_position(&self) -> Vector2 {
        self.position + self.curve_position_at(1.0)
    }

    /// `StackOffset`: up and left by 6.4 × scale per stack step.
    pub fn stack_offset(&self, scale: f32) -> Vector2 {
        let step = self.stack_height as f32 * scale * -6.4;
        Vector2::new(step, step)
    }

    /// `CurvePositionAt`: the point on the path at `progress` (0 to 1) of the object's whole time,
    /// going back and forth along the path once per span.
    pub fn curve_position_at(&self, progress: f64) -> Vector2 {
        let Some(path) = &self.path else { return Vector2::ZERO };
        let spans = self.span_count as f64;
        let mut p = (progress * spans) % 1.0;
        if (progress * spans) as i64 % 2 == 1 {
            p = 1.0 - p;
        }
        path.position_at(p)
    }
}

/// The playfield for `file`, or `None` for a taiko, catch or mania difficulty.
pub fn playfield(file: &OsuFile) -> Option<Playfield> {
    let Answer::Objects(answers) = rules::answer(file, &[]) else { return None };
    let version = file.format_version().unwrap_or(LATEST_VERSION);
    let hit_objects = file.hit_objects();
    let raw: HashMap<LineId, &dawsu_osu_format::HitObject> = hit_objects.iter().map(|o| (o.line(), o)).collect();

    let float = |key: &str| file.value("Difficulty", key).and_then(|v| parse_float(&v, MAX_PARSE_VALUE).ok());
    // BeatmapDifficulty defaults to 5; without its own line the approach rate is the overall difficulty.
    let overall = float("OverallDifficulty").unwrap_or(5.0);
    let circle_size = float("CircleSize").unwrap_or(5.0).clamp(0.0, 10.0);
    let approach_rate = float("ApproachRate").unwrap_or(overall).clamp(0.0, 10.0);
    let stack_leniency = file.general("StackLeniency").and_then(|v| parse_float(&v, MAX_PARSE_VALUE).ok()).unwrap_or(0.7);

    let scale = scale_from_circle_size(circle_size);
    let preempt = f64::from(difficulty_range_int(f64::from(approach_rate), 1800.0, 1200.0, 450.0));
    let fade_in = 400.0 * (preempt / 450.0).min(1.0);

    let mut objects: Vec<PlayfieldObject> = answers
        .iter()
        .filter_map(|answer| {
            let o = raw.get(&answer.line)?;
            let (x, y) = (o.x().clamp(0.0, 512.0), o.y().clamp(0.0, 512.0));
            let position = if version >= FIRST_LAZER_VERSION { Vector2::new(x, y) } else { Vector2::new(x.trunc(), y.trunc()) };
            // Every spinner sits in the playfield's centre (ConvertHitObjectParser.cs:162).
            let position = if matches!(o.kind(), ObjectKind::Spinner { .. }) { Vector2::new(256.0, 192.0) } else { position };
            let (path, span_count) = match (o.kind(), &answer.slider) {
                (ObjectKind::Slider(fields), Some(slider)) => {
                    let points = slider_path::parse_curve(&fields.curve, position, version).ok()?;
                    let length = fields.length.map(|l| l.max(0.0)).filter(|l| *l != 0.0);
                    (Some(SliderGeometry::new(&points, length, true)), slider.repeat_count + 1)
                }
                _ => (None, 1),
            };
            Some(PlayfieldObject {
                line: answer.line,
                kind: answer.kind,
                start_time: answer.start_time,
                end_time: answer.end_time.unwrap_or(answer.start_time),
                position,
                stack_height: 0,
                span_count,
                ticks: answer.slider.as_ref().map_or_else(Vec::new, |slider| {
                    slider.parts.iter().filter(|part| part.kind == rules::PartKind::Tick).map(|part| part.time).collect()
                }),
                path,
            })
        })
        .collect();

    let threshold = (preempt as i32) as f32 * stack_leniency;
    if version >= 6 {
        apply_stacking(&mut objects, threshold);
    } else {
        apply_stacking_old(&mut objects, threshold);
    }
    Some(Playfield { scale, radius: OBJECT_RADIUS * scale, preempt, fade_in, objects })
}

/// `LegacyRulesetExtensions.CalculateScaleFromCircleSize` with the fudge the osu! ruleset applies.
fn scale_from_circle_size(circle_size: f32) -> f32 {
    const BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE: f32 = 1.00041;
    let range = (f64::from(circle_size) - 5.0) / 5.0;
    // C# works out 0.7f × range in double, then casts.
    (((1.0 - f64::from(0.7f32) * range) as f32) / 2.0) * BROKEN_GAMEFIELD_ROUNDING_ALLOWANCE
}

/// `IBeatmapDifficultyInfo.DifficultyRangeInt` (`min` at 0, `mid` at 5, `max` at 10, truncated).
fn difficulty_range_int(difficulty: f64, min: f64, mid: f64, max: f64) -> i32 {
    let range = (difficulty - 5.0) / 5.0;
    let value = if difficulty > 5.0 {
        mid + (max - mid) * range
    } else if difficulty < 5.0 {
        mid + (mid - min) * range
    } else {
        mid
    };
    value as i32
}

fn distance(a: Vector2, b: Vector2) -> f32 {
    (a - b).length()
}

/// `OsuBeatmapProcessor.applyStacking` over the whole map (`startIndex` 0, `endIndex` the last).
/// Over the whole map the first pass that extends the end index does nothing, so it is left out.
fn apply_stacking(objects: &mut [PlayfieldObject], threshold: f32) {
    if objects.is_empty() {
        return;
    }
    let start_index = 0usize;
    let extended_end_index = objects.len() - 1;
    let mut extended_start_index = start_index;
    let mut i = extended_end_index;
    while i > start_index {
        let mut n = i as i64;
        let mut object_i = i;
        if objects[object_i].stack_height != 0 || objects[object_i].kind == Kind::Spinner {
            i -= 1;
            continue;
        }
        if objects[object_i].kind == Kind::Circle {
            loop {
                n -= 1;
                if n < 0 {
                    break;
                }
                let object_n = n as usize;
                if objects[object_n].kind == Kind::Spinner {
                    continue;
                }
                // Truncation to integer matches stable.
                if ((objects[object_i].start_time as i32).wrapping_sub(objects[object_n].end_time as i32)) as f32 > threshold {
                    break;
                }
                if object_n < extended_start_index {
                    objects[object_n].stack_height = 0;
                    extended_start_index = object_n;
                }
                // Circles under the last slider of a stack move down and right (negative stacking).
                if objects[object_n].kind == Kind::Slider && distance(objects[object_n].end_position(), objects[object_i].position) < STACK_DISTANCE {
                    let offset = objects[object_i].stack_height - objects[object_n].stack_height + 1;
                    let end = objects[object_n].end_position();
                    for obj in &mut objects[object_n + 1..=i] {
                        if distance(end, obj.position) < STACK_DISTANCE {
                            obj.stack_height -= offset;
                        }
                    }
                    break;
                }
                if distance(objects[object_n].position, objects[object_i].position) < STACK_DISTANCE {
                    objects[object_n].stack_height = objects[object_i].stack_height + 1;
                    object_i = object_n;
                }
            }
        } else if objects[object_i].kind == Kind::Slider {
            // The first slider in a possible stack: from here it always stacks positive.
            loop {
                n -= 1;
                if n < start_index as i64 {
                    break;
                }
                let object_n = n as usize;
                if objects[object_n].kind == Kind::Spinner {
                    continue;
                }
                if objects[object_i].start_time - objects[object_n].start_time > f64::from(threshold) {
                    break;
                }
                if distance(objects[object_n].end_position(), objects[object_i].position) < STACK_DISTANCE {
                    objects[object_n].stack_height = objects[object_i].stack_height + 1;
                    object_i = object_n;
                }
            }
        }
        i -= 1;
    }
}

/// `OsuBeatmapProcessor.applyStackingOld`, for maps before format version 6.
fn apply_stacking_old(objects: &mut [PlayfieldObject], threshold: f32) {
    for i in 0..objects.len() {
        if objects[i].stack_height != 0 && objects[i].kind != Kind::Slider {
            continue;
        }
        let mut start_time = objects[i].end_time;
        let mut slider_stack = 0;
        // The end of the path, not the end position: `Position + Path.PositionAt(1)`.
        let position2 = match &objects[i].path {
            Some(path) if objects[i].kind == Kind::Slider => objects[i].position + path.position_at(1.0),
            _ => objects[i].position,
        };
        for j in i + 1..objects.len() {
            if objects[j].start_time - f64::from(threshold) > start_time {
                break;
            }
            if distance(objects[j].position, objects[i].position) < STACK_DISTANCE {
                objects[i].stack_height += 1;
                start_time = objects[j].start_time;
            } else if distance(objects[j].position, position2) < STACK_DISTANCE {
                // Sliders bump notes down and right, rather than up and left.
                slider_stack += 1;
                objects[j].stack_height -= slider_stack;
                start_time = objects[j].start_time;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_size_and_approach_rate_follow_lazer() {
        // CS 4: (1 - 0.7 × -0.2) / 2 × 1.00041.
        assert!((scale_from_circle_size(4.0) - 0.570_233_7).abs() < 1e-6);
        assert_eq!(difficulty_range_int(9.0, 1800.0, 1200.0, 450.0), 600);
        assert_eq!(difficulty_range_int(5.0, 1800.0, 1200.0, 450.0), 1200);
        assert_eq!(difficulty_range_int(0.0, 1800.0, 1200.0, 450.0), 1800);
        // The map's value is a float, as in lazer: 8.3f is a little over 8.3, so 1200 - 750 × 0.66
        // lands just under 705 and truncates to 704.
        assert_eq!(difficulty_range_int(f64::from(8.3f32), 1800.0, 1200.0, 450.0), 704);
    }
}
