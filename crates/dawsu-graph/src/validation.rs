//! Graph validation, link rules, cycle checks, upstream pull ordering, and downstream reachability.

use crate::document::{Link, NodeData, GraphDocument};
use crate::ids::{NodeId, SocketConstraint, SocketId, SocketType};
use crate::registry::{EvalError, NodeRegistry};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Reason a link cannot be created or why a loaded link is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    UnknownSourceNode(NodeId),
    UnknownTargetNode(NodeId),
    UnknownSourceSocket { node: NodeId, socket: SocketId },
    UnknownTargetSocket { node: NodeId, socket: SocketId },
    PlaceholderEndpoint(NodeId),
    TypeMismatch { from: SocketType, to: SocketType },
    ReservedSocketType(SocketType),
    SelfLoop(NodeId),
    CreatesCycle { from: NodeId, to: NodeId },
    DuplicateLink,
    InputAlreadyConnected { node: NodeId, socket: SocketId },
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::UnknownSourceNode(id) => write!(f, "unknown source node '{id}'"),
            LinkError::UnknownTargetNode(id) => write!(f, "unknown target node '{id}'"),
            LinkError::UnknownSourceSocket { node, socket } => {
                write!(f, "node '{node}' has no output socket '{socket}'")
            }
            LinkError::UnknownTargetSocket { node, socket } => {
                write!(f, "node '{node}' has no input socket '{socket}'")
            }
            LinkError::PlaceholderEndpoint(id) => {
                write!(f, "cannot link placeholder node '{id}'")
            }
            LinkError::TypeMismatch { from, to } => {
                write!(f, "socket type mismatch: cannot connect {from} to {to}")
            }
            LinkError::ReservedSocketType(ty) => {
                write!(f, "socket type '{ty}' is reserved and cannot be linked in v1")
            }
            LinkError::SelfLoop(id) => write!(f, "cannot connect node '{id}' to itself"),
            LinkError::CreatesCycle { from, to } => {
                write!(f, "connecting '{from}' to '{to}' would create a cycle")
            }
            LinkError::DuplicateLink => write!(f, "that link already exists"),
            LinkError::InputAlreadyConnected { node, socket } => {
                write!(f, "input '{node}.{socket}' already has a link")
            }
        }
    }
}

/// Non-destructive validation issue recorded on a loaded or edited graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationIssue {
    DuplicateNodeId(NodeId),
    PlaceholderNode { node: NodeId, type_id: String },
    InvalidLink { link: Link, error: LinkError },
    MissingRequiredInput { node: NodeId, socket: SocketId },
    CycleDetected(Vec<NodeId>),
    InvalidParam { node: NodeId, message: String },
}

impl ValidationIssue {
    pub fn involves_node(&self, id: &NodeId) -> bool {
        match self {
            ValidationIssue::DuplicateNodeId(n)
            | ValidationIssue::PlaceholderNode { node: n, .. }
            | ValidationIssue::MissingRequiredInput { node: n, .. }
            | ValidationIssue::InvalidParam { node: n, .. } => n == id,
            ValidationIssue::InvalidLink { link, .. } => &link.from.node == id || &link.to.node == id,
            ValidationIssue::CycleDetected(nodes) => nodes.contains(id),
        }
    }
}

/// Complete validation report for a `GraphDocument`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValidationReport {
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn issues_for_node<'a>(&'a self, id: &'a NodeId) -> Vec<&'a ValidationIssue> {
        self.issues.iter().filter(|issue| issue.involves_node(id)).collect()
    }
}

/// Checks whether adding `link` to `doc` is valid (strict socket match, arity, and acyclicity).
pub fn can_connect(
    doc: &GraphDocument,
    registry: &NodeRegistry,
    link: &Link,
) -> Result<(), LinkError> {
    if link.from.node == link.to.node {
        return Err(LinkError::SelfLoop(link.from.node.clone()));
    }
    if doc.links.contains(link) {
        return Err(LinkError::DuplicateLink);
    }
    let from_node = doc
        .node(&link.from.node)
        .ok_or_else(|| LinkError::UnknownSourceNode(link.from.node.clone()))?;
    let to_node = doc
        .node(&link.to.node)
        .ok_or_else(|| LinkError::UnknownTargetNode(link.to.node.clone()))?;
    if from_node.placeholder.is_some() {
        return Err(LinkError::PlaceholderEndpoint(from_node.id.clone()));
    }
    if to_node.placeholder.is_some() {
        return Err(LinkError::PlaceholderEndpoint(to_node.id.clone()));
    }
    let from_type = registry
        .get(&from_node.type_id)
        .ok_or_else(|| LinkError::PlaceholderEndpoint(from_node.id.clone()))?;
    let to_type = registry
        .get(&to_node.type_id)
        .ok_or_else(|| LinkError::PlaceholderEndpoint(to_node.id.clone()))?;

    let from_spec = from_type.spec();
    let to_spec = to_type.spec();
    let out_sock = from_spec
        .output(&link.from.socket)
        .ok_or_else(|| LinkError::UnknownSourceSocket {
            node: from_node.id.clone(),
            socket: link.from.socket.clone(),
        })?;
    let in_sock = to_spec
        .input(&link.to.socket)
        .ok_or_else(|| LinkError::UnknownTargetSocket {
            node: to_node.id.clone(),
            socket: link.to.socket.clone(),
        })?;

    if !in_sock.multi
        && doc
            .links
            .iter()
            .any(|existing| existing.to == link.to)
    {
        return Err(LinkError::InputAlreadyConnected {
            node: link.to.node.clone(),
            socket: link.to.socket.clone(),
        });
    }

    match (out_sock.kind, in_sock.kind) {
        (SocketConstraint::Exact(from_ty), SocketConstraint::Exact(to_ty)) if from_ty != to_ty => {
            return Err(LinkError::TypeMismatch {
                from: from_ty,
                to: to_ty,
            });
        }
        (SocketConstraint::Exact(from_ty), SocketConstraint::Any)
            if !SocketConstraint::Any.accepts(from_ty) =>
        {
            return Err(LinkError::ReservedSocketType(from_ty));
        }
        _ => {}
    }

    // Check if `link.from.node` is reachable from `link.to.node`, which would close a cycle.
    if is_reachable(doc, &link.to.node, &link.from.node) {
        return Err(LinkError::CreatesCycle {
            from: link.from.node.clone(),
            to: link.to.node.clone(),
        });
    }
    Ok(())
}

/// Validates an entire loaded or edited document without discarding invalid links (Invariant 8).
pub fn validate_graph(doc: &GraphDocument, registry: &NodeRegistry) -> ValidationReport {
    let mut issues = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for node in &doc.nodes {
        if !seen_ids.insert(node.id.clone()) {
            issues.push(ValidationIssue::DuplicateNodeId(node.id.clone()));
        }
        if node.placeholder.is_some() {
            issues.push(ValidationIssue::PlaceholderNode {
                node: node.id.clone(),
                type_id: node.type_id.0.clone(),
            });
            continue;
        }
        let Some(node_type) = registry.get(&node.type_id) else {
            issues.push(ValidationIssue::PlaceholderNode {
                node: node.id.clone(),
                type_id: node.type_id.0.clone(),
            });
            continue;
        };
        let spec = node_type.spec();
        for input in &spec.inputs {
            if input.required
                && !doc
                    .links
                    .iter()
                    .any(|l| l.to.node == node.id && l.to.socket == input.id)
            {
                issues.push(ValidationIssue::MissingRequiredInput {
                    node: node.id.clone(),
                    socket: input.id.clone(),
                });
            }
        }
        validate_node_params(node, &spec.params, &mut issues);
    }

    let mut seen_links = BTreeSet::new();
    let mut input_counts: BTreeMap<(NodeId, SocketId), usize> = BTreeMap::new();
    for link in &doc.links {
        if !seen_links.insert(link.clone()) {
            issues.push(ValidationIssue::InvalidLink {
                link: link.clone(),
                error: LinkError::DuplicateLink,
            });
            continue;
        }
        if link.from.node == link.to.node {
            issues.push(ValidationIssue::InvalidLink {
                link: link.clone(),
                error: LinkError::SelfLoop(link.from.node.clone()),
            });
            continue;
        }
        let Some(from_node) = doc.node(&link.from.node) else {
            issues.push(ValidationIssue::InvalidLink {
                link: link.clone(),
                error: LinkError::UnknownSourceNode(link.from.node.clone()),
            });
            continue;
        };
        let Some(to_node) = doc.node(&link.to.node) else {
            issues.push(ValidationIssue::InvalidLink {
                link: link.clone(),
                error: LinkError::UnknownTargetNode(link.to.node.clone()),
            });
            continue;
        };
        if from_node.placeholder.is_some() || to_node.placeholder.is_some() {
            continue;
        }
        let Some(from_type) = registry.get(&from_node.type_id) else {
            continue;
        };
        let Some(to_type) = registry.get(&to_node.type_id) else {
            continue;
        };
        let from_spec = from_type.spec();
        let to_spec = to_type.spec();
        let Some(out_sock) = from_spec.output(&link.from.socket) else {
            issues.push(ValidationIssue::InvalidLink {
                link: link.clone(),
                error: LinkError::UnknownSourceSocket {
                    node: from_node.id.clone(),
                    socket: link.from.socket.clone(),
                },
            });
            continue;
        };
        let Some(in_sock) = to_spec.input(&link.to.socket) else {
            issues.push(ValidationIssue::InvalidLink {
                link: link.clone(),
                error: LinkError::UnknownTargetSocket {
                    node: to_node.id.clone(),
                    socket: link.to.socket.clone(),
                },
            });
            continue;
        };
        if !in_sock.multi {
            let count = input_counts
                .entry((link.to.node.clone(), link.to.socket.clone()))
                .or_insert(0);
            *count += 1;
            if *count > 1 {
                issues.push(ValidationIssue::InvalidLink {
                    link: link.clone(),
                    error: LinkError::InputAlreadyConnected {
                        node: link.to.node.clone(),
                        socket: link.to.socket.clone(),
                    },
                });
            }
        }
        match (out_sock.kind, in_sock.kind) {
            (SocketConstraint::Exact(from_ty), SocketConstraint::Exact(to_ty)) if from_ty != to_ty => {
                issues.push(ValidationIssue::InvalidLink {
                    link: link.clone(),
                    error: LinkError::TypeMismatch {
                        from: from_ty,
                        to: to_ty,
                    },
                });
            }
            (SocketConstraint::Exact(from_ty), SocketConstraint::Any)
                if !SocketConstraint::Any.accepts(from_ty) =>
            {
                issues.push(ValidationIssue::InvalidLink {
                    link: link.clone(),
                    error: LinkError::ReservedSocketType(from_ty),
                });
            }
            _ => {}
        }
    }

    let cycle_nodes = find_cycle_nodes(doc);
    if !cycle_nodes.is_empty() {
        issues.push(ValidationIssue::CycleDetected(cycle_nodes));
    }
    ValidationReport { issues }
}

fn validate_node_params(
    node: &NodeData,
    specs: &[crate::ids::ParamSpec],
    issues: &mut Vec<ValidationIssue>,
) {
    for spec in specs {
        let Some(value) = node.params.get(spec.id.as_str()) else {
            continue;
        };
        if value.is_null() && spec.default.is_null() {
            continue;
        }
        match spec.socket_type {
            SocketType::Number => {
                if !value.is_number() {
                    issues.push(ValidationIssue::InvalidParam {
                        node: node.id.clone(),
                        message: format!("parameter '{}' must be a number", spec.id),
                    });
                    continue;
                }
            }
            SocketType::Integer => {
                if !(value.is_i64() || value.is_u64()) {
                    issues.push(ValidationIssue::InvalidParam {
                        node: node.id.clone(),
                        message: format!("parameter '{}' must be an integer", spec.id),
                    });
                    continue;
                }
            }
            SocketType::Bool => {
                if !value.is_boolean() {
                    issues.push(ValidationIssue::InvalidParam {
                        node: node.id.clone(),
                        message: format!("parameter '{}' must be a boolean", spec.id),
                    });
                    continue;
                }
            }
            SocketType::Enum if !spec.enum_values.is_empty() => {
                if !value.is_string() {
                    issues.push(ValidationIssue::InvalidParam {
                        node: node.id.clone(),
                        message: format!("parameter '{}' must be a string variant", spec.id),
                    });
                    continue;
                }
            }
            _ => {}
        }
        if let Some(num) = value.as_f64() {
            if !num.is_finite() {
                issues.push(ValidationIssue::InvalidParam {
                    node: node.id.clone(),
                    message: format!("parameter '{}' must be finite", spec.id),
                });
                continue;
            }
            if let Some(min) = spec.min
                && num < min
            {
                issues.push(ValidationIssue::InvalidParam {
                    node: node.id.clone(),
                    message: format!("parameter '{}' ({num}) is below minimum {min}", spec.id),
                });
            }
            if let Some(max) = spec.max
                && num > max
            {
                issues.push(ValidationIssue::InvalidParam {
                    node: node.id.clone(),
                    message: format!("parameter '{}' ({num}) is above maximum {max}", spec.id),
                });
            }
        }
        if !spec.enum_values.is_empty()
            && let Some(s) = value.as_str()
            && !spec.enum_values.iter().any(|allowed| allowed == s)
        {
            issues.push(ValidationIssue::InvalidParam {
                node: node.id.clone(),
                message: format!("parameter '{}' has invalid variant '{s}'", spec.id),
            });
        }
    }
}

/// Returns the deterministic topological evaluation order for `targets` and their upstream ancestors only.
pub fn upstream_order(doc: &GraphDocument, targets: &[NodeId]) -> Result<Vec<NodeId>, EvalError> {
    let mut needed = BTreeSet::new();
    let mut queue = VecDeque::new();
    for target in targets {
        if doc.node(target).is_none() {
            return Err(EvalError::UnknownNode(target.clone()));
        }
        if needed.insert(target.clone()) {
            queue.push_back(target.clone());
        }
    }
    while let Some(id) = queue.pop_front() {
        for link in doc.links_into(&id) {
            if doc.node(&link.from.node).is_some() && needed.insert(link.from.node.clone()) {
                queue.push_back(link.from.node.clone());
            }
        }
    }
    topological_subset(doc, &needed)
}

/// Returns `roots` together with every node transitively reachable downstream of them.
pub fn downstream_nodes(doc: &GraphDocument, roots: &[NodeId]) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    let mut queue = VecDeque::new();
    for root in roots {
        if doc.node(root).is_some() && out.insert(root.clone()) {
            queue.push_back(root.clone());
        }
    }
    while let Some(id) = queue.pop_front() {
        for link in doc.links_from(&id) {
            if doc.node(&link.to.node).is_some() && out.insert(link.to.node.clone()) {
                queue.push_back(link.to.node.clone());
            }
        }
    }
    out
}

fn is_reachable(doc: &GraphDocument, start: &NodeId, goal: &NodeId) -> bool {
    let mut visited = BTreeSet::new();
    let mut stack = vec![start.clone()];
    while let Some(node) = stack.pop() {
        if &node == goal {
            return true;
        }
        if visited.insert(node.clone()) {
            for link in doc.links_from(&node) {
                stack.push(link.to.node.clone());
            }
        }
    }
    false
}

fn find_cycle_nodes(doc: &GraphDocument) -> Vec<NodeId> {
    let all: BTreeSet<NodeId> = doc.nodes.iter().map(|n| n.id.clone()).collect();
    let Err(EvalError::Cycle(stuck)) = topological_subset(doc, &all) else {
        return Vec::new();
    };
    // Reverse-prune nodes with outdegree 0 inside the remaining subgraph so acyclic downstream
    // nodes fed by a cycle are excluded from the reported cycle set.
    let mut active: BTreeSet<NodeId> = stuck.into_iter().collect();
    loop {
        let mut has_outgoing = BTreeSet::new();
        for link in &doc.links {
            if active.contains(&link.from.node) && active.contains(&link.to.node) {
                has_outgoing.insert(link.from.node.clone());
            }
        }
        let before = active.len();
        active.retain(|id| has_outgoing.contains(id));
        if active.len() == before {
            break;
        }
    }
    active.into_iter().collect()
}

fn topological_subset(
    doc: &GraphDocument,
    subset: &BTreeSet<NodeId>,
) -> Result<Vec<NodeId>, EvalError> {
    let mut indegree: BTreeMap<NodeId, usize> = subset.iter().map(|id| (id.clone(), 0)).collect();
    let mut outgoing: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    for link in &doc.links {
        if subset.contains(&link.from.node) && subset.contains(&link.to.node) {
            let targets = outgoing.entry(link.from.node.clone()).or_default();
            if targets.insert(link.to.node.clone()) {
                *indegree.entry(link.to.node.clone()).or_insert(0) += 1;
            }
        }
    }
    let mut ready: BTreeSet<NodeId> = indegree
        .iter()
        .filter_map(|(id, &deg)| (deg == 0).then_some(id.clone()))
        .collect();
    let mut order = Vec::with_capacity(subset.len());
    while let Some(id) = ready.pop_first() {
        order.push(id.clone());
        if let Some(nexts) = outgoing.get(&id) {
            for next in nexts {
                if let Some(deg) = indegree.get_mut(next) {
                    *deg -= 1;
                    if *deg == 0 {
                        ready.insert(next.clone());
                    }
                }
            }
        }
    }
    if order.len() != subset.len() {
        let stuck: Vec<NodeId> = indegree
            .into_iter()
            .filter_map(|(id, deg)| (deg > 0).then_some(id))
            .collect();
        return Err(EvalError::Cycle(stuck));
    }
    Ok(order)
}
