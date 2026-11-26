//! Reviewable map output. Planning reads snapshots; applying checks every target again.
use crate::{Document, OutputProblem, generate_receipt, mixes};
use dawsu_foundation::safe_write::{self, BackupPolicy};
use dawsu_hitsounding::{
    Hitsounds,
    copy::{CopyOptions, CopyOutcome, TargetOutcome},
    generate::{GenerateOutcome, NewDiff},
};
use dawsu_osu_format::{OsuFile, SaveOutcome};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Maximum number of individual changed lines returned per `.osu` file diff so huge diffs stay fast.
pub const DEFAULT_DIFF_MAX_LINES: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OsuDiffKind {
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsuDiffLine {
    /// Active `.osu` section header (e.g. `[TimingPoints]`, `[HitObjects]`), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// 1-based line number (in the existing file for `Removed`, in the proposed file for `Added`).
    pub line: usize,
    pub kind: OsuDiffKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsuDiffSectionCount {
    pub section: String,
    pub added: usize,
    pub removed: usize,
    pub truncated: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsuDiffSummary {
    pub byte_identical: bool,
    pub added: usize,
    pub removed: usize,
    pub truncated: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<OsuDiffSectionCount>,
    pub entries: Vec<OsuDiffLine>,
}

impl OsuDiffSummary {
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0
    }
}

fn split_osu_lines(bytes: &[u8]) -> Vec<String> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(bytes);
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect()
}

fn section_header_name(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.len() > 2 && trimmed.starts_with('[') && trimmed.ends_with(']') {
        Some(trimmed)
    } else {
        None
    }
}

struct SectionBlock<'a> {
    header: Option<&'a str>,
    start_idx: usize,
    lines: &'a [String],
}

fn split_section_blocks(lines: &[String]) -> Vec<SectionBlock<'_>> {
    if lines.is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::new();
    let mut start = 0usize;
    let mut current_header = section_header_name(&lines[0]);
    for (idx, line) in lines.iter().enumerate().skip(1) {
        if let Some(header) = section_header_name(line) {
            blocks.push(SectionBlock {
                header: current_header,
                start_idx: start,
                lines: &lines[start..idx],
            });
            start = idx;
            current_header = Some(header);
        }
    }
    blocks.push(SectionBlock {
        header: current_header,
        start_idx: start,
        lines: &lines[start..],
    });
    blocks
}

#[derive(Clone, Copy)]
enum RawEdit {
    Removed(usize),
    Added(usize),
}

enum BlockOp {
    OldOnly(usize),
    NewOnly(usize),
    Both(usize, usize),
}

fn align_section_blocks(old_blocks: &[SectionBlock<'_>], new_blocks: &[SectionBlock<'_>]) -> Vec<BlockOp> {
    let n = old_blocks.len();
    let m = new_blocks.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old_blocks[i].header == new_blocks[j].header {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old_blocks[i].header == new_blocks[j].header {
            ops.push(BlockOp::Both(i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push(BlockOp::OldOnly(i));
            i += 1;
        } else {
            ops.push(BlockOp::NewOnly(j));
            j += 1;
        }
    }
    while i < n {
        ops.push(BlockOp::OldOnly(i));
        i += 1;
    }
    while j < m {
        ops.push(BlockOp::NewOnly(j));
        j += 1;
    }
    ops
}

/// Bounded Myers O((N+M)D) shortest edit script. Returns `None` if edit distance exceeds `max_d`.
fn myers_diff(a: &[String], b: &[String], max_d: usize) -> Option<Vec<RawEdit>> {
    let n = a.len();
    let m = b.len();
    let max = (n + m).min(max_d);
    let offset = max as isize;
    let mut v = vec![0u32; 2 * max + 1];
    let mut trace: Vec<Vec<u32>> = Vec::new();
    let mut found_d = None;

    for d in 0..=max {
        let d_i = d as isize;
        for k in (-d_i..=d_i).step_by(2) {
            let idx = (k + offset) as usize;
            let mut x = if d == 0 {
                0usize
            } else if k == -d_i || (k != d_i && v[idx - 1] < v[idx + 1]) {
                v[idx + 1] as usize
            } else {
                v[idx - 1] as usize + 1
            };
            let mut y = (x as isize - k) as usize;
            while x < n && y < m && a[x] == b[y] {
                x += 1;
                y += 1;
            }
            v[idx] = x as u32;
            if x >= n && y >= m {
                trace.push(v.clone());
                found_d = Some(d);
                break;
            }
        }
        if found_d.is_some() {
            break;
        }
        trace.push(v.clone());
    }

    let found_d = found_d?;
    let mut edits = Vec::new();
    let mut x = n;
    let mut y = m;

    for d in (1..=found_d).rev() {
        let prev_v = &trace[d - 1];
        let k = x as isize - y as isize;
        let d_i = d as isize;
        let prev_k = if k == -d_i
            || (k != d_i && prev_v[(k - 1 + offset) as usize] < prev_v[(k + 1 + offset) as usize])
        {
            k + 1
        } else {
            k - 1
        };
        let prev_x = prev_v[(prev_k + offset) as usize] as usize;
        let prev_y = (prev_x as isize - prev_k) as usize;

        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
        }
        if x == prev_x {
            edits.push(RawEdit::Added(prev_y));
        } else {
            edits.push(RawEdit::Removed(prev_x));
        }
        x = prev_x;
        y = prev_y;
    }

    edits.reverse();
    Some(edits)
}

/// Linear-time fallback when a gap still exceeds `MAX_MYERS_D` edits.
fn fallback_diff(a: &[String], b: &[String]) -> Vec<RawEdit> {
    let mut edits = Vec::new();
    let shared = a.len().min(b.len());
    for i in 0..shared {
        if a[i] != b[i] {
            edits.push(RawEdit::Removed(i));
            edits.push(RawEdit::Added(i));
        }
    }
    for i in shared..a.len() {
        edits.push(RawEdit::Removed(i));
    }
    for j in shared..b.len() {
        edits.push(RawEdit::Added(j));
    }
    edits
}

/// Extracts `x,y,time,type` identity prefix of a `.osu` hit-object line (`x,y,time,type,...`).
fn hitobject_identity_prefix(line: &str) -> &str {
    let mut commas = 0usize;
    for (idx, b) in line.bytes().enumerate() {
        if b == b',' {
            commas += 1;
            if commas == 4 && idx > 0 {
                return &line[..idx];
            }
        }
    }
    line
}

/// Longest Increasing Subsequence over unique `(a_idx, b_idx)` pairs sharing `key(line)`.
fn patience_anchors<'a, F>(a: &'a [String], b: &'a [String], key: F) -> Vec<(usize, usize)>
where
    F: Fn(&'a str) -> &'a str,
{
    let mut counts_a: HashMap<&'a str, (usize, usize)> = HashMap::with_capacity(a.len());
    for (i, line) in a.iter().enumerate() {
        let entry = counts_a.entry(key(line.as_str())).or_insert((0, i));
        entry.0 += 1;
    }
    let mut counts_b: HashMap<&'a str, (usize, usize)> = HashMap::with_capacity(b.len());
    for (j, line) in b.iter().enumerate() {
        let entry = counts_b.entry(key(line.as_str())).or_insert((0, j));
        entry.0 += 1;
    }
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    for (i, line) in a.iter().enumerate() {
        let k = key(line.as_str());
        if let (Some(&(1, _)), Some(&(1, j))) = (counts_a.get(k), counts_b.get(k)) {
            candidates.push((i, j));
        }
    }
    if candidates.is_empty() {
        return Vec::new();
    }
    // Patience LIS on `j` (since candidates are already sorted by `i`).
    let mut tails: Vec<usize> = Vec::with_capacity(candidates.len());
    let mut tail_indices: Vec<usize> = Vec::with_capacity(candidates.len());
    let mut prev: Vec<Option<usize>> = vec![None; candidates.len()];
    for (idx, &(_, j)) in candidates.iter().enumerate() {
        let pos = tails.partition_point(|&tj| tj < j);
        if pos == tails.len() {
            tails.push(j);
            tail_indices.push(idx);
        } else {
            tails[pos] = j;
            tail_indices[pos] = idx;
        }
        if pos > 0 {
            prev[idx] = Some(tail_indices[pos - 1]);
        }
    }
    let mut lis = Vec::with_capacity(tail_indices.len());
    let mut cur = tail_indices.last().copied();
    while let Some(idx) = cur {
        lis.push(candidates[idx]);
        cur = prev[idx];
    }
    lis.reverse();
    lis
}

fn push_myers_or_fallback(
    a: &[String],
    b: &[String],
    offset_a: usize,
    offset_b: usize,
    out: &mut Vec<RawEdit>,
) {
    let mut p = 0usize;
    let min_len = a.len().min(b.len());
    while p < min_len && a[p] == b[p] {
        p += 1;
    }
    let mut s = 0usize;
    while s < (a.len() - p) && s < (b.len() - p) && a[a.len() - 1 - s] == b[b.len() - 1 - s] {
        s += 1;
    }
    let ra = &a[p..a.len() - s];
    let rb = &b[p..b.len() - s];
    let oa = offset_a + p;
    let ob = offset_b + p;
    if ra.is_empty() {
        out.extend((0..rb.len()).map(|j| RawEdit::Added(ob + j)));
        return;
    }
    if rb.is_empty() {
        out.extend((0..ra.len()).map(|i| RawEdit::Removed(oa + i)));
        return;
    }
    const MAX_MYERS_D: usize = 512;
    let edits = myers_diff(ra, rb, MAX_MYERS_D).unwrap_or_else(|| fallback_diff(ra, rb));
    for edit in edits {
        match edit {
            RawEdit::Removed(i) => out.push(RawEdit::Removed(oa + i)),
            RawEdit::Added(j) => out.push(RawEdit::Added(ob + j)),
        }
    }
}

fn diff_with_patience<'a, F>(
    a: &'a [String],
    b: &'a [String],
    offset_a: usize,
    offset_b: usize,
    depth: u8,
    key: F,
    out: &mut Vec<RawEdit>,
) where
    F: Copy + Fn(&'a str) -> &'a str,
{
    let mut p = 0usize;
    let min_len = a.len().min(b.len());
    while p < min_len && a[p] == b[p] {
        p += 1;
    }
    let mut s = 0usize;
    while s < (a.len() - p) && s < (b.len() - p) && a[a.len() - 1 - s] == b[b.len() - 1 - s] {
        s += 1;
    }
    let ra = &a[p..a.len() - s];
    let rb = &b[p..b.len() - s];
    let oa = offset_a + p;
    let ob = offset_b + p;
    if ra.is_empty() {
        out.extend((0..rb.len()).map(|j| RawEdit::Added(ob + j)));
        return;
    }
    if rb.is_empty() {
        out.extend((0..ra.len()).map(|i| RawEdit::Removed(oa + i)));
        return;
    }
    let anchors = patience_anchors(ra, rb, key);
    if anchors.is_empty() {
        push_myers_or_fallback(ra, rb, oa, ob, out);
        return;
    }
    let (mut prev_a, mut prev_b) = (0usize, 0usize);
    for (ai, bj) in anchors {
        let gap_a = &ra[prev_a..ai];
        let gap_b = &rb[prev_b..bj];
        if !gap_a.is_empty() || !gap_b.is_empty() {
            if depth < 2 && !gap_a.is_empty() && !gap_b.is_empty() {
                diff_with_patience(gap_a, gap_b, oa + prev_a, ob + prev_b, depth + 1, key, out);
            } else {
                push_myers_or_fallback(gap_a, gap_b, oa + prev_a, ob + prev_b, out);
            }
        }
        if ra[ai] != rb[bj] {
            out.push(RawEdit::Removed(oa + ai));
            out.push(RawEdit::Added(ob + bj));
        }
        prev_a = ai + 1;
        prev_b = bj + 1;
    }
    let tail_a = &ra[prev_a..];
    let tail_b = &rb[prev_b..];
    if !tail_a.is_empty() || !tail_b.is_empty() {
        if depth < 2 && !tail_a.is_empty() && !tail_b.is_empty() {
            diff_with_patience(tail_a, tail_b, oa + prev_a, ob + prev_b, depth + 1, key, out);
        } else {
            push_myers_or_fallback(tail_a, tail_b, oa + prev_a, ob + prev_b, out);
        }
    }
}

struct SectionBucket {
    label: String,
    section: Option<String>,
    edits: Vec<RawEdit>,
}

/// Computes a bounded, section-scoped changed-lines diff between existing and proposed `.osu` bytes.
pub fn compute_osu_line_diff(
    old_bytes: &[u8],
    new_bytes: &[u8],
    max_entries: usize,
) -> OsuDiffSummary {
    if old_bytes == new_bytes {
        return OsuDiffSummary {
            byte_identical: true,
            ..Default::default()
        };
    }
    let old_lines = split_osu_lines(old_bytes);
    let new_lines = split_osu_lines(new_bytes);
    if old_lines == new_lines {
        return OsuDiffSummary::default();
    }

    let old_blocks = split_section_blocks(&old_lines);
    let new_blocks = split_section_blocks(&new_lines);
    let ops = align_section_blocks(&old_blocks, &new_blocks);

    let mut buckets: Vec<SectionBucket> = Vec::new();
    for op in ops {
        let (section, edits) = match op {
            BlockOp::OldOnly(bi) => {
                let block = &old_blocks[bi];
                let edits = (0..block.lines.len())
                    .map(|rel| RawEdit::Removed(block.start_idx + rel))
                    .collect();
                (block.header.map(str::to_owned), edits)
            }
            BlockOp::NewOnly(bj) => {
                let block = &new_blocks[bj];
                let edits = (0..block.lines.len())
                    .map(|rel| RawEdit::Added(block.start_idx + rel))
                    .collect();
                (block.header.map(str::to_owned), edits)
            }
            BlockOp::Both(bi, bj) => {
                let ob = &old_blocks[bi];
                let nb = &new_blocks[bj];
                if ob.lines == nb.lines {
                    continue;
                }
                let mut edits = Vec::new();
                if ob.header == Some("[HitObjects]") {
                    if ob.lines.len() == nb.lines.len()
                        && ob.lines.iter().zip(nb.lines).all(|(a, b)| {
                            a == b || hitobject_identity_prefix(a) == hitobject_identity_prefix(b)
                        })
                    {
                        for (rel, (a_line, b_line)) in ob.lines.iter().zip(nb.lines).enumerate() {
                            if a_line != b_line {
                                edits.push(RawEdit::Removed(ob.start_idx + rel));
                                edits.push(RawEdit::Added(nb.start_idx + rel));
                            }
                        }
                    } else {
                        diff_with_patience(
                            ob.lines,
                            nb.lines,
                            ob.start_idx,
                            nb.start_idx,
                            0,
                            hitobject_identity_prefix,
                            &mut edits,
                        );
                    }
                } else {
                    diff_with_patience(
                        ob.lines,
                        nb.lines,
                        ob.start_idx,
                        nb.start_idx,
                        0,
                        |line| line,
                        &mut edits,
                    );
                }
                (nb.header.map(str::to_owned), edits)
            }
        };
        if edits.is_empty() {
            continue;
        }
        let label = section.clone().unwrap_or_else(|| "Header".to_owned());
        if let Some(existing) = buckets.iter_mut().find(|bucket| bucket.section == section) {
            existing.edits.extend(edits);
        } else {
            buckets.push(SectionBucket {
                label,
                section,
                edits,
            });
        }
    }

    let mut summary = OsuDiffSummary::default();
    let mut sections = Vec::with_capacity(buckets.len());
    for bucket in &buckets {
        let mut added = 0usize;
        let mut removed = 0usize;
        for edit in &bucket.edits {
            match edit {
                RawEdit::Added(_) => added += 1,
                RawEdit::Removed(_) => removed += 1,
            }
        }
        summary.added += added;
        summary.removed += removed;
        sections.push(OsuDiffSectionCount {
            section: bucket.label.clone(),
            added,
            removed,
            truncated: 0,
        });
    }
    summary.sections = sections;

    let total_edits = summary.added + summary.removed;
    if max_entries == 0 {
        for section in &mut summary.sections {
            section.truncated = section.added + section.removed;
        }
        summary.truncated = total_edits;
        return summary;
    }

    let quotas: Vec<usize> = if total_edits <= max_entries || max_entries < 16 {
        let mut rem = max_entries;
        buckets
            .iter()
            .map(|bucket| {
                let take = bucket.edits.len().min(rem);
                rem -= take;
                take
            })
            .collect()
    } else {
        // Fair water-filling budget across changed sections so huge `[TimingPoints]` / `[HitObjects]`
        // blocks never crowd out edits in other sections.
        let mut quotas = vec![0usize; buckets.len()];
        let mut remaining = max_entries;
        loop {
            let needy = quotas
                .iter()
                .enumerate()
                .filter(|&(idx, &q)| q < buckets[idx].edits.len())
                .count();
            if needy == 0 || remaining == 0 {
                break;
            }
            let share = (remaining / needy).max(1);
            let mut distributed = 0usize;
            for idx in 0..buckets.len() {
                if remaining == 0 {
                    break;
                }
                let cap = buckets[idx].edits.len() - quotas[idx];
                if cap > 0 {
                    let give = cap.min(share).min(remaining);
                    quotas[idx] += give;
                    remaining -= give;
                    distributed += give;
                }
            }
            if distributed == 0 {
                break;
            }
        }
        quotas
    };

    for (idx, &quota) in quotas.iter().enumerate() {
        summary.sections[idx].truncated = buckets[idx].edits.len().saturating_sub(quota);
    }

    for (bucket, &quota) in buckets.iter().zip(&quotas) {
        for &edit in bucket.edits.iter().take(quota) {
            match edit {
                RawEdit::Removed(idx) => summary.entries.push(OsuDiffLine {
                    section: bucket.section.clone(),
                    line: idx + 1,
                    kind: OsuDiffKind::Removed,
                    text: old_lines[idx].clone(),
                }),
                RawEdit::Added(idx) => summary.entries.push(OsuDiffLine {
                    section: bucket.section.clone(),
                    line: idx + 1,
                    kind: OsuDiffKind::Added,
                    text: new_lines[idx].clone(),
                }),
            }
        }
    }
    summary.truncated = total_edits.saturating_sub(summary.entries.len());
    summary
}

pub struct Source {
    pub file: OsuFile,
    pub hits: Hitsounds,
    pub path: PathBuf,
    pub folder_files: Vec<String>,
    pub version: u64,
    /// The open document's pending files: mixes read them and allocations skip their names.
    pub(crate) pending: crate::pending::Pending,
    /// Persistent map of mix content SHA-256 hex -> assigned custom index.
    pub(crate) export_index_schema: std::collections::BTreeMap<String, u32>,
}
impl Source {
    pub fn from_document(d: &Document) -> Self {
        Self {
            file: d.file.clone(),
            hits: d.composer.hitsounds().clone(),
            path: d.path.clone(),
            // The mapset's files as the editor sees them, pending ones included.
            folder_files: {
                let mut files = d.files.clone();
                files.extend(d.pending.files().map(|file| file.name.clone()));
                files.sort();
                files.dedup();
                files
            },
            version: d.composer.history.version(),
            pending: d.pending.clone(),
            export_index_schema: d.export_index_schema.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationCheckItem {
    pub id: String,
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileVerificationReport {
    pub passed: bool,
    pub moments_checked: usize,
    pub bodies_checked: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_time_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    pub checks: Vec<VerificationCheckItem>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFileVerificationEntry {
    pub path: String,
    pub file_name: String,
    pub is_base: bool,
    pub passed: bool,
    pub moments_checked: usize,
    pub bodies_checked: usize,
    pub byte_identical: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_time_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    pub checks: Vec<VerificationCheckItem>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportVerificationSummary {
    pub passed: bool,
    pub checked_files: usize,
    pub failed_files: usize,
    pub moments_checked: usize,
    pub bodies_checked: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_error: Option<String>,
    pub files: Vec<ExportFileVerificationEntry>,
}

#[derive(Clone)]
pub(crate) enum VerifyExpectation {
    Hitsounds(Hitsounds),
    NewEmptyDiff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RedLineSignature {
    time_bits: u64,
    beat_length_bits: u64,
    meter: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ObjectSignature {
    time_bits: u64,
    kind_tag: u8,
}

fn normalize_zero_bits(value: f64) -> u64 {
    if value == 0.0 { 0.0_f64.to_bits() } else { value.to_bits() }
}

fn red_line_signatures(file: &OsuFile) -> Vec<RedLineSignature> {
    let mut sigs: Vec<RedLineSignature> = file
        .timing_points()
        .iter()
        .filter(|tp| tp.uninherited())
        .map(|tp| RedLineSignature {
            time_bits: normalize_zero_bits(tp.time()),
            beat_length_bits: normalize_zero_bits(tp.beat_length()),
            meter: tp.meter().unwrap_or(4),
        })
        .collect();
    sigs.sort_by_key(|s| (s.time_bits, s.beat_length_bits, s.meter));
    sigs
}

fn object_signatures(file: &OsuFile) -> Vec<ObjectSignature> {
    file.hit_objects()
        .iter()
        .map(|obj| {
            let kind_tag = match obj.kind() {
                dawsu_osu_format::ObjectKind::Circle => 0,
                dawsu_osu_format::ObjectKind::Slider(_) => 1,
                dawsu_osu_format::ObjectKind::Spinner { .. } => 2,
                dawsu_osu_format::ObjectKind::Hold { .. } => 3,
            };
            ObjectSignature {
                time_bits: normalize_zero_bits(obj.time()),
                kind_tag,
            }
        })
        .collect()
}

fn warning_counts_from_slice(warnings: &[dawsu_osu_format::ReadWarning]) -> BTreeMap<(String, String), usize> {
    let mut counts = BTreeMap::new();
    for warning in warnings {
        *counts
            .entry((warning.section.clone(), warning.reason.clone()))
            .or_insert(0) += 1;
    }
    counts
}

fn warning_counts_from_file(file: &OsuFile) -> BTreeMap<(String, String), usize> {
    let bytes = if file.original_bytes().is_empty() {
        file.to_bytes()
    } else {
        file.original_bytes().to_vec()
    };
    warning_counts_from_slice(&OsuFile::read(bytes).warnings)
}

pub struct PlannedFile {
    pub path: PathBuf,
    pub file: OsuFile,
    pub expected: Option<Vec<u8>>,
    diff_cache: OnceLock<OsuDiffSummary>,
    verify_expectation: VerifyExpectation,
    is_generated_base: bool,
    source_red_lines: Vec<RedLineSignature>,
    source_objects: Option<Vec<ObjectSignature>>,
    source_warning_counts: BTreeMap<(String, String), usize>,
    verification: Option<FileVerificationReport>,
}
impl PlannedFile {
    fn new(
        path: PathBuf,
        file: OsuFile,
        expected: Option<Vec<u8>>,
        verify_expectation: VerifyExpectation,
        is_generated_base: bool,
        baseline_file: Option<&OsuFile>,
        baseline_warning_counts: Option<BTreeMap<(String, String), usize>>,
    ) -> Self {
        let source_red_lines = red_line_signatures(baseline_file.unwrap_or(&file));
        let source_objects = (expected.is_some() && !is_generated_base)
            .then(|| object_signatures(baseline_file.unwrap_or(&file)));
        let source_warning_counts = baseline_warning_counts
            .or_else(|| baseline_file.map(warning_counts_from_file))
            .unwrap_or_default();
        Self {
            path,
            file,
            expected,
            diff_cache: OnceLock::new(),
            verify_expectation,
            is_generated_base,
            source_red_lines,
            source_objects,
            source_warning_counts,
            verification: None,
        }
    }

    /// Cached changed-lines diff between the on-disk snapshot (`expected`) and the planned `.osu`.
    pub fn diff_summary(&self) -> &OsuDiffSummary {
        self.diff_cache.get_or_init(|| {
            let old = self.expected.as_deref().unwrap_or(&[]);
            let new = self.file.to_bytes();
            compute_osu_line_diff(old, &new, DEFAULT_DIFF_MAX_LINES)
        })
    }

    /// Replaces the planned `.osu` AST (for example after volume-curve, tick-render, or green-line cleanup)
    /// and clears both cached diff and verification so future reads recompute cleanly.
    pub fn replace_file(&mut self, file: OsuFile) {
        self.file = file;
        self.diff_cache = OnceLock::new();
        self.verification = None;
    }

    pub(crate) fn expected_hitsounds(&self) -> Option<&Hitsounds> {
        match &self.verify_expectation {
            VerifyExpectation::Hitsounds(hits) => Some(hits),
            VerifyExpectation::NewEmptyDiff => None,
        }
    }

    pub(crate) fn set_verify_hitsounds(&mut self, hits: Hitsounds) {
        self.verify_expectation = VerifyExpectation::Hitsounds(hits);
    }

    pub fn verification(&self) -> Option<&FileVerificationReport> {
        self.verification.as_ref()
    }
}

struct VerificationContext<'a> {
    root: Option<&'a Path>,
    is_hitsound_only: bool,
    skip_paths: Vec<&'a Path>,
    planned_write_names: Vec<String>,
    planned_write_lower: HashSet<String>,
    newly_created_mix_names: Vec<String>,
    staged_validity_error: Option<String>,
    after_disk_files: Vec<String>,
    editor_files: Vec<String>,
    pending_snapshot: Option<&'a crate::pending::Pending>,
}

fn verify_single_planned_file(
    planned: &PlannedFile,
    pre_envelope_error: Option<&(Option<f64>, String)>,
    ctx: &VerificationContext<'_>,
) -> FileVerificationReport {
    let bytes = planned.file.to_bytes();
    let reparsed = OsuFile::read(bytes.clone());
    let mut checks = Vec::with_capacity(4);
    let mut failure_time_ms: Option<f64> = None;

    let parse_check = match reparsed.file.format_version() {
        None => VerificationCheckItem {
            id: "parse".into(),
            label: "Valid .osu file".into(),
            passed: false,
            detail: "Planned .osu text is missing a valid osu file format header.".into(),
        },
        Some(version) => {
            if reparsed.file.to_bytes() != bytes {
                VerificationCheckItem {
                    id: "parse".into(),
                    label: "Valid .osu file".into(),
                    passed: false,
                    detail: "Re-reading planned .osu produced different bytes.".into(),
                }
            } else {
                let new_counts = warning_counts_from_slice(&reparsed.warnings);
                let extra_warning = reparsed.warnings.iter().find(|w| {
                    let key = (w.section.clone(), w.reason.clone());
                    let new_n = new_counts.get(&key).copied().unwrap_or(0);
                    let old_n = planned.source_warning_counts.get(&key).copied().unwrap_or(0);
                    new_n > old_n
                });
                if let Some(w) = extra_warning {
                    VerificationCheckItem {
                        id: "parse".into(),
                        label: "Valid .osu file".into(),
                        passed: false,
                        detail: format!(
                            "Re-reading planned .osu raised a new {} warning (line {}): {}",
                            w.section, w.line_number, w.reason
                        ),
                    }
                } else {
                    VerificationCheckItem {
                        id: "parse".into(),
                        label: "Valid .osu file".into(),
                        passed: true,
                        detail: format!("Parsed osu file format v{version} cleanly."),
                    }
                }
            }
        }
    };
    let parse_ok = parse_check.passed;
    checks.push(parse_check);

    let structure_check = if reparsed.file.format_version().is_none() {
        VerificationCheckItem {
            id: "structure".into(),
            label: "Hit objects & red lines".into(),
            passed: false,
            detail: "Skipped: valid .osu file header required first.".into(),
        }
    } else {
        let reparsed_reds = red_line_signatures(&reparsed.file);
        let expected_objects = planned.file.hit_objects().len();
        let actual_objects = reparsed.file.hit_objects().len();
        if reparsed_reds != planned.source_red_lines {
            VerificationCheckItem {
                id: "structure".into(),
                label: "Hit objects & red lines".into(),
                passed: false,
                detail: "Uninherited timing red lines changed or went missing when re-reading planned .osu.".into(),
            }
        } else if let Some(ref expected_sigs) = planned.source_objects
            && object_signatures(&reparsed.file) != *expected_sigs
        {
            VerificationCheckItem {
                id: "structure".into(),
                label: "Hit objects & red lines".into(),
                passed: false,
                detail: "Gameplay hit objects changed count, timestamp, or type in planned .osu.".into(),
            }
        } else if actual_objects != expected_objects {
            VerificationCheckItem {
                id: "structure".into(),
                label: "Hit objects & red lines".into(),
                passed: false,
                detail: format!("Hit object count mismatch ({expected_objects} planned, {actual_objects} re-read)."),
            }
        } else if planned.is_generated_base
            && !reparsed
                .file
                .value("Metadata", "Version")
                .is_some_and(|v| !v.trim().is_empty())
        {
            VerificationCheckItem {
                id: "structure".into(),
                label: "Hit objects & red lines".into(),
                passed: false,
                detail: "Base hitsound difficulty is missing its Version name.".into(),
            }
        } else {
            VerificationCheckItem {
                id: "structure".into(),
                label: "Hit objects & red lines".into(),
                passed: true,
                detail: format!(
                    "{actual_objects} hit object(s), {} red line(s) verified.",
                    reparsed_reds.len()
                ),
            }
        }
    };
    checks.push(structure_check);

    let (moments_checked, bodies_checked) = match &planned.verify_expectation {
        VerifyExpectation::Hitsounds(expected) => (expected.moments().len(), expected.bodies().len()),
        VerifyExpectation::NewEmptyDiff => (0, 0),
    };
    let hitsounds_check = if let Some((err_time, err)) = pre_envelope_error {
        failure_time_ms = *err_time;
        VerificationCheckItem {
            id: "hitsounds".into(),
            label: "Hitsounds & volumes".into(),
            passed: false,
            detail: err.clone(),
        }
    } else if reparsed.file.format_version().is_none() {
        let _ = parse_ok;
        VerificationCheckItem {
            id: "hitsounds".into(),
            label: "Hitsounds & volumes".into(),
            passed: false,
            detail: "Skipped: valid .osu file header required first.".into(),
        }
    } else {
        match &planned.verify_expectation {
            VerifyExpectation::Hitsounds(expected_hits) => {
                match dawsu_hitsounding::verify_compiled_osu(&bytes, expected_hits, &ctx.after_disk_files) {
                    Ok(()) => VerificationCheckItem {
                        id: "hitsounds".into(),
                        label: "Hitsounds & volumes".into(),
                        passed: true,
                        detail: format!(
                            "Matched {moments_checked} hit moment(s) and {bodies_checked} slider body(s)."
                        ),
                    },
                    Err(mismatch) => {
                        failure_time_ms = mismatch.time_ms;
                        let detail = match mismatch.time_ms {
                            Some(t) => format!("Mismatch at {t:.0} ms: {}", mismatch.detail),
                            None => mismatch.detail,
                        };
                        VerificationCheckItem {
                            id: "hitsounds".into(),
                            label: "Hitsounds & volumes".into(),
                            passed: false,
                            detail,
                        }
                    }
                }
            }
            VerifyExpectation::NewEmptyDiff => {
                match dawsu_hitsounding::decompile(
                    &reparsed.file,
                    &ctx.after_disk_files,
                    &|| false,
                    &mut |_, _| {},
                ) {
                    dawsu_hitsounding::Decompiled::Done(done) => {
                        if done.moments().is_empty() && done.bodies().is_empty() {
                            VerificationCheckItem {
                                id: "hitsounds".into(),
                                label: "Hitsounds & volumes".into(),
                                passed: true,
                                detail: "Empty difficulty verified with 0 hitsounds.".into(),
                            }
                        } else {
                            VerificationCheckItem {
                                id: "hitsounds".into(),
                                label: "Hitsounds & volumes".into(),
                                passed: false,
                                detail: format!(
                                    "Expected an empty difficulty, but re-reading found {} hit moment(s) and {} slider body(s).",
                                    done.moments().len(),
                                    done.bodies().len(),
                                ),
                            }
                        }
                    }
                    dawsu_hitsounding::Decompiled::UnsupportedMode { mode } => VerificationCheckItem {
                        id: "hitsounds".into(),
                        label: "Hitsounds & volumes".into(),
                        passed: false,
                        detail: format!("Compiled output parsed as unsupported game mode {mode}."),
                    },
                    dawsu_hitsounding::Decompiled::Cancelled => {
                        unreachable!("read-back verification does not cancel")
                    }
                }
            }
        }
    };
    checks.push(hitsounds_check);

    let samples_check = if reparsed.file.format_version().is_none() {
        VerificationCheckItem {
            id: "samples".into(),
            label: "Sample files".into(),
            passed: false,
            detail: "Skipped: valid .osu file header required first.".into(),
        }
    } else {
        match ctx.root {
            Some(root) if root.exists() => {
                let staged_res = if let Some(ref err) = ctx.staged_validity_error {
                    Err(err.clone())
                } else {
                    Ok(())
                };
                let written_res = if let Some(pending) = ctx.pending_snapshot {
                    crate::pending::check_written(
                        &reparsed.file,
                        root,
                        &ctx.planned_write_names,
                        &ctx.editor_files,
                        pending,
                    )
                } else {
                    Ok(())
                };
                let index_res = if ctx.is_hitsound_only {
                    Ok(())
                } else {
                    crate::pending::check_indexes_free_except(
                        root,
                        &ctx.skip_paths,
                        &ctx.newly_created_mix_names,
                    )
                };
                match staged_res.and(written_res).and(index_res) {
                    Ok(()) => {
                        let played =
                            crate::pending::played_names(&reparsed.file, &ctx.after_disk_files);
                        let detail = if ctx.is_hitsound_only {
                            let existing_custom_count = played
                                .iter()
                                .filter(|n| !ctx.planned_write_lower.contains(&n.to_ascii_lowercase()))
                                .count();
                            if existing_custom_count > 0 {
                                format!(
                                    "Checked {} staged sample(s) for isolated hitsound-only export ({existing_custom_count} existing mapset sample(s) stay in original folder).",
                                    ctx.planned_write_names.len()
                                )
                            } else {
                                format!(
                                    "Checked {} sample reference(s) for isolated hitsound-only export ({} staged file(s); sibling mapset index check skipped).",
                                    played.len(),
                                    ctx.planned_write_names.len()
                                )
                            }
                        } else {
                            format!(
                                "Verified {} played sample reference(s) and custom sample indexes.",
                                played.len()
                            )
                        };
                        VerificationCheckItem {
                            id: "samples".into(),
                            label: "Sample files".into(),
                            passed: true,
                            detail,
                        }
                    }
                    Err(err) => VerificationCheckItem {
                        id: "samples".into(),
                        label: "Sample files".into(),
                        passed: false,
                        detail: err,
                    },
                }
            }
            _ => VerificationCheckItem {
                id: "samples".into(),
                label: "Sample files".into(),
                passed: false,
                detail: "Missing mapset folder for sample verification.".into(),
            },
        }
    };
    checks.push(samples_check);

    let first_failed = checks.iter().find(|c| !c.passed);
    let passed = first_failed.is_none();
    let failure_message = first_failed.map(|c| format!("{}: {}", c.label, c.detail));
    if !first_failed.is_some_and(|c| c.id == "hitsounds") {
        failure_time_ms = None;
    }
    FileVerificationReport {
        passed,
        moments_checked,
        bodies_checked,
        failure_time_ms,
        failure_message,
        checks,
    }
}
/// What Copy did to one target, counted for the Export window.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetCounts {
    pub path: PathBuf,
    /// Source moments matched to a target moment.
    pub matched: usize,
    /// Source moments with no target moment within Match within.
    pub unmatched: usize,
    /// Target moments given a new sound or volume.
    pub changed: usize,
    /// Target moments where a later source moment replaced an earlier one.
    pub collisions: usize,
    /// Target moments that kept their own sound to avoid a silent hitnormal.
    pub kept_from_silent: usize,
    /// Slider and spinner ends muted.
    pub muted: usize,
    /// Unmatched target gameplay object edges given the optional fallback sound.
    pub fallback_applied: usize,
}

pub struct WritePlan {
    pub label: String,
    pub files: Vec<PlannedFile>,
    pub(super) mixes: Vec<mixes::StagedMix>,
    /// The pending files the planned difficulties play, written with the mixes.
    pub(super) pending: Vec<mixes::StagedMix>,
    /// Where the files this plan creates are recorded.
    pub(super) ledger: Option<crate::pending::Ledger>,
    pub(super) combinations: Vec<crate::ExportCombination>,
    pub(super) generation: Option<PathBuf>,
    /// The difficulty Generate makes, and how many of the first `mixes` are Generate's own.
    /// Only these go into Generate's receipt; Copy's files and mixes never do.
    pub(super) generated: Option<(PathBuf, usize)>,
    include_slider_audio: bool,
    pub details: Vec<String>,
    /// Copy's counts per target, in target order. Empty for Generate.
    pub targets: Vec<TargetCounts>,
    /// The target difficulties whose per-difficulty project should start with HS mode on.
    pub(super) hs_mode_targets: Vec<PathBuf>,
    /// Non-blocking osu! Ranking Criteria & playback warnings for newly staged mixes.
    pub sample_warnings: Vec<crate::sample_checks::SampleHealthReport>,
    /// Updated custom-index stability schema produced while staging mixes.
    pub(super) export_index_schema: std::collections::BTreeMap<String, u32>,
    /// Slider tick green lines injected by `TickPolicy::Render`, keyed by target filename.
    pub(super) tick_writes: std::collections::BTreeMap<String, Vec<dawsu_hitsounding::tick_render::TickWrite>>,
    pub verification: ExportVerificationSummary,
    pub(crate) pending_snapshot: Option<crate::pending::Pending>,
    pub source: PathBuf,
    pub version: u64,
}
impl WritePlan {
    pub fn export_combinations(&self) -> &[crate::ExportCombination] {
        &self.combinations
    }

    pub fn includes_slider_audio(&self) -> bool {
        self.include_slider_audio
    }

    /// The base hitsound difficulty this plan writes, if Generate is part of it.
    pub fn generated_file(&self) -> Option<&Path> {
        self.generated.as_ref().map(|(path, _)| path.as_path())
    }

    pub fn mix_paths(&self) -> impl Iterator<Item = &Path> {
        self.mixes.iter().map(|mix| mix.path.as_path())
    }

    pub fn created_mix_paths(&self) -> impl Iterator<Item = &Path> {
        self.mixes
            .iter()
            .filter(|mix| !mix.is_reused())
            .map(|mix| mix.path.as_path())
    }

    pub fn reused_mix_paths(&self) -> impl Iterator<Item = &Path> {
        self.mixes
            .iter()
            .filter(|mix| mix.is_reused())
            .map(|mix| mix.path.as_path())
    }

    /// Plain-English summary of per-object `.osu` volume updates, timing-line changes, and
    /// created vs. reused `.wav` mixes for the Write Review modal.
    pub fn volume_clarity_lines(&self) -> Vec<String> {
        let mut object_changes = 0usize;
        let mut timing_changes = 0usize;
        for target in &self.files {
            let summary = target.file.change_summary();
            object_changes += summary.hit_objects_changed;
            timing_changes += summary.green_lines_added
                + summary.green_lines_edited
                + summary.green_lines_removed;
        }
        let created_wav = self.created_mix_paths().count();
        let reused_wav = self.reused_mix_paths().count();
        let mut lines = Vec::new();
        if object_changes > 0 && timing_changes == 0 {
            lines.push(format!(
                "Per-object .osu volumes only: {object_changes} object(s) updated, 0 new timing lines needed."
            ));
        } else if timing_changes > 0 {
            lines.push(format!(
                "Per-object .osu changes: {object_changes} | Timing-line volume changes: {timing_changes} (used where shared moments or slider bodies need distinct volumes)."
            ));
        }
        if created_wav > 0 || reused_wav > 0 {
            let ext = if self.mixes.iter().all(|mix| {
                mix.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ogg"))
            }) {
                ".ogg"
            } else if self.mixes.iter().all(|mix| {
                mix.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav"))
            }) {
                ".wav"
            } else {
                "sample file(s)"
            };
            lines.push(format!(
                "Sample mixes: {created_wav} new {ext} created, {reused_wav} existing {ext} reused."
            ));
        }
        lines
    }

    /// Plain-English osu! Ranking Criteria warnings and auto-fix notes for newly staged sample mixes.
    pub fn sample_warning_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .sample_warnings
            .iter()
            .flat_map(crate::sample_checks::SampleHealthReport::warnings)
            .collect();
        for mix in self.mixes.iter().filter(|mix| !mix.is_reused()) {
            let trimmed_ms = mix.trimmed_leading_silence_ms();
            if trimmed_ms >= 0.05 {
                let name = mix
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| mix.path.display().to_string());
                lines.push(format!(
                    "{name}: auto-fix trimmed {trimmed_ms:.1} ms of leading silence (layered sounds inside this mix moved earlier)."
                ));
            }
        }
        lines
    }

    /// Inspects newly staged mixes in this plan and records any RC health warnings.
    pub(super) fn check_staged_mixes(&mut self) {
        self.sample_warnings.clear();
        for mix in self.mixes.iter().filter(|mix| !mix.is_reused()) {
            let name = mix
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| mix.path.display().to_string());
            if let Some(report) = crate::sample_checks::inspect_wav_bytes(name, mix.wav_bytes())
                && !report.is_healthy()
            {
                self.sample_warnings.push(report);
            }
        }
    }

    pub(super) fn mix_clip(&self, index: usize) -> Option<dawsu_audio::AudioClip> {
        self.mixes.get(index).map(mixes::StagedMix::clip)
    }

    pub(crate) fn clean_redundant_greenlines(&mut self) -> usize {
        let mut total = 0;
        for target in &mut self.files {
            let cleaned = dawsu_hitsounding::cleanup::clean_up_green_lines(&target.file);
            if cleaned.removed > 0 {
                target.replace_file(cleaned.file);
                total += cleaned.removed;
            }
        }
        if total > 0 {
            let noun = if total == 1 { "greenline" } else { "greenlines" };
            self.details.push(format!("Removed {total} redundant {noun}."));
        }
        total
    }

    /// The names of the pending files this plan writes into the mapset.
    pub(crate) fn pending_names(&self) -> Vec<String> {
        self.pending.iter().filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)).collect()
    }

    pub(crate) fn folder_listing_after_writes(&self) -> Vec<String> {
        let pending = crate::pending::Pending::default();
        let mut files = self
            .source
            .parent()
            .filter(|root| root.exists())
            .and_then(|root| crate::pending::listing(root, &pending).ok())
            .unwrap_or_default();
        for mix in self.mixes.iter().chain(&self.pending) {
            if let Some(name) = mix.path.file_name().and_then(|n| n.to_str()) {
                files.push(name.to_owned());
            }
        }
        files.sort();
        files.dedup();
        files
    }

    fn verification_context<'a>(
        &'a self,
        is_hitsound_only: bool,
        include_mix_indexes: bool,
        pending_override: Option<&'a crate::pending::Pending>,
    ) -> VerificationContext<'a> {
        let root = self.source.parent();
        let pending_snapshot = pending_override.or(self.pending_snapshot.as_ref());
        let mix_names: Vec<String> = self
            .mixes
            .iter()
            .filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned))
            .collect();
        let pending_names = self.pending_names();
        let mut newly_created_mix_names: Vec<String> = if include_mix_indexes {
            self.mixes
                .iter()
                .filter(|mix| !mix.is_reused())
                .filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned))
                .collect()
        } else {
            Vec::new()
        };
        newly_created_mix_names.extend(pending_names.iter().cloned());
        let staged_validity_error = self
            .mixes
            .iter()
            .chain(&self.pending)
            .find_map(|mix| {
                let name = mix
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| "Staged sample is missing a valid filename.".to_owned());
                match name {
                    Err(e) => Some(e),
                    Ok(n) => {
                        if dawsu_osu_format::check_map_path(n).is_err() {
                            return Some(format!("Staged sample `{n}` has an unsafe filename."));
                        }
                        if mix.wav_bytes().is_empty() {
                            return Some(format!("Staged sample `{n}` has 0 bytes."));
                        }
                        None
                    }
                }
            });
        let planned_write_names = [mix_names.clone(), pending_names].concat();
        let planned_write_lower: HashSet<String> = planned_write_names
            .iter()
            .map(|n| n.to_ascii_lowercase())
            .collect();
        let mut after_disk_files = root
            .filter(|r| r.exists())
            .and_then(|r| {
                let empty_pending = crate::pending::Pending::default();
                crate::pending::listing(r, &empty_pending).ok()
            })
            .unwrap_or_default();
        after_disk_files.extend(planned_write_names.iter().cloned());
        after_disk_files.sort();
        after_disk_files.dedup();

        let mut editor_files = match (root.filter(|r| r.exists()), pending_snapshot) {
            (Some(r), Some(pending)) => crate::pending::listing(r, pending).unwrap_or_else(|_| after_disk_files.clone()),
            _ => after_disk_files.clone(),
        };
        editor_files.extend(mix_names);
        editor_files.sort();
        editor_files.dedup();

        VerificationContext {
            root,
            is_hitsound_only,
            skip_paths: self.files.iter().map(|f| f.path.as_path()).collect(),
            planned_write_names,
            planned_write_lower,
            newly_created_mix_names,
            staged_validity_error,
            after_disk_files,
            editor_files,
            pending_snapshot,
        }
    }

    /// Runs the full 4-part read-back verification across every planned `.osu` file and pre-warms
    /// every cached `.osu` line diff. Stores the result in `self.verification` even if any check fails.
    pub(crate) fn verify_and_finalize(
        &mut self,
        source: &Source,
        pre_envelope_errors: &HashMap<PathBuf, (Option<f64>, String)>,
        is_hitsound_only: bool,
    ) -> ExportVerificationSummary {
        self.pending_snapshot = Some(source.pending.clone());
        let byte_identicals: Vec<bool> = self
            .files
            .iter()
            .map(|target| target.diff_summary().byte_identical)
            .collect();
        let reports: Vec<FileVerificationReport> = {
            let ctx = self.verification_context(is_hitsound_only, true, Some(&source.pending));
            self.files
                .iter()
                .map(|target| {
                    verify_single_planned_file(
                        target,
                        pre_envelope_errors.get(&target.path),
                        &ctx,
                    )
                })
                .collect()
        };
        let mut files = Vec::with_capacity(self.files.len());
        let mut failed_files = 0usize;
        let mut moments_checked = 0usize;
        let mut bodies_checked = 0usize;
        let mut first_error = None;
        for ((target, report), byte_identical) in self
            .files
            .iter_mut()
            .zip(reports)
            .zip(byte_identicals)
        {
            if !report.passed {
                failed_files += 1;
                if first_error.is_none() {
                    let msg = report.failure_message.clone().unwrap_or_else(|| {
                        report
                            .checks
                            .iter()
                            .find(|c| !c.passed)
                            .map(|c| format!("{}: {}", c.label, c.detail))
                            .unwrap_or_else(|| "unknown verification failure".into())
                    });
                    first_error = Some(format!("{}: {msg}", display_file_name(&target.path)));
                }
            }
            moments_checked += report.moments_checked;
            bodies_checked += report.bodies_checked;
            let entry = ExportFileVerificationEntry {
                path: target.path.display().to_string(),
                file_name: display_file_name(&target.path),
                is_base: target.is_generated_base,
                passed: report.passed,
                moments_checked: report.moments_checked,
                bodies_checked: report.bodies_checked,
                byte_identical,
                failure_time_ms: report.failure_time_ms,
                failure_message: report.failure_message.clone(),
                checks: report.checks.clone(),
            };
            target.verification = Some(report);
            files.push(entry);
        }
        let summary = ExportVerificationSummary {
            passed: failed_files == 0,
            checked_files: files.len(),
            failed_files,
            moments_checked,
            bodies_checked,
            first_error,
            files,
        };
        self.verification = summary.clone();
        summary
    }

    /// Hard pre-write verification gate for both `WritePlan::apply` and isolated hitsound-only export.
    pub fn verify_planned_files_for_export(&self, is_hitsound_only: bool) -> Result<(), String> {
        if !is_hitsound_only {
            for target in &self.files {
                let current = std::fs::read(&target.path).ok();
                if current != target.expected {
                    return Err(format!(
                        "{} changed since this review. Nothing was written.",
                        target.path.display()
                    ));
                }
            }
        }
        for target in &self.files {
            if let Some(report) = target.verification.as_ref().filter(|r| !r.passed) {
                let reason = report
                    .checks
                    .iter()
                    .find(|c| !c.passed)
                    .map(|c| format!("{}: {}", c.label, c.detail))
                    .or_else(|| report.failure_message.clone())
                    .unwrap_or_else(|| "unknown verification failure".into());
                return Err(format!(
                    "Save verification stopped before writing {}: {reason}",
                    target.path.display()
                ));
            }
        }
        let ctx = self.verification_context(is_hitsound_only, false, None);
        for target in &self.files {
            let report = verify_single_planned_file(target, None, &ctx);
            if !report.passed {
                let reason = report
                    .checks
                    .iter()
                    .find(|c| !c.passed)
                    .map(|c| format!("{}: {}", c.label, c.detail))
                    .unwrap_or_else(|| "unknown verification failure".into());
                return Err(format!(
                    "Save verification stopped before writing {}: {reason}",
                    target.path.display()
                ));
            }
        }
        Ok(())
    }

    /// Hard pre-write verification gate for Direct `WritePlan::apply`.
    pub fn verify_planned_files_for_apply(&self) -> Result<(), String> {
        self.verify_planned_files_for_export(false)
    }

    /// Adds the pending files the planned difficulties play, then checks that each plays from
    /// the mapset after the write what the editor plays (the Save check).
    pub(super) fn add_pending(&mut self, source: &Source) -> Result<(), String> {
        let root = source.path.parent().ok_or("Missing mapset folder")?;
        let mix_names: Vec<String> = self.mixes.iter()
            .filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)).collect();
        let mut used = std::collections::BTreeMap::new();
        for target in &self.files {
            for (name, bytes) in crate::pending::used(&target.file, root, &source.pending, &mix_names)? {
                used.entry(name.to_ascii_lowercase()).or_insert((name, bytes));
            }
        }
        self.ledger = source.pending.ledger();
        self.pending_snapshot = Some(source.pending.clone());
        self.pending = used.into_values().map(|(name, bytes)| mixes::StagedMix::pending(root, &name, bytes)).collect();
        let pending_names = self.pending_names();
        let skip: Vec<&Path> = self.files.iter().map(|f| f.path.as_path()).collect();
        crate::pending::check_indexes_free_except(root, &skip, &pending_names)?;
        let writes = [mix_names.clone(), pending_names].concat();
        let mut editor = crate::pending::listing(root, &source.pending)?;
        editor.extend(mix_names);
        editor.sort();
        for target in &self.files {
            crate::pending::check_written(&target.file, root, &writes, &editor, &source.pending)?;
        }
        Ok(())
    }
    pub fn apply(&self, backups: &Path, cancel: &dyn Fn() -> bool) -> Result<usize, String> {
        self.verify_planned_files_for_apply()?;
        // One all-or-nothing write: the mixes, then the pending files the difficulties play.
        let publication = mixes::publish(self.mixes.iter().chain(&self.pending), self.ledger.as_ref(), cancel)?;
        let policy = BackupPolicy::new(backups, 20);
        let mut written: Vec<&PlannedFile> = Vec::new();
        for target in &self.files {
            let outcome = if cancel() {
                Err("Write cancelled".into())
            } else {
                let result = if let Some(expected) = &target.expected {
                    target
                        .file
                        .save_over_expected(&target.path, expected, &policy)
                } else {
                    safe_write::create(&target.path, &target.file.to_bytes())
                        .map(|()| SaveOutcome::Saved { warnings: Vec::new() })
                        .map_err(dawsu_osu_format::Error::Write)
                };
                result
                    .map_err(|e| e.to_string())
                    .and_then(|outcome| match outcome {
                        SaveOutcome::Saved { .. } | SaveOutcome::NothingToSave => Ok(()),
                        other => Err(format!("{}: {}", target.path.display(), crate::refusals::write(&other))),
                    })
            };
            if let Err(error) = outcome {
                let mut kept = Vec::new();
                for previous in written.into_iter().rev() {
                    let now = std::fs::read(&previous.path).ok();
                    if now.as_deref() != Some(previous.file.to_bytes().as_slice()) {
                        kept.push(previous.path.display().to_string());
                        continue;
                    }
                    if let Some(original) = &previous.expected {
                        let restore = OsuFile::read(original.clone()).file.save_over_expected(
                            &previous.path,
                            &previous.file.to_bytes(),
                            &policy,
                        );
                        if !matches!(
                            restore,
                            Ok(SaveOutcome::Saved { .. } | SaveOutcome::NothingToSave)
                        ) {
                            kept.push(previous.path.display().to_string());
                        }
                    } else if std::fs::remove_file(&previous.path).is_err() {
                        kept.push(previous.path.display().to_string());
                    }
                }
                // A target that could not be restored may still reference these files.
                if kept.is_empty() {
                    return Err(publication.failed(format!("{error}. Previous targets were restored.")));
                }
                publication.keep();
                return Err(format!(
                    "{error}. These files had newer changes or could not be restored and were kept: {}",
                    kept.join(", ")
                ));
            }
            written.push(target);
        }
        publication.keep();
        Ok(written.len())
    }
}
fn display_file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
fn folder(source: &Source) -> Result<Vec<(String, Option<String>)>, String> {
    let root = source.path.parent().ok_or("Missing mapset folder")?;
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("osu"))
        {
            let file = OsuFile::read(std::fs::read(&path).map_err(|e| e.to_string())?).file;
            result.push((
                display_file_name(&path),
                file.value("Metadata", "Version"),
            ));
        }
    }
    Ok(result)
}
pub fn generate(source: &Source, cancel: &dyn Fn() -> bool) -> Result<WritePlan, String> {
    generate_report(source, Path::new(""), None, None, false, cancel, &mut Vec::new(), &mut |_, _| {})
}
#[allow(clippy::too_many_arguments)]
pub(super) fn generate_report(
    source: &Source,
    defaults: &Path,
    settings: Option<&crate::ExportSettings>,
    projects: Option<&Path>,
    include_slider_audio: bool,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<WritePlan, String> {
    generate_report_with_options_and_format(
        source,
        defaults,
        settings,
        projects,
        include_slider_audio,
        false,
        mixes::MixFormat::Wav16,
        cancel,
        diagnostics,
        progress,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn generate_report_with_options(
    source: &Source,
    defaults: &Path,
    settings: Option<&crate::ExportSettings>,
    projects: Option<&Path>,
    include_slider_audio: bool,
    auto_fix_staged_mixes: bool,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<WritePlan, String> {
    generate_report_with_options_and_format(
        source,
        defaults,
        settings,
        projects,
        include_slider_audio,
        auto_fix_staged_mixes,
        mixes::MixFormat::Wav16,
        cancel,
        diagnostics,
        progress,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn generate_report_with_options_and_format(
    source: &Source,
    defaults: &Path,
    settings: Option<&crate::ExportSettings>,
    projects: Option<&Path>,
    include_slider_audio: bool,
    auto_fix_staged_mixes: bool,
    format: mixes::MixFormat,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<WritePlan, String> {
    generate_report_for_version(
        source,
        dawsu_hitsounding::generate::HITSOUNDS_VERSION,
        defaults,
        settings,
        projects,
        include_slider_audio,
        auto_fix_staged_mixes,
        format,
        cancel,
        diagnostics,
        progress,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn generate_report_for_version(
    source: &Source,
    version: &str,
    defaults: &Path,
    settings: Option<&crate::ExportSettings>,
    projects: Option<&Path>,
    include_slider_audio: bool,
    auto_fix_staged_mixes: bool,
    format: mixes::MixFormat,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<WritePlan, String> {
    let name = source
        .path
        .file_name()
        .ok_or("Missing source filename")?
        .to_string_lossy();
    let root = source.path.parent().ok_or("Missing mapset folder")?;
    let file_name = dawsu_hitsounding::generate::file_name_for(
        &source.file, &name, version,
    );
    let generation = projects.map(|projects| generate_receipt::path(projects, &source.path));
    let previous = if version.eq_ignore_ascii_case(dawsu_hitsounding::generate::HITSOUNDS_VERSION) {
        generate_receipt::previous(generation.as_deref(), &root.join(&file_name))?
    } else {
        None
    };
    let mut siblings = folder(source)?;
    if previous.is_some() {
        siblings.retain(|(sibling, _)| sibling != &file_name);
    }
    match dawsu_hitsounding::generate::generate_with_version(
        &source.hits,
        &source.file,
        &name,
        version,
        &siblings,
        cancel,
        progress,
    ) {
        GenerateOutcome::Planned {
            file,
            file_name,
            problems,
            expected_hitsounds,
        } => {
            let target = root.join(file_name);
            let old_bytes = previous.as_ref().map_or(&[][..], |output| output.bytes.as_slice());
            let old_mixes = previous.as_ref().map_or(&[][..], |output| output.mixes.as_slice());
            let can_mix = problems.iter().all(|problem| {
                mixes::can_render(problem, dawsu_hitsounding::Placement::OnObjects)
            });
            let staged = if problems.is_empty() {
                Ok((file, mixes::MixFiles::default()))
            } else if can_mix {
                crate::mixes::stage_in_file_with_options(
                    &source.hits,
                    file,
                    &problems,
                    root,
                    defaults,
                    old_bytes,
                    dawsu_hitsounding::Placement::OnObjects,
                    &[],
                    settings,
                    old_mixes,
                    2,
                    format,
                    &source.pending,
                    auto_fix_staged_mixes,
                    crate::export_manifest::SingleSampleFormat::MatchMixFormat,
                    &source.export_index_schema,
                    None,
                    cancel,
                )
            } else {
                Err(format!(
                    "Generate needs fixes at {} places. No files were written.",
                    problems.len()
                ))
            };
            let (mut file, mut mixes) = staged.inspect_err(|error| {
                diagnostics.extend(problems.iter().map(|problem| {
                    let diagnostic = OutputProblem::new(problem, &target, &source.path);
                    if can_mix { diagnostic.with_message(error) } else { diagnostic }
                }));
            })?;
            let mut slider_expected = None;
            if include_slider_audio {
                let (slider_mixes, slider_hits) = crate::slider_export::stage(
                    &crate::slider_export::StageInput {
                        source,
                        root,
                        defaults,
                        previous: old_bytes,
                        already_staged: &mixes.staged,
                        format,
                    },
                    &mut file,
                    cancel,
                )?;
                mixes.staged.extend(slider_mixes);
                slider_expected = slider_hits;
            }
            let verify_expectation = match if include_slider_audio {
                slider_expected
                    .or(mixes.verified_hits)
                    .or_else(|| expected_hitsounds.map(|h| *h))
            } else {
                mixes.verified_hits.or_else(|| expected_hitsounds.map(|h| *h))
            } {
                Some(hits) => VerifyExpectation::Hitsounds(hits),
                None => VerifyExpectation::NewEmptyDiff,
            };
            let details = if previous.is_some() {
                "The earlier generated difficulty will be replaced after creating a backup."
            } else {
                "A new hitsound difficulty is created. The editing difficulty is unchanged."
            };
            let mut plan = WritePlan {
                label: "Generate hitsound difficulty".into(),
                generated: Some((target.clone(), mixes.staged.len())),
                files: vec![PlannedFile::new(
                    target.clone(),
                    file,
                    previous.map(|output| output.bytes),
                    verify_expectation,
                    true,
                    Some(&source.file),
                    None,
                )],
                pending: Vec::new(), ledger: None,
                mixes: mixes.staged,
                combinations: mixes.combinations,
                generation,
                include_slider_audio,
                details: vec![details.into()],
                targets: Vec::new(),
                hs_mode_targets: vec![target.clone()],
                sample_warnings: Vec::new(),
                export_index_schema: mixes.index_schema,
                tick_writes: Default::default(),
                verification: Default::default(),
                pending_snapshot: None,
                source: source.path.clone(),
                version: source.version,
            };
            plan.add_pending(source)?;
            plan.check_staged_mixes();
            Ok(plan)
        }
        other => Err(crate::refusals::generate(&other)),
    }
}
pub fn new_diff(source: &Source, name: &str) -> Result<WritePlan, String> {
    match dawsu_hitsounding::generate::new_hitsound_diff(&source.file, &display_file_name(&source.path), name, &folder(source)?) {
        NewDiff::Planned{file,file_name}=>{
            let target_path = source.path.parent().ok_or_else(|| "Missing mapset folder".to_string())?.join(file_name);
            Ok(WritePlan{label:"New hitsound difficulty".into(),mixes:Vec::new(),pending:Vec::new(),ledger:None,combinations:Vec::new(),targets:Vec::new(),generation:None,generated:None,include_slider_audio:false,files:vec![PlannedFile::new(target_path.clone(),file,None,VerifyExpectation::NewEmptyDiff,false,Some(&source.file),None)],details:vec!["All gameplay objects are removed in the new difficulty; the source stays unchanged.".into()],hs_mode_targets:vec![target_path],sample_warnings:Vec::new(),export_index_schema:Default::default(),tick_writes:Default::default(),verification:Default::default(),pending_snapshot:None,source:source.path.clone(),version:source.version})
        },other=>Err(crate::refusals::new_difficulty(&other))
    }
}
pub fn copy(
    source: &Source,
    targets: &[PathBuf],
    options: &CopyOptions,
    cancel: &dyn Fn() -> bool,
) -> Result<WritePlan, String> {
    copy_report(source, Path::new(""), targets, options, cancel, &mut Vec::new(), &mut |_, _| {})
}
pub(super) fn copy_report(
    source: &Source,
    defaults: &Path,
    targets: &[PathBuf],
    options: &CopyOptions,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<WritePlan, String> {
    copy_after(
        source,
        defaults,
        targets,
        options,
        Vec::new(),
        false,
        mixes::MixFormat::Wav16,
        crate::export_manifest::SingleSampleFormat::default(),
        dawsu_hitsounding::index_packing::AdditionOnlyNormal::Difficulty,
        cancel,
        diagnostics,
        progress,
    )
}
/// Copy, planned after mixes staged earlier in the same write, so no index or name is taken twice.
#[allow(clippy::too_many_arguments)]
fn copy_after(
    source: &Source,
    defaults: &Path,
    targets: &[PathBuf],
    options: &CopyOptions,
    already_staged: Vec<mixes::StagedMix>,
    auto_fix_staged_mixes: bool,
    format: mixes::MixFormat,
    single_format: crate::export_manifest::SingleSampleFormat,
    addition_only_normal: dawsu_hitsounding::index_packing::AdditionOnlyNormal,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<WritePlan, String> {
    if targets.is_empty() {
        return Err("Choose at least one target difficulty".into());
    }
    let root = source
        .path
        .parent()
        .ok_or("Missing mapset folder")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut copy_opts = options.clone();
    copy_opts.addition_only_normal = addition_only_normal;
    let reserved_silent_index = if addition_only_normal == dawsu_hitsounding::index_packing::AdditionOnlyNormal::Silent {
        let mut occupied = mixes::occupied_indexes(&root, &source.pending)?;
        occupied.extend(already_staged.iter().filter_map(|m| m.index));
        let mut candidate = copy_opts.start_index.max(2);
        while candidate <= i32::MAX as u32 && occupied.contains(&candidate) {
            candidate += 1;
        }
        if candidate <= i32::MAX as u32 {
            copy_opts.silent_normal_index = Some(candidate);
            Some(candidate)
        } else {
            None
        }
    } else {
        None
    };
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    let mut files = Vec::new();
    let mut baseline_warnings = Vec::new();
    for target in targets {
        let path = target.canonicalize().map_err(|e| e.to_string())?;
        if path == source.path {
            return Err("The Copy source cannot also be a target".into());
        }
        if path.parent() != Some(root.as_path()) {
            return Err("Copy targets must be difficulties in this mapset".into());
        }
        if !seen.insert(path.clone()) {
            return Err("A Copy target was selected twice".into());
        }
        let read = OsuFile::read(std::fs::read(&path).map_err(|e| e.to_string())?);
        baseline_warnings.push(warning_counts_from_slice(&read.warnings));
        files.push(read.file);
        paths.push(path);
    }
    let mut silence = std::collections::HashMap::new();
    let mut is_silent = |file: &str| {
        *silence.entry(file.to_owned()).or_insert_with(|| {
            source.pending.get(file).map(|kept| Ok(kept.path.clone())).unwrap_or_else(|| crate::safe_child(&root, file))
                .and_then(|path| dawsu_audio::decode_with_cancel(&path, cancel))
                .is_ok_and(|clip| clip.samples.iter().all(|sample| sample.abs() < 1.0e-4))
        })
    };
    let mut copy_folder_files = source.folder_files.clone();
    if let Some(silent_idx) = reserved_silent_index {
        let silent_name = format!("normal-hitnormal{silent_idx}.wav");
        if !copy_folder_files.iter().any(|n| n.eq_ignore_ascii_case(&silent_name)) {
            copy_folder_files.push(silent_name);
            copy_folder_files.sort();
        }
    }
    let outcomes = match dawsu_hitsounding::copy::copy_hitsounds_with_silence(
        &source.hits,
        &source.file,
        &files,
        &copy_folder_files,
        &copy_opts,
        &mut is_silent,
        cancel,
        progress,
    ) {
        CopyOutcome::Targets(outcomes) => outcomes,
        other => return Err(crate::refusals::copy(&other)),
    };
    let earlier = already_staged.len();
    let mut plan = WritePlan {
        label: "Copy hitsounds".into(),
        mixes: already_staged,
        pending: Vec::new(), ledger: None,
        combinations: Vec::new(),
        generation: None,
        generated: None,
        include_slider_audio: false,
        files: Vec::new(),
        details: Vec::new(),
        targets: Vec::new(),
        hs_mode_targets: Vec::new(),
        sample_warnings: Vec::new(),
        export_index_schema: source.export_index_schema.clone(),
        tick_writes: Default::default(),
        verification: Default::default(),
        pending_snapshot: None,
        source: source.path.clone(),
        version: source.version,
    };
    let mut refused = None;
    for (((outcome, path), original), target_warnings) in outcomes
        .into_iter()
        .zip(paths)
        .zip(files)
        .zip(baseline_warnings)
    {
        match outcome {
            TargetOutcome::Planned(target) => {
                let expected = original.original_bytes().to_vec();
                let can_mix = target.problems.iter().all(|problem| {
                    mixes::can_render(problem, dawsu_hitsounding::Placement::GreenLines)
                });
                let staged = if target.problems.is_empty() {
                    Ok((target.file, mixes::MixFiles::default()))
                } else if can_mix {
                    mixes::stage_in_file_with_options(
                        &source.hits,
                        target.file,
                        &target.problems,
                        &root,
                        defaults,
                        &expected,
                        dawsu_hitsounding::Placement::GreenLines,
                        &plan.mixes,
                        None,
                        &[],
                        copy_opts.start_index,
                        format,
                        &source.pending,
                        auto_fix_staged_mixes,
                        single_format,
                        &plan.export_index_schema,
                        reserved_silent_index,
                        cancel,
                    )
                } else {
                    Err(format!("{} sample-mix or placement fixes needed before Copy", target.problems.len()))
                };
                let (mut file, mixes) = match staged {
                    Ok(staged) => staged,
                    Err(error) => {
                        refused.get_or_insert_with(|| format!("{}: {error}", path.display()));
                        diagnostics.extend(target.problems.iter().map(|problem| {
                            let diagnostic = OutputProblem::new(problem, &path, &path);
                            if can_mix { diagnostic.with_message(&error) } else { diagnostic }
                        }));
                        continue;
                    }
                };
                plan.export_index_schema.extend(mixes.index_schema);
                plan.targets.push(TargetCounts {
                    path: path.clone(),
                    matched: target.matched,
                    unmatched: target.dropped.len(),
                    changed: target.changed,
                    collisions: target.collisions.len(),
                    kept_from_silent: target.kept_from_silent,
                    muted: target.muted,
                    fallback_applied: target.fallback_applied,
                });
                plan.details.push(format!(
                    "{}: {} matched, {} unmatched source moments",
                    display_file_name(&path),
                    target.matched,
                    target.dropped.len()
                ));
                for collision in &target.collisions {
                    let displaced = collision.displaced_source_times.iter()
                        .map(|time| format!("{} ms", time.ms()))
                        .collect::<Vec<_>>().join(", ");
                    plan.details.push(format!(
                        "{} · {} ms: source at {} ms won; replaced {displaced}.",
                        display_file_name(&path),
                        collision.target_time.ms(),
                        collision.winning_source_time.ms(),
                    ));
                }
                if target.merged > 0 {
                    let noun = if target.merged == 1 { "hit point" } else { "hit points" };
                    plan.details.push(format!(
                        "{}: {} {noun} merged nearby source hits.",
                        display_file_name(&path), target.merged,
                    ));
                }
                if target.kept_from_silent > 0 {
                    plan.details.push(format!(
                        "{}: {} hit points kept their own sound to avoid a silent hitnormal.",
                        display_file_name(&path), target.kept_from_silent,
                    ));
                }
                plan.mixes.extend(mixes.staged);
                mixes::stage_timed_copy_with_options(&source.hits, &mut file, &path, defaults, &target.slider_placements,
                    &mut plan.mixes, &mut plan.details, &source.pending, format, auto_fix_staged_mixes, single_format, cancel, earlier)?;
                let verify_hits = mixes.verified_hits.unwrap_or(target.expected_hitsounds);
                plan.files.push(PlannedFile::new(
                    path,
                    file,
                    Some(expected),
                    VerifyExpectation::Hitsounds(verify_hits),
                    false,
                    Some(&original),
                    Some(target_warnings),
                ));
            }
            other => return Err(format!("{}: {}", path.display(), crate::refusals::copy_target(&other))),
        }
    }
    if let Some(refused) = refused {
        return Err(refused);
    }
    if let Some(silent_idx) = reserved_silent_index {
        let used = plan.files.iter().any(|planned| {
            planned.file.hit_objects().iter().any(|obj| {
                obj.hit_sample().is_some_and(|hs| hs.index as u32 == silent_idx)
                    || matches!(
                        obj.kind(),
                        dawsu_osu_format::ObjectKind::Slider(slider)
                            if slider.edge_sets.as_ref().is_some_and(|edges| {
                                edges.iter().flatten().any(|edge| edge.index as u32 == silent_idx)
                            })
                    )
            }) || planned.file.timing_points().iter().any(|tp| {
                tp.sample_index().is_some_and(|i| i as u32 == silent_idx)
            })
        });
        if used {
            plan.mixes.push(mixes::StagedMix::indexed_bytes(
                &root,
                &format!("normal-hitnormal{silent_idx}.wav"),
                silent_idx,
                dawsu_hitsounding::index_packing::STANDARD_SILENT_WAV.to_vec(),
            ));
        }
    }
    plan.add_pending(source)?;
    plan.check_staged_mixes();
    Ok(plan)

}
/// The Export window's one write: the base hitsound difficulty (Generate) when chosen, then Copy
/// into the included difficulties. Copy is planned after Generate's mixes, and everything is
/// written or rolled back together.
pub(super) struct ExportConfig<'a> {
    pub defaults: &'a Path,
    pub settings: Option<&'a crate::ExportSettings>,
    pub projects: Option<&'a Path>,
    pub options: &'a crate::ExportOptions,
}

pub(super) fn export_report(
    source: &Source,
    config: &ExportConfig<'_>,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<WritePlan, String> {
    let ExportConfig { defaults, settings, projects, options } = *config;
    if !options.base {
        return copy_after(
            source,
            defaults,
            &options.targets,
            &options.copy,
            Vec::new(),
            options.auto_fix_staged_mixes,
            options.mix_format,
            options.single_format,
            options.addition_only_normal,
            cancel,
            diagnostics,
            progress,
        );
    }
    let mut generate_total = 0;
    let mut base = generate_report_with_options_and_format(
        source,
        defaults,
        settings,
        projects,
        options.slider_audio,
        options.auto_fix_staged_mixes,
        options.mix_format,
        cancel,
        diagnostics,
        &mut |done, total| {
            generate_total = total;
            progress(done, total);
        },
    )?;
    if options.targets.is_empty() {
        return Ok(base);
    }
    let generated = base.generated.clone().ok_or("Generate must write one difficulty.")?;
    let base_name = generated.0.file_name().map(|name| name.to_string_lossy().to_lowercase());
    if options.targets.iter().any(|target| {
        target.file_name().map(|name| name.to_string_lossy().to_lowercase()) == base_name
    }) {
        return Err("The base hitsound difficulty cannot also be a Copy target.".into());
    }
    let mut plan = copy_after(
        source,
        defaults,
        &options.targets,
        &options.copy,
        std::mem::take(&mut base.mixes),
        options.auto_fix_staged_mixes,
        options.mix_format,
        options.single_format,
        options.addition_only_normal,
        cancel,
        diagnostics,
        &mut |done, total| progress(generate_total + done, generate_total + total),
    )?;
    let mut files = base.files;
    files.append(&mut plan.files);
    let mut details = base.details;
    details.append(&mut plan.details);
    let mut merged_schema = base.export_index_schema;
    merged_schema.extend(plan.export_index_schema);
    let mut export = WritePlan {
        label: "Export hitsounds".into(),
        files,
        mixes: plan.mixes,
        // Recomputed below over every difficulty the export writes.
        pending: Vec::new(), ledger: None,
        combinations: base.combinations,
        generation: base.generation,
        generated: Some(generated),
        include_slider_audio: base.include_slider_audio,
        details,
        targets: plan.targets,
        hs_mode_targets: base.hs_mode_targets,
        sample_warnings: Vec::new(),
        export_index_schema: merged_schema,
        tick_writes: Default::default(),
        verification: Default::default(),
        pending_snapshot: None,
        source: plan.source,
        version: plan.version,
    };
    export.add_pending(source)?;
    export.check_staged_mixes();
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_clarity_lines_reports_created_and_reused_mixes() {
        let dir = std::env::temp_dir().join("dawsu-output-clarity-test");
        let plan = WritePlan {
            label: "Test".into(),
            files: Vec::new(),
            mixes: vec![
                mixes::StagedMix::pending(&dir, "new_mix.wav", vec![1, 2, 3]),
            ],
            pending: Vec::new(),
            ledger: None,
            combinations: Vec::new(),
            generation: None,
            generated: None,
            include_slider_audio: false,
            details: Vec::new(),
            targets: Vec::new(),
            hs_mode_targets: Vec::new(),
            sample_warnings: Vec::new(),
            export_index_schema: Default::default(),
            tick_writes: Default::default(),
            verification: Default::default(),
            pending_snapshot: None,
            source: dir.join("map.osu"),
            version: 1,
        };
        let lines = plan.volume_clarity_lines();
        assert_eq!(plan.created_mix_paths().count(), 1);
        assert_eq!(plan.reused_mix_paths().count(), 0);
        assert!(lines.iter().any(|l| l.contains("1 new .wav created, 0 existing .wav reused")));
    }

    #[test]
    fn osu_line_diff_identical_files_is_empty() {
        let content = b"[TimingPoints]\r\n1000,500,4,1,0,100,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";
        let diff = compute_osu_line_diff(content, content, DEFAULT_DIFF_MAX_LINES);
        assert!(diff.is_empty());
        assert!(diff.byte_identical);
        assert_eq!(diff.added, 0);
        assert_eq!(diff.removed, 0);
        assert_eq!(diff.truncated, 0);
        assert!(diff.sections.is_empty());
        assert!(diff.entries.is_empty());
    }

    #[test]
    fn osu_line_diff_reports_inserted_greenline_and_edited_hitobject_with_line_numbers() {
        let old = concat!(
            "[TimingPoints]\n",
            "1000,500,4,1,0,100,1,0\n",
            "\n",
            "[HitObjects]\n",
            "256,192,1000,1,0,0:0:0:0:\n",
            "256,192,2000,1,0,0:0:0:0:\n",
        );
        let new = concat!(
            "[TimingPoints]\n",
            "1000,500,4,1,0,100,1,0\n",
            "1500,-100,4,2,1,80,0,0\n",
            "\n",
            "[HitObjects]\n",
            "256,192,1000,1,4,0:0:0:80:\n",
            "256,192,2000,1,0,0:0:0:0:\n",
        );
        let diff = compute_osu_line_diff(old.as_bytes(), new.as_bytes(), DEFAULT_DIFF_MAX_LINES);
        assert_eq!(diff.added, 2);
        assert_eq!(diff.removed, 1);
        assert_eq!(diff.truncated, 0);
        assert_eq!(
            diff.entries,
            vec![
                OsuDiffLine {
                    section: Some("[TimingPoints]".into()),
                    line: 3,
                    kind: OsuDiffKind::Added,
                    text: "1500,-100,4,2,1,80,0,0".into(),
                },
                OsuDiffLine {
                    section: Some("[HitObjects]".into()),
                    line: 5,
                    kind: OsuDiffKind::Removed,
                    text: "256,192,1000,1,0,0:0:0:0:".into(),
                },
                OsuDiffLine {
                    section: Some("[HitObjects]".into()),
                    line: 6,
                    kind: OsuDiffKind::Added,
                    text: "256,192,1000,1,4,0:0:0:80:".into(),
                },
            ]
        );
    }

    #[test]
    fn osu_line_diff_truncates_cleanly_at_cap() {
        let mut old = String::from("[HitObjects]\n");
        let mut new = String::from("[HitObjects]\n");
        for i in 0..10 {
            old.push_str(&format!("256,192,{},1,0,0:0:0:0:\n", i * 100));
            new.push_str(&format!("256,192,{},1,8,0:0:0:70:\n", i * 100));
        }
        let diff = compute_osu_line_diff(old.as_bytes(), new.as_bytes(), 5);
        assert_eq!(diff.added, 10);
        assert_eq!(diff.removed, 10);
        assert_eq!(diff.entries.len(), 5);
        assert_eq!(diff.truncated, 15);
    }

    #[test]
    fn osu_line_diff_handles_brand_new_file() {
        let new = "[Metadata]\nVersion:Hitsounds\n[HitObjects]\n256,192,1000,1,2,0:0:0:0:";
        let diff = compute_osu_line_diff(&[], new.as_bytes(), 2);
        assert!(!diff.byte_identical);
        assert_eq!(diff.added, 4);
        assert_eq!(diff.removed, 0);
        assert_eq!(diff.entries.len(), 2);
        assert_eq!(diff.truncated, 2);
        assert_eq!(diff.entries[0].line, 1);
        assert_eq!(diff.entries[0].kind, OsuDiffKind::Added);
        assert_eq!(diff.entries[1].section.as_deref(), Some("[Metadata]"));
    }

    #[test]
    fn osu_line_diff_marathon_500_objects_with_inserted_greenlines_has_no_line_shift() {
        let mut old = String::from("[TimingPoints]\n0,500,4,1,0,100,1,0\n");
        let mut new = String::from("[TimingPoints]\n0,500,4,1,0,100,1,0\n");
        for i in 0..20 {
            new.push_str(&format!("{},-100,4,2,1,{},0,0\n", i * 500 + 250, 70 + (i % 20)));
        }
        old.push_str("\n[HitObjects]\n");
        new.push_str("\n[HitObjects]\n");
        for i in 0..500 {
            let t = i * 250 + 1000;
            old.push_str(&format!("256,192,{t},1,0,0:0:0:0:\n"));
            if i % 2 == 0 {
                new.push_str(&format!("256,192,{t},1,8,0:0:0:80:\n"));
            } else {
                new.push_str(&format!("256,192,{t},1,0,0:0:0:0:\n"));
            }
        }
        let diff = compute_osu_line_diff(old.as_bytes(), new.as_bytes(), DEFAULT_DIFF_MAX_LINES);
        assert!(!diff.byte_identical);
        assert_eq!(diff.added, 20 + 250);
        assert_eq!(diff.removed, 250);
        assert_eq!(diff.entries.len(), DEFAULT_DIFF_MAX_LINES);
        assert_eq!(diff.truncated, (20 + 500) - DEFAULT_DIFF_MAX_LINES);
        // Every TimingPoints line stays scoped to [TimingPoints] and every HitObjects line to [HitObjects].
        let tp_entries: Vec<_> = diff
            .entries
            .iter()
            .filter(|e| e.section.as_deref() == Some("[TimingPoints]"))
            .collect();
        let ho_entries: Vec<_> = diff
            .entries
            .iter()
            .filter(|e| e.section.as_deref() == Some("[HitObjects]"))
            .collect();
        assert_eq!(tp_entries.len(), 20);
        assert_eq!(ho_entries.len(), DEFAULT_DIFF_MAX_LINES - 20);
        assert!(tp_entries.iter().all(|e| e.text.contains("-100")));
        assert!(ho_entries.iter().all(|e| e.text.starts_with("256,192,")));
    }

    #[test]
    fn osu_line_diff_orders_sections_and_allocates_fair_budget() {
        let mut old = String::from("[TimingPoints]\n0,500,4,1,0,100,1,0\n");
        let mut new = String::from("[TimingPoints]\n0,500,4,1,0,100,1,0\n");
        for i in 1..=250 {
            new.push_str(&format!("{},-100,4,2,1,80,0,0\n", i * 100));
        }
        old.push_str("\n[HitObjects]\n");
        new.push_str("\n[HitObjects]\n");
        for i in 0..50 {
            let t = i * 300 + 50000;
            old.push_str(&format!("128,128,{t},1,0,0:0:0:0:\n"));
            new.push_str(&format!("128,128,{t},1,4,0:0:0:90:\n"));
        }
        let diff = compute_osu_line_diff(old.as_bytes(), new.as_bytes(), DEFAULT_DIFF_MAX_LINES);
        assert_eq!(
            diff.sections,
            vec![
                OsuDiffSectionCount {
                    section: "[TimingPoints]".into(),
                    added: 250,
                    removed: 0,
                    truncated: 150,
                },
                OsuDiffSectionCount {
                    section: "[HitObjects]".into(),
                    added: 50,
                    removed: 50,
                    truncated: 0,
                },
            ]
        );
        let tp_shown = diff
            .entries
            .iter()
            .filter(|e| e.section.as_deref() == Some("[TimingPoints]"))
            .count();
        let ho_shown = diff
            .entries
            .iter()
            .filter(|e| e.section.as_deref() == Some("[HitObjects]"))
            .count();
        assert_eq!(ho_shown, 100);
        assert_eq!(tp_shown, 100);
        assert_eq!(diff.entries.len(), 200);
        assert_eq!(diff.truncated, 150);
    }

    #[test]
    fn osu_line_diff_handles_300_stacked_duplicate_prefix_hit_objects() {
        let mut old = String::from("[HitObjects]\n");
        let mut new = String::from("[HitObjects]\n");
        // All 300 circles share identical (x, y, time, type) prefix; >256 edits would overflow Myers without the O(N) path.
        for _ in 0..300 {
            old.push_str("256,192,1000,1,0,0:0:0:0:\n");
            new.push_str("256,192,1000,1,8,0:0:0:80:\n");
        }
        let diff = compute_osu_line_diff(old.as_bytes(), new.as_bytes(), DEFAULT_DIFF_MAX_LINES);
        assert_eq!(diff.added, 300);
        assert_eq!(diff.removed, 300);
        assert_eq!(diff.entries.len(), DEFAULT_DIFF_MAX_LINES);
        assert_eq!(diff.truncated, 400);
        assert_eq!(
            diff.sections,
            vec![OsuDiffSectionCount {
                section: "[HitObjects]".into(),
                added: 300,
                removed: 300,
                truncated: 400,
            }]
        );
    }

    #[test]
    fn osu_line_diff_crlf_vs_lf_only_sets_byte_identical_false_with_empty_entries() {
        let old = b"osu file format v14\r\n[General]\r\nMode:0\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";
        let new = b"osu file format v14\n[General]\nMode:0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        let diff = compute_osu_line_diff(old, new, DEFAULT_DIFF_MAX_LINES);
        assert!(!diff.byte_identical);
        assert!(diff.is_empty());
        assert_eq!(diff.added, 0);
        assert_eq!(diff.removed, 0);
        assert!(diff.sections.is_empty());
        assert!(diff.entries.is_empty());
    }

    #[test]
    fn planned_file_replace_file_resets_diff_cache() {
        let raw_old = b"osu file format v14\n\n[General]\nMode:0\n\n[Metadata]\nVersion:Test\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        let file1 = OsuFile::read(raw_old.to_vec()).file;
        let mut planned = PlannedFile::new(
            PathBuf::from("test.osu"),
            file1,
            Some(raw_old.to_vec()),
            VerifyExpectation::NewEmptyDiff,
            false,
            None,
            None,
        );
        assert!(planned.diff_summary().is_empty());
        planned.verification = Some(FileVerificationReport {
            passed: true,
            ..Default::default()
        });
        assert!(planned.verification().is_some());

        let raw_new = b"osu file format v14\n\n[General]\nMode:0\n\n[Metadata]\nVersion:Test\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,8,0:0:0:80:\n";
        let file2 = OsuFile::read(raw_new.to_vec()).file;
        planned.replace_file(file2);
        assert!(planned.verification().is_none());
        let updated = planned.diff_summary();
        assert_eq!(updated.added, 1);
        assert_eq!(updated.removed, 1);
    }

    #[test]
    fn verify_and_finalize_blocks_tampered_output_and_apply_writes_nothing() {
        let scratch = dawsu_test_support::Scratch::new("verify-tamper");
        let map_path = scratch.path("src.osu");
        let target_path = scratch.path("dst.osu");
        let src_bytes = b"osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Src\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,2,0:0:0:0:\n";
        let dst_bytes = b"osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Dst\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        std::fs::write(&map_path, src_bytes).unwrap();
        std::fs::write(&target_path, dst_bytes).unwrap();
        let mut doc = crate::Document::read(&map_path).unwrap();
        let source = Source::from_document(&doc);
        let mut diagnostics = Vec::new();
        let mut plan = copy_report(
            &source,
            Path::new(""),
            std::slice::from_ref(&target_path),
            &CopyOptions::default(),
            &|| false,
            &mut diagnostics,
            &mut |_, _| {},
        )
        .unwrap();
        // Tamper with the planned file so verification catches the hitsound mismatch.
        let tampered_bytes = b"osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Dst\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        plan.files[0].replace_file(OsuFile::read(tampered_bytes.to_vec()).file);
        let summary = plan.verify_and_finalize(&source, &HashMap::new(), false);
        assert!(!summary.passed);
        assert_eq!(summary.failed_files, 1);
        let err = plan.apply(&scratch.path("backups"), &|| false).unwrap_err();
        assert!(err.contains("Save verification stopped"), "unexpected error: {err}");
        assert_eq!(std::fs::read(&target_path).unwrap(), dst_bytes);
        let _ = &mut doc;
    }
}
