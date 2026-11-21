//! Undoable graph editing and proposal application (Layer 5 · Editing).
//!
//! References `dawsu-foundation`, `dawsu-editor`, `dawsu-graph`, `dawsu-hitsounding`, and
//! `dawsu-editor-composer`. Never references `dawsu-graph-audio` or `dawsu-graph-stems` (Rule R9).

pub mod proposals;
pub mod session;

pub use proposals::{
    AnalysisStamp, ApplyError, ApplyOptions, ApplyReport, GenerationBatchRecord, HitKeyRecord,
    HitProposal, ProposalBatch, STALE_ANALYSIS_MESSAGE, SlotBindings, StaleReason,
    apply_proposals, select_batch_hits,
};
pub use session::{NodalError, NodalSession};
