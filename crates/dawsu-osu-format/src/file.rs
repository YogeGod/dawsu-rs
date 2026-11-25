//! A `.osu` file held as its original lines (Responsibilities 1, 2, 4 to 7 and 14).
//! A line that was not edited is written from its original bytes (invariant 2).

use crate::records::{self, HitObject, TimingPoint, KIND_CIRCLE, KIND_SLIDER, KIND_SPINNER};
use crate::{Error, LineId};
use dawsu_foundation::safe_write::{self, BackupPolicy, Warning as WriteWarning, WriteOutcome};
use std::path::Path;

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// The ending a line had in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    CrLf,
    Lf,
    Cr,
    /// The last line of a file with no final newline.
    None,
}

impl LineEnding {
    fn bytes(self) -> &'static [u8] {
        match self {
            LineEnding::CrLf => b"\r\n",
            LineEnding::Lf => b"\n",
            LineEnding::Cr => b"\r",
            LineEnding::None => b"",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    Original,
    Inserted,
}

#[derive(Debug, Clone)]
struct Line {
    id: LineId,
    content: Vec<u8>,
    ending: LineEnding,
    origin: Origin,
    edited: bool,
    /// The content as read, for the change summary.
    original: Vec<u8>,
    /// The section the line belongs to (`None` before the first header).
    section: Option<String>,
}

/// A read-only view of one line.
#[derive(Debug, Clone, Copy)]
pub struct LineView<'a> {
    line: &'a Line,
}

impl<'a> LineView<'a> {
    pub fn id(&self) -> LineId {
        self.line.id
    }
    pub fn content(&self) -> &'a [u8] {
        &self.line.content
    }
    pub fn ending(&self) -> LineEnding {
        self.line.ending
    }
}

/// A line lazer could not read: it is skipped, and its bytes stay as they were (Responsibility 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadWarning {
    /// 1-based line number in the file.
    pub line_number: usize,
    pub section: String,
    pub reason: String,
}

/// A file and the warnings from reading it. Reading never fails: any bytes are a file.
#[derive(Debug, Clone)]
pub struct Read {
    pub file: OsuFile,
    pub warnings: Vec<ReadWarning>,
}

/// A refused edit: a normal answer, not an error (the Errors subsection).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The line's original bytes are not valid UTF-8 (invariant 5).
    NotUtf8,
    /// No line with that id.
    NotFound,
    /// The line is a red line; only green lines are removed this way (invariant 8).
    RedLine,
    /// The line is not of the kind the operation needs.
    WrongKind,
    /// A field before the one to set is missing and cannot be filled with what osu! played.
    CannotFill,
    /// The text holds a line break, which would make it two lines.
    LineBreak,
}

/// One line as it was at a moment, to be put back (see [`OsuFile::snapshot_line`]).
#[derive(Debug, Clone)]
pub struct LineSnapshot {
    id: LineId,
    content: Vec<u8>,
    ending: LineEnding,
    edited: bool,
}

/// A storyboard sample line of `[Events]`, read as lazer reads it
/// (`LegacyStoryboardDecoder.cs:186-194` at 131fe72): `Sample,time,layer,"path",volume`, or `5,...`.
#[derive(Debug, Clone, PartialEq)]
pub struct StoryboardSample {
    pub line: LineId,
    pub time: f64,
    /// The layer as written (a number or a name).
    pub layer: String,
    /// The path lazer looks up: variables replaced, `\\` made `\`, quotes trimmed, `\` made `/`.
    pub path: String,
    /// The path field as written, variables and quotes kept, for writing the line again.
    pub raw_path: String,
    /// 100 when the line has no volume; truncated and never clamped, as lazer reads it.
    pub volume: f32,
}

/// A storyboard sample event (`Sample` or `5`) at no indent, or `None`.
fn sample_line(line: LineId, text: &str, as_written: &str) -> Option<StoryboardSample> {
    if text.starts_with([' ', '_']) {
        return None;
    }
    let split: Vec<&str> = text.split(',').collect();
    if !matches!(split.first().map(|s| s.trim()), Some("Sample" | "5")) || split.len() < 4 {
        return None;
    }
    let time = crate::number::parse_double(split[1], crate::number::MAX_PARSE_VALUE, false).ok()?;
    let volume = match split.get(4) {
        Some(v) => crate::number::parse_float(v, crate::number::MAX_PARSE_VALUE).ok()?.trunc(),
        None => 100.0,
    };
    let path = split[3].replace("\\\\", "\\").trim_matches('"').replace('\\', "/");
    Some(StoryboardSample { line, time, layer: split[2].trim().to_string(), path, raw_path: as_written.split(',').nth(3).unwrap_or(split[3]).to_string(), volume })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOutcome {
    Changed,
    /// The field already held that value; nothing changed (invariant 3).
    Unchanged,
}

/// The answer of the one write (Responsibility 14, ADR 0038).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved { warnings: Vec<WriteWarning> },
    /// The file changed since it was read; nothing was written.
    ChangedOnDisk,
    /// The file has no edits; nothing was written and no backup kept.
    NothingToSave,
    /// `save_new` found a file already at the target; nothing was written.
    AlreadyExists,
}

/// What a write would change (Responsibility 6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSummary {
    pub hit_objects_changed: usize,
    pub hit_objects_added: usize,
    pub hit_objects_removed: usize,
    pub green_lines_added: usize,
    pub green_lines_edited: usize,
    pub green_lines_removed: usize,
    pub red_lines_changed: usize,
    pub other_lines_changed: usize,
    /// Every changed line: (section, before, after); `None` for an added or removed line.
    pub lines: Vec<(String, Option<String>, Option<String>)>,
}

impl ChangeSummary {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// A write changes more than hitsounds when it adds or removes objects, touches a red line
    /// or changes any line outside timing lines and hit objects.
    pub fn touches_more_than_hitsounds(&self) -> bool {
        self.hit_objects_added > 0 || self.hit_objects_removed > 0 || self.red_lines_changed > 0 || self.other_lines_changed > 0
    }
}

/// A `.osu` file as its lines.
#[derive(Debug, Clone)]
pub struct OsuFile {
    bom: bool,
    lines: Vec<Line>,
    original: Vec<u8>,
    removed: Vec<(String, Vec<u8>)>,
    next_id: u64,
}

/// Sections in the order osu! writes them, for adding a missing one.
const SECTION_ORDER: [&str; 9] = ["General", "Editor", "Metadata", "Difficulty", "Events", "TimingPoints", "Colours", "HitObjects", "Variables"];

/// The text lazer parses from a line: comments stripped (except in Metadata) and the end trimmed.
/// `None` for a line lazer skips (blank, or a `//` comment).
fn logical_text(content: &[u8], section: Option<&str>) -> Option<String> {
    let text = String::from_utf8_lossy(content);
    if text.trim().is_empty() || text.trim_start().starts_with("//") {
        return None;
    }
    let mut s: &str = &text;
    if section != Some("Metadata")
        && let Some(i) = s.find("//")
            && i > 0 {
                s = &s[..i];
            }
    Some(s.trim_end().to_string())
}

/// The section a header line opens, if it is one.
fn header_name(logical: &str) -> Option<&str> {
    if logical.starts_with('[') && logical.ends_with(']') && logical.len() >= 2 {
        Some(&logical[1..logical.len() - 1])
    } else {
        None
    }
}

impl OsuFile {
    /// Holds `bytes` as lines; lines lazer cannot read in timing points and hit objects become warnings.
    pub fn read(bytes: Vec<u8>) -> Read {
        let bom = bytes.starts_with(BOM);
        let body = if bom { &bytes[BOM.len()..] } else { &bytes[..] };
        let mut lines = Vec::new();
        let mut start = 0;
        let mut i = 0;
        let mut next_id = 0u64;
        let mut push = |lines: &mut Vec<Line>, content: &[u8], ending: LineEnding| {
            lines.push(Line { id: LineId(next_id), content: content.to_vec(), ending, origin: Origin::Original, edited: false, original: content.to_vec(), section: None });
            next_id += 1;
        };
        while i < body.len() {
            match body[i] {
                b'\r' if body.get(i + 1) == Some(&b'\n') => {
                    push(&mut lines, &body[start..i], LineEnding::CrLf);
                    i += 2;
                    start = i;
                }
                b'\r' => {
                    push(&mut lines, &body[start..i], LineEnding::Cr);
                    i += 1;
                    start = i;
                }
                b'\n' => {
                    push(&mut lines, &body[start..i], LineEnding::Lf);
                    i += 1;
                    start = i;
                }
                _ => i += 1,
            }
        }
        if start < body.len() {
            push(&mut lines, &body[start..], LineEnding::None);
        }
        let mut current: Option<String> = None;
        for line in &mut lines {
            if let Some(text) = logical_text(&line.content, current.as_deref())
                && let Some(name) = header_name(&text) {
                    current = Some(name.to_string());
                }
            line.section = current.clone();
        }
        let file = OsuFile { bom, lines, original: bytes.clone(), removed: Vec::new(), next_id };
        let warnings = file.scan_warnings();
        Read { file, warnings }
    }

    fn scan_warnings(&self) -> Vec<ReadWarning> {
        let mut out = Vec::new();
        for (n, (line, section)) in self.lines.iter().zip(self.sections()).enumerate() {
            let Some(section) = section else { continue };
            let Some(text) = logical_text(&line.content, Some(&section)) else { continue };
            if header_name(&text).is_some() {
                continue;
            }
            let result = match section.as_str() {
                "TimingPoints" => records::parse_timing_point(line.id, &text).map(|_| ()),
                "HitObjects" => records::parse_hit_object(line.id, &text).map(|_| ()),
                _ => Ok(()),
            };
            if let Err(reason) = result {
                out.push(ReadWarning { line_number: n + 1, section, reason });
            }
        }
        out
    }

    /// The section each line belongs to (`None` before the first header).
    fn sections(&self) -> Vec<Option<String>> {
        self.lines.iter().map(|l| l.section.clone()).collect()
    }

    /// The bytes to write: the original bytes of every unedited line (Responsibility 2).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.original.len() + 64);
        if self.bom {
            out.extend_from_slice(BOM);
        }
        for line in &self.lines {
            out.extend_from_slice(&line.content);
            out.extend_from_slice(line.ending.bytes());
        }
        out
    }

    /// The bytes the file was read from.
    pub fn original_bytes(&self) -> &[u8] {
        &self.original
    }

    pub fn has_bom(&self) -> bool {
        self.bom
    }

    pub fn lines(&self) -> impl Iterator<Item = LineView<'_>> {
        self.lines.iter().map(|line| LineView { line })
    }

    fn index_of(&self, id: LineId) -> Option<usize> {
        self.lines.iter().position(|l| l.id == id)
    }

    pub fn section_of(&self, id: LineId) -> Option<String> {
        let i = self.index_of(id)?;
        self.lines[i].section.clone()
    }

    /// The line's text, if it is UTF-8.
    pub fn line_text(&self, id: LineId) -> Option<String> {
        let i = self.index_of(id)?;
        String::from_utf8(self.lines[i].content.clone()).ok()
    }

    pub fn line_ending(&self, id: LineId) -> Option<LineEnding> {
        self.index_of(id).map(|i| self.lines[i].ending)
    }

    /// `osu file format vN` from the first line, if there is one.
    pub fn format_version(&self) -> Option<u32> {
        let first = self.lines.iter().find_map(|l| logical_text(&l.content, None))?;
        first.trim().strip_prefix("osu file format v")?.trim().parse().ok()
    }

    /// A `[General]` value, trimmed, as lazer splits `key: value`.
    pub fn general(&self, key: &str) -> Option<String> {
        self.value("General", key)
    }

    /// A `key: value` from `section` (such as `[Difficulty] SliderMultiplier`), trimmed. Of several
    /// lines for one key the last counts, since lazer assigns the value for every line it reads
    /// (`LegacyBeatmapDecoder.cs:351-380` at 131fe72).
    pub fn value(&self, section_name: &str, key: &str) -> Option<String> {
        let i = self.value_line(section_name, key)?;
        let text = logical_text(&self.lines[i].content, Some(section_name))?;
        text.split_once(':').map(|(_, v)| v.trim().to_string())
    }

    /// Every `key:value` of `section` in file order, keys and values trimmed (as lazer reads
    /// `[Colours]`, `LegacyDecoder.cs:130-145` at 131fe72). Lines without a colon are left out.
    pub fn section_values(&self, section_name: &str) -> Vec<(String, String)> {
        self.lines.iter()
            .filter(|line| line.section.as_deref() == Some(section_name))
            .filter_map(|line| logical_text(&line.content, Some(section_name)))
            .filter_map(|text| text.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect()
    }

    /// The index of the last `key:value` line for `key` in `section`.
    fn value_line(&self, section_name: &str, key: &str) -> Option<usize> {
        self.lines.iter().enumerate().rev().find_map(|(i, line)| {
            if line.section.as_deref() != Some(section_name) {
                return None;
            }
            let text = logical_text(&line.content, Some(section_name))?;
            let (k, _) = text.split_once(':')?;
            (k.trim() == key).then_some(i)
        })
    }

    /// Sets the value of the last `key:value` line for `key` in `section`, keeping the key and the
    /// spacing after its colon. A missing line is added at the end of the section, and a missing
    /// section in osu!'s order (Responsibility 4).
    pub fn set_value(&mut self, section_name: &str, key: &str, text: &str) -> Result<SetOutcome, Refusal> {
        if text.contains(['\r', '\n']) {
            return Err(Refusal::LineBreak);
        }
        match self.value_line(section_name, key) {
            Some(i) => {
                let content = std::str::from_utf8(&self.lines[i].content).map_err(|_| Refusal::NotUtf8)?;
                let colon = content.find(':').ok_or(Refusal::WrongKind)?;
                let rest = &content[colon + 1..];
                let spacing = &rest[..rest.len() - rest.trim_start().len()];
                let new = format!("{}{spacing}{text}", &content[..=colon]);
                if new.as_bytes() == self.lines[i].content.as_slice() {
                    return Ok(SetOutcome::Unchanged);
                }
                let line = &mut self.lines[i];
                line.content = new.into_bytes();
                line.edited = line.content != line.original || line.origin == Origin::Inserted;
                Ok(SetOutcome::Changed)
            }
            None => {
                // osu! writes `[General]` and `[Editor]` values after ": ", the others after ":".
                let separator = if matches!(section_name, "General" | "Editor") { ": " } else { ":" };
                let after = self.last_line_of(section_name);
                self.insert_after(Some(after), &format!("{key}{separator}{text}"));
                Ok(SetOutcome::Changed)
            }
        }
    }

    /// The index of the last non-blank line of `section` (its header when it has none), adding
    /// the section when it is missing.
    fn last_line_of(&mut self, section_name: &str) -> usize {
        let header = self.ensure_section(section_name);
        let mut after = header;
        for (i, line) in self.lines.iter().enumerate().skip(header + 1) {
            if line.section.as_deref() != Some(section_name) {
                break;
            }
            if logical_text(&line.content, Some(section_name)).is_some() {
                after = i;
            }
        }
        after
    }

    /// Every storyboard sample line of `[Events]` lazer can read, in file order (Responsibility 11),
    /// with this file's own `[Variables]`. For a `.osb`, use [`OsuFile::storyboard_samples_after`].
    pub fn storyboard_samples(&self) -> Vec<StoryboardSample> {
        self.samples_with(Vec::new()).0
    }

    /// The storyboard samples of a `.osb` read after its `.osu`: lazer reads both with one
    /// storyboard decoder, the `.osu` first, so the `.osu`'s variables carry into the `.osb`, never
    /// back (`LegacyStoryboardDecoder.cs` and `WorkingBeatmapCache.cs` at 131fe72).
    pub fn storyboard_samples_after(&self, earlier: &OsuFile) -> Vec<StoryboardSample> {
        self.samples_with(earlier.samples_with(Vec::new()).1).0
    }

    /// The sample lines, and the variables in force at the end. As lazer at 131fe72: a variable
    /// line is split at its first `=` with nothing trimmed (no `=`: the whole line, empty value);
    /// a name defined again keeps its place with the new value; a variable applies only to the
    /// lines after it; an `[Events]` line holding `$` has each variable replaced in turn, plainly,
    /// before it is split.
    fn samples_with(&self, mut vars: Vec<(String, String)>) -> (Vec<StoryboardSample>, Vec<(String, String)>) {
        let mut out = Vec::new();
        for line in &self.lines {
            let section = line.section.as_deref();
            let Some(text) = logical_text(&line.content, section) else { continue };
            if header_name(&text).is_some() {
                continue;
            }
            match section {
                Some("Variables") => {
                    let (key, value) = text.split_once('=').unwrap_or((&text, ""));
                    match vars.iter_mut().find(|(k, _)| k == key) {
                        Some(v) => v.1 = value.to_string(),
                        None => vars.push((key.to_string(), value.to_string())),
                    }
                }
                Some("Events") => {
                    let mut replaced = text.clone();
                    if replaced.contains('$') {
                        for (k, v) in &vars {
                            replaced = replaced.replace(k.as_str(), v);
                        }
                    }
                    out.extend(sample_line(line.id, &replaced, &text));
                }
                _ => {}
            }
        }
        (out, vars)
    }

    /// Removes one storyboard sample line (Responsibility 5).
    pub fn remove_storyboard_sample(&mut self, id: LineId) -> Result<(), Refusal> {
        if !self.storyboard_samples().iter().any(|s| s.line == id) {
            return Err(if self.index_of(id).is_some() { Refusal::WrongKind } else { Refusal::NotFound });
        }
        self.remove_line(id, "Events");
        Ok(())
    }

    fn records_in(&self, section_name: &str) -> Vec<(usize, LineId, String)> {
        let mut out = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            if line.section.as_deref() != Some(section_name) {
                continue;
            }
            let Some(text) = logical_text(&line.content, Some(section_name)) else { continue };
            if header_name(&text).is_some() {
                continue;
            }
            out.push((i, line.id, text));
        }
        out
    }

    /// Every timing line lazer can read, in file order.
    pub fn timing_points(&self) -> Vec<TimingPoint> {
        self.records_in("TimingPoints").into_iter().filter_map(|(_, id, t)| records::parse_timing_point(id, &t).ok()).collect()
    }

    /// Every hit-object line lazer can read, in file order.
    pub fn hit_objects(&self) -> Vec<HitObject> {
        self.records_in("HitObjects").into_iter().filter_map(|(_, id, t)| records::parse_hit_object(id, &t).ok()).collect()
    }

    // ---------------------------------------------------------------- editing (Responsibility 4)

    /// Sets field `index` (0-based, comma-separated) of a line, filling any missing fields
    /// before it with what osu! played for them (invariant 6).
    pub fn set_field(&mut self, id: LineId, index: usize, text: &str) -> Result<SetOutcome, Refusal> {
        if text.contains(['\r', '\n']) {
            return Err(Refusal::LineBreak);
        }
        let i = self.index_of(id).ok_or(Refusal::NotFound)?;
        let section = self.lines[i].section.clone();
        let content = std::str::from_utf8(&self.lines[i].content).map_err(|_| Refusal::NotUtf8)?.to_string();
        let (data, comment_suffix) = if section.as_deref() != Some("Metadata") {
            if let Some(pos) = content.find("//") {
                let before = content[..pos].trim_end_matches([' ', '\t']);
                (before, &content[before.len()..])
            } else {
                (content.as_str(), "")
            }
        } else {
            (content.as_str(), "")
        };
        let mut fields: Vec<String> = data.split(',').map(String::from).collect();
        if index < fields.len() {
            if fields[index] == text {
                return Ok(SetOutcome::Unchanged);
            }
            fields[index] = text.to_string();
        } else {
            while fields.len() < index {
                let fill = self.fill_value(section.as_deref(), &fields, fields.len()).ok_or(Refusal::CannotFill)?;
                fields.push(fill);
            }
            fields.push(text.to_string());
        }
        let mut rebuilt = fields.join(",");
        rebuilt.push_str(comment_suffix);
        let line = &mut self.lines[i];
        line.content = rebuilt.into_bytes();
        line.edited = line.content != line.original || line.origin == Origin::Inserted;
        Ok(SetOutcome::Changed)
    }

    /// What osu! played for a missing field `at` of a line with `fields` so far.
    fn fill_value(&self, section: Option<&str>, fields: &[String], at: usize) -> Option<String> {
        match section? {
            "TimingPoints" => match at {
                2 => Some("4".into()),
                3 => Some(self.default_sample_set().to_string()),
                4 => Some("0".into()),
                5 => Some(self.general("SampleVolume").unwrap_or_else(|| "100".into())),
                6 => Some("1".into()),
                7 => Some("0".into()),
                _ => None,
            },
            "HitObjects" => {
                let kind = records::kind_bits_of(&fields.join(","))?;
                if kind & KIND_CIRCLE != 0 {
                    (at == 5).then(|| "0:0:0:0:".into())
                } else if kind & KIND_SLIDER != 0 {
                    let slides: i32 = fields.get(6)?.trim().parse().ok()?;
                    let nodes = records::slider_nodes(slides);
                    let hitsound = fields.get(4)?.trim().to_string();
                    match at {
                        7 => Some("0".into()),
                        8 => Some(vec![hitsound; nodes].join("|")),
                        9 => Some(vec!["0:0"; nodes].join("|")),
                        10 => Some("0:0:0:0:".into()),
                        _ => None,
                    }
                } else if kind & KIND_SPINNER != 0 {
                    (at == 6).then(|| "0:0:0:0:".into())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// The `[General]` sample set as the number a timing line holds (`None` and missing are 0).
    fn default_sample_set(&self) -> i32 {
        match self.general("SampleSet").as_deref() {
            Some("Normal") => 1,
            Some("Soft") => 2,
            Some("Drum") => 3,
            Some(other) => other.parse().unwrap_or(0),
            None => 0,
        }
    }

    // ---------------------------------------------------------------- inserting and removing (Responsibility 5)

    fn first_ending(&self) -> LineEnding {
        self.lines.iter().map(|l| l.ending).find(|e| *e != LineEnding::None).unwrap_or(LineEnding::CrLf)
    }

    /// Inserts a line after line index `after` (or at the start), copying a neighbour's ending (invariant 7).
    fn insert_after(&mut self, after: Option<usize>, text: &str) -> LineId {
        let id = LineId(self.next_id);
        self.next_id += 1;
        let at = after.map_or(0, |a| a + 1);
        let ending = match after {
            Some(a) if self.lines[a].ending == LineEnding::None => {
                // The old last line gets the file's first ending; the new line ends the file without one.
                self.lines[a].ending = self.first_ending();
                LineEnding::None
            }
            Some(a) => self.lines[a].ending,
            None => self.first_ending(),
        };
        // A header opens its own section; any other line joins the section of the line before it.
        let section = match header_name(text) {
            Some(name) => Some(name.to_string()),
            None => after.and_then(|a| self.lines[a].section.clone()),
        };
        self.lines.insert(at, Line { id, content: text.as_bytes().to_vec(), ending, origin: Origin::Inserted, edited: true, original: Vec::new(), section });
        id
    }

    /// The index of the header line of `name`, adding the section in osu!'s order when missing.
    fn ensure_section(&mut self, name: &str) -> usize {
        let sections = self.sections();
        for (i, line) in self.lines.iter().enumerate() {
            if let Some(text) = logical_text(&line.content, sections[i].as_deref())
                && header_name(&text) == Some(name) {
                    return i;
                }
        }
        let rank = SECTION_ORDER.iter().position(|s| *s == name).unwrap_or(SECTION_ORDER.len());
        let later = self.lines.iter().enumerate().find_map(|(i, line)| {
            let text = logical_text(&line.content, sections[i].as_deref())?;
            let h = header_name(&text)?;
            let r = SECTION_ORDER.iter().position(|s| *s == h)?;
            (r > rank).then_some(i)
        });
        let header = format!("[{name}]");
        match later {
            Some(i) => {
                let before = i.checked_sub(1);
                let id = self.insert_after(before, &header);
                let h = self.index_of(id).unwrap();
                self.insert_after(Some(h), "");
                h
            }
            None => {
                let last = self.lines.len().checked_sub(1);
                let blank = self.insert_after(last, "");
                let b = self.index_of(blank).unwrap();
                let id = self.insert_after(Some(b), &header);
                self.index_of(id).unwrap()
            }
        }
    }

    /// The index of the last record line in `section` whose time is at or before `time`,
    /// or the section's header when there is none.
    fn insertion_point(&mut self, section: &str, time: f64) -> usize {
        let header = self.ensure_section(section);
        let mut after = header;
        for (i, _, text) in self.records_in(section) {
            let t = text.split(',').nth(if section == "HitObjects" { 2 } else { 0 }).and_then(|s| s.trim().parse::<f64>().ok());
            if let Some(t) = t
                && t <= time {
                    after = after.max(i);
                }
        }
        after
    }

    /// Inserts a green line after the last timing point at or before its time.
    pub fn insert_green_line(&mut self, text: &str) -> LineId {
        let time = text.split(',').next().and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(f64::MIN);
        let after = self.insertion_point("TimingPoints", time);
        self.insert_after(Some(after), text)
    }

    /// Inserts a hit-object line in time order.
    pub fn insert_hit_object(&mut self, text: &str) -> LineId {
        let time = text.split(',').nth(2).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(f64::MIN);
        let after = self.insertion_point("HitObjects", time);
        self.insert_after(Some(after), text)
    }

    /// Removes a green line; a red line is refused (invariant 8).
    pub fn remove_green_line(&mut self, id: LineId) -> Result<(), Refusal> {
        let tp = self.timing_points().into_iter().find(|t| t.line() == id);
        match tp {
            None => Err(if self.index_of(id).is_some() { Refusal::WrongKind } else { Refusal::NotFound }),
            Some(tp) if tp.uninherited() => Err(Refusal::RedLine),
            Some(_) => {
                self.remove_line(id, "TimingPoints");
                Ok(())
            }
        }
    }

    /// Removes a hit-object line.
    pub fn remove_hit_object(&mut self, id: LineId) -> Result<(), Refusal> {
        if !self.hit_objects().iter().any(|o| o.line() == id) {
            return Err(if self.index_of(id).is_some() { Refusal::WrongKind } else { Refusal::NotFound });
        }
        self.remove_line(id, "HitObjects");
        Ok(())
    }

    /// Appends hit-object lines after the last line of `[HitObjects]`, in the order given (the
    /// caller gives them in time order), adding the section when it is missing. For a new hitsound
    /// difficulty, whose section is empty; one pass instead of a search per line.
    pub fn append_hit_objects(&mut self, texts: &[String]) -> Vec<LineId> {
        let mut after = self.last_line_of("HitObjects");
        texts
            .iter()
            .map(|text| {
                let id = self.insert_after(Some(after), text);
                after += 1;
                id
            })
            .collect()
    }

    /// A copy of one line as it is now, to put back with [`OsuFile::restore_line`] when a group
    /// of edits to it has to be undone.
    pub fn snapshot_line(&self, id: LineId) -> Option<LineSnapshot> {
        let line = &self.lines[self.index_of(id)?];
        Some(LineSnapshot { id, content: line.content.clone(), ending: line.ending, edited: line.edited })
    }

    /// The id the next added line gets; lines added later have ids at or after it.
    pub fn next_line_id(&self) -> LineId {
        LineId(self.next_id)
    }

    /// Removes every line added since [`OsuFile::next_line_id`] gave `mark`, a header included.
    pub fn remove_lines_added_since(&mut self, mark: LineId) {
        let added: Vec<LineId> = self.lines.iter().filter(|l| l.id >= mark).map(|l| l.id).collect();
        for id in added.into_iter().rev() {
            // An added line is never in the change summary's removed list, so the section is unused.
            self.remove_line(id, "");
        }
    }

    /// Puts a line back as a snapshot holds it.
    pub fn restore_line(&mut self, snapshot: LineSnapshot) {
        if let Some(i) = self.index_of(snapshot.id) {
            let line = &mut self.lines[i];
            line.content = snapshot.content;
            line.ending = snapshot.ending;
            line.edited = snapshot.edited;
        }
    }

    /// Removes every `[HitObjects]` record line, those lazer cannot read included; blank lines,
    /// comments and the header stay. For a new hitsound difficulty, which starts from a copy.
    pub fn remove_all_hit_objects(&mut self) {
        for (_, id, _) in self.records_in("HitObjects") {
            self.remove_line(id, "HitObjects");
        }
    }

    fn remove_line(&mut self, id: LineId, section: &str) {
        let i = self.index_of(id).unwrap();
        let line = self.lines.remove(i);
        if line.ending == LineEnding::None && i > 0 {
            // The new last line takes over "no final newline".
            self.lines[i - 1].ending = LineEnding::None;
        }
        if line.origin == Origin::Original {
            self.removed.push((section.to_string(), line.original));
        }
    }

    /// Writes the `[Editor]` bookmarks line, adding the line or the section when missing.
    pub fn set_bookmarks(&mut self, times: &[i64]) {
        let text = format!("Bookmarks: {}", times.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(","));
        let header = self.ensure_section("Editor");
        let sections = self.sections();
        let existing = (header + 1..self.lines.len()).find(|&i| {
            sections[i].as_deref() == Some("Editor")
                && logical_text(&self.lines[i].content, Some("Editor")).is_some_and(|t| t.split_once(':').is_some_and(|(k, _)| k.trim() == "Bookmarks"))
        });
        match existing {
            Some(i) => {
                let line = &mut self.lines[i];
                line.content = text.into_bytes();
                line.edited = line.content != line.original || line.origin == Origin::Inserted;
            }
            None => {
                self.insert_after(Some(header), &text);
            }
        }
    }

    /// Adds a storyboard sample line at the end of `[Events]`.
    pub fn insert_storyboard_sample(&mut self, text: &str) -> LineId {
        let after = self.last_line_of("Events");
        self.insert_after(Some(after), text)
    }

    // ---------------------------------------------------------------- the change summary (Responsibility 6)

    pub fn change_summary(&self) -> ChangeSummary {
        let mut s = ChangeSummary::default();
        let sections = self.sections();
        let is_red = |text: &str| records::parse_timing_point(LineId(0), text).map(|t| t.uninherited()).unwrap_or(false);
        for (line, section) in self.lines.iter().zip(sections) {
            let section = section.unwrap_or_default();
            let after = String::from_utf8_lossy(&line.content).into_owned();
            match line.origin {
                Origin::Inserted => {
                    match section.as_str() {
                        "HitObjects" => s.hit_objects_added += 1,
                        "TimingPoints" if is_red(&after) => s.red_lines_changed += 1,
                        "TimingPoints" => s.green_lines_added += 1,
                        _ => s.other_lines_changed += 1,
                    }
                    s.lines.push((section, None, Some(after)));
                }
                Origin::Original if line.content != line.original => {
                    let before = String::from_utf8_lossy(&line.original).into_owned();
                    match section.as_str() {
                        "HitObjects" => s.hit_objects_changed += 1,
                        "TimingPoints" if is_red(&before) || is_red(&after) => s.red_lines_changed += 1,
                        "TimingPoints" => s.green_lines_edited += 1,
                        _ => s.other_lines_changed += 1,
                    }
                    s.lines.push((section, Some(before), Some(after)));
                }
                Origin::Original => {}
            }
        }
        for (section, original) in &self.removed {
            let before = String::from_utf8_lossy(original).into_owned();
            match section.as_str() {
                "HitObjects" => s.hit_objects_removed += 1,
                "TimingPoints" if is_red(&before) => s.red_lines_changed += 1,
                "TimingPoints" => s.green_lines_removed += 1,
                _ => s.other_lines_changed += 1,
            }
            s.lines.push((section.clone(), Some(before), None));
        }
        s
    }

    // ---------------------------------------------------------------- the one write (Responsibility 14)

    /// Writes this difficulty over the file it was read from, in one step (ADR 0038).
    pub fn save_over(&self, target: &Path, backups: &BackupPolicy) -> Result<SaveOutcome, Error> {
        let bytes = self.to_bytes();
        if bytes == self.original {
            return Ok(SaveOutcome::NothingToSave);
        }
        match safe_write::replace(target, &self.original, &bytes, backups).map_err(Error::Write)? {
            WriteOutcome::Saved { warnings } => Ok(SaveOutcome::Saved { warnings }),
            WriteOutcome::ChangedOnDisk => Ok(SaveOutcome::ChangedOnDisk),
        }
    }

    /// Writes a compiled document against the most recent successfully read/written disk version.
    /// An editor can keep its original compile baseline for undo without treating an earlier save
    /// as an external edit. `expected` must be the caller's last observed bytes, never a fresh read
    /// taken merely to make a changed-on-disk check pass.
    pub fn save_over_expected(&self, target: &Path, expected: &[u8], backups: &BackupPolicy) -> Result<SaveOutcome, Error> {
        let bytes = self.to_bytes();
        if bytes == expected {
            let actual = std::fs::read(target).map_err(|e| Error::io("checking the saved difficulty", target, e))?;
            return Ok(if actual == expected { SaveOutcome::NothingToSave } else { SaveOutcome::ChangedOnDisk });
        }
        match safe_write::replace(target, expected, &bytes, backups).map_err(Error::Write)? {
            WriteOutcome::Saved { warnings } => Ok(SaveOutcome::Saved { warnings }),
            WriteOutcome::ChangedOnDisk => Ok(SaveOutcome::ChangedOnDisk),
        }
    }

    /// Writes this difficulty as a new file; a file already at the target is left alone.
    pub fn save_new(&self, target: &Path) -> Result<SaveOutcome, Error> {
        match safe_write::create(target, &self.to_bytes()) {
            Ok(()) => Ok(SaveOutcome::Saved { warnings: vec![] }),
            Err(dawsu_foundation::Error::Io { source, .. })
                if source.kind() == std::io::ErrorKind::AlreadyExists =>
            {
                Ok(SaveOutcome::AlreadyExists)
            }
            Err(e) => Err(Error::Write(e)),
        }
    }
}

/// The round-trip check of Responsibility 7.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundTripReport {
    /// Reading and writing with no edits gives the same bytes.
    pub same_bytes: bool,
    /// Every timing line and hit object parses.
    pub all_fields_parse: bool,
    /// Setting every field of every timing line and hit object to its own value changes nothing.
    pub same_value_write_changes_nothing: bool,
}

pub fn round_trip_check(bytes: &[u8]) -> RoundTripReport {
    let read = OsuFile::read(bytes.to_vec());
    let same_bytes = read.file.to_bytes() == bytes;
    let all_fields_parse = read.warnings.is_empty();
    let mut file = read.file;
    let ids: Vec<(LineId, String)> = file
        .records_in("TimingPoints")
        .into_iter()
        .chain(file.records_in("HitObjects"))
        .filter_map(|(i, id, _)| String::from_utf8(file.lines[i].content.clone()).ok().map(|t| (id, t)))
        .collect();
    for (id, text) in ids {
        for (n, value) in text.split(',').enumerate() {
            let _ = file.set_field(id, n, value);
        }
    }
    let same_value_write_changes_nothing = file.to_bytes() == bytes && file.change_summary().is_empty();
    RoundTripReport { same_bytes, all_fields_parse, same_value_write_changes_nothing }
}
