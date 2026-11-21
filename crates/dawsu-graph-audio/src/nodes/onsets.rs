//! Onset curve extraction node (`onset.flux`) with 3.1.4 bit-exact parity.

use dawsu_graph::{
    AnalysisRange, Curve, EvalContext, EvalError, NodeDiagnostic, NodeInputs, NodeOutputs,
    NodeParams, NodeSpec, NodeType, NodeTypeId, NormaliseScope, ParamSpec, SocketId, SocketSpec,
    SocketType, SocketValue,
};
use std::sync::Arc;

const NORM_PERCENTILE: f64 = 0.9;

pub struct OnsetCurveNode;

impl NodeType for OnsetCurveNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("onset.flux"),
            type_version: 1,
            inputs: vec![SocketSpec::exact(
                "spectrogram",
                SocketType::Spectrogram,
                true,
            )],
            outputs: vec![SocketSpec::exact("curve", SocketType::Curve, false)],
            params: vec![
                ParamSpec::number("loHz", 30.0, Some(1.0), Some(24000.0)),
                ParamSpec::number("hiHz", 120.0, Some(1.0), Some(24000.0)),
                ParamSpec::number("compression", 3.0, Some(0.0), None),
                ParamSpec::integer("maxBins", 3, Some(1), Some(15)),
                ParamSpec::enumeration("normalise", "p90", &["off", "p90"]),
                ParamSpec::enumeration("normaliseOver", "range", &["range", "song"]),
            ],
            bypass_passthrough: None,
        }
    }

    fn normalisation_scope(&self, params: &NodeParams<'_>) -> Option<NormaliseScope> {
        if params.str_or("normalise", "p90") == "p90"
            && params.str_or("normaliseOver", "range") == "song"
        {
            Some(NormaliseScope::Song)
        } else {
            None
        }
    }

    fn crop_output_to_range(&self, mut outputs: NodeOutputs, range: AnalysisRange) -> NodeOutputs {
        let key = SocketId::from("curve");
        if let Some(SocketValue::Curve(curve)) = outputs.values.get(&key) {
            let sliced = Arc::new(curve.slice_to_range(range));
            outputs.values.insert(key, SocketValue::Curve(sliced));
        }
        outputs
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Spectrogram(spec)) = inputs.get("spectrogram") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input spectrogram")],
                ..Default::default()
            });
        };
        if spec.frames == 0 || spec.bins < 3 || spec.sample_rate == 0 || spec.window < 2 {
            let empty = Arc::new(Curve {
                values: Vec::new(),
                hop_ms: spec.hop_ms(),
                start_ms: spec.start_ms,
            });
            return Ok(NodeOutputs::with_value("curve", SocketValue::Curve(empty)));
        }

        let (frames, bins) = (spec.frames, spec.bins);
        let rate = f64::from(spec.sample_rate);
        let bin_for = |hz: f64| {
            ((hz * spec.window as f64 / rate).round_ties_even() as i64)
                .clamp(1, spec.window as i64 / 2) as usize
        };
        let lo = bin_for(params.f64_or("loHz", 30.0)).max(1);
        let hi = bin_for(params.f64_or("hiHz", 120.0)).min(bins - 2);
        let compression = params.f64_or("compression", 3.0).max(0.0);
        let half_bins = params.usize_or("maxBins", 3).saturating_sub(1) / 2;

        let mut out = vec![0.0; frames];
        if hi >= lo {
            let log_row = |t: usize, row: &mut Vec<f64>| {
                row.clear();
                row.extend(
                    spec.magnitudes[t * bins..(t + 1) * bins]
                        .iter()
                        .map(|&v| (1.0 + compression * f64::from(v)).ln()),
                );
            };
            let (mut previous, mut current) =
                (Vec::with_capacity(bins), Vec::with_capacity(bins));
            log_row(0, &mut previous);
            for t in 1..frames {
                if context.is_cancelled() {
                    return Err(EvalError::Cancelled);
                }
                log_row(t, &mut current);
                let mut s = 0.0;
                for k in lo..=hi {
                    let prev = if half_bins == 1 {
                        previous[k].max(previous[k - 1].max(previous[k + 1]))
                    } else if half_bins == 0 {
                        previous[k]
                    } else {
                        let start = k.saturating_sub(half_bins);
                        let end = k.saturating_add(half_bins).min(bins - 1);
                        previous[start..=end]
                            .iter()
                            .copied()
                            .fold(f64::NEG_INFINITY, f64::max)
                    };
                    s += (current[k] - prev).max(0.0);
                }
                out[t] = s / (hi - lo + 1) as f64;
                std::mem::swap(&mut previous, &mut current);
            }
        }

        let mut diagnostics = Vec::new();
        if params.str_or("normalise", "p90") == "p90" {
            out = normalise_p90(out);
            if params.str_or("normaliseOver", "range") == "song"
                && context.song_range.is_none()
                && context.song_duration_ms.is_none()
            {
                diagnostics.push(NodeDiagnostic::info(
                    "song duration unknown; normalised over range",
                ));
            }
        }

        let curve = Arc::new(Curve {
            values: out,
            hop_ms: spec.hop_ms(),
            start_ms: spec.start_ms,
        });
        Ok(NodeOutputs {
            values: [(dawsu_graph::SocketId::from("curve"), SocketValue::Curve(curve))].into(),
            diagnostics,
            ..Default::default()
        })
    }
}

/// Scales a curve so that the 90th percentile of its salient local maxima becomes 1.0 (3.1.4).
pub(crate) fn normalise_p90(o: Vec<f64>) -> Vec<f64> {
    let mut maxima: Vec<f64> = (1..o.len().saturating_sub(1))
        .filter(|&t| o[t] > 0.0 && o[t] >= o[t - 1] && o[t] > o[t + 1])
        .map(|t| o[t])
        .collect();
    if maxima.is_empty() {
        return o;
    }
    let top = maxima.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    maxima.retain(|&v| v >= top * 0.056);
    maxima.sort_by(f64::total_cmp);
    let index =
        ((maxima.len() - 1) as f64).min((maxima.len() as f64 * NORM_PERCENTILE).floor()) as usize;
    let scale = maxima[index];
    if scale <= 1e-12 {
        return o;
    }
    o.into_iter().map(|v| v / scale).collect()
}
