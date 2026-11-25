//! Reviewable map output. Planning reads snapshots; applying checks every target again.
use crate::{Document, OutputProblem, generate_receipt, mixes};
use dawsu_foundation::safe_write::{self, BackupPolicy};
use dawsu_hitsounding::{
    Hitsounds,
    copy::{CopyOptions, CopyOutcome, TargetOutcome},
    generate::{GenerateOutcome, NewDiff},
};
use dawsu_osu_format::{OsuFile, SaveOutcome};
use std::collections::HashSet;
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

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsuDiffSummary {
    pub added: usize,
    pub removed: usize,
    pub truncated: usize,
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

fn section_headers(lines: &[String]) -> Vec<Option<String>> {
    let mut current: Option<String> = None;
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let trimmed = line.trim();
        if trimmed.len() > 2 && trimmed.starts_with('[') && trimmed.ends_with(']') {
            current = Some(trimmed.to_owned());
        }
        out.push(current.clone());
    }
    out
}

enum RawEdit {
    Removed(usize),
    Added(usize),
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

/// Linear-time fallback when a file has more than `MAX_D` line edits (e.g. marathon diffs).
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

/// Computes a bounded, section-aware changed-lines diff between existing and proposed `.osu` bytes.
pub fn compute_osu_line_diff(
    old_bytes: &[u8],
    new_bytes: &[u8],
    max_entries: usize,
) -> OsuDiffSummary {
    if old_bytes == new_bytes {
        return OsuDiffSummary::default();
    }
    let old_lines = split_osu_lines(old_bytes);
    let new_lines = split_osu_lines(new_bytes);
    if old_lines == new_lines {
        return OsuDiffSummary::default();
    }

    let old_sections = section_headers(&old_lines);
    let new_sections = section_headers(&new_lines);

    let mut prefix = 0usize;
    let min_len = old_lines.len().min(new_lines.len());
    while prefix < min_len && old_lines[prefix] == new_lines[prefix] {
        prefix += 1;
    }

    let mut suffix = 0usize;
    while suffix < (old_lines.len() - prefix)
        && suffix < (new_lines.len() - prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let a = &old_lines[prefix..old_lines.len() - suffix];
    let b = &new_lines[prefix..new_lines.len() - suffix];

    const MAX_MYERS_D: usize = 512;
    let edits = if a.is_empty() {
        (0..b.len()).map(RawEdit::Added).collect()
    } else if b.is_empty() {
        (0..a.len()).map(RawEdit::Removed).collect()
    } else {
        myers_diff(a, b, MAX_MYERS_D).unwrap_or_else(|| fallback_diff(a, b))
    };

    let mut summary = OsuDiffSummary::default();
    for edit in edits {
        match edit {
            RawEdit::Removed(rel) => {
                summary.removed += 1;
                if summary.entries.len() < max_entries {
                    let idx = prefix + rel;
                    summary.entries.push(OsuDiffLine {
                        section: old_sections[idx].clone(),
                        line: idx + 1,
                        kind: OsuDiffKind::Removed,
                        text: old_lines[idx].clone(),
                    });
                } else {
                    summary.truncated += 1;
                }
            }
            RawEdit::Added(rel) => {
                summary.added += 1;
                if summary.entries.len() < max_entries {
                    let idx = prefix + rel;
                    summary.entries.push(OsuDiffLine {
                        section: new_sections[idx].clone(),
                        line: idx + 1,
                        kind: OsuDiffKind::Added,
                        text: new_lines[idx].clone(),
                    });
                } else {
                    summary.truncated += 1;
                }
            }
        }
    }
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
pub struct PlannedFile {
    pub path: PathBuf,
    pub file: OsuFile,
    pub expected: Option<Vec<u8>>,
    diff_cache: OnceLock<OsuDiffSummary>,
}
impl PlannedFile {
    fn new(path: PathBuf, file: OsuFile, expected: Option<Vec<u8>>) -> Self {
        Self {
            path,
            file,
            expected,
            diff_cache: OnceLock::new(),
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
                target.file = cleaned.file;
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
        for target in &self.files {
            let current = std::fs::read(&target.path).ok();
            if current != target.expected {
                return Err(format!(
                    "{} changed since this review. Nothing was written.",
                    target.path.display()
                ));
            }
            let bytes = target.file.to_bytes();
            let read = OsuFile::read(bytes);
            if read.file.format_version().is_none() {
                return Err(format!(
                    "Save verification stopped before writing {}: compiled output is missing a valid osu! file format header.",
                    target.path.display()
                ));
            }
            if let dawsu_hitsounding::Decompiled::UnsupportedMode { mode } =
                dawsu_hitsounding::decompile(&read.file, &[], &|| false, &mut |_, _| {})
            {
                return Err(format!(
                    "Save verification stopped before writing {}: compiled output parsed as unsupported game mode {mode}.",
                    target.path.display()
                ));
            }
        }
        if let Some(root) = self.source.parent() {
            let pending_names = self.pending_names();
            let skip: Vec<&Path> = self.files.iter().map(|f| f.path.as_path()).collect();
            crate::pending::check_indexes_free_except(root, &skip, &pending_names)?;
        }
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
                path.file_name().unwrap().to_string_lossy().to_string(),
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
    let name = source
        .path
        .file_name()
        .ok_or("Missing source filename")?
        .to_string_lossy();
    let root = source.path.parent().ok_or("Missing mapset folder")?;
    let file_name = dawsu_hitsounding::generate::file_name_for(
        &source.file, &name, dawsu_hitsounding::generate::HITSOUNDS_VERSION,
    );
    let generation = projects.map(|projects| generate_receipt::path(projects, &source.path));
    let previous = generate_receipt::previous(generation.as_deref(), &root.join(&file_name))?;
    let mut siblings = folder(source)?;
    if previous.is_some() {
        siblings.retain(|(sibling, _)| sibling != &file_name);
    }
    match dawsu_hitsounding::generate::generate(
        &source.hits,
        &source.file,
        &name,
        &siblings,
        cancel,
        progress,
    ) {
        GenerateOutcome::Planned {
            file,
            file_name,
            problems,
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
            if include_slider_audio {
                let slider_mixes = crate::slider_export::stage(
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
            }
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
    match dawsu_hitsounding::generate::new_hitsound_diff(&source.file,&source.path.file_name().unwrap().to_string_lossy(),name,&folder(source)?){
        NewDiff::Planned{file,file_name}=>{
            let target_path = source.path.parent().unwrap().join(file_name);
            Ok(WritePlan{label:"New hitsound difficulty".into(),mixes:Vec::new(),pending:Vec::new(),ledger:None,combinations:Vec::new(),targets:Vec::new(),generation:None,generated:None,include_slider_audio:false,files:vec![PlannedFile::new(target_path.clone(),file,None)],details:vec!["All gameplay objects are removed in the new difficulty; the source stays unchanged.".into()],hs_mode_targets:vec![target_path],sample_warnings:Vec::new(),export_index_schema:Default::default(),tick_writes:Default::default(),source:source.path.clone(),version:source.version})
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
        files.push(OsuFile::read(std::fs::read(&path).map_err(|e| e.to_string())?).file);
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
    let outcomes = match dawsu_hitsounding::copy::copy_hitsounds_with_silence(
        &source.hits,
        &source.file,
        &files,
        &source.folder_files,
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
        source: source.path.clone(),
        version: source.version,
    };
    let mut refused = None;
    for ((outcome, path), original) in outcomes.into_iter().zip(paths).zip(files) {
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
                    path.file_name().unwrap().to_string_lossy(),
                    target.matched,
                    target.dropped.len()
                ));
                for collision in &target.collisions {
                    let displaced = collision.displaced_source_times.iter()
                        .map(|time| format!("{} ms", time.ms()))
                        .collect::<Vec<_>>().join(", ");
                    plan.details.push(format!(
                        "{} · {} ms: source at {} ms won; replaced {displaced}.",
                        path.file_name().unwrap().to_string_lossy(),
                        collision.target_time.ms(),
                        collision.winning_source_time.ms(),
                    ));
                }
                if target.merged > 0 {
                    let noun = if target.merged == 1 { "hit point" } else { "hit points" };
                    plan.details.push(format!(
                        "{}: {} {noun} merged nearby source hits.",
                        path.file_name().unwrap().to_string_lossy(), target.merged,
                    ));
                }
                if target.kept_from_silent > 0 {
                    plan.details.push(format!(
                        "{}: {} hit points kept their own sound to avoid a silent hitnormal.",
                        path.file_name().unwrap().to_string_lossy(), target.kept_from_silent,
                    ));
                }
                plan.mixes.extend(mixes.staged);
                mixes::stage_timed_copy_with_options(&source.hits, &mut file, &path, defaults, &target.slider_placements,
                    &mut plan.mixes, &mut plan.details, &source.pending, format, auto_fix_staged_mixes, single_format, cancel, earlier)?;
                plan.files.push(PlannedFile::new(
                    path,
                    file,
                    Some(expected),
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
        assert_eq!(diff.added, 0);
        assert_eq!(diff.removed, 0);
        assert_eq!(diff.truncated, 0);
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
        assert_eq!(diff.added, 4);
        assert_eq!(diff.removed, 0);
        assert_eq!(diff.entries.len(), 2);
        assert_eq!(diff.truncated, 2);
        assert_eq!(diff.entries[0].line, 1);
        assert_eq!(diff.entries[0].kind, OsuDiffKind::Added);
        assert_eq!(diff.entries[1].section.as_deref(), Some("[Metadata]"));
    }
}
