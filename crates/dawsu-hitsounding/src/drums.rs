//! Drum maps (hitsound domain design doc Responsibilities 17-20a, invariants 9 and 10; the
//! drum-map plan). Ported from the old app's `OsuHitsound.Lanes` at c81774e: `DrumMaps.cs`,
//! `DrumKitMapping.cs`, `GgdMapFile.cs`, `DrumRender.cs`, and the lane-to-note map and view rows of
//! `MainWindow.DrumVst.cs` and `ArrangeModel.DrumMaps.cs`. The module reads no file: it is given
//! a file's text and gives text back. A file that cannot be read is a refusal, an expected outcome
//! (ADR 0034 point 2), never a half-read map.

use crate::model::{Hitsounds, Lane, LaneSource, MomentId};
use crate::rules::{played_volume, read_sample_file_name, set_word, sound_word};
use dawsu_foundation::{Hitsound, SampleIdentity, SongTime, Sound};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fmt;

// ---------------------------------------------------------------- the model (Responsibility 17)

/// One named drum map: a flat list of instrument rows, each assigned to one sample lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrumMap {
    /// A GUID in its usual text form, as the old app writes it.
    pub id: String,
    pub name: String,
    /// In the order the drum-map view shows them. Several rows on one lane are aliases.
    pub rows: Vec<DrumRow>,
    /// Sample audio a portable map carries; only a `.dawsudrum` file holds any.
    pub audio: Vec<DrumAudio>,
}

/// One instrument row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrumRow {
    pub id: String,
    /// The instrument's name; a display name only, never a lane's name.
    pub name: String,
    /// The lane it plays, as a lane key ([`lane_key`]); `None` is unassigned.
    pub lane: Option<String>,
    /// The drum-set sample slot the lane fills, as a lane key.
    pub slot: Option<String>,
    /// The instrument note it renders with; `None` leaves the lane out of a render.
    pub midi_note: Option<u8>,
}

/// One sample's audio in a portable drum map, base64 in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrumAudio {
    pub lane: String,
    pub slot: String,
    pub bytes: Vec<u8>,
}

/// A mapset's drum maps, shared by its difficulties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrumMaps {
    pub maps: Vec<DrumMap>,
    /// The id of the map in use.
    pub active: Option<String>,
    /// Whether the drum-map view is on.
    pub enabled: bool,
}

/// Why a drum file or drum map was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrumFileRefusal {
    /// The text is over the limit for its kind of file.
    TooLarge { limit_bytes: usize },
    /// The text is not the kind of file expected: not JSON, not XML, or another format.
    NotThisFormat(String),
    /// The file reads, but its content breaks a rule the old app checks.
    Invalid(String),
}

impl fmt::Display for DrumFileRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrumFileRefusal::TooLarge { limit_bytes } => write!(f, "the file is over {} MB", limit_bytes / (1024 * 1024)),
            DrumFileRefusal::NotThisFormat(why) | DrumFileRefusal::Invalid(why) => f.write_str(why),
        }
    }
}

fn invalid(why: impl Into<String>) -> DrumFileRefusal {
    DrumFileRefusal::Invalid(why.into())
}

const MAX_NAME_UTF16: usize = 128;
const MAX_ROWS: usize = 4096;
const MAX_AUDIO: usize = 512;
const MAX_AUDIO_BYTES: usize = 32 * 1024 * 1024;
const MAX_AUDIO_TOTAL: usize = 64 * 1024 * 1024;
const MAX_KEY_UTF16: usize = 1100;
const MAX_MAPS: usize = 64;
const MAX_MAPSET_ROWS: usize = 16384;
/// A portable `.dawsudrum` file's limit (old `DrumMapStore.Export`/`Import`).
pub const MAX_DAWSUDRUM_BYTES: usize = 96 * 1024 * 1024;

impl DrumMap {
    /// The old app's `DrumMapPreset.Validate`: a GUID id, a name of 1 to 128 UTF-16 units, at most
    /// 4096 rows and 512 audio entries, unique row ids, notes 0-127, keys of the four hitsounds
    /// inside the folder, and at most 32 MiB of audio each and 64 MiB in all.
    pub fn validate(&self) -> Result<(), DrumFileRefusal> {
        check_id(&self.id, "the drum map")?;
        check_name(&self.name, "the drum map")?;
        if self.rows.len() > MAX_ROWS || self.audio.len() > MAX_AUDIO {
            return Err(invalid("a drum map holds at most 4096 rows and 512 sample files"));
        }
        let mut ids = HashSet::new();
        for r in &self.rows {
            check_id(&r.id, &format!("row \"{}\"", r.name))?;
            check_name(&r.name, "a row")?;
            if !ids.insert(&r.id) {
                return Err(invalid(format!("row id {} is used twice", r.id)));
            }
            if r.midi_note.is_some_and(|n| n > 127) {
                return Err(invalid(format!("row \"{}\" has a MIDI note over 127", r.name)));
            }
            for key in [&r.lane, &r.slot].into_iter().flatten() {
                check_key(key)?;
            }
        }
        let mut lanes = HashSet::new();
        let mut total = 0usize;
        for a in &self.audio {
            check_key(&a.lane)?;
            check_key(&a.slot)?;
            if a.bytes.is_empty() || a.bytes.len() > MAX_AUDIO_BYTES {
                return Err(invalid(format!("the audio for {} is empty or over 32 MB", a.lane)));
            }
            if !lanes.insert(&a.lane) {
                return Err(invalid(format!("{} has audio twice", a.lane)));
            }
            total += a.bytes.len();
        }
        if total > MAX_AUDIO_TOTAL {
            return Err(invalid("a drum map holds at most 64 MB of audio"));
        }
        Ok(())
    }
}

impl DrumMaps {
    /// The old app's `DrumMapCollection.Validate`: at most 64 maps and 16,384 rows in all, unique
    /// map ids, the active id naming a map, and every map valid.
    pub fn validate(&self) -> Result<(), DrumFileRefusal> {
        if self.maps.len() > MAX_MAPS || self.maps.iter().map(|m| m.rows.len()).sum::<usize>() > MAX_MAPSET_ROWS {
            return Err(invalid("a mapset holds at most 64 drum maps and 16,384 rows"));
        }
        let mut ids = HashSet::new();
        if !self.maps.iter().all(|m| ids.insert(&m.id)) {
            return Err(invalid("a drum map id is used twice"));
        }
        if self.active.as_ref().is_some_and(|a| !ids.contains(a)) {
            return Err(invalid("the active drum map is not in the mapset"));
        }
        self.maps.iter().try_for_each(DrumMap::validate)
    }
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// A name the old app accepts: not blank and at most 128 UTF-16 units (.NET's string length).
fn check_name(name: &str, what: &str) -> Result<(), DrumFileRefusal> {
    if name.trim().is_empty() || utf16_len(name) > MAX_NAME_UTF16 {
        return Err(invalid(format!("{what} needs a name of 1 to 128 characters")));
    }
    Ok(())
}

/// A GUID in its usual 8-4-4-4-12 text form and not the nil GUID: the form System.Text.Json reads.
fn check_id(id: &str, what: &str) -> Result<(), DrumFileRefusal> {
    let groups: Vec<&str> = id.split('-').collect();
    let shape = groups.len() == 5 && groups.iter().zip([8, 4, 4, 4, 12]).all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()));
    if !shape || id.bytes().all(|b| b == b'0' || b == b'-') {
        return Err(invalid(format!("{what} has an id that is not a GUID")));
    }
    Ok(())
}

// ---------------------------------------------------------------- lane keys

/// The old app's text for a lane, `Source|Name`, which drum-map rows store: `Skin|drum-hitnormal`
/// for a skin sound, `BeatmapFile|drum-hitclap2.wav` for a file in the folder. A lane whose file
/// is missing from the folder keeps the key the old app gave it: a named file stays
/// `BeatmapFile|<name>`. A numbered file that is missing is a lane of its own here (Responsibility
/// 3), which the old app folded into the skin's sound; it gets `Skin|<set>-<sound><index>`, which
/// no old row names (an assumption of the drum-map plan).
pub fn lane_key(lane: &Lane) -> String {
    match (&lane.source, &lane.identity) {
        (LaneSource::File(name), _) | (LaneSource::Skin, SampleIdentity::File(name)) => format!("BeatmapFile|{name}"),
        (LaneSource::Skin, SampleIdentity::Standard { set, sound, index }) => match index.get() {
            0 => format!("Skin|{}-{}", set_word(*set), sound_word(*sound)),
            n => format!("Skin|{}-{}{n}", set_word(*set), sound_word(*sound)),
        },
    }
}

/// The old app's `ValidKey`: a known source other than a storyboard file, a name inside the folder
/// (`check_map_path`, which follows the old `MapFolder.IsInside`), and, when the name is a sample's
/// name, one of the four hitsounds.
fn check_key(key: &str) -> Result<(), DrumFileRefusal> {
    let bad = || Err(invalid(format!("\"{key}\" is not a lane dawsu! can assign")));
    if utf16_len(key) > MAX_KEY_UTF16 {
        return bad();
    }
    let Some((source, name)) = key.split_once('|') else { return bad() };
    // A skin sound's name has no extension; read it as its file would be named.
    let sample = match source {
        "Skin" => read_sample_file_name(&format!("{name}.wav")),
        "BeatmapFile" => read_sample_file_name(name),
        _ => return bad(),
    };
    if dawsu_osu_format::check_map_path(name).is_err() {
        return bad();
    }
    match sample {
        Some(SampleIdentity::Standard { sound: Sound::Slider(_), .. }) => bad(),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------- the .dawsudrum file (Responsibility 18)

/// The General MIDI note an instrument name starts with (old `DrumMapMidi.DefaultNote`); a row in
/// an old file with no note field gets it.
pub fn default_note(name: &str) -> Option<u8> {
    Some(match name {
        "Kick" => 36,
        "Snare" | "Ghost Notes" => 38,
        "T1" => 50,
        "T2" => 48,
        "T3" => 45,
        "T4" => 41,
        "Tight Tip" | "Tight Edge" | "Closed Tip" | "Closed Edge" => 42,
        "Hi-Hat Open 1" | "Hi-Hat Open 2" | "Hi-Hat Open 3" => 46,
        "Ride Bell" => 53,
        "Bow Ride (Low)" => 51,
        "Bow Ride (High)" | "Ride Crash" => 59,
        "Right Crash" => 49,
        "Left Crash" => 57,
        "China" | "Stack" => 52,
        "Mini Stack Left" | "Mini Stack Right" | "Splash" => 55,
        _ => return None,
    })
}

// The file's JSON as System.Text.Json writes it: PascalCase names, nulls written out. Reading
// ignores unknown fields, as System.Text.Json does.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MapJson {
    id: Option<String>,
    name: Option<String>,
    rows: Option<Vec<Option<RowJson>>>,
    #[serde(default = "no_audio")]
    audio: Option<Vec<Option<AudioJson>>>,
}

fn no_audio() -> Option<Vec<Option<AudioJson>>> {
    Some(Vec::new())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RowJson {
    id: Option<String>,
    name: Option<String>,
    lane_key: Option<String>,
    slot_key: Option<String>,
    /// Outer `None`: the field is missing (the row takes its default note); `Some(None)`: null.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    midi_note: Option<Option<i64>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AudioJson {
    lane_key: Option<String>,
    slot_key: Option<String>,
    bytes: Option<String>,
}

fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<i64>>, D::Error> {
    Option::<i64>::deserialize(d).map(Some)
}

/// Reads a portable `.dawsudrum` file's text, the files the old app wrote included, and checks it
/// as the old app does (invariant 10).
pub fn read_dawsudrum(text: &str) -> Result<DrumMap, DrumFileRefusal> {
    if text.len() > MAX_DAWSUDRUM_BYTES {
        return Err(DrumFileRefusal::TooLarge { limit_bytes: MAX_DAWSUDRUM_BYTES });
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let json: MapJson = serde_json::from_str(text).map_err(|e| DrumFileRefusal::NotThisFormat(format!("this is not a drum map file (line {}, column {})", e.line(), e.column())))?;
    let missing = |what: &str| invalid(format!("the drum map has no {what}"));
    let rows = json
        .rows
        .ok_or_else(|| missing("rows"))?
        .into_iter()
        .map(|r| {
            let r = r.ok_or_else(|| missing("row"))?;
            let name = r.name.ok_or_else(|| missing("row name"))?;
            let midi_note = match r.midi_note {
                None => default_note(&name),
                Some(None) => None,
                Some(Some(n)) => Some(u8::try_from(n).ok().filter(|n| *n <= 127).ok_or_else(|| invalid(format!("row \"{name}\" has a MIDI note outside 0-127")))?),
            };
            Ok(DrumRow { id: r.id.ok_or_else(|| missing("row id"))?, name, lane: r.lane_key, slot: r.slot_key, midi_note })
        })
        .collect::<Result<Vec<_>, DrumFileRefusal>>()?;
    let audio = json
        .audio
        .ok_or_else(|| missing("audio list"))?
        .into_iter()
        .map(|a| {
            let a = a.ok_or_else(|| missing("audio entry"))?;
            let (lane, slot) = (a.lane_key.ok_or_else(|| missing("audio lane"))?, a.slot_key.ok_or_else(|| missing("audio slot"))?);
            let bytes = a.bytes.as_deref().and_then(base64_decode).ok_or_else(|| invalid(format!("the audio for {lane} is not base64")))?;
            Ok(DrumAudio { lane, slot, bytes })
        })
        .collect::<Result<Vec<_>, DrumFileRefusal>>()?;
    let map = DrumMap { id: json.id.ok_or_else(|| missing("id"))?, name: json.name.ok_or_else(|| missing("name"))?, rows, audio };
    map.validate()?;
    Ok(map)
}

/// Writes a drum map as a portable `.dawsudrum` file's text, indented as the old app writes it.
/// Refuses a map the old app would refuse, so a written file always reads back.
pub fn write_dawsudrum(map: &DrumMap) -> Result<String, DrumFileRefusal> {
    map.validate()?;
    let json = MapJson {
        id: Some(map.id.clone()),
        name: Some(map.name.clone()),
        rows: Some(map.rows.iter().map(|r| Some(RowJson { id: Some(r.id.clone()), name: Some(r.name.clone()), lane_key: r.lane.clone(), slot_key: r.slot.clone(), midi_note: Some(r.midi_note.map(i64::from)) })).collect()),
        audio: Some(map.audio.iter().map(|a| Some(AudioJson { lane_key: Some(a.lane.clone()), slot_key: Some(a.slot.clone()), bytes: Some(base64_encode(&a.bytes)) })).collect()),
    };
    let text = serde_json::to_string_pretty(&json).expect("a drum map always serialises");
    if text.len() > MAX_DAWSUDRUM_BYTES {
        return Err(DrumFileRefusal::TooLarge { limit_bytes: MAX_DAWSUDRUM_BYTES });
    }
    Ok(text)
}

/// The old app's two starter kits, every lane unassigned; the first is active and the view is off.
pub fn default_drum_maps(new_id: &mut dyn FnMut() -> String) -> DrumMaps {
    const EXPANDED: [&str; 25] = [
        "Kick", "Snare", "Ghost Notes", "T1", "T2", "T3", "T4", "Tight Tip", "Tight Edge", "Closed Tip", "Closed Edge", "Hi-Hat Open 1", "Hi-Hat Open 2", "Hi-Hat Open 3", "Ride Bell", "Bow Ride (Low)", "Bow Ride (High)", "Right Crash",
        "Left Crash", "Ride Crash", "China", "Stack", "Mini Stack Left", "Mini Stack Right", "Splash",
    ];
    const SIMPLE: [&str; 18] = ["Kick", "Snare", "T1", "T2", "T3", "T4", "Tight Tip", "Tight Edge", "Closed Tip", "Closed Edge", "Hi-Hat Open 1", "Hi-Hat Open 2", "Hi-Hat Open 3", "Ride Bell", "Right Crash", "Left Crash", "Ride Crash", "China"];
    let mut make = |name: &str, names: &[&str]| DrumMap {
        id: new_id(),
        name: name.into(),
        rows: names.iter().map(|n| DrumRow { id: new_id(), name: (*n).into(), lane: None, slot: None, midi_note: default_note(n) }).collect(),
        audio: Vec::new(),
    };
    let maps = vec![make("Yoge Expanded Kit", &EXPANDED), make("Yoge Simple Kit", &SIMPLE)];
    DrumMaps { active: Some(maps[0].id.clone()), maps, enabled: false }
}

// ---------------------------------------------------------------- rows from a kit (Responsibility 19)

/// One articulation as a drum instrument lists it: its group ("SNARE"), its item ("Flam",
/// "Rack Tom 1: Hit") and its note, `None` when the instrument leaves it unmapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Articulation {
    pub group: String,
    pub item: String,
    pub note: Option<u8>,
}

/// An instrument row made from a kit: the row's name, its note and the osu! hitsound it goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitRow {
    pub name: String,
    pub note: u8,
    pub sound: Hitsound,
}

/// Articulations from the names an instrument reports for its notes, in note order; notes
/// outside 0-127 and blank names are left out.
pub fn kit_from_note_names(names: &BTreeMap<i32, String>) -> Vec<Articulation> {
    names
        .iter()
        .filter(|(n, name)| (0..=127).contains(*n) && !name.trim().is_empty())
        .map(|(n, name)| Articulation { group: String::new(), item: name.trim().into(), note: Some(*n as u8) })
        .collect()
}

pub fn is_choke(a: &Articulation) -> bool {
    a.item.to_lowercase().contains("choke")
}

/// The row's name: "Hit" alone is its group ("Kick"), "Choke" alone is "China Choke", "Rack Tom
/// 1: Hit" is "Rack Tom 1", "Main Crash L: Choke" is "Main Crash L Choke", and any other
/// articulation keeps its own name ("Flam").
pub fn label(a: &Articulation) -> String {
    let item = a.item.trim();
    if item.eq_ignore_ascii_case("hit") {
        return title(&a.group);
    }
    if item.eq_ignore_ascii_case("choke") {
        return format!("{} Choke", title(&a.group)).trim().to_string();
    }
    if let Some(colon) = item.rfind(':').filter(|&c| c > 0) {
        let (head, tail) = (item[..colon].trim(), item[colon + 1..].trim());
        return if tail.eq_ignore_ascii_case("hit") { head.to_string() } else { format!("{head} {tail}") };
    }
    item.to_string()
}

/// The hitsound an instrument goes to, as hitsounders usually map a kit: kick and toms on
/// hitnormal, the snare and its articulations on hitclap, hats and ride on hitwhistle, crashes,
/// splashes, china and stack on hitfinish.
pub fn kit_sound(a: &Articulation) -> Hitsound {
    let text = format!("{} {}", a.group, a.item).to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    if has(&["crash", "splash", "china", "stack", "cymbal"]) {
        Hitsound::Finish
    } else if has(&["hat", "ride", "bell", "tip", "edge", "open", "pedal", "cc"]) {
        Hitsound::Whistle
    } else if has(&["snare", "flam", "ruff", "rim", "sidestick", "side stick", "wires", "stick", "clap", "aux"]) {
        Hitsound::Clap
    } else {
        Hitsound::Normal
    }
}

/// The rows to make, in the kit's order: mapped notes only, chokes only when asked for, one row
/// per name (a repeated name gets its group in front, or is left out).
pub fn kit_rows(kit: &[Articulation], include_chokes: bool) -> Vec<KitRow> {
    let mut rows = Vec::new();
    let mut names = HashSet::new();
    for a in kit {
        let Some(note) = a.note.filter(|n| *n <= 127) else { continue };
        if !include_chokes && is_choke(a) {
            continue;
        }
        let mut name = label(a);
        if name.is_empty() || utf16_len(&name) > MAX_NAME_UTF16 {
            continue;
        }
        if !names.insert(name.to_lowercase()) {
            if a.group.is_empty() {
                continue;
            }
            name = format!("{} {name}", title(&a.group));
            if !names.insert(name.to_lowercase()) {
                continue;
            }
        }
        rows.push(KitRow { name, note, sound: kit_sound(a) });
    }
    rows
}

/// The kit row an existing drum-map row stands for: the same name; else the starter names' usual
/// kit names (T1, Right Crash, Ghost Notes...); else the same words in any order, ignoring
/// "hi-hat" (Tight Tip and Tip Tight, Hi-Hat Open 1 and Open 1). `None` when the kit has nothing.
pub fn match_row<'a>(row_name: &str, kit: &'a [KitRow]) -> Option<&'a KitRow> {
    let named = |name: &str| kit.iter().find(|k| k.name.to_lowercase() == name.to_lowercase());
    let row = row_name.trim();
    if let Some(k) = named(row) {
        return Some(k);
    }
    let aliases: &[&str] = match row.to_lowercase().as_str() {
        "kick" => &["Kick", "Bass Drum"],
        "ghost notes" => &["Snare"],
        "t1" => &["Rack Tom 1", "Tom 1", "High Tom"],
        "t2" => &["Rack Tom 2", "Tom 2", "Mid Tom"],
        "t3" => &["Floor Tom 1", "Tom 3", "Low Tom"],
        "t4" => &["Floor Tom 2", "Tom 4", "Floor Tom"],
        "ride bell" => &["Bell", "Ride Bell"],
        "bow ride (low)" | "bow ride (high)" => &["Ride", "Ride Bow"],
        "right crash" => &["Main Crash R", "Crash R", "Crash 2"],
        "left crash" => &["Main Crash L", "Crash L", "Crash 1"],
        "splash" => &["Splash L", "Splash C", "Splash"],
        "mini stack left" | "mini stack right" => &["Stack"],
        _ => &[],
    };
    if let Some(k) = aliases.iter().find_map(|a| named(a)) {
        return Some(k);
    }
    let words = words(row_name);
    if words.is_empty() {
        return None;
    }
    kit.iter().find(|k| self::words(&k.name) == words)
}

fn words(name: &str) -> HashSet<String> {
    name.to_lowercase().replace("hi-hat", " ").replace("hihat", " ").split([' ', '-', '(', ')']).filter(|w| !w.is_empty() && *w != "hat" && *w != "hats").map(String::from).collect()
}

fn title(group: &str) -> String {
    group
        .split(' ')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match (c.next(), w.chars().count()) {
                (Some(first), n) if n > 1 => first.to_uppercase().chain(c.as_str().to_lowercase().chars()).collect::<String>(),
                _ => w.to_uppercase(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The header line of a GGD map export (`.nka`, saved by EXPORT MAP on a GGD library's Mappings page).
pub const GGD_HEADER: &str = "%ART__articulation_map";
const GGD_MAX_BYTES: usize = 65536;

/// Which line of a GGD export is which articulation, in the order the Mappings page shows them
/// (old `GgdMapFile.Lines`, worked out from a P4 Kit "Invasion" export).
const GGD_LINES: [(usize, &str, &str); 36] = [
    (1, "KICK", "Hit"),
    (4, "SNARE", "Hit"), (3, "SNARE", "Flam"), (6, "SNARE", "Wires Off"), (5, "SNARE", "Ruff"), (2, "SNARE", "Sidestick"),
    (7, "TOMS", "Rack Tom 1: Hit"), (8, "TOMS", "Rack Tom 2: Hit"), (9, "TOMS", "Floor Tom 1: Hit"), (10, "TOMS", "Floor Tom 2: Hit"),
    (37, "AUX", "Stick Click"),
    (19, "HATS", "Tip Tight"), (13, "HATS", "Edge Tight"), (18, "HATS", "Tip Closed"), (12, "HATS", "Edge Closed"),
    (14, "HATS", "Open 1"), (15, "HATS", "Open 2"), (16, "HATS", "Open 3"), (17, "HATS", "Pedal"), (20, "HATS", "CC"),
    (21, "RIDE", "Bell"), (22, "RIDE", "Ride"),
    (30, "CHINA", "Hit"), (29, "CHINA", "Choke"),
    (24, "CRASHES", "Main Crash L: Hit"), (23, "CRASHES", "Main Crash L: Choke"),
    (26, "CRASHES", "Wide Crash L: Hit"), (25, "CRASHES", "Wide Crash L: Choke"),
    (28, "CRASHES", "Main Crash R: Hit"), (27, "CRASHES", "Main Crash R: Choke"),
    (32, "SPLASHES", "Splash L: Hit"), (31, "SPLASHES", "Splash L: Choke"),
    (34, "SPLASHES", "Splash C: Hit"), (33, "SPLASHES", "Splash C: Choke"),
    (35, "MINI HATS", "Hit"),
    (36, "STACK", "Hit"),
];

/// A GGD library's own mapping from its exported map's text: the header, then one note per line
/// (-1 is unmapped), blank lines skipped, at least one line for every articulation.
pub fn ggd_kit(text: &str) -> Result<Vec<Articulation>, DrumFileRefusal> {
    if text.len() > GGD_MAX_BYTES {
        return Err(DrumFileRefusal::TooLarge { limit_bytes: GGD_MAX_BYTES });
    }
    let lines: Vec<&str> = text.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty()).collect();
    let last = GGD_LINES.iter().map(|l| l.0).max().unwrap_or(0);
    if lines.len() < last + 2 || lines[0].trim_start_matches('\u{feff}') != GGD_HEADER {
        return Err(DrumFileRefusal::NotThisFormat("this isn't a GGD map export (EXPORT MAP on the library's Mappings page)".into()));
    }
    let notes = lines[1..]
        .iter()
        .enumerate()
        .map(|(i, l)| match l.parse::<i32>() {
            Ok(-1) => Ok(None),
            Ok(n @ 0..=127) => Ok(Some(n as u8)),
            _ => Err(invalid(format!("line {} of the GGD map isn't a MIDI note", i + 2))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GGD_LINES.iter().map(|(line, group, item)| Articulation { group: (*group).into(), item: (*item).into(), note: notes[*line] }).collect())
}

/// GGD Matt Garstka Signature Drums, mapping preset "GGD", as the instrument's Mappings page shows it.
pub fn ggd_matt_garstka() -> Vec<Articulation> {
    let a = |group: &str, item: &str, note: Option<u8>| Articulation { group: group.into(), item: item.into(), note };
    vec![
        a("KICK", "Hit", Some(36)),
        a("SNARE", "Hit", Some(37)), a("SNARE", "Flam", Some(38)), a("SNARE", "Wires Off", Some(41)), a("SNARE", "Ruff", Some(39)), a("SNARE", "Sidestick", Some(40)),
        a("TOMS", "Rack Tom 1: Hit", Some(42)), a("TOMS", "Rack Tom 2: Hit", Some(45)), a("TOMS", "Floor Tom 1: Hit", Some(43)), a("TOMS", "Floor Tom 2: Hit", Some(46)),
        a("AUX", "Stick Click", None),
        a("HATS", "Tip Tight", Some(48)), a("HATS", "Edge Tight", Some(54)), a("HATS", "Tip Closed", Some(49)), a("HATS", "Edge Closed", None),
        a("HATS", "Open 1", Some(52)), a("HATS", "Open 2", Some(53)), a("HATS", "Open 3", Some(57)), a("HATS", "Pedal", Some(55)),
        a("HATS", "Pedal Ching", None), a("HATS", "CC", None),
        a("RIDE", "Bell", Some(74)), a("RIDE", "Ride", Some(73)),
        a("CHINA", "Hit", Some(71)), a("CHINA", "Choke", Some(72)),
        a("CRASHES", "Main Crash L: Hit", Some(58)), a("CRASHES", "Main Crash L: Choke", Some(59)),
        a("CRASHES", "Wide Crash L: Hit", None), a("CRASHES", "Wide Crash L: Choke", None),
        a("CRASHES", "Main Crash R: Hit", Some(61)), a("CRASHES", "Main Crash R: Choke", Some(62)),
        a("SPLASHES", "Splash L: Hit", Some(77)), a("SPLASHES", "Splash L: Choke", Some(83)),
        a("SPLASHES", "Splash C: Hit", None), a("SPLASHES", "Splash C: Choke", None),
        a("MINI HATS", "Hit", None),
        a("STACK", "Hit", None),
    ]
}
// ---------------------------------------------------------------- Cubase .drm (Responsibility 20a)

const DRM_MAX_BYTES: usize = 16 * 1024 * 1024;

/// A child element such as `<int name="ONote" value="40"/>`.
fn named<'a, 'i>(node: roxmltree::Node<'a, 'i>, tag: &str, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    node.children().find(|c| c.is_element() && c.tag_name().name() == tag && c.attribute("name") == Some(name))
}

fn items<'a, 'i>(node: roxmltree::Node<'a, 'i>) -> Vec<roxmltree::Node<'a, 'i>> {
    node.children().filter(|c| c.is_element() && c.tag_name().name() == "item").collect()
}

/// Builds a drum map from a Cubase `.drm` file's XML (ADR 0046), as Yoge decided on 2026-11-15:
/// the rows in the Order list's order, where each Order value is a row's position in the Map
/// list (Cubase keys rows by pitch), or in the Map's own order when there is no Order list; only
/// named rows, a name being neither blank nor Cubase's placeholder `Sound <position + 1>`; each
/// with its output note (O-Note), the note that plays, and no lane. Every other field is dropped.
/// A blank map name takes `fallback_name` (the caller passes the file's name).
pub fn read_drm(text: &str, fallback_name: &str, new_id: &mut dyn FnMut() -> String) -> Result<DrumMap, DrumFileRefusal> {
    if text.len() > DRM_MAX_BYTES {
        return Err(DrumFileRefusal::TooLarge { limit_bytes: DRM_MAX_BYTES });
    }
    let not_drm = |why: String| DrumFileRefusal::NotThisFormat(format!("this isn't a Cubase drum map: {why}"));
    // DTDs are refused (roxmltree's default), so no entity expands.
    let doc = roxmltree::Document::parse(text.strip_prefix('\u{feff}').unwrap_or(text)).map_err(|e| not_drm(e.to_string()))?;
    let root = doc.root_element();
    if root.tag_name().name() != "DrumMap" {
        return Err(not_drm("its root is not DrumMap".into()));
    }
    let list = |name: &str| named(root, "list", name);
    let map = items(list("Map").ok_or_else(|| not_drm("it has no Map list".into()))?);
    let order: Vec<usize> = match list("Order") {
        None => (0..map.len()).collect(),
        Some(o) => {
            let mut seen = HashSet::new();
            items(o)
                .iter()
                .map(|item| {
                    let pos = item.attribute("value").and_then(|v| v.trim().parse::<usize>().ok()).filter(|p| *p <= 127 && *p < map.len());
                    pos.filter(|p| seen.insert(*p)).ok_or_else(|| invalid("the Order list names a row that is not in the map, or one row twice"))
                })
                .collect::<Result<_, _>>()?
        }
    };
    let mut rows = Vec::new();
    for pos in order {
        let item = map[pos];
        let name = named(item, "string", "Name").and_then(|n| n.attribute("value")).unwrap_or("").trim();
        if name.is_empty() || name == format!("Sound {}", pos + 1) {
            continue;
        }
        let note = named(item, "int", "ONote")
            .and_then(|n| n.attribute("value"))
            .and_then(|v| v.trim().parse::<i32>().ok())
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| *n <= 127)
            .ok_or_else(|| invalid(format!("row \"{name}\" has no output note in 0-127")))?;
        rows.push(DrumRow { id: new_id(), name: name.to_string(), lane: None, slot: None, midi_note: Some(note) });
    }
    let title = named(root, "string", "Name").and_then(|n| n.attribute("value")).unwrap_or("").trim();
    let m = DrumMap { id: new_id(), name: if title.is_empty() { fallback_name.trim().to_string() } else { title.to_string() }, rows, audio: Vec::new() };
    m.validate()?;
    Ok(m)
}
// ---------------------------------------------------------------- the view rows (invariant 9)

/// One row of the drum-map view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrumViewRow<'a> {
    /// A drum-map row and the lane it plays, shown under the row's name.
    Instrument { row: &'a DrumRow, lane: &'a Lane },
    /// A row whose lane is not among the lanes: shown, and no lane is made for it.
    Unassigned(&'a DrumRow),
    /// A lane no row names, after the rows, so no sample or hit is ever out of reach.
    Other(&'a Lane),
}

/// The drum-map view's rows for `lanes`: every row of the map in its order, then every lane no
/// row names, in the lanes' own order. Applying a drum map never adds, removes or renames a lane
/// (invariant 9): the view's lanes are exactly `lanes`, a lane several rows name (an alias) shows
/// once per row, and turning the map off (`None`) shows the lanes as they are.
pub fn drum_rows<'a>(map: Option<&'a DrumMap>, lanes: &'a [Lane]) -> Vec<DrumViewRow<'a>> {
    let keys: Vec<String> = lanes.iter().map(lane_key).collect();
    let find = |key: &Option<String>| key.as_ref().and_then(|k| keys.iter().position(|l| l == k));
    let rows = map.map_or(&[][..], |m| &m.rows);
    let mut named = vec![false; lanes.len()];
    let mut out: Vec<DrumViewRow<'a>> = rows
        .iter()
        .map(|row| match find(&row.lane) {
            Some(i) => {
                named[i] = true;
                DrumViewRow::Instrument { row, lane: &lanes[i] }
            }
            None => DrumViewRow::Unassigned(row),
        })
        .collect();
    out.extend(lanes.iter().zip(named).filter(|(_, n)| !n).map(|(l, _)| DrumViewRow::Other(l)));
    out
}

// ---------------------------------------------------------------- the render plan (Responsibility 20)

/// The instrument note each lane renders with: a skin lane's sound starts on its General MIDI
/// note (hitnormal 36, hitclap 38, hitwhistle 42, hitfinish 49), then the map's rows override
/// it; the first row on a lane wins, so aliases play one sound, and a row with no note takes its
/// lane out of the render (old `MainWindow.DrumVstNotes`).
pub fn render_notes(map: Option<&DrumMap>, lanes: &[Lane]) -> BTreeMap<SampleIdentity, u8> {
    let mut notes = BTreeMap::new();
    for lane in lanes {
        if let (LaneSource::Skin, SampleIdentity::Standard { sound: Sound::Hit(h), .. }) = (&lane.source, &lane.identity) {
            let note = match h {
                Hitsound::Normal => 36,
                Hitsound::Clap => 38,
                Hitsound::Whistle => 42,
                Hitsound::Finish => 49,
            };
            notes.insert(lane.identity.clone(), note);
        }
    }
    let keys: Vec<String> = lanes.iter().map(lane_key).collect();
    let mut assigned = HashSet::new();
    for row in map.map_or(&[][..], |m| &m.rows) {
        let Some(i) = row.lane.as_ref().and_then(|k| keys.iter().position(|l| l == k)) else { continue };
        if !assigned.insert(i) {
            continue;
        }
        match row.midi_note {
            Some(n) => notes.insert(lanes[i].identity.clone(), n),
            None => notes.remove(&lanes[i].identity),
        };
    }
    notes
}

/// One hit to render: which moment and lane, at which time, with which note and velocity.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderHit {
    pub moment: MomentId,
    pub time: SongTime,
    pub lane: SampleIdentity,
    pub note: u8,
    pub velocity: u8,
}

/// Every hit on a lane with a note, in moment order, for the instrument engine. The velocity is
/// the hit's played volume (the rules object, R5) scaled to 0-127, rounded half to even as the old
/// app's `Math.Round`; times are the moments' own. The old app's floor was stable's 8; the played
/// volume's floor here is lazer's 5.
pub fn drum_render_plan(hitsounds: &Hitsounds, notes: &BTreeMap<SampleIdentity, u8>) -> Vec<RenderHit> {
    let mut plan = Vec::new();
    for (i, m) in hitsounds.moments().iter().enumerate() {
        let velocity = (f64::from(played_volume(m.volume())) * 127.0 / 100.0).round_ties_even().clamp(0.0, 127.0) as u8;
        for lane in m.hits() {
            if let Some(&note) = notes.get(lane) {
                plan.push(RenderHit { moment: MomentId(i), time: m.time(), lane: lane.clone(), note, velocity });
            }
        }
    }
    plan
}

/// The distinct (note, velocity) pairs a plan needs rendered, by note then velocity.
pub fn render_keys(plan: &[RenderHit]) -> Vec<(u8, u8)> {
    let keys: std::collections::BTreeSet<(u8, u8)> = plan.iter().map(|p| (p.note, p.velocity)).collect();
    keys.into_iter().collect()
}
// ---------------------------------------------------------------- base64 (ADR 0047: our own code)

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, as .NET writes a byte array.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() { BASE64[(n >> shift) as usize & 63] as char } else { '=' });
        }
    }
    out
}

/// Standard base64 with padding; `None` for anything else.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let t = text.as_bytes();
    if t.len() % 4 != 0 {
        return None;
    }
    let value = |c: u8| BASE64.iter().position(|&b| b == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(t.len() / 4 * 3);
    for (i, quad) in t.chunks(4).enumerate() {
        let last = i == t.len() / 4 - 1;
        let pad = quad.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n = 0u32;
        for &c in &quad[..4 - pad] {
            n = (n << 6) | value(c)?;
        }
        n <<= 6 * pad as u32;
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    Some(out)
}
