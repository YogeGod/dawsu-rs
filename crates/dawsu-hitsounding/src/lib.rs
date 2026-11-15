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
pub mod lazer;
pub mod model;
pub mod rules;

pub use compile::{compile, Compiled, Placement, ProblemKind, SaveProblem};
pub use cues::cue_list;
pub use decompile::{decompile, Decompiled};
pub use error::Error;
pub use model::{Hitsounds, Lane, LaneSource, Moment, MomentId, ObjectId, Place, SliderBody};
