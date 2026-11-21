//! UI-free, audio-free offline node-graph engine (Layer 4 · Engines).
//!
//! References `dawsu-foundation`, `serde`, `serde_json`, and the standard library only.

pub mod cache;
pub mod document;
pub mod eval;
pub mod ids;
pub mod registry;
pub mod validation;
pub mod value;

pub use cache::{CacheKey, CacheStats, CachedNodeOutput, DEFAULT_MAX_BYTES, EvalCache};
pub use document::{
    CURRENT_DOCUMENT_VERSION, DocumentError, Endpoint, FORMAT_ID, GraphDocument, Link, NodeData,
    NodeUi, OutputDecl, PlaceholderInfo, PlaceholderReason,
};
pub use eval::{
    DirtyTracker, EvalTarget, GraphEvalResult, evaluate_graph, evaluate_graph_with_song_range,
};
pub use ids::{
    AnalysisRange, NodeId, NodeTypeId, NormaliseScope, ParamId, ParamSpec, SocketConstraint,
    SocketId, SocketSpec, SocketType,
};
pub use registry::{
    DiagnosticSeverity, EvalContext, EvalError, NodeDiagnostic, NodeInputs, NodeOutputs,
    NodeParams, NodeRegistry, NodeSpec, NodeType,
};
pub use validation::{
    LinkError, ValidationIssue, ValidationReport, can_connect, downstream_nodes, upstream_order,
    validate_graph,
};
pub use value::{
    AudioBuffer, Curve, EventList, GraphEvent, SocketValue, Spectrogram, TimingGrid, TimingLine,
};
