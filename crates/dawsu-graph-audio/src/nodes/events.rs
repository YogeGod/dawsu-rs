//! Event nodes: `events.peaks`, `events.sample_time`, `events.measure`, `events.condition`,
//! `events.merge`, and `events.snap`.

use crate::classic::{GRID_TOLERANCE_MS, MAX_GRID_SHIFT_MS};
use dawsu_graph::{
    Curve, EvalContext, EvalError, EventList, GraphEvent, NodeDiagnostic, NodeInputs, NodeOutputs,
    NodeParams, NodeSpec, NodeType, NodeTypeId, ParamSpec, SocketId, SocketSpec, SocketType,
    SocketValue, TimingGrid,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct PeakPickNode;

impl NodeType for PeakPickNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("events.peaks"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("curve", SocketType::Curve, true)],
            outputs: vec![SocketSpec::exact("events", SocketType::Events, false)],
            params: vec![
                ParamSpec::number("threshold", 1.5, Some(0.05), Some(10.0)),
                ParamSpec::number("floorScale", 0.3, Some(0.0), None),
                ParamSpec::number("deltaScale", 0.12, Some(0.0), None),
                ParamSpec::integer("maxHalfFrames", 3, Some(0), Some(32)),
                ParamSpec::integer("meanBefore", 10, Some(0), Some(64)),
                ParamSpec::integer("meanAfter", 3, Some(0), Some(64)),
                ParamSpec::number("minGapMs", 60.0, Some(0.0), None),
                ParamSpec::number("biasMs", 4.0, None, None),
                ParamSpec::json("tag", SocketType::Enum, serde_json::json!("")),
            ],
            bypass_passthrough: None,
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Curve(curve)) = inputs.get("curve") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input curve")],
                ..Default::default()
            });
        };
        let scale = params.f64_or("threshold", 1.5);
        if !scale.is_finite() || scale <= 0.0 || curve.values.is_empty() || curve.hop_ms <= 0.0 {
            let empty = Arc::new(EventList::default());
            return Ok(NodeOutputs {
                values: [(SocketId::from("events"), SocketValue::Events(empty))].into(),
                diagnostics: vec![NodeDiagnostic::info("0 events")],
                ..Default::default()
            });
        }

        let o = &curve.values;
        let delta = params.f64_or("deltaScale", 0.12) * scale;
        let floor = params.f64_or("floorScale", 0.3) * scale;
        let peak_half = params.usize_or("maxHalfFrames", 3);
        let mean_before = params.usize_or("meanBefore", 10);
        let mean_after = params.usize_or("meanAfter", 3);
        let min_gap_ms = params.f64_or("minGapMs", 60.0);
        let bias_ms = params.f64_or("biasMs", 4.0);
        let tag = params.str_or("tag", "").to_owned();

        let mut picked = Vec::new();
        for t in 1..o.len().saturating_sub(1) {
            let v = o[t];
            if v < floor {
                continue;
            }
            let range = t.saturating_sub(peak_half)..=(t + peak_half).min(o.len() - 1);
            if range
                .clone()
                .any(|u| u != t && (o[u] > v || (o[u] == v && u < t)))
            {
                continue;
            }
            let around = t.saturating_sub(mean_before)..=(t + mean_after).min(o.len() - 1);
            let (mut mean, mut n) = (0.0, 0);
            for u in around {
                mean += o[u];
                n += 1;
            }
            if v < mean / f64::from(n) + delta {
                continue;
            }
            picked.push((t, v));
        }

        let gap = (min_gap_ms / curve.hop_ms).ceil().max(0.0) as usize;
        picked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut kept: Vec<(usize, f64)> = Vec::new();
        for p in picked {
            if kept.iter().all(|k| k.0.abs_diff(p.0) >= gap) {
                kept.push(p);
            }
        }
        kept.sort_by_key(|p| p.0);

        let events: Vec<GraphEvent> = kept
            .into_iter()
            .map(|(frame, strength)| {
                let detected_at_ms = curve.start_ms + frame as f64 * curve.hop_ms;
                GraphEvent {
                    time_ms: (detected_at_ms + bias_ms).max(0.0),
                    detected_at_ms,
                    frame,
                    strength,
                    tag: tag.clone(),
                    measures: BTreeMap::new(),
                }
            })
            .collect();
        let count = events.len();
        let list = Arc::new(EventList::sorted(events));
        Ok(NodeOutputs {
            values: [(SocketId::from("events"), SocketValue::Events(list))].into(),
            diagnostics: vec![NodeDiagnostic::info(format!("{count} events"))],
            ..Default::default()
        })
    }
}

pub struct TimeSamplerNode;

impl NodeType for TimeSamplerNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("events.sample_time"),
            type_version: 1,
            inputs: vec![
                SocketSpec::exact("curve", SocketType::Curve, true),
                SocketSpec::exact("timing", SocketType::Timing, false),
            ],
            outputs: vec![SocketSpec::exact("events", SocketType::Events, false)],
            params: vec![
                ParamSpec::enumeration("mode", "interval", &["interval", "beat-grid"]),
                ParamSpec::number("intervalMs", 100.0, Some(1.0), None),
                ParamSpec::integer("divisor", 4, Some(1), Some(16)),
                ParamSpec::json("tag", SocketType::Enum, serde_json::json!("sample")),
            ],
            bypass_passthrough: None,
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Curve(curve)) = inputs.get("curve") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input curve")],
                ..Default::default()
            });
        };
        let mode = params.str_or("mode", "interval");
        let tag = params.str_or("tag", "sample").to_owned();
        let mut diagnostics = Vec::new();
        let times: Vec<f64> = if mode == "beat-grid" {
            let Some(SocketValue::Timing(timing)) = inputs.get("timing") else {
                return Ok(NodeOutputs {
                    diagnostics: vec![NodeDiagnostic::error("needs input timing for beat-grid mode")],
                    ..Default::default()
                });
            };
            let divisor = params.usize_or("divisor", 4).max(1) as u32;
            let times = timing.grid_times_in_range(context.range, divisor);
            if times.is_empty() {
                diagnostics.push(NodeDiagnostic::info("no timing lines in range"));
            }
            times
        } else {
            let step = params.f64_or("intervalMs", 100.0).max(1.0);
            let mut out = Vec::new();
            let mut t = context.range.from_ms;
            while t < context.range.to_ms - 1e-9 {
                out.push(t);
                t += step;
            }
            out
        };

        let events: Vec<GraphEvent> = times
            .into_iter()
            .map(|t| {
                let (frame, strength) = sample_curve_at(curve, t);
                GraphEvent {
                    time_ms: t.max(0.0),
                    detected_at_ms: t,
                    frame,
                    strength,
                    tag: tag.clone(),
                    measures: BTreeMap::new(),
                }
            })
            .collect();

        let list = Arc::new(EventList::sorted(events));
        Ok(NodeOutputs {
            values: [(SocketId::from("events"), SocketValue::Events(list))].into(),
            diagnostics,
            ..Default::default()
        })
    }
}

fn sample_curve_at(curve: &Curve, time_ms: f64) -> (usize, f64) {
    if curve.values.is_empty() || curve.hop_ms <= 0.0 {
        return (0, 0.0);
    }
    let idx = ((time_ms - curve.start_ms) / curve.hop_ms)
        .round()
        .clamp(0.0, (curve.values.len() - 1) as f64) as usize;
    (idx, curve.values[idx])
}

pub struct MeasureAtEventsNode;

impl NodeType for MeasureAtEventsNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("events.measure"),
            type_version: 1,
            inputs: vec![
                SocketSpec::exact("events", SocketType::Events, true),
                SocketSpec::exact("curve", SocketType::Curve, true),
            ],
            outputs: vec![SocketSpec::exact("events", SocketType::Events, false)],
            params: vec![
                ParamSpec::json("name", SocketType::Enum, serde_json::json!("measure")),
                ParamSpec::integer("halfFrames", 2, Some(0), Some(32)),
                ParamSpec::enumeration("reduce", "max", &["max", "mean"]),
            ],
            bypass_passthrough: Some((SocketId::from("events"), SocketId::from("events"))),
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Events(events)) = inputs.get("events") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input events")],
                ..Default::default()
            });
        };
        let Some(SocketValue::Curve(curve)) = inputs.get("curve") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input curve")],
                ..Default::default()
            });
        };
        let name = params.str_or("name", "measure").to_owned();
        let half = params.usize_or("halfFrames", 2);
        let use_mean = params.str_or("reduce", "max") == "mean";

        let mut updated = Vec::with_capacity(events.events.len());
        for ev in &events.events {
            let mut next = ev.clone();
            let val = if curve.values.is_empty() || curve.hop_ms <= 0.0 {
                0.0
            } else {
                let frame = ((ev.detected_at_ms - curve.start_ms) / curve.hop_ms)
                    .round()
                    .clamp(0.0, (curve.values.len() - 1) as f64) as usize;
                let lo = frame.saturating_sub(half);
                let hi = (frame + half).min(curve.values.len() - 1);
                if use_mean {
                    let slice = &curve.values[lo..=hi];
                    slice.iter().sum::<f64>() / slice.len() as f64
                } else {
                    curve.values[lo..=hi]
                        .iter()
                        .copied()
                        .fold(0.0, f64::max)
                }
            };
            next.measures.insert(name.clone(), val);
            updated.push(next);
        }

        Ok(NodeOutputs::with_value(
            "events",
            SocketValue::Events(Arc::new(EventList { events: updated })),
        ))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ComparisonSpec {
    lhs: String,
    op: String,
    #[serde(default = "one")]
    factor: f64,
    rhs: serde_json::Value,
}

fn one() -> f64 {
    1.0
}

pub struct ConditionFilterNode;

impl NodeType for ConditionFilterNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("events.condition"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("events", SocketType::Events, true)],
            outputs: vec![SocketSpec::exact("events", SocketType::Events, false)],
            params: vec![
                ParamSpec::enumeration("action", "drop", &["drop", "keep"]),
                ParamSpec::json("clauses", SocketType::Enum, serde_json::json!([])),
            ],
            bypass_passthrough: Some((SocketId::from("events"), SocketId::from("events"))),
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Events(events)) = inputs.get("events") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input events")],
                ..Default::default()
            });
        };
        let keep_when_match = params.str_or("action", "drop") == "keep";
        let clauses: Vec<Vec<ComparisonSpec>> = params
            .raw("clauses")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        if clauses.is_empty() {
            return Ok(NodeOutputs::with_value(
                "events",
                SocketValue::Events(events.clone()),
            ));
        }

        let filtered: Vec<GraphEvent> = events
            .events
            .iter()
            .filter(|ev| {
                let matched = clauses.iter().any(|clause| {
                    !clause.is_empty()
                        && clause.iter().take(4).all(|cmp| eval_comparison(ev, cmp))
                });
                if keep_when_match { matched } else { !matched }
            })
            .cloned()
            .collect();

        Ok(NodeOutputs::with_value(
            "events",
            SocketValue::Events(Arc::new(EventList { events: filtered })),
        ))
    }
}

fn eval_comparison(ev: &GraphEvent, cmp: &ComparisonSpec) -> bool {
    let lhs = resolve_operand(ev, &serde_json::Value::String(cmp.lhs.clone()));
    let rhs = cmp.factor * resolve_operand(ev, &cmp.rhs);
    match cmp.op.as_str() {
        "<" => lhs < rhs,
        "<=" => lhs <= rhs,
        ">" => lhs > rhs,
        ">=" => lhs >= rhs,
        _ => false,
    }
}

fn resolve_operand(ev: &GraphEvent, operand: &serde_json::Value) -> f64 {
    if let Some(num) = operand.as_f64() {
        return num;
    }
    match operand.as_str() {
        Some("strength") => ev.strength,
        Some("const") => 1.0,
        Some(measure) => ev.measures.get(measure).copied().unwrap_or(0.0),
        None => 0.0,
    }
}

pub struct MergeEventsNode;

impl NodeType for MergeEventsNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("events.merge"),
            type_version: 1,
            inputs: vec![SocketSpec::multi("events", SocketType::Events, true)],
            outputs: vec![SocketSpec::exact("events", SocketType::Events, false)],
            params: vec![],
            bypass_passthrough: None,
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        _params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let mut combined = Vec::new();
        for val in inputs.get_all("events") {
            if let SocketValue::Events(list) = val {
                combined.extend(list.events.iter().cloned());
            }
        }
        Ok(NodeOutputs::with_value(
            "events",
            SocketValue::Events(Arc::new(EventList::sorted(combined))),
        ))
    }
}

pub struct BeatSnapNode;

impl NodeType for BeatSnapNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("events.snap"),
            type_version: 1,
            inputs: vec![
                SocketSpec::exact("events", SocketType::Events, true),
                SocketSpec::exact("timing", SocketType::Timing, true),
            ],
            outputs: vec![SocketSpec::exact("events", SocketType::Events, false)],
            params: vec![
                ParamSpec::json("divisors", SocketType::Enum, serde_json::json!([4])),
                ParamSpec::number("toleranceMs", GRID_TOLERANCE_MS, Some(0.0), None),
                ParamSpec::boolean("measureLag", true),
                ParamSpec::number("maxLagMs", MAX_GRID_SHIFT_MS, Some(0.0), None),
                ParamSpec::integer("minEventsForLag", 8, Some(1), None),
                ParamSpec::number("lagRefineWindowMs", 25.0, Some(0.0), None),
                ParamSpec::integer("lagRefinePasses", 2, Some(0), Some(10)),
                ParamSpec::boolean("dropOffGrid", true),
            ],
            bypass_passthrough: Some((SocketId::from("events"), SocketId::from("events"))),
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Events(events)) = inputs.get("events") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input events")],
                ..Default::default()
            });
        };
        let Some(SocketValue::Timing(timing)) = inputs.get("timing") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input timing")],
                ..Default::default()
            });
        };

        let divisors: Vec<u32> = params
            .raw("divisors")
            .and_then(|v| serde_json::from_value::<Vec<u32>>(v.clone()).ok())
            .filter(|d| !d.is_empty())
            .unwrap_or_else(|| vec![4]);
        let tolerance_ms = params.f64_or("toleranceMs", GRID_TOLERANCE_MS);
        let measure_lag = params.bool_or("measureLag", true);
        let max_lag_ms = params.f64_or("maxLagMs", MAX_GRID_SHIFT_MS);
        let min_events_for_lag = params.usize_or("minEventsForLag", 8);
        let refine_window = params.f64_or("lagRefineWindowMs", 25.0);
        let refine_passes = params.usize_or("lagRefinePasses", 2);
        let drop_off_grid = params.bool_or("dropOffGrid", true);

        let mut diagnostics = Vec::new();
        let mut unmapped_count = 0usize;
        let first_divisor = divisors[0];
        let mut deltas = Vec::new();

        for ev in &events.events {
            let (snapped, step) = timing.snap(ev.time_ms, first_divisor);
            if step.is_finite() && step > 0.0 {
                deltas.push(ev.time_ms - snapped);
            } else {
                unmapped_count += 1;
            }
        }
        if unmapped_count > 0 {
            diagnostics.push(NodeDiagnostic::info(format!(
                "no timing for {unmapped_count} events"
            )));
        }

        let shift = if !measure_lag {
            0.0
        } else if deltas.len() < min_events_for_lag {
            diagnostics.push(NodeDiagnostic::info(format!(
                "lag not measured (fewer than {min_events_for_lag} events with timing)"
            )));
            0.0
        } else {
            let mut candidate = median_f64(deltas.clone());
            for _ in 0..refine_passes {
                let near: Vec<f64> = deltas
                    .iter()
                    .copied()
                    .filter(|x| (x - candidate).abs() <= refine_window)
                    .collect();
                if near.len() < min_events_for_lag {
                    break;
                }
                candidate = median_f64(near);
            }
            if candidate.abs() <= max_lag_ms {
                candidate
            } else {
                diagnostics.push(NodeDiagnostic::warning(format!(
                    "measured timing lag ({candidate:.1} ms) exceeds ±{max_lag_ms:.0} ms; lag compensation set to 0"
                )));
                0.0
            }
        };

        let mut snapped_events = Vec::with_capacity(events.events.len());
        let mut dropped = 0usize;
        for ev in &events.events {
            let shifted = ev.time_ms - shift;
            if let Some(snapped_time) = snap_to_divisors(timing, shifted, &divisors, tolerance_ms) {
                let mut next = ev.clone();
                next.time_ms = snapped_time.max(0.0);
                snapped_events.push(next);
            } else if drop_off_grid {
                dropped += 1;
            } else {
                let mut next = ev.clone();
                next.time_ms = shifted.max(0.0);
                snapped_events.push(next);
            }
        }

        diagnostics.push(NodeDiagnostic::info(format!(
            "lag {shift:.2} ms, dropped {dropped} off-grid"
        )));

        Ok(NodeOutputs {
            values: [(
                SocketId::from("events"),
                SocketValue::Events(Arc::new(EventList::sorted(snapped_events))),
            )]
            .into(),
            diagnostics,
            ..Default::default()
        })
    }
}

fn median_f64(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

fn snap_to_divisors(
    timing: &TimingGrid,
    time: f64,
    divisors: &[u32],
    tolerance_ms: f64,
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for &divisor in divisors {
        let (snapped, step) = timing.snap(time, divisor);
        let tol = if step.is_finite() && step > 0.0 {
            tolerance_ms.min(step / 3.0)
        } else {
            f64::INFINITY
        };
        if (snapped - time).abs() <= tol
            && best.is_none_or(|b| (snapped - time).abs() < (b - time).abs())
        {
            best = Some(snapped);
        }
    }
    best
}
