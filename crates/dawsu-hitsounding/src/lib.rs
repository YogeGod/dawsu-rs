//! The Hitsound domain: the hitsound rules, lanes, hits and moments,
//! decompile, compile and the cue list.

pub mod cleanup;
pub mod compile;
pub mod copy;
pub mod cues;
pub mod decompile;
pub mod drums;
pub mod error;
pub mod generate;
pub mod hs;
pub mod index_packing;
pub mod lazer;
pub mod mania_import;
pub mod model;
pub mod playfield;
pub mod rules;
pub mod sound_sets;
pub mod tick_render;
pub mod verify;
pub mod volume_envelope;

pub use compile::{compile, Compiled, Placement, ProblemKind, SaveProblem};
pub use cues::{cue_list, cue_list_filtered};
pub use decompile::{decompile, Decompiled};
pub use error::Error;
pub use model::{BodyState, Hitsounds, Lane, LaneSource, Moment, MomentId, ObjectId, Place, SliderBody};
pub use verify::{verify_compiled_osu, verify_envelope_preserved, VerificationMismatch};
