//! Node graph editor session, background evaluation, viewer preview downsampling, and batch apply.

use crate::{Application, project};
use dawsu_audio::AudioClip;
use dawsu_editor_nodal::{
    AnalysisStamp, ApplyError, ApplyOptions, GenerationBatchRecord, NodalSession, ProposalBatch,
    SlotBindings, apply_proposals, select_batch_hits,
};
use dawsu_foundation::SampleIdentity;
use dawsu_graph::{
    AnalysisRange, Curve, DiagnosticSeverity, DocumentError, Endpoint, EvalCache, EvalError,
    EvalTarget, GraphDocument, GraphEvalResult, Link, NodeId, NodeOutputSummary, NodeRegistry,
    NodeSpec, NodeTypeId, ParamSpec, SocketValue, TimingGrid, TimingLine, ValidationIssue,
    evaluate_graph_with_song_range,
};
use dawsu_graph_audio::{
    AudioProvider, DecodedAudio, StaticProviders, TimingProvider, classic_kick_snare_graph,
    drum_stem_graph, register_audio_nodes, simple_hihat_graph, simple_kick_graph,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

const MAX_VIEWER_POINTS: usize = 2048;

fn norm_bits(value: f64) -> u64 {
    if value == 0.0 {
        0.0_f64.to_bits()
    } else {
        value.to_bits()
    }
}

fn deinterleave_clip(clip: &AudioClip, offset_ms: f64) -> DecodedAudio {
    let ch_count = clip.channels.max(1);
    let frames = clip.samples.len() / ch_count;
    let mut channels = vec![Vec::with_capacity(frames); ch_count];
    for frame in clip.samples.chunks_exact(ch_count) {
        for (c, &sample) in frame.iter().enumerate() {
            channels[c].push(sample);
        }
    }
    DecodedAudio::from_planar(channels, clip.sample_rate, offset_ms)
}

/// Application-backed audio provider for background graph evaluation.
struct AppAudioProvider {
    song: Option<Arc<DecodedAudio>>,
    map_folder: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    stems: Mutex<HashMap<String, Result<Arc<DecodedAudio>, String>>>,
}

impl AppAudioProvider {
    fn new(
        song: Option<Arc<DecodedAudio>>,
        map_folder: Option<PathBuf>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            song,
            map_folder,
            cancel,
            stems: Mutex::new(HashMap::new()),
        }
    }

    fn resolve_path(&self, raw: &str) -> PathBuf {
        let p = Path::new(raw);
        if p.is_absolute() {
            p.to_path_buf()
        } else if let Some(folder) = &self.map_folder {
            folder.join(p)
        } else {
            p.to_path_buf()
        }
    }
}

impl AudioProvider for AppAudioProvider {
    fn song_audio(&self) -> Result<Arc<DecodedAudio>, String> {
        self.song
            .clone()
            .ok_or_else(|| "This difficulty has no song audio loaded.".to_owned())
    }

    fn file_audio(&self, path: &str) -> Result<Arc<DecodedAudio>, String> {
        if path.trim().is_empty() {
            return Err("No audio file path specified.".to_owned());
        }
        {
            let cache = self.stems.lock().map_err(|_| "Audio cache unavailable.".to_owned())?;
            if let Some(cached) = cache.get(path) {
                return cached.clone();
            }
        }
        let full = self.resolve_path(path);
        let cancel = self.cancel.clone();
        let stop = move || cancel.load(Ordering::Relaxed);
        let decoded = dawsu_audio::decode_with_cancel(&full, &stop)
            .map(|clip| Arc::new(deinterleave_clip(&clip, 0.0)))
            .map_err(|err| format!("Could not decode '{}': {err}", full.display()));
        if let Ok(mut cache) = self.stems.lock() {
            cache.insert(path.to_owned(), decoded.clone());
        }
        decoded
    }
}

/// Application-backed timing grid built from the active map's tempo sections.
struct AppTimingProvider {
    grid: Arc<TimingGrid>,
}

impl AppTimingProvider {
    fn from_lines(lines: Vec<TimingLine>) -> Self {
        Self {
            grid: Arc::new(TimingGrid::new(lines)),
        }
    }
}

impl TimingProvider for AppTimingProvider {
    fn timing_grid(&self) -> Arc<TimingGrid> {
        self.grid.clone()
    }
}

struct GraphEvalDone {
    result: GraphEvalResult,
    viewer_value: Option<SocketValue>,
    updated_cache: EvalCache,
    stamp: AnalysisStamp,
    song_stamp: u64,
    run: u64,
    song_duration_ms: f64,
}

enum GraphOutcome {
    Done(Box<GraphEvalDone>),
    Cancelled,
    Failed(String),
}

struct GraphJob {
    receiver: mpsc::Receiver<GraphOutcome>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<(usize, usize)>>,
    song_stamp: u64,
    map_revision: u64,
    song_offset_bits: u64,
}

struct ReadyBatch {
    eval: GraphEvalResult,
    batch: ProposalBatch,
    slot_nodes: BTreeMap<String, NodeId>,
    song_stamp: u64,
    song_duration_ms: f64,
    viewer: Option<ViewerOverlay>,
    song_hash: String,
}

pub(crate) struct NodeGraphState {
    registry: Arc<NodeRegistry>,
    session: NodalSession,
    cache: EvalCache,
    bindings: BTreeMap<String, String>,
    job: Option<GraphJob>,
    ready: Option<ReadyBatch>,
    last_applied: Option<GenerationBatchRecord>,
    use_locator_range: bool,
    apply_options: ApplyOptions,
    viewer_socket: Option<Endpoint>,
    song_stamp: u64,
    runs: u64,
    error: Option<String>,
    cached_song: Option<(u64, u64, Arc<DecodedAudio>)>,
}

impl Default for NodeGraphState {
    fn default() -> Self {
        let mut registry = NodeRegistry::new();
        let providers = Arc::new(StaticProviders::default());
        register_audio_nodes(&mut registry, providers.clone(), providers)
            .expect("built-in audio nodes must register without duplicates");
        let registry = Arc::new(registry);
        let preset = classic_kick_snare_graph(&registry)
            .expect("built-in classic preset must parse");
        Self {
            registry,
            session: NodalSession::new(preset),
            cache: EvalCache::default(),
            bindings: BTreeMap::new(),
            job: None,
            ready: None,
            last_applied: None,
            use_locator_range: false,
            apply_options: ApplyOptions::default(),
            viewer_socket: None,
            song_stamp: 0,
            runs: 0,
            error: None,
            cached_song: None,
        }
    }
}

impl NodeGraphState {
    fn cancel_job(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn song_loaded(&mut self) {
        self.song_stamp += 1;
        self.cancel_job();
        self.ready = None;
        self.last_applied = None;
        self.error = None;
        self.cached_song = None;
        self.bindings.clear();
        self.cache.clear();
    }
}

/// Downsampled preview payload rendered over the timeline waveform.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ViewerOverlay {
    Curve {
        #[serde(rename = "startMs")]
        start_ms: f64,
        #[serde(rename = "hopMs")]
        hop_ms: f64,
        #[serde(rename = "sampleStep")]
        sample_step: usize,
        min: f64,
        max: f64,
        values: Vec<f64>,
    },
    Events {
        #[serde(rename = "timesMs")]
        times_ms: Vec<f64>,
        strengths: Vec<f64>,
    },
    Scalar {
        label: String,
    },
    Empty,
}

impl ViewerOverlay {
    fn from_socket_value(value: &SocketValue) -> Self {
        match value {
            SocketValue::Curve(curve) => Self::from_curve(curve),
            SocketValue::Events(list) => Self::Events {
                times_ms: list.events.iter().map(|e| e.time_ms).collect(),
                strengths: list.events.iter().map(|e| e.strength).collect(),
            },
            SocketValue::Number(v) => Self::Scalar {
                label: format!("number: {v:.4}"),
            },
            SocketValue::Integer(v) => Self::Scalar {
                label: format!("integer: {v}"),
            },
            SocketValue::Bool(v) => Self::Scalar {
                label: format!("bool: {v}"),
            },
            SocketValue::Enum(v) => Self::Scalar {
                label: format!("enum: {v}"),
            },
            SocketValue::Range(r) => Self::Scalar {
                label: format!("range: {:.0}..{:.0} ms", r.from_ms, r.to_ms),
            },
            SocketValue::Audio(a) => Self::Scalar {
                label: format!("audio: {} ch, {} frames @ {} Hz", a.channels.len(), a.frame_count(), a.sample_rate),
            },
            SocketValue::Spectrogram(s) => Self::Scalar {
                label: format!("spectrogram: {}x{}", s.frames, s.bins),
            },
            SocketValue::Timing(t) => Self::Scalar {
                label: format!("timing: {} red lines", t.lines().len()),
            },
        }
    }

    fn from_curve(curve: &Curve) -> Self {
        if curve.values.is_empty() {
            return Self::Empty;
        }
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &v in &curve.values {
            if v.is_finite() {
                min = min.min(v);
                max = max.max(v);
            }
        }
        if !min.is_finite() || !max.is_finite() {
            return Self::Empty;
        }
        let step = curve.values.len().div_ceil(MAX_VIEWER_POINTS).max(1);
        let mut downsampled = Vec::with_capacity(curve.values.len().div_ceil(step));
        for chunk in curve.values.chunks(step) {
            let peak = chunk
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .fold(0.0_f64, |acc, v| if v.abs() > acc.abs() { v } else { acc });
            downsampled.push(peak);
        }
        Self::Curve {
            start_ms: curve.start_ms,
            hop_ms: curve.hop_ms,
            sample_step: step,
            min,
            max,
            values: downsampled,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SocketSpecView {
    pub id: String,
    pub socket_type: String,
    pub any: bool,
    pub required: bool,
    pub multi: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpecView {
    pub id: String,
    pub socket_type: String,
    pub default: serde_json::Value,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub enum_values: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticView {
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeSummaryView {
    Events { count: usize },
    Curve { sparkline: Vec<f32>, min: f64, max: f64 },
    Audio { #[serde(rename = "durationMs")] duration_ms: f64, channels: usize },
    Spectrogram { frames: usize, bins: usize },
    Timing { lines: usize },
    Range { #[serde(rename = "fromMs")] from_ms: f64, #[serde(rename = "toMs")] to_ms: f64 },
    Scalar { label: String },
}

impl NodeSummaryView {
    fn from_summary(summary: &NodeOutputSummary) -> Self {
        match summary {
            NodeOutputSummary::Events { count } => Self::Events { count: *count },
            NodeOutputSummary::Curve { sparkline, min, max } => Self::Curve {
                sparkline: sparkline.clone(),
                min: *min,
                max: *max,
            },
            NodeOutputSummary::Audio { duration_ms, channels } => Self::Audio {
                duration_ms: *duration_ms,
                channels: *channels,
            },
            NodeOutputSummary::Spectrogram { frames, bins } => Self::Spectrogram {
                frames: *frames,
                bins: *bins,
            },
            NodeOutputSummary::Timing { lines } => Self::Timing { lines: *lines },
            NodeOutputSummary::Range { from_ms, to_ms } => Self::Range {
                from_ms: *from_ms,
                to_ms: *to_ms,
            },
            NodeOutputSummary::Scalar { label } => Self::Scalar { label: label.clone() },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeView {
    pub id: String,
    pub type_id: String,
    pub type_version: u32,
    pub x: f64,
    pub y: f64,
    pub muted: bool,
    pub placeholder: Option<String>,
    pub dirty: bool,
    pub params: BTreeMap<String, serde_json::Value>,
    pub inputs: Vec<SocketSpecView>,
    pub outputs: Vec<SocketSpecView>,
    pub param_specs: Vec<ParamSpecView>,
    pub diagnostics: Vec<DiagnosticView>,
    pub summary: Option<NodeSummaryView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    pub from_node: String,
    pub from_socket: String,
    pub to_node: String,
    pub to_socket: String,
    pub socket_type: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotView {
    pub slot: String,
    pub bound_lane_key: Option<String>,
    pub bound_lane_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaneChoiceView {
    pub key: String,
    pub name: String,
    pub hidden: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeTypeView {
    pub type_id: String,
    pub category: String,
    pub label: String,
    pub type_version: u32,
    pub inputs: Vec<SocketSpecView>,
    pub outputs: Vec<SocketSpecView>,
    pub params: Vec<ParamSpecView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyOptionsView {
    pub tolerance_ms: f64,
    pub dedupe_ms: f64,
    pub include_unmatched: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalView {
    pub id: String,
    pub slot: String,
    pub lane_key: String,
    pub lane_name: String,
    pub time_ms: f64,
    pub strength: f64,
    pub checked: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchCountsView {
    pub total: usize,
    pub checked: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchView {
    pub batch_id: String,
    pub graph_name: String,
    pub stale: bool,
    pub stale_reason: Option<String>,
    pub counts: BatchCountsView,
    pub proposals: Vec<ProposalView>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointView {
    pub node: String,
    pub socket: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeGraphView {
    pub status: String,
    pub done: usize,
    pub total: usize,
    pub error: Option<String>,
    pub has_document: bool,
    pub has_song: bool,
    pub map_title: String,
    pub map_version: String,
    pub song_duration_ms: f64,
    pub has_locator_range: bool,
    pub use_locator_range: bool,
    pub range_from_ms: f64,
    pub range_to_ms: f64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub revision: u64,
    pub graph_name: String,
    pub graph_description: String,
    pub graph_hash: String,
    pub selected_nodes: Vec<String>,
    pub nodes: Vec<NodeView>,
    pub links: Vec<LinkView>,
    pub slots: Vec<SlotView>,
    pub lanes: Vec<LaneChoiceView>,
    pub catalog: Vec<NodeTypeView>,
    pub validation_issues: Vec<String>,
    pub apply_options: ApplyOptionsView,
    pub batch: Option<BatchView>,
    pub can_select_last_hits: bool,
    pub viewer_socket: Option<EndpointView>,
    pub viewer: Option<ViewerOverlay>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePositionInput {
    pub id: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeGraphAction {
    AddNode {
        #[serde(rename = "typeId")]
        type_id: String,
        x: f64,
        y: f64,
    },
    RemoveNodes {
        ids: Vec<String>,
    },
    DuplicateSelected {
        #[serde(rename = "offsetX")]
        offset_x: f64,
        #[serde(rename = "offsetY")]
        offset_y: f64,
    },
    SelectNodes {
        ids: Vec<String>,
    },
    MoveNodes {
        positions: Vec<NodePositionInput>,
    },
    Connect {
        #[serde(rename = "fromNode")]
        from_node: String,
        #[serde(rename = "fromSocket")]
        from_socket: String,
        #[serde(rename = "toNode")]
        to_node: String,
        #[serde(rename = "toSocket")]
        to_socket: String,
    },
    Disconnect {
        #[serde(rename = "fromNode")]
        from_node: String,
        #[serde(rename = "fromSocket")]
        from_socket: String,
        #[serde(rename = "toNode")]
        to_node: String,
        #[serde(rename = "toSocket")]
        to_socket: String,
    },
    SetParam {
        #[serde(rename = "nodeId")]
        node_id: String,
        param: String,
        value: serde_json::Value,
    },
    SetMuted {
        #[serde(rename = "nodeId")]
        node_id: String,
        muted: bool,
    },
    Undo,
    Redo,
    LoadPreset {
        name: String,
    },
    ImportJson {
        json: String,
    },
    SetBinding {
        slot: String,
        #[serde(rename = "laneKey")]
        lane_key: Option<String>,
    },
    SetRangeMode {
        #[serde(rename = "useLocatorRange")]
        use_locator_range: bool,
    },
    SetApplyOptions {
        #[serde(rename = "toleranceMs")]
        tolerance_ms: f64,
        #[serde(rename = "dedupeMs")]
        dedupe_ms: f64,
        #[serde(rename = "includeUnmatched")]
        include_unmatched: bool,
    },
    SetViewerTarget {
        node: Option<String>,
        socket: Option<String>,
    },
    Run,
    Cancel,
    ToggleProposal {
        id: String,
        checked: bool,
    },
    SetAllProposals {
        checked: bool,
    },
    Apply,
    SelectLastHits,
}

fn slider_body(lane: &SampleIdentity) -> bool {
    matches!(
        lane,
        SampleIdentity::Standard {
            sound: dawsu_foundation::Sound::Slider(_),
            ..
        }
    )
}

fn socket_view(spec: &dawsu_graph::SocketSpec) -> SocketSpecView {
    let (socket_type, any) = match spec.kind {
        dawsu_graph::SocketConstraint::Exact(t) => (t.to_string(), false),
        dawsu_graph::SocketConstraint::Any => ("any".to_owned(), true),
    };
    SocketSpecView {
        id: spec.id.0.clone(),
        socket_type,
        any,
        required: spec.required,
        multi: spec.multi,
    }
}

fn param_view(spec: &ParamSpec) -> ParamSpecView {
    ParamSpecView {
        id: spec.id.0.clone(),
        socket_type: spec.socket_type.to_string(),
        default: spec.default.clone(),
        min: spec.min,
        max: spec.max,
        enum_values: spec.enum_values.clone(),
    }
}

fn catalog_entry(spec: &NodeSpec) -> NodeTypeView {
    let id = spec.id.as_str();
    let (category, label) = id.split_once('.').unwrap_or(("other", id));
    NodeTypeView {
        type_id: id.to_owned(),
        category: category.to_owned(),
        label: label.to_owned(),
        type_version: spec.type_version,
        inputs: spec.inputs.iter().map(socket_view).collect(),
        outputs: spec.outputs.iter().map(socket_view).collect(),
        params: spec.params.iter().map(param_view).collect(),
    }
}

fn format_validation_issue(issue: &ValidationIssue) -> String {
    match issue {
        ValidationIssue::DuplicateNodeId(id) => format!("Duplicate node id '{id}'"),
        ValidationIssue::PlaceholderNode { node, type_id } => {
            format!("Placeholder node '{node}' ('{type_id}') is read-only")
        }
        ValidationIssue::InvalidLink { link, error } => {
            format!("Invalid link {}.{} -> {}.{}: {error}", link.from.node, link.from.socket, link.to.node, link.to.socket)
        }
        ValidationIssue::MissingRequiredInput { node, socket } => {
            format!("Node '{node}' missing required input '{socket}'")
        }
        ValidationIssue::CycleDetected(nodes) => {
            let names: Vec<&str> = nodes.iter().map(NodeId::as_str).collect();
            format!("Cycle detected: {}", names.join(" -> "))
        }
        ValidationIssue::InvalidParam { node, message } => {
            format!("Node '{node}' parameter error: {message}")
        }
    }
}

fn severity_label(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Info => "info",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Error => "error",
    }
}

impl Application {
    pub(crate) fn clear_node_graph_run(&mut self) {
        self.node_graph.cancel_job();
    }

    pub fn cancel_node_graph_and_clear_viewer(&mut self) {
        self.node_graph.cancel_job();
        self.node_graph.viewer_socket = None;
        if let Some(ready) = &mut self.node_graph.ready {
            ready.viewer = None;
        }
    }

    pub(crate) fn node_graph_song_loaded(&mut self) {
        self.node_graph.song_loaded();
        if let Some(doc) = &self.document
            && let Some(saved) = &doc.node_graph_bindings
        {
            self.node_graph.bindings = saved.clone();
        }
    }

    fn decoded_song_for_graph(&mut self) -> Option<Arc<DecodedAudio>> {
        let clip = self.song.as_ref()?;
        let offset_bits = norm_bits(self.song_offset);
        if let Some((stamp, bits, decoded)) = &self.node_graph.cached_song
            && *stamp == self.node_graph.song_stamp
            && *bits == offset_bits
        {
            return Some(decoded.clone());
        }
        let decoded = Arc::new(deinterleave_clip(clip, self.song_offset));
        self.node_graph.cached_song = Some((self.node_graph.song_stamp, offset_bits, decoded.clone()));
        Some(decoded)
    }

    fn graph_analysis_range(&self, song_duration_ms: f64) -> Result<AnalysisRange, String> {
        let offset_end = song_duration_ms + self.song_offset.max(0.0);
        let ceiling = offset_end.max(song_duration_ms).max(self.duration_ms()).max(1.0);
        if self.node_graph.use_locator_range && self.locator_right > self.locator_left {
            AnalysisRange::new(self.locator_left.max(0.0), self.locator_right)
                .map_err(|e| format!("Invalid locator range: {e}"))
        } else {
            AnalysisRange::new(0.0, ceiling).map_err(|e| format!("Invalid song range: {e}"))
        }
    }

    fn current_slot_bindings(&self) -> SlotBindings {
        let Some(document) = &self.document else {
            return SlotBindings::new();
        };
        let mut out = SlotBindings::new();
        for (slot, key) in &self.node_graph.bindings {
            if let Some(identity) = project::lane_identity(key)
                && !slider_body(&identity)
                && document.composer.hitsounds().lane(&identity).is_some()
            {
                out.insert(slot.clone(), identity);
            }
        }
        out
    }

    fn slot_nodes_in_doc(doc: &GraphDocument, registry: &NodeRegistry) -> BTreeMap<String, NodeId> {
        let mut out = BTreeMap::new();
        for node in &doc.nodes {
            if node.type_id.as_str() == "output.lane"
                && let Some(spec) = registry.get(&node.type_id)
            {
                let slot: String = node
                    .params
                    .get("slot")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        spec.spec()
                            .params
                            .iter()
                            .find(|p| p.id.as_str() == "slot")
                            .and_then(|p| p.default.as_str())
                            .unwrap_or("default")
                            .to_owned()
                    });
                if !slot.trim().is_empty() {
                    out.entry(slot).or_insert_with(|| node.id.clone());
                }
            }
        }
        out
    }

    fn current_stamp_for_ready(&self, ready: &ReadyBatch) -> Option<AnalysisStamp> {
        let document = self.document.as_ref()?;
        let map_rev = document.composer.history.version();
        let current_range = self
            .graph_analysis_range(ready.song_duration_ms)
            .unwrap_or(ready.batch.range);
        Some(AnalysisStamp::new(
            map_rev,
            self.song_offset,
            &ready.song_hash,
            self.node_graph.session.document().structural_hash(),
            current_range,
        ))
    }

    fn rebuild_ready_proposals(&mut self, eval: &GraphEvalResult) {
        let Some(ready) = &self.node_graph.ready else {
            return;
        };
        let bindings = self.current_slot_bindings();
        let batch = ProposalBatch::from_eval_result(
            ready.batch.batch_id.clone(),
            self.node_graph.session.document().name.clone(),
            eval,
            &bindings,
            &ready.slot_nodes,
            ready.batch.stamp.clone(),
        );
        if let Some(r) = &mut self.node_graph.ready {
            r.batch = batch;
        }
    }

    /// Collects finished background graph evaluation runs without blocking the editor thread.
    pub fn poll_node_graph(&mut self) {
        if self.document.is_none() {
            if self.node_graph.job.is_some() || self.node_graph.ready.is_some() {
                self.node_graph.cancel_job();
                self.node_graph.ready = None;
                self.node_graph.last_applied = None;
            }
            return;
        }
        let current_rev = self.document.as_ref().map(|d| d.composer.history.version());
        let current_offset_bits = norm_bits(self.song_offset);
        let outcome = {
            let Some(job) = &self.node_graph.job else {
                return;
            };
            if job.song_stamp != self.node_graph.song_stamp
                || current_rev != Some(job.map_revision)
                || current_offset_bits != job.song_offset_bits
            {
                self.node_graph.cancel_job();
                return;
            }
            match job.receiver.try_recv() {
                Ok(outcome) => outcome,
                Err(mpsc::TryRecvError::Disconnected) => {
                    GraphOutcome::Failed("Node graph worker stopped unexpectedly.".to_owned())
                }
                Err(mpsc::TryRecvError::Empty) => return,
            }
        };
        let _ = self.node_graph.job.take();
        match outcome {
            GraphOutcome::Cancelled => {}
            GraphOutcome::Failed(msg) => {
                self.node_graph.error = Some(msg);
            }
            GraphOutcome::Done(done) => {
                let done = *done;
                if done.song_stamp != self.node_graph.song_stamp
                    || done.run != self.node_graph.runs
                    || current_rev != Some(done.stamp.map_revision)
                    || current_offset_bits != done.stamp.song_offset_bits
                {
                    return;
                }
                // Invariant 5c / R6: commit updated cache ONLY when job finishes cleanly and is current.
                self.node_graph.cache = done.updated_cache;
                self.node_graph.error = None;
                let bindings = self.current_slot_bindings();
                let slot_nodes =
                    Self::slot_nodes_in_doc(self.node_graph.session.document(), &self.node_graph.registry);
                let batch = ProposalBatch::from_eval_result(
                    format!("graph-run-{}", done.run),
                    self.node_graph.session.document().name.clone(),
                    &done.result,
                    &bindings,
                    &slot_nodes,
                    done.stamp.clone(),
                );
                let viewer = done
                    .viewer_value
                    .as_ref()
                    .map(ViewerOverlay::from_socket_value);
                let song_hash = done.stamp.source_audio_sha256;
                self.node_graph.ready = Some(ReadyBatch {
                    eval: done.result,
                    batch,
                    slot_nodes,
                    song_stamp: done.song_stamp,
                    song_duration_ms: done.song_duration_ms,
                    viewer,
                    song_hash,
                });
            }
        }
    }

    pub fn node_graph_export_json(&self) -> Result<String, String> {
        self.node_graph
            .session
            .document()
            .to_json()
            .map_err(|e| format!("Export failed: {e}"))
    }

    pub fn node_graph_view(&self) -> NodeGraphView {
        let doc = self.node_graph.session.document();
        let registry = &self.node_graph.registry;
        let report = self.node_graph.session.validate(registry);
        let selected: BTreeSet<&NodeId> = self.node_graph.session.selection().items().iter().collect();
        let dirty = self.node_graph.session.dirty_nodes();
        let batch_diags = self.node_graph.ready.as_ref().map(|r| &r.batch.diagnostics);
        let node_summaries = self.node_graph.ready.as_ref().map(|r| &r.eval.node_summaries);

        let nodes: Vec<NodeView> = doc
            .nodes
            .iter()
            .map(|n| {
                let spec_opt = registry.get(&n.type_id).map(|t| t.spec());
                let (inputs, outputs, param_specs) = match &spec_opt {
                    Some(spec) => (
                        spec.inputs.iter().map(socket_view).collect(),
                        spec.outputs.iter().map(socket_view).collect(),
                        spec.params.iter().map(param_view).collect(),
                    ),
                    None => {
                        let (in_socks, out_socks) = doc.inferred_placeholder_sockets(&n.id);
                        let map_unknown = |s: dawsu_graph::SocketId| SocketSpecView {
                            id: s.0,
                            socket_type: "unknown".to_owned(),
                            any: false,
                            required: false,
                            multi: false,
                        };
                        (
                            in_socks.into_iter().map(map_unknown).collect(),
                            out_socks.into_iter().map(map_unknown).collect(),
                            Vec::new(),
                        )
                    }
                };
                let diagnostics = batch_diags
                    .and_then(|m| m.get(&n.id))
                    .map(|ds| {
                        ds.iter()
                            .map(|d| DiagnosticView {
                                severity: severity_label(d.severity).to_owned(),
                                message: d.message.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let placeholder = n.placeholder.as_ref().map(|p| match &p.reason {
                    dawsu_graph::PlaceholderReason::UnknownType => {
                        format!("Unknown node type '{}'", n.type_id)
                    }
                    dawsu_graph::PlaceholderReason::NewerVersion { found, known } => {
                        format!("Newer version v{found} (editor knows v{known})")
                    }
                });
                NodeView {
                    id: n.id.0.clone(),
                    type_id: n.type_id.0.clone(),
                    type_version: n.type_version,
                    x: n.ui.x,
                    y: n.ui.y,
                    muted: n.muted,
                    placeholder,
                    dirty: dirty.contains(&n.id),
                    params: n.params.clone(),
                    inputs,
                    outputs,
                    param_specs,
                    diagnostics,
                    summary: node_summaries
                        .and_then(|m| m.get(&n.id))
                        .map(NodeSummaryView::from_summary),
                }
            })
            .collect();

        let links: Vec<LinkView> = doc
            .links
            .iter()
            .map(|l| {
                let socket_type = doc
                    .node(&l.from.node)
                    .and_then(|n| registry.get(&n.type_id))
                    .and_then(|t| t.spec().output(&l.from.socket).cloned())
                    .map(|s| match s.kind {
                        dawsu_graph::SocketConstraint::Exact(st) => st.to_string(),
                        dawsu_graph::SocketConstraint::Any => "any".to_owned(),
                    })
                    .unwrap_or_else(|| "unknown".to_owned());
                LinkView {
                    from_node: l.from.node.0.clone(),
                    from_socket: l.from.socket.0.clone(),
                    to_node: l.to.node.0.clone(),
                    to_socket: l.to.socket.0.clone(),
                    socket_type,
                }
            })
            .collect();

        let slot_nodes = Self::slot_nodes_in_doc(doc, registry);
        let slots: Vec<SlotView> = slot_nodes
            .keys()
            .map(|slot| {
                let bound_lane_key = self.node_graph.bindings.get(slot).cloned();
                let bound_lane_name = bound_lane_key
                    .as_deref()
                    .and_then(project::lane_identity)
                    .map(|id| crate::lane_name(&id));
                SlotView {
                    slot: slot.clone(),
                    bound_lane_key,
                    bound_lane_name,
                }
            })
            .collect();

        let lanes: Vec<LaneChoiceView> = self
            .document
            .as_ref()
            .map(|d| {
                d.composer
                    .hitsounds()
                    .lanes()
                    .iter()
                    .filter(|l| !slider_body(&l.identity))
                    .map(|l| LaneChoiceView {
                        key: project::lane_key(&l.identity),
                        name: crate::lane_name(&l.identity),
                        hidden: d.composer.hitsounds().is_lane_hidden(&l.identity),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let catalog: Vec<NodeTypeView> = registry
            .specs()
            .iter()
            .map(catalog_entry)
            .collect();

        let (status, done, total) = match (&self.node_graph.job, &self.node_graph.ready) {
            (Some(job), _) => {
                let (d, t) = job.progress.lock().map(|p| *p).unwrap_or((0, 1));
                ("running".to_owned(), d, t)
            }
            (None, Some(_)) => ("ready".to_owned(), 1, 1),
            (None, None) => ("idle".to_owned(), 0, 1),
        };

        let song_duration_ms = self.duration_ms();
        let (range_from_ms, range_to_ms) = self
            .graph_analysis_range(song_duration_ms)
            .map_or((0.0, song_duration_ms), |r| (r.from_ms, r.to_ms));

        let batch = self.node_graph.ready.as_ref().map(|ready| {
            let stale_reason = self.current_stamp_for_ready(ready).and_then(|cur| {
                ready.batch.stamp.differs_from(&cur).map(|r| format!("{r:?}"))
            });
            let total_props = ready.batch.proposals.len();
            let checked_props = ready.batch.proposals.iter().filter(|p| p.checked).count();
            let proposals = ready
                .batch
                .proposals
                .iter()
                .map(|p| ProposalView {
                    id: p.id.clone(),
                    slot: p.slot.clone(),
                    lane_key: project::lane_key(&p.lane),
                    lane_name: crate::lane_name(&p.lane),
                    time_ms: p.time_ms,
                    strength: p.strength,
                    checked: p.checked,
                })
                .collect();
            BatchView {
                batch_id: ready.batch.batch_id.clone(),
                graph_name: ready.batch.graph_name.clone(),
                stale: stale_reason.is_some(),
                stale_reason,
                counts: BatchCountsView {
                    total: total_props,
                    checked: checked_props,
                },
                proposals,
            }
        });

        let viewer_socket = self.node_graph.viewer_socket.as_ref().map(|ep| EndpointView {
            node: ep.node.0.clone(),
            socket: ep.socket.0.clone(),
        });
        let viewer = self.node_graph.ready.as_ref().and_then(|r| r.viewer.clone());

        NodeGraphView {
            status,
            done,
            total,
            error: self.node_graph.error.clone(),
            has_document: self.document.is_some(),
            has_song: self.song.is_some(),
            map_title: self.document.as_ref().map(crate::Document::map_title).unwrap_or_default(),
            map_version: self.document.as_ref().map(crate::Document::version).unwrap_or_default(),
            song_duration_ms,
            has_locator_range: self.locator_right > self.locator_left,
            use_locator_range: self.node_graph.use_locator_range,
            range_from_ms,
            range_to_ms,
            can_undo: self.node_graph.session.can_undo(),
            can_redo: self.node_graph.session.can_redo(),
            revision: self.node_graph.session.revision(),
            graph_name: doc.name.clone(),
            graph_description: doc.description.clone(),
            graph_hash: doc.structural_hash(),
            selected_nodes: selected.into_iter().map(|id| id.0.clone()).collect(),
            nodes,
            links,
            slots,
            lanes,
            catalog,
            validation_issues: report.issues.iter().map(format_validation_issue).collect(),
            apply_options: ApplyOptionsView {
                tolerance_ms: self.node_graph.apply_options.tolerance_ms,
                dedupe_ms: self.node_graph.apply_options.dedupe_ms,
                include_unmatched: self.node_graph.apply_options.include_unmatched,
            },
            batch,
            can_select_last_hits: self.document.is_some() && self.node_graph.last_applied.is_some(),
            viewer_socket,
            viewer,
        }
    }

    pub fn node_graph_action(&mut self, action: NodeGraphAction) -> Result<(), String> {
        self.node_graph.error = None;
        match action {
            NodeGraphAction::AddNode { type_id, x, y } => {
                self.node_graph.cancel_job();
                let reg = self.node_graph.registry.clone();
                self.node_graph
                    .session
                    .add_node(&reg, &NodeTypeId::from(type_id.as_str()), x, y)
                    .map(|_| ())
                    .map_err(|e| e.to_string())?;
            }
            NodeGraphAction::RemoveNodes { ids } => {
                self.node_graph.cancel_job();
                let node_ids: Vec<NodeId> = ids.into_iter().map(NodeId::from).collect();
                self.node_graph
                    .session
                    .remove_nodes(&node_ids)
                    .map(|_| ())
                    .map_err(|e| e.to_string())?;
                if let Some(ep) = &self.node_graph.viewer_socket
                    && self.node_graph.session.document().node(&ep.node).is_none()
                {
                    self.node_graph.viewer_socket = None;
                }
            }
            NodeGraphAction::DuplicateSelected { offset_x, offset_y } => {
                self.node_graph.cancel_job();
                self.node_graph
                    .session
                    .duplicate_selected(offset_x, offset_y)
                    .map(|_| ())
                    .map_err(|e| e.to_string())?;
            }
            NodeGraphAction::SelectNodes { ids } => {
                let node_ids: Vec<NodeId> = ids.into_iter().map(NodeId::from).collect();
                self.node_graph.session.selection_mut().replace(node_ids);
            }
            NodeGraphAction::MoveNodes { positions } => {
                let mapped: Vec<(NodeId, f64, f64)> = positions
                    .into_iter()
                    .map(|p| (NodeId::from(p.id), p.x, p.y))
                    .collect();
                self.node_graph.session.begin_move();
                let res = self.node_graph.session.move_nodes(&mapped);
                if let Err(err) = res {
                    self.node_graph.session.cancel_move();
                    return Err(err.to_string());
                }
                self.node_graph.session.commit_move();
            }
            NodeGraphAction::Connect {
                from_node,
                from_socket,
                to_node,
                to_socket,
            } => {
                self.node_graph.cancel_job();
                let reg = self.node_graph.registry.clone();
                let link = Link::new(
                    Endpoint::new(from_node, from_socket),
                    Endpoint::new(to_node, to_socket),
                );
                self.node_graph
                    .session
                    .connect(&reg, link)
                    .map_err(|e| e.to_string())?;
            }
            NodeGraphAction::Disconnect {
                from_node,
                from_socket,
                to_node,
                to_socket,
            } => {
                self.node_graph.cancel_job();
                let link = Link::new(
                    Endpoint::new(from_node, from_socket),
                    Endpoint::new(to_node, to_socket),
                );
                self.node_graph
                    .session
                    .disconnect(&link)
                    .map(|_| ())
                    .map_err(|e| e.to_string())?;
            }
            NodeGraphAction::SetParam {
                node_id,
                param,
                value,
            } => {
                self.node_graph.cancel_job();
                self.node_graph
                    .session
                    .set_param(&NodeId::from(node_id.as_str()), &param, value)
                    .map_err(|e| e.to_string())?;
            }
            NodeGraphAction::SetMuted { node_id, muted } => {
                self.node_graph.cancel_job();
                self.node_graph
                    .session
                    .set_muted(&NodeId::from(node_id.as_str()), muted)
                    .map_err(|e| e.to_string())?;
            }
            NodeGraphAction::Undo => {
                self.node_graph.cancel_job();
                self.node_graph.session.undo();
            }
            NodeGraphAction::Redo => {
                self.node_graph.cancel_job();
                self.node_graph.session.redo();
            }
            NodeGraphAction::LoadPreset { name } => {
                self.node_graph.cancel_job();
                let doc = match name.as_str() {
                    "classic" => classic_kick_snare_graph(&self.node_graph.registry)
                        .map_err(|e| format!("Failed loading classic preset: {e}"))?,
                    "simple_kick" => simple_kick_graph(&self.node_graph.registry)
                        .map_err(|e| format!("Failed loading Simple Kick preset: {e}"))?,
                    "simple_hihat" => simple_hihat_graph(&self.node_graph.registry)
                        .map_err(|e| format!("Failed loading Simple Hi-Hat preset: {e}"))?,
                    "drum_stem" => drum_stem_graph(&self.node_graph.registry)
                        .map_err(|e| format!("Failed loading Drum Stem preset: {e}"))?,
                    "empty" => GraphDocument::new("Untitled"),
                    other => return Err(format!("Unknown preset '{other}'")),
                };
                self.node_graph.session = NodalSession::new(doc);
                self.node_graph.ready = None;
                self.node_graph.viewer_socket = None;
                self.node_graph.cache.clear();
            }
            NodeGraphAction::ImportJson { json } => {
                let doc = GraphDocument::from_json(&json, &self.node_graph.registry)
                    .map_err(|e: DocumentError| format!("Could not import graph: {e}"))?;
                self.node_graph.cancel_job();
                self.node_graph.session = NodalSession::new(doc);
                self.node_graph.ready = None;
                self.node_graph.viewer_socket = None;
                self.node_graph.cache.clear();
            }
            NodeGraphAction::SetBinding { slot, lane_key } => {
                match lane_key {
                    Some(key) if !key.is_empty() => {
                        let identity = project::lane_identity(&key)
                            .ok_or_else(|| "Invalid hitsound lane key.".to_owned())?;
                        if slider_body(&identity) {
                            return Err("Slider body lanes cannot receive node graph hits.".to_owned());
                        }
                        self.node_graph.bindings.insert(slot, key);
                    }
                    _ => {
                        self.node_graph.bindings.remove(&slot);
                    }
                }
                if let Some(doc) = &mut self.document {
                    doc.node_graph_bindings = (!self.node_graph.bindings.is_empty())
                        .then(|| self.node_graph.bindings.clone());
                }
                if let Some(eval) = self.node_graph.ready.as_ref().map(|r| r.eval.clone()) {
                    self.rebuild_ready_proposals(&eval);
                }
            }
            NodeGraphAction::SetRangeMode { use_locator_range } => {
                self.node_graph.use_locator_range = use_locator_range;
            }
            NodeGraphAction::SetApplyOptions {
                tolerance_ms,
                dedupe_ms,
                include_unmatched,
            } => {
                if !tolerance_ms.is_finite() || tolerance_ms < 0.0 || !dedupe_ms.is_finite() || dedupe_ms < 0.0 {
                    return Err("Tolerance and deduplication windows must be non-negative finite numbers.".to_owned());
                }
                self.node_graph.apply_options.tolerance_ms = tolerance_ms;
                self.node_graph.apply_options.dedupe_ms = dedupe_ms;
                self.node_graph.apply_options.include_unmatched = include_unmatched;
            }
            NodeGraphAction::SetViewerTarget { node, socket } => {
                self.node_graph.viewer_socket = match (node, socket) {
                    (Some(n), Some(s)) if !n.is_empty() && !s.is_empty() => {
                        Some(Endpoint::new(n, s))
                    }
                    _ => None,
                };
                if let Some(ready) = &mut self.node_graph.ready {
                    ready.viewer = if self.node_graph.viewer_socket.is_none() {
                        self.node_graph
                            .session
                            .document()
                            .nodes
                            .iter()
                            .find(|n| n.type_id.as_str() == "output.viewer" && !n.muted)
                            .and_then(|n| ready.eval.viewer_values.get(&n.id))
                            .map(ViewerOverlay::from_socket_value)
                    } else {
                        None
                    };
                }
            }
            NodeGraphAction::Run => {
                self.start_node_graph_run()?;
            }
            NodeGraphAction::Cancel => {
                self.node_graph.cancel_job();
            }
            NodeGraphAction::ToggleProposal { id, checked } => {
                if let Some(ready) = &mut self.node_graph.ready {
                    ready.batch.set_checked(&id, checked);
                }
            }
            NodeGraphAction::SetAllProposals { checked } => {
                if let Some(ready) = &mut self.node_graph.ready {
                    for p in &mut ready.batch.proposals {
                        p.checked = checked;
                    }
                }
            }
            NodeGraphAction::Apply => {
                self.apply_node_graph_batch()?;
            }
            NodeGraphAction::SelectLastHits => {
                self.select_last_node_graph_hits()?;
            }
        }
        Ok(())
    }

    fn start_node_graph_run(&mut self) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Wait for the current editor job before running the node graph.".to_owned());
        }
        let document = self
            .document
            .as_ref()
            .ok_or_else(|| "Open a difficulty first.".to_owned())?;
        let map_revision = document.composer.history.version();
        let map_folder = document.path.parent().map(Path::to_path_buf);
        let timing_lines: Vec<TimingLine> = document
            .composer
            .tempo
            .sections()
            .iter()
            .map(|s| TimingLine {
                start_ms: s.start.ms(),
                beat_length_ms: s.beat_length_ms,
            })
            .collect();

        let song = self.decoded_song_for_graph();
        let song_hash = song
            .as_ref()
            .map(|s| s.content_hash.clone())
            .unwrap_or_default();
        let song_offset_ms = self.song_offset;
        let song_duration_ms = song
            .as_ref()
            .map(|s| s.duration_ms())
            .unwrap_or_else(|| self.duration_ms());
        let song_timeline_end_ms = (song_duration_ms + song_offset_ms.max(0.0))
            .max(song_duration_ms)
            .max(1.0);
        let range = self.graph_analysis_range(song_duration_ms)?;
        let song_range = AnalysisRange::new(0.0, song_timeline_end_ms).ok();

        let doc_snapshot = self.node_graph.session.document().clone();
        let graph_hash = doc_snapshot.structural_hash();
        let viewer_socket = self.node_graph.viewer_socket.clone();
        let mut cache_snapshot = self.node_graph.cache.clone();

        self.node_graph.cancel_job();
        self.node_graph.runs += 1;
        let run = self.node_graph.runs;
        let song_stamp = self.node_graph.song_stamp;
        let stamp = AnalysisStamp::new(map_revision, song_offset_ms, song_hash, graph_hash, range);

        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new((0usize, 100usize)));
        let (send, receiver) = mpsc::channel();

        {
            let cancel = cancel.clone();
            let progress = progress.clone();
            std::thread::spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let audio = Arc::new(AppAudioProvider::new(song, map_folder, cancel.clone()));
                    let timing = Arc::new(AppTimingProvider::from_lines(timing_lines));
                    let mut job_registry = NodeRegistry::new();
                    register_audio_nodes(&mut job_registry, audio, timing)
                        .expect("audio nodes must register");

                    let stop = || cancel.load(Ordering::Relaxed);
                    let mut report_progress = |fraction: f64| {
                        if let Ok(mut p) = progress.lock() {
                            let pct = (fraction.clamp(0.0, 1.0) * 100.0).round() as usize;
                            *p = (pct, 100);
                        }
                    };
                    let eval = evaluate_graph_with_song_range(
                        &doc_snapshot,
                        &job_registry,
                        &mut cache_snapshot,
                        EvalTarget::AllLaneOutputs,
                        range,
                        Some(song_timeline_end_ms),
                        song_range,
                        &stop,
                        &mut report_progress,
                    );
                    let result = match eval {
                        Ok(res) => res,
                        Err(EvalError::Cancelled) => return GraphOutcome::Cancelled,
                        Err(err) => return GraphOutcome::Failed(err.to_string()),
                    };

                    let viewer_value = match viewer_socket {
                        Some(ref ep) if doc_snapshot.node(&ep.node).is_some() => {
                            let mut dummy = |_f: f64| {};
                            match evaluate_graph_with_song_range(
                                &doc_snapshot,
                                &job_registry,
                                &mut cache_snapshot,
                                EvalTarget::Socket(ep.clone()),
                                range,
                                Some(song_timeline_end_ms),
                                song_range,
                                &stop,
                                &mut dummy,
                            ) {
                                Ok(v) => v.socket_preview,
                                Err(EvalError::Cancelled) => return GraphOutcome::Cancelled,
                                Err(_) => None,
                            }
                        }
                        _ => {
                            let viewer_node = doc_snapshot
                                .nodes
                                .iter()
                                .find(|n| n.type_id.as_str() == "output.viewer" && !n.muted)
                                .map(|n| n.id.clone());
                            if let Some(viewer_id) = viewer_node {
                                let mut dummy = |_f: f64| {};
                                match evaluate_graph_with_song_range(
                                    &doc_snapshot,
                                    &job_registry,
                                    &mut cache_snapshot,
                                    EvalTarget::Nodes(vec![viewer_id.clone()]),
                                    range,
                                    Some(song_timeline_end_ms),
                                    song_range,
                                    &stop,
                                    &mut dummy,
                                ) {
                                    Ok(mut v) => v.viewer_values.remove(&viewer_id),
                                    Err(EvalError::Cancelled) => return GraphOutcome::Cancelled,
                                    Err(_) => None,
                                }
                            } else {
                                None
                            }
                        }
                    };

                    GraphOutcome::Done(Box::new(GraphEvalDone {
                        result,
                        viewer_value,
                        updated_cache: cache_snapshot,
                        stamp,
                        song_stamp,
                        run,
                        song_duration_ms,
                    }))
                }));
                let _ = send.send(match outcome {
                    Ok(done) => done,
                    Err(_) => GraphOutcome::Failed("Node graph evaluation stopped unexpectedly.".to_owned()),
                });
            });
        }

        self.node_graph.job = Some(GraphJob {
            receiver,
            cancel,
            progress,
            song_stamp,
            map_revision,
            song_offset_bits: norm_bits(song_offset_ms),
        });
        Ok(())
    }

    fn apply_node_graph_batch(&mut self) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Wait for the current job before applying graph hits.".to_owned());
        }
        let Some(ready) = self.node_graph.ready.as_ref() else {
            return Err("Run the node graph first.".to_owned());
        };
        if ready.song_stamp != self.node_graph.song_stamp {
            self.node_graph.ready = None;
            return Err("The song changed. Run the node graph again.".to_owned());
        }
        if ready.batch.proposals.iter().all(|p| !p.checked) {
            return Err("Bind at least one output slot and check at least one hit to apply.".to_owned());
        }

        let current_stamp = self
            .current_stamp_for_ready(ready)
            .ok_or_else(|| "Open a difficulty first.".to_owned())?;
        let options = self.node_graph.apply_options;
        let doc = self
            .document
            .as_mut()
            .ok_or_else(|| "Open a difficulty first.".to_owned())?;
        let report = apply_proposals(&mut doc.composer, &ready.batch, &current_stamp, &options)
            .map_err(|err| match err {
                ApplyError::StaleAnalysis { message, .. } => message,
                other => other.to_string(),
            })?;
        self.node_graph.last_applied = report.record;
        self.node_graph.ready = None;
        self.refresh_recovery_edit();
        let c = report.counts;
        self.status = format!(
            "Applied {} node graph hits ({} on objects, {} loose).",
            c.added, c.on_objects, c.loose
        );
        Ok(())
    }

    fn select_last_node_graph_hits(&mut self) -> Result<(), String> {
        let Some(record) = self.node_graph.last_applied.as_ref() else {
            return Err("No node graph batch has been applied yet.".to_owned());
        };
        let doc = self
            .document
            .as_mut()
            .ok_or_else(|| "Open a difficulty first.".to_owned())?;
        let selected = select_batch_hits(&mut doc.composer, record);
        self.status = format!(
            "Selected {} generated hit(s) from '{}'.",
            selected.len(),
            record.graph_name
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_analysis_range_includes_positive_song_offset_tail() {
        let mut app = Application::new();
        app.song_offset = 12_500.0;
        let range = app.graph_analysis_range(60_000.0).unwrap();
        assert_eq!(range.from_ms, 0.0);
        assert!(
            range.to_ms >= 72_500.0,
            "whole-song analysis must include positive song_offset tail, got {}",
            range.to_ms
        );

        // Negative song_offset must not shrink the whole-song ceiling below song_duration_ms.
        app.song_offset = -5_000.0;
        let neg_range = app.graph_analysis_range(60_000.0).unwrap();
        assert!(
            neg_range.to_ms >= 60_000.0,
            "negative song_offset must not cut song duration short, got {}",
            neg_range.to_ms
        );
    }
}
