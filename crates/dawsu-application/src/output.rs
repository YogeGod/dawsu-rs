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

pub struct Source {
    pub file: OsuFile,
    pub hits: Hitsounds,
    pub path: PathBuf,
    pub folder_files: Vec<String>,
    pub version: u64,
    /// The open document's pending files: mixes read them and allocations skip their names.
    pub(crate) pending: crate::pending::Pending,
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
        }
    }
}
pub struct PlannedFile {
    pub path: PathBuf,
    pub file: OsuFile,
    pub expected: Option<Vec<u8>>,
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
            lines.push(format!(
                "Sample mixes: {created_wav} new .wav created, {reused_wav} existing .wav reused."
            ));
        }
        lines
    }

    pub(super) fn mix_clip(&self, index: usize) -> Option<dawsu_audio::AudioClip> {
        self.mixes.get(index).map(mixes::StagedMix::clip)
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
                crate::mixes::stage_in_file(
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
                    &source.pending,
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
                    source, &mut file, root, defaults, old_bytes, &mixes.staged, cancel,
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
                files: vec![PlannedFile {
                    path: target.clone(),
                    file,
                    expected: previous.map(|output| output.bytes),
                }],
                pending: Vec::new(), ledger: None,
                mixes: mixes.staged,
                combinations: mixes.combinations,
                generation,
                include_slider_audio,
                details: vec![details.into()],
                targets: Vec::new(),
                hs_mode_targets: vec![target.clone()],
                source: source.path.clone(),
                version: source.version,
            };
            plan.add_pending(source)?;
            Ok(plan)
        }
        other => Err(crate::refusals::generate(&other)),
    }
}
pub fn new_diff(source: &Source, name: &str) -> Result<WritePlan, String> {
    match dawsu_hitsounding::generate::new_hitsound_diff(&source.file,&source.path.file_name().unwrap().to_string_lossy(),name,&folder(source)?){
        NewDiff::Planned{file,file_name}=>{
            let target_path = source.path.parent().unwrap().join(file_name);
            Ok(WritePlan{label:"New hitsound difficulty".into(),mixes:Vec::new(),pending:Vec::new(),ledger:None,combinations:Vec::new(),targets:Vec::new(),generation:None,generated:None,include_slider_audio:false,files:vec![PlannedFile{path:target_path.clone(),file,expected:None}],details:vec!["All gameplay objects are removed in the new difficulty; the source stays unchanged.".into()],hs_mode_targets:vec![target_path],source:source.path.clone(),version:source.version})
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
    copy_after(source, defaults, targets, options, Vec::new(), cancel, diagnostics, progress)
}
/// Copy, planned after mixes staged earlier in the same write, so no index or name is taken twice.
fn copy_after(
    source: &Source,
    defaults: &Path,
    targets: &[PathBuf],
    options: &CopyOptions,
    already_staged: Vec<mixes::StagedMix>,
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
        options,
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
                    mixes::stage_in_file(
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
                        options.start_index,
                        &source.pending,
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
                plan.targets.push(TargetCounts {
                    path: path.clone(),
                    matched: target.matched,
                    unmatched: target.dropped.len(),
                    changed: target.changed,
                    collisions: target.collisions.len(),
                    kept_from_silent: target.kept_from_silent,
                    muted: target.muted,
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
                mixes::stage_timed_copy(&source.hits, &mut file, &path, defaults, &target.slider_placements,
                    &mut plan.mixes, &mut plan.details, &source.pending, cancel, earlier)?;
                plan.files.push(PlannedFile {
                    path,
                    file,
                    expected: Some(expected),
                });
            }
            other => return Err(format!("{}: {}", path.display(), crate::refusals::copy_target(&other))),
        }
    }
    if let Some(refused) = refused {
        return Err(refused);
    }
    plan.add_pending(source)?;
    Ok(plan)

}
/// The Export window's one write: the base hitsound difficulty (Generate) when chosen, then Copy
/// into the included difficulties. Copy is planned after Generate's mixes, and everything is
/// written or rolled back together.
pub(super) fn export_report(
    source: &Source,
    defaults: &Path,
    settings: Option<&crate::ExportSettings>,
    projects: Option<&Path>,
    options: &crate::ExportOptions,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<WritePlan, String> {
    if !options.base {
        return copy_report(source, defaults, &options.targets, &options.copy, cancel, diagnostics, progress);
    }
    let mut generate_total = 0;
    let mut base = generate_report(
        source, defaults, settings, projects, options.slider_audio, cancel, diagnostics,
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
        source, defaults, &options.targets, &options.copy, std::mem::take(&mut base.mixes),
        cancel, diagnostics, &mut |done, total| progress(generate_total + done, generate_total + total),
    )?;
    let mut files = base.files;
    files.append(&mut plan.files);
    let mut details = base.details;
    details.append(&mut plan.details);
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
        source: plan.source,
        version: plan.version,
    };
    export.add_pending(source)?;
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
            source: dir.join("map.osu"),
            version: 1,
        };
        let lines = plan.volume_clarity_lines();
        assert_eq!(plan.created_mix_paths().count(), 1);
        assert_eq!(plan.reused_mix_paths().count(), 0);
        assert!(lines.iter().any(|l| l.contains("1 new .wav created, 0 existing .wav reused")));
    }
}
