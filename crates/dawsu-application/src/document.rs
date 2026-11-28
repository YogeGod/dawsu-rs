use crate::app_save_io::{left_out_problem, problem_description};
use crate::editor_meter::{EditorBarSection, EditorTimeSignature};
use crate::{
    DrumLanes, ExportChoices, ExportSettings, LaneLayout, OutputProblem, bookmarks, editor_meter,
    gain_recovery, mixes, pending, project, recovery, refusals, roll_back, sample_path, sound_sets,
    unwritten,
};
use dawsu_editor::tempo::{TempoMap, TimingSection};
use dawsu_editor::transport::BookmarkList;
use dawsu_editor_composer::Composer;
use dawsu_foundation::safe_write::BackupPolicy;
use dawsu_foundation::{SampleIdentity, SongTime, Sound};
use dawsu_hitsounding::{Compiled, Decompiled, Hitsounds, Placement, SaveProblem, compile, decompile};
use dawsu_osu_format::{LineId, OsuFile, SaveOutcome, list_folder};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageSummary {
    pub empty: usize,
    pub fallback_active: bool,
    pub fallback_covered: usize,
    pub matched: usize,
    pub orphaned: usize,
    pub total_objects: usize,
}

pub struct Document {
    pub path: PathBuf,
    pub file: OsuFile,
    pub composer: Composer,
    pub title: String,
    pub files: Vec<String>,
    pub difficulties: Vec<PathBuf>,
    pub hs_mode: bool,
    pub starter_mode: Option<crate::HsStarterMode>,
    pub(crate) compile_base: OsuFile,
    /// The hits as last written: the final compile input of the last Save, or the hits as opened.
    /// A part Save cannot write as drawn goes back to its state here (F052).
    pub(crate) written: Hitsounds,
    /// What the last Save left out of the `.osu`, listed while each part still holds it.
    pub(crate) left_out: Vec<roll_back::LeftOut>,
    pub(crate) owned_hs_circles: BTreeSet<LineId>,
    pub(crate) bookmarks: BookmarkList,
    pub(crate) bookmark_override: bool,
    pub(crate) lane_layout: Option<LaneLayout>,
    pub(crate) time_signatures: Vec<EditorTimeSignature>,
    pub(crate) editor_bars: Vec<EditorBarSection>,
    pub(crate) export_settings: Option<ExportSettings>,
    pub(crate) drum_lanes: Option<DrumLanes>,
    pub(crate) export_choices: Option<ExportChoices>,
    pub(crate) export_index_schema: std::collections::BTreeMap<String, u32>,
    /// Sample files dawsu! made that the mapset does not hold yet. Their names are in the hits'
    /// folder listing, not in `files`.
    pub(crate) pending: pending::Pending,
    /// Sound set snapshots kept as pending files, recorded once a Save writes their files.
    pub(crate) pending_sets: Vec<sound_sets::SnapshotEntry>,
    /// This session's hold on the mapset, so others know its pending names are in use. A cleanup
    /// job holds it too, so the mapset stays marked open until the job ends.
    pub(crate) presence: Option<Arc<pending::Presence>>,
    pub(crate) node_graph_bindings: Option<std::collections::BTreeMap<String, String>>,
}
impl Document {
    /// The map's `Title` from the `.osu` metadata, or empty when it has none.
    pub fn map_title(&self) -> String {
        self.file.value("Metadata", "Title").unwrap_or_default()
    }
    /// The difficulty name (`Version`) from the `.osu` metadata, or empty when it has none.
    pub fn version(&self) -> String {
        self.file.value("Metadata", "Version").unwrap_or_default()
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        Self::read_matching(path, None)
    }
    /// `files` is what is on disk; pending files join the hits' listing later.
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn read_matching(
        path: &Path,
        recovery: Option<&recovery::RecoveryCopy>,
    ) -> Result<Self, String> {
        let canonical = path.canonicalize().map_err(|e| {
            if recovery.is_some() && e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "The difficulty is missing or moved. Put it back at {} and retry. Use Discard to remove this card.",
                    path.display()
                )
            } else {
                format!("Opening {}: {e}", path.display())
            }
        })?;
        let bytes = std::fs::read(&canonical).map_err(|e| e.to_string())?;
        if let Some(copy) = recovery {
            copy.verify_source(&canonical, &bytes)?;
        }
        let path = canonical;
        let read = OsuFile::read(bytes);
        if read.file.format_version().is_none() {
            return Err("This is not an osu! difficulty".into());
        }
        let root = path.parent().ok_or("Missing mapset folder")?;
        let files = match recovery {
            Some(copy) => copy.files().to_vec(),
            None => list_folder(root).map_err(|e| e.to_string())?,
        };
        let compile_base = match recovery.and_then(recovery::RecoveryCopy::original) {
            Some(original) => OsuFile::read(original.to_vec()).file,
            None => read.file.clone(),
        };
        let h = match decompile(&compile_base, &files, &|| false, &mut |_, _| {}) {
            Decompiled::Done(h) => h,
            other => return Err(refusals::open(&other)),
        };
        let sections = read
            .file
            .timing_points()
            .iter()
            .filter(|p| p.uninherited() && p.beat_length() > 0.0)
            .map(|p| TimingSection {
                start: SongTime::from_ms(p.time()),
                beat_length_ms: p.beat_length(),
                beats_per_bar: p.meter().unwrap_or(4).max(1) as u32,
            })
            .collect();
        let title = format!(
            "{} — {} [{}]",
            read.file.value("Metadata", "Artist").unwrap_or_default(),
            read.file.value("Metadata", "Title").unwrap_or_default(),
            read.file.value("Metadata", "Version").unwrap_or_default()
        );
        let difficulties = std::fs::read_dir(root)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("osu")))
            .collect();
        let written = h.clone();
        let mut composer = Composer::new(h);
        composer.tempo = TempoMap::new(sections);
        let bookmarks = bookmarks::import(&read.file);
        Ok(Self {
            path,
            file: read.file,
            compile_base,
            written,
            left_out: Vec::new(),
            owned_hs_circles: BTreeSet::new(),
            bookmarks,
            bookmark_override: false,
            composer,
            title,
            files,
            difficulties,
            hs_mode: false,
            starter_mode: None,
            lane_layout: None,
            time_signatures: Vec::new(),
            editor_bars: Vec::new(),
            export_settings: None,
            drum_lanes: None,
            export_choices: None,
            export_index_schema: std::collections::BTreeMap::new(),
            pending: pending::Pending::default(),
            pending_sets: Vec::new(),
            presence: None,
            node_graph_bindings: None,
        })
    }
    pub fn lane_layout(&self) -> Option<&LaneLayout> {
        self.lane_layout.as_ref()
    }
    pub fn time_signatures(&self) -> &[EditorTimeSignature] {
        &self.time_signatures
    }
    pub fn export_settings(&self) -> Option<&ExportSettings> {
        self.export_settings.as_ref()
    }
    pub fn export_choices(&self) -> Option<&ExportChoices> {
        self.export_choices.as_ref()
    }

    /// Lightweight per-difficulty coverage counts computed in Rust.
    /// When an export Fallback sound is configured, gameplay objects with no explicit
    /// hitsounds count as `fallback_covered` (and are included in `matched`, not `empty`).
    pub fn coverage_summary(&self) -> CoverageSummary {
        let fallback_active = self
            .export_choices
            .as_ref()
            .and_then(|c| c.fallback_sound)
            .is_some();
        let mut matched = 0usize;
        let mut empty = 0usize;
        let mut fallback_covered = 0usize;
        let mut orphaned = 0usize;
        for moment in self.composer.hitsounds().moments() {
            let on_object = moment.place() != dawsu_hitsounding::Place::NoObject;
            let hit_count = moment.hits().count();
            if on_object {
                if hit_count > 0 {
                    matched += 1;
                } else if fallback_active {
                    matched += 1;
                    fallback_covered += 1;
                } else {
                    empty += 1;
                }
            } else if hit_count > 0 {
                orphaned += 1;
            }
        }
        CoverageSummary {
            matched,
            empty,
            fallback_covered,
            orphaned,
            total_objects: matched + empty,
            fallback_active,
        }
    }
    pub fn editor_bars(&self) -> &[EditorBarSection] {
        &self.editor_bars
    }
    pub fn editor_bar_at(&self, time: SongTime) -> (dawsu_editor::tempo::BarBeat, (u32, u32)) {
        let index = self.editor_bars.partition_point(|section| section.start <= time.ms());
        if let Some(section) = self.editor_bars.get(index.saturating_sub(1)) {
            return (section.position(time.ms()), (section.numerator, section.denominator));
        }
        (self.composer.tempo.bar_beat_at(time), (self.composer.tempo.section_at(time).beats_per_bar, 4))
    }
    pub(crate) fn update_time_signatures(&mut self, mut signatures: Vec<EditorTimeSignature>) {
        editor_meter::normalise(&mut signatures);
        self.editor_bars = editor_meter::sections(&self.composer.tempo, &signatures);
        self.time_signatures = signatures;
    }
    pub fn duration_ms(&self) -> f64 {
        self.composer
            .hitsounds()
            .moments()
            .iter()
            .map(|m| m.time().ms())
            .fold(0.0, f64::max)
            + 2000.0
    }
}
pub(crate) const DIFFICULTY_CHANGED: &str =
    "The difficulty changed on disk. Your edits are kept. Reopen or save a separate copy.";

pub struct SaveReport {
    pub file: OsuFile,
    pub problems: Vec<SaveProblem>,
    pub cleaned_greenlines: usize,
    /// Green lines this Save added beyond the ones the difficulty had (each change and its restore line).
    pub added_greenlines: usize,
    pub(crate) written: Hitsounds,
    pub(crate) drawn: Hitsounds,
    pub(crate) mixed: Option<Hitsounds>,
    pub(crate) left_out: Vec<roll_back::LeftOut>,
    pub(crate) hs_circle_ownership: Option<project::HsCircleOwnership>,
    pub(crate) gain_recovery_warning: Option<String>,
    /// What the failed project write after the `.osu` write held, for the status line.
    pub(crate) unfinished_record: String,
    /// The sample mix files this Save wrote or reused in the mapset.
    pub(crate) published_mixes: Vec<String>,
    /// The pending files this Save wrote into the mapset.
    pub(crate) published_pending: Vec<String>,
    /// The pending files the project keeps because the drawn hits play them.
    pub(crate) pending_kept: Vec<String>,
}
pub fn save_document(document: &Document, backups: PathBuf) -> Result<SaveReport, String> {
    save_document_with_cleanup(document, backups, false)
}
pub fn save_document_with_cleanup(
    document: &Document,
    backups: PathBuf,
    clean_redundant_greenlines: bool,
) -> Result<SaveReport, String> {
    prepare_save(
        PrepareSaveInput {
            file: document.compile_base.clone(),
            hits: document.composer.hitsounds(),
            written: &document.written,
            expected: document.file.original_bytes().to_vec(),
            path: &document.path,
            backups,
            hs: document.hs_mode,
            owned: &document.owned_hs_circles,
            defaults: Path::new(""),
            pending: &document.pending,
            project: None,
            review: false,
            clean_redundant_greenlines,
        },
        &|| false,
        &mut Vec::new(),
        &mut |_, _| {},
    )?.apply(None, &|| false)
}
pub(crate) fn compile_for_save(
    file: &OsuFile,
    hits: &Hitsounds,
    hs: bool,
    owned: &BTreeSet<LineId>,
    loose_hits_saved_in_project: bool,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<(OsuFile, Vec<SaveProblem>, BTreeSet<LineId>), String> {
    compile_pass(file, hits, hs, owned, loose_hits_saved_in_project, cancel, progress)
        .map(|pass| (pass.out, pass.problems, pass.owned))
}
/// One compile of Save: the patched difficulty, its problems, the HS circles it owns, and the HS
/// group (by its file time) of each circle it made.
pub(crate) struct Pass {
    pub(crate) out: OsuFile,
    pub(crate) problems: Vec<SaveProblem>,
    pub(crate) owned: BTreeSet<LineId>,
    pub(crate) circles: std::collections::BTreeMap<LineId, i64>,
    pub(crate) removed_circles: BTreeSet<LineId>,
}
pub(crate) fn compile_pass(
    file: &OsuFile,
    hits: &Hitsounds,
    hs: bool,
    owned: &BTreeSet<LineId>,
    loose_hits_saved_in_project: bool,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Pass, String> {
    let mut owned = owned.clone();
    let mut circles = std::collections::BTreeMap::new();
    let mut removed_circles = BTreeSet::new();
    let (out, mut problems) = if hs {
        let groups = dawsu_hitsounding::hs::hs_groups(file, hits);
        // Save adds the green lines a hit needs in HS mode too (Yoge 2026-11-28): under a timing line
        // with a custom index, a skin sound (index 0) on a circle can only play through a green line
        // of its own (lazer LegacyDecoder.cs:192-200, an object's index 0 takes the line's index).
        // On objects only, every such hit was left out. Generate and New hitsound diff stay on objects.
        match dawsu_hitsounding::hs::compile_hs(file, hits, Placement::GreenLines, cancel, progress) {
            dawsu_hitsounding::hs::CompiledHs::Done { mut file, mut problems, created } => {
                let mut removed = BTreeSet::new();
                for moment in hits.moments() {
                    if let dawsu_hitsounding::Place::Circle(id) = moment.place()
                        && moment.hits().next().is_none()
                        && owned.remove(&id.line())
                    {
                        if cancel() {
                            return Err("Save cancelled".into());
                        }
                        file.remove_hit_object(id.line())
                            .map_err(|error| format!("Could not remove an empty circle that HS mode made: {}.", refusals::line(&error)))?;
                        removed.insert(id);
                    }
                }
                problems.retain(|problem| !matches!(problem.place, dawsu_hitsounding::Place::Circle(id) if removed.contains(&id)));
                removed_circles = removed.into_iter().map(dawsu_hitsounding::ObjectId::line).collect();
                circles = created.iter().map(|id| id.line()).zip(groups.into_keys()).collect();
                owned.extend(created.into_iter().map(dawsu_hitsounding::ObjectId::line));
                (file, problems)
            }
            _ => return Err("Save cancelled".into()),
        }
    } else {
        match compile(file, hits, Placement::GreenLines, cancel, progress) {
            Compiled::Done { file, problems } => (file, problems),
            _ => return Err("Save cancelled".into()),
        }
    };
    if loose_hits_saved_in_project && !hs {
        problems.retain(|problem| !matches!(problem.kind, dawsu_hitsounding::ProblemKind::NoObject));
    }
    // Save merges per role, and a slider sound has no role on a hit: that moment is left out.
    for problem in &mut problems {
        if let dawsu_hitsounding::ProblemKind::NeedsSampleMix(layers) = &problem.kind
            && let Some((lane, _)) = layers.iter().find(|(lane, _)| matches!(lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. }))
        {
            problem.kind = dawsu_hitsounding::ProblemKind::SliderSoundOnHit(lane.clone());
        }
    }
    Ok(Pass { out, problems, owned, circles, removed_circles })
}
/// Save's first compile pass, without sample mixes or writing. Recovery checks the last save with it.
pub(crate) fn compile_snapshot(
    document: &Document,
    hits: &Hitsounds,
    hs: bool,
    loose_hits_saved_in_project: bool,
    cancel: &dyn Fn() -> bool,
) -> Result<(OsuFile, Vec<SaveProblem>), String> {
    compile_for_save(
        &document.compile_base, hits, hs, &document.owned_hs_circles,
        loose_hits_saved_in_project, cancel, &mut |_, _| {},
    )
    .map(|(file, problems, _)| (file, problems))
}
pub(crate) struct PrepareSaveInput<'a> {
    pub(crate) file: OsuFile,
    pub(crate) hits: &'a Hitsounds,
    pub(crate) written: &'a Hitsounds,
    pub(crate) expected: Vec<u8>,
    pub(crate) path: &'a Path,
    pub(crate) backups: PathBuf,
    pub(crate) hs: bool,
    pub(crate) owned: &'a BTreeSet<LineId>,
    pub(crate) defaults: &'a Path,
    pub(crate) pending: &'a pending::Pending,
    pub(crate) project: Option<&'a project::Project>,
    pub(crate) review: bool,
    pub(crate) clean_redundant_greenlines: bool,
}

fn gain_recovery_without_project(hits: &Hitsounds, pending: &pending::Pending) -> String {
    // A new sample the gains mix from would be left out with nothing to keep it.
    if let Some(file) = pending::needed(hits, pending, &[]).first() {
        return pending::left_out_without_project(&file.name);
    }
    "Editable hit gains need a writable editor project to save safely.".into()
}

pub(crate) fn prepare_save(
    input: PrepareSaveInput<'_>,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PreparedSave, String> {
    let PrepareSaveInput {
        file,
        hits,
        written,
        expected,
        path,
        backups,
        hs,
        owned,
        defaults,
        pending,
        project,
        review,
        clean_redundant_greenlines,
    } = input;
    let root = path.parent().ok_or("Missing mapset folder")?;
    let inputs = if review {
        let defaults_list = crate::pending::sample_folder(defaults);
        let samples: BTreeSet<_> = hits.moments().iter().flat_map(|moment| moment.hits()).collect();
        let paths: BTreeSet<_> = samples.into_iter().filter_map(|sample| {
            sample_path(hits.lane(sample)?, root, pending, defaults, &defaults_list).ok()
        }).collect();
        let mut inputs = Vec::new();
        for path in paths {
            if cancel() {
                return Err("Save review cancelled".into());
            }
            let digest = sample_digest(&path)?;
            inputs.push((path, digest));
        }
        inputs
    } else {
        Vec::new()
    };
    if project.is_none() && !hits.placements().is_empty() {
        return Err("Sound set links need a writable editor project to save safely.".into());
    }
    if project.is_none()
        && (gain_recovery::Captured::read(hits, root, pending, defaults, &BTreeSet::new())?.needs_recovery()
            || hs && gain_recovery::Captured::needs_hs_no_object_recovery(hits, &file))
    {
        return Err(gain_recovery_without_project(hits, pending));
    }
    // Save adds the green lines a hit needs in HS mode too (Yoge 2026-11-28), as `compile_pass` does.
    let placement = Placement::GreenLines;
    let mut mix_files = mixes::MixFiles::default();
    let mut final_mix_target: Option<Hitsounds> = None;
    let mut checked = false;
    // A part that can't be written as drawn is rolled back first; the mixes the rest needs are
    // then rendered afresh each round. Without a project nothing could keep a left-out part.
    let inputs_for_compile = roll_back::Inputs {
        base: &file, drawn: hits, written, hs, owned, loose_in_project: project.is_some(), roll_back: project.is_some(),
    };
    let rolled = roll_back::compile_with_roll_back(
        &inputs_for_compile,
        &|problem| mixes::can_render(problem, placement),
        &mut |pass, effective, mixable| {
            mix_files = mixes::MixFiles::default();
            final_mix_target = None;
            if mixable.is_empty() {
                return Ok(Ok(pass.out.clone()));
            }
            // Check before touching the mapset. The format writer checks again before replacement.
            if !checked {
                if std::fs::read(path).map_err(|error| error.to_string())? != expected {
                    return Err(DIFFICULTY_CHANGED.into());
                }
                checked = true;
            }
            let files = pending::listing(root, pending)?;
            // This snapshot includes circles added by HS mode and all edits the pass encoded.
            let mut target = match decompile(&pass.out, &files, cancel, &mut |_, _| {}) {
                Decompiled::Done(target) => target,
                Decompiled::Cancelled => return Err("Save cancelled".into()),
                Decompiled::UnsupportedMode { .. } => return Err("Cannot mix this difficulty".into()),
            };
            if let Err(error) = mix_files.prepare_by_role(
                effective, &mut target, &mixable, root, defaults, &expected, pending, cancel,
            ) {
                diagnostics.extend(mixable.iter().map(|problem| {
                    OutputProblem::new(problem, path, path).with_message(&error)
                }));
                return Err(error);
            }
            match compile(&pass.out, &target, placement, cancel, &mut |_, _| {}) {
                Compiled::Done { file, problems } if problems.is_empty() => {
                    final_mix_target = Some(target);
                    Ok(Ok(file))
                }
                Compiled::Done { problems, .. } => Ok(Err(problems)),
                Compiled::Cancelled => Err("Save cancelled".into()),
            }
        },
        cancel,
        progress,
    )?;
    let refused = rolled.refused;
    if !refused.is_empty() {
        diagnostics.extend(refused.iter().map(|problem| OutputProblem::new(problem, path, path)));
        return Err(format!(
            "Save needs review: {} hits cannot be written. First problem at {:.0} ms: {}",
            refused.len(),
            refused[0].time.ms(),
            problem_description(&refused[0].kind, refused[0].time.ms())
        ));
    }
    let (mut out, owned, effective, left_out, circles, removed_circles) = (
        rolled.out, rolled.owned, rolled.effective, rolled.left_out, rolled.circles, rolled.removed_circles,
    );
    // The green lines this Save adds to the difficulty on disk, before any redundant ones go.
    let green_lines = |f: &OsuFile| f.timing_points().iter().filter(|tp| !tp.uninherited()).count();
    let added_greenlines = green_lines(&out).saturating_sub(green_lines(&OsuFile::read(expected.clone()).file));
    let cleaned_greenlines = if clean_redundant_greenlines {
        let cleanup = dawsu_hitsounding::cleanup::clean_up_green_lines(&out);
        out = cleanup.file;
        cleanup.removed
    } else {
        0
    };
    let written = effective.clone();
    // The pending files the written difficulty plays go into the mapset with its mixes; the
    // project keeps the others the drawn hits still play.
    let mix_names: Vec<String> = mix_files.staged.iter()
        .filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)).collect();
    let pending_writes: Vec<mixes::StagedMix> = pending::used(&out, root, pending, &mix_names)?
        .into_iter().map(|(name, bytes)| mixes::StagedMix::pending(root, &name, bytes)).collect();
    let pending_names: Vec<String> = pending_writes.iter()
        .filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)).collect();
    let mut editor = pending::listing(root, pending)?;
    editor.extend(mix_names.iter().cloned());
    editor.sort();
    pending::check_written(&out, root, &[mix_names, pending_names.clone()].concat(), &editor, pending)?;

    pending::check_indexes_free(root, path, &pending_names)?;
    let pending_kept = pending::needed(hits, pending, &pending_names);
    if project.is_none() && let Some(file) = pending_kept.first() {
        return Err(pending::left_out_without_project(&file.name));
    }
    // A sound set placement's moment needs gain recovery only when this Save mixes it.
    let mixed = if hits.placements().is_empty() {
        gain_recovery::Locations::new()
    } else {
        gain_recovery::mixed_locations(&out, root, pending, &mix_files.staged, cancel)?
    };
    let hs_circles = if hs { Some((&file, &circles)) } else { None };
    let recovery = gain_recovery::Captured::read_with_hs(&effective, root, pending, defaults, &mixed, hs_circles)?;
    if cancel() {
        return Err("Save cancelled before writing".into());
    }
    if project.is_none() && recovery.needs_recovery() {
        return Err(gain_recovery_without_project(&effective, pending));
    }
    let recovery_revision = if recovery.needs_recovery() || project
        .and_then(|project| project.settings.gain_recovery.as_ref())
        .is_some_and(|record| record.has_authored_for(&expected))
    {
        Some(recovery.revision(
            &gain_recovery::RevisionInput {
                hits: &effective,
                file: &out,
                root,
                pending,
                defaults,
                staged: &mix_files.staged,
                hs_circles,
            },
            cancel,
        )?)
    } else {
        None
    };
    // The left-out parts go to the project: a record for the written file, and loose hits for an
    // HS group that never became a circle. An old record with parts is replaced, even by an empty one.
    let (unwritten, loose) = match project {
        Some(project) => {
            let files = if left_out.is_empty() { Vec::new() } else { pending::listing(root, pending)? };
            let built = unwritten::build(&left_out, &out, &files, &circles)?;
            let replaces = project.settings.unwritten.as_ref().is_some_and(|old| !old.is_empty());
            ((!built.record.is_empty() || replaces).then_some(built.record), built.loose)
        }
        None => (None, Vec::new()),
    };
    // The sound set links go to the project too; an old record with links is replaced.
    let sound_sets = project.and_then(|project| {
        let record = sound_sets::build_links(hits, &file, &out, &circles, hs, &project.kept_links, &project.accepted_versions);
        let replaces = project.settings.sound_sets.as_ref().is_some_and(|old| !old.is_empty());
        (!record.is_empty() || replaces).then_some(record)
    });
    let bytes = out.to_bytes();
    let verify_expected = build_verify_expected(
        &file,
        &effective,
        hs,
        &circles,
        &removed_circles,
        final_mix_target.as_ref(),
        &editor,
    )?;
    dawsu_hitsounding::verify_compiled_osu(&bytes, &verify_expected, &editor)
        .map_err(|mismatch| mismatch.to_string())?;
    let ownership = project::HsCircleOwnership::capture(&out, &bytes, &owned);
    let map_changed = bytes != expected;
    // With the map unchanged, a revision the project already holds is not written again.
    let recovery_revision = recovery_revision.filter(|revision| map_changed || !project
        .and_then(|project| project.settings.gain_recovery.as_ref())
        .is_some_and(|record| record.is_current(&expected, revision)));
    let plan = PreparedSave {
        path: path.to_path_buf(), backups, expected, out, cleaned_greenlines, added_greenlines, mix_files, recovery_revision,
        ownership, inputs, map_changed, left_out, written, unwritten, sound_sets, loose, drawn: hits.clone(),
        mixed: final_mix_target,
        pending_writes, pending_kept, ledger: pending.ledger(),
        verify_expected, verify_folder_files: editor,
    };
    plan.check_current(project, cancel)?;
    // Left-out parts are listed for the Save hitsounds summary, which every Save shows first.
    diagnostics.extend(plan.left_out.iter().map(|entry| left_out_problem(entry, path)));
    Ok(plan)
}

/// Builds the expected hitsound state for read-back verification of a compiled Save output.
pub(crate) fn build_verify_expected(
    compile_base: &OsuFile,
    effective: &Hitsounds,
    hs: bool,
    created_circles: &std::collections::BTreeMap<i64, LineId>,
    removed_circles: &BTreeSet<LineId>,
    mix_target: Option<&Hitsounds>,
    folder_files: &[String],
) -> Result<Hitsounds, String> {
    let mut base_file = compile_base.clone();
    let mut created_lines: Vec<(i64, LineId)> = Vec::new();
    if hs {
        for &line in removed_circles {
            base_file.remove_hit_object(line).map_err(|error| {
                format!(
                    "Save verification could not model removed circle: {}.",
                    refusals::line(&error)
                )
            })?;
        }
        for &time in created_circles.keys() {
            let line = base_file.insert_hit_object(&format!("256,192,{time},1,0,0:0:0:0:"));
            created_lines.push((time, line));
        }
    }
    let mut expected = match decompile(&base_file, folder_files, &|| false, &mut |_, _| {}) {
        Decompiled::Done(h) => h,
        other => {
            return Err(format!(
                "Save verification could not decompile base difficulty: {}",
                refusals::open(&other)
            ));
        }
    };
    for b in effective.bodies().iter().filter(|b| b.is_changed()) {
        if let Some(i) = expected.bodies().iter().position(|t| t.object() == b.object()) {
            expected.set_body_state(i, b.state());
        }
    }
    let by_place: HashMap<dawsu_hitsounding::Place, dawsu_hitsounding::MomentId> = expected
        .moments()
        .iter()
        .enumerate()
        .map(|(i, m)| (m.place(), dawsu_hitsounding::MomentId(i)))
        .collect();
    for m in effective
        .moments()
        .iter()
        .filter(|m| m.is_changed() && m.place() != dawsu_hitsounding::Place::NoObject)
    {
        if let Some(&id) = by_place.get(&m.place()) {
            let hits: Vec<_> = m
                .hits()
                .map(|lane| (lane.clone(), m.hit_volume(lane)))
                .collect();
            expected.set_moment_state(id, m.volume(), hits);
        }
    }
    if hs && !created_lines.is_empty() {
        let groups = dawsu_hitsounding::hs::hs_groups(compile_base, effective);
        for &(time, line) in &created_lines {
            let Some(members) = groups.get(&time) else { continue };
            let Some(id) = expected.moments().iter().enumerate().find_map(|(i, m)| {
                matches!(m.place(), dawsu_hitsounding::Place::Circle(obj) if obj.line() == line)
                    .then_some(dawsu_hitsounding::MomentId(i))
            }) else {
                continue;
            };
            let layers: Vec<(SampleIdentity, i32)> = members
                .iter()
                .map(|mid| effective.moment(*mid))
                .flat_map(|m| m.hits().map(|h| (h.clone(), m.hit_volume(h))))
                .collect();
            let volumes: BTreeSet<i32> = layers
                .iter()
                .map(|(_, v)| dawsu_hitsounding::rules::saved_volume(*v))
                .collect();
            let lanes: BTreeSet<&SampleIdentity> = layers.iter().map(|(l, _)| l).collect();
            if !layers.is_empty() && volumes.len() == 1 && lanes.len() == layers.len() {
                expected.set_moment_state(id, layers[0].1, layers);
            }
        }
    }
    if let Some(mix_target) = mix_target {
        if expected.moments().len() != mix_target.moments().len() {
            return Err(format!(
                "Save verification stopped before writing: expected {} hit moments, but mix pass had {}.",
                expected.moments().len(),
                mix_target.moments().len(),
            ));
        }
        for (i, m) in mix_target.moments().iter().enumerate() {
            if m.is_changed() || effective.moments().get(i).is_some_and(|em| em.is_changed()) {
                let hits: Vec<_> = m
                    .hits()
                    .map(|lane| (lane.clone(), m.hit_volume(lane)))
                    .collect();
                expected.set_moment_state(dawsu_hitsounding::MomentId(i), m.volume(), hits);
            }
        }
    }
    Ok(expected)
}

/// The status line's words for project records a failed write after the `.osu` write could not keep.
pub(crate) fn unfinished_record(recovery: bool, left_out: bool, links: bool) -> String {
    if recovery && !left_out && !links {
        return "Gain recovery could not be finalized.".into();
    }
    let parts: Vec<&str> = [(recovery, "gain recovery"), (left_out, "left-out moments"), (links, "sound set links")]
        .into_iter().filter_map(|(on, name)| on.then_some(name)).collect();
    let named = match parts.as_slice() {
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => "the project record".to_string(),
    };
    let mut chars = named.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    format!("{first}{} could not be recorded.", chars.as_str())
}

/// What a Save wrote into the `.osu`, for the next Save to fall back on. Outside HS mode loose hits
/// are kept in the project, never as circles, so a group that was never a circle can't come back
/// as one.
pub(crate) fn last_written(compiled: &Hitsounds, hs: bool) -> Hitsounds {
    let mut written = compiled.clone();
    if !hs {
        let loose: Vec<_> = written.moments().iter().enumerate()
            .filter(|(_, m)| m.place() == dawsu_hitsounding::Place::NoObject)
            .map(|(index, _)| dawsu_hitsounding::MomentId(index)).collect();
        for id in loose {
            written.reset_moment(id);
        }
    }
    written
}

pub(crate) fn sample_digest(path: &Path) -> Result<String, String> {
    std::fs::read(path)
        .map(|bytes| dawsu_foundation::digest::sha256_hex(&bytes))
        .map_err(|error| format!("Reading {}: {error}", path.display()))
}

pub(crate) struct PreparedSave {
    pub(crate) path: PathBuf,
    pub(crate) backups: PathBuf,
    pub(crate) expected: Vec<u8>,
    pub(crate) out: OsuFile,
    pub(crate) cleaned_greenlines: usize,
    pub(crate) added_greenlines: usize,
    pub(crate) mix_files: mixes::MixFiles,
    pub(crate) recovery_revision: Option<gain_recovery::Revision>,
    pub(crate) ownership: Option<project::HsCircleOwnership>,
    pub(crate) inputs: Vec<(PathBuf, String)>,
    pub(crate) map_changed: bool,
    /// The parts Save leaves out of the `.osu`.
    pub(crate) left_out: Vec<roll_back::LeftOut>,
    /// The final compile input: what this Save writes, with the loose hits kept in the project.
    pub(crate) written: Hitsounds,
    /// The hits as drawn when this Save was prepared.
    pub(crate) drawn: Hitsounds,
    /// The decompiled post-mix target compiled into `out`, when this Save rendered mixes.
    pub(crate) mixed: Option<Hitsounds>,
    /// The project record of the left-out parts, when the project needs a new one.
    pub(crate) unwritten: Option<unwritten::Record>,
    /// The project record of the sound set links, when the project needs a new one.
    pub(crate) sound_sets: Option<sound_sets::LinkRecord>,
    /// HS groups left out before they ever became a circle; the project keeps them as loose hits.
    pub(crate) loose: Vec<roll_back::MomentState>,
    /// The pending files the written difficulty plays, published with the mixes.
    pub(crate) pending_writes: Vec<mixes::StagedMix>,
    /// The pending files the drawn hits play that this Save leaves out of the mapset.
    pub(crate) pending_kept: Vec<pending::PendingSample>,
    /// Where the files this Save creates are recorded.
    pub(crate) ledger: Option<pending::Ledger>,
    /// Expected hitsound state for read-back verification right before writing to disk.
    pub(crate) verify_expected: Hitsounds,
    /// Folder listing (including staged mixes and pending files) for read-back verification.
    pub(crate) verify_folder_files: Vec<String>,
}

impl PreparedSave {
    pub(crate) fn check_current(&self, project: Option<&project::Project>, cancel: &dyn Fn() -> bool) -> Result<(), String> {
        if std::fs::read(&self.path).map_err(|error| error.to_string())? != self.expected {
            return Err(DIFFICULTY_CHANGED.into());
        }
        if let Some(project) = project {
            project.check_current()?;
        }
        for (path, expected) in &self.inputs {
            if cancel() {
                return Err("Save cancelled before writing".into());
            }
            if sample_digest(path).as_ref() != Ok(expected) {
                return Err(format!("{} changed since the Save review. Review Save again.", path.display()));
            }
        }
        if cancel() {
            return Err("Save cancelled before writing".into());
        }
        Ok(())
    }

    pub(crate) fn apply(mut self, mut project: Option<&mut project::Project>, cancel: &dyn Fn() -> bool) -> Result<SaveReport, String> {
        self.check_current(project.as_deref(), cancel)?;
        let needs_recovery_write = self.recovery_revision.is_some();
        // Gain recovery and the left-out parts are staged in one project write before the `.osu`
        // write and promoted in one write after it. With the map bytes unchanged there is nothing
        // to wait for, so the left-out record is written as current straight away.
        let promote_unwritten = self.unwritten.is_some() && self.map_changed;
        let promote_links = self.sound_sets.is_some() && self.map_changed;
        // The pending files this Save leaves out are listed before the `.osu` is written, so a
        // crash in between can't lose them. The list after the write is exact again.
        let list_pending = !self.pending_kept.is_empty() && project.is_some();
        let staged = needs_recovery_write || self.unwritten.is_some() || self.sound_sets.is_some() || list_pending;
        if staged {
            let project = project.as_mut().expect("a project record needs a project");
            let mut settings = project.settings.clone();
            if list_pending {
                let listed = settings.pending_samples.get_or_insert_with(Vec::new);
                for file in &self.pending_kept {
                    if !listed.contains(file) {
                        listed.push(file.clone());
                    }
                }
            }
            if let Some(revision) = self.recovery_revision {
                settings.gain_recovery.get_or_insert_with(Default::default).stage(&self.expected, revision);
            }
            if let Some(record) = self.unwritten {
                let unwritten = settings.unwritten.get_or_insert_with(Default::default);
                if self.map_changed {
                    unwritten.stage(&self.expected, record);
                } else {
                    unwritten.set_current(record);
                }
            }
            if let Some(record) = self.sound_sets {
                let links = settings.sound_sets.get_or_insert_with(Default::default);
                if self.map_changed {
                    links.stage(&self.expected, record);
                } else {
                    links.set_current(record);
                }
            }
            project.save(settings)?;
        }

        // One all-or-nothing write: the mixes, then the pending files the difficulty plays.
        let published_mixes: Vec<String> = self.mix_files.staged.iter()
            .filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)).collect();
        let published_pending: Vec<String> = self.pending_writes.iter()
            .filter_map(|mix| mix.path.file_name()?.to_str().map(str::to_owned)).collect();
        let mut writes = std::mem::take(&mut self.mix_files.staged);
        writes.append(&mut self.pending_writes);
        let publication = mixes::publish(&writes, self.ledger.as_ref(), cancel)?;
        if staged
            && let Err(error) = project.as_ref().expect("a staged record has a project").check_current()
        {
            return Err(publication.failed(error));
        }
        if cancel() {
            return Err(publication.failed("Save cancelled before writing".into()));
        }
        let out_bytes = self.out.to_bytes();
        if let Err(mismatch) = dawsu_hitsounding::verify_compiled_osu(
            &out_bytes,
            &self.verify_expected,
            &self.verify_folder_files,
        ) {
            return Err(publication.failed(mismatch.to_string()));
        }
        match self.out.save_over_expected(&self.path, &self.expected, &BackupPolicy::new(self.backups, 20)) {
            Ok(SaveOutcome::Saved { .. } | SaveOutcome::NothingToSave) => {
                publication.keep();
                let bytes = self.out.to_bytes();
                let gain_recovery_warning = if needs_recovery_write || promote_unwritten || promote_links {
                    let project = project.expect("a staged record has a project");
                    let mut settings = project.settings.clone();
                    if needs_recovery_write {
                        settings.gain_recovery.as_mut().expect("recovery was staged").promote(&bytes);
                    }
                    if promote_unwritten {
                        settings.unwritten.as_mut().expect("the left-out parts were staged").promote(&bytes);
                    }
                    if promote_links {
                        settings.sound_sets.as_mut().expect("the sound set links were staged").promote(&bytes);
                    }
                    project.save(settings).err()
                } else {
                    None
                };
                let unfinished_record = unfinished_record(needs_recovery_write, promote_unwritten, promote_links);
                Ok(SaveReport {
                    file: OsuFile::read(bytes).file,
                    problems: Vec::new(),
                    cleaned_greenlines: self.cleaned_greenlines,
                    added_greenlines: self.added_greenlines,
                    written: self.written,
                    drawn: self.drawn,
                    mixed: self.mixed,
                    left_out: self.left_out,
                    hs_circle_ownership: self.ownership,
                    gain_recovery_warning,
                    unfinished_record,
                    published_mixes,
                    published_pending,
                    pending_kept: self.pending_kept.iter().map(|file| file.name.clone()).collect(),
                })
            }
            Ok(SaveOutcome::ChangedOnDisk) => Err(publication.failed(DIFFICULTY_CHANGED.into())),
            Ok(SaveOutcome::AlreadyExists) => Err(publication.failed("The target already exists".into())),
            Err(error) => Err(publication.failed(error.to_string())),
        }
    }
}
