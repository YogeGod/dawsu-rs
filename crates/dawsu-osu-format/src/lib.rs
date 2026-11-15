//! Reads and writes `.osu` files so that every unedited line stays byte-identical (ADR 0028).
//! The file is held as its original lines; typed fields are read the way lazer reads them,
//! and every write goes through foundation's safe write (ADR 0038).

mod error;
mod file;
mod folder;
pub mod number;
mod records;

pub use error::Error;
pub use file::{round_trip_check, ChangeSummary, LineEnding, LineView, OsuFile, Read, ReadWarning, Refusal, LineSnapshot, RoundTripReport, SaveOutcome, SetOutcome, StoryboardSample};
pub use folder::{check_map_path, find_case_insensitive, list_folder, PathRefusal};
pub use records::{HitObject, HitSample, ObjectKind, SliderFields, TimingPoint};

/// A line of a file, stable across inserts and removals of other lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineId(pub(crate) u64);
