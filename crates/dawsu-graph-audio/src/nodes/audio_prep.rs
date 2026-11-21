//! Audio preparation & filtering nodes: `audio.mono` and `audio.filter`.

use dawsu_graph::{
    AudioBuffer, EvalContext, EvalError, NodeDiagnostic, NodeInputs, NodeOutputs, NodeParams,
    NodeSpec, NodeType, NodeTypeId, ParamSpec, SocketId, SocketSpec, SocketType, SocketValue,
};
use std::f64::consts::PI;
use std::sync::Arc;

pub struct MonoMixNode;

impl NodeType for MonoMixNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("audio.mono"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("audio", SocketType::Audio, true)],
            outputs: vec![SocketSpec::exact("audio", SocketType::Audio, false)],
            params: vec![
                ParamSpec::enumeration("mix", "average", &["average", "left", "right"]),
                ParamSpec::boolean("peakNormalise", true),
                ParamSpec::enumeration("normaliseOver", "range", &["range", "song"]),
                ParamSpec::number("silenceBelow", 1e-6, Some(0.0), None),
            ],
            bypass_passthrough: Some((SocketId::from("audio"), SocketId::from("audio"))),
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Audio(audio)) = inputs.get("audio") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input audio")],
                ..Default::default()
            });
        };
        let frames = audio.frame_count();
        let mut mono = match (params.str_or("mix", "average"), audio.channels.len()) {
            (_, 0) => Vec::new(),
            ("left", _) => audio.channels[0].clone(),
            ("right", n) => audio.channels[usize::from(n > 1)].clone(),
            (_, 1) => audio.channels[0].clone(),
            (_, 2) => audio.channels[0]
                .iter()
                .zip(&audio.channels[1])
                .map(|(&l, &r)| (l + r) * 0.5)
                .collect(),
            (_, n) => (0..frames)
                .map(|i| audio.channels.iter().map(|ch| ch[i]).sum::<f32>() / n as f32)
                .collect(),
        };

        let silence_below = params.f64_or("silenceBelow", 1e-6).max(0.0) as f32;
        let peak = mono.iter().fold(0f32, |p, &v| p.max(v.abs()));
        let mut diagnostics = Vec::new();
        let silent =
            audio.silent || mono.is_empty() || !peak.is_finite() || peak <= 0.0 || peak < silence_below;
        let norm_peak = if params.str_or("normaliseOver", "range") == "song" {
            audio
                .full_source_peak
                .filter(|&p| p.is_finite() && p > 0.0)
                .unwrap_or(peak)
        } else {
            peak
        };
        if silent {
            diagnostics.push(NodeDiagnostic::info("input is silent"));
        } else if params.bool_or("peakNormalise", true) {
            for sample in &mut mono {
                *sample /= norm_peak;
            }
        }
        let out = Arc::new(AudioBuffer {
            channels: vec![mono],
            sample_rate: audio.sample_rate,
            start_ms: audio.start_ms,
            silent,
            full_source_peak: audio.full_source_peak,
        });
        Ok(NodeOutputs {
            values: [(SocketId::from("audio"), SocketValue::Audio(out))].into(),
            diagnostics,
            ..Default::default()
        })
    }
}

pub struct AudioFilterNode;

impl NodeType for AudioFilterNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("audio.filter"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("audio", SocketType::Audio, true)],
            outputs: vec![SocketSpec::exact("audio", SocketType::Audio, false)],
            params: vec![
                ParamSpec::enumeration("mode", "bandpass", &["lowpass", "highpass", "bandpass"]),
                ParamSpec::number("loHz", 80.0, Some(1.0), Some(22000.0)),
                ParamSpec::number("hiHz", 5000.0, Some(1.0), Some(22000.0)),
                ParamSpec::number("q", std::f64::consts::FRAC_1_SQRT_2, Some(0.1), Some(20.0)),
            ],
            bypass_passthrough: Some((SocketId::from("audio"), SocketId::from("audio"))),
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Audio(audio)) = inputs.get("audio") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input audio")],
                ..Default::default()
            });
        };
        if audio.silent || audio.sample_rate <= 2 || audio.frame_count() == 0 {
            return Ok(NodeOutputs::with_value("audio", SocketValue::Audio(audio.clone())));
        }
        let rate = f64::from(audio.sample_rate);
        let nyquist = (rate * 0.49).max(1.0);
        let mode = params.str_or("mode", "bandpass");
        let lo_raw = params.f64_or("loHz", 80.0);
        let hi_raw = params.f64_or("hiHz", 5000.0);
        let q_raw = params.f64_or("q", std::f64::consts::FRAC_1_SQRT_2);
        let lo_hz = if lo_raw.is_finite() { lo_raw } else { 80.0 }.clamp(1.0, nyquist);
        let hi_hz = if hi_raw.is_finite() { hi_raw } else { 5000.0 }.clamp(1.0, nyquist);
        let q = if q_raw.is_finite() { q_raw } else { std::f64::consts::FRAC_1_SQRT_2 }.clamp(0.1, 20.0);

        let stages = match mode {
            "lowpass" => vec![biquad_lowpass(hi_hz, q, rate)],
            "highpass" => vec![biquad_highpass(lo_hz, q, rate)],
            _ => {
                let (low, high) = if lo_hz <= hi_hz {
                    (lo_hz, hi_hz)
                } else {
                    (hi_hz, lo_hz)
                };
                vec![biquad_highpass(low, q, rate), biquad_lowpass(high, q, rate)]
            }
        };

        let mut filtered_channels: Vec<Vec<f32>> = Vec::with_capacity(audio.channels.len());
        for channel in &audio.channels {
            let mut work: Vec<f64> = channel.iter().map(|&s| f64::from(s)).collect();
            for &(b0, b1, b2, a1, a2) in &stages {
                let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
                for (idx, sample) in work.iter_mut().enumerate() {
                    if idx & 16_383 == 0 && context.is_cancelled() {
                        return Err(EvalError::Cancelled);
                    }
                    let x0 = *sample;
                    let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
                    x2 = x1;
                    x1 = x0;
                    y2 = y1;
                    y1 = y0;
                    *sample = y0;
                }
            }
            filtered_channels.push(work.into_iter().map(|v| v as f32).collect());
        }

        let peak = filtered_channels
            .iter()
            .flat_map(|c| c.iter())
            .fold(0f32, |p, &v| p.max(v.abs()));
        let out = Arc::new(AudioBuffer {
            channels: filtered_channels,
            sample_rate: audio.sample_rate,
            start_ms: audio.start_ms,
            silent: peak < 1e-9,
            full_source_peak: None,
        });
        Ok(NodeOutputs::with_value("audio", SocketValue::Audio(out)))
    }
}

fn biquad_lowpass(freq: f64, q: f64, rate: f64) -> (f64, f64, f64, f64, f64) {
    let w0 = 2.0 * PI * freq / rate;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    let b1 = (1.0 - cos_w0) / a0;
    let b0 = b1 * 0.5;
    let b2 = b0;
    let a1 = (-2.0 * cos_w0) / a0;
    let a2 = (1.0 - alpha) / a0;
    (b0, b1, b2, a1, a2)
}

fn biquad_highpass(freq: f64, q: f64, rate: f64) -> (f64, f64, f64, f64, f64) {
    let w0 = 2.0 * PI * freq / rate;
    let cos_w0 = w0.cos();
    let alpha = w0.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    let b0 = ((1.0 + cos_w0) * 0.5) / a0;
    let b1 = -(1.0 + cos_w0) / a0;
    let b2 = b0;
    let a1 = (-2.0 * cos_w0) / a0;
    let a2 = (1.0 - alpha) / a0;
    (b0, b1, b2, a1, a2)
}
