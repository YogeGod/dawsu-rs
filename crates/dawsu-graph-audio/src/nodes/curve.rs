//! Curve arithmetic node (`curve.math`).

use super::onsets::normalise_p90;
use dawsu_graph::{
    Curve, EvalContext, EvalError, NodeDiagnostic, NodeInputs, NodeOutputs, NodeParams, NodeSpec,
    NodeType, NodeTypeId, ParamSpec, SocketId, SocketSpec, SocketType, SocketValue,
};
use std::sync::Arc;

pub struct CurveMathNode;

impl NodeType for CurveMathNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("curve.math"),
            type_version: 1,
            inputs: vec![
                SocketSpec::exact("a", SocketType::Curve, true),
                SocketSpec::exact("b", SocketType::Curve, false),
            ],
            outputs: vec![SocketSpec::exact("curve", SocketType::Curve, false)],
            params: vec![
                ParamSpec::enumeration(
                    "op",
                    "geomean",
                    &["add", "multiply", "geomean", "scale", "max"],
                ),
                ParamSpec::number("factor", 1.0, None, None),
                ParamSpec::enumeration("normalise", "p90", &["off", "p90"]),
                ParamSpec::enumeration("normaliseOver", "range", &["range", "song"]),
            ],
            bypass_passthrough: Some((SocketId::from("a"), SocketId::from("curve"))),
        }
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        _context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError> {
        let Some(SocketValue::Curve(a)) = inputs.get("a") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input a")],
                ..Default::default()
            });
        };
        let op = params.str_or("op", "geomean");
        let factor = params.f64_or("factor", 1.0);
        let b_opt = match inputs.get("b") {
            Some(SocketValue::Curve(b)) => Some(b),
            _ => None,
        };
        if op != "scale" && b_opt.is_none() {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input b")],
                ..Default::default()
            });
        }
        if a.values.is_empty() || b_opt.is_some_and(|b| b.values.is_empty()) {
            let empty = Arc::new(Curve {
                values: Vec::new(),
                hop_ms: a.hop_ms,
                start_ms: a.start_ms,
            });
            return Ok(NodeOutputs::with_value("curve", SocketValue::Curve(empty)));
        }

        let mut values: Vec<f64> = match op {
            "scale" => a.values.iter().map(|&v| v * factor).collect(),
            "add" => a
                .values
                .iter()
                .zip(&b_opt.expect("checked").values)
                .map(|(&x, &y)| x + y)
                .collect(),
            "multiply" => a
                .values
                .iter()
                .zip(&b_opt.expect("checked").values)
                .map(|(&x, &y)| x * y)
                .collect(),
            "max" => a
                .values
                .iter()
                .zip(&b_opt.expect("checked").values)
                .map(|(&x, &y)| x.max(y))
                .collect(),
            _ => a
                .values
                .iter()
                .zip(&b_opt.expect("checked").values)
                .map(|(&x, &y)| (x * y).max(0.0).sqrt())
                .collect(),
        };

        if params.str_or("normalise", "p90") == "p90" {
            values = normalise_p90(values);
        }

        let out = Arc::new(Curve {
            values,
            hop_ms: a.hop_ms,
            start_ms: a.start_ms,
        });
        Ok(NodeOutputs::with_value("curve", SocketValue::Curve(out)))
    }
}
