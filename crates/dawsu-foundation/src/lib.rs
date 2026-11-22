//! Small domain-free building blocks: safe file writes, the time-ordered list,
//! time types, the shared sample vocabulary, the cue and the FFT.
//!
//! This crate uses the Rust standard library only and references no other dawsu crate
//! (foundation invariants 1 and 2).

pub mod cue;
pub mod digest;
pub mod error;
pub mod fft;
pub mod midi;
pub mod safe_write;
pub mod samples;
pub mod time;
pub mod time_list;

pub use cue::Cue;
pub use error::Error;
pub use midi::{
    MidiError, MidiEvent, MidiEventKind, MidiFile, MidiFormat, MidiNote, MidiTrack, TempoChange,
    TempoMap, TimeDivision, TimeSignature, TimedMidi, TimedProgramChange, TimedSustainEvent,
    TimedTimeSignature, TimedTrack,
};
pub use samples::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, Sound};
pub use time::{NoteTicks, SongTime};
pub use time_list::{Timed, TimeOrderedList};
