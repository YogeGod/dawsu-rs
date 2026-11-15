//! The hitsound model (hitsound domain Responsibilities 1-6): lanes, moments with their hits, and
//! slider bodies. Decompile builds it, edits change it, compile writes it back, and the cue list
//! is built from it.

use crate::lazer::samples::LegacySample;
use crate::rules::{self, Plays};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, Sound, SongTime};
use dawsu_osu_format::LineId;
use std::collections::BTreeSet;

/// The object of the difficulty a moment or a slider body belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(pub(crate) LineId);

/// Where a lane's sound comes from (Responsibility 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneSource {
    /// A file in the mapset folder, as the folder's listing names it.
    File(String),
    /// A sound of the skin.
    Skin,
}

/// One sample sound (Responsibilities 1-3). A numbered layer is its own lane: the same sound and
/// set with another custom index. A numbered lane whose file is missing plays the skin's sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lane {
    pub identity: SampleIdentity,
    pub source: LaneSource,
}

impl Lane {
    /// What the audio engine plays for this lane: the folder file, or the skin's own sound.
    pub fn plays_as(&self) -> SampleIdentity {
        match (&self.source, &self.identity) {
            (LaneSource::File(name), _) => SampleIdentity::File(name.clone()),
            (LaneSource::Skin, SampleIdentity::Standard { set, sound, .. }) => SampleIdentity::Standard { set: *set, sound: *sound, index: CustomIndex::new(0) },
            // A map's own sample file that is not in the folder: lazer falls back to the skin's hitnormal.
            (LaneSource::Skin, SampleIdentity::File(_)) => SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Normal), index: CustomIndex::new(0) },
        }
    }
}

/// Where a moment sits (Responsibility 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Circle(ObjectId),
    /// A slider's head (node 0), a repeat, or its tail (the last node).
    SliderNode { object: ObjectId, node: usize },
    SpinnerEnd(ObjectId),
    /// Hits that have no object in the map (Responsibility 5).
    NoObject,
}

/// A moment: one time, one volume for all its hits (invariant 2), and the lanes sounding there.
#[derive(Debug, Clone, PartialEq)]
pub struct Moment {
    pub(crate) time: SongTime,
    pub(crate) place: Place,
    pub(crate) volume: i32,
    pub(crate) hits: BTreeSet<SampleIdentity>,
    /// The hits and volume decompile found, to tell an edited moment from an untouched one.
    pub(crate) decompiled: Option<(BTreeSet<SampleIdentity>, i32)>,
}

impl Moment {
    pub fn time(&self) -> SongTime {
        self.time
    }
    pub fn place(&self) -> Place {
        self.place
    }
    /// The stored volume, which may be below the played floor (invariant 8).
    pub fn volume(&self) -> i32 {
        self.volume
    }
    pub fn hits(&self) -> impl Iterator<Item = &SampleIdentity> {
        self.hits.iter()
    }
    pub fn has_hit(&self, lane: &SampleIdentity) -> bool {
        self.hits.contains(lane)
    }
    /// True when an edit changed the hits or the volume since decompile.
    pub fn is_changed(&self) -> bool {
        match &self.decompiled {
            Some((hits, volume)) => *hits != self.hits || *volume != self.volume,
            None => !self.hits.is_empty(),
        }
    }
}

/// A slider's body sound (Responsibility 6): the slide, the slider whistle when set, and the ticks.
#[derive(Debug, Clone, PartialEq)]
pub struct SliderBody {
    pub(crate) object: ObjectId,
    pub(crate) start: SongTime,
    pub(crate) end: SongTime,
    pub(crate) volume: i32,
    pub(crate) slide: SampleIdentity,
    pub(crate) whistle: bool,
    pub(crate) decompiled_whistle: bool,
    pub(crate) ticks: Vec<SongTime>,
    pub(crate) tick: SampleIdentity,
}

impl SliderBody {
    pub fn object(&self) -> ObjectId {
        self.object
    }
    pub fn start(&self) -> SongTime {
        self.start
    }
    pub fn end(&self) -> SongTime {
        self.end
    }
    pub fn volume(&self) -> i32 {
        self.volume
    }
    pub fn slide(&self) -> &SampleIdentity {
        &self.slide
    }
    /// The slider whistle's lane: the slide's set and index with the whistle sound.
    pub fn whistle_lane(&self) -> SampleIdentity {
        with_sound(&self.slide, Sound::Slider(SliderSound::Whistle))
    }
    pub fn whistle(&self) -> bool {
        self.whistle
    }
    pub fn ticks(&self) -> &[SongTime] {
        &self.ticks
    }
    pub fn tick(&self) -> &SampleIdentity {
        &self.tick
    }
    pub fn is_changed(&self) -> bool {
        self.whistle != self.decompiled_whistle
    }
}

pub(crate) fn with_sound(identity: &SampleIdentity, sound: Sound) -> SampleIdentity {
    match identity {
        SampleIdentity::Standard { set, index, .. } => SampleIdentity::Standard { set: *set, sound, index: *index },
        other => other.clone(),
    }
}

/// A moment's place in [`Hitsounds::moments`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MomentId(pub usize);

/// One difficulty's hitsounds as lanes, moments and slider bodies.
#[derive(Debug, Clone, PartialEq)]
pub struct Hitsounds {
    pub(crate) lanes: Vec<Lane>,
    pub(crate) moments: Vec<Moment>,
    pub(crate) bodies: Vec<SliderBody>,
    pub(crate) folder_files: Vec<String>,
}

impl Hitsounds {
    /// Every lane, sorted: per sample set the hitnormal first (Responsibility 2).
    pub fn lanes(&self) -> &[Lane] {
        &self.lanes
    }
    pub fn lane(&self, identity: &SampleIdentity) -> Option<&Lane> {
        self.lanes.iter().find(|l| &l.identity == identity)
    }
    /// Every moment, in time order as decompile found them, then moments added since.
    pub fn moments(&self) -> &[Moment] {
        &self.moments
    }
    pub fn moment(&self, id: MomentId) -> &Moment {
        &self.moments[id.0]
    }
    pub fn bodies(&self) -> &[SliderBody] {
        &self.bodies
    }

    /// The lane for `identity`, added when missing, with the sound the rules resolve for it.
    fn ensure_lane(&mut self, identity: &SampleIdentity) {
        if self.lane(identity).is_some() {
            return;
        }
        let source = match rules::resolve_one(&legacy_sample(identity), &self.folder_files).plays {
            Plays::BeatmapFile(name) => LaneSource::File(name),
            Plays::Skin { .. } => LaneSource::Skin,
        };
        let at = self.lanes.partition_point(|l| l.identity < *identity);
        self.lanes.insert(at, Lane { identity: identity.clone(), source });
    }

    /// Adds a hit on `lane` at a moment; it sounds at the moment's volume (invariant 2).
    pub fn add_hit(&mut self, moment: MomentId, lane: SampleIdentity) {
        self.ensure_lane(&lane);
        self.moments[moment.0].hits.insert(lane);
    }

    pub fn remove_hit(&mut self, moment: MomentId, lane: &SampleIdentity) {
        self.moments[moment.0].hits.remove(lane);
    }

    /// Sets the volume of every hit at a moment.
    pub fn set_volume(&mut self, moment: MomentId, volume: i32) {
        self.moments[moment.0].volume = volume;
    }

    /// Adds a moment with no object, for hits placed where the map has none (Responsibility 5).
    pub fn add_moment(&mut self, time: SongTime, volume: i32) -> MomentId {
        self.moments.push(Moment { time, place: Place::NoObject, volume, hits: BTreeSet::new(), decompiled: None });
        MomentId(self.moments.len() - 1)
    }

    pub fn set_body_whistle(&mut self, body: usize, whistle: bool) {
        let lane = self.bodies[body].whistle_lane();
        if whistle {
            self.ensure_lane(&lane);
        }
        self.bodies[body].whistle = whistle;
    }
}

pub(crate) fn sound_name(sound: Sound) -> &'static str {
    match sound {
        Sound::Hit(Hitsound::Normal) => "hitnormal",
        Sound::Hit(Hitsound::Whistle) => "hitwhistle",
        Sound::Hit(Hitsound::Finish) => "hitfinish",
        Sound::Hit(Hitsound::Clap) => "hitclap",
        Sound::Slider(SliderSound::Slide) => "sliderslide",
        Sound::Slider(SliderSound::Whistle) => "sliderwhistle",
        Sound::Slider(SliderSound::Tick) => "slidertick",
    }
}

pub(crate) fn set_name(set: SampleSet) -> &'static str {
    match set {
        SampleSet::Normal => "normal",
        SampleSet::Soft => "soft",
        SampleSet::Drum => "drum",
    }
}

/// The sample lazer would hold for a lane, to ask the rules what plays for it.
fn legacy_sample(identity: &SampleIdentity) -> LegacySample {
    match identity {
        SampleIdentity::Standard { set, sound, index } => LegacySample::legacy(sound_name(*sound), Some(set_name(*set)), 100, false, index.get() as i32, false),
        SampleIdentity::File(name) => LegacySample::file(name, 100),
    }
}
