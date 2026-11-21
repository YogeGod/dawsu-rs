//! Proposal conversion, stale-analysis validation, and single-undo batch application.

use dawsu_editor_composer::{
    BatchCounts, BatchHit, BatchOptions, BatchRefusal, Composer, HitKey, Policy,
};
use dawsu_foundation::SampleIdentity;
use dawsu_graph::{AnalysisRange, GraphEvalResult, NodeDiagnostic, NodeId};
use std::collections::BTreeMap;
use std::fmt;

pub const STALE_ANALYSIS_MESSAGE: &str =
    "The map or audio changed since the analysis. Run it again.";

/// Per-map sidecar bindings from graph output slot names to map lanes.
pub type SlotBindings = BTreeMap<String, SampleIdentity>;

/// Which part of the `AnalysisStamp` changed since the batch was evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReason {
    MapRevision,
    SongOffset,
    SourceAudio,
    GraphHash,
    Range,
}

/// Snapshot of everything a proposal batch depends on (Rule R9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisStamp {
    pub map_revision: u64,
    pub song_offset_bits: u64,
    pub source_audio_sha256: String,
    pub graph_hash: String,
    pub range_from_bits: u64,
    pub range_to_bits: u64,
}

impl AnalysisStamp {
    pub fn new(
        map_revision: u64,
        song_offset_ms: f64,
        source_audio_sha256: impl Into<String>,
        graph_hash: impl Into<String>,
        range: AnalysisRange,
    ) -> Self {
        let norm = |v: f64| if v == 0.0 { 0.0_f64.to_bits() } else { v.to_bits() };
        let (range_from_bits, range_to_bits) = range.canonical_bits();
        Self {
            map_revision,
            song_offset_bits: norm(song_offset_ms),
            source_audio_sha256: source_audio_sha256.into(),
            graph_hash: graph_hash.into(),
            range_from_bits,
            range_to_bits,
        }
    }

    pub fn differs_from(&self, current: &Self) -> Option<StaleReason> {
        if self.map_revision != current.map_revision {
            Some(StaleReason::MapRevision)
        } else if self.song_offset_bits != current.song_offset_bits {
            Some(StaleReason::SongOffset)
        } else if self.source_audio_sha256 != current.source_audio_sha256 {
            Some(StaleReason::SourceAudio)
        } else if self.graph_hash != current.graph_hash {
            Some(StaleReason::GraphHash)
        } else if self.range_from_bits != current.range_from_bits
            || self.range_to_bits != current.range_to_bits
        {
            Some(StaleReason::Range)
        } else {
            None
        }
    }
}

/// One reviewable hit proposal produced by a bound graph slot.
#[derive(Debug, Clone, PartialEq)]
pub struct HitProposal {
    pub id: String,
    pub slot: String,
    pub lane: SampleIdentity,
    pub time_ms: f64,
    pub strength: f64,
    pub source_node: NodeId,
    pub checked: bool,
}

/// Review batch produced from a `GraphEvalResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalBatch {
    pub batch_id: String,
    pub graph_name: String,
    pub stamp: AnalysisStamp,
    pub range: AnalysisRange,
    pub proposals: Vec<HitProposal>,
    pub diagnostics: BTreeMap<NodeId, Vec<NodeDiagnostic>>,
}

impl ProposalBatch {
    /// Converts bound slot outputs from `result` into `HitProposal`s (rounded to 4 decimals).
    /// Unbound slots produce no proposals.
    pub fn from_eval_result(
        batch_id: impl Into<String>,
        graph_name: impl Into<String>,
        result: &GraphEvalResult,
        bindings: &SlotBindings,
        slot_nodes: &BTreeMap<String, NodeId>,
        stamp: AnalysisStamp,
    ) -> Self {
        let mut proposals = Vec::new();
        for (slot, events) in &result.slot_events {
            let Some(lane) = bindings.get(slot) else {
                continue;
            };
            let source_node = slot_nodes
                .get(slot)
                .cloned()
                .unwrap_or_else(|| NodeId::from(slot.as_str()));
            for (index, ev) in events.events.iter().enumerate() {
                proposals.push(HitProposal {
                    id: format!("{slot}:{}:{index}", ev.frame),
                    slot: slot.clone(),
                    lane: lane.clone(),
                    time_ms: round4(ev.time_ms),
                    strength: round4(ev.strength),
                    source_node: source_node.clone(),
                    checked: true,
                });
            }
        }
        proposals.sort_by(|a, b| {
            a.time_ms
                .total_cmp(&b.time_ms)
                .then_with(|| a.slot.cmp(&b.slot))
                .then_with(|| a.id.cmp(&b.id))
        });
        Self {
            batch_id: batch_id.into(),
            graph_name: graph_name.into(),
            stamp,
            range: result.range,
            proposals,
            diagnostics: result.diagnostics.clone(),
        }
    }

    pub fn set_checked(&mut self, id: &str, checked: bool) -> bool {
        if let Some(p) = self.proposals.iter_mut().find(|p| p.id == id) {
            p.checked = checked;
            true
        } else {
            false
        }
    }

    pub fn checked_proposals(&self) -> impl Iterator<Item = &HitProposal> {
        self.proposals.iter().filter(|p| p.checked)
    }
}

/// Options controlling proposal anchoring and deduplication inside `Composer::add_batch`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ApplyOptions {
    pub tolerance_ms: f64,
    pub dedupe_ms: f64,
    pub include_unmatched: bool,
    pub policy: Option<Policy>,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            tolerance_ms: 30.0,
            dedupe_ms: 2.0,
            include_unmatched: false,
            policy: None,
        }
    }
}

/// Metadata recorded for a generation batch so generated hits can be re-selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationBatchRecord {
    pub batch_id: String,
    pub graph_name: String,
    pub graph_hash: String,
    pub hits: Vec<HitKeyRecord>,
}

/// Recorded `HitKey` for batch re-selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitKeyRecord {
    pub moment: usize,
    pub lane: SampleIdentity,
}

impl From<&HitKey> for HitKeyRecord {
    fn from(value: &HitKey) -> Self {
        Self {
            moment: value.moment,
            lane: value.lane.clone(),
        }
    }
}

impl From<&HitKeyRecord> for HitKey {
    fn from(value: &HitKeyRecord) -> Self {
        Self {
            moment: value.moment,
            lane: value.lane.clone(),
        }
    }
}

/// Error returned when `apply_proposals` refuses a batch.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplyError {
    StaleAnalysis { reason: StaleReason, message: String },
    ComposerRefused(BatchRefusal),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::StaleAnalysis { message, .. } => f.write_str(message),
            ApplyError::ComposerRefused(BatchRefusal::StepOpen) => {
                write!(f, "Finish the current edit before applying graph proposals.")
            }
            ApplyError::ComposerRefused(BatchRefusal::MissingLane(lane)) => {
                write!(f, "Target lane {lane:?} no longer exists.")
            }
        }
    }
}

impl std::error::Error for ApplyError {}

/// Outcome of `apply_proposals`.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyReport {
    pub counts: BatchCounts,
    pub record: Option<GenerationBatchRecord>,
    pub diagnostics: BTreeMap<NodeId, Vec<NodeDiagnostic>>,
}

/// Checks `AnalysisStamp` for staleness and commits all checked proposals in one undo step.
pub fn apply_proposals(
    composer: &mut Composer,
    batch: &ProposalBatch,
    current_stamp: &AnalysisStamp,
    options: &ApplyOptions,
) -> Result<ApplyReport, ApplyError> {
    if let Some(reason) = batch.stamp.differs_from(current_stamp) {
        return Err(ApplyError::StaleAnalysis {
            reason,
            message: STALE_ANALYSIS_MESSAGE.to_owned(),
        });
    }

    let batch_hits: Vec<BatchHit> = batch
        .checked_proposals()
        .map(|p| BatchHit {
            time: p.time_ms,
            lane: p.lane.clone(),
            strength: p.strength,
            grid: Some(p.time_ms),
        })
        .collect();

    let policy = options.policy.unwrap_or(Policy::Classic {
        filter_grid: false,
        include_unmatched: options.include_unmatched,
    });
    let batch_options = BatchOptions {
        object_ms: options.tolerance_ms,
        dedupe_ms: options.dedupe_ms,
        shift_ms: 0.0,
        range: Some((batch.range.from_ms, batch.range.to_ms)),
        policy,
    };

    let report = composer
        .add_batch(&batch.graph_name, &batch_hits, &batch_options)
        .map_err(ApplyError::ComposerRefused)?;

    let record = (!report.added.is_empty()).then(|| GenerationBatchRecord {
        batch_id: batch.batch_id.clone(),
        graph_name: batch.graph_name.clone(),
        graph_hash: batch.stamp.graph_hash.clone(),
        hits: report.added.iter().map(HitKeyRecord::from).collect(),
    });

    Ok(ApplyReport {
        counts: report.counts,
        record,
        diagnostics: batch.diagnostics.clone(),
    })
}

/// Re-selects all surviving hits from a previously applied `GenerationBatchRecord`.
pub fn select_batch_hits(composer: &mut Composer, record: &GenerationBatchRecord) -> Vec<HitKey> {
    let surviving: Vec<HitKey> = record
        .hits
        .iter()
        .filter_map(|rec| {
            let moment = composer.hitsounds().moments().get(rec.moment)?;
            moment
                .has_hit(&rec.lane)
                .then(|| HitKey::from(rec))
        })
        .collect();
    composer.selected.replace(surviving.clone());
    surviving
}

fn round4(value: f64) -> f64 {
    if value.is_finite() {
        (value * 10_000.0).round_ties_even() / 10_000.0
    } else {
        value
    }
}
