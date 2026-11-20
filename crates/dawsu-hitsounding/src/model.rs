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

impl ObjectId {
    /// The corresponding hit-object line for this parsed difficulty.
    pub fn line(self) -> LineId {
        self.0
    }
}

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Place {
    Circle(ObjectId),
    /// A slider's head (node 0), a repeat, or its tail (the last node).
    SliderNode { object: ObjectId, node: usize },
    SpinnerEnd(ObjectId),
    /// Hits that have no object in the map (Responsibility 5).
    NoObject,
}

/// A moment: one time, a default volume and the lanes sounding there.
#[derive(Debug, Clone, PartialEq)]
pub struct Moment {
    pub(crate) time: SongTime,
    pub(crate) place: Place,
    pub(crate) volume: i32,
    pub(crate) hits: BTreeSet<SampleIdentity>,
    hit_volumes: BTreeMap<SampleIdentity, i32>,
    baseline: Option<MomentBaseline>,
}

#[derive(Debug, Clone, PartialEq)]
struct MomentBaseline {
    hits: BTreeSet<SampleIdentity>,
    volume: i32,
    hit_volumes: BTreeMap<SampleIdentity, i32>,
}

impl Moment {
    fn new(time: SongTime, place: Place, volume: i32, hits: BTreeSet<SampleIdentity>) -> Self {
        Self { time, place, volume, hits, hit_volumes: BTreeMap::new(), baseline: None }
    }

    pub(crate) fn decoded(time: SongTime, place: Place, volume: i32, hits: BTreeSet<SampleIdentity>) -> Self {
        let mut moment = Self::new(time, place, volume, hits);
        moment.remember_baseline();
        moment
    }

    fn remember_baseline(&mut self) {
        self.baseline = Some(MomentBaseline {
            hits: self.hits.clone(),
            volume: self.volume,
            hit_volumes: self.hit_volumes.clone(),
        });
    }

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
    /// This hit's stored volume, falling back to the moment's default.
    pub fn hit_volume(&self, lane: &SampleIdentity) -> i32 {
        self.hit_volumes.get(lane).copied().unwrap_or(self.volume)
    }

    pub fn has_hit_volume_overrides(&self) -> bool {
        !self.hit_volumes.is_empty()
    }

    /// Volume edits move from the moment's own volume while a hit plays at it, else from its loudest hit.
    pub fn reference_volume(&self) -> i32 {
        if self.hits.iter().all(|lane| self.hit_volumes.contains_key(lane)) {
            self.hits.iter().map(|lane| self.hit_volume(lane)).max().unwrap_or(self.volume)
        } else {
            self.volume
        }
    }

    fn set_hit_volume(&mut self, lane: &SampleIdentity, volume: i32) {
        if volume == self.volume {
            self.hit_volumes.remove(lane);
        } else if self.has_hit(lane) {
            self.hit_volumes.insert(lane.clone(), volume);
        }
    }

    fn set_volume(&mut self, volume: i32) {
        self.volume = volume;
        self.hit_volumes.clear();
    }

    fn move_volume(&mut self, volume: i32) {
        let reference = self.reference_volume();
        if volume != reference {
            self.move_volume_from(reference, volume);
        }
    }

    fn move_volume_from(&mut self, from: i32, volume: i32) {
        let change = volume - from;
        let gains: Vec<_> = self.hit_volumes.iter()
            .map(|(lane, gain)| (lane.clone(), (gain + change).clamp(0, 100)))
            .collect();
        self.set_volume(volume);
        for (lane, gain) in gains {
            self.set_hit_volume(&lane, gain);
        }
    }

    pub fn hits(&self) -> impl Iterator<Item = &SampleIdentity> {
        self.hits.iter()
    }
    pub fn has_hit(&self, lane: &SampleIdentity) -> bool {
        self.hits.contains(lane)
    }
    /// True when an edit changed the hits or gains since loading this moment.
    pub fn is_changed(&self) -> bool {
        match &self.baseline {
            Some(baseline) => {
                baseline.hits != self.hits || baseline.volume != self.volume || baseline.hit_volumes != self.hit_volumes
            }
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
    pub(crate) whistle_set: Option<SampleSet>,
    pub(crate) decompiled_whistle: (Option<SampleSet>, SampleIdentity),
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
    /// The chosen whistle sample set. Auto is inherited from the slide's set.
    pub fn whistle_set(&self) -> Option<SampleSet> {
        self.whistle_set.or_else(|| {
            // The older slide-only edit preserves what the whistle plays. Expose the explicit
            // set that compile will write if keeping that sound breaks the inheritance.
            let effective = set_of(&self.whistle_lane);
            (self.set() != effective).then_some(effective).flatten()
        })
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
        (self.set(), self.volume, self.whistle) != self.decompiled || self.whistle_set_changed()
    }
    pub(crate) fn whistle_changed(&self) -> bool {
        self.whistle != self.decompiled.2
    }
    pub(crate) fn whistle_set_changed(&self) -> bool {
        self.whistle_set != self.decompiled_whistle.0 || self.whistle_lane != self.decompiled_whistle.1
    }
    pub(crate) fn set_changed(&self) -> bool {
        self.set() != self.decompiled.0
    }
    pub(crate) fn volume_changed(&self) -> bool {
        self.volume != self.decompiled.1
    }

    /// The body's editable sound now.
    pub fn state(&self) -> BodyState {
        BodyState { set: self.set(), volume: self.volume, whistle: self.whistle, whistle_set: self.whistle_set, whistle_lane: self.whistle_lane.clone() }
    }

    /// The body's sound as decompile found it.
    pub fn decompiled_state(&self) -> BodyState {
        let (set, volume, whistle) = self.decompiled;
        BodyState { set, volume, whistle, whistle_set: self.decompiled_whistle.0, whistle_lane: self.decompiled_whistle.1.clone() }
    }
}

/// A slider body's editable sound: the set of its slide and ticks, its volume, and its whistle
/// with the whistle's own set and lane.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    pub set: Option<SampleSet>,
    pub volume: i32,
    pub whistle: bool,
    pub whistle_set: Option<SampleSet>,
    pub whistle_lane: SampleIdentity,
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
    pub(crate) hidden_lanes: BTreeSet<SampleIdentity>,
    /// Sound set placements: the link from a set to the hits it placed, kept here so undo carries it.
    pub(crate) placements: Vec<crate::sound_sets::SoundSetPlacement>,
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

    fn visibility_key(&self, lane: &SampleIdentity) -> SampleIdentity {
        let slot = self.lane_slot(lane).unwrap_or(lane);
        match slot {
            SampleIdentity::File(name) => SampleIdentity::File(name.replace('\\', "/").to_ascii_lowercase()),
            _ => slot.clone(),
        }
    }

    pub fn is_lane_hidden(&self, lane: &SampleIdentity) -> bool {
        !self.hidden_lanes.is_empty() && self.hidden_lanes.contains(&self.visibility_key(lane))
    }

    pub fn hidden_lanes(&self) -> impl Iterator<Item = &SampleIdentity> {
        self.hidden_lanes.iter()
    }

    pub fn set_lane_hidden(&mut self, lane: &SampleIdentity, hidden: bool) {
        let slot = self.visibility_key(lane);
        if hidden {
            self.hidden_lanes.insert(slot);
        } else {
            self.hidden_lanes.remove(&slot);
        }
    }

    /// A saved hide only applies to empty lanes. Hits in the map may have come back by discarding
    /// an edit or from another mapper's changes.
    pub fn restore_hidden_lanes(&mut self, lanes: impl IntoIterator<Item = SampleIdentity>) {
        let occupied: BTreeSet<_> = self.moments.iter().flat_map(Moment::hits)
            .map(|lane| self.visibility_key(lane)).collect();
        self.hidden_lanes = lanes.into_iter().map(|lane| self.visibility_key(&lane))
            .filter(|slot| !occupied.contains(slot)).collect();
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

    /// Moves every editable hit without changing its position or gain.
    pub fn replace_lane(&mut self, source: &SampleIdentity, replacement: SampleIdentity) {
        if source == &replacement {
            return;
        }
        let slot = self.lane_slots.remove(source).unwrap_or_else(|| source.clone());
        self.bind_lane_slot(replacement.clone(), slot);
        for moment in &mut self.moments {
            if moment.hits.remove(source) {
                let gain = moment.hit_volumes.remove(source).unwrap_or(moment.volume);
                if moment.hits.insert(replacement.clone()) {
                    moment.set_hit_volume(&replacement, gain);
                }
            }
        }
    }

    /// The sound set placements, in the order they were made.
    pub fn placements(&self) -> &[crate::sound_sets::SoundSetPlacement] {
        &self.placements
    }

    /// Places a sound set on a moment: each `(member id, lane, volume)` becomes a hit unless the
    /// moment already has that lane, which stays as it is and is recorded as not added. One
    /// placement per set per moment; returns the placement's id, or None when the set is already there.
    pub fn place_sound_set(&mut self, moment: MomentId, set_id: &str, set_digest: &str, gain: i32, hits: &[(String, SampleIdentity, i32)]) -> Option<u64> {
        if self.placements.iter().any(|p| p.moment == moment && p.set_id == set_id && self.is_placement_live(p)) {
            return None;
        }
        // A placement of this set whose hits were all deleted is replaced.
        let dead: Vec<u64> = self.placements.iter().filter(|p| p.moment == moment && p.set_id == set_id).map(|p| p.id).collect();
        self.placements.retain(|p| !dead.contains(&p.id));
        let mut applied = Vec::with_capacity(hits.len());
        for (member_id, lane, volume) in hits {
            let added = !self.moments[moment.0].has_hit(lane);
            if added {
                self.add_hit_with_volume(moment, lane.clone(), *volume);
            }
            applied.push(crate::sound_sets::Applied { member_id: member_id.clone(), lane: lane.clone(), volume: *volume, added });
        }
        let id = self.placements.iter().map(|p| p.id).max().map_or(1, |id| id + 1);
        self.placements.push(crate::sound_sets::SoundSetPlacement {
            id, set_id: set_id.to_owned(), set_digest: set_digest.to_owned(), accepted_digest: None, moment, gain, applied,
        });
        Some(id)
    }

    /// True while one of the hits the placement added is still on its moment, or it added none.
    pub fn is_placement_live(&self, placement: &crate::sound_sets::SoundSetPlacement) -> bool {
        let moment = &self.moments[placement.moment.0];
        let mut added = placement.applied.iter().filter(|a| a.added).peekable();
        added.peek().is_none() || added.any(|a| moment.has_hit(&a.lane))
    }

    /// The lanes the live placements on a moment added; another set's placement leaves them alone.
    pub fn placed_lanes(&self, moment: MomentId) -> BTreeSet<SampleIdentity> {
        self.placements.iter().filter(|p| p.moment == moment && self.is_placement_live(p))
            .flat_map(|p| p.applied.iter().filter(|a| a.added).map(|a| a.lane.clone())).collect()
    }

    /// Removes a placement's link; its hits stay.
    pub fn unlink_placement(&mut self, id: u64) {
        self.placements.retain(|p| p.id != id);
    }

    /// Puts back placements read from a project or a recovery copy; ones whose moment is out of range are dropped.
    pub fn restore_placements(&mut self, placements: Vec<crate::sound_sets::SoundSetPlacement>) {
        let count = self.moments.len();
        self.placements = placements.into_iter().filter(|p| p.moment.0 < count).collect();
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

    /// Adds a hit on the lane at a moment using the moment's default volume.
    pub fn add_hit(&mut self, moment: MomentId, lane: SampleIdentity) {
        self.ensure_lane(&lane);
        if self.is_lane_hidden(&lane) {
            self.set_lane_hidden(&lane, false);
        }
        self.moments[moment.0].hits.insert(lane);
    }

    /// Adds a copied or moved hit with its stored volume. A hit already there keeps its volume.
    pub fn add_hit_with_volume(&mut self, moment: MomentId, lane: SampleIdentity, volume: i32) {
        if !self.moments[moment.0].has_hit(&lane) {
            self.add_hit(moment, lane.clone());
            self.set_hit_volume(moment, &lane, volume);
        }
    }

    pub fn remove_hit(&mut self, moment: MomentId, lane: &SampleIdentity) {
        self.moments[moment.0].hits.remove(lane);
        self.moments[moment.0].hit_volumes.remove(lane);
    }

    /// Changes one existing hit's stored volume.
    pub fn set_hit_volume(&mut self, moment: MomentId, lane: &SampleIdentity, volume: i32) {
        self.moments[moment.0].set_hit_volume(lane, volume);
    }

    /// Sets the volume of every hit at a moment.
    pub fn set_volume(&mut self, moment: MomentId, volume: i32) {
        self.moments[moment.0].set_volume(volume);
    }

    /// Sets a moment's volume and moves every hit with its own volume by the same amount, so the
    /// hits keep their difference. A hit stops at 0 or 100 and keeps that value afterwards.
    pub fn move_volume(&mut self, moment: MomentId, volume: i32) {
        self.moments[moment.0].move_volume(volume);
    }

    /// Restores the editable hit state certified against the decoded object's saved representation.
    pub fn restore_moment_hits(
        &mut self,
        id: MomentId,
        default_volume: i32,
        hits: impl IntoIterator<Item = (SampleIdentity, i32)>,
    ) {
        let moment = &mut self.moments[id.0];
        moment.hits.clear();
        moment.set_volume(default_volume);
        for (lane, volume) in hits {
            self.add_hit_with_volume(id, lane, volume);
        }
        self.moments[id.0].remember_baseline();
    }

    /// Sets a moment's hits and gains without touching what decompile found, so the moment still
    /// counts as changed where it differs from the file. A hidden lane stays hidden.
    pub fn set_moment_state(&mut self, id: MomentId, default_volume: i32, hits: impl IntoIterator<Item = (SampleIdentity, i32)>) {
        let hits: Vec<_> = hits.into_iter().collect();
        for (lane, _) in &hits {
            self.ensure_lane(lane);
        }
        let moment = &mut self.moments[id.0];
        moment.hits.clear();
        moment.set_volume(default_volume);
        for (lane, volume) in hits {
            moment.hits.insert(lane.clone());
            moment.set_hit_volume(&lane, volume);
        }
    }

    /// Puts a moment back to what decompile found. A moment with no object loses its hits.
    pub fn reset_moment(&mut self, id: MomentId) {
        let moment = &mut self.moments[id.0];
        match moment.baseline.clone() {
            Some(baseline) => {
                moment.hits = baseline.hits;
                moment.volume = baseline.volume;
                moment.hit_volumes = baseline.hit_volumes;
            }
            None => {
                moment.hits.clear();
                moment.hit_volumes.clear();
            }
        }
    }

    /// Sets a slider body's sound. Unlike [`Hitsounds::set_body_volume`], the head's volume stays.
    pub fn set_body_state(&mut self, body: usize, state: BodyState) {
        let b = &mut self.bodies[body];
        if let Some(set) = state.set {
            b.slide = with_set(&b.slide, set);
            b.tick = with_set(&b.tick, set);
        }
        b.volume = state.volume;
        b.whistle = state.whistle;
        b.whistle_set = state.whistle_set;
        b.whistle_lane = state.whistle_lane;
        let mut lanes = vec![b.slide.clone(), b.tick.clone()];
        if b.whistle {
            lanes.push(b.whistle_lane.clone());
        }
        for lane in lanes {
            self.ensure_lane(&lane);
        }
    }

    /// Adds a moment with no object, for hits placed where the map has none (Responsibility 5).
    pub fn add_moment(&mut self, time: SongTime, volume: i32) -> MomentId {
        self.moments.push(Moment::new(time, Place::NoObject, volume, BTreeSet::new()));
        MomentId(self.moments.len() - 1)
    }

    pub fn set_body_whistle(&mut self, body: usize, whistle: bool) {
        let lane = self.bodies[body].whistle_lane();
        if whistle {
            self.ensure_lane(&lane);
        }
        self.bodies[body].whistle = whistle;
    }

    /// Changes the whistle's own sample set. None inherits the slide's set.
    pub fn set_body_whistle_set(&mut self, body: usize, set: Option<SampleSet>) {
        let slider = &mut self.bodies[body];
        slider.whistle_set = set;
        if let Some(effective) = set.or_else(|| slider.set()) {
            slider.whistle_lane = with_set(&slider.whistle_lane, effective);
        }
        if slider.whistle {
            let lane = slider.whistle_lane();
            self.ensure_lane(&lane);
        }
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
            // The body plays at the head's own volume, so the head's hits move by the body's change.
            head.move_volume_from(head.volume, volume);
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
pub(crate) fn legacy_sample(identity: &SampleIdentity) -> LegacySample {
    match identity {
        SampleIdentity::Standard { set, sound, index } => LegacySample::legacy(sound_name(*sound), Some(set_name(*set)), 100, false, index.get() as i32, false),
        SampleIdentity::File(name) => LegacySample::file(name, 100),
    }
}
