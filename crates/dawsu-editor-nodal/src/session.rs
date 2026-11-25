//! Undoable graph editing state machine built on `dawsu-editor::EditHistory`.

use dawsu_editor::{EditHistory, Selection};
use dawsu_graph::{
    DirtyTracker, GraphDocument, Link, LinkError, NodeData, NodeId, NodeRegistry, NodeTypeId,
    NodeUi, ValidationReport, can_connect, downstream_nodes, validate_graph,
};
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::fmt;

/// Error returned when a graph editing transition is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodalError {
    UnknownNodeType(NodeTypeId),
    UnknownNode(NodeId),
    LinkRefused(LinkError),
    PlaceholderImmutable(NodeId),
    TransactionOpen,
    NoOpenTransaction,
    InvalidPosition,
}

impl fmt::Display for NodalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodalError::UnknownNodeType(id) => write!(f, "unknown node type '{id}'"),
            NodalError::UnknownNode(id) => write!(f, "unknown node '{id}'"),
            NodalError::LinkRefused(err) => write!(f, "{err}"),
            NodalError::PlaceholderImmutable(id) => {
                write!(f, "cannot edit placeholder node '{id}'")
            }
            NodalError::TransactionOpen => {
                write!(f, "finish the open graph transaction first")
            }
            NodalError::NoOpenTransaction => {
                write!(f, "no graph move transaction is open")
            }
            NodalError::InvalidPosition => {
                write!(f, "node coordinates must be finite")
            }
        }
    }
}

impl std::error::Error for NodalError {}

/// Undoable editing session over one `GraphDocument`.
pub struct NodalSession {
    history: EditHistory<GraphDocument>,
    selection: Selection<NodeId>,
    dirty: DirtyTracker,
    next_id: u64,
}

impl NodalSession {
    pub fn new(doc: GraphDocument) -> Self {
        let mut dirty = DirtyTracker::new();
        dirty.mark_all_dirty(&doc);
        let next_id = next_free_counter(&doc);
        Self {
            history: EditHistory::new(doc),
            selection: Selection::new(),
            dirty,
            next_id,
        }
    }

    pub fn document(&self) -> &GraphDocument {
        self.history.document()
    }

    pub fn selection(&self) -> &Selection<NodeId> {
        &self.selection
    }

    pub fn selection_mut(&mut self) -> &mut Selection<NodeId> {
        &mut self.selection
    }

    pub fn revision(&self) -> u64 {
        self.history.version()
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_clean();
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn dirty_nodes(&self) -> &BTreeSet<NodeId> {
        self.dirty.dirty_nodes()
    }

    pub fn clear_dirty(&mut self, nodes: &[NodeId]) {
        for id in nodes {
            self.dirty.mark_clean(id);
        }
    }

    pub fn validate(&self, registry: &NodeRegistry) -> ValidationReport {
        validate_graph(self.document(), registry)
    }

    fn alloc_id(&mut self) -> NodeId {
        loop {
            self.next_id += 1;
            let candidate = NodeId::from(format!("n{}", self.next_id));
            if self.history.document().node(&candidate).is_none() {
                return candidate;
            }
        }
    }

    /// Adds a new node of `type_id` at `(x, y)` with its default parameters as one undo step.
    pub fn add_node(
        &mut self,
        registry: &NodeRegistry,
        type_id: &NodeTypeId,
        x: f64,
        y: f64,
    ) -> Result<NodeId, NodalError> {
        if self.history.is_transaction_open() {
            return Err(NodalError::TransactionOpen);
        }
        let node_type = registry
            .get(type_id)
            .ok_or_else(|| NodalError::UnknownNodeType(type_id.clone()))?;
        let spec = node_type.spec();
        let id = self.alloc_id();
        let mut params = BTreeMap::new();
        for p in &spec.params {
            params.insert(p.id.0.clone(), p.default.clone());
        }
        let node = NodeData {
            id: id.clone(),
            type_id: spec.id,
            type_version: spec.type_version,
            params,
            muted: false,
            ui: NodeUi { x, y },
            placeholder: None,
        };
        let _ = self.history.apply("Add node", true, |doc| {
            doc.nodes.push(node);
            Ok::<_, Infallible>(())
        });
        self.dirty
            .mark_dirty(self.history.document(), std::slice::from_ref(&id));
        self.selection.replace([id.clone()]);
        Ok(id)
    }

    /// Removes `ids` and all links touching them as one undo step, dirtying downstream nodes.
    pub fn remove_nodes(&mut self, ids: &[NodeId]) -> Result<usize, NodalError> {
        if self.history.is_transaction_open() {
            return Err(NodalError::TransactionOpen);
        }
        let targets: BTreeSet<NodeId> = ids
            .iter()
            .filter(|id| self.document().node(id).is_some())
            .cloned()
            .collect();
        if targets.is_empty() {
            return Ok(0);
        }
        let downstream: Vec<NodeId> = downstream_nodes(
            self.document(),
            &targets.iter().cloned().collect::<Vec<_>>(),
        )
        .into_iter()
        .filter(|id| !targets.contains(id))
        .collect();

        let removed = targets.len();
        let _ = self.history.apply("Delete nodes", true, |doc| {
            doc.nodes.retain(|n| !targets.contains(&n.id));
            doc.links
                .retain(|l| !targets.contains(&l.from.node) && !targets.contains(&l.to.node));
            Ok::<_, Infallible>(())
        });
        self.dirty.mark_dirty(self.history.document(), &downstream);
        for id in &targets {
            self.dirty.mark_clean(id);
        }
        self.selection
            .retain(|id| self.history.document().node(id).is_some());
        Ok(removed)
    }

    pub fn remove_selected(&mut self) -> Result<usize, NodalError> {
        let ids: Vec<NodeId> = self.selection.items().iter().cloned().collect();
        self.remove_nodes(&ids)
    }

    /// Duplicates all selected nodes (and internal links between them) with `(offset_x, offset_y)`.
    pub fn duplicate_selected(
        &mut self,
        offset_x: f64,
        offset_y: f64,
    ) -> Result<Vec<NodeId>, NodalError> {
        if self.history.is_transaction_open() {
            return Err(NodalError::TransactionOpen);
        }
        let selected: BTreeSet<NodeId> = self.selection.items().iter().cloned().collect();
        let originals: Vec<NodeData> = self
            .document()
            .nodes
            .iter()
            .filter(|n| selected.contains(&n.id))
            .cloned()
            .collect();
        if originals.is_empty() {
            return Ok(Vec::new());
        }
        if !offset_x.is_finite() || !offset_y.is_finite() {
            return Err(NodalError::InvalidPosition);
        }
        if let Some(placeholder) = originals.iter().find(|n| n.placeholder.is_some()) {
            return Err(NodalError::PlaceholderImmutable(placeholder.id.clone()));
        }
        let mut id_map = BTreeMap::new();
        let mut clones = Vec::with_capacity(originals.len());
        let mut new_ids = Vec::with_capacity(originals.len());
        for mut orig in originals {
            let new_id = self.alloc_id();
            id_map.insert(orig.id.clone(), new_id.clone());
            orig.id = new_id.clone();
            orig.ui.x += offset_x;
            orig.ui.y += offset_y;
            clones.push(orig);
            new_ids.push(new_id);
        }
        let internal_links: Vec<Link> = self
            .document()
            .links
            .iter()
            .filter_map(|l| {
                let from = id_map.get(&l.from.node)?.clone();
                let to = id_map.get(&l.to.node)?.clone();
                Some(Link::new(
                    dawsu_graph::Endpoint::new(from, l.from.socket.clone()),
                    dawsu_graph::Endpoint::new(to, l.to.socket.clone()),
                ))
            })
            .collect();
        let _ = self.history.apply("Duplicate nodes", true, |doc| {
            doc.nodes.extend(clones);
            doc.links.extend(internal_links);
            Ok::<_, Infallible>(())
        });
        self.dirty.mark_dirty(self.history.document(), &new_ids);
        self.selection.replace(new_ids.clone());
        Ok(new_ids)
    }

    /// Starts a node-drag transaction so multiple `move_nodes` updates collapse into one undo step.
    pub fn begin_move(&mut self) {
        self.history.begin("Move nodes");
    }

    /// Updates node positions without marking any node's DSP cache dirty.
    pub fn move_nodes(&mut self, positions: &[(NodeId, f64, f64)]) -> Result<(), NodalError> {
        if !self.history.is_transaction_open() {
            return Err(NodalError::NoOpenTransaction);
        }
        let mut changed = false;
        for (id, x, y) in positions {
            if !x.is_finite() || !y.is_finite() {
                return Err(NodalError::InvalidPosition);
            }
            let Some(node) = self.document().node(id) else {
                continue;
            };
            if node.placeholder.is_some() {
                return Err(NodalError::PlaceholderImmutable(id.clone()));
            }
            if node.ui.x != *x || node.ui.y != *y {
                changed = true;
            }
        }
        if !changed {
            return Ok(());
        }
        self
            .history
            .edit(true, |doc| {
                for (id, x, y) in positions {
                    if let Some(node) = doc.node_mut(id) {
                        node.ui.x = *x;
                        node.ui.y = *y;
                    }
                }
                Ok::<_, Infallible>(())
            })
            .map_err(|_| NodalError::NoOpenTransaction)?;
        Ok(())
    }

    pub fn commit_move(&mut self) {
        self.history.commit();
    }

    pub fn cancel_move(&mut self) {
        self.history.cancel();
    }

    /// Validates and adds `link` as one undo step; invalid links are refused without touching history.
    pub fn connect(&mut self, registry: &NodeRegistry, link: Link) -> Result<(), NodalError> {
        if self.history.is_transaction_open() {
            return Err(NodalError::TransactionOpen);
        }
        can_connect(self.document(), registry, &link).map_err(NodalError::LinkRefused)?;
        let target = link.to.node.clone();
        let _ = self.history.apply("Connect nodes", true, |doc| {
            doc.links.push(link);
            Ok::<_, Infallible>(())
        });
        self.dirty.mark_dirty(self.history.document(), &[target]);
        Ok(())
    }

    /// Removes `link` as one undo step and marks downstream nodes dirty.
    pub fn disconnect(&mut self, link: &Link) -> Result<bool, NodalError> {
        if self.history.is_transaction_open() {
            return Err(NodalError::TransactionOpen);
        }
        if !self.document().links.contains(link) {
            return Ok(false);
        }
        let target = link.to.node.clone();
        let _ = self.history.apply("Disconnect nodes", true, |doc| {
            doc.links.retain(|l| l != link);
            Ok::<_, Infallible>(())
        });
        self.dirty.mark_dirty(self.history.document(), &[target]);
        Ok(true)
    }

    /// Updates one parameter on `node_id` and marks the node and its downstream graph dirty.
    pub fn set_param(
        &mut self,
        node_id: &NodeId,
        param: &str,
        value: serde_json::Value,
    ) -> Result<(), NodalError> {
        if self.history.is_transaction_open() {
            return Err(NodalError::TransactionOpen);
        }
        let Some(node) = self.document().node(node_id) else {
            return Err(NodalError::UnknownNode(node_id.clone()));
        };
        if node.placeholder.is_some() {
            return Err(NodalError::PlaceholderImmutable(node_id.clone()));
        }
        if node.params.get(param) == Some(&value) {
            return Ok(());
        }
        let _ = self.history.apply("Edit node parameter", true, |doc| {
            if let Some(n) = doc.node_mut(node_id) {
                n.params.insert(param.to_owned(), value);
            }
            Ok::<_, Infallible>(())
        });
        self.dirty
            .mark_dirty(self.history.document(), std::slice::from_ref(node_id));
        Ok(())
    }

    pub fn set_muted(&mut self, node_id: &NodeId, muted: bool) -> Result<(), NodalError> {
        if self.history.is_transaction_open() {
            return Err(NodalError::TransactionOpen);
        }
        let Some(node) = self.document().node(node_id) else {
            return Err(NodalError::UnknownNode(node_id.clone()));
        };
        if node.placeholder.is_some() {
            return Err(NodalError::PlaceholderImmutable(node_id.clone()));
        }
        if node.muted == muted {
            return Ok(());
        }
        let _ = self.history.apply(
            if muted { "Mute node" } else { "Unmute node" },
            true,
            |doc| {
                if let Some(n) = doc.node_mut(node_id) {
                    n.muted = muted;
                }
                Ok::<_, Infallible>(())
            },
        );
        self.dirty
            .mark_dirty(self.history.document(), std::slice::from_ref(node_id));
        Ok(())
    }

    pub fn toggle_muted(&mut self, node_id: &NodeId) -> Result<bool, NodalError> {
        let next = !self
            .document()
            .node(node_id)
            .ok_or_else(|| NodalError::UnknownNode(node_id.clone()))?
            .muted;
        self.set_muted(node_id, next)?;
        Ok(next)
    }

    /// Selects nodes whose `ui` coordinates fall inside `[x0..=x1, y0..=y1]`.
    pub fn box_select(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, add: bool) -> Vec<NodeId> {
        let (min_x, max_x) = (x0.min(x1), x0.max(x1));
        let (min_y, max_y) = (y0.min(y1), y0.max(y1));
        let hits: Vec<NodeId> = self
            .document()
            .nodes
            .iter()
            .filter(|n| {
                n.ui.x >= min_x && n.ui.x <= max_x && n.ui.y >= min_y && n.ui.y <= max_y
            })
            .map(|n| n.id.clone())
            .collect();
        if add {
            for id in &hits {
                self.selection.add(id.clone());
            }
        } else {
            self.selection.replace(hits.clone());
        }
        hits
    }

    pub fn undo(&mut self) -> bool {
        if !self.history.undo() {
            return false;
        }
        self.selection
            .retain(|id| self.history.document().node(id).is_some());
        self.dirty.clear();
        self.dirty.mark_all_dirty(self.history.document());
        true
    }

    pub fn redo(&mut self) -> bool {
        if !self.history.redo() {
            return false;
        }
        self.selection
            .retain(|id| self.history.document().node(id).is_some());
        self.dirty.clear();
        self.dirty.mark_all_dirty(self.history.document());
        true
    }
}

fn next_free_counter(doc: &GraphDocument) -> u64 {
    doc.nodes
        .iter()
        .filter_map(|n| n.id.as_str().strip_prefix('n')?.parse::<u64>().ok())
        .max()
        .unwrap_or(doc.nodes.len() as u64)
}
