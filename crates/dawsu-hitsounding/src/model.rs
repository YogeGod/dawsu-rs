//! The hitsound model (hitsound domain Responsibilities 1-6): lanes, moments with their hits, and
//! slider bodies. Decompile builds it, edits change it, compile writes it back, and the cue list
//! is built from it.

use crate::lazer::samples::LegacySample;
use crate::rules::{self, Plays};
use dawsu_foundation::{CustomIndex, Hitsound, SampleIdentity, SampleSet, SliderSound, Sound, SongTime};
use dawsu_osu_format::LineId;
use std::collections::{BTreeMap, BTreeSet};

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
            // A map's own sample file that is not in the folder: after the file's names, lazer's
            // lookup names go on to the normal hitnormal (ConvertHitObjectParser.cs:691 and
            // :696-700 at 131fe72), which the skin answers (hitsound rules R13).
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
    /// The slider whistle's lane, from the slider's addition bank (the rules' `whistle_sample`).
    pub(crate) whistle_lane: SampleIdentity,
    pub(crate) whistle: bool,
    pub(crate) ticks: Vec<SongTime>,
    pub(crate) tick: SampleIdentity,
    /// The set, volume and whistle decompile found, to tell an edited body from an untouched one.
    pub(crate) decompiled: (Option<SampleSet>, i32, bool),
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
    /// The slider whistle's lane: lazer plays the slider whistle from the slider's addition bank,
    /// which may differ from the slide's set.
    pub fn whistle_lane(&self) -> SampleIdentity {
        self.whistle_lane.clone()
    }
    /// The body's sample set: the set of its slide and ticks.
    pub fn set(&self) -> Option<SampleSet> {
        set_of(&self.slide)
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
        (self.set(), self.volume, self.whistle) != self.decompiled
    }
    pub(crate) fn whistle_changed(&self) -> bool {
        self.whistle != self.decompiled.2
    }
    pub(crate) fn set_changed(&self) -> bool {
        self.set() != self.decompiled.0
    }
    pub(crate) fn volume_changed(&self) -> bool {
        self.volume != self.decompiled.1
    }
}

pub(crate) fn with_sound(identity: &SampleIdentity, sound: Sound) -> SampleIdentity {
    match identity {
        SampleIdentity::Standard { set, index, .. } => SampleIdentity::Standard { set: *set, sound, index: *index },
        other => other.clone(),
    }
}

fn with_set(identity: &SampleIdentity, set: SampleSet) -> SampleIdentity {
    match identity {
        SampleIdentity::Standard { sound, index, .. } => SampleIdentity::Standard { set, sound: *sound, index: *index },
        other => other.clone(),
    }
}

fn set_of(identity: &SampleIdentity) -> Option<SampleSet> {
    match identity {
        SampleIdentity::Standard { set, .. } => Some(*set),
        SampleIdentity::File(_) => None,
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
    pub(crate) lane_slots: BTreeMap<SampleIdentity, SampleIdentity>,
}

impl Hitsounds {
    /// Every lane, sorted: per sample set the hitnormal first (Responsibility 2).
    pub fn lanes(&self) -> &[Lane] {
        &self.lanes
    }
    pub fn lane(&self, identity: &SampleIdentity) -> Option<&Lane> {
        self.lanes.iter().find(|l| &l.identity == identity)
    }
    /// The logical slot originally occupied by this lane's hits.
    pub fn lane_slot(&self, lane: &SampleIdentity) -> Option<&SampleIdentity> {
        self.lane_slots.get(lane)
    }

    /// Bindings from a sample's backing lane to its original logical slot.
    pub fn lane_slots(&self) -> impl Iterator<Item = (&SampleIdentity, &SampleIdentity)> {
        self.lane_slots.iter()
    }

    pub fn bind_lane_slot(&mut self, source: SampleIdentity, slot: SampleIdentity) {
        if source != slot && self.lane(&source).is_some() {
            self.ensure_lane(&slot);
            self.lane_slots.insert(source, slot);
        }
    }

    /// Moves every editable hit without changing its position or shared volume.
    pub fn replace_lane(&mut self, source: &SampleIdentity, replacement: SampleIdentity) {
        let slot = self.lane_slots.remove(source).unwrap_or_else(|| source.clone());
        self.bind_lane_slot(replacement.clone(), slot);
        for moment in &mut self.moments {
            if moment.hits.remove(source) {
                moment.hits.insert(replacement.clone());
            }
        }
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

    /// Registers files added to the mapset, keeping hits and existing lane identities.
    pub fn add_folder_samples(&mut self, names: &[String]) {
        self.folder_files.extend_from_slice(names);
        self.folder_files.sort();
        self.folder_files.dedup();
        let identities: BTreeSet<_> = names.iter().map(|name| {
            rules::read_sample_file_name(name).unwrap_or_else(|| SampleIdentity::File(name.clone()))
        }).collect();
        for lane in &mut self.lanes {
            let matched_filename = match &lane.identity {
                SampleIdentity::File(file) => dawsu_osu_format::find_case_insensitive(names, file).is_some(),
                _ => false,
            };
            if identities.contains(&lane.identity) || matched_filename {
                lane.source = match rules::resolve_one(&legacy_sample(&lane.identity), &self.folder_files).plays {
                    Plays::BeatmapFile(name) => LaneSource::File(name),
                    Plays::Skin { .. } => LaneSource::Skin,
                };
            }
        }
        for identity in identities {
            self.ensure_lane(&identity);
        }
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

    /// Sets the sample set of a slider body's slide and ticks; the slider whistle keeps its set
    /// (roadmap, Yoge 2026-11-15). The index is not changed here (todo.md, slider body index).
    pub fn set_body_set(&mut self, body: usize, set: SampleSet) {
        let b = &mut self.bodies[body];
        b.slide = with_set(&b.slide, set);
        b.tick = with_set(&b.tick, set);
        let (slide, tick) = (b.slide.clone(), b.tick.clone());
        self.ensure_lane(&slide);
        self.ensure_lane(&tick);
    }

    /// Sets the volume of a slider body's slide, whistle and ticks, and of its head: the head reads
    /// its volume 1 ms before the body, so one green line sets both (ADR 0042 point 6).
    pub fn set_body_volume(&mut self, body: usize, volume: i32) {
        let object = self.bodies[body].object;
        self.bodies[body].volume = volume;
        if let Some(head) = self.moments.iter_mut().find(|m| m.place == Place::SliderNode { object, node: 0 }) {
            head.volume = volume;
        }
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
