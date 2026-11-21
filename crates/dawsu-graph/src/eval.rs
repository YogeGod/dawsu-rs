//! Pull-based lazy DAG evaluation with dirty tracking and atomic cache commits.

use crate::cache::{CacheKey, CachedNodeOutput, EvalCache};
use crate::document::{Endpoint, GraphDocument, PlaceholderReason};
use crate::ids::{AnalysisRange, NodeId, SocketId};
use crate::registry::{
    EvalContext, EvalError, NodeDiagnostic, NodeInputs, NodeOutputs, NodeParams, NodeRegistry,
};
use crate::value::{EventList, SocketValue};
use crate::validation::{downstream_nodes, upstream_order};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Which sink(s) to pull through the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalTarget {
    /// Evaluate every `output.lane` node in the document.
    AllLaneOutputs,
    /// Evaluate a specific set of target nodes.
    Nodes(Vec<NodeId>),
    /// Preview one socket value (such as for the timeline Viewer).
    Socket(Endpoint),
}

/// Complete output of one pull evaluation run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GraphEvalResult {
    pub range: AnalysisRange,
    pub graph_hash: String,
    pub slot_events: BTreeMap<String, Arc<EventList>>,
    pub viewer_values: BTreeMap<NodeId, SocketValue>,
    pub socket_preview: Option<SocketValue>,
    pub diagnostics: BTreeMap<NodeId, Vec<NodeDiagnostic>>,
    pub evaluated_nodes: Vec<NodeId>,
    pub cached_nodes: Vec<NodeId>,
}

impl Default for AnalysisRange {
    fn default() -> Self {
        Self { from_ms: 0.0, to_ms: 1.0 }
    }
}

/// Tracks which nodes are dirty and propagates dirtiness downstream on edits.
#[derive(Debug, Clone, Default)]
pub struct DirtyTracker {
    dirty: BTreeSet<NodeId>,
}

impl DirtyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_dirty(&mut self, doc: &GraphDocument, roots: &[NodeId]) {
        self.dirty.extend(downstream_nodes(doc, roots));
    }

    pub fn mark_all_dirty(&mut self, doc: &GraphDocument) {
        self.dirty.extend(doc.nodes.iter().map(|n| n.id.clone()));
    }

    pub fn mark_clean(&mut self, id: &NodeId) {
        self.dirty.remove(id);
    }

    pub fn clear(&mut self) {
        self.dirty.clear();
    }

    pub fn is_dirty(&self, id: &NodeId) -> bool {
        self.dirty.contains(id)
    }

    pub fn dirty_nodes(&self) -> &BTreeSet<NodeId> {
        &self.dirty
    }
}

/// Evaluates only the nodes upstream of `target` over `range`.
/// Cancelled or failed runs write nothing to `cache` (Invariant 5c / R6).
#[allow(clippy::too_many_arguments)]
pub fn evaluate_graph(
    doc: &GraphDocument,
    registry: &NodeRegistry,
    cache: &mut EvalCache,
    target: EvalTarget,
    range: AnalysisRange,
    song_duration_ms: Option<f64>,
    cancel: &(dyn Fn() -> bool + Sync),
    progress: &mut dyn FnMut(f64),
) -> Result<GraphEvalResult, EvalError> {
    if cancel() {
        return Err(EvalError::Cancelled);
    }
    AnalysisRange::new(range.from_ms, range.to_ms).map_err(EvalError::InvalidRange)?;

    let target_nodes: Vec<NodeId> = match &target {
        EvalTarget::AllLaneOutputs => doc
            .nodes
            .iter()
            .filter(|n| n.type_id.as_str() == "output.lane")
            .map(|n| n.id.clone())
            .collect(),
        EvalTarget::Nodes(ids) => ids.clone(),
        EvalTarget::Socket(endpoint) => vec![endpoint.node.clone()],
    };

    if target_nodes.is_empty() {
        progress(1.0);
        return Ok(GraphEvalResult {
            range,
            graph_hash: doc.structural_hash(),
            ..Default::default()
        });
    }

    let order = upstream_order(doc, &target_nodes)?;
    let total = order.len().max(1);

    let mut node_outputs: BTreeMap<NodeId, CachedNodeOutput> = BTreeMap::new();
    let mut node_keys: BTreeMap<NodeId, CacheKey> = BTreeMap::new();
    let mut node_blocked: BTreeSet<NodeId> = BTreeSet::new();
    let mut pending_cache: Vec<(CacheKey, CachedNodeOutput)> = Vec::new();
    let mut evaluated_nodes = Vec::new();
    let mut cached_nodes = Vec::new();
    let mut diagnostics: BTreeMap<NodeId, Vec<NodeDiagnostic>> = BTreeMap::new();

    for (index, node_id) in order.iter().enumerate() {
        if cancel() {
            return Err(EvalError::Cancelled);
        }

        let node = doc
            .node(node_id)
            .ok_or_else(|| EvalError::UnknownNode(node_id.clone()))?;

        // Check placeholders first (Invariant 7 / R8).
        if let Some(placeholder) = &node.placeholder {
            let msg = match &placeholder.reason {
                PlaceholderReason::UnknownType => {
                    format!("needs node {} (unknown type '{}')", node.id, node.type_id)
                }
                PlaceholderReason::NewerVersion { found, known } => format!(
                    "needs node {} (type '{}' v{found} > known v{known})",
                    node.id, node.type_id
                ),
            };
            diagnostics.insert(node_id.clone(), vec![NodeDiagnostic::error(msg)]);
            node_blocked.insert(node_id.clone());
            progress((index + 1) as f64 / total as f64);
            continue;
        }

        let Some(node_type) = registry.get(&node.type_id) else {
            diagnostics.insert(
                node_id.clone(),
                vec![NodeDiagnostic::error(format!(
                    "needs node {} (unknown type '{}')",
                    node.id, node.type_id
                ))],
            );
            node_blocked.insert(node_id.clone());
            progress((index + 1) as f64 / total as f64);
            continue;
        };

        let spec = node_type.spec();
        let mut incoming: BTreeMap<SocketId, Vec<SocketValue>> = BTreeMap::new();
        let mut input_keys_map: BTreeMap<SocketId, Vec<CacheKey>> = BTreeMap::new();
        let mut upstream_error: Option<String> = None;

        // Collect links in deterministic document order.
        for link in doc.links_into(node_id) {
            if node_blocked.contains(&link.from.node) {
                if let Some(upstream) = doc.node(&link.from.node)
                    && upstream.placeholder.is_some()
                {
                    upstream_error.get_or_insert_with(|| {
                        format!(
                            "needs node {} (unknown type '{}')",
                            upstream.id, upstream.type_id
                        )
                    });
                } else {
                    upstream_error.get_or_insert_with(|| "waiting for input".to_owned());
                }
                continue;
            }
            if let Some(out) = node_outputs.get(&link.from.node)
                && let Some(val) = out.values.get(&link.from.socket)
            {
                incoming
                    .entry(link.to.socket.clone())
                    .or_default()
                    .push(val.clone());
                if let Some(up_key) = node_keys.get(&link.from.node) {
                    input_keys_map
                        .entry(link.to.socket.clone())
                        .or_default()
                        .push(up_key.clone());
                }
            }
        }

        if let Some(err_msg) = upstream_error {
            diagnostics.insert(node_id.clone(), vec![NodeDiagnostic::error(err_msg)]);
            node_blocked.insert(node_id.clone());
            progress((index + 1) as f64 / total as f64);
            continue;
        }

        // Check required inputs (unless muted with bypass passthrough missing only non-bypass inputs).
        let mut missing_required: Option<SocketId> = None;
        for input_spec in &spec.inputs {
            if input_spec.required
                && incoming
                    .get(&input_spec.id)
                    .is_none_or(Vec::is_empty)
            {
                missing_required = Some(input_spec.id.clone());
                break;
            }
        }
        if let Some(missing) = missing_required {
            diagnostics.insert(
                node_id.clone(),
                vec![NodeDiagnostic::error(format!("needs input {missing}"))],
            );
            node_blocked.insert(node_id.clone());
            progress((index + 1) as f64 / total as f64);
            continue;
        }

        let params_view = NodeParams::new(&node.params, &spec.params);
        let canonical_params = params_view.effective_canonical_json();
        let input_keys: Vec<(SocketId, Vec<CacheKey>)> = input_keys_map.into_iter().collect();

        let base_progress = index as f64 / total as f64;
        let span_progress = 1.0 / total as f64;
        let mut node_progress = |frac: f64| {
            progress(base_progress + frac.clamp(0.0, 1.0) * span_progress);
        };
        let mut ctx = EvalContext::new(range, song_duration_ms, cancel, &mut node_progress);
        let source_key = node_type.source_cache_key(&params_view, &ctx);

        let cache_key = CacheKey::compute(
            &node.type_id,
            spec.type_version,
            node.muted,
            &canonical_params,
            source_key.as_deref(),
            &input_keys,
            range,
            song_duration_ms,
        );
        node_keys.insert(node_id.clone(), cache_key.clone());

        if let Some(hit) = cache.get(&cache_key) {
            if !hit.diagnostics.is_empty() {
                diagnostics.insert(node_id.clone(), hit.diagnostics.clone());
            }
            node_outputs.insert(node_id.clone(), hit);
            cached_nodes.push(node_id.clone());
            progress((index + 1) as f64 / total as f64);
            continue;
        }

        let evaluated: NodeOutputs = if node.muted {
            if let Some((in_sock, out_sock)) = &spec.bypass_passthrough {
                if let Some(val) = incoming.get(in_sock).and_then(|v| v.first()).cloned() {
                    let mut out = NodeOutputs::with_value(out_sock.clone(), val);
                    out.diagnostics.push(NodeDiagnostic::info("muted (bypassed)"));
                    out
                } else {
                    NodeOutputs {
                        diagnostics: vec![NodeDiagnostic::info("muted")],
                        ..Default::default()
                    }
                }
            } else {
                NodeOutputs {
                    diagnostics: vec![NodeDiagnostic::info("muted")],
                    ..Default::default()
                }
            }
        } else {
            let inputs_view = NodeInputs::new(incoming);
            node_type.evaluate(&inputs_view, &params_view, &mut ctx)?
        };

        if cancel() {
            return Err(EvalError::Cancelled);
        }

        let has_error = evaluated.has_error();
        if !evaluated.diagnostics.is_empty() {
            diagnostics.insert(node_id.clone(), evaluated.diagnostics.clone());
        }
        let byte_size = CachedNodeOutput::estimate_bytes(
            &evaluated.values,
            &evaluated.slot_output,
            &evaluated.viewer_output,
        );
        let output = CachedNodeOutput {
            values: evaluated.values,
            diagnostics: evaluated.diagnostics,
            slot_output: evaluated.slot_output,
            viewer_output: evaluated.viewer_output,
            byte_size,
        };
        if has_error {
            node_blocked.insert(node_id.clone());
        } else {
            pending_cache.push((cache_key, output.clone()));
        }
        node_outputs.insert(node_id.clone(), output);
        evaluated_nodes.push(node_id.clone());
        progress((index + 1) as f64 / total as f64);
    }

    if cancel() {
        return Err(EvalError::Cancelled);
    }

    // Commit cache entries only once the entire run succeeds without cancellation (R6).
    for (key, out) in pending_cache {
        cache.insert(key, out);
    }

    let mut slot_events: BTreeMap<String, Arc<EventList>> = BTreeMap::new();
    let mut viewer_values: BTreeMap<NodeId, SocketValue> = BTreeMap::new();
    for id in &target_nodes {
        if let Some(out) = node_outputs.get(id) {
            if let Some((slot, events)) = &out.slot_output {
                slot_events.insert(slot.clone(), events.clone());
            }
            if let Some(viewer) = &out.viewer_output {
                viewer_values.insert(id.clone(), viewer.clone());
            }
        }
    }
    let socket_preview = match &target {
        EvalTarget::Socket(endpoint) => node_outputs
            .get(&endpoint.node)
            .and_then(|out| out.values.get(&endpoint.socket))
            .cloned(),
        _ => None,
    };

    Ok(GraphEvalResult {
        range,
        graph_hash: doc.structural_hash(),
        slot_events,
        viewer_values,
        socket_preview,
        diagnostics,
        evaluated_nodes,
        cached_nodes,
    })
}
