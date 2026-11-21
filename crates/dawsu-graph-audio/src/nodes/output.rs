//! Sink nodes: data-driven `output.lane` and polymorphic `output.viewer`.

use dawsu_graph::{
    EvalContext, EvalError, EventList, NodeDiagnostic, NodeInputs, NodeOutputs, NodeParams,
    NodeSpec, NodeType, NodeTypeId, ParamSpec, SocketSpec, SocketType, SocketValue,
};
use std::sync::Arc;

pub struct LaneOutputNode;

impl NodeType for LaneOutputNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("output.lane"),
            type_version: 1,
            inputs: vec![SocketSpec::exact("events", SocketType::Events, true)],
            outputs: vec![],
            params: vec![
                ParamSpec::json("slot", SocketType::Enum, serde_json::json!("")),
                ParamSpec::json("tagFilter", SocketType::Enum, serde_json::json!("")),
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
        let Some(SocketValue::Events(events)) = inputs.get("events") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input events")],
                ..Default::default()
            });
        };
        let slot = params.str_or("slot", "").trim();
        if slot.is_empty() {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("output slot name must not be empty")],
                ..Default::default()
            });
        }
        let tag_filter = params.str_or("tagFilter", "");
        let filtered = if tag_filter.is_empty() {
            events.clone()
        } else {
            Arc::new(EventList {
                events: events
                    .events
                    .iter()
                    .filter(|ev| ev.tag == tag_filter)
                    .cloned()
                    .collect(),
            })
        };
        Ok(NodeOutputs {
            slot_output: Some((slot.to_owned(), filtered)),
            ..Default::default()
        })
    }
}

pub struct ViewerOutputNode;

impl NodeType for ViewerOutputNode {
    fn spec(&self) -> NodeSpec {
        NodeSpec {
            id: NodeTypeId::from("output.viewer"),
            type_version: 1,
            inputs: vec![SocketSpec::any("value", true)],
            outputs: vec![],
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
        let Some(val) = inputs.get("value") else {
            return Ok(NodeOutputs {
                diagnostics: vec![NodeDiagnostic::error("needs input value")],
                ..Default::default()
            });
        };
        Ok(NodeOutputs {
            viewer_output: Some(val.clone()),
            ..Default::default()
        })
    }
}
