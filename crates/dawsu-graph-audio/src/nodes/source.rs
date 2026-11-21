//! Source nodes: `input.song`, `input.file`, `input.timing`, `input.range`.

use crate::providers::{AudioProvider, DecodedAudio, TimingProvider};
use dawsu_foundation::digest::sha256_hex;
use dawsu_graph::{
    AnalysisRange, AudioBuffer, EvalContext, EvalError, NodeDiagnostic, NodeInputs, NodeOutputs,
    NodeParams, NodeSpec, NodeType, NodeTypeId, ParamSpec, SocketSpec, SocketType, SocketValue,
};
use std::collections::BTreeMap;
use std::sync::Arc;

fn norm_bits(value: f64) -> u64 {
    if value == 0.0 {
        0.0_f64.to_bits()
    } else {
        value.to_bits()
    }
}

pub struct SongAudioNode {
    pub provider: Arc<dyn AudioProvider>,
}

impl NodeType for SongAudioNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("input.song"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("range", SocketType::Range, false)],
            outputs: vec![SocketSpec::exact("audio", SocketType::Audio, false)],
            params: vec![],
            bypass_passthrough: None,
        }
    }

    fn source_cache_key(&self, _params: &NodeParams<'_>, _context: &EvalContext<'_>) -> Option<String> {
        self.provider.song_audio().ok().map(|audio| {
            let payload = format!(
                "song:{}:{}:{}",
                audio.content_hash,
                norm_bits(audio.offset_ms),
                audio.decoder_id
            );
            sha256_hex(payload.as_bytes())
        })
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        _params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let range = match inputs.get("range") {
            Some(SocketValue::Range(r)) => *r,
            _ => context.range,
        };
        let audio = match self.provider.song_audio() {
            Ok(a) => a,
            Err(err) => {
                return Ok(NodeOutputs {
                    diagnostics: vec![NodeDiagnostic::error(format!(
                        "The song could not be decoded. ({err})"
                    ))],
                    ..Default::default()
                });
            }
        };
        Ok(slice_audio(&audio, audio.offset_ms, range, "song"))
    }
}

pub struct AudioFileNode {
    pub provider: Arc<dyn AudioProvider>,
}

impl NodeType for AudioFileNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("input.file"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("range", SocketType::Range, false)],
            outputs: vec![SocketSpec::exact("audio", SocketType::Audio, false)],
            params: vec![
                ParamSpec::json("path", SocketType::Enum, serde_json::json!("")),
                ParamSpec::json("offsetMs", SocketType::Number, serde_json::Value::Null),
                ParamSpec::boolean("expectLengthOfSong", false),
            ],
            bypass_passthrough: None,
        }
    }

    fn source_cache_key(&self, params: &NodeParams<'_>, _context: &EvalContext<'_>) -> Option<String> {
        let path = params.str_or("path", "");
        self.provider.file_audio(path).ok().map(|audio| {
            let offset_ms = params
                .raw("offsetMs")
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite())
                .unwrap_or(audio.offset_ms);
            let payload = format!(
                "file:{path}:{}:{}:{}",
                audio.content_hash,
                norm_bits(offset_ms),
                audio.decoder_id
            );
            sha256_hex(payload.as_bytes())
        })
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let range = match inputs.get("range") {
            Some(SocketValue::Range(r)) => *r,
            _ => context.range,
        };
        let path = params.str_or("path", "");
        if path.is_empty() {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs audio file path")],
                ..Default::default()
            });
        }
        let audio = match self.provider.file_audio(path) {
            Ok(a) => a,
            Err(err) => {
                return Ok(NodeOutputs {
                    diagnostics: vec![NodeDiagnostic::error(format!(
                        "The audio file could not be decoded. ({err})"
                    ))],
                    ..Default::default()
                });
            }
        };
        let offset_ms = params
            .raw("offsetMs")
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
            .unwrap_or(audio.offset_ms);
        let mut out = slice_audio(&audio, offset_ms, range, "file");
        if params.bool_or("expectLengthOfSong", false)
            && let Some(song_ms) = context.song_duration_ms
            && (audio.duration_ms() - song_ms).abs() > 1000.0
        {
            out.diagnostics.push(NodeDiagnostic::warning(format!(
                "stem length ({:.0} ms) differs from song length ({:.0} ms)",
                audio.duration_ms(),
                song_ms
            )));
        }
        Ok(out)
    }
}

pub struct MapTimingNode {
    pub provider: Arc<dyn TimingProvider>,
}

impl NodeType for MapTimingNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("input.timing"),
            type_version: 1,
            inputs: vec![],
            outputs: vec![SocketSpec::exact("timing", SocketType::Timing, false)],
            params: vec![],
            bypass_passthrough: None,
        }
    }

    fn source_cache_key(&self, _params: &NodeParams<'_>, _context: &EvalContext<'_>) -> Option<String> {
        Some(self.provider.timing_grid().content_hash())
    }

    fn evaluate(
        &self,
        _inputs: &NodeInputs,
        _params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let grid = self.provider.timing_grid();
        Ok(NodeOutputs::with_value("timing", SocketValue::Timing(grid)))
    }
}

pub struct SelectedRangeNode;

impl NodeType for SelectedRangeNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("input.range"),
            type_version: 1,
            inputs: vec![],
            outputs: vec![SocketSpec::exact("range", SocketType::Range, false)],
            params: vec![ParamSpec::enumeration("mode", "selection", &["selection", "song"])],
            bypass_passthrough: None,
        }
    }

    fn evaluate(
        &self,
        _inputs: &NodeInputs,
        params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let range = match params.str_or("mode", "selection") {
            "song" => context
                .song_range
                .or_else(|| {
                    context
                        .song_duration_ms
                        .filter(|&d| d > 0.0)
                        .and_then(|d| AnalysisRange::new(0.0, d).ok())
                })
                .unwrap_or(context.range),
            _ => context.range,
        };
        Ok(NodeOutputs::with_value("range", SocketValue::Range(range)))
    }
}

fn slice_audio(
    audio: &DecodedAudio,
    offset_ms: f64,
    range: AnalysisRange,
    kind: &str,
) -> NodeOutputs {
    let rate = audio.sample_rate;
    let total = audio.frame_count();
    if rate == 0 || total == 0 || audio.channels.is_empty() {
        return NodeOutputs {
            diagnostics: vec![NodeDiagnostic::error(format!(
                "The selected range contains no {kind} audio."
            ))],
            ..Default::default()
        };
    }
    let rel_from = range.from_ms - offset_ms;
    let rel_to = range.to_ms - offset_ms;
    let first = (rel_from * f64::from(rate) / 1000.0).round().max(0.0) as usize;
    let last = (rel_to * f64::from(rate) / 1000.0).round().max(0.0) as usize;
    let first = first.min(total);
    let last = last.min(total);
    if last <= first {
        return NodeOutputs {
            diagnostics: vec![NodeDiagnostic::error(format!(
                "The selected range contains no {kind} audio."
            ))],
            ..Default::default()
        };
    }
    let channels: Vec<Vec<f32>> = audio
        .channels
        .iter()
        .map(|ch| ch[first..last].to_vec())
        .collect();
    let start_ms = offset_ms + first as f64 * 1000.0 / f64::from(rate);
    let buf = Arc::new(AudioBuffer {
        channels,
        sample_rate: rate,
        start_ms,
        silent: false,
        full_source_peak: Some(audio.full_average_peak()),
    });
    let mut values = BTreeMap::new();
    values.insert("audio".into(), SocketValue::Audio(buf));
    NodeOutputs {
        values,
        ..Default::default()
    }
}
