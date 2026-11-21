//! Node type trait, registry, typed input/param accessors, diagnostics, and evaluation context.

use crate::ids::{AnalysisRange, NodeId, NodeTypeId, ParamId, ParamSpec, SocketId, SocketSpec};
use crate::value::{EventList, SocketValue};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Severity of a diagnostic attached to a node during validation or evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

/// Diagnostic message attached to a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDiagnostic {
    pub severity: DiagnosticSeverity,
    pub message: String,
}

impl NodeDiagnostic {
    pub fn info(message: impl Into<String>) -> Self {
        Self { severity: DiagnosticSeverity::Info, message: message.into() }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self { severity: DiagnosticSeverity::Warning, message: message.into() }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self { severity: DiagnosticSeverity::Error, message: message.into() }
    }
}

/// Static contract of one node type in the registry.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSpec {
    pub id: NodeTypeId,
    pub type_version: u32,
    pub inputs: Vec<SocketSpec>,
    pub outputs: Vec<SocketSpec>,
    pub params: Vec<ParamSpec>,
    /// Optional `(input_socket, output_socket)` passed straight through when the node is muted.
    pub bypass_passthrough: Option<(SocketId, SocketId)>,
}

impl NodeSpec {
    pub fn input(&self, id: &SocketId) -> Option<&SocketSpec> {
        self.inputs.iter().find(|s| &s.id == id)
    }

    pub fn output(&self, id: &SocketId) -> Option<&SocketSpec> {
        self.outputs.iter().find(|s| &s.id == id)
    }

    pub fn param(&self, id: &ParamId) -> Option<&ParamSpec> {
        self.params.iter().find(|p| &p.id == id)
    }
}

/// Resolved input socket values passed to `NodeType::evaluate`.
#[derive(Debug, Clone, Default)]
pub struct NodeInputs {
    values: BTreeMap<SocketId, Vec<SocketValue>>,
}

impl NodeInputs {
    pub fn new(values: BTreeMap<SocketId, Vec<SocketValue>>) -> Self {
        Self { values }
    }

    pub fn get(&self, socket: &str) -> Option<&SocketValue> {
        self.values.get(&SocketId::from(socket)).and_then(|v| v.first())
    }

    pub fn get_all(&self, socket: &str) -> &[SocketValue] {
        self.values
            .get(&SocketId::from(socket))
            .map_or(&[], Vec::as_slice)
    }
}

/// Parameter view combining explicit node values with declared defaults.
pub struct NodeParams<'a> {
    raw: &'a BTreeMap<String, serde_json::Value>,
    specs: &'a [ParamSpec],
}

impl<'a> NodeParams<'a> {
    pub fn new(raw: &'a BTreeMap<String, serde_json::Value>, specs: &'a [ParamSpec]) -> Self {
        Self { raw, specs }
    }

    pub fn raw(&self, id: &str) -> Option<&serde_json::Value> {
        self.raw
            .get(id)
            .or_else(|| self.specs.iter().find(|p| p.id.as_str() == id).map(|p| &p.default))
    }

    pub fn f64_or(&self, id: &str, fallback: f64) -> f64 {
        self.raw(id).and_then(|v| v.as_f64()).unwrap_or(fallback)
    }

    pub fn i64_or(&self, id: &str, fallback: i64) -> i64 {
        self.raw(id)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
            .unwrap_or(fallback)
    }

    pub fn usize_or(&self, id: &str, fallback: usize) -> usize {
        self.i64_or(id, fallback as i64).max(0) as usize
    }

    pub fn bool_or(&self, id: &str, fallback: bool) -> bool {
        self.raw(id).and_then(|v| v.as_bool()).unwrap_or(fallback)
    }

    pub fn str_or<'b>(&'b self, id: &str, fallback: &'b str) -> &'b str
    where
        'a: 'b,
    {
        self.raw(id).and_then(|v| v.as_str()).unwrap_or(fallback)
    }

    /// Canonical sorted JSON across all declared parameters (defaults filled in) plus any extra keys.
    pub fn effective_canonical_json(&self) -> String {
        let mut merged = BTreeMap::<String, serde_json::Value>::new();
        for spec in self.specs {
            merged.insert(spec.id.0.clone(), spec.default.clone());
        }
        for (k, v) in self.raw {
            merged.insert(k.clone(), canonical_json_value(v));
        }
        serde_json::to_string(&merged).unwrap_or_default()
    }
}

pub(crate) fn canonical_json_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonical_json_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (k, v) in map {
                sorted.insert(k.clone(), canonical_json_value(v));
            }
            let out: serde_json::Map<String, serde_json::Value> = sorted.into_iter().collect();
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Outputs and diagnostics produced by evaluating a single node.
#[derive(Debug, Clone, Default)]
pub struct NodeOutputs {
    pub values: BTreeMap<SocketId, SocketValue>,
    pub diagnostics: Vec<NodeDiagnostic>,
    /// Populated by `output.lane` nodes (`(slot_name, events)`).
    pub slot_output: Option<(String, Arc<EventList>)>,
    /// Populated by `output.viewer` nodes.
    pub viewer_output: Option<SocketValue>,
}

impl NodeOutputs {
    pub fn with_value(socket: impl Into<SocketId>, value: SocketValue) -> Self {
        let mut values = BTreeMap::new();
        values.insert(socket.into(), value);
        Self {
            values,
            ..Default::default()
        }
    }

    pub fn has_error(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
    }
}

/// Execution context passed to each node's `evaluate` call.
pub struct EvalContext<'a> {
    pub range: AnalysisRange,
    pub song_duration_ms: Option<f64>,
    cancel: &'a (dyn Fn() -> bool + Sync),
    progress: &'a mut dyn FnMut(f64),
}

impl<'a> EvalContext<'a> {
    pub fn new(
        range: AnalysisRange,
        song_duration_ms: Option<f64>,
        cancel: &'a (dyn Fn() -> bool + Sync),
        progress: &'a mut dyn FnMut(f64),
    ) -> Self {
        Self {
            range,
            song_duration_ms,
            cancel,
            progress,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        (self.cancel)()
    }

    pub fn cancel_fn(&self) -> &(dyn Fn() -> bool + Sync) {
        self.cancel
    }

    pub fn report_progress(&mut self, fraction: f64) {
        (self.progress)(fraction.clamp(0.0, 1.0));
    }
}

/// Fatal evaluation error stopping the graph run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    Cancelled,
    InvalidRange(String),
    Cycle(Vec<NodeId>),
    UnknownNode(NodeId),
    NodeFailed { node: NodeId, message: String },
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::Cancelled => write!(f, "graph evaluation was cancelled"),
            EvalError::InvalidRange(msg) => write!(f, "invalid analysis range: {msg}"),
            EvalError::Cycle(nodes) => {
                let ids: Vec<&str> = nodes.iter().map(NodeId::as_str).collect();
                write!(f, "graph contains a cycle involving: {}", ids.join(", "))
            }
            EvalError::UnknownNode(id) => write!(f, "target node '{id}' does not exist"),
            EvalError::NodeFailed { node, message } => {
                write!(f, "node '{node}' failed: {message}")
            }
        }
    }
}

impl std::error::Error for EvalError {}

/// Trait implemented by every registered node type.
pub trait NodeType: Send + Sync {
    fn spec(&self) -> NodeSpec;

    /// Migrates node parameters from `from_version` to `from_version + 1` (one step per call).
    fn migrate_params(
        &self,
        _from_version: u32,
        params: BTreeMap<String, serde_json::Value>,
    ) -> Result<BTreeMap<String, serde_json::Value>, String> {
        Ok(params)
    }

    /// Optional external source fingerprint (for example source file SHA-256 + offset + decoder id).
    fn source_cache_key(&self, _params: &NodeParams<'_>, _context: &EvalContext<'_>) -> Option<String> {
        None
    }

    fn evaluate(
        &self,
        inputs: &NodeInputs,
        params: &NodeParams<'_>,
        context: &mut EvalContext<'_>,
    ) -> Result<NodeOutputs, EvalError>;
}

/// Registry of available node types keyed by their stable `NodeTypeId`.
#[derive(Default, Clone)]
pub struct NodeRegistry {
    types: BTreeMap<NodeTypeId, Arc<dyn NodeType>>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, node_type: Arc<dyn NodeType>) -> Result<(), String> {
        let spec = node_type.spec();
        if self.types.contains_key(&spec.id) {
            return Err(format!("duplicate node type id '{}'", spec.id));
        }
        self.types.insert(spec.id, node_type);
        Ok(())
    }

    pub fn get(&self, id: &NodeTypeId) -> Option<&Arc<dyn NodeType>> {
        self.types.get(id)
    }

    pub fn specs(&self) -> Vec<NodeSpec> {
        self.types.values().map(|t| t.spec()).collect()
    }
}
