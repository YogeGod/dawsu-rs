//! Graph document model, `.dawsugraph` JSON serialization, migrations, and placeholder preservation.

use crate::ids::{NodeId, NodeTypeId, SocketId};
use crate::registry::{NodeRegistry, canonical_json_value};
use dawsu_foundation::digest::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const FORMAT_ID: &str = "dawsu-graph";
pub const CURRENT_DOCUMENT_VERSION: u32 = 1;

/// Canvas position for one node (UI-only metadata; excluded from structural/cache hashes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeUi {
    pub x: f64,
    pub y: f64,
}

/// Why a node loaded as a disabled placeholder instead of a live registered type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceholderReason {
    UnknownType,
    NewerVersion { found: u32, known: u32 },
}

/// Original raw JSON retained for an unknown or newer-version node so saving writes it back unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceholderInfo {
    pub reason: PlaceholderReason,
    pub raw: serde_json::Value,
}

/// One node instance inside a `GraphDocument`.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeData {
    pub id: NodeId,
    pub type_id: NodeTypeId,
    pub type_version: u32,
    pub params: BTreeMap<String, serde_json::Value>,
    pub muted: bool,
    pub ui: NodeUi,
    pub placeholder: Option<PlaceholderInfo>,
}

/// A directed socket reference (`node` + `socket`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Endpoint {
    pub node: NodeId,
    pub socket: SocketId,
}

impl Endpoint {
    pub fn new(node: impl Into<NodeId>, socket: impl Into<SocketId>) -> Self {
        Self {
            node: node.into(),
            socket: socket.into(),
        }
    }
}

/// A typed link from an output endpoint to an input endpoint.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Link {
    pub from: Endpoint,
    pub to: Endpoint,
}

impl Link {
    pub fn new(from: Endpoint, to: Endpoint) -> Self {
        Self { from, to }
    }
}

/// Declared output slot metadata in `.dawsugraph` files.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OutputDecl {
    pub slot: String,
}

/// Serializable node-graph document (`.dawsugraph`).
#[derive(Debug, Clone, PartialEq)]
pub struct GraphDocument {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub description: String,
    pub nodes: Vec<NodeData>,
    pub links: Vec<Link>,
    pub outputs: Vec<OutputDecl>,
}

impl Default for GraphDocument {
    fn default() -> Self {
        Self::new("")
    }
}

/// Error returned while reading, migrating, or writing a `.dawsugraph` document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    InvalidJson(String),
    InvalidFormat(String),
    UnsupportedVersion(u32),
    MigrationFailed { node: NodeId, message: String },
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentError::InvalidJson(e) => write!(f, "invalid graph JSON: {e}"),
            DocumentError::InvalidFormat(fmt_id) => {
                write!(f, "unexpected graph format '{fmt_id}' (expected '{FORMAT_ID}')")
            }
            DocumentError::UnsupportedVersion(v) => {
                write!(f, "unsupported graph document version {v}")
            }
            DocumentError::MigrationFailed { node, message } => {
                write!(f, "failed migrating node '{node}': {message}")
            }
        }
    }
}

impl std::error::Error for DocumentError {}

#[derive(Deserialize)]
struct RawDocument {
    format: String,
    version: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    nodes: Vec<serde_json::Value>,
    #[serde(default)]
    links: Vec<Link>,
    #[serde(default)]
    outputs: Vec<OutputDecl>,
}

#[derive(Serialize, Deserialize)]
struct SerialNode {
    id: NodeId,
    #[serde(rename = "type")]
    type_id: NodeTypeId,
    #[serde(rename = "typeVersion")]
    type_version: u32,
    #[serde(default)]
    params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    ui: NodeUi,
}

#[derive(Serialize)]
struct SerialDocument {
    format: String,
    version: u32,
    name: String,
    description: String,
    nodes: Vec<serde_json::Value>,
    links: Vec<Link>,
    outputs: Vec<OutputDecl>,
}

impl GraphDocument {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format: FORMAT_ID.to_owned(),
            version: CURRENT_DOCUMENT_VERSION,
            name: name.into(),
            description: String::new(),
            nodes: Vec::new(),
            links: Vec::new(),
            outputs: Vec::new(),
        }
    }

    pub fn node(&self, id: &NodeId) -> Option<&NodeData> {
        self.nodes.iter().find(|n| &n.id == id)
    }

    pub fn node_mut(&mut self, id: &NodeId) -> Option<&mut NodeData> {
        self.nodes.iter_mut().find(|n| &n.id == id)
    }

    pub fn links_into<'a>(&'a self, id: &'a NodeId) -> impl Iterator<Item = &'a Link> {
        self.links.iter().filter(move |l| &l.to.node == id)
    }

    pub fn links_from<'a>(&'a self, id: &'a NodeId) -> impl Iterator<Item = &'a Link> {
        self.links.iter().filter(move |l| &l.from.node == id)
    }

    /// Infers `(input_sockets, output_sockets)` on a placeholder node from links touching it.
    pub fn inferred_placeholder_sockets(&self, id: &NodeId) -> (BTreeSet<SocketId>, BTreeSet<SocketId>) {
        let inputs = self
            .links_into(id)
            .map(|l| l.to.socket.clone())
            .collect();
        let outputs = self
            .links_from(id)
            .map(|l| l.from.socket.clone())
            .collect();
        (inputs, outputs)
    }

    /// Parses a `.dawsugraph` JSON string, running per-node migrations and preserving unknown or
    /// newer-version nodes as disabled placeholders (Invariant 7 / R8).
    pub fn from_json(json: &str, registry: &NodeRegistry) -> Result<Self, DocumentError> {
        let raw: RawDocument =
            serde_json::from_str(json).map_err(|e| DocumentError::InvalidJson(e.to_string()))?;
        if raw.format != FORMAT_ID {
            return Err(DocumentError::InvalidFormat(raw.format));
        }
        if raw.version != CURRENT_DOCUMENT_VERSION {
            return Err(DocumentError::UnsupportedVersion(raw.version));
        }
        let mut nodes = Vec::with_capacity(raw.nodes.len());
        for raw_node in raw.nodes {
            let header = parse_node_header(&raw_node)?;
            if let Some(node_type) = registry.get(&header.type_id) {
                let spec = node_type.spec();
                if header.type_version > spec.type_version {
                    nodes.push(NodeData {
                        id: header.id,
                        type_id: header.type_id,
                        type_version: header.type_version,
                        params: header.params,
                        muted: header.muted,
                        ui: header.ui,
                        placeholder: Some(PlaceholderInfo {
                            reason: PlaceholderReason::NewerVersion {
                                found: header.type_version,
                                known: spec.type_version,
                            },
                            raw: raw_node,
                        }),
                    });
                } else {
                    let parsed: SerialNode = serde_json::from_value(raw_node.clone())
                        .map_err(|e| DocumentError::InvalidJson(e.to_string()))?;
                    let mut params = parsed.params;
                    let mut version = parsed.type_version;
                    while version < spec.type_version {
                        params = node_type.migrate_params(version, params).map_err(|message| {
                            DocumentError::MigrationFailed {
                                node: parsed.id.clone(),
                                message,
                            }
                        })?;
                        version += 1;
                    }
                    nodes.push(NodeData {
                        id: parsed.id,
                        type_id: parsed.type_id,
                        type_version: version,
                        params,
                        muted: parsed.muted,
                        ui: parsed.ui,
                        placeholder: None,
                    });
                }
            } else {
                nodes.push(NodeData {
                    id: header.id,
                    type_id: header.type_id,
                    type_version: header.type_version,
                    params: header.params,
                    muted: header.muted,
                    ui: header.ui,
                    placeholder: Some(PlaceholderInfo {
                        reason: PlaceholderReason::UnknownType,
                        raw: raw_node,
                    }),
                });
            }
        }
        Ok(Self {
            format: raw.format,
            version: raw.version,
            name: raw.name,
            description: raw.description,
            nodes,
            links: raw.links,
            outputs: raw.outputs,
        })
    }

    /// Serializes the document as UTF-8 LF JSON, keeping placeholder nodes byte-preservingly intact.
    pub fn to_json(&self) -> Result<String, DocumentError> {
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if let Some(placeholder) = &node.placeholder {
                nodes.push(placeholder.raw.clone());
            } else {
                let serial = SerialNode {
                    id: node.id.clone(),
                    type_id: node.type_id.clone(),
                    type_version: node.type_version,
                    params: node.params.clone(),
                    muted: node.muted,
                    ui: node.ui,
                };
                nodes.push(
                    serde_json::to_value(serial)
                        .map_err(|e| DocumentError::InvalidJson(e.to_string()))?,
                );
            }
        }
        let doc = SerialDocument {
            format: self.format.clone(),
            version: self.version,
            name: self.name.clone(),
            description: self.description.clone(),
            nodes,
            links: self.links.clone(),
            outputs: self.outputs.clone(),
        };
        let mut out = serde_json::to_string_pretty(&doc)
            .map_err(|e| DocumentError::InvalidJson(e.to_string()))?;
        out.push('\n');
        Ok(out)
    }

    /// Canonical hash of graph structure, parameters, mute states, links, and outputs.
    /// Excludes node `ui` coordinates so moving nodes on the canvas does not invalidate runs.
    pub fn structural_hash(&self) -> String {
        let mut sorted_nodes: Vec<&NodeData> = self.nodes.iter().collect();
        sorted_nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let canonical_nodes: Vec<serde_json::Value> = sorted_nodes
            .into_iter()
            .map(|n| {
                let mut params = BTreeMap::new();
                for (k, v) in &n.params {
                    params.insert(k.clone(), canonical_json_value(v));
                }
                serde_json::json!({
                    "id": n.id,
                    "type": n.type_id,
                    "typeVersion": n.type_version,
                    "muted": n.muted,
                    "placeholder": n.placeholder.is_some(),
                    "params": params,
                })
            })
            .collect();
        let mut sorted_links = self.links.clone();
        sorted_links.sort();
        let mut sorted_outputs = self.outputs.clone();
        sorted_outputs.sort();
        let canonical = serde_json::json!({
            "version": self.version,
            "nodes": canonical_nodes,
            "links": sorted_links,
            "outputs": sorted_outputs,
        });
        let text = serde_json::to_string(&canonical).unwrap_or_default();
        sha256_hex(text.as_bytes())
    }
}

fn parse_node_header(raw_node: &serde_json::Value) -> Result<SerialNode, DocumentError> {
    let obj = raw_node
        .as_object()
        .ok_or_else(|| DocumentError::InvalidJson("node entry must be an object".to_owned()))?;
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DocumentError::InvalidJson("node missing string 'id'".to_owned()))?;
    let type_id = obj
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DocumentError::InvalidJson("node missing string 'type'".to_owned()))?;
    let type_version = obj
        .get("typeVersion")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or(1);
    let params = obj
        .get("params")
        .and_then(|v| v.as_object())
        .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let muted = obj.get("muted").and_then(|v| v.as_bool()).unwrap_or(false);
    let ui = obj
        .get("ui")
        .and_then(|v| serde_json::from_value::<NodeUi>(v.clone()).ok())
        .unwrap_or_default();
    Ok(SerialNode {
        id: NodeId::from(id),
        type_id: NodeTypeId::from(type_id),
        type_version,
        params,
        muted,
        ui,
    })
}
