//! New hitsound diff and Generate hitsound diff (design doc "Generate and Copy hitsounds",
//! Responsibilities 1-5 and 11-12; the Generate and Copy plan, Tasks 2 and 4).
//!
//! A plan is a changed copy of the source difficulty: it starts from the source's bytes (ADR 0028
//! Decision 3) and keeps its line endings, BOM and format version. Because of that it must be
//! written only as a new file (`OsuFile::save_new`): written over the source's path, the one
//! write's changed-on-disk check would pass and the source would be replaced. Planning opens and
//! writes no file (invariant 1).

use crate::compile::{compile, mix_problem, Compiled, Placement, SaveProblem};
use crate::copy::set_moment;
use crate::decompile::{decompile, Decompiled};
use crate::lazer::control_points::offset_for;
use crate::model::{Hitsounds, MomentId};
use crate::rules::saved_volume;
use dawsu_foundation::SampleIdentity;
use dawsu_osu_format::OsuFile;
use std::collections::{BTreeMap, BTreeSet};

/// The Version a generated hitsound difficulty gets (old app `HitsoundDiff.Version`).
pub const HITSOUNDS_VERSION: &str = "Hitsounds";

/// The outcome of Generate hitsound diff.
#[derive(Debug, Clone)]
pub enum GenerateOutcome {
    /// The planned difficulty, its file name in the source's folder, and the groups compile could
    /// not write (each circle stays a bare hitnormal; the app decides whether to write anyway).
    Planned { file: OsuFile, file_name: String, problems: Vec<SaveProblem> },
    /// A file with the hitsound difficulty's name is already in the folder. Replacing an earlier
    /// output comes with the app (M3); in M2 it is refused (plan decision C).
    NameTaken,
    /// A difficulty in the folder already has the Version "Hitsounds".
    VersionTaken,
    Cancelled,
}

/// Plans a hitsound difficulty from `source` (decompiled from `source_file`, edits included): one
/// centred circle per group of moments at one time (the file's time, rounded half to even), with
/// the group's hits and volume written on the circle by compile (design doc Responsibility 2).
/// Slider body and tick audio are not included in M2: they always need a sample file.
/// `cancel` is looked at between groups and inside decompile and compile; `progress` gets
/// (groups done, groups in total).
pub fn generate(
    source: &Hitsounds,
    source_file: &OsuFile,
    source_file_name: &str,
    folder: &[(String, Option<String>)],
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> GenerateOutcome {
    generate_with_version(
        source,
        source_file,
        source_file_name,
        HITSOUNDS_VERSION,
        folder,
        cancel,
        progress,
    )
}

/// Same as [`generate`], but targets an explicit `version` (e.g. `"Hitsounds 2"`).
pub fn generate_with_version(
    source: &Hitsounds,
    source_file: &OsuFile,
    source_file_name: &str,
    version: &str,
    folder: &[(String, Option<String>)],
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> GenerateOutcome {
    let file_name = file_name_for(source_file, source_file_name, version);
    if folder.iter().any(|(name, _)| name.to_lowercase() == file_name.to_lowercase()) {
        return GenerateOutcome::NameTaken;
    }
    if folder.iter().any(|(_, v)| v.as_deref().is_some_and(|v| v.to_lowercase() == version.to_lowercase())) {
        return GenerateOutcome::VersionTaken;
    }

    // Groups: every moment with a hit, by its file time rounded half to even (old app: .NET's
    // Math.Round), the format-version offset taken off.
    let offset = offset_for(source_file.format_version());
    let mut groups: BTreeMap<i64, Vec<(SampleIdentity, i32)>> = BTreeMap::new();
    for m in source.moments() {
        if m.hits().next().is_none() {
            continue;
        }
        let key = (m.time().ms() - offset).round_ties_even() as i64;
        groups.entry(key).or_default().extend(m.hits().map(|h| (h.clone(), m.hit_volume(h))));
    }

    let mut file = empty_copy(source_file, version);
    file.append_hit_objects(&groups.keys().map(|t| format!("256,192,{t},1,0,0:0:0:0:")).collect::<Vec<_>>());
    let mut circles = match decompile(&file, &[], cancel, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        Decompiled::Cancelled => return GenerateOutcome::Cancelled,
        // The copy keeps the source's mode, which the source's decompile already accepted.
        Decompiled::UnsupportedMode { .. } => return GenerateOutcome::Planned { file, file_name, problems: Vec::new() },
    };

    let mut problems = Vec::new();
    let total = groups.len();
    for (i, layers) in groups.values().enumerate() {
        if cancel() {
            return GenerateOutcome::Cancelled;
        }
        let volumes: BTreeSet<i32> = layers.iter().map(|l| saved_volume(l.1)).collect();
        let lanes: BTreeSet<SampleIdentity> = layers.iter().map(|l| l.0.clone()).collect();
        if volumes.len() > 1 || lanes.len() < layers.len() {
            // One circle holds one volume and each lane once: this group needs a sample mix.
            let m = &circles.moments()[i];
            problems.push(SaveProblem { time: m.time(), place: m.place(), kind: mix_problem(layers) });
        } else {
            set_moment(&mut circles, MomentId(i), &lanes.into_iter().collect::<Vec<_>>(), layers[0].1);
        }
        progress(i + 1, total);
    }

    match compile(&file, &circles, Placement::OnObjects, cancel, &mut |_, _| {}) {
        Compiled::Done { file, problems: more } => {
            problems.extend(more);
            problems.sort_by(|a, b| a.time.ms().total_cmp(&b.time.ms()));
            GenerateOutcome::Planned { file, file_name, problems }
        }
        Compiled::Cancelled => GenerateOutcome::Cancelled,
    }
}

/// The outcome of New hitsound diff.
#[derive(Debug, Clone)]
pub enum NewDiff {
    /// The planned difficulty and the file name it is written under, in the source's folder.
    Planned { file: OsuFile, file_name: String },
    BlankName,
    /// Another difficulty in the folder has this Version (compared ignoring case).
    VersionTaken,
    /// A file with this name is in the folder (compared ignoring case, as Windows does).
    NameTaken,
}

/// Plans a new difficulty from `source`: every hit object removed, `Version:` set to `version`
/// (trimmed) and `BeatmapID:` to 0, every other line kept. `folder` lists the folder's `.osu`
/// files with their Versions, which the caller reads.
pub fn new_hitsound_diff(source: &OsuFile, source_file_name: &str, version: &str, folder: &[(String, Option<String>)]) -> NewDiff {
    let version = version.trim();
    if version.is_empty() {
        return NewDiff::BlankName;
    }
    let file_name = file_name_for(source, source_file_name, version);
    if folder.iter().any(|(name, _)| name.to_lowercase() == file_name.to_lowercase()) {
        return NewDiff::NameTaken;
    }
    if folder.iter().any(|(_, v)| v.as_deref().is_some_and(|v| v.to_lowercase() == version.to_lowercase())) {
        return NewDiff::VersionTaken;
    }
    NewDiff::Planned { file: empty_copy(source, version), file_name }
}

/// The source with no hit objects, the given Version and `BeatmapID:0`.
fn empty_copy(source: &OsuFile, version: &str) -> OsuFile {
    let mut file = source.clone();
    file.remove_all_hit_objects();
    // Neither can be refused: the text holds no line break, and a line that is not UTF-8 keeps
    // its bytes (it stays a difference the change summary shows).
    let _ = file.set_value("Metadata", "Version", version);
    let _ = file.set_value("Metadata", "BeatmapID", "0");
    file
}

/// `Artist - Title (Creator) [Version].osu`, or, without an artist or title, the source's file
/// name with its `[...]` part replaced (old app `HitsoundDiff.FileNameFor`).
pub fn file_name_for(source: &OsuFile, source_file_name: &str, version: &str) -> String {
    let meta = |key: &str| source.value("Metadata", key).unwrap_or_default();
    let (artist, title) = (meta("Artist"), meta("Title"));
    let name = if !artist.is_empty() && !title.is_empty() {
        format!("{artist} - {title} ({}) [{version}].osu", meta("Creator"))
    } else {
        match (source_file_name.rfind('['), source_file_name.ends_with("].osu")) {
            (Some(open), true) => format!("{}[{version}].osu", &source_file_name[..open]),
            _ => source_file_name.to_string(),
        }
    };
    // One fixed set on every system: the characters Windows does not allow in a file name, and
    // control characters (the old app used the running system's set, which differs on Linux).
    name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect()
}
