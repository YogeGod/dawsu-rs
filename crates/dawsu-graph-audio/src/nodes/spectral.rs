//! Spectral nodes: `spectral.stft` and `spectral.hpss` with 3.1.4 bit-exact parity.

use dawsu_foundation::fft::fft;
use dawsu_graph::{
    EvalContext, EvalError, NodeDiagnostic, NodeInputs, NodeOutputs, NodeParams, NodeSpec,
    NodeType, NodeTypeId, ParamSpec, SocketId, SocketSpec, SocketType, SocketValue, Spectrogram,
};
use std::f64::consts::PI;
use std::sync::Arc;

pub struct SpectrogramNode;

impl NodeType for SpectrogramNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("spectral.stft"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("audio", SocketType::Audio, true)],
            outputs: vec![SocketSpec::exact(
                "spectrogram",
                SocketType::Spectrogram,
                false,
            )],
            params: vec![
                ParamSpec::integer("window", 2048, Some(64), Some(16384)),
                ParamSpec::enumeration("windowShape", "hann", &["hann"]),
                ParamSpec::number("hopMs", 10.0, Some(1.0), Some(100.0)),
                ParamSpec::number("maxHz", 11000.0, Some(100.0), Some(24000.0)),
                ParamSpec::integer("minHops", 4, Some(1), Some(64)),
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
        let Some(SocketValue::Audio(audio)) = inputs.get("audio") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input audio")],
                ..Default::default()
            });
        };
        let window = params.usize_or("window", 2048).next_power_of_two();
        let hop_ms = params.f64_or("hopMs", 10.0).max(0.1);
        let max_hz = params.f64_or("maxHz", 11_000.0);
        let min_hops = params.usize_or("minHops", 4);
        let rate = audio.sample_rate;
        let hop = ((f64::from(rate) * hop_ms / 1000.0).floor() as usize).max(1);

        let mono = audio.channels.first().map_or(&[][..], Vec::as_slice);
        if audio.silent || rate == 0 || mono.len() < hop * min_hops {
            let reason = if audio.silent {
                "input is silent"
            } else {
                "range too short"
            };
            let empty = Arc::new(Spectrogram {
                magnitudes: Vec::new(),
                frames: 0,
                bins: 0,
                window,
                hop_samples: hop,
                sample_rate: rate,
                start_ms: audio.start_ms,
            });
            return Ok(NodeOutputs {
                values: [(
                    SocketId::from("spectrogram"),
                    SocketValue::Spectrogram(empty),
                )]
                .into(),
                diagnostics: vec![NodeDiagnostic::info(reason)],
                ..Default::default()
            });
        }

        let frames = mono.len() / hop + 1;
        let max_bin = ((max_hz * window as f64 / f64::from(rate)).round_ties_even() as i64)
            .clamp(1, window as i64 / 2) as usize;
        let bins = (max_bin + 2).min(window / 2 + 1);

        let mut win_sum = 0.0;
        let win: Vec<f64> = (0..window)
            .map(|i| {
                let v = 0.5 - 0.5 * (2.0 * PI * i as f64 / window as f64).cos();
                win_sum += v;
                v
            })
            .collect();

        let mut magnitudes = vec![0f32; frames * bins];
        let mut re = vec![0.0; window];
        let mut im = vec![0.0; window];

        for t in 0..frames {
            if context.is_cancelled() {
                return Err(EvalError::Cancelled);
            }
            let start = (t * hop) as isize - window as isize / 2;
            for i in 0..window {
                let j = start + i as isize;
                re[i] = if j >= 0 && (j as usize) < mono.len() {
                    f64::from(mono[j as usize]) * win[i]
                } else {
                    0.0
                };
                im[i] = 0.0;
            }
            fft(&mut re, &mut im);
            let row = &mut magnitudes[t * bins..(t + 1) * bins];
            for (k, value) in row.iter_mut().enumerate() {
                *value = ((re[k] * re[k] + im[k] * im[k]).sqrt() * 2.0 / win_sum) as f32;
            }
            if t % 64 == 0 {
                context.report_progress(t as f64 / frames.max(1) as f64);
            }
        }
        context.report_progress(1.0);

        let spec = Arc::new(Spectrogram {
            magnitudes,
            frames,
            bins,
            window,
            hop_samples: hop,
            sample_rate: rate,
            start_ms: audio.start_ms,
        });
        Ok(NodeOutputs::with_value(
            "spectrogram",
            SocketValue::Spectrogram(spec),
        ))
    }
}

pub struct PercussiveNode;

impl NodeType for PercussiveNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("spectral.hpss"),
            type_version: 1,
            inputs: vec![SocketSpec::exact(
                "spectrogram",
                SocketType::Spectrogram,
                true,
            )],
            outputs: vec![SocketSpec::exact(
                "spectrogram",
                SocketType::Spectrogram,
                false,
            )],
            params: vec![
                ParamSpec::integer("medianHalf", 8, Some(1), Some(64)),
                ParamSpec::enumeration("mask", "soft-squared", &["soft-squared"]),
                ParamSpec::number("epsilon", 1e-20, Some(0.0), None),
                ParamSpec::enumeration("keep", "percussive", &["percussive", "harmonic"]),
            ],
            bypass_passthrough: Some((
                SocketId::from("spectrogram"),
                SocketId::from("spectrogram"),
            )),
        }
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
        if spec.frames == 0 || spec.bins == 0 || spec.magnitudes.is_empty() {
            return Ok(NodeOutputs::with_value(
                "spectrogram",
                SocketValue::Spectrogram(spec.clone()),
            ));
        }

        let half = params.usize_or("medianHalf", 8).max(1);
        let epsilon = params.f64_or("epsilon", 1e-20);
        let keep_harmonic = params.str_or("keep", "percussive") == "harmonic";
        let (frames, bins) = (spec.frames, spec.bins);
        let mag = &spec.magnitudes;
        let mut out = vec![0f32; mag.len()];
        let mut buf = Vec::with_capacity(2 * half + 1);
        let window_range =
            |centre: usize, len: usize| centre.saturating_sub(half)..(centre + half + 1).min(len);

        for t in 0..frames {
            if context.is_cancelled() {
                return Err(EvalError::Cancelled);
            }
            let row = &mut out[t * bins..(t + 1) * bins];
            for (k, value) in row.iter_mut().enumerate() {
                buf.clear();
                buf.extend(window_range(t, frames).map(|tt| mag[tt * bins + k]));
                let h = f64::from(median(&mut buf));
                buf.clear();
                buf.extend(window_range(k, bins).map(|kk| mag[t * bins + kk]));
                let p = f64::from(median(&mut buf));
                let (p2, h2) = (p * p, h * h);
                let mask = if p2 + h2 > epsilon {
                    if keep_harmonic {
                        h2 / (p2 + h2)
                    } else {
                        p2 / (p2 + h2)
                    }
                } else {
                    0.0
                };
                *value = (f64::from(mag[t * bins + k]) * mask) as f32;
            }
            if t % 64 == 0 {
                context.report_progress(t as f64 / frames.max(1) as f64);
            }
        }
        context.report_progress(1.0);

        let result = Arc::new(Spectrogram {
            magnitudes: out,
            frames,
            bins,
            window: spec.window,
            hop_samples: spec.hop_samples,
            sample_rate: spec.sample_rate,
            start_ms: spec.start_ms,
        });
        Ok(NodeOutputs::with_value(
            "spectrogram",
            SocketValue::Spectrogram(result),
        ))
    }
}

fn median(values: &mut [f32]) -> f32 {
    values.sort_unstable_by(f32::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or(0.0)
}
