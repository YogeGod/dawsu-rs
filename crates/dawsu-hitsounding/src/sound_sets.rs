//! Sound sets and their snapshot in a mapset (hitsound domain design doc Responsibilities 15 and
//! 16, invariant 11; the last-four-parts plan, part D), after the old app's `SoundSets.cs` at
//! c81774e. A sound set is a named combination of samples with their relative volumes; its
//! `.dawsuset` file carries the audio. Placing it plays one hit per member, worked out from the set
//! each time, so a later edit to the set reaches every placement; keeping the placements and their
//! links on hits is the edit session's (M3). The module reads and writes no file: it plans the
//! snapshot's files, and `dawsu-application` writes them.

use crate::drums::{base64_decode, base64_encode, check_id, utf16_len};
use crate::rules::read_sample_file_name;
use dawsu_foundation::{SampleIdentity, Sound};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashSet};
use std::fmt;

/// A sound set.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundSet {
    pub id: String,
    pub name: String,
    pub members: Vec<SetMember>,
    /// The id of the set's file in the mapper's library, if it came from there.
    pub library_id: Option<String>,
    /// Fields of the file this port does not use, kept so a written file loses nothing.
    pub kept: Kept,
}

/// One sample of a sound set.
#[derive(Debug, Clone, PartialEq)]
pub struct SetMember {
    pub id: String,
    /// The lane it was captured from, as a lane key (`Source|Name`).
    pub original_key: String,
    /// The lane it plays: after a snapshot, its file in the mapset.
    pub lane: SetLane,
    /// Its role, `<set>-<hitsound>` ("drum-hitnormal"), which names its snapshot file.
    pub slot: String,
    /// Its volume relative to the set, 0-100.
    pub volume: i32,
    /// The sample's own bytes, frozen when it was captured.
    pub audio: Vec<u8>,
    pub kept: Kept,
}

/// A member's lane as the old app writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct SetLane {
    pub source: SetSource,
    pub name: String,
    pub missing: bool,
    pub kept: Kept,
}

/// The old app's `LaneSource`, written as its number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetSource {
    /// 0: a file in the mapset folder.
    File,
    /// 1: a sound of the skin.
    Skin,
    /// 2: a storyboard's file; never allowed in a set.
    Storyboard,
}

/// Fields read from a file that the port does not use, written back unchanged.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Kept(serde_json::Map<String, serde_json::Value>);

/// Why a sound set or its file was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetRefusal {
    TooLarge,
    NotThisFormat(String),
    Invalid(String),
    /// A member's file has an extension osu! does not play.
    UnsupportedExtension(String),
}

impl fmt::Display for SetRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetRefusal::TooLarge => f.write_str("the sound set file is over 96 MB"),
            SetRefusal::NotThisFormat(why) | SetRefusal::Invalid(why) => f.write_str(why),
            SetRefusal::UnsupportedExtension(name) => write!(f, "{name} is not a .wav, .mp3 or .ogg file"),
        }
    }
}

fn invalid(why: impl Into<String>) -> SetRefusal {
    SetRefusal::Invalid(why.into())
}

const MAX_FILE_BYTES: usize = 96 * 1024 * 1024;
const MAX_MEMBER_AUDIO: usize = 32 * 1024 * 1024;
const MAX_SET_AUDIO: usize = 64 * 1024 * 1024;
const SETS: [&str; 3] = ["normal", "soft", "drum"];
const HITSOUNDS: [&str; 4] = ["hitnormal", "hitwhistle", "hitfinish", "hitclap"];

impl SetLane {
    /// The lane key the old app stores: `BeatmapFile|name`, `Skin|name` or `StoryboardFile|name`.
    pub fn key(&self) -> String {
        let source = match self.source {
            SetSource::File => "BeatmapFile",
            SetSource::Skin => "Skin",
            SetSource::Storyboard => "StoryboardFile",
        };
        format!("{source}|{}", self.name)
    }

    /// The sample the lane's name stands for, if it follows the `<set>-<sound><index>` pattern.
    fn sample(&self) -> Option<SampleIdentity> {
        match self.source {
            SetSource::Skin => read_sample_file_name(&format!("{}.wav", self.name)),
            _ => read_sample_file_name(&self.name),
        }
    }
}

impl SoundSet {
    /// The old app's `SoundSet.Validate`: a GUID id; a name of 1 to 128 characters; 1 to 128
    /// members, each with its own GUID, a lane that is not missing and not a storyboard file, a
    /// name of 1 to 1024 characters inside the folder, a volume of 0-100, 1 byte to 32 MiB of audio,
    /// and a slot that is one of the 12 hitsound roles and matches the lane's sample name (or is
    /// `normal-hitnormal` for a name that is no sample name); at most 64 MiB of audio in all.
    pub fn validate(&self) -> Result<(), SetRefusal> {
        let id = |id: &str, what: &str| check_id(id, what).map_err(|e| invalid(e.to_string()));
        id(&self.id, "the sound set")?;
        if self.name.trim().is_empty() || utf16_len(&self.name) > 128 || self.members.is_empty() || self.members.len() > 128 {
            return Err(invalid("a sound set needs a name and 1-128 sounds"));
        }
        let mut ids = HashSet::new();
        let mut total = 0usize;
        for m in &self.members {
            id(&m.id, "a sound")?;
            if !ids.insert(&m.id) {
                return Err(invalid(format!("sound id {} is used twice", m.id)));
            }
            let bad = |why: &str| Err(invalid(format!("{} in \"{}\": {why}", m.lane.name, self.name)));
            if m.lane.missing || m.lane.source == SetSource::Storyboard {
                return bad("a missing or storyboard sound");
            }
            if m.lane.name.is_empty() || utf16_len(&m.lane.name) > 1024 || utf16_len(&m.original_key) > 1024 || dawsu_osu_format::check_map_path(&m.lane.name).is_err() {
                return bad("not a file name inside the mapset");
            }
            if !(0..=100).contains(&m.volume) || m.audio.is_empty() || m.audio.len() > MAX_MEMBER_AUDIO {
                return bad("a volume outside 0-100 or audio that is empty or over 32 MB");
            }
            let wanted = match m.lane.sample() {
                Some(SampleIdentity::Standard { set, sound, .. }) => match sound {
                    Sound::Hit(_) => format!("{}-{}", crate::rules::set_word(set), crate::rules::sound_word(sound)),
                    Sound::Slider(_) => return bad("a slider sound"),
                },
                _ => "normal-hitnormal".to_string(),
            };
            if m.slot != wanted || !SETS.iter().any(|s| HITSOUNDS.iter().any(|h| m.slot == format!("{s}-{h}"))) {
                return bad(&format!("its role {} does not match its name", m.slot));
            }
            total += m.audio.len();
        }
        if total > MAX_SET_AUDIO {
            return Err(invalid("a sound set holds at most 64 MB of audio"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- the .dawsuset file

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SetJson {
    id: Option<String>,
    name: Option<String>,
    members: Option<Vec<Option<MemberJson>>>,
    #[serde(default)]
    library_id: Option<String>,
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MemberJson {
    id: Option<String>,
    original_key: Option<String>,
    lane: Option<LaneJson>,
    slot: Option<String>,
    volume: Option<i32>,
    audio: Option<String>,
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LaneJson {
    source: Option<i64>,
    name: Option<String>,
    #[serde(default)]
    missing: bool,
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

/// Reads a `.dawsuset` file's text, the old app's included, and checks it as the old app does.
pub fn read_dawsuset(text: &str) -> Result<SoundSet, SetRefusal> {
    if text.len() > MAX_FILE_BYTES {
        return Err(SetRefusal::TooLarge);
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let json: SetJson = serde_json::from_str(text).map_err(|e| SetRefusal::NotThisFormat(format!("this is not a sound set file (line {}, column {})", e.line(), e.column())))?;
    let missing = |what: &str| invalid(format!("the sound set has no {what}"));
    let members = json
        .members
        .ok_or_else(|| missing("sounds"))?
        .into_iter()
        .map(|m| {
            let m = m.ok_or_else(|| missing("sound"))?;
            let lane = m.lane.ok_or_else(|| missing("lane"))?;
            let source = match lane.source {
                Some(0) => SetSource::File,
                Some(1) => SetSource::Skin,
                Some(2) => SetSource::Storyboard,
                _ => return Err(invalid("a sound's lane has an unknown source")),
            };
            let audio = m.audio.as_deref().and_then(base64_decode).ok_or_else(|| invalid("a sound's audio is not base64"))?;
            Ok(SetMember {
                id: m.id.ok_or_else(|| missing("sound id"))?,
                original_key: m.original_key.ok_or_else(|| missing("original lane"))?,
                lane: SetLane { source, name: lane.name.ok_or_else(|| missing("lane name"))?, missing: lane.missing, kept: Kept(lane.rest) },
                slot: m.slot.ok_or_else(|| missing("role"))?,
                volume: m.volume.ok_or_else(|| missing("volume"))?,
                audio,
                kept: Kept(m.rest),
            })
        })
        .collect::<Result<Vec<_>, SetRefusal>>()?;
    let set = SoundSet { id: json.id.ok_or_else(|| missing("id"))?, name: json.name.ok_or_else(|| missing("name"))?, members, library_id: json.library_id, kept: Kept(json.rest) };
    set.validate()?;
    Ok(set)
}

/// Writes a sound set as a `.dawsuset` file's text, with every field it was read with. Refuses a
/// set the old app would refuse, so a written file always reads back.
pub fn write_dawsuset(set: &SoundSet) -> Result<String, SetRefusal> {
    set.validate()?;
    let json = SetJson {
        id: Some(set.id.clone()),
        name: Some(set.name.clone()),
        members: Some(
            set.members
                .iter()
                .map(|m| {
                    Some(MemberJson {
                        id: Some(m.id.clone()),
                        original_key: Some(m.original_key.clone()),
                        lane: Some(LaneJson {
                            source: Some(match m.lane.source {
                                SetSource::File => 0,
                                SetSource::Skin => 1,
                                SetSource::Storyboard => 2,
                            }),
                            name: Some(m.lane.name.clone()),
                            missing: m.lane.missing,
                            rest: m.lane.kept.0.clone(),
                        }),
                        slot: Some(m.slot.clone()),
                        volume: Some(m.volume),
                        audio: Some(base64_encode(&m.audio)),
                        rest: m.kept.0.clone(),
                    })
                })
                .collect(),
        ),
        library_id: set.library_id.clone(),
        rest: set.kept.0.clone(),
    };
    let text = serde_json::to_string_pretty(&json).expect("a sound set always serialises");
    if text.len() > MAX_FILE_BYTES {
        return Err(SetRefusal::TooLarge);
    }
    Ok(text)
}

// ---------------------------------------------------------------- capturing a selection

/// One selected hit to save in a new set: the lane it plays, the old app's key for it, its volume
/// and the bytes of its sample.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedHit {
    pub lane: SetLane,
    pub original_key: String,
    pub volume: i32,
    pub audio: Vec<u8>,
}

/// The old app's `SoundSetFiles.Capture`: a new set named `name` (trimmed) with one member per
/// hit, each with a fresh id from `new_id`, its lane, volume and audio, and the role its sample
/// name gives (`normal-hitnormal` for a file with no sample name). Slider sounds are refused, and
/// the set must pass `validate`.
pub fn capture(new_id: &mut dyn FnMut() -> String, name: &str, hits: Vec<CapturedHit>) -> Result<SoundSet, SetRefusal> {
    let id = new_id();
    let members = hits
        .into_iter()
        .map(|hit| {
            let slot = match hit.lane.sample() {
                Some(SampleIdentity::Standard { set, sound: sound @ Sound::Hit(_), .. }) => format!("{}-{}", crate::rules::set_word(set), crate::rules::sound_word(sound)),
                Some(SampleIdentity::Standard { sound: Sound::Slider(_), .. }) => return Err(invalid("use hitnormal, whistle, finish or clap sounds in a sound set")),
                _ => "normal-hitnormal".to_string(),
            };
            Ok(SetMember { id: new_id(), original_key: hit.original_key, lane: hit.lane, slot, volume: hit.volume, audio: hit.audio, kept: Kept::default() })
        })
        .collect::<Result<Vec<_>, SetRefusal>>()?;
    let set = SoundSet { id, name: name.trim().to_string(), members, library_id: None, kept: Kept::default() };
    set.validate()?;
    Ok(set)
}

// ---------------------------------------------------------------- placing (Responsibility 15)

/// The hits one placement of `set` plays at `gain` percent: each member's lane key and volume,
/// `volume × gain / 100` rounded half to even (the old app's `Math.Round`) and clamped to 0-100.
pub fn placement_hits(set: &SoundSet, gain: i32) -> Vec<(String, i32)> {
    set.members.iter().map(|m| (m.lane.key(), (f64::from(m.volume) * f64::from(gain) / 100.0).round_ties_even().clamp(0.0, 100.0) as i32)).collect()
}

/// One placement of a sound set on a moment: the link the panel shows and an update follows. It
/// holds no audio; the set comes from the library and its files from the mapset snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SoundSetPlacement {
    pub id: u64,
    pub set_id: String,
    /// The set's version (its file's checksum) when placed or last updated.
    pub set_digest: String,
    /// A newer version the mapper chose to keep this placement off.
    pub accepted_digest: Option<String>,
    pub moment: crate::MomentId,
    pub gain: i32,
    pub applied: Vec<Applied>,
}

/// One member's hit as placed: `added` is false when the moment already had the lane, and such a
/// hit is never touched by an update or unlink.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub member_id: String,
    pub lane: SampleIdentity,
    pub volume: i32,
    pub added: bool,
}

// ---------------------------------------------------------------- the snapshot (Responsibility 16)

/// A planned snapshot: the set with each member on its file in the mapset, and the files to write
/// (name in the mapset folder, bytes). `dawsu-application` writes them, each as a new file.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub set: SoundSet,
    pub files: Vec<(String, Vec<u8>)>,
}

/// Plans copying `set`'s samples into a mapset as a snapshot that keeps each sample's role
/// (old `SoundSetFiles.Install`). Every member shares one custom index, since an object plays one
/// index for its hitnormal and additions: the lowest from 2 that no difficulty or storyboard uses
/// (`used_indexes`) and no folder file's name holds. A role the set holds twice can't share that
/// file name, so each repeat takes the next free index. A member is `<slot><index><extension>`,
/// with `.wav` for a skin sound and else the source's extension in lower case.
///
/// The snapshot is all-or-nothing: `previous` (the last snapshot of this set in the mapset) is
/// reused only when it holds every member with the same role and audio and each of its files
/// still holds that audio (`existing` reads a folder file's bytes); a file whose bytes merely match
/// is never taken. Otherwise the whole set is copied again at a new index. So placing a set again
/// needs no new file (invariant 11), and later placements play the snapshot, never the outside
/// source.
pub fn plan_snapshot(set: &SoundSet, folder: &[String], used_indexes: &BTreeSet<u32>, previous: Option<&SoundSet>, existing: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Snapshot, SetRefusal> {
    set.validate()?;
    if let Some(previous) = previous.filter(|p| p.members.len() == set.members.len()) {
        let reused: Option<Vec<SetMember>> = set
            .members
            .iter()
            .map(|m| {
                let q = previous.members.iter().find(|q| q.id == m.id && q.slot == m.slot && q.audio == m.audio && q.lane.source == SetSource::File)?;
                let on_disk = dawsu_osu_format::find_case_insensitive(folder, &q.lane.name).and_then(existing)?;
                (on_disk == m.audio).then(|| SetMember { lane: q.lane.clone(), ..m.clone() })
            })
            .collect();
        if let Some(members) = reused {
            return Ok(Snapshot { set: SoundSet { members, ..set.clone() }, files: Vec::new() });
        }
    }
    let mut occupied: BTreeSet<u32> = used_indexes.clone();
    occupied.extend(folder.iter().filter_map(|f| file_index(f)));
    let lowest_free = |occupied: &BTreeSet<u32>| (2u32..).find(|i| !occupied.contains(i)).expect("an index is free");
    let shared = lowest_free(&occupied);
    occupied.insert(shared);
    let mut slots = HashSet::new();
    let mut files = Vec::new();
    let mut members = Vec::new();
    for m in &set.members {
        let extension = match m.lane.source {
            SetSource::Skin => ".wav".to_string(),
            _ => m.lane.name.rfind('.').map(|i| m.lane.name[i..].to_lowercase()).unwrap_or_default(),
        };
        if ![".wav", ".mp3", ".ogg"].contains(&extension.as_str()) {
            return Err(SetRefusal::UnsupportedExtension(m.lane.name.clone()));
        }
        let index = if slots.insert(m.slot.as_str()) {
            shared
        } else {
            let index = lowest_free(&occupied);
            occupied.insert(index);
            index
        };
        let name = format!("{}{index}{extension}", m.slot);
        files.push((name.clone(), m.audio.clone()));
        members.push(SetMember { lane: SetLane { source: SetSource::File, name, missing: false, kept: Kept::default() }, ..m.clone() });
    }
    Ok(Snapshot { set: SoundSet { members, ..set.clone() }, files })
}

/// The custom index a folder file's name holds, whatever its extension ("soft-hitclap2.ogg" is 2).
fn file_index(name: &str) -> Option<u32> {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    match read_sample_file_name(&format!("{stem}.wav"))? {
        SampleIdentity::Standard { index, .. } => Some(index.get()),
        SampleIdentity::File(_) => None,
    }
}
