//! The moments Save left out of the `.osu`, kept in dawsu!'s project so they come back on reopen
//! (F052; Yoge, 2026-11-19/20). Like gain recovery, a record is staged before the `.osu` write and
//! promoted after it, and names each part by its object's ordinal in the written file.

use super::Document;
use super::project::{lane_identity, lane_key};
use super::roll_back::{Held, LeftOut, MomentState, Part};
use dawsu_foundation::{SampleIdentity, SampleSet, digest::sha256_hex};
use dawsu_hitsounding::{BodyState, Decompiled, Hitsounds, MomentId, Place, decompile};
use dawsu_osu_format::{LineId, OsuFile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Unwritten {
    current: Option<Record>,
    pending: Option<Record>,
    before: Option<String>,
}

/// The left-out parts of one written file, by that file's digest.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Record {
    map_digest: String,
    moments: Vec<MomentRecord>,
    #[serde(default)]
    bodies: Vec<BodyRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct MomentRecord {
    object: usize,
    node: Option<usize>,
    time_bits: u64,
    default_volume: i32,
    hits: Vec<HitRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct HitRecord {
    lane: String,
    volume: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct BodyRecord {
    object: usize,
    /// The slider's start, to check the object and to name it in a warning.
    time_bits: u64,
    volume: i32,
    whistle: bool,
    set: Option<String>,
    whistle_set: Option<String>,
    whistle_lane: String,
}

impl Unwritten {
    fn for_digest(&self, digest: &str) -> Option<&Record> {
        self.pending.as_ref()
            .filter(|record| record.map_digest == digest && self.before.as_deref() != Some(digest))
            .or_else(|| self.current.as_ref().filter(|record| record.map_digest == digest))
    }

    /// True when no revision holds a left-out part.
    pub(crate) fn is_empty(&self) -> bool {
        self.current.iter().chain(self.pending.iter()).all(Record::is_empty)
    }

    /// Before writing `before`'s replacement: the record for the new file waits until it is written.
    pub(crate) fn stage(&mut self, before: &[u8], after: Record) {
        let digest = sha256_hex(before);
        if let Some(current) = self.for_digest(&digest) {
            self.current = Some(current.clone());
        }
        self.before = Some(digest);
        self.pending = Some(after);
    }

    /// After the write: the staged record is the current one when it names the written bytes.
    pub(crate) fn promote(&mut self, written: &[u8]) {
        let digest = sha256_hex(written);
        if self.pending.as_ref().is_some_and(|pending| pending.map_digest == digest) {
            self.current = self.pending.take();
            self.before = None;
        }
    }

    /// When the map bytes are unchanged there is nothing to wait for.
    pub(crate) fn set_current(&mut self, record: Record) {
        self.current = Some(record);
        self.pending = None;
        self.before = None;
    }

    /// The times of the parts this record holds, for a warning.
    fn times(&self) -> Vec<f64> {
        let mut times: Vec<f64> = self.current.iter().chain(self.pending.iter())
            .flat_map(|record| record.moments.iter().map(|m| m.time_bits).chain(record.bodies.iter().map(|b| b.time_bits)))
            .map(f64::from_bits)
            .collect();
        times.sort_by(f64::total_cmp);
        times.dedup();
        times
    }
}

impl Record {
    pub(crate) fn is_empty(&self) -> bool {
        self.moments.is_empty() && self.bodies.is_empty()
    }
}

fn location(place: Place) -> Option<(LineId, Option<usize>)> {
    match place {
        Place::Circle(id) | Place::SpinnerEnd(id) => Some((id.line(), None)),
        Place::SliderNode { object, node } => Some((object.line(), Some(node))),
        Place::NoObject => None,
    }
}

fn set_name(set: Option<SampleSet>) -> Option<String> {
    set.map(|set| match set {
        SampleSet::Normal => "normal",
        SampleSet::Soft => "soft",
        SampleSet::Drum => "drum",
    }.to_string())
}

fn set_of(name: &Option<String>) -> Result<Option<SampleSet>, String> {
    match name.as_deref() {
        None => Ok(None),
        Some("normal") => Ok(Some(SampleSet::Normal)),
        Some("soft") => Ok(Some(SampleSet::Soft)),
        Some("drum") => Ok(Some(SampleSet::Drum)),
        Some(other) => Err(format!("unknown sample set {other}")),
    }
}

fn hit_records(hits: &[(SampleIdentity, i32)]) -> Vec<HitRecord> {
    hits.iter().map(|(lane, volume)| HitRecord { lane: lane_key(lane), volume: *volume }).collect()
}

/// What a Save keeps of its left-out parts: the record for the written file, and the loose
/// moments of HS groups that never became a circle, which go to the project's loose hits.
pub(crate) struct Built {
    pub record: Record,
    pub loose: Vec<MomentState>,
}

/// Builds the record of `left_out` for `out`, the file this Save writes. `drawn` gives each
/// body's start; `circles` the circle each HS group became, if any.
pub(crate) fn build(
    left_out: &[LeftOut], drawn: &Hitsounds, out: &OsuFile, files: &[String], circles: &BTreeMap<i64, LineId>,
) -> Result<Built, String> {
    let bytes = out.to_bytes();
    let ordinals: HashMap<LineId, usize> = out.hit_objects().iter().enumerate().map(|(i, o)| (o.line(), i)).collect();
    let ordinal = |line: LineId| ordinals.get(&line).copied().ok_or("A left-out part has no object in the written file.");
    let mut record = Record { map_digest: sha256_hex(&bytes), moments: Vec::new(), bodies: Vec::new() };
    let mut loose = Vec::new();
    let mut circle_times: Option<HashMap<LineId, u64>> = None;
    for entry in left_out {
        match (entry.part, &entry.held) {
            (Part::Moment(place), Held::Moment(state)) => {
                let (line, node) = location(place).ok_or("A left-out moment has no object.")?;
                record.moments.push(MomentRecord {
                    object: ordinal(line)?, node, time_bits: state.time_bits, default_volume: state.volume, hits: hit_records(&state.hits),
                });
            }
            (Part::Body(object), Held::Body(state)) => {
                let body = drawn.bodies().iter().find(|b| b.object() == object).ok_or("A left-out slider body is missing.")?;
                record.bodies.push(BodyRecord {
                    object: ordinal(object.line())?, time_bits: body.start().ms().to_bits(), volume: state.volume, whistle: state.whistle,
                    set: set_name(state.set), whistle_set: set_name(state.whistle_set), whistle_lane: lane_key(&state.whistle_lane),
                });
            }
            (Part::Group(time), Held::Group(states)) => match circles.get(&time) {
                // Written before: the circle keeps its last written hits, and reopening puts the
                // drawn ones back on it.
                Some(&line) => {
                    let mut lanes = BTreeSet::new();
                    let mut hits = Vec::new();
                    for (lane, volume) in states.iter().flat_map(|state| state.hits.iter()) {
                        if !lanes.insert(lane.clone()) {
                            return Err("Save can't keep a left-out HS circle that holds one sample twice. Undo the last change there and save again.".into());
                        }
                        hits.push((lane.clone(), *volume));
                    }
                    let times = match &mut circle_times {
                        Some(times) => times,
                        None => circle_times.insert(circle_moment_times(out, files)?),
                    };
                    let time_bits = *times.get(&line).ok_or("A left-out HS circle is missing.")?;
                    let default_volume = states.first().map_or(100, |state| state.volume);
                    record.moments.push(MomentRecord { object: ordinal(line)?, node: None, time_bits, default_volume, hits: hit_records(&hits) });
                }
                None => loose.extend(states.iter().cloned()),
            },
            _ => return Err("A left-out part does not match what it held.".into()),
        }
    }
    Ok(Built { record, loose })
}

/// Each circle's moment time in `out`, as reopening it decodes it.
fn circle_moment_times(out: &OsuFile, files: &[String]) -> Result<HashMap<LineId, u64>, String> {
    let Decompiled::Done(decoded) = decompile(out, files, &|| false, &mut |_, _| {}) else {
        return Err("Could not read back the written difficulty.".into());
    };
    Ok(decoded.moments().iter().filter_map(|m| match m.place() {
        Place::Circle(id) => Some((id.line(), m.time().ms().to_bits())),
        _ => None,
    }).collect())
}

fn clock(ms: f64) -> String {
    let rounded = ms.round().max(0.0) as u64;
    format!("{:02}:{:02}.{:03}", rounded / 60_000, rounded % 60_000 / 1000, rounded % 1000)
}

/// Puts the drawn hits of the record for the opened file back, without making them the
/// baseline, so each part still counts as changed. Returns whether anything was put back, or a
/// warning when the record is kept but not applied.
pub(crate) fn restore(document: &mut Document, unwritten: &Unwritten) -> Result<bool, String> {
    let digest = sha256_hex(document.file.original_bytes());
    let Some(record) = unwritten.for_digest(&digest) else {
        if unwritten.is_empty() {
            return Ok(false);
        }
        let times: Vec<_> = unwritten.times().into_iter().map(clock).collect();
        return Err(format!(
            "The difficulty changed outside dawsu!, so the moments the last Save left out were not put back ({}). The project keeps them until the next Save.",
            times.join(", ")
        ));
    };
    if record.is_empty() {
        return Ok(false);
    }
    let checked = check(document, record).map_err(|error| format!(
        "The moments the last Save left out could not be put back: {error}. The project keeps them until the next Save."
    ))?;
    let (moments, bodies) = checked;
    let composer = &mut document.composer;
    composer.history.update_external(|hits| {
        for (id, volume, lanes) in &moments {
            hits.set_moment_state(*id, *volume, lanes.clone());
        }
        for (index, state) in &bodies {
            hits.set_body_state(*index, state.clone());
        }
    });
    for lane in composer.hitsounds().lanes().iter().map(|lane| lane.identity.clone()).collect::<Vec<_>>() {
        if !composer.lanes.contains(&lane) {
            composer.lanes.push(lane);
        }
    }
    Ok(true)
}

type Checked = (Vec<(MomentId, i32, Vec<(SampleIdentity, i32)>)>, Vec<(usize, BodyState)>);

/// Each record entry matched to the opened difficulty, or why one does not match.
fn check(document: &Document, record: &Record) -> Result<Checked, String> {
    let objects = document.file.hit_objects();
    let hits = document.composer.hitsounds();
    let volume = |v: i32| if (0..=100).contains(&v) { Ok(v) } else { Err(format!("volume {v} is out of range")) };
    let mut moments = Vec::new();
    for saved in &record.moments {
        let object = objects.get(saved.object).ok_or("an object is missing")?;
        let id = hits.moments().iter().position(|m| location(m.place()) == Some((object.line(), saved.node)))
            .ok_or("a hit point is missing")?;
        if hits.moment(MomentId(id)).time().ms().to_bits() != saved.time_bits {
            return Err("a hit point moved".into());
        }
        let lanes = saved.hits.iter().map(|hit| {
            Ok((lane_identity(&hit.lane).ok_or_else(|| format!("unknown lane {}", hit.lane))?, volume(hit.volume)?))
        }).collect::<Result<Vec<_>, String>>()?;
        moments.push((MomentId(id), volume(saved.default_volume)?, lanes));
    }
    let mut bodies = Vec::new();
    for saved in &record.bodies {
        let object = objects.get(saved.object).ok_or("a slider is missing")?;
        let index = hits.bodies().iter().position(|b| b.object().line() == object.line()).ok_or("a slider body is missing")?;
        if hits.bodies()[index].start().ms().to_bits() != saved.time_bits {
            return Err("a slider moved".into());
        }
        let whistle_lane = lane_identity(&saved.whistle_lane).ok_or_else(|| format!("unknown lane {}", saved.whistle_lane))?;
        bodies.push((index, BodyState {
            set: set_of(&saved.set)?, volume: volume(saved.volume)?, whistle: saved.whistle,
            whistle_set: set_of(&saved.whistle_set)?, whistle_lane,
        }));
    }
    Ok((moments, bodies))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(digest: &[u8], object: usize) -> Record {
        Record {
            map_digest: sha256_hex(digest),
            moments: vec![MomentRecord { object, node: Some(1), time_bits: 2500f64.to_bits(), default_volume: 80, hits: vec![
                HitRecord { lane: "Skin|soft-hitclap".into(), volume: 80 }, HitRecord { lane: "Skin|drum-hitclap".into(), volume: 60 },
            ] }],
            bodies: Vec::new(),
        }
    }

    #[test]
    fn a_staged_record_counts_for_the_new_file_only_once_promoted_or_after_a_crash() {
        let (old, new) = (b"old".as_slice(), b"new".as_slice());
        let mut unwritten = Unwritten::default();
        unwritten.set_current(record(old, 0));
        unwritten.stage(old, record(new, 1));
        // A crash between the `.osu` write and the promote: the new file finds its pending record.
        assert_eq!(unwritten.for_digest(&sha256_hex(new)).map(|r| r.moments[0].object), Some(1));
        assert_eq!(unwritten.for_digest(&sha256_hex(old)).map(|r| r.moments[0].object), Some(0));
        unwritten.promote(new);
        assert_eq!(unwritten.current.as_ref().map(|r| r.moments[0].object), Some(1));
        assert!(unwritten.pending.is_none() && unwritten.before.is_none());
        unwritten.stage(new, Record { map_digest: sha256_hex(new), ..Record::default() });
        assert!(unwritten.for_digest(&sha256_hex(new)).is_some_and(|r| r.moments[0].object == 1), "the same bytes keep the old record until promoted");
        unwritten.promote(new);
        assert!(unwritten.is_empty());
        assert_eq!(unwritten.times(), Vec::<f64>::new());
    }

    #[test]
    fn the_record_round_trips_and_a_body_list_may_be_missing() {
        let mut unwritten = Unwritten::default();
        unwritten.set_current(record(b"map", 41));
        let json = serde_json::to_value(&unwritten).unwrap();
        assert_eq!(json["current"]["moments"][0]["hits"][1]["lane"], "Skin|drum-hitclap");
        assert_eq!(serde_json::from_value::<Unwritten>(json).unwrap(), unwritten);
        let old: Unwritten = serde_json::from_str(r#"{"current":{"map_digest":"x","moments":[]}}"#).unwrap();
        assert!(old.is_empty());
        assert_eq!(clock(62_500.4), "01:02.500");
    }
}
