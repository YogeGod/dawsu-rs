//! Typed fields of timing lines and hit objects (Responsibility 3), read by lazer's rules
//! (Responsibility 8). They follow lazer at the pin:
//! `osu.Game/Beatmaps/Formats/LegacyBeatmapDecoder.cs:484-532` (timing lines) and
//! `osu.Game/Rulesets/Objects/Legacy/ConvertHitObjectParser.cs:55-240` (hit objects).
//! A line these rules cannot read is one lazer skips; the reason is returned as text.
//! The values are the file's own: no sample-set default is resolved here (Non-goals).
//! Follows osu!lazer (ppy/osu) at 131fe72a15581049401a588a13684938d4d0e0c7; copyright (c) ppy Pty Ltd <contact@ppy.sh>, MIT Licence (ADR 0027, THIRD-PARTY-NOTICES.md).

use crate::number::{parse_double, parse_float, parse_int, MAX_COORDINATE_VALUE, MAX_PARSE_VALUE};
use crate::LineId;

/// One timing line. Fields a short line leaves out are `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct TimingPoint {
    pub(crate) line: LineId,
    pub(crate) time: f64,
    pub(crate) beat_length: f64,
    pub(crate) meter: Option<i32>,
    pub(crate) sample_set: Option<i32>,
    pub(crate) sample_index: Option<i32>,
    pub(crate) volume: Option<i32>,
    pub(crate) uninherited: bool,
    pub(crate) effects: Option<i32>,
    pub(crate) field_count: usize,
}

impl TimingPoint {
    pub fn line(&self) -> LineId {
        self.line
    }
    pub fn time(&self) -> f64 {
        self.time
    }
    pub fn beat_length(&self) -> f64 {
        self.beat_length
    }
    /// The meter, or `None` when the field is missing or starts with `0` (lazer reads 4/4 then).
    pub fn meter(&self) -> Option<i32> {
        self.meter
    }
    pub fn sample_set(&self) -> Option<i32> {
        self.sample_set
    }
    pub fn sample_index(&self) -> Option<i32> {
        self.sample_index
    }
    pub fn volume(&self) -> Option<i32> {
        self.volume
    }
    /// A red line; a missing field counts as red, as in lazer.
    pub fn uninherited(&self) -> bool {
        self.uninherited
    }
    pub fn effects(&self) -> Option<i32> {
        self.effects
    }
    pub fn field_count(&self) -> usize {
        self.field_count
    }
}

pub(crate) fn parse_timing_point(line: LineId, text: &str) -> Result<TimingPoint, String> {
    let split: Vec<&str> = text.split(',').collect();
    let time = parse_double(field(&split, 0)?.trim(), MAX_PARSE_VALUE, false)?;
    let beat_length = parse_double(field(&split, 1)?.trim(), MAX_PARSE_VALUE, true)?;
    let mut meter = None;
    if split.len() >= 3 {
        let f = split[2];
        let first = f.chars().next().ok_or("the meter field is empty")?;
        if first != '0' {
            meter = Some(parse_int(f)?);
        }
    }
    let sample_set = if split.len() >= 4 { Some(parse_int(split[3])?) } else { None };
    let sample_index = if split.len() >= 5 { Some(parse_int(split[4])?) } else { None };
    let volume = if split.len() >= 6 { Some(parse_int(split[5])?) } else { None };
    let mut uninherited = true;
    if split.len() >= 7 {
        let first = split[6].chars().next().ok_or("the red-line field is empty")?;
        uninherited = first == '1';
    }
    let effects = if split.len() >= 8 { Some(parse_int(split[7])?) } else { None };
    if uninherited && beat_length.is_nan() {
        // LegacyBeatmapDecoder.cs:530-533: a red line with a beat length that is not a number throws.
        return Err("a red line's beat length is not a number".into());
    }
    Ok(TimingPoint { line, time, beat_length, meter, sample_set, sample_index, volume, uninherited, effects, field_count: split.len() })
}

/// The bank part of a hit sample: `normal:addition:index:volume:filename`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitSample {
    pub normal_set: i32,
    pub addition_set: i32,
    /// 0 when the field is missing.
    pub index: i32,
    /// 0 when the field is missing.
    pub volume: i32,
    /// The file name as written, never taken apart (Responsibility 12).
    pub filename: Option<String>,
}

/// A slider's own fields.
#[derive(Debug, Clone, PartialEq)]
pub struct SliderFields {
    pub curve: String,
    pub slides: i32,
    pub length: Option<f64>,
    /// Per-node hitsounds as written; lazer reads each with `int.TryParse`.
    pub edge_sounds: Option<Vec<String>>,
    /// Per-node banks; `None` for a node whose edge set is empty, which keeps the slider's own banks.
    pub edge_sets: Option<Vec<Option<(i32, i32)>>>,
}

/// What kind of object a line is.
#[derive(Debug, Clone, PartialEq)]
pub enum ObjectKind {
    Circle,
    Slider(SliderFields),
    Spinner { end: f64 },
    Hold { end: f64 },
}

/// One hit-object line.
#[derive(Debug, Clone, PartialEq)]
pub struct HitObject {
    pub(crate) line: LineId,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) time: f64,
    pub(crate) type_bits: i32,
    pub(crate) hitsound: i32,
    pub(crate) kind: ObjectKind,
    pub(crate) hit_sample: Option<HitSample>,
    pub(crate) field_count: usize,
}

impl HitObject {
    pub fn line(&self) -> LineId {
        self.line
    }
    pub fn x(&self) -> f32 {
        self.x
    }
    pub fn y(&self) -> f32 {
        self.y
    }
    /// The start time as written (no format-version offset is applied here).
    pub fn time(&self) -> f64 {
        self.time
    }
    /// The type field with its combo bits.
    pub fn type_bits(&self) -> i32 {
        self.type_bits
    }
    pub fn hitsound(&self) -> i32 {
        self.hitsound
    }
    pub fn kind(&self) -> &ObjectKind {
        &self.kind
    }
    pub fn slider(&self) -> Option<&SliderFields> {
        match &self.kind {
            ObjectKind::Slider(s) => Some(s),
            _ => None,
        }
    }
    pub fn spinner_end(&self) -> Option<f64> {
        match self.kind {
            ObjectKind::Spinner { end } => Some(end),
            _ => None,
        }
    }
    pub fn hold_end(&self) -> Option<f64> {
        match self.kind {
            ObjectKind::Hold { end } => Some(end),
            _ => None,
        }
    }
    /// The object's own hit sample (for a slider, field 11, of which lazer reads only the banks).
    pub fn hit_sample(&self) -> Option<&HitSample> {
        self.hit_sample.as_ref()
    }
    pub fn field_count(&self) -> usize {
        self.field_count
    }
}

const CIRCLE: i32 = 1;
const SLIDER: i32 = 2;
const NEW_COMBO: i32 = 4;
const SPINNER: i32 = 8;
const COMBO_OFFSET: i32 = 16 | 32 | 64;
const HOLD: i32 = 128;

fn field<'a>(split: &[&'a str], i: usize) -> Result<&'a str, String> {
    split.get(i).copied().ok_or_else(|| format!("field {} is missing", i + 1))
}

/// `readCustomSampleBanks` (ConvertHitObjectParser.cs:197-235). An empty text leaves the defaults.
fn read_banks(text: &str, banks_only: bool) -> Result<Option<HitSample>, String> {
    if text.is_empty() {
        return Ok(None);
    }
    let split: Vec<&str> = text.split(':').collect();
    let normal_set = parse_int(split[0])?;
    let addition_set = parse_int(field(&split, 1)?)?;
    let mut sample = HitSample { normal_set, addition_set, index: 0, volume: 0, filename: None };
    if banks_only {
        // lazer reads no index, volume or file name here; keep what is written for the domain.
        sample.index = split.get(2).and_then(|s| parse_int(s).ok()).unwrap_or(0);
        sample.volume = split.get(3).and_then(|s| parse_int(s).ok()).unwrap_or(0);
        sample.filename = split.get(4).map(|s| s.to_string());
        return Ok(Some(sample));
    }
    if split.len() > 2 {
        sample.index = parse_int(split[2])?;
    }
    if split.len() > 3 {
        sample.volume = parse_int(split[3])?.max(0);
    }
    sample.filename = split.get(4).map(|s| s.to_string());
    Ok(Some(sample))
}

pub(crate) fn parse_hit_object(line: LineId, text: &str) -> Result<HitObject, String> {
    let split: Vec<&str> = text.split(',').collect();
    let x = parse_float(field(&split, 0)?, MAX_COORDINATE_VALUE)?;
    let y = parse_float(field(&split, 1)?, MAX_COORDINATE_VALUE)?;
    let time = parse_double(field(&split, 2)?, MAX_PARSE_VALUE, false)?;
    let type_bits = parse_int(field(&split, 3)?)?;
    let hitsound = parse_int(field(&split, 4)?)?;
    let kind_bits = type_bits & !COMBO_OFFSET & !NEW_COMBO;

    let mut hit_sample = None;
    let kind = if kind_bits & CIRCLE != 0 {
        if split.len() > 5 {
            hit_sample = read_banks(split[5], false)?;
        }
        ObjectKind::Circle
    } else if kind_bits & SLIDER != 0 {
        let slides = parse_int(field(&split, 6)?)?;
        if slides > 9000 {
            return Err("the repeat count is way too high".into());
        }
        let length = if split.len() > 7 { Some(parse_double(split[7], MAX_COORDINATE_VALUE, false)?) } else { None };
        if split.len() > 10 {
            hit_sample = read_banks(split[10], true)?;
        }
        let nodes = (slides - 1).max(0) as usize + 2;
        let edge_sets = if split.len() > 9 && !split[9].is_empty() {
            let mut sets = Vec::new();
            for (i, set) in split[9].split('|').enumerate() {
                if i >= nodes {
                    break;
                }
                let banks = read_banks(set, false)?;
                sets.push(banks.map(|b| (b.normal_set, b.addition_set)));
            }
            Some(sets)
        } else {
            None
        };
        let edge_sounds = if split.len() > 8 && !split[8].is_empty() {
            Some(split[8].split('|').map(String::from).collect())
        } else {
            None
        };
        ObjectKind::Slider(SliderFields { curve: field(&split, 5)?.to_string(), slides, length, edge_sounds, edge_sets })
    } else if kind_bits & SPINNER != 0 {
        let end = parse_double(field(&split, 5)?, MAX_PARSE_VALUE, false)?;
        if split.len() > 6 {
            hit_sample = read_banks(split[6], false)?;
        }
        ObjectKind::Spinner { end }
    } else if kind_bits & HOLD != 0 {
        let mut end = time;
        if split.len() > 5 && !split[5].is_empty() {
            let ss: Vec<&str> = split[5].split(':').collect();
            end = parse_double(ss[0], MAX_PARSE_VALUE, false)?;
            hit_sample = read_banks(&ss[1..].join(":"), false)?;
        }
        ObjectKind::Hold { end }
    } else {
        return Err(format!("unknown hit object type {type_bits}"));
    };

    Ok(HitObject { line, x, y, time, type_bits, hitsound, kind, hit_sample, field_count: split.len() })
}

/// The number of nodes a slider with `slides` has: one per repeat plus the head and the tail.
pub(crate) fn slider_nodes(slides: i32) -> usize {
    (slides - 1).max(0) as usize + 2
}

/// What a hit-object line's type is, from its type field alone (for filling fields).
pub(crate) fn kind_bits_of(text: &str) -> Option<i32> {
    let t = text.split(',').nth(3)?;
    parse_int(t).ok().map(|b| b & !COMBO_OFFSET & !NEW_COMBO)
}

pub(crate) const KIND_CIRCLE: i32 = CIRCLE;
pub(crate) const KIND_SLIDER: i32 = SLIDER;
pub(crate) const KIND_SPINNER: i32 = SPINNER;
