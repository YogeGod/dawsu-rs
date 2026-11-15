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
    if section != Some("Metadata") {
        if let Some(i) = s.find("//") {
            if i > 0 {
                s = &s[..i];
            }
        }
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
            if let Some(text) = logical_text(&line.content, current.as_deref()) {
                if let Some(name) = header_name(&text) {
                    current = Some(name.to_string());
                }
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
        for (line, section) in self.lines.iter().zip(self.sections()) {
            if section.as_deref() != Some("General") {
                continue;
            }
            let Some(text) = logical_text(&line.content, Some("General")) else { continue };
            if let Some((k, v)) = text.split_once(':') {
                if k.trim() == key {
                    return Some(v.trim().to_string());
                }
            }
        }
        None
    }

    fn records_in(&self, section_name: &str) -> Vec<(usize, LineId, String)> {
        let mut out = Vec::new();
        for (i, (line, section)) in self.lines.iter().zip(self.sections()).enumerate() {
            if section.as_deref() != Some(section_name) {
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
        let i = self.index_of(id).ok_or(Refusal::NotFound)?;
        let section = self.lines[i].section.clone();
        let content = std::str::from_utf8(&self.lines[i].content).map_err(|_| Refusal::NotUtf8)?.to_string();
        let mut fields: Vec<String> = content.split(',').map(String::from).collect();
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
        let line = &mut self.lines[i];
        line.content = fields.join(",").into_bytes();
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
            if let Some(text) = logical_text(&line.content, sections[i].as_deref()) {
                if header_name(&text) == Some(name) {
                    return i;
                }
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
            if let Some(t) = t {
                if t <= time {
                    after = after.max(i);
                }
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
        let header = self.ensure_section("Events");
        let sections = self.sections();
        let mut after = header;
        for i in header + 1..self.lines.len() {
            if sections[i].as_deref() != Some("Events") {
                break;
            }
            if logical_text(&self.lines[i].content, Some("Events")).is_some() {
                after = i;
            }
        }
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

    /// Writes this difficulty as a new file; a file already at the target is left alone.
    pub fn save_new(&self, target: &Path) -> Result<SaveOutcome, Error> {
        if target.exists() {
            return Ok(SaveOutcome::AlreadyExists);
        }
        safe_write::write_new(target, &self.to_bytes()).map_err(Error::Write)?;
        Ok(SaveOutcome::Saved { warnings: vec![] })
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
