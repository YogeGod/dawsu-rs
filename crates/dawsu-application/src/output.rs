//! Reviewable map output. Planning reads snapshots; applying checks every target again.
use crate::Document;
use dawsu_foundation::safe_write::BackupPolicy;
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
}
impl Source {
    pub fn from_document(d: &Document) -> Self {
        Self {
            file: d.file.clone(),
            hits: d.composer.hitsounds().clone(),
            path: d.path.clone(),
            folder_files: d.files.clone(),
            version: d.composer.history.version(),
        }
    }
}
pub struct PlannedFile {
    pub path: PathBuf,
    pub file: OsuFile,
    pub expected: Option<Vec<u8>>,
}
pub struct WritePlan {
    pub label: String,
    pub files: Vec<PlannedFile>,
    pub details: Vec<String>,
    pub source: PathBuf,
    pub version: u64,
}
impl WritePlan {
    pub fn apply(&self, backups: &Path, cancel: &dyn Fn() -> bool) -> Result<usize, String> {
        for target in &self.files {
            let current = std::fs::read(&target.path).ok();
            if current != target.expected {
                return Err(format!(
                    "{} changed since this review. Nothing was written.",
                    target.path.display()
                ));
            }
        }
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
                    target.file.save_new(&target.path)
                };
                result
                    .map_err(|e| e.to_string())
                    .and_then(|outcome| match outcome {
                        SaveOutcome::Saved { .. } | SaveOutcome::NothingToSave => Ok(()),
                        other => Err(format!("{}: {other:?}", target.path.display())),
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
                return Err(if kept.is_empty() {
                    format!("{error}. Previous targets were restored.")
                } else {
                    format!(
                        "{error}. These files had newer changes or could not be restored and were kept: {}",
                        kept.join(", ")
                    )
                });
            }
            written.push(target);
        }
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
    let name = source
        .path
        .file_name()
        .ok_or("Missing source filename")?
        .to_string_lossy();
    match dawsu_hitsounding::generate::generate(
        &source.hits,
        &source.file,
        &name,
        &folder(source)?,
        cancel,
        &mut |_, _| {},
    ) {
        GenerateOutcome::Planned {
            file,
            file_name,
            problems,
        } => {
            if !problems.is_empty() {
                return Err(format!(
                    "Generate needs sample-mix review: {} problems; no files written",
                    problems.len()
                ));
            }
            Ok(WritePlan {
                label: "Generate hitsound difficulty".into(),
                files: vec![PlannedFile {
                    path: source.path.parent().unwrap().join(file_name),
                    file,
                    expected: None,
                }],
                details: vec![
                    "A new hitsound difficulty is created. The editing difficulty is unchanged."
                        .into(),
                ],
                source: source.path.clone(),
                version: source.version,
            })
        }
        other => Err(format!("Generate refused: {other:?}")),
    }
}
pub fn new_diff(source: &Source, name: &str) -> Result<WritePlan, String> {
    match dawsu_hitsounding::generate::new_hitsound_diff(&source.file,&source.path.file_name().unwrap().to_string_lossy(),name,&folder(source)?){
        NewDiff::Planned{file,file_name}=>Ok(WritePlan{label:"New hitsound difficulty".into(),files:vec![PlannedFile{path:source.path.parent().unwrap().join(file_name),file,expected:None}],details:vec!["All gameplay objects are removed in the new difficulty; the source stays unchanged.".into()],source:source.path.clone(),version:source.version}),other=>Err(format!("New difficulty refused: {other:?}"))
    }
}
pub fn copy(
    source: &Source,
    targets: &[PathBuf],
    options: &CopyOptions,
    cancel: &dyn Fn() -> bool,
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
    let outcomes = match dawsu_hitsounding::copy::copy_hitsounds(
        &source.hits,
        &source.file,
        &files,
        &source.folder_files,
        options,
        cancel,
        &mut |_, _| {},
    ) {
        CopyOutcome::Targets(outcomes) => outcomes,
        other => return Err(format!("Copy refused: {other:?}")),
    };
    let mut plan = WritePlan {
        label: "Copy hitsounds".into(),
        files: Vec::new(),
        details: Vec::new(),
        source: source.path.clone(),
        version: source.version,
    };
    for ((outcome, path), original) in outcomes.into_iter().zip(paths).zip(files) {
        match outcome {
            TargetOutcome::Planned(target) => {
                if !target.problems.is_empty() {
                    return Err(format!(
                        "{} needs {} sample-mix or placement fixes before Copy",
                        path.display(),
                        target.problems.len()
                    ));
                }
                plan.details.push(format!(
                    "{}: {} changed, {} matched, {} unmatched source moments",
                    path.file_name().unwrap().to_string_lossy(),
                    target.changed,
                    target.matched,
                    target.dropped.len()
                ));
                plan.files.push(PlannedFile {
                    path,
                    file: target.file,
                    expected: Some(original.original_bytes().to_vec()),
                });
            }
            other => return Err(format!("{}: {other:?}", path.display())),
        }
    }
    Ok(plan)
}
