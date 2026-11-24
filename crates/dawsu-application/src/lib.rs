//! Documents, playback, storage and background jobs.

mod bookmarks;
mod default_sounds;
mod slider_body;
mod slider_export;
mod commands;
mod edit_workflows;
mod controller;
mod editor_meter;
pub mod drum_classify;
mod drum_detect;
pub mod drum_vst;
mod export;
mod export_settings;
mod generate_receipt;
mod gain_recovery;
mod lane_edit;
mod layers;
mod mixes;
mod original_snapshot;
mod pending;
mod sweep;
pub mod output;
pub mod node_graph;
pub mod midi_import;
pub mod midi_record;
pub mod vst3;
mod keysound_session;
mod project;
pub use project::lane_key as lane_layout_key;
pub use node_graph::{NodeGraphAction, NodeGraphView, ViewerOverlay};
pub use dawsu_audio::{OutputBackend, OutputDevice};
pub use dawsu_audio_instruments::midi_in::{
    MidiInputEvent, MidiInputRouter, decode_midi_bytes, list_midi_input_ports,
};
pub use midi_import::{
    MidiImportOptions, MidiImportOutcome, MidiInspectResult, MidiTrackSummary, apply_midi_import,
    export_midi_file, inspect_midi_file,
};
pub use midi_record::MidiRecordStatus;
pub use vst3::{
    BounceFormat, BounceOptions, BounceOutput, LoadedPluginSummary, MidiPreviewNote, PluginRole,
    ScanSkip, VelocityExportMode, Vst3BounceJob, Vst3DrumExportPlan, Vst3DrumHit, Vst3Error,
    Vst3HitAssignment, Vst3Manager, Vst3ScanReport, plan_vst3_drum_export,
};
mod preferences;
mod shortcuts;
mod problems;
mod recovery;
mod refusals;
mod roll_back;
mod sound_sets;
mod unwritten;
mod sample_watch;
pub mod waveform;
use dawsu_editor::commands::{
    CommandCatalog, EditorScope, HelpGroup, KeyMap, KeyOutcome, RunOutcome,
};
use dawsu_editor::keys::{KeyChord, Mods};
use dawsu_editor::transport::BookmarkList;
pub use problems::{OutputProblem, orphaned_hit_times};
pub use editor_meter::{EditorBarSection, EditorTimeSignature};
pub use dawsu_graph_audio::classic::Drum;
pub use drum_detect::{DrumDetectState, DrumDetectView, DrumLanes, DrumOptions, DrumRow, DrumSource, ObjectKind, RowLands, GRIDS, MAX_OBJECT_MS};
pub use export::{ExportChoices, ExportOptions, ExportPreview, ExportStatus, PreviewOrigin, Stale};
pub use export_settings::{ExportCombination, ExportSettings};
pub use project::LaneLayout;
pub use recovery::RecoveryOffer;
pub use sound_sets::{LibrarySet, PlacementView, SaveSetError, SetUpdatePrompt, SoundSetLibrary};
pub use waveform::WaveBin;
pub use edit_workflows::{PatternStampOptions, RangeOperation, TimelineEdit, VolumeScope};

use dawsu_audio::{AudioClip, AudioEngine, ScheduledSound};
use dawsu_editor::tempo::{SnapSpacing, TempoMap, TimingSection};
use dawsu_editor::tools::{PointerButton, PointerInput, PointerSurface, ToolBox, ToolKind, ToolTarget};
use dawsu_editor::volume_ramp::RampShape;
use dawsu_editor_composer::{Composer, HitKey};
use dawsu_foundation::safe_write::BackupPolicy;
use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, SliderSound, SongTime, Sound};
use dawsu_hitsounding::{
    Compiled, Decompiled, Hitsounds, Lane, LaneSource, Placement, SaveProblem, compile,
    cue_list_filtered, decompile,
};
use dawsu_osu_format::{LineId, OsuFile, SaveOutcome, check_map_path, find_case_insensitive, list_folder};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

pub struct Document {
    pub path: PathBuf,
    pub file: OsuFile,
    pub composer: Composer,
    pub title: String,
    pub files: Vec<String>,
    pub difficulties: Vec<PathBuf>,
    pub hs_mode: bool,
    compile_base: OsuFile,
    /// The hits as last written: the final compile input of the last Save, or the hits as opened.
    /// A part Save cannot write as drawn goes back to its state here (F052).
    written: Hitsounds,
    /// What the last Save left out of the `.osu`, listed while each part still holds it.
    left_out: Vec<roll_back::LeftOut>,
    owned_hs_circles: BTreeSet<LineId>,
    bookmarks: BookmarkList,
    bookmark_override: bool,
    lane_layout: Option<LaneLayout>,
    time_signatures: Vec<EditorTimeSignature>,
    editor_bars: Vec<EditorBarSection>,
    export_settings: Option<ExportSettings>,
    drum_lanes: Option<DrumLanes>,
    export_choices: Option<ExportChoices>,
    /// Sample files dawsu! made that the mapset does not hold yet. Their names are in the hits'
    /// folder listing, not in `files`.
    pending: pending::Pending,
    /// Sound set snapshots kept as pending files, recorded once a Save writes their files.
    pending_sets: Vec<sound_sets::SnapshotEntry>,
    /// This session's hold on the mapset, so others know its pending names are in use. A cleanup
    /// job holds it too, so the mapset stays marked open until the job ends.
    presence: Option<Arc<pending::Presence>>,
    node_graph_bindings: Option<std::collections::BTreeMap<String, String>>,
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
    fn read_matching(
        path: &Path,
        recovery: Option<&recovery::RecoveryCopy>,
    ) -> Result<Self, String> {
        let canonical = path.canonicalize().map_err(|e| {
            if recovery.is_some() && e.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "The difficulty is missing or moved. Put it back at {} and retry.",
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
            lane_layout: None,
            time_signatures: Vec::new(),
            editor_bars: Vec::new(),
            export_settings: None,
            drum_lanes: None,
            export_choices: None,
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
    fn update_time_signatures(&mut self, mut signatures: Vec<EditorTimeSignature>) {
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
const DIFFICULTY_CHANGED: &str =
    "The difficulty changed on disk. Your edits are kept. Reopen or save a separate copy.";
/// The farthest the timeline zooms out, in pixels per millisecond: low enough that Fit shows a
/// 30-minute song in a 1000 px timeline. The view, Zoom out and reopening all use it.
pub const MIN_ZOOM: f64 = 0.0005;
pub const DEFAULT_SONG_OFFSET_MS: f64 = -22.0;

pub struct SaveReport {
    pub file: OsuFile,
    pub problems: Vec<SaveProblem>,
    written: Hitsounds,
    drawn: Hitsounds,
    left_out: Vec<roll_back::LeftOut>,
    hs_circle_ownership: Option<project::HsCircleOwnership>,
    gain_recovery_warning: Option<String>,
    /// What the failed project write after the `.osu` write held, for the status line.
    unfinished_record: String,
    /// The pending files this Save wrote into the mapset.
    published_pending: Vec<String>,
    /// The pending files the project keeps because the drawn hits play them.
    pending_kept: Vec<String>,
}
pub fn save_document(document: &Document, backups: PathBuf) -> Result<SaveReport, String> {
    prepare_save(
        document.compile_base.clone(), document.composer.hitsounds(), &document.written,
        document.file.original_bytes().to_vec(), &document.path, backups, document.hs_mode,
        &document.owned_hs_circles, Path::new(""), &document.pending, None, false,
        &|| false, &mut Vec::new(), &mut |_, _| {},
    )?.apply(None, &|| false)
}
fn compile_for_save(
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
struct Pass {
    out: OsuFile,
    problems: Vec<SaveProblem>,
    owned: BTreeSet<LineId>,
    circles: std::collections::BTreeMap<LineId, i64>,
}
fn compile_pass(
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
    let (out, mut problems) = if hs {
        let groups = dawsu_hitsounding::hs::hs_groups(file, hits);
        match dawsu_hitsounding::hs::compile_hs(file, hits, Placement::OnObjects, cancel, progress) {
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
    Ok(Pass { out, problems, owned, circles })
}
/// Save's first compile pass, without sample mixes or writing. Recovery checks the last save with it.
fn compile_snapshot(
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
fn prepare_save(
    file: OsuFile,
    hits: &Hitsounds,
    written: &Hitsounds,
    expected: Vec<u8>,
    path: &Path,
    backups: PathBuf,
    hs: bool,
    owned: &BTreeSet<LineId>,
    defaults: &Path,
    pending: &pending::Pending,
    project: Option<&project::Project>,
    review: bool,
    cancel: &dyn Fn() -> bool,
    diagnostics: &mut Vec<OutputProblem>,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<PreparedSave, String> {
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
    if project.is_none() && gain_recovery::Captured::read(hits, root, pending, defaults, &BTreeSet::new())?.needs_recovery() {
        // A new sample the gains mix from would be left out with nothing to keep it.
        if let Some(file) = pending::needed(hits, pending, &[]).first() {
            return Err(pending::left_out_without_project(&file.name));
        }
        return Err("Editable hit gains need a writable editor project to save safely.".into());
    }
    let placement = if hs { Placement::OnObjects } else { Placement::GreenLines };
    let mut mix_files = mixes::MixFiles::default();
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
                Compiled::Done { file, problems } if problems.is_empty() => Ok(Ok(file)),
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
    let (out, owned, effective, left_out, circles) = (rolled.out, rolled.owned, rolled.effective, rolled.left_out, rolled.circles);
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
    let recovery_revision = if recovery.needs_recovery() || project
        .and_then(|project| project.settings.gain_recovery.as_ref())
        .is_some_and(|record| record.has_authored_for(&expected))
    {
        Some(recovery.revision(&effective, &out, root, pending, defaults, &mix_files.staged, cancel)?)
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
    let ownership = project::HsCircleOwnership::capture(&out, &bytes, &owned);
    let map_changed = bytes != expected;
    // With the map unchanged, a revision the project already holds is not written again.
    let recovery_revision = recovery_revision.filter(|revision| map_changed || !project
        .and_then(|project| project.settings.gain_recovery.as_ref())
        .is_some_and(|record| record.is_current(&expected, revision)));
    let plan = PreparedSave {
        path: path.to_path_buf(), backups, expected, out, mix_files, recovery_revision,
        ownership, inputs, map_changed, left_out, written, unwritten, sound_sets, loose, drawn: hits.clone(),
        pending_writes, pending_kept, ledger: pending.ledger(),
    };
    plan.check_current(project, cancel)?;
    // Left-out parts are listed for the Save hitsounds summary, which every Save shows first.
    diagnostics.extend(plan.left_out.iter().map(|entry| left_out_problem(entry, path)));
    Ok(plan)
}

/// The status line's words for project records a failed write after the `.osu` write could not keep.
fn unfinished_record(recovery: bool, left_out: bool, links: bool) -> String {
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
fn last_written(compiled: &Hitsounds, hs: bool) -> Hitsounds {
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

fn sample_digest(path: &Path) -> Result<String, String> {
    std::fs::read(path)
        .map(|bytes| dawsu_foundation::digest::sha256_hex(&bytes))
        .map_err(|error| format!("Reading {}: {error}", path.display()))
}

struct PreparedSave {
    path: PathBuf,
    backups: PathBuf,
    expected: Vec<u8>,
    out: OsuFile,
    mix_files: mixes::MixFiles,
    recovery_revision: Option<gain_recovery::Revision>,
    ownership: Option<project::HsCircleOwnership>,
    inputs: Vec<(PathBuf, String)>,
    map_changed: bool,
    /// The parts Save leaves out of the `.osu`.
    left_out: Vec<roll_back::LeftOut>,
    /// The final compile input: what this Save writes, with the loose hits kept in the project.
    written: Hitsounds,
    /// The hits as drawn when this Save was prepared.
    drawn: Hitsounds,
    /// The project record of the left-out parts, when the project needs a new one.
    unwritten: Option<unwritten::Record>,
    /// The project record of the sound set links, when the project needs a new one.
    sound_sets: Option<sound_sets::LinkRecord>,
    /// HS groups left out before they ever became a circle; the project keeps them as loose hits.
    loose: Vec<roll_back::MomentState>,
    /// The pending files the written difficulty plays, published with the mixes.
    pending_writes: Vec<mixes::StagedMix>,
    /// The pending files the drawn hits play that this Save leaves out of the mapset.
    pending_kept: Vec<pending::PendingSample>,
    /// Where the files this Save creates are recorded.
    ledger: Option<pending::Ledger>,
}

impl PreparedSave {
    fn check_current(&self, project: Option<&project::Project>, cancel: &dyn Fn() -> bool) -> Result<(), String> {
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

    fn apply(mut self, mut project: Option<&mut project::Project>, cancel: &dyn Fn() -> bool) -> Result<SaveReport, String> {
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
                    written: self.written,
                    drawn: self.drawn,
                    left_out: self.left_out,
                    hs_circle_ownership: self.ownership,
                    gain_recovery_warning,
                    unfinished_record,
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
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub recent: Vec<PathBuf>,
    pub songs_folder: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hitsound_library: Option<PathBuf>,
    pub default_samples: PathBuf,
    pub device: Option<String>,
    #[serde(skip_serializing_if = "system_audio_backend")]
    pub audio_backend: OutputBackend,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_buffer_ms: Option<u32>,
    pub song_gain: f32,
    pub song_muted: bool,
    #[serde(default = "default_hitsound_gain")]
    pub hitsound_gain: f32,
    #[serde(default)]
    pub hitsounds_muted: bool,
    pub master_gain: f32,
    pub audition: bool,
    pub follow: bool,
    pub return_on_stop: bool,
    pub show_browser: bool,
    pub show_inspector: bool,
    pub show_controller: bool,
    pub used_only: bool,
    pub lane_names: bool,
    pub object_lines: bool,
    pub classic_browser: bool,
    pub theme: usize,
    pub key_preset: String,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub key_overrides: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>>,
    pub protect_hitnormals: bool,
    /// The map preview's own window: where it was, and whether it was open when the editor closed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_window: Option<WindowPlace>,
}

/// A window's place and size in logical pixels, and whether it was open.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPlace {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub open: bool,
}
fn default_hitsound_gain() -> f32 {
    1.0
}

fn system_audio_backend(backend: &OutputBackend) -> bool {
    *backend == OutputBackend::System
}

fn open_preferred_audio(preferences: &Preferences, buffer_ms: Option<u32>) -> Result<AudioEngine, String> {
    match (preferences.audio_backend, preferences.audio_device_id.as_deref(), preferences.device.as_deref()) {
        (OutputBackend::System, None, Some(legacy_name)) => AudioEngine::open_with_buffer(Some(legacy_name), buffer_ms),
        (backend, device, _) => AudioEngine::open_output(backend, device, buffer_ms),
    }
}

impl Preferences {
    fn effective_song_gain(&self) -> f32 {
        if self.song_muted { 0.0 } else { self.song_gain }
    }
    fn effective_hitsound_gain(&self) -> f32 {
        if self.hitsounds_muted { 0.0 } else { self.hitsound_gain }
    }
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            recent: Vec::new(),
            songs_folder: PathBuf::new(),
            hitsound_library: None,
            default_samples: PathBuf::new(),
            device: None,
            audio_backend: OutputBackend::System,
            audio_device_id: None,
            audio_buffer_ms: None,
            song_gain: 0.7,
            song_muted: false,
            hitsound_gain: 1.0,
            hitsounds_muted: false,
            master_gain: 0.2,
            audition: true,
            follow: false,
            return_on_stop: true,
            show_browser: true,
            show_inspector: true,
            show_controller: true,
            used_only: false,
            lane_names: true,
            object_lines: false,
            classic_browser: false,
            theme: 0,
            key_preset: "DAW".into(),
            key_overrides: Default::default(),
            protect_hitnormals: false,
            preview_window: None,
        }
    }
}
/// The developer's local default sounds, used by the desktop app when no default sounds
/// folder is saved ([`Application::for_desktop`]). On other computers the folder is missing
/// and nothing changes.
const LOCAL_DEFAULT_SAMPLES: &str = "E:/GitHub/dawsu-hitsounding-editor/src/OsuHitsound.Audio/DefaultSamples";
pub fn data_folder() -> PathBuf {
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        PathBuf::from(path).join("dawsu-rs")
    } else if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        PathBuf::from(path).join("dawsu")
    } else {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share/dawsu")
    }
}
#[derive(Debug, Clone)]
pub enum PlatformRequest {
    OpenMap,
    OpenSamples,
    OpenSongs,
    OpenFolder(PathBuf),
}
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
}
struct Loaded {
    document: Document,
    project: Option<project::Project>,
    song_path: Option<PathBuf>,
    samples: HashMap<SampleIdentity, Arc<AudioClip>>,
    warnings: Vec<String>,
    recovery: Option<recovery::RecoveryCopy>,
    /// The pending names a record of the outgoing mapset still lists, read by the open job.
    outgoing_keep: Option<BTreeSet<String>>,
}
enum OpenSource {
    Map(PathBuf),
    Recovery(PathBuf),
}
struct PendingSave {
    version: u64,
    hs: bool,
    plan: PreparedSave,
    project: Option<project::Project>,
    settings: Option<project::SessionSettings>,
}

impl PendingSave {
    fn finish(mut self, cancel: &dyn Fn() -> bool) -> JobResult {
        let path = self.plan.path.clone();
        let result = self.plan.apply(self.project.as_mut(), cancel);
        JobResult::Save {
            version: self.version, path, hs_mode: self.hs, project: self.project, settings: self.settings,
            result, diagnostics: Vec::new(),
        }
    }
}

enum JobResult {
    Open(Option<u64>, bool, Result<Loaded, String>),
    SongReady {
        generation: u64,
        song: Option<Arc<AudioClip>>,
        waveform: Vec<WaveBin>,
        warning: Option<String>,
    },
    Save {
        version: u64,
        path: PathBuf,
        hs_mode: bool,
        settings: Option<project::SessionSettings>,
        project: Option<project::Project>,
        result: Result<SaveReport, String>,
        diagnostics: Vec<OutputProblem>,
    },
    SaveCheck {
        version: u64,
        path: PathBuf,
        hs: bool,
        result: Result<PendingSave, String>,
        diagnostics: Vec<OutputProblem>,
    },
    Review(Result<output::WritePlan, String>, Vec<OutputProblem>),
    /// The files written, a receipt warning, and the pending files the write put in the mapset.
    Output(Result<(usize, Option<String>, Vec<String>), String>),
}
#[derive(Clone)]
struct JobProgress {
    label: &'static str,
    current: Arc<Mutex<Option<(usize, usize)>>>,
}

impl JobProgress {
    fn report(&self, done: usize, total: usize) {
        if let Ok(mut current) = self.current.lock() {
            *current = Some((done, total));
        }
    }
}

pub struct Application {
    sample_watch: sample_watch::Watch,
    document: Option<Document>,
    audio: Option<AudioEngine>,
    samples: HashMap<SampleIdentity, Arc<AudioClip>>,
    song: Option<Arc<AudioClip>>,
    /// The sample file last auditioned or outlined from the Browser, kept while the file is unchanged
    /// so a pick decodes it once for both.
    sample_file: Option<(PathBuf, (u64, std::time::SystemTime), Arc<AudioClip>)>,
    pub playback_speed: f64,
    pub waveform: Vec<WaveBin>,
    pub preferences: Preferences,
    pub status: String,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub requests: Vec<PlatformRequest>,
    pub pending_open: Option<PathBuf>,
    edit_dialog: Option<&'static str>,
    quantize_strength: f64,
    pub revert_requested: bool,
    has_original_snapshot: Arc<AtomicBool>,
    pub busy: bool,
    pub song_loading: bool,
    pending_play_on_song: bool,
    song_job: Option<mpsc::Receiver<JobResult>>,
    song_cancel: Arc<AtomicBool>,
    song_generation: u64,
    waveform_revision: u64,
    pending_locator_right: Option<(f64, Option<f64>, bool)>,
    #[cfg(test)]
    song_decode_hold: Option<Arc<AtomicBool>>,
    #[cfg(test)]
    started_play_count: usize,
    pub output_review: Option<output::WritePlan>,
    export: export::Slot,
    /// Counts loads and Saves of the open difficulty, for the Export preview's identity.
    document_generation: u64,
    pending_save_review: Option<PendingSave>,
    output_problems: Vec<OutputProblem>,
    output_problem_revision: Option<(u64, bool)>,
    save_problem_review: bool,
    tools: ToolBox<HitKey>,
    controller_stroke: Option<controller::ControllerStroke>,
    jobs: Option<mpsc::Receiver<JobResult>>,
    /// The running cleanup of unused sample files, which keeps `busy` set until close or open
    /// stops waiting for it.
    sweep: Option<mpsc::Receiver<sweep::Outcome>>,
    /// Set once close or open stopped waiting: the job's document is gone, so what it moved goes back.
    sweep_detached: bool,
    /// How long close and open wait for a cleanup job (Yoge, 2026-11-21: 5 seconds).
    sweep_wait: std::time::Duration,
    job_progress: Option<JobProgress>,
    cancel: Arc<AtomicBool>,
    muted: Vec<SampleIdentity>,
    solo: Vec<SampleIdentity>,
    lane_gains: HashMap<SampleIdentity, f32>,
    song_solo: bool,
    hitsounds_solo: bool,
    cached_version: u64,
    cached_listen_version: u64,
    pub zoom: f64,
    pub row_height: f32,
    project_root: Option<PathBuf>,
    project: Option<project::Project>,
    pub locator_left: f64,
    pub locator_right: f64,
    pub cycle: bool,
    pub song_offset: f64,
    pub active_lane: usize,
    selection_lane_scope: Option<BTreeSet<SampleIdentity>>,
    playback_start: f64,
    return_position: f64,
    last_endpoint_jump: Option<(bool, std::time::Instant)>,
    key_lane_view: Option<(PathBuf, Vec<usize>, f64)>,
    pub first_launch: bool,
    catalogue: Arc<CommandCatalog<Application>>,
    pub keymap: KeyMap,
    pub help_requested: bool,
    /// The bookmark Previous or Next bookmark last jumped to, until the Tauri view takes it.
    pub bookmark_step: Option<f64>,
    persist_preferences: bool,
    data_root: PathBuf,
    checkpoint: Option<recovery::Checkpoint>,
    restored_recovery: Option<String>,
    /// Open-time questions about placements on an older version of a library set (M9).
    sound_set_prompts: Vec<sound_sets::SetUpdatePrompt>,
    /// The open-time check waits for the library index's background read.
    sound_set_check_pending: bool,
    /// Difficulties opened so far: an update question closed without an answer comes back on the
    /// next open.
    opens: u64,
    drum: drum_detect::DrumDetect,
    node_graph: node_graph::NodeGraphState,
    pub midi_recorder: midi_record::MidiRecorder,
    pub keysounds: dawsu_keysounding::KeysoundProject,
    pub vst3: vst3::Vst3Manager,
    pub drum_vst: Option<drum_vst::DrumVstLiveHandle>,
    drum_vst_state_b64: Option<String>,
}
impl Application {
    pub fn new() -> Self {
        Self::with_data_root(data_folder())
    }
    /// The desktop app's start: [`Self::with_data_root`], then the local default sounds when
    /// the preferences name no default sounds folder. Tests that build the app in the same
    /// process start with `with_data_root` alone, so they never see a folder that exists on
    /// one computer only.
    pub fn for_desktop(data_root: PathBuf) -> Self {
        let mut app = Self::with_data_root(data_root);
        app.use_local_default_samples(Path::new(LOCAL_DEFAULT_SAMPLES));
        app
    }
    /// Uses `local` as the default sounds folder when none is saved and the folder exists.
    /// The folder is only read.
    fn use_local_default_samples(&mut self, local: &Path) {
        if self.preferences.default_samples.as_os_str().is_empty() && local.is_dir() {
            self.preferences.default_samples = local.to_path_buf();
        }
    }
    pub fn with_data_root(data_root: PathBuf) -> Self {
        let file = data_root.join("preferences.json");
        let legacy = std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("OsuHitsoundEditor").join("preferences.json"));
        let (mut preferences, first_launch) = preferences::load(&file, legacy.as_deref());
        preferences.master_gain = preferences.master_gain.clamp(0.2, 1.0);
        let output = open_preferred_audio(&preferences, preferences.audio_buffer_ms);
        let (audio, error) = match output {
            Ok(engine) => (Some(engine), None),
            Err(error) => {
                let same_device = preferences.audio_buffer_ms.is_some()
                    .then(|| open_preferred_audio(&preferences, None));
                match same_device {
                    Some(Ok(engine)) => (Some(engine), Some(format!("Using the default audio buffer. {error}"))),
                    _ if preferences.audio_backend != OutputBackend::System
                        || preferences.audio_device_id.is_some() || preferences.device.is_some() => {
                        match AudioEngine::open_output(OutputBackend::System, None, None) {
                            Ok(engine) => (Some(engine), Some(format!("Using the default system output. {error}"))),
                            Err(fallback) => (None, Some(format!("{error} System default: {fallback}"))),
                        }
                    }
                    Some(Err(fallback)) => (None, Some(format!("{error} Device default: {fallback}"))),
                    None => (None, Some(error)),
                }
            }
        };
        if let Some(a) = &audio {
            a.gains(preferences.effective_song_gain(), preferences.master_gain);
            a.hitsound_gain(preferences.effective_hitsound_gain());
        }
        Self::create(preferences, first_launch, audio, error, true, data_root)
    }
    /// An application with no audio and no saved preferences. Its data folder is a temporary
    /// one, so the files it keeps never reach the user's own.
    pub fn headless() -> Self {
        let data_root = std::env::temp_dir().join("dawsu-headless").join(std::process::id().to_string());
        Self::create(Preferences::default(), false, None, None, false, data_root)
    }
    pub fn headless_with_projects(data_root: PathBuf) -> Self {
        let mut app = Self::create(Preferences::default(), false, None, None, false, data_root.clone());
        app.project_root = Some(data_root.join("projects"));
        app
    }
    fn create(
        preferences: Preferences,
        first_launch: bool,
        audio: Option<AudioEngine>,
        error: Option<String>,
        persist_preferences: bool,
        data_root: PathBuf,
    ) -> Self {
        let catalogue = Arc::new(commands::catalogue());
        let keymap = shortcuts::keymap(catalogue.definitions(), &preferences);
        Self {
            document: None,
            sample_watch: sample_watch::Watch::default(),
            audio,
            samples: HashMap::new(),
            song: None,
            playback_speed: 1.0,
            waveform: Vec::new(),
            preferences,
            status: "Open a difficulty to start".into(),
            error,
            warnings: Vec::new(),
            requests: Vec::new(),
            pending_open: None,
            edit_dialog: None,
            quantize_strength: 100.0,
            revert_requested: false,
            has_original_snapshot: Arc::new(AtomicBool::new(false)),
            busy: false,
            song_loading: false,
            pending_play_on_song: false,
            song_job: None,
            song_cancel: Arc::new(AtomicBool::new(false)),
            song_generation: 0,
            waveform_revision: 0,
            pending_locator_right: None,
            #[cfg(test)]
            song_decode_hold: None,
            #[cfg(test)]
            started_play_count: 0,
            output_review: None,
            export: export::Slot::default(),
            document_generation: 0,
            pending_save_review: None,
            output_problems: Vec::new(),
            output_problem_revision: None,
            save_problem_review: false,
            tools: ToolBox::new(),
            controller_stroke: None,
            jobs: None,
            sweep: None,
            sweep_detached: false,
            sweep_wait: std::time::Duration::from_secs(5),
            job_progress: None,
            cancel: Arc::new(AtomicBool::new(false)),
            muted: Vec::new(),
            solo: Vec::new(),
            lane_gains: HashMap::new(),
            song_solo: false,
            hitsounds_solo: false,
            sample_file: None,
            cached_version: 0,
            cached_listen_version: 0,
            zoom: 0.12,
            row_height: 30.0,
            locator_left: 0.0,
            locator_right: 10000.0,
            cycle: false,
            song_offset: DEFAULT_SONG_OFFSET_MS,
            active_lane: 0,
            selection_lane_scope: None,
            playback_start: 0.0,
            return_position: 0.0,
            last_endpoint_jump: None,
            key_lane_view: None,
            first_launch,
            catalogue,
            keymap,
            help_requested: false,
            bookmark_step: None,
            persist_preferences,
            project_root: persist_preferences.then(|| data_root.join("projects")),
            data_root,
            project: None,
            checkpoint: None,
            restored_recovery: None,
            sound_set_prompts: Vec::new(),
            sound_set_check_pending: false,
            opens: 0,
            drum: Default::default(),
            node_graph: Default::default(),
            midi_recorder: midi_record::MidiRecorder::new(),
            keysounds: dawsu_keysounding::KeysoundProject::new("", "Keysounds"),
            vst3: vst3::Vst3Manager::new(),
            drum_vst: None,
            drum_vst_state_b64: None,
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }
    pub fn is_dirty(&self) -> bool {
        self.document.as_ref().is_some_and(|d| d.composer.dirty())
    }
    /// Writes the last completed state, even if the live editor was interrupted mid-edit.
    pub fn write_recovery_copy(&self) -> Result<Option<RecoveryOffer>, String> {
        self.checkpoint
            .as_ref()
            .map(|copy| copy.write(&self.data_root))
            .transpose()
    }
    pub fn recovery_offers(&self) -> Result<Vec<RecoveryOffer>, String> {
        recovery::offers(&self.data_root)
    }
    pub fn restore_recovery(&mut self, id: &str) -> Result<(), String> {
        if self.busy || self.document.is_some() {
            return Err("Restore a recovery copy from the start screen.".into());
        }
        let path = recovery::pending_path(&self.data_root, id)?;
        self.open_now(OpenSource::Recovery(path));
        Ok(())
    }
    fn refresh_recovery_edit(&mut self) {
        if let (Some(copy), Some(document)) = (&mut self.checkpoint, &self.document) {
            copy.refresh(document);
        }
    }
    pub fn is_playing(&self) -> bool {
        self.audio.as_ref().is_some_and(|a| a.is_playing())
    }
    pub fn position_ms(&self) -> f64 {
        self.audio
            .as_ref()
            .map(|a| a.position_ms())
            .unwrap_or(self.playback_start)
    }
    pub fn duration_ms(&self) -> f64 {
        self.song
            .as_ref()
            .map(|s| s.duration_ms())
            .unwrap_or_else(|| {
                self.document
                    .as_ref()
                    .map(|d| d.duration_ms())
                    .unwrap_or(10000.0)
            })
    }
    pub fn device_name(&self) -> &str {
        self.audio
            .as_ref()
            .map(|a| a.device_name.as_str())
            .unwrap_or("Audio unavailable")
    }
    pub fn devices(&self) -> Vec<String> {
        AudioEngine::devices()
    }
    pub fn output_devices(&self, backend: OutputBackend) -> Result<Vec<OutputDevice>, String> {
        if backend == OutputBackend::Asio
            && let Some(audio) = self.audio.as_ref().filter(|audio| audio.backend() == backend)
        {
            // CPAL ASIO enumerates by loading drivers. Keep the live driver intact.
            return Ok(audio.device_id().map(|id| OutputDevice {
                id: id.to_owned(),
                name: audio.device_name.clone(),
            }).into_iter().collect());
        }
        AudioEngine::output_devices(backend)
    }

    pub fn active_output_backend(&self) -> Option<OutputBackend> {
        self.audio.as_ref().map(AudioEngine::backend)
    }

    pub fn effective_output_buffer_ms(&self) -> Option<u32> {
        self.audio.as_ref().and_then(AudioEngine::effective_buffer_ms)
    }

    pub fn select_device(&mut self, name: String) {
        let mut preferences = self.preferences.clone();
        preferences.audio_backend = OutputBackend::System;
        preferences.audio_device_id = None;
        preferences.device = Some(name);
        if let Err(error) = self.configure_audio_output(preferences) {
            self.error = Some(error);
        }
    }

    pub fn set_output_buffer(&mut self, buffer_ms: Option<u32>) -> Result<(), String> {
        let mut preferences = self.preferences.clone();
        preferences.audio_buffer_ms = buffer_ms;
        self.configure_audio_output(preferences)
    }

    pub fn set_output_settings(
        &mut self, backend: OutputBackend, device_id: Option<String>, buffer_ms: Option<u32>,
    ) -> Result<(), String> {
        let mut preferences = self.preferences.clone();
        preferences.audio_backend = backend;
        preferences.audio_device_id = device_id;
        preferences.audio_buffer_ms = buffer_ms;
        if backend == OutputBackend::System {
            preferences.device = None;
        }
        self.configure_audio_output(preferences)
    }

    fn configure_audio_output(&mut self, mut preferences: Preferences) -> Result<(), String> {
        if self.busy || self.is_playing() {
            return Err("Stop playback and finish the current job before changing audio output.".into());
        }
        if preferences.audio_backend == OutputBackend::Asio
            && self.active_output_backend() == Some(OutputBackend::Asio)
        {
            if preferences.audio_device_id == self.preferences.audio_device_id
                && preferences.audio_buffer_ms == self.preferences.audio_buffer_ms
            {
                return Ok(());
            }
            return Err("Select WASAPI before changing the ASIO device or buffer.".into());
        }
        let cursor = self.position_ms();
        let audio = open_preferred_audio(&preferences, preferences.audio_buffer_ms)?;
        if preferences.audio_backend == OutputBackend::System && preferences.audio_device_id.is_some() {
            preferences.device = Some(audio.device_name.clone());
        }
        self.audio = Some(audio);
        self.preferences = preferences;
        self.error = None;
        self.sync_audio();
        self.seek(cursor);
        self.cycle_changed();
        self.gains_changed();
        self.save_preferences();
        Ok(())
    }
    pub fn set_lane_layout(&mut self, mut layout: LaneLayout) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current edit before changing the lane layout".into());
        }
        let document = self.document.as_mut().ok_or("No difficulty is open")?;
        layout.normalise();
        document.lane_layout = Some(layout);
        Ok(())
    }

    /// Counts loads, Saves and reverts of the open difficulty: with the history version it
    /// tells whether anything read from the document is still current.
    pub fn document_generation(&self) -> u64 {
        self.document_generation
    }

    /// Counts background song/waveform completions for the open difficulty.
    pub fn waveform_revision(&self) -> u64 {
        self.waveform_revision
    }

    /// Closes the difficulty: its pending files no record still lists are forgotten and their
    /// names released, so the numbers free up. Call after `save_session`.
    pub fn close_document(&mut self) {
        self.song_cancel.store(true, Ordering::Relaxed);
        self.song_job = None;
        self.song_loading = false;
        self.pending_play_on_song = false;
        self.pending_locator_right = None;
        self.song_generation += 1;
        self.song = None;
        self.waveform.clear();
        self.waveform_revision += 1;
        // A cleanup still running after a Save finishes first. One still running past the wait
        // finishes in the background, and this close cleans up nothing: the next one does.
        self.poll_sweep(true);
        let keep = if self.sweep.is_none() {
            self.start_sweep(true);
            self.poll_sweep(true).flatten()
        } else {
            None
        };
        self.forget_unneeded_pending(keep);
        self.document = None;
        self.checkpoint = None;
    }

    /// Forgets the pending files no record lists. `keep` are the listed names when a job read them.
    fn forget_unneeded_pending(&mut self, keep: Option<BTreeSet<String>>) {
        let project_root = self.project_root.clone();
        let Some(document) = self.document.as_mut() else { return };
        let Some(mapset) = document.path.parent().map(Path::to_path_buf) else { return };
        // A recovery copy that can't be read may list any of them: none is forgotten.
        let Some(keep) = keep.or_else(|| recorded_pending(project_root.as_deref(), &self.data_root, &mapset)) else { return };
        document.pending.forget_all_but(&keep);
    }

    pub fn save_session(&mut self) -> Result<(), String> {
        self.write_session(None, None, None)
    }

    pub fn set_export_settings(&mut self, settings: ExportSettings) -> Result<(), String> {
        if self.busy {
            return Err("Wait for the current job before changing export settings.".into());
        }
        settings.checked_bytes()?;
        let doc = self.document.as_mut().ok_or("Open a difficulty first.")?;
        doc.export_settings = Some(settings);
        self.output_review = None;
        Ok(())
    }

    pub fn set_time_signatures(&mut self, signatures: Vec<EditorTimeSignature>) -> Result<(), String> {
        if self.busy {
            return Err("Wait for the current job before changing time signatures".into());
        }
        if !signatures.iter().all(EditorTimeSignature::valid) {
            return Err("Each time signature needs a bar of 1 or later, 1–32 beats, and a beat unit of 1, 2, 4, 8 or 16.".into());
        }
        let document = self.document.as_mut().ok_or("Open a difficulty first")?;
        document.update_time_signatures(signatures);
        Ok(())
    }

    fn session_settings(
        &self,
        saved_hits: Option<Vec<project::ProjectHit>>,
        saved_slots: Option<std::collections::BTreeMap<String, String>>,
        saved_ownership: Option<Option<project::HsCircleOwnership>>,
    ) -> Option<project::SessionSettings> {
        let document = self.document.as_ref()?;
        Some(project::SessionSettings {
            unanchored: saved_hits.or_else(|| {
                self.project
                    .as_ref()
                    .and_then(|p| p.settings.unanchored.clone())
            }),
            song_offset: self.song_offset,
            locator_left: self.locator_left,
            locator_right: Some(self.locator_right),
            cycle: self.cycle,
            zoom: Some(self.zoom),
            row_height: Some(self.row_height),
            view_start: document.composer.view_start,
            bookmarks: document.bookmark_override.then(|| {
                document.bookmarks.items().iter().map(|mark| project::ProjectBookmark {
                    time: mark.time.ms(),
                    label: mark.label.clone(),
                }).collect()
            }),
            lane_layout: document.lane_layout.clone(),
            hidden_lanes: {
                let items: std::collections::BTreeSet<_> = document.composer.hitsounds()
                    .hidden_lanes().map(project::lane_key).collect();
                (!items.is_empty() || self.project.as_ref().is_some_and(|p| p.settings.hidden_lanes.is_some()))
                    .then_some(items)
            },
            hs_mode: Some(document.hs_mode),
            hs_circle_ownership: saved_ownership.unwrap_or_else(|| {
                self.project.as_ref().and_then(|p| p.settings.hs_circle_ownership.clone())
            }),
            gain_recovery: self.project.as_ref().and_then(|p| p.settings.gain_recovery.clone()),
            unwritten: self.project.as_ref().and_then(|p| p.settings.unwritten.clone()),
            sound_sets: self.project.as_ref().and_then(|p| p.settings.sound_sets.clone()),
            pending_samples: self.project.as_ref().and_then(|p| p.settings.pending_samples.clone()),
            time_signatures: Some(document.time_signatures.clone()),
            export_settings: document.export_settings.clone(),
            drum_lanes: document.drum_lanes.clone(),
            export_choices: document.export_choices.clone(),
            node_graph_bindings: document.node_graph_bindings.clone(),
            lane_slots: saved_slots.or_else(|| self.project.as_ref().and_then(|p| p.settings.lane_slots.clone())),
            drum_vst: self.drum_vst.as_ref().map(|h| project::DrumVstSession {
                plugin_path: h.plugin_path.clone(),
                plugin_uid: h.plugin_uid.clone(),
                enabled: h.is_enabled(),
                state_b64: self.drum_vst_state_b64.clone(),
            }),
        })
    }

    fn write_session(
        &mut self,
        saved_hits: Option<Vec<project::ProjectHit>>,
        saved_slots: Option<std::collections::BTreeMap<String, String>>,
        saved_ownership: Option<Option<project::HsCircleOwnership>>,
    ) -> Result<(), String> {
        let settings = self.session_settings(saved_hits, saved_slots, saved_ownership);
        if let (Some(project), Some(settings)) = (&mut self.project, settings) {
            project.save(settings).map_err(|e| format!("Saving session settings: {e}"))?;
        }
        Ok(())
    }

    pub fn save_preferences(&mut self) {
        if !self.persist_preferences {
            return;
        }
        let root = &self.data_root;
        let result = std::fs::create_dir_all(root)
            .map_err(|e| e.to_string())
            .and_then(|_| serde_json::to_vec_pretty(&self.preferences).map_err(|e| e.to_string()))
            .and_then(|bytes| {
                dawsu_foundation::safe_write::write_new(&root.join("preferences.json"), &bytes)
                    .map_err(|e| e.to_string())
            });
        if let Err(e) = result {
            self.error = Some(e);
        }
    }
    /// Imports selected MIDI tracks into the active document as one undo step.
    pub fn apply_midi_import(
        &mut self,
        options: &MidiImportOptions,
        drum_map: Option<&dawsu_hitsounding::drums::DrumMap>,
    ) -> Result<MidiImportOutcome, String> {
        if self.busy {
            return Err("Wait for the current job to finish before importing MIDI.".into());
        }
        let document = self.document.as_mut().ok_or("Open a difficulty first.")?;
        let outcome = midi_import::apply_midi_import(&mut document.composer, options, drum_map)?;
        self.refresh_recovery_edit();
        self.status = format!(
            "Imported {} MIDI hits ({} skipped).",
            outcome.added, outcome.skipped
        );
        Ok(outcome)
    }

    /// Exports the active document's hitsounds (and any keysound tracks) to a Standard MIDI File.
    pub fn export_midi(&self, path: &Path) -> Result<usize, String> {
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let bpm = document
            .composer
            .tempo
            .sections()
            .first()
            .map(|s| 60_000.0 / s.beat_length_ms)
            .unwrap_or(120.0);
        let ks = (!self.keysounds.document.tracks.is_empty()).then_some(&self.keysounds);
        midi_import::export_midi_file(path, &document.composer, bpm, ks)
    }

    /// Arms or disarms live MIDI recording; disarming commits any captured notes as one undo step.
    pub fn toggle_midi_recording(
        &mut self,
        port_index: Option<usize>,
        drum_map: Option<&dawsu_hitsounding::drums::DrumMap>,
        snap_divisor: Option<u32>,
    ) -> Result<midi_record::MidiRecordStatus, String> {
        if self.midi_recorder.armed {
            let port_name = self.midi_recorder.port_name.clone();
            let notes = self.midi_recorder.disarm();
            let mut committed = 0usize;
            if !notes.is_empty() {
                if let Some(doc) = self.document.as_mut() {
                    committed = midi_record::commit_to_composer(
                        &notes,
                        &mut doc.composer,
                        drum_map,
                        snap_divisor,
                    )?;
                    self.refresh_recovery_edit();
                }
                if let Some(first_track) = self.keysounds.document.tracks.first() {
                    let tid = first_track.id;
                    midi_record::commit_to_keysounds(&notes, &mut self.keysounds, tid, 200.0);
                }
                self.status = format!("Recorded {committed} MIDI hits.");
            } else {
                self.status = "MIDI recording stopped.".into();
            }
            return Ok(midi_record::MidiRecordStatus {
                armed: false,
                port_name,
                buffered_notes: 0,
                committed_notes: committed,
            });
        }
        let port_name = self.midi_recorder.arm(port_index)?;
        self.status = match &port_name {
            Some(name) => format!("MIDI recording armed ({name}). Press Space to play and record."),
            None => "MIDI recording armed (no hardware MIDI port; ready for virtual input).".into(),
        };
        Ok(midi_record::MidiRecordStatus {
            armed: true,
            port_name,
            buffered_notes: 0,
            committed_notes: 0,
        })
    }

    /// Scans the standard system VST3 folders (plus `extra_folders`) for instrument/FX plugins.
    pub fn scan_vst3_plugins(&mut self, extra_folders: &[PathBuf]) -> Result<vst3::Vst3ScanReport, String> {
        if !extra_folders.is_empty() {
            let mut dirs = self.vst3.search_dirs().to_vec();
            dirs.extend(extra_folders.iter().cloned());
            self.vst3.set_search_dirs(dirs);
        }
        self.vst3.scan_plugins().map_err(|e| e.to_string())
    }

    /// Returns a JSON summary of the keysound tracks, notes, and scanned VST3 instruments.
    pub fn keysounds_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "targetDifficulty": self.keysounds.target_difficulty,
            "tracks": self.keysounds.document.tracks.iter().map(|t| serde_json::json!({
                "id": t.id,
                "name": t.name,
                "channel": t.channel,
                "isDrum": t.is_drum,
                "muted": t.muted,
                "noteCount": t.notes.len(),
                "instrument": t.instrument.name,
            })).collect::<Vec<_>>(),
            "vst3Plugins": self.vst3.scanned_plugins().iter().map(|p| serde_json::json!({
                "id": p.uid,
                "name": p.name,
                "vendor": p.vendor,
                "isInstrument": p.is_instrument,
            })).collect::<Vec<_>>(),
        })
    }

    /// Adds a keysound track (optionally naming a scanned VST3 instrument) and returns its ID.
    pub fn keysounds_add_track(&mut self, name: &str, vst3_plugin_name: Option<&str>) -> u64 {
        let ch = (self.keysounds.document.tracks.len() % 15) as u8;
        let id = self.keysounds.document.add_track(name, ch, false);
        if let Some(plugin) = vst3_plugin_name {
            if let Some(track) = self.keysounds.document.tracks.iter_mut().find(|t| t.id == id) {
                track.instrument.name = plugin.to_string();
            }
        }
        id
    }

    /// Executes a Keysounding Piano Roll / VST3 / `[Keysounds]` generation action.
    pub fn keysounds_action(&mut self, payload: &serde_json::Value) -> Result<serde_json::Value, String> {
        keysound_session::handle(self, payload)
    }

    pub fn request_open(&mut self, path: PathBuf) {
        if self.busy {
            return;
        }
        if self.is_dirty() {
            self.pending_open = Some(path);
            return;
        }
        self.open_now(OpenSource::Map(path));
    }
    pub fn confirm_revert(&mut self) {
        if self.busy || !std::mem::take(&mut self.revert_requested) {
            return;
        }
        if let Some(path) = self.document.as_ref().map(|d| d.path.clone()) {
            self.pending_open = None;
            self.start_open(OpenSource::Map(path), false);
        }
    }

    pub fn has_original_snapshot(&self) -> bool {
        self.document.is_some() && self.has_original_snapshot.load(Ordering::Relaxed)
    }

    pub fn revert_mapset_to_original(&mut self) -> Result<usize, String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before reverting the mapset.".into());
        }
        let path = self
            .document
            .as_ref()
            .map(|d| d.path.clone())
            .ok_or("Open a difficulty first.")?;
        let folder = path.parent().ok_or("Missing mapset folder.")?;
        let restored = original_snapshot::restore(&self.data_root, folder)?;
        self.pending_open = None;
        self.revert_requested = false;
        self.start_open(OpenSource::Map(path), false);
        self.status = format!("Reverted {restored} mapset file(s) to original snapshot (pre-revert backup saved).");
        Ok(restored)
    }

    pub fn discard_and_open(&mut self) {
        if self.busy {
            return;
        }
        if let Some(path) = self.pending_open.take() {
            self.open_now(OpenSource::Map(path));
        }
    }
    fn open_now(&mut self, source: OpenSource) {
        self.start_open(source, true);
    }

    fn start_open(&mut self, source: OpenSource, keep_settings: bool) {
        // A cleanup the last Save started finishes before the difficulty changes.
        self.poll_sweep(true);
        self.cancel_timeline_edit();
        if keep_settings && let Err(error) = self.save_session() {
            self.error = Some(error);
            return;
        }
        self.stop();
        self.cancel_controller_stroke();
        self.clear_drum_detection();
        self.clear_node_graph_run();
        if let Some(d) = self.document.as_mut() {
            self.tools.cancel(&mut d.composer);
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.song_cancel.store(true, Ordering::Relaxed);
        self.song_cancel = Arc::new(AtomicBool::new(false));
        self.song_job = None;
        self.song_loading = false;
        self.pending_play_on_song = false;
        self.pending_locator_right = None;
        let cancel = self.cancel.clone();
        let defaults = self.preferences.default_samples.clone();
        let project_root = self.project_root.clone();
        let data_root = self.data_root.clone();
        let imports = self.data_root.join("imports");
        let legacy_root = self
            .persist_preferences
            .then(|| std::env::var_os("LOCALAPPDATA"))
            .flatten()
            .map(|base| {
                PathBuf::from(base)
                    .join("OsuHitsoundEditor")
                    .join("projects")
            });
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        let outgoing = self.document.as_ref().map(|d| d.composer.history.version());
        let outgoing_mapset = self.document.as_ref().and_then(|d| d.path.parent().map(Path::to_path_buf));
        self.status = "Opening map and decoding audio…".into();
        if self.sweep_detached {
            self.status = sweep::with_note(&self.status, sweep::STILL_RUNNING);
        }
        std::thread::spawn(move || {
            let result = (|| {
                let (path, recovery) = match source {
                    OpenSource::Map(path) => (path, None),
                    OpenSource::Recovery(path) => {
                        let copy = recovery::read_copy(&path)?;
                        (copy.source().to_owned(), Some(copy))
                    }
                };
                let path = if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("osz"))
                {
                    std::fs::create_dir_all(&imports).map_err(|e| e.to_string())?;
                    match dawsu_osu_format::import_osz(
                        &path,
                        &imports,
                        Default::default(),
                        &|| cancel.load(Ordering::Relaxed),
                        &mut |_, _| {},
                    )
                    .map_err(|e| e.to_string())?
                    {
                        dawsu_osu_format::OszImport::Imported { folder } => {
                            std::fs::read_dir(folder)
                                .map_err(|e| e.to_string())?
                                .filter_map(Result::ok)
                                .map(|e| e.path())
                                .find(|p| {
                                    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu"))
                                })
                                .ok_or("Archive contains no difficulty")?
                        }
                        other => return Err(refusals::import(&other)),
                    }
                } else {
                    path
                };
                let mut loaded = load(
                    &path,
                    &defaults,
                    &data_root,
                    &cancel,
                    project_root.as_deref(),
                    legacy_root.as_deref(),
                    recovery,
                )?;
                loaded.outgoing_keep = outgoing_mapset.and_then(|mapset| recorded_pending(project_root.as_deref(), &data_root, &mapset));
                Ok(loaded)
            })();
            let _ = send.send(JobResult::Open(outgoing, keep_settings, result));
            // Housekeeping of dawsu!'s data folder, after the open so it never delays it.
            pending::prune_kept_copies(&data_root, project_root.as_deref(), pending::PRUNE_MIN_AGE);
            pending::cap_removed_samples(
                &data_root,
                std::time::Duration::from_secs(pending::REMOVED_SAMPLES_DAYS * 24 * 3600),
                pending::REMOVED_SAMPLES_BYTES,
            );
        });
    }
    pub fn cancel_job(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.song_cancel.store(true, Ordering::Relaxed);
        self.pending_play_on_song = false;
        if !self.busy {
            self.song_loading = false;
            self.song_job = None;
            self.pending_locator_right = None;
        }
        self.status = "Cancelling…".into();
    }
    pub fn collect_retired_audio(&self) -> bool {
        self.audio
            .as_ref()
            .is_some_and(AudioEngine::collect_retired)
    }

    fn track_progress(&mut self, label: &'static str) -> JobProgress {
        let progress = JobProgress { label, current: Arc::new(Mutex::new(None)) };
        self.status = label.into();
        self.job_progress = Some(progress.clone());
        progress
    }

    pub fn poll(&mut self) {
        let stroke_open = self.is_stroke_open();
        if !stroke_open {
            // Edits made outside the refreshing entry points still reach the recovery checkpoint.
            self.refresh_recovery_edit();
        }
        self.poll_sweep(false);
        self.check_set_versions_when_read();
        self.poll_drum_detection();
        self.poll_node_graph();
        let _ = self.midi_recorder.poll(self.position_ms(), self.is_playing());
        // Before main's job: its arms can return early.
        self.poll_export();
        let song_result = match self.song_job.as_ref().map(|r| r.try_recv()) {
            Some(Ok(result)) => {
                self.song_job = None;
                Some(result)
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.song_job = None;
                self.song_loading = false;
                self.pending_play_on_song = false;
                self.pending_locator_right = None;
                None
            }
            _ => None,
        };
        if let Some(result) = song_result {
            self.apply_song_job_result(result);
        }
        if !self.cancel.load(Ordering::Relaxed)
            && let Some(progress) = &self.job_progress
            && let Ok(mut current) = progress.current.try_lock()
            && let Some((done, total)) = current.take()
        {
            self.status = format!("{} {done} / {total} objects", progress.label);
        }
        let result = match self.jobs.as_ref().map(|r| r.try_recv()) {
            Some(Ok(result)) => Some(result),
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.jobs = None;
                self.busy = false;
                self.job_progress = None;
                self.error = Some(
                    "The background job stopped unexpectedly. Your open edits are kept.".into(),
                );
                None
            }
            _ => None,
        };
        if let Some(result) = result {
            self.job_progress = None;
            self.jobs = None;
            self.busy = false;
            match result {
                JobResult::Open(outgoing, keep_settings, Ok(loaded)) => {
                    if self.document.as_ref().map(|d| d.composer.history.version()) != outgoing {
                        self.pending_open = Some(loaded.document.path);
                        self.status =
                            "The outgoing difficulty changed. Review your edits before switching."
                                .into();
                        return;
                    }
                    if keep_settings && let Err(error) = self.save_session() {
                        self.error = Some(error);
                        return;
                    }
                    let path = loaded.document.path.clone();
                    let snap_flag = Arc::new(AtomicBool::new(false));
                    self.has_original_snapshot = snap_flag.clone();
                    if let Some(folder) = path.parent().map(Path::to_path_buf) {
                        let data_root = self.data_root.clone();
                        std::thread::spawn(move || {
                            let _ = original_snapshot::capture_once(&data_root, &folder);
                            if original_snapshot::has_snapshot(&data_root, &folder) {
                                snap_flag.store(true, Ordering::Relaxed);
                            }
                        });
                    }
                    // The outgoing difficulty closes here.
                    let keep = loaded.outgoing_keep;
                    self.forget_unneeded_pending(keep);
                    let mut checkpoint = recovery::Checkpoint::new(&loaded.document);
                    let restored_id = loaded.recovery.map(|copy| copy.resume(&mut checkpoint));
                    self.document = Some(loaded.document);
                    // Reviews of the outgoing load must not match this one: history versions restart.
                    self.document_generation += 1;
                    self.song_generation = self.document_generation;
                    self.waveform_revision += 1;
                    self.opens += 1;
                    self.output_review = None;
                    self.export.clear();
                    self.pending_save_review = None;
                    self.last_endpoint_jump = None;
                    self.selection_lane_scope = None;
                    self.output_problems.clear();
                    self.output_problem_revision = None;
                    self.save_problem_review = false;
                    self.sample_watch = sample_watch::Watch::default();
                    if let Some(d) = self.document.as_mut() {
                        d.composer.protect_normals = self.preferences.protect_hitnormals;
                    }
                    self.song_cancel.store(true, Ordering::Relaxed);
                    self.song_job = None;
                    self.song_loading = false;
                    self.pending_play_on_song = false;
                    self.pending_locator_right = None;
                    self.song = None;
                    self.drum.song_loaded();
                    self.node_graph_song_loaded();
                    self.playback_speed = 1.0;
                    self.samples = loaded.samples;
                    self.waveform.clear();
                    self.warnings = loaded.warnings;
                    self.muted.clear();
                    self.solo.clear();
                    self.song_solo = false;
                    self.hitsounds_solo = false;
                    self.lane_gains.clear();
                    self.active_lane = 0;
                    self.restore_active_lane(None);
                    self.preferences.recent.retain(|p| p != &path);
                    self.preferences.recent.insert(0, path);
                    self.preferences.recent.truncate(20);
                    self.project = loaded.project;
                    let settings = self
                        .project
                        .as_ref()
                        .map(|project| project.settings.clone())
                        .unwrap_or_default();
                    let duration = self.duration_ms();
                    self.locator_left = settings.locator_left.clamp(0.0, duration);
                    self.locator_right = settings
                        .locator_right
                        .unwrap_or(duration)
                        .clamp(0.0, duration);
                    self.cycle = settings.cycle && self.locator_left < self.locator_right;
                    self.song_offset = if self.project.is_some() {
                        settings.song_offset
                    } else {
                        DEFAULT_SONG_OFFSET_MS
                    };
                    self.zoom = settings.zoom.unwrap_or(0.12).clamp(MIN_ZOOM, 3.0);
                    self.row_height = settings.row_height.unwrap_or(30.0).clamp(20.0, 65.0);
                    self.set_view_start(settings.view_start.clamp(0.0, duration));
                    self.seek(0.0);
                    self.cycle_changed();
                    if let Some(saved_vst) = settings.drum_vst.clone() {
                        if let Err(err) = self.restore_drum_vst_session(&saved_vst) {
                            self.status = format!("Difficulty loaded (Drum VST3 unavailable: {err})");
                        }
                    } else if self.drum_vst.is_some() {
                        self.unload_drum_vst();
                    }
                    self.sync_audio();
                    if let Some(song_path) = loaded.song_path {
                        if settings.locator_right.is_none_or(|r| r > duration) {
                            self.pending_locator_right =
                                Some((duration, settings.locator_right, settings.cycle));
                        }
                        self.spawn_song_decode(song_path);
                    }
                    self.status = if restored_id.is_some() {
                        "Recovery copy restored. Save the difficulty to clear the offer.".into()
                    } else {
                        "Difficulty loaded".into()
                    };
                    if self.sweep_detached {
                        self.status = sweep::with_note(&self.status, sweep::STILL_RUNNING);
                    }
                    self.save_preferences();
                    self.checkpoint = Some(checkpoint);
                    self.restored_recovery = restored_id;
                    self.ask_about_set_versions();
                }
                JobResult::SongReady { .. } => {
                    self.apply_song_job_result(result);
                }
                JobResult::Open(_, _, Err(e)) => {
                    self.error = Some(e);
                    self.status = "Open failed; previous document kept".into();
                    if self.sweep_detached {
                        self.status = sweep::with_note(&self.status, sweep::STILL_RUNNING);
                    }
                }
                JobResult::Review(result, diagnostics) => {
                    self.output_problems = diagnostics;
                    self.save_problem_review = false;
                    match result {
                        Ok(plan) => {
                            self.output_review = Some(plan);
                            self.status = "Review the output before writing".into();
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.status = "Output planning stopped; no files written".into();
                        }
                    }
                }
                JobResult::SaveCheck { version, path, hs, result, diagnostics } => {
                    if !self.document.as_ref().is_some_and(|d| {
                        d.path == path && d.composer.history.version() == version && d.hs_mode == hs
                    }) {
                        self.status = "The difficulty changed. Review Save again.".into();
                    } else {
                        self.output_problems = diagnostics;
                        match result {
                            Ok(plan) => {
                                if !plan.plan.left_out.is_empty() {
                                    self.save_problem_review = true;
                                    self.status = "Review the moments Save leaves out".into();
                                } else {
                                    self.status = "Review Save before writing".into();
                                }
                                self.pending_save_review = Some(plan);
                            }
                            Err(_) if !self.output_problems.is_empty() => {
                                self.status = "Review problems before saving".into();
                                self.save_problem_review = true;
                            }
                            Err(error) => {
                                self.error = Some(error);
                                self.status = "Save review did not complete".into();
                            }
                        }
                    }
                }                JobResult::Output(result) => match result {
                    Err(e) if self.export.take_write(&Err(e.clone())) => {
                        self.status = "Output did not complete".into();
                    }
                    Ok((count, tracking_warning, published)) => {
                        self.export.take_write(&Ok(count));
                        self.pending_published(&published, &[]);
                        self.status = format!("Wrote {count} reviewed files with backups");
                        if let Some(warning) = tracking_warning {
                            self.warnings.push(warning);
                            self.status = format!("Wrote {count} reviewed files. Generate replacement could not be recorded.");
                        }
                        self.start_sweep(false);
                        if let Some(d) = self.document.as_mut() {
                            if let Ok(entries) = std::fs::read_dir(d.path.parent().unwrap()) {
                                d.difficulties = entries
                                    .filter_map(Result::ok)
                                    .map(|e| e.path())
                                    .filter(|p| {
                                        p.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu"))
                                    })
                                    .collect();
                            }
                        }
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.status = "Output did not complete".into();
                    }
                },
                JobResult::Save {
                    version,
                    path,
                    hs_mode,
                    settings,
                    project,
                    result,
                    diagnostics,
                } => {
                    self.output_problems = diagnostics;
                    if project.is_some() {
                        self.project = project;
                    }
                    match result {
                        Ok(report) => {
                            let mut archive_error = None;
                            // A recovery copy keeps the whole compile input, loose hits included, as
                            // the state Undo returns to.
                            let compiled = report.written;
                            let drawn = report.drawn;
                            if self.document.as_ref().is_some_and(|d| d.path == path) {
                                self.pending_published(&report.published_pending, &report.pending_kept);
                            }
                            if let Some(d) = self.document.as_mut().filter(|d| d.path == path) {
                                d.file = report.file;
                                self.document_generation += 1;
                                self.export.clear();
                                d.written = last_written(&compiled, hs_mode);
                                d.left_out = report.left_out;
                                // The difficulty is written, even when a step below returns early.
                                self.checkpoint =
                                    Some(recovery::Checkpoint::after_save(d, &compiled, &drawn, hs_mode));
                                if let Some(id) = &self.restored_recovery {
                                    match recovery::archive(&self.data_root, id) {
                                        Ok(()) => self.restored_recovery = None,
                                        Err(error) => archive_error = Some(error),
                                    }
                                }
                            }
                            let with_archive_error = |error: String| match &archive_error {
                                Some(archive) => {
                                    format!("{error}\nCould not archive the recovery copy: {archive}")
                                }
                                None => error,
                            };
                            let current = self.document.as_ref().is_some_and(|d| {
                                d.path == path && d.composer.history.version() == version
                            });
                            if let Some(error) = report.gain_recovery_warning {
                                self.error = Some(with_archive_error(error));
                                self.status = format!("Difficulty saved. {}", report.unfinished_record);
                                return;
                            }
                            if let (Some(mut settings), Some(project)) = (settings, self.project.as_mut()) {
                                settings.hs_circle_ownership = report.hs_circle_ownership;
                                settings.gain_recovery = project.settings.gain_recovery.clone();
                                settings.unwritten = project.settings.unwritten.clone();
                                settings.sound_sets = project.settings.sound_sets.clone();
                                if let Err(error) = project.save(settings) {
                                    self.error = Some(with_archive_error(format!("Saving session settings: {error}")));
                                    self.status = "Difficulty saved. Session settings could not be saved.".into();
                                    return;
                                }
                            }
                            if let Some(error) = &archive_error {
                                self.error = Some(format!(
                                    "Difficulty saved. Could not archive the recovery copy: {error}"
                                ));
                            }
                            // The files dawsu! made that nothing plays now leave the mapset, in a job.
                            self.start_sweep(false);
                            if !current {
                                self.status = "Saved the earlier revision. Newer edits are still open.".into();
                                return;
                            }
                            let document = self.document.as_mut().unwrap();
                            document.composer.mark_saved();
                            self.checkpoint =
                                Some(recovery::Checkpoint::after_save(document, &compiled, &drawn, hs_mode));
                            let left_out = document.left_out.len();
                            self.status = if archive_error.is_some() {
                                "Difficulty saved. Recovery copy is still pending.".into()
                            } else if left_out > 0 {
                                format!("Saved with backup. {left_out} part{} left out.", if left_out == 1 { "" } else { "s" })
                            } else {
                                "Saved with backup".into()
                            };
                            if let Some(path) = self.pending_open.take() {
                                self.open_now(OpenSource::Map(path));
                            }
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.status = "Save did not complete; edits kept".into();
                        }
                    }
                }
            }
        }
        self.poll_sample_folder();
        if !self.is_stroke_open()
            && self.document.as_ref().is_some_and(|d| {
                d.composer.history.version() != self.cached_version
                    || d.composer.listen_version != self.cached_listen_version
            })
        {
            self.sync_audio();
        }
        let requests = self
            .document
            .as_mut()
            .map(|d| std::mem::take(&mut d.composer.audition_requests))
            .unwrap_or_default();
        if self.preferences.audition {
            for lane in requests {
                self.audition(&lane);
            }
        }
    }
    /// The problems of the last check of this revision, then each moment the last Save left out
    /// (F052), which stays listed after other edits.
    pub fn output_problems(&self) -> Vec<OutputProblem> {
        let revision = self
            .document
            .as_ref()
            .map(|doc| (doc.composer.history.version(), doc.hs_mode));
        let mut problems = if revision == self.output_problem_revision {
            self.output_problems.clone()
        } else {
            Vec::new()
        };
        if let Some(doc) = &self.document {
            let hits = doc.composer.hitsounds();
            let checked = problems.len();
            for entry in roll_back::still_held(&doc.left_out, hits, &doc.compile_base, doc.hs_mode) {
                let problem = left_out_problem(entry, &doc.path);
                // A newer check of the same time replaces the entry; two left-out parts can share one.
                if !problems[..checked].iter().any(|p| p.target == problem.target && p.time == problem.time) {
                    problems.push(problem);
                }
            }
        }
        problems
    }

    pub fn save_problem_review(&self) -> bool {
        self.save_problem_review && !self.output_problems().is_empty()
    }

    /// Whether the Save hitsounds problem summary was requested, before checking if any problems remain.
    pub fn save_problem_review_requested(&self) -> bool {
        self.save_problem_review
    }

    /// True while a reviewed Save waits for the mapper's answer.
    pub fn save_review_pending(&self) -> bool {
        self.pending_save_review.is_some()
    }

    /// Cancel or Show in the Save hitsounds summary: the reviewed plan is dropped with it.
    pub fn dismiss_save_problem_review(&mut self) {
        self.save_problem_review = false;
        self.pending_save_review = None;
    }

    pub fn review_save(&mut self) {
        self.begin_save(self.data_root.join("backups"), true);
    }

    pub fn save_review_files(&self) -> Option<Vec<(&'static str, &Path)>> {
        let review = self.pending_save_review.as_ref()?;
        let plan = &review.plan;
        let mut files = Vec::new();
        if plan.map_changed {
            files.push(("Replace difficulty", plan.path.as_path()));
            files.push(("Backup folder", plan.backups.as_path()));
        }
        for mix in &plan.mix_files.staged {
            files.push((if mix.is_reused() { "Reuse WAV" } else { "Create WAV" }, mix.path.as_path()));
        }
        for file in plan.pending_writes.iter().filter(|file| !file.is_reused()) {
            files.push(("Add sample file", file.path.as_path()));
        }
        if let (Some(project), Some(settings)) = (&review.project, &review.settings)
            && (plan.recovery_revision.is_some() || plan.unwritten.is_some() || plan.sound_sets.is_some() || *settings != project.settings)
        {
            files.push(("Write editor project", project.path()));
        }
        Some(files)
    }

    pub fn answer_save_review(&mut self, apply: bool) {
        if self.busy {
            return;
        }
        let Some(review) = self.pending_save_review.take() else {
            return;
        };
        self.save_problem_review = false;
        if !apply {
            self.status = "Save review cancelled".into();
            return;
        }
        if self.is_stroke_open() || !self.document.as_ref().is_some_and(|doc| {
            doc.path == review.plan.path && doc.composer.history.version() == review.version
                && doc.hs_mode == review.hs
        }) {
            self.error = Some("The difficulty changed. Review Save again.".into());
            return;
        }
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Writing reviewed Save…".into();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        std::thread::spawn(move || {
            let _ = send.send(review.finish(&|| cancel.load(Ordering::Relaxed)));
        });
    }

    pub fn save(&mut self) {
        self.save_with_backups(self.data_root.join("backups"));
    }

    fn save_with_backups(&mut self, backups: PathBuf) {
        self.begin_save(backups, false);
    }

    fn begin_save(&mut self, backups: PathBuf, review: bool) {
        if self.busy {
            return;
        }
        if self.is_stroke_open() {
            self.error = Some("Finish or cancel the current drag before saving.".into());
            return;
        }
        let Some(d) = &self.document else {
            return;
        };
        let file = d.compile_base.clone();
        let expected = d.file.original_bytes().to_vec();
        let hits = d.composer.hitsounds().clone();
        let written = d.written.clone();
        let path = d.path.clone();
        let version = d.composer.history.version();
        let hs = d.hs_mode;
        let owned = d.owned_hs_circles.clone();
        let project = self.project.clone();
        let settings = project.as_ref().and_then(|_| {
            let saved_hits = if hs { Vec::new() } else { project_hits(d) };
            let saved_slots = hits.lane_slots()
                .map(|(source, slot)| (project::lane_key(source), project::lane_key(slot))).collect();
            self.session_settings(Some(saved_hits), Some(saved_slots), None)
        });
        let defaults = self.preferences.default_samples.clone();
        let pending = d.pending.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.pending_save_review = None;
        self.save_problem_review = false;
        self.output_problems.clear();
        self.output_problem_revision = Some((version, hs));
        let label = if review { "Preparing Save review…" } else { "Compiling and saving…" };
        let progress = self.track_progress(label);
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        std::thread::spawn(move || {
            let cancelled = || cancel.load(Ordering::Relaxed);
            let mut diagnostics = Vec::new();
            let result = prepare_save(
                file, &hits, &written, expected, &path, backups, hs, &owned, &defaults, &pending,
                project.as_ref(), review, &cancelled, &mut diagnostics,
                &mut |done, total| progress.report(done, total),
            ).map(|plan| {
                let settings = settings.map(|mut settings| {
                    settings.hs_circle_ownership = plan.ownership.clone();
                    // In HS mode every written loose hit is a circle; a group left out before it
                    // ever was one stays a loose hit in the project.
                    if hs {
                        settings.unanchored = Some(loose_rows(&plan.loose));
                    }
                    settings.pending_samples = (!plan.pending_kept.is_empty()).then(|| plan.pending_kept.clone());
                    settings
                });
                PendingSave { version, hs, plan, project: project.clone(), settings }
            });
            let job = if review {
                JobResult::SaveCheck { version, path, hs, result, diagnostics }
            } else {
                match result {
                    // Every Save that leaves moments out shows the Save hitsounds summary first.
                    Ok(plan) if !plan.plan.left_out.is_empty() => {
                        JobResult::SaveCheck { version, path, hs, result: Ok(plan), diagnostics }
                    }
                    Ok(plan) => plan.finish(&cancelled),
                    Err(error) => JobResult::Save {
                        version, path, hs_mode: hs, project, settings: None, result: Err(error), diagnostics,
                    },
                }
            };
            let _ = send.send(job);
        });
    }
    fn spawn_song_decode(&mut self, song_path: PathBuf) {
        self.song_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.song_cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.song_job = Some(receive);
        self.song_loading = true;
        let generation = self.song_generation;
        #[cfg(test)]
        let hold = self.song_decode_hold.clone();
        std::thread::spawn(move || {
            #[cfg(test)]
            if let Some(hold) = hold {
                while hold.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            if cancel.load(Ordering::Relaxed) {
                let _ = send.send(JobResult::SongReady {
                    generation,
                    song: None,
                    waveform: Vec::new(),
                    warning: None,
                });
                return;
            }
            let (song, waveform, warning) =
                match dawsu_audio::decode_with_cancel(&song_path, &|| cancel.load(Ordering::Relaxed)) {
                    Ok(audio) => {
                        let wf = if cancel.load(Ordering::Relaxed) {
                            Vec::new()
                        } else {
                            waveform::analyse(&audio)
                        };
                        (Some(audio), wf, None)
                    }
                    Err(e) => {
                        if cancel.load(Ordering::Relaxed) {
                            (None, Vec::new(), None)
                        } else {
                            (None, Vec::new(), Some(format!("Song audio unavailable: {e}")))
                        }
                    }
                };
            let _ = send.send(JobResult::SongReady {
                generation,
                song,
                waveform,
                warning,
            });
        });
    }

    fn apply_song_job_result(&mut self, result: JobResult) {
        let JobResult::SongReady { generation, song, waveform, warning } = result else {
            return;
        };
        if generation != self.song_generation || self.document.is_none() {
            return;
        }
        self.song = song;
        self.waveform = waveform;
        self.song_loading = false;
        self.waveform_revision += 1;
        if let Some(w) = warning {
            self.warnings.push(w);
        }
        if let Some((pre_duration, saved_right, saved_cycle)) = self.pending_locator_right.take()
            && (self.locator_right - pre_duration).abs() < 1e-6
        {
            let duration = self.duration_ms();
            self.locator_right = saved_right.unwrap_or(duration).clamp(0.0, duration);
            self.cycle = saved_cycle && self.locator_left < self.locator_right;
            self.cycle_changed();
        }
        self.sync_audio();
        if std::mem::take(&mut self.pending_play_on_song) {
            let start = self.playback_start;
            self.seek(start);
            self.return_position = self.playback_start;
            if let Some(a) = &self.audio {
                a.play(true);
            }
            #[cfg(test)]
            {
                self.started_play_count += 1;
            }
            if self.status == "Loading song…" {
                self.status = "Difficulty loaded".into();
            }
        } else if self.status == "Loading song…" {
            self.status = "Difficulty loaded".into();
        }
    }

    pub fn play_pause(&mut self) {
        self.play_pause_from(None);
    }
    pub fn play_pause_from(&mut self, start: Option<f64>) {
        if self.document.is_none() {
            return;
        }
        if self.is_playing() {
            self.stop();
            return;
        }
        if self.song_loading {
            if self.pending_play_on_song && start.is_none() {
                self.pending_play_on_song = false;
                self.status = "Loading song…".into();
                return;
            }
            if let Some(start) = start {
                self.seek(start);
            } else {
                self.playback_start = self.position_ms();
            }
            self.return_position = self.playback_start;
            self.pending_play_on_song = true;
            self.status = "Loading song…".into();
            return;
        }
        if let Some(start) = start {
            self.seek(start);
        } else {
            self.playback_start = self.position_ms();
        }
        self.return_position = self.playback_start;
        if let Some(a) = &self.audio {
            a.play(true);
        }
        #[cfg(test)]
        {
            self.started_play_count += 1;
        }
    }
    /// Plays from `start` whether or not it is already playing (play_pause_from toggles).
    pub fn play_from(&mut self, start: f64) {
        if self.document.is_none() {
            return;
        }
        self.seek(start);
        self.return_position = self.playback_start;
        if self.song_loading {
            self.pending_play_on_song = true;
            self.status = "Loading song…".into();
            return;
        }
        if let Some(a) = &self.audio {
            a.play(true);
        }
        #[cfg(test)]
        {
            self.started_play_count += 1;
        }
    }
    pub fn stop(&mut self) {
        self.pending_play_on_song = false;
        if let Some(a) = &self.audio {
            a.play(false);
            if self.preferences.return_on_stop {
                self.playback_start = self.return_position;
                a.seek(self.return_position);
            }
        }
    }
    pub fn seek(&mut self, ms: f64) {
        let ms = ms.clamp(0.0, self.duration_ms());
        self.playback_start = ms;
        if let Some(a) = &self.audio {
            a.seek(ms);
        }
    }
    fn playback_song_gain(&self) -> f32 {
        if (!self.solo.is_empty() || self.hitsounds_solo) && !self.song_solo {
            0.0
        } else {
            self.preferences.effective_song_gain()
        }
    }

    pub fn playback_hitsound_gain(&self) -> f32 {
        if self.song_solo && !self.hitsounds_solo && self.solo.is_empty() {
            0.0
        } else {
            self.preferences.effective_hitsound_gain()
        }
    }

    pub fn gains_changed(&mut self) {
        self.preferences.master_gain = self.preferences.master_gain.clamp(0.2, 1.0);
        self.preferences.hitsound_gain = self.preferences.hitsound_gain.clamp(0.0, 2.0);
        if let Some(a) = &self.audio {
            a.gains(self.playback_song_gain(), self.preferences.master_gain);
            a.hitsound_gain(self.playback_hitsound_gain());
        }
    }
    pub fn cycle_changed(&mut self) {
        if let Some(a) = &self.audio {
            a.cycle(
                self.cycle
                    .then_some((self.locator_left, self.locator_right)),
            );
            a.offset(self.song_offset);
        }
    }
    pub fn lane_clip(&self, lane: &SampleIdentity) -> Option<Arc<AudioClip>> {
        self.clip_for_lane(lane).cloned()
    }
    fn clip_for_lane(&self, lane: &SampleIdentity) -> Option<&Arc<AudioClip>> {
        let model = self.document()?.composer.hitsounds().lane(lane)?;
        self.samples.get(&model.plays_as())
    }
    pub fn audition(&self, lane: &SampleIdentity) {
        self.audition_lane(lane, 1.0);
    }
    fn audition_lane(&self, lane: &SampleIdentity, gain: f32) {
        if let Some(vst) = &self.drum_vst
            && let Some(note) = vst.routed_note_for(lane)
        {
            let audible = !self.muted.contains(lane)
                && (self.solo.is_empty() || self.solo.contains(lane));
            if audible {
                vst.preview(note, drum_vst::gain_to_midi_velocity(gain * self.lane_gain(lane)));
            }
            return;
        }
        if let (Some(a), Some(clip)) = (&self.audio, self.clip_for_lane(lane)) {
            a.audition(clip.clone(), gain * self.lane_gain(lane));
        }
    }
    pub fn audition_sample_file(&mut self, source: &Path) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before previewing a sample.".into());
        }
        let clip = self.sample_file_clip(source)?;
        if let Some(audio) = &self.audio {
            audio.audition(clip, 1.0);
        }
        Ok(())
    }
    /// The length in ms and the outline (`AudioClip::peaks`) of the sample a lane plays, once loaded.
    pub fn lane_sample_outline(&self, lane: &SampleIdentity, bars: usize) -> Option<(f64, Vec<f32>)> {
        self.clip_for_lane(lane).map(|clip| (clip.duration_ms(), clip.peaks(bars)))
    }
    /// The same for a sample file on disk.
    pub fn sample_file_outline(&mut self, source: &Path, bars: usize) -> Result<(f64, Vec<f32>), String> {
        let clip = self.sample_file_clip(source)?;
        Ok((clip.duration_ms(), clip.peaks(bars)))
    }
    /// A sample file's audio, decoded again only when the file changed on disk since the last call.
    fn sample_file_clip(&mut self, source: &Path) -> Result<Arc<AudioClip>, String> {
        let stamp = sample_watch::file_stamp(source).ok_or_else(|| format!("Reading {}", source.display()))?;
        if let Some((path, kept, clip)) = &self.sample_file
            && path == source && *kept == stamp
        {
            return Ok(clip.clone());
        }
        let clip = dawsu_audio::decode(source)?;
        self.sample_file = Some((source.to_path_buf(), stamp, clip.clone()));
        Ok(clip)
    }
    pub fn is_muted(&self, lane: &SampleIdentity) -> bool {
        self.muted.contains(lane)
    }
    pub fn is_solo(&self, lane: &SampleIdentity) -> bool {
        self.solo.contains(lane)
    }
    pub fn mute(&mut self, lane: SampleIdentity) {
        toggle(&mut self.muted, lane);
        self.sync_audio();
    }
    pub fn solo(&mut self, lane: SampleIdentity) {
        toggle(&mut self.solo, lane);
        self.sync_audio();
    }
    pub fn song_solo(&self) -> bool {
        self.song_solo
    }

    pub fn set_song_solo(&mut self, on: bool) {
        if self.document.is_some() && self.song_solo != on {
            self.song_solo = on;
            self.sync_audio();
        }
    }

    pub fn hitsounds_solo(&self) -> bool {
        self.hitsounds_solo
    }

    pub fn audio_peaks(&self) -> (f32, f32, f32) {
        self.audio.as_ref().map_or((0.0, 0.0, 0.0), |a| a.peaks())
    }
    pub fn set_hitsounds_solo(&mut self, on: bool) {
        if self.document.is_some() && self.hitsounds_solo != on {
            self.hitsounds_solo = on;
            self.sync_audio();
        }
    }

    pub fn lane_gain(&self, lane: &SampleIdentity) -> f32 {
        self.lane_gains.get(lane).copied().unwrap_or(1.0)
    }

    pub fn set_lane_gain(&mut self, lane: SampleIdentity, gain: f32) {
        if !gain.is_finite() {
            return;
        }
        let clamped = gain.clamp(0.0, 2.0);
        if (clamped - 1.0).abs() < 1e-4 {
            self.lane_gains.remove(&lane);
        } else {
            self.lane_gains.insert(lane, clamped);
        }
        self.sync_audio();
    }

    pub fn set_speed(&mut self, speed: f64) {
        if self.busy || self.document.is_none() {
            return;
        }
        if !speed.is_finite() || !(0.25..=1.5).contains(&speed) {
            self.error = Some("Playback speed must be between 25% and 150%".into());
            return;
        }
        let speed = if (speed - 1.0).abs() < 0.001 { 1.0 } else { speed };
        self.playback_speed = speed;
        self.sync_audio();
        self.status = format!("Playback speed: {:.0}%", speed * 100.0);
    }
    pub fn grid_seek(&mut self, direction: i32) {
        self.grid_seek_steps(direction, 1);
    }
    fn grid_seek_steps(&mut self, direction: i32, count: u8) {
        let time = SongTime::from_ms(self.position_ms());
        let playing = self.is_playing();
        if let Some(d) = self.document.as_ref() {
            let composer = &d.composer;
            let target_ms = if playing {
                // Match osu!lazer (Editor.cs:1329 + EditorClock.cs:116-136): unsnapped continuous
                // seek while playing, scaled by BPM and running multiplier.
                let section = composer.tempo.section_at(time);
                let beat_len = if section.beat_length_ms > 0.0 {
                    section.beat_length_ms
                } else {
                    500.0
                };
                let bpm = 60_000.0 / beat_len;
                let running_mult = 1.0 + (250.0 / beat_len).floor();
                let delta = beat_len * f64::from(count) * (bpm / 120.0) * running_mult;
                time.ms() + delta * f64::from(direction.signum())
            } else {
                (0..count)
                    .fold(time, |at, _| {
                        composer.tempo.grid_step(at, direction, composer.spacing)
                    })
                    .ms()
            };
            self.seek(target_ms);
        }
    }
    pub fn set_tool(&mut self, tool: ToolKind) {
        self.cancel_controller_stroke();
        if let Some(d) = self.document.as_mut() {
            self.tools.select_tool(tool, &mut d.composer);
        }
        self.refresh_recovery_edit();
    }
    pub fn tool(&self) -> ToolKind {
        self.tools.active()
    }
    pub fn ramp_shape(&self) -> RampShape {
        self.tools.ramp_shape
    }
    pub fn set_ramp_shape(&mut self, shape: RampShape) {
        if self.document.is_some() && !self.busy && !self.is_stroke_open() {
            self.tools.ramp_shape = shape;
        }
    }
    fn is_stroke_open(&self) -> bool {
        self.controller_stroke.is_some() || self.tools.is_stroke_open()
    }
    fn cancel_controller_stroke(&mut self) {
        if self.controller_stroke.take().is_some() && let Some(d) = self.document.as_mut() {
            d.composer.history.cancel();
        }
    }
    pub fn pointer(&mut self, phase: PointerPhase, input: PointerInput) {
        self.pointer_input(phase, input);
        self.refresh_recovery_edit();
    }
    fn pointer_input(&mut self, phase: PointerPhase, input: PointerInput) {
        if self.busy {
            return;
        }
        if input.click_count >= 2 {
            if matches!(phase, PointerPhase::Down)
                && input.button == PointerButton::Left
                && input.surface == PointerSurface::Grid
                && input.mods == Mods::NONE
                && self.tools.active() == ToolKind::Select
                && !self.is_stroke_open()
            {
                self.select_double_click(input);
            }
            return;
        }
        let Some(d) = self.document.as_mut() else { return };
        if let Some(stroke) = &mut self.controller_stroke {
            match phase {
                PointerPhase::Down => {}
                PointerPhase::Move => stroke.apply(&mut d.composer, input),
                PointerPhase::Up => {
                    stroke.finish(&mut d.composer, input);
                    self.controller_stroke = None;
                }
                PointerPhase::Cancel => self.cancel_controller_stroke(),
            }
            return;
        }
        if matches!(phase, PointerPhase::Down) {
            if input.surface == PointerSurface::Grid {
                self.active_lane = input.row.max(0) as usize;
            } else if !input.mods.shift() && self.tools.active() != ToolKind::Ramp {
                if !self.tools.is_stroke_open() {
                    self.controller_stroke = controller::ControllerStroke::start(&mut d.composer, input, self.zoom);
                }
                return;
            }
        }
        match phase {
            PointerPhase::Down => self.tools.down(&mut d.composer, input),
            PointerPhase::Move => self.tools.moved(&mut d.composer, input),
            PointerPhase::Up => self.tools.up(&mut d.composer, input),
            PointerPhase::Cancel => self.tools.cancel(&mut d.composer),
        }
    }
    fn select_double_click(&mut self, input: PointerInput) {
        let Some(document) = self.document.as_mut() else { return };
        let composer = &mut document.composer;
        let Ok(row) = usize::try_from(input.row) else { return };
        if row >= composer.lanes.len() {
            return;
        }
        self.active_lane = row;
        if let Some(hit) = composer.hit_test(&input) {
            let moment = &composer.hitsounds().moments()[hit.moment];
            let time = moment.time().ms();
            let gain = moment.hit_volume(&hit.lane) as f32 / 100.0;
            let lane = hit.lane.clone();
            composer.selected.replace([hit]);
            self.seek(time);
            self.audition_lane(&lane, gain);
        } else {
            self.tools.down(composer, input);
            self.tools.up(composer, input);
        }
    }

    pub fn go_to_history(&mut self, position: usize) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        let slot = self.active_lane_slot();
        if let Some(document) = self.document.as_mut() {
            document.composer.history.go_to(position);
            document.composer.selected.clear();
        }
        self.restore_active_lane(slot);
        self.refresh_recovery_edit();
    }

    fn active_lane_slot(&self) -> Option<SampleIdentity> {
        let composer = &self.document()?.composer;
        let lane = composer.lanes.get(self.active_lane)?;
        Some(composer.hitsounds().lane_slot(lane).unwrap_or(lane).clone())
    }

    fn restore_active_lane(&mut self, slot: Option<SampleIdentity>) {
        let Some(document) = &self.document else { return };
        let composer = &document.composer;
        let hits = composer.hitsounds();
        let visible = |lane: &SampleIdentity| {
            !hits.is_lane_hidden(lane) && !hits.lane_slots().any(|(_, logical)| logical == lane)
        };
        let wanted = slot.as_ref().map(|slot| {
            hits.lane_slots().find_map(|(source, logical)| (logical == slot).then_some(source)).unwrap_or(slot)
        });
        self.active_lane = wanted.and_then(|wanted| {
            composer.lanes.iter().position(|lane| lane == wanted && visible(lane))
        }).or_else(|| {
            composer.lanes.iter().enumerate().filter(|(_, lane)| visible(lane))
                .min_by_key(|(row, _)| row.abs_diff(self.active_lane)).map(|(row, _)| row)
        }).unwrap_or(composer.lanes.len());
    }

    pub fn set_view_start(&mut self, ms: f64) {
        if let Some(d) = self.document.as_mut() {
            if ms.is_finite() {
                d.composer.view_start = ms;
            }
        }
    }
    pub fn set_snap(&mut self, enabled: bool, division: u32) {
        if let Some(d) = self.document.as_mut() {
            d.composer.snap_enabled = enabled;
            d.composer.spacing = SnapSpacing::fraction(division.max(1)).unwrap();
        }
    }
    pub fn set_normal_protection(&mut self, on: bool) {
        self.preferences.protect_hitnormals = on;
        if let Some(d) = self.document.as_mut() {
            d.composer.protect_normals = on;
        }
        self.save_preferences();
    }
    pub fn set_snap_options(
        &mut self,
        enabled: bool,
        beats: u32,
        divisions: u32,
        mode: dawsu_editor::tempo::SnapMode,
    ) {
        if let Some(d) = self.document.as_mut() {
            match SnapSpacing::new(beats, divisions) {
                Ok(spacing) => {
                    d.composer.snap_enabled = enabled;
                    d.composer.spacing = spacing;
                    d.composer.snap_mode = mode;
                }
                Err(error) => self.error = Some(refusals::snap(beats, divisions, error)),
            }
        }
    }
    pub fn set_insert_volume(&mut self, v: i32) {
        if let Some(d) = self.document.as_mut() {
            // New hits are never drawn under 5% (see `Composer::volume_for_lanes`).
            d.composer.insert_volume = v.clamp(dawsu_hitsounding::rules::SAVED_VOLUME_FLOOR, 100);
        }
    }
    pub fn set_selection_lane_scope(&mut self, lanes: Option<BTreeSet<SampleIdentity>>) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current edit before changing volume scope".into());
        }
        self.document.as_ref().ok_or("Open a difficulty first")?;
        self.selection_lane_scope = lanes;
        Ok(())
    }

    pub fn selection_volume(&mut self, v: i32, relative: bool) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        if let Some(d) = self.document.as_mut() {
            d.composer.volume_for_lanes(v, relative, self.selection_lane_scope.as_ref());
        }
        self.refresh_recovery_edit();
    }
    pub fn set_hs_mode(&mut self, hs: bool) {
        if self.busy {
            return;
        }
        if let Some(d) = self.document.as_mut() {
            d.hs_mode = hs;
        }
        self.refresh_recovery_edit();
    }
    /// Selects the hits attached to one waveform object or one of its nodes.
    pub fn select_waveform_object(&mut self, map: &Path, object: usize, time: f64, node: bool, preview: bool) {
        if self.busy || self.is_stroke_open() || !time.is_finite() {
            return;
        }
        let Some(document) = self.document.as_mut().filter(|document| document.path == map) else {
            return;
        };
        let Some(object_line) = document.file.hit_objects().get(object).map(|object| object.line()) else {
            return;
        };
        let composer = &mut document.composer;
        let mut hits = Vec::new();
        let mut found = false;
        for (index, moment) in composer.hitsounds().moments().iter().enumerate() {
            let object = match moment.place() {
                dawsu_hitsounding::Place::Circle(object)
                | dawsu_hitsounding::Place::SliderNode { object, .. }
                | dawsu_hitsounding::Place::SpinnerEnd(object) => object,
                dawsu_hitsounding::Place::NoObject => continue,
            };
            if object.line() != object_line || (node && moment.time().ms() != time) {
                continue;
            }
            found = true;
            hits.extend(moment.hits().map(|lane| {
                let gain = moment.hit_volume(lane) as f32 / 100.0;
                (HitKey { moment: index, lane: lane.clone() }, gain)
            }));
            if node {
                break;
            }
        }
        if !found {
            return;
        }
        composer.selected.replace(hits.iter().map(|(hit, _)| hit.clone()));
        self.seek(time);
        if preview {
            for (hit, gain) in hits {
                self.audition_lane(&hit.lane, gain);
            }
        }
    }

    pub fn select_hit(&mut self, key: HitKey, add: bool) {
        if let Some(d) = self.document.as_mut() {
            if add {
                d.composer.selected.add(key);
            } else {
                d.composer.selected.replace([key]);
            }
        }
    }
    fn review_job(
        &mut self,
        prepare: impl FnOnce(output::Source, Arc<AtomicBool>, &mut Vec<OutputProblem>, &mut dyn FnMut(usize, usize)) -> Result<output::WritePlan, String>
        + Send
        + 'static,
    ) {
        if self.busy {
            return;
        }
        if self.is_stroke_open() {
            self.error = Some("Finish or cancel the current drag before preparing output.".into());
            return;
        }
        let Some(d) = self.document.as_ref() else {
            return;
        };
        let source = output::Source::from_document(d);
        self.output_problems.clear();
        self.output_problem_revision = Some((source.version, d.hs_mode));
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        let progress = self.track_progress("Preparing output review…");
        std::thread::spawn(move || {
            let mut diagnostics = Vec::new();
            let result = prepare(source, cancel, &mut diagnostics, &mut |done, total| {
                progress.report(done, total);
            });
            let _ = send.send(JobResult::Review(result, diagnostics));
        });
    }
    pub fn generate(&mut self) {
        self.generate_with_slider_audio(false);
    }

    pub fn generate_with_slider_audio(&mut self, include_slider_audio: bool) {
        let defaults = self.preferences.default_samples.clone();
        let settings = self.document.as_ref().and_then(|document| document.export_settings.clone());
        let projects = self.project_root.clone();
        self.review_job(move |source, cancel, diagnostics, progress| {
            output::generate_report(&source, &defaults, settings.as_ref(), projects.as_deref(),
                include_slider_audio, &|| cancel.load(Ordering::Relaxed), diagnostics, progress)
        });
    }
    pub fn new_diff(&mut self, name: String) {
        self.review_job(move |source, _, _, _| output::new_diff(&source, &name));
    }
    pub fn copy_hitsounds(
        &mut self,
        targets: Vec<PathBuf>,
        options: dawsu_hitsounding::copy::CopyOptions,
    ) {
        let defaults = self.preferences.default_samples.clone();
        self.review_job(move |source, cancel, diagnostics, progress| {
            output::copy_report(&source, &defaults, &targets, &options, &|| cancel.load(Ordering::Relaxed), diagnostics, progress)
        });
    }
    pub fn audition_output_mix(&self, index: usize) -> Result<(), String> {
        let clip = self.output_review.as_ref()
            .and_then(|plan| plan.mix_clip(index))
            .ok_or("This sample mix is no longer in the review.")?;
        let audio = self.audio.as_ref().ok_or("Audio output is unavailable.")?;
        audio.audition(Arc::new(clip), 1.0);
        Ok(())
    }

    pub fn apply_output(&mut self) {
        self.apply_output_in(self.data_root.join("backups"));
    }

    fn apply_output_in(&mut self, backups: PathBuf) {
        if self.busy {
            return;
        }
        let Some(plan) = self.output_review.take() else {
            return;
        };
        if !self
            .document
            .as_ref()
            .is_some_and(|d| d.path == plan.source && d.composer.history.version() == plan.version)
        {
            self.error =
                Some("The source changed since this review. Prepare the output again.".into());
            return;
        }
        self.write_reviewed(plan, backups);
    }

    fn write_reviewed(&mut self, plan: output::WritePlan, backups: PathBuf) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Writing reviewed output…".into();
        let project_root = self.project_root.clone();
        std::thread::spawn(move || {
            let result = plan.apply(&backups, &|| cancel.load(Ordering::Relaxed))
                .map(|count| {
                    if let Some(root) = project_root.as_deref() {
                        for path in &plan.hs_mode_targets {
                            if let Ok(mut proj) = project::Project::load(root, None, path)
                                && proj.settings.hs_mode.is_none()
                            {
                                let mut next = proj.settings.clone();
                                next.hs_mode = Some(true);
                                let _ = proj.save(next);
                            }
                        }
                    }
                    (count, generate_receipt::record(&plan).err(), plan.pending_names())
                });
            let _ = send.send(JobResult::Output(result));
        });
    }
    pub fn command(&mut self, id: &str) {
        let catalogue = self.catalogue.clone();
        if let RunOutcome::Failed { message, .. } = catalogue.run(self, id, EditorScope::Hitsound) {
            self.error = Some(message);
        }
        self.refresh_recovery_edit();
    }
    pub fn key(&mut self, chord: KeyChord) {
        let catalogue = self.catalogue.clone();
        let keymap = self.keymap.clone();
        if let KeyOutcome::Command {
            outcome: RunOutcome::Failed { message, .. },
            ..
        } = catalogue.handle_key(self, &keymap, chord, EditorScope::Hitsound)
        {
            self.error = Some(message);
        }
        self.refresh_recovery_edit();
    }
    pub fn key_with_lane_view(&mut self, chord: KeyChord, path: PathBuf, rows: Vec<usize>, span: f64) {
        self.key_lane_view = Some((path, rows, span));
        self.key(chord);
        self.key_lane_view = None;
    }

    pub fn help(&self) -> Vec<HelpGroup> {
        self.catalogue.help(&self.keymap)
    }
    pub fn choose_key_preset(&mut self, preset: &str) {
        self.preferences.key_preset = preset.into();
        self.keymap = shortcuts::keymap(self.catalogue.definitions(), &self.preferences);
        self.first_launch = false;
        self.save_preferences();
    }
    /// Replaces the active lane's editable hits, retaining its logical slot.
    pub fn replace_lane_sample(
        &mut self,
        expected_map: &Path,
        expected_lane: &SampleIdentity,
        source: &Path,
    ) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before replacing a sample.".into());
        }
        let d = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let current = d.composer.lanes.get(self.active_lane);
        if d.path != expected_map || current != Some(expected_lane) {
            return Err("The active lane changed while choosing a sample.".into());
        }
        if matches!(expected_lane, SampleIdentity::Standard { sound: Sound::Slider(_), .. }) {
            return Err("Slider body samples are read-only here.".into());
        }

        let root = d.path.parent().ok_or("Missing mapset folder")?;
        let source = source.canonicalize().map_err(|e| format!("Opening {}: {e}", source.display()))?;
        let extension = source.extension().and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase).filter(|ext| matches!(ext.as_str(), "wav" | "ogg" | "mp3"))
            .ok_or("Choose a WAV, OGG, or MP3 sample.")?;
        let hits = d.composer.hitsounds();
        let logical = hits.lane_slot(expected_lane).unwrap_or(expected_lane);
        let file_path = |lane: &SampleIdentity| match &hits.lane(lane)?.source {
            LaneSource::File(name) => safe_child(root, name).ok(),
            LaneSource::Skin => None,
        };
        if file_path(expected_lane).as_ref() == Some(&source) {
            return Ok(());
        }
        let existing = source.starts_with(root).then(|| hits.lanes().iter().find(|candidate| {
            let lane = &candidate.identity;
            let compatible = match (expected_lane, lane) {
                (SampleIdentity::Standard { set, sound, .. }, SampleIdentity::Standard { set: other_set, sound: other_sound, .. }) =>
                    set == other_set && sound == other_sound,
                (SampleIdentity::File(_), SampleIdentity::File(_)) => true,
                _ => false,
            };
            compatible && self.clip_for_lane(lane).is_some()
                && (lane == logical || hits.lane_slot(lane).is_none())
                && !hits.lane_slots().any(|(_, slot)| slot == lane)
                && !hits.moments().iter().any(|moment| moment.hits().any(|used| used == lane))
                && file_path(lane).as_ref() == Some(&source)
        })).flatten().map(|lane| lane.identity.clone());
        let replacement = if let Some(lane) = existing {
            lane
        } else {
            let bytes = std::fs::read(&source).map_err(|e| format!("Reading sample: {e}"))?;
            let clip = dawsu_audio::decode(&source)?;
            if !std::fs::read(&source).is_ok_and(|current| current == bytes) {
                return Err("The sample changed while it was being read. Choose it again.".into());
            }
            let name = layers::create_backing(d, expected_lane, &extension, &bytes)?;
            let lane = dawsu_hitsounding::rules::read_sample_file_name(&name)
                .unwrap_or_else(|| SampleIdentity::File(name.clone()));
            self.document.as_mut().expect("open document").pending.add(&name, &bytes)?;
            self.register_samples(vec![(name, clip)], false);
            lane
        };
        let composer = &mut self.document.as_mut().expect("open document").composer;
        let selected = composer.selected.items().iter().cloned().map(|mut hit| {
            if hit.lane == *expected_lane {
                hit.lane = replacement.clone();
            }
            hit
        }).collect::<Vec<_>>();
        composer.apply("Replace lane sample", |hits| hits.replace_lane(expected_lane, replacement.clone()));
        composer.selected.replace(selected);
        self.active_lane = composer.lanes.iter().position(|lane| lane == &replacement)
            .expect("the backing lane is registered before replacing hits");
        self.status = format!("Assigned {}", source.file_name().unwrap_or_default().to_string_lossy());
        self.sync_audio();
        Ok(())
    }

    fn add_numbered_layer(&mut self) -> Result<(), String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before adding a sample layer.".into());
        }
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let family = document.composer.lanes.get(self.active_lane).ok_or("Choose a sample family.")?;
        let (name, bytes, clip) = layers::create(document, &self.preferences.default_samples, family)?;
        let lane = dawsu_hitsounding::rules::read_sample_file_name(&name)
            .expect("numbered layer names are constructed from sample identities");
        self.document.as_mut().expect("open document").pending.add(&name, &bytes)?;
        self.register_samples(vec![(name.clone(), clip)], false);
        if let Some(document) = &self.document
            && let Some(row) = document.composer.lanes.iter().position(|identity| *identity == lane)
        {
            self.active_lane = row;
        }
        self.status = format!("Added {name} with the default family sound.");
        Ok(())
    }

    fn poll_sample_folder(&mut self) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let mut additions = self.sample_watch.poll(document);
        additions.retain(|(name, _)| document.files.binary_search(name).is_err());
        // A file another program wrote under a pending name leaves the lane on its kept copy.
        // Save uses the file when it holds the same bytes and otherwise refuses and names it.
        let (clashes, additions): (Vec<_>, Vec<_>) = additions.into_iter().partition(|(name, _)| document.pending.get(name).is_some());
        let root = document.path.parent().map(Path::to_path_buf);
        let clashes: Vec<String> = clashes.into_iter().map(|(name, _)| name).collect();
        self.sample_watch.remember_reported(clashes.iter().cloned());
        let document = self.document.as_ref().expect("open document");
        let differing: Vec<String> = clashes.into_iter().filter(|name| {
            let kept = document.pending.get(name).and_then(|file| std::fs::read(&file.path).ok());
            let on_disk = root.as_ref().and_then(|root| safe_child(root, name).ok()).and_then(|path| std::fs::read(path).ok());
            kept != on_disk
        }).collect();
        if !differing.is_empty() {
            self.status = format!("Another program wrote {} in the mapset. dawsu!'s new sample of that name keeps playing; rename or remove the other file before saving.", differing.join(", "));
        }
        self.register_added_samples(additions);
    }

    /// After a write kept its files: the pending files it wrote are mapset files now, and the
    /// snapshot record lists the sound set files among them and among those the project keeps.
    fn pending_published(&mut self, names: &[String], kept: &[String]) {
        let Some(d) = self.document.as_mut() else { return };
        for name in names {
            d.pending.release(name);
        }
        d.files.extend(names.iter().cloned());
        d.files.sort();
        d.files.dedup();
        if let Err(error) = self.record_written_sets(&[names, kept].concat()) {

            self.error = Some(format!("The files were written. The sound set record could not be updated: {error}"));
        }
    }

    fn register_added_samples(&mut self, additions: Vec<(String, Arc<AudioClip>)>) {

        self.register_samples(additions, true);
    }

    /// Registers decoded samples as lanes. Files on disk join `Document.files`; pending files
    /// only join the hits' folder listing.
    fn register_samples(&mut self, additions: Vec<(String, Arc<AudioClip>)>, on_disk: bool) {
        if additions.is_empty() {
            return;
        }
        let Some(document) = &mut self.document else {
            return;
        };
        let mut names = Vec::with_capacity(additions.len());
        for (name, clip) in additions {
            self.samples.insert(SampleIdentity::File(name.clone()), clip);
            names.push(name);
        }
        if on_disk {
            document.files.extend_from_slice(&names);
            document.files.sort();
            document.files.dedup();
        }
        let composer = &mut document.composer;
        composer.history.update_external(|hits| hits.add_folder_samples(&names));
        for lane in composer.history.document().lanes() {
            if !composer.lanes.contains(&lane.identity) {
                composer.lanes.push(lane.identity.clone());
            }
        }
    }

    fn scheduled_sounds(&self, d: &Document) -> Vec<ScheduledSound> {
        self.scheduled_audio_and_midi(d).0
    }
    fn scheduled_audio_and_midi(&self, d: &Document) -> (Vec<ScheduledSound>, Vec<drum_vst::DrumMidiCue>) {
        if self.song_solo && !self.hitsounds_solo && self.solo.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let composer = &d.composer;
        let hits = composer.hitsounds();
        // Per-hit mute is keyed by moment and lane. Apply it before cue_list_filtered resolves
        // multiple lanes to possibly the same physical sample, without cloning `hits`.
        let muted_set: Option<std::collections::HashSet<(usize, &SampleIdentity)>> =
            (composer.listen_muted.len() > 16)
                .then(|| composer.listen_muted.iter().map(|k| (k.moment, &k.lane)).collect());
        let mut sounds = Vec::new();
        let mut midi_cues = Vec::new();
        for cue in cue_list_filtered(hits, &self.muted, &self.solo, |moment, lane| {
            if let Some(set) = &muted_set {
                set.contains(&(moment, lane))
            } else {
                composer.listen_muted.iter().any(|k| k.moment == moment && &k.lane == lane)
            }
        }) {
            let lane_gain = self.lane_gain(cue.sample());
            if let Some(vst) = &self.drum_vst
                && let Some(note) = vst.routed_note_for(cue.sample())
            {
                midi_cues.push(drum_vst::DrumMidiCue {
                    at_ms: cue.time().ms(),
                    note,
                    velocity: drum_vst::gain_to_midi_velocity(cue.gain() as f32 * lane_gain),
                });
                continue;
            }
            if let Some(clip) = self.samples.get(cue.sample()) {
                sounds.push(ScheduledSound {
                    at_ms: cue.time().ms(),
                    end_ms: cue.loop_end().map(|t| t.ms()),
                    clip: clip.clone(),
                    gain: cue.gain() as f32 * lane_gain,
                });
            }
        }
        (sounds, midi_cues)
    }
    fn sync_audio(&mut self) {
        if let Some(d) = &self.document {
            self.cached_version = d.composer.history.version();
            self.cached_listen_version = d.composer.listen_version;
            let (cues, midi_cues) = self.scheduled_audio_and_midi(d);
            if let Some(vst) = &self.drum_vst {
                vst.set_schedule(midi_cues);
            }
            if let Some(a) = &self.audio {
                a.load_at_speed(
                    self.song.clone(),
                    cues,
                    self.playback_speed,
                );
                a.gains(self.playback_song_gain(), self.preferences.master_gain);
                a.hitsound_gain(self.playback_hitsound_gain());
            }
        }
    }

    /// Loads a VST3 drum instrument into an isolated sidecar and attaches it to live audio playback.
    pub fn load_drum_vst(
        &mut self,
        plugin_path: PathBuf,
        plugin_uid: Option<String>,
    ) -> Result<LoadedPluginSummary, Vst3Error> {
        self.unload_drum_vst();
        let summary = self.vst3.load_instrument(plugin_path.clone(), plugin_uid.clone())?;
        let shm = self.vst3.take_shm(&summary.id)?.ok_or_else(|| {
            Vst3Error::Bounce("VST3 shared-memory audio channel unavailable".to_string())
        })?;
        let (mut handle, source) = drum_vst::DrumVstLiveHandle::new(
            summary.id.clone(),
            summary.name.clone(),
            plugin_path.to_string_lossy().into_owned(),
            plugin_uid,
            true,
            Box::new(shm),
        );
        // Seed skin lane defaults immediately until drum_view supplies explicit drum-map rows.
        if let Some(d) = &self.document {
            let mut notes = HashMap::new();
            for lane in &d.composer.lanes {
                if let Some(note) = drum_vst::default_skin_midi_note(lane) {
                    notes.insert(lane.clone(), note);
                }
            }
            handle.set_routed_notes(notes);
        }
        if let Some(a) = &self.audio {
            a.set_live_source(Some(Box::new(source)));
        }
        self.drum_vst = Some(handle);
        self.drum_vst_state_b64 = self.vst3.get_plugin_state(&summary.id).ok();
        self.sync_audio();
        let _ = self.write_session(None, None, None);
        Ok(summary)
    }

    fn restore_drum_vst_session(&mut self, saved: &project::DrumVstSession) -> Result<(), String> {
        let path = PathBuf::from(&saved.plugin_path);
        if !path.exists() {
            return Err(format!("plugin not found at {}", saved.plugin_path));
        }
        let summary = self
            .load_drum_vst(path, saved.plugin_uid.clone())
            .map_err(|e| e.to_string())?;
        if let Some(state_b64) = saved.state_b64.as_deref()
            && !state_b64.is_empty()
        {
            let _ = self.vst3.set_plugin_state(&summary.id, state_b64);
            self.drum_vst_state_b64 = Some(state_b64.to_string());
        }
        self.set_drum_vst_enabled(saved.enabled);
        Ok(())
    }

    /// Instant ON/OFF toggle for live VST3 drum routing (keeps plugin loaded in memory).
    pub fn set_drum_vst_enabled(&mut self, enabled: bool) {
        if let Some(vst) = &self.drum_vst {
            vst.set_enabled(enabled);
        }
        self.sync_audio();
        let _ = self.write_session(None, None, None);
    }

    /// Updates the lane -> MIDI note routing map (from `DrumView` + skin defaults).
    pub fn set_drum_vst_routing(&mut self, notes: HashMap<SampleIdentity, u8>) {
        let changed = self
            .drum_vst
            .as_ref()
            .is_some_and(|vst| vst.routed_notes() != &notes);
        if let Some(vst) = &mut self.drum_vst {
            vst.set_routed_notes(notes);
        }
        if changed {
            self.sync_audio();
        }
    }

    /// Detaches and unloads the active live VST3 drum instrument.
    pub fn unload_drum_vst(&mut self) {
        if let Some(a) = &self.audio {
            a.clear_live_source();
        }
        if let Some(vst) = self.drum_vst.take() {
            let _ = self.vst3.unload_plugin(&vst.instance_id);
        }
        self.drum_vst_state_b64 = None;
        self.sync_audio();
        let _ = self.write_session(None, None, None);
    }

    /// Refreshes the cached VST3 component state chunk and saves it into the map's session.
    pub fn save_drum_vst_session_state(&mut self) -> Result<String, Vst3Error> {
        let id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or_else(|| Vst3Error::PluginNotFound("No Drum VST3 loaded".into()))?;
        let b64 = self.vst3.get_plugin_state(&id)?;
        self.drum_vst_state_b64 = Some(b64.clone());
        let _ = self.write_session(None, None, None);
        Ok(b64)
    }

    /// Restores a base64 state chunk into the active Drum VST3 and saves it into the map's session.
    pub fn restore_drum_vst_state_b64(&mut self, state_b64: &str) -> Result<(), Vst3Error> {
        let id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or_else(|| Vst3Error::PluginNotFound("No Drum VST3 loaded".into()))?;
        self.vst3.set_plugin_state(&id, state_b64)?;
        self.drum_vst_state_b64 = Some(state_b64.to_string());
        let _ = self.write_session(None, None, None);
        Ok(())
    }

    /// Plans a "Render drums to WAVs…" bake over all hits on currently routed VST3 drum lanes.
    pub fn plan_drum_vst_render(&self, mode: VelocityExportMode) -> Result<Vst3DrumExportPlan, String> {
        let d = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let vst = self.drum_vst.as_ref().ok_or("Load a Drum VST3 instrument first.")?;
        let routed = vst.routed_notes();
        let mut hits = Vec::new();
        for moment in d.composer.hitsounds().moments() {
            for lane in moment.hits() {
                if let Some(&pitch) = routed.get(lane) {
                    let volume = moment.hit_volume(lane).clamp(5, 100) as u8;
                    hits.push(Vst3DrumHit {
                        time_ms: moment.time().ms(),
                        name: lane_name(lane),
                        pitch,
                        volume,
                    });
                }
            }
        }
        Ok(plan_vst3_drum_export(&hits, mode))
    }

    /// Executes "Render drums to WAVs…" into the mapset folder (default: Tiered velocity).
    pub fn render_drum_vst_to_wavs(
        &mut self,
        mode: VelocityExportMode,
    ) -> Result<(Vst3DrumExportPlan, Vec<BounceOutput>), String> {
        let plan = self.plan_drum_vst_render(mode)?;
        if plan.jobs.is_empty() {
            return Err("No hits found on mapped VST3 drum lanes.".into());
        }
        let instance_id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or("Load a Drum VST3 instrument first.")?;
        let out_dir = self
            .document
            .as_ref()
            .and_then(|d| d.path.parent().map(Path::to_path_buf))
            .ok_or("Map folder unavailable.")?;
        let outputs = self
            .vst3
            .execute_vst3_drum_export(&instance_id, &plan, &out_dir, 44_100)
            .map_err(|e| e.to_string())?;
        self.status = format!(
            "Rendered {} VST3 drum WAV {} ({} mapped hits).",
            outputs.len(),
            if outputs.len() == 1 { "file" } else { "files" },
            plan.assignments.len()
        );
        Ok((plan, outputs))
    }
}
impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}
fn toggle<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if let Some(i) = items.iter().position(|v| v == &item) {
        items.remove(i);
    } else {
        items.push(item);
    }
}
pub fn lane_name(identity: &SampleIdentity) -> String {
    match identity {
        SampleIdentity::File(name) => name.clone(),
        SampleIdentity::Standard { set, sound, index } => {
            let set = match set {
                SampleSet::Normal => "normal",
                SampleSet::Soft => "soft",
                SampleSet::Drum => "drum",
            };
            let sound = match sound {
                Sound::Hit(Hitsound::Normal) => "hitnormal",
                Sound::Hit(Hitsound::Whistle) => "hitwhistle",
                Sound::Hit(Hitsound::Finish) => "hitfinish",
                Sound::Hit(Hitsound::Clap) => "hitclap",
                Sound::Slider(SliderSound::Slide) => "sliderslide",
                Sound::Slider(SliderSound::Whistle) => "sliderwhistle",
                Sound::Slider(SliderSound::Tick) => "slidertick",
            };
            format!(
                "{set}-{sound}{}",
                if index.get() > 1 {
                    index.get().to_string()
                } else {
                    String::new()
                }
            )
        }
    }
}
fn safe_child(root: &Path, name: &str) -> Result<PathBuf, String> {
    check_map_path(name).map_err(|e| refusals::file_name(&e))?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let child = root
        .join(name.replace('\\', "/"))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !child.starts_with(&root) {
        return Err("A sample path leaves its folder".into());
    }
    Ok(child)
}
/// The file a lane plays: a pending file's kept copy, the mapset's file, or the skin's sound
/// (the default sounds folder's file, else the one dawsu! ships).
fn sample_path(
    lane: &Lane,
    folder: &Path,
    pending: &pending::Pending,
    defaults: &Path,
    defaults_list: &[String],
) -> Result<PathBuf, String> {
    match &lane.source {
        LaneSource::File(name) => match pending.get(name) {
            Some(file) => Ok(file.path.clone()),
            None => safe_child(folder, name),
        },
        LaneSource::Skin => {
            let name = lane_name(&lane.plays_as());
            ["wav", "ogg", "mp3"]
                .iter()
                .find_map(|ext| {
                    find_case_insensitive(defaults_list, &format!("{name}.{ext}"))
                        .and_then(|n| safe_child(defaults, n).ok())
                })
                // The chosen default sounds folder lacks it: osu!'s own sound, shipped with dawsu!.
                .or_else(|| default_sounds::path(&format!("{name}.wav")))
                .ok_or_else(|| format!("Sample unavailable: {}", lane_name(&lane.identity)))
        }
    }
}

fn load(
    path: &Path,
    defaults: &Path,
    data_root: &Path,
    cancel: &AtomicBool,
    project_root: Option<&Path>,
    legacy_root: Option<&Path>,
    recovery: Option<recovery::RecoveryCopy>,
) -> Result<Loaded, String> {
    let mut document = Document::read_matching(path, recovery.as_ref())?;
    let mapset = document.path.parent().ok_or("Missing mapset folder")?.to_path_buf();
    document.pending = pending::Pending::for_mapset(data_root, &mapset);
    document.presence = Some(pending::Presence::enter(data_root, &mapset).map(Arc::new)
        .map_err(|error| format!("Could not record this editing session for the mapset, so nothing was opened: {error}"))?);
    // With no other session on the mapset, claims no record keeps were left by a session that
    // closed without Save or crashed: their numbers are free again.
    if document.presence.as_ref().is_some_and(|presence| !presence.others_live()) {
        // A recovery copy that can't be read may need any claim: none is released.
        if let Some(keep) = recorded_pending(project_root, data_root, &mapset) {
            document.pending.release_stale(&keep);
        }
        // No cleanup is running: a file one left moving aside goes back under its name, and its
        // lane plays it again.
        let back = sweep::put_back_stopped_moves(&mapset);
        if !back.is_empty() {
            if recovery.is_none() {
                document.files = pending::listing(&mapset, &document.pending)?;
            }
            document.composer.history.update_external(|hits| hits.add_folder_samples(&back));
        }
    }
    let mut warnings = Vec::new();
    // A recovery copy's pending files come back first, so every hit it restores finds its sound.
    if let Some(copy) = &recovery {
        let names = restore_pending(&mut document, copy.pending(), &mut warnings);
        document.composer.history.update_external(|hits| hits.add_folder_samples(&names));
    }
    let mut project = project_root.and_then(|root| {
        match project::Project::load(root, legacy_root, &document.path) {
            Ok(project) => Some(project),
            Err(error) => {
                warnings.push(format!(
                    "Session settings unavailable: {error}. The existing project file was kept."
                ));
                None
            }
        }
    });
    // A recovery copy is matched against the decoded difficulty. It already holds the
    // loose hits from the project, and its row order and HS mode win over the project's.
    let recovered_view = match &recovery {
        Some(copy) => {
            copy.apply(&mut document, project.is_some(), cancel)?;
            Some((document.composer.lanes.clone(), document.hs_mode))
        }
        None => None,
    };
    // The pending files the last Save left out come back before any hit that plays them.
    if let Some(kept) = project.as_ref().and_then(|p| p.settings.pending_samples.as_ref()) {
        let names = restore_pending(&mut document, kept, &mut warnings);
        let composer = &mut document.composer;
        composer.history.update_external(|hits| hits.add_folder_samples(&names));
        for lane in composer.history.document().lanes() {
            if !composer.lanes.contains(&lane.identity) {
                composer.lanes.push(lane.identity.clone());
            }
        }
    }
    if let Some(recovery) = project.as_ref().and_then(|p| p.settings.gain_recovery.as_ref())
        && let Err(error) = gain_recovery::restore(&mut document, recovery, defaults)

    {
        warnings.push(format!("Editable hit gains could not be restored: {error}"));
    }
    // What was last written: what the opened file holds, with its restored gains. Loose hits and
    // left-out moments only live in the project. A recovery copy brings its own last Save.
    if recovery.is_none() {
        document.written = document.composer.hitsounds().clone();
    }
    if recovery.is_none()
        && let Some(saved) = project
            .as_ref()
            .and_then(|p| p.settings.unanchored.as_deref())
    {
        restore_project_hits(&mut document.composer, saved)?;
    }
    if let Some(slots) = project.as_ref().and_then(|p| p.settings.lane_slots.as_ref()) {
        let composer = &mut document.composer;
        composer.history.update_external(|hits| {
            for (source, slot) in slots {
                if let (Some(source), Some(slot)) = (project::lane_identity(source), project::lane_identity(slot)) {
                    hits.bind_lane_slot(source, slot);
                }
            }
        });
        composer.lanes = composer.hitsounds().lanes().iter().map(|lane| lane.identity.clone()).collect();
    }
    let mut restored_left_out = false;
    if recovery.is_none()
        && let Some(record) = project.as_ref().and_then(|p| p.settings.unwritten.as_ref())
    {
        match unwritten::restore(&mut document, record) {
            Ok(restored) => restored_left_out = restored,
            Err(warning) => warnings.push(warning),
        }
    }
    if let Some(saved) = project.as_ref().and_then(|p| p.settings.bookmarks.as_deref()) {
        document.bookmarks = BookmarkList::default();
        for mark in saved {
            if mark.time.is_finite() && mark.time >= 0.0 {
                document.bookmarks.add(SongTime::from_ms(mark.time), mark.label.clone());
            }
        }
        document.bookmark_override = true;
    }
    if let Some(hidden) = project.as_ref().and_then(|p| p.settings.hidden_lanes.as_ref()) {
        document.composer.history.update_external(|hits| {
            hits.restore_hidden_lanes(hidden.iter().filter_map(|key| project::lane_identity(key)));
        });
    }
    document.lane_layout = project.as_ref().and_then(|p| p.settings.lane_layout.clone());
    document.hs_mode = project.as_ref().and_then(|p| p.settings.hs_mode).unwrap_or(false);
    if let Some(ownership) = project.as_ref().and_then(|p| p.settings.hs_circle_ownership.as_ref()) {
        document.owned_hs_circles = ownership.verified_lines(&document.compile_base);
    }
    if let Some(signatures) = project.as_ref().and_then(|p| p.settings.time_signatures.clone()) {
        document.update_time_signatures(signatures);
    }
    document.export_settings = project.as_ref().and_then(|p| p.settings.export_settings.clone());
    document.drum_lanes = project.as_ref().and_then(|p| p.settings.drum_lanes.clone());
    document.export_choices = project.as_ref().and_then(|p| p.settings.export_choices.clone());
    document.node_graph_bindings = project.as_ref().and_then(|p| p.settings.node_graph_bindings.clone());
    if let Some(layout) = &mut document.lane_layout {
        layout.normalise();
    }
    if let Some((lanes, hs_mode)) = recovered_view {
        document.composer.lanes = lanes;
        document.hs_mode = hs_mode;
    }
    // Sound set links come back last, once every hit they name is in place; a recovery copy
    // brings its own.
    if recovery.is_none()
        && let Some(project) = project.as_mut()
        && let Some(links) = project.settings.sound_sets.clone()
    {
        let (kept, warning) = sound_sets::restore_links(&mut document, &links);
        project.kept_links = kept;
        warnings.extend(warning);
    }
    // The moments the last Save left out are listed again before any Save: those put back from
    // the project or a recovery copy, and in HS mode loose hits kept in the project because they
    // never became a circle.
    let loose = || document.composer.hitsounds().moments().iter()
        .any(|m| m.place() == dawsu_hitsounding::Place::NoObject && m.hits().next().is_some());
    if project.is_some() && (restored_left_out || recovery.is_some() || document.hs_mode && loose()) {
        match left_out_on_open(&document, &|| cancel.load(Ordering::Relaxed)) {
            Ok(left_out) => document.left_out = left_out,
            Err(error) => warnings.push(format!("The moments the last Save left out could not be listed: {error}")),
        }
    }
    let folder = document.path.parent().ok_or("Missing folder")?;
    let defaults_list = if defaults.is_dir() {
        crate::pending::sample_folder(defaults)
    } else {
        Vec::new()
    };
    let mut unique_sources: Vec<(SampleIdentity, Result<PathBuf, String>)> = Vec::new();
    let mut seen_sources = std::collections::HashSet::new();
    for lane in document.composer.hitsounds().lanes() {
        let source = lane.plays_as();
        if seen_sources.insert(source.clone()) {
            let resolved = sample_path(lane, folder, &document.pending, defaults, &defaults_list);
            unique_sources.push((source, resolved));
        }
    }
    let song_path = match document.file.general("AudioFilename") {
        Some(name) => match safe_child(folder, &name) {
            Ok(path) => Some(path),
            Err(e) => {
                warnings.push(format!("Song audio unavailable: {e}"));
                None
            }
        },
        None => None,
    };
    let samples_mutex: Mutex<(HashMap<SampleIdentity, Arc<AudioClip>>, Vec<String>)> =
        Mutex::new((HashMap::with_capacity(unique_sources.len()), Vec::new()));

    if !unique_sources.is_empty() {
        let worker_count = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, 6);
        let next_idx = std::sync::atomic::AtomicUsize::new(0);

        std::thread::scope(|scope| {
            // Decode unique hitsound samples in parallel across worker threads; the song audio
            // and its FFT waveform decode asynchronously after the document opens.
            let sample_workers = worker_count.min(unique_sources.len());
            for _ in 0..sample_workers {
                let sources_ref = &unique_sources;
                let idx_ref = &next_idx;
                let out_ref = &samples_mutex;
                let cancel_ref = &cancel;
                scope.spawn(move || {
                    let mut local_clips = Vec::new();
                    let mut local_warnings = Vec::new();
                    loop {
                        if cancel_ref.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = idx_ref.fetch_add(1, Ordering::Relaxed);
                        let Some((source, resolved)) = sources_ref.get(i) else {
                            break;
                        };
                        match resolved
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(|path| dawsu_audio::decode_with_cancel(path, &|| cancel_ref.load(Ordering::Relaxed)))
                        {
                            Ok(clip) => local_clips.push((source.clone(), clip)),
                            Err(error) => local_warnings.push(error),
                        }
                    }
                    if let Ok(mut guard) = out_ref.lock() {
                        guard.0.extend(local_clips);
                        guard.1.extend(local_warnings);
                    }
                });
            }
        });
    }

    if cancel.load(Ordering::Relaxed) {
        return Err("Open cancelled".into());
    }
    let (samples, sample_warnings) = samples_mutex.into_inner().unwrap_or_default();
    warnings.extend(sample_warnings);
    Ok(Loaded {
        document,
        project,
        song_path,
        samples,
        warnings,
        recovery,
        outgoing_keep: None,
    })
}

/// The pending files a record still lists for this mapset: the project of any of its
/// difficulties, or a recovery copy not yet restored. Lowercased names. `None` when a recovery
/// copy can't be read: it could list any name.
fn recorded_pending(project_root: Option<&Path>, data_root: &Path, mapset: &Path) -> Option<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    let entries = std::fs::read_dir(mapset).map(|entries| entries.filter_map(Result::ok).map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default();
    if let Some(root) = project_root {
        for map in entries.iter().filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("osu"))) {
            if let Ok(project) = project::Project::load(root, None, map) {
                names.extend(project.settings.pending_samples.iter().flatten().map(|file| file.name.to_ascii_lowercase()));
            }
        }
    }
    for copy in recovery::mapset_copies(data_root, mapset).ok()? {
        names.extend(copy.pending().iter().map(|file| file.name.to_ascii_lowercase()));
    }
    Some(names)
}

/// Takes back listed pending files whose kept copies still match; names each one that can't be.
fn restore_pending(document: &mut Document, list: &[pending::PendingSample], warnings: &mut Vec<String>) -> Vec<String> {
    let mut names = Vec::new();
    for file in list {
        match document.pending.restore(&file.name, &file.sha256) {
            Ok(_) => names.push(file.name.clone()),
            Err(error) => warnings.push(format!("{error} Its lane plays the skin's sound.")),
        }
    }
    names
}

fn project_hits(document: &Document) -> Vec<project::ProjectHit> {
    let mut saved = Vec::new();
    for (group, moment) in document
        .composer
        .hitsounds()
        .moments()
        .iter()
        .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject)
        .enumerate()
    {
        for lane in moment.hits() {
            let key = project::lane_key(lane);
            saved.push(project::ProjectHit {
                group: Some(group),
                default_volume: moment.has_hit_volume_overrides().then(|| moment.volume()),
                lane: key,
                time: moment.time().ms(),
                volume: Some(moment.hit_volume(lane)),
            });
        }
    }
    saved
}

/// What Save would leave out of the opened difficulty, without rendering any sample mix.
fn left_out_on_open(document: &Document, cancel: &dyn Fn() -> bool) -> Result<Vec<roll_back::LeftOut>, String> {
    let hits = document.composer.hitsounds();
    let placement = if document.hs_mode { Placement::OnObjects } else { Placement::GreenLines };
    let inputs = roll_back::Inputs {
        base: &document.compile_base, drawn: hits, written: &document.written, hs: document.hs_mode,
        owned: &document.owned_hs_circles, loose_in_project: true, roll_back: true,
    };
    roll_back::compile_with_roll_back(
        &inputs, &|problem| mixes::can_render(problem, placement), &mut |pass, _, _| Ok(Ok(pass.out.clone())),
        cancel, &mut |_, _| {},
    ).map(|rolled| rolled.left_out)
}

/// Project rows for loose moments, one group each. Each row names its moment's volume, so a row is
/// never taken for a hit the file already holds.
fn loose_rows(states: &[roll_back::MomentState]) -> Vec<project::ProjectHit> {
    states.iter().enumerate().flat_map(|(group, state)| state.hits.iter().map(move |(lane, volume)| project::ProjectHit {
        group: Some(group),
        default_volume: Some(state.volume),
        lane: project::lane_key(lane),
        time: f64::from_bits(state.time_bits),
        volume: Some(*volume),
    })).collect()
}

fn restore_project_hits(
    composer: &mut Composer,
    saved: &[project::ProjectHit],
) -> Result<(), String> {
    if saved.is_empty() {
        return Ok(());
    }
    let mut hits = composer.hitsounds().clone();
    let mut anchored = HashMap::new();
    for m in hits.moments() {
        for lane in m.hits() {
            anchored
                .entry((m.time().ms().to_bits(), lane.clone(), m.hit_volume(lane)))
                .and_modify(|count| *count += 1)
                .or_insert(1usize);
        }
    }
    let mut groups = HashMap::new();
    let mut voices = HashMap::new();
    for hit in saved {
        let lane = project::lane_identity(&hit.lane)
            .ok_or_else(|| format!("Cannot restore project hit on lane {}", hit.lane))?;
        let volume = hit.volume.unwrap_or(100).clamp(0, 100);
        let time = hit.time.to_bits();
        if hit.default_volume.is_none()
            && let Some(count) = anchored.get_mut(&(time, lane.clone(), volume))
        {
            if *count > 0 {
                *count -= 1;
                continue;
            }
        }
        // Legacy rows have no moment identifier. Co-timed lanes share volume;
        // repeated voices on the same lane need distinct moments.
        let group = hit.group.unwrap_or_else(|| {
            let voice = voices.entry((time, volume, lane.clone())).or_insert(0);
            let index = *voice;
            *voice += 1;
            index
        });
        let default = hit.default_volume.unwrap_or(volume).clamp(0, 100);
        let moment = groups
            .entry((time, default, group))
            .or_insert_with(|| hits.add_moment(SongTime::from_ms(hit.time), default));
        hits.add_hit_with_volume(*moment, lane, volume);
    }
    let tempo = std::mem::take(&mut composer.tempo);
    *composer = Composer::new(hits);
    composer.tempo = tempo;
    Ok(())
}

/// A left-out part on the problem list. A slider body's row names its volume, so it reads apart
/// from its head's row at the same time.
fn left_out_problem(entry: &roll_back::LeftOut, path: &Path) -> OutputProblem {
    let problem = OutputProblem::new(&entry.problem, path, path);
    match entry.part {
        roll_back::Part::Body(_) => problem.with_message(body_description(&entry.problem.kind)),
        _ => problem,
    }
}

fn body_description(problem: &dawsu_hitsounding::ProblemKind) -> &'static str {
    use dawsu_hitsounding::ProblemKind::*;
    match problem {
        NeedsGreenLine => "The slider body volume needs a green line that the output mode does not allow.",
        LookupWindowTaken => "The slider body volume needs a green line, and another lookup overlaps it.",
        BodyVolumeNeedsHead => "The slider body volume conflicts with its head.",
        _ => "This slider can't hold the slider body volume or sounds as drawn.",
    }
}

fn problem_description(problem: &dawsu_hitsounding::ProblemKind, time: f64) -> String {
    use dawsu_hitsounding::ProblemKind::*;
    if let SliderSoundOnHit(lane) = problem {
        return format!(
            "At {time:.0} ms a slider sound ({}) is on a hit. osu! plays slider sounds only on a slider body. Remove it from the hit.",
            lane_name(lane),
        );
    }
    match problem {
        NoObject => "This hit has no object. Enable HS mode or Generate a hitsound difficulty.",
        NeedsSampleMix(layers) if layers.is_empty() => {
            "This object has no hits left, and osu! always plays a sound here. Its old sound stays in the difficulty."
        }
        NeedsSampleMix(_) => {
            "This hit needs a sample mix that the current Save mode cannot write."
        }
        NeedsGreenLine => "This hit needs a green line that the output mode does not allow.",
        LookupWindowTaken => "Another lookup overlaps the timing line needed by this hit.",
        LineRefused => "This object cannot hold the requested change.",
        BodyVolumeNeedsHead => "The slider body volume conflicts with its head.",
        SliderSoundOnHit(_) => unreachable!("described above"),
    }
    .into()
}

#[cfg(test)]
mod project_workflow {
    use super::*;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    fn open(app: &mut Application, path: PathBuf) {
        app.request_open(path);
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn selected_lane_volume_and_shortcut_leave_other_voices_at_the_moment() {
        let fixture = Scratch::new("selected-lane-volume");
        let path = fixture.path("map.osu");
        let map = b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,1,70,1,0\n[HitObjects]\n256,192,1000,1,8,1:1:1:70:\n128,192,1000,1,0,1:1:1:70:\n";
        std::fs::write(&path, map).unwrap();
        let wav_head = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0";
        for (name, sample) in [("normal-hitnormal.wav", 8192_i16), ("normal-hitclap.wav", 16384)] {
            let mut wav = wav_head.to_vec();
            wav.extend(sample.to_le_bytes());
            std::fs::write(fixture.path(name), wav).unwrap();
        }
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("sessions"));
        open(&mut app, path.clone());
        let lanes: Vec<_> = app.document().unwrap().composer.hitsounds().moments()[0].hits().cloned().collect();
        let normal = lanes.iter().find(|lane| matches!(lane,
            SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Normal), .. })).unwrap().clone();
        let clap = lanes.iter().find(|lane| *lane != &normal).unwrap().clone();
        app.select_hit(HitKey { moment: 0, lane: clap.clone() }, false);
        app.set_selection_lane_scope(Some(BTreeSet::from([normal.clone()]))).unwrap();
        app.selection_volume(30, false);
        app.command("volumeUp");
        let gains = |app: &Application| {
            let moment = &app.document().unwrap().composer.hitsounds().moments()[0];
            (moment.hit_volume(&normal), moment.hit_volume(&clap))
        };
        assert_eq!(gains(&app), (35, 70));
        use dawsu_editor::keys::{Key, Mods};
        let fine = Mods::ALT.union(Mods::SHIFT);
        app.key(KeyChord::new(Key::Up, fine));
        assert_eq!(gains(&app), (36, 70));
        app.key(KeyChord::new(Key::Down, fine));
        assert_eq!(gains(&app), (35, 70));
        app.command("undo");
        assert_eq!(gains(&app), (36, 70));
        app.command("undo");
        assert_eq!(gains(&app), (35, 70));
        app.command("undo");
        assert_eq!(gains(&app), (30, 70));
        app.command("undo");
        assert_eq!(gains(&app), (70, 70));
        app.command("redo");
        app.command("redo");
        let save = |app: &mut Application| {
            app.save_with_backups(fixture.path("backups"));
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(app.error.is_none(), "{:?}", app.error);
        };
        save(&mut app);
        let written = std::fs::read(&path).unwrap();
        assert_ne!(written, map);
        assert!(written.ends_with(b"128,192,1000,1,0,1:1:1:70:\n"));
        let decoded = Document::read(&path).unwrap();
        // Save merges per role at the clap's volume: the hitnormal file holds it at half.
        let saved = &decoded.composer.hitsounds().moments()[0];
        assert_eq!(saved.volume(), 70);
        assert!(saved.has_hit(&SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Normal), index: dawsu_foundation::CustomIndex::new(2) }));
        let pcm = dawsu_audio::decode(&fixture.path("normal-hitnormal2.wav")).unwrap();
        assert!((pcm.samples[0] - 0.25 * 0.5).abs() < 0.0001);
        let pcm = dawsu_audio::decode(&fixture.path("normal-hitclap2.wav")).unwrap();
        assert!((pcm.samples[0] - 0.5).abs() < 0.0001);
        let mut reopened = Application::headless();
        reopened.project_root = app.project_root.clone();
        open(&mut reopened, path.clone());
        assert_eq!(gains(&reopened), (35, 70));
        save(&mut reopened);
        assert_eq!(std::fs::read(&path).unwrap(), written);

        // A pending recovery record must stay inactive if the map write fails.
        reopened.select_hit(HitKey { moment: 0, lane: clap.clone() }, false);
        reopened.set_selection_lane_scope(Some(BTreeSet::from([normal.clone()]))).unwrap();
        reopened.selection_volume(55, false);
        let blocked_backup = fixture.path("blocked");
        std::fs::write(&blocked_backup, b"not a backup directory").unwrap();
        reopened.save_with_backups(blocked_backup);
        let deadline = Instant::now() + Duration::from_secs(10);
        while reopened.busy {
            reopened.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(reopened.error.is_some());
        assert!(reopened.is_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), written);

        let mut retry = Application::headless();
        retry.project_root = app.project_root.clone();
        open(&mut retry, path.clone());
        assert_eq!(gains(&retry), (35, 70));

        let original_sample = fixture.path("normal-hitclap.wav");
        let mut changed = std::fs::read(&original_sample).unwrap();
        changed[44] ^= 1;
        std::fs::write(&original_sample, changed).unwrap();
        let mut external = Application::headless();
        external.project_root = app.project_root.clone();
        open(&mut external, path.clone());
        assert!(external.warnings.iter().any(|warning| warning.contains("An original sample changed")));
        assert!(!external.document().unwrap().composer.hitsounds().moments()[0].has_hit(&clap));
        assert_eq!(std::fs::read(path).unwrap(), written);
    }


    const SLIDER_MAP: &[u8] = b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Normal\r\n\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n100,100,2000,2,0,L|200:100,1,140\r\n";

    fn wav(path: &Path, frame: i16) {
        let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
        wav.extend(frame.to_le_bytes());
        std::fs::write(path, wav).unwrap();
    }

    fn skin(set: SampleSet, sound: Hitsound) -> SampleIdentity {
        SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: dawsu_foundation::CustomIndex::new(0) }
    }

    /// HS mode: the circle needs a sample mix of two files, which Save renders; the slider's tail
    /// asks for a soft and a drum clap, which no object line can hold.
    fn mix_and_unwritable(scratch: &Scratch, second: &str) -> (Document, Hitsounds, project::Project) {
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        wav(&folder.join("a.wav"), 8192);
        wav(&folder.join("b.wav"), 4096);
        // A file in the folder that is not audio, for the test whose mix must fail.
        if second == "broken.wav" {
            std::fs::write(folder.join(second), b"not audio").unwrap();
        }
        let document = Document::read(&map).unwrap();
        let mut hits = document.composer.hitsounds().clone();
        hits.set_moment_state(dawsu_hitsounding::MomentId(0), 80, [
            (SampleIdentity::File("a.wav".into()), 80), (SampleIdentity::File(second.into()), 40),
        ]);
        let tail = hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 1, .. })).unwrap();
        hits.add_hit(dawsu_hitsounding::MomentId(tail), skin(SampleSet::Soft, Hitsound::Clap));
        hits.add_hit(dawsu_hitsounding::MomentId(tail), skin(SampleSet::Drum, Hitsound::Clap));
        let project = project::Project::load(&scratch.path("projects"), None, &document.path).unwrap();
        (document, hits, project)
    }

    fn prepare(scratch: &Scratch, document: &Document, hits: &Hitsounds, project: Option<&project::Project>, diagnostics: &mut Vec<OutputProblem>) -> Result<PreparedSave, String> {
        prepare_save(
            document.compile_base.clone(), hits, &document.written, document.file.original_bytes().to_vec(), &document.path,
            scratch.path("backups"), true, &BTreeSet::new(), Path::new(""), &document.pending, project, false, &|| false, diagnostics, &mut |_, _| {},

        )
    }

    #[test]
    fn save_renders_the_mixes_it_can_and_lists_only_the_parts_it_leaves_out() {
        let scratch = Scratch::new("mix-beside-left-out");
        let (document, hits, project) = mix_and_unwritable(&scratch, "b.wav");
        let mut diagnostics = Vec::new();
        let plan = prepare(&scratch, &document, &hits, Some(&project), &mut diagnostics).unwrap();
        assert_eq!(diagnostics.iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0], "the rendered mix is not a problem");
        assert_eq!(plan.left_out.len(), 1);
        assert_eq!(plan.mix_files.staged.len(), 1, "the circle's mix is rendered");
    }

    #[test]
    fn hs_save_merges_a_circle_per_role_and_leaves_a_slider_head_out() {
        let scratch = Scratch::new("hs-merge-roles");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        wav(&folder.join("normal-hitnormal3.wav"), 8192);
        wav(&folder.join("drum-hitfinish3.wav"), 4096);
        wav(&folder.join("soft-hitfinish4.wav"), 16384);
        let document = Document::read(&map).unwrap();
        let mut hits = document.composer.hitsounds().clone();
        let numbered = |set, sound, index| SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: dawsu_foundation::CustomIndex::new(index) };
        hits.set_moment_state(dawsu_hitsounding::MomentId(0), 80, [
            (numbered(SampleSet::Normal, Hitsound::Normal, 3), 80),
            (numbered(SampleSet::Drum, Hitsound::Finish, 3), 80),
            (numbered(SampleSet::Soft, Hitsound::Finish, 4), 40),
        ]);
        // On objects only, a slider head cannot take a new index, so it stays left out.
        let head = hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
        hits.set_moment_state(dawsu_hitsounding::MomentId(head), 70, [
            (skin(SampleSet::Normal, Hitsound::Normal), 70), (skin(SampleSet::Drum, Hitsound::Finish), 70), (skin(SampleSet::Soft, Hitsound::Finish), 35),
        ]);
        let project = project::Project::load(&scratch.path("projects"), None, &document.path).unwrap();
        let mut diagnostics = Vec::new();
        let plan = prepare(&scratch, &document, &hits, Some(&project), &mut diagnostics).unwrap();
        let names: Vec<_> = plan.mix_files.staged.iter().map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["normal-hitnormal2.wav", "drum-hitfinish2.wav"]);
        assert!((plan.mix_files.staged[0].clip().samples[0] - 0.25).abs() < 0.0001);
        assert!((plan.mix_files.staged[1].clip().samples[0] - (0.125 + 0.5 * 0.5)).abs() < 0.0001);
        let text = String::from_utf8(plan.out.to_bytes()).unwrap();
        assert!(text.contains("256,192,1000,1,4,0:3:2:80:\r\n"), "{text}");
        assert_eq!(plan.left_out.len(), 1);
        assert_eq!(diagnostics.iter().map(|p| p.time).collect::<Vec<_>>(), [2000.0]);
    }

    #[test]
    fn a_slider_sound_on_a_hit_is_left_out_and_save_writes_the_rest() {
        let scratch = Scratch::new("slider-sound-left-out");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        let document = Document::read(&map).unwrap();
        let mut hits = document.composer.hitsounds().clone();
        let slide = SampleIdentity::Standard { set: SampleSet::Soft, sound: Sound::Slider(SliderSound::Slide), index: dawsu_foundation::CustomIndex::new(0) };
        hits.add_hit(dawsu_hitsounding::MomentId(0), slide);
        let tail = tail_id(&hits);
        hits.add_hit(tail, skin(SampleSet::Normal, Hitsound::Whistle));
        let project = project::Project::load(&scratch.path("projects"), None, &document.path).unwrap();
        let mut diagnostics = Vec::new();
        let plan = prepare(&scratch, &document, &hits, Some(&project), &mut diagnostics).unwrap();
        // The tail's whistle is written; the circle keeps its old line and is listed.
        let text = String::from_utf8(plan.out.to_bytes()).unwrap();
        assert!(text.contains("256,192,1000,1,0,0:0:0:0:\r\n"), "{text}");
        assert!(text.contains("100,100,2000,2,0,L|200:100,1,140,0|2"), "{text}");
        assert_eq!(plan.left_out.len(), 1);
        assert!(plan.mix_files.staged.is_empty());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].time, 1000.0);
        assert_eq!(diagnostics[0].message, "At 1000 ms a slider sound (soft-sliderslide) is on a hit. osu! plays slider sounds only on a slider body. Remove it from the hit.");
    }

    /// The mix fails on a file that is in the folder but is not audio. A file that is not in the
    /// folder no longer fails it: osu! plays the normal hitnormal in its place (`Lane::plays_as`),
    /// and dawsu! ships that sound.
    #[test]
    fn a_failed_mix_render_stops_the_whole_save() {
        let scratch = Scratch::new("mix-render-fails");
        let (document, hits, project) = mix_and_unwritable(&scratch, "broken.wav");
        let mut diagnostics = Vec::new();
        let error = prepare(&scratch, &document, &hits, Some(&project), &mut diagnostics).err().unwrap();
        assert!(error.contains("Could not mix"), "{error}");
        assert_eq!(diagnostics.iter().map(|p| p.time).collect::<Vec<_>>(), [1000.0]);
        assert!(diagnostics[0].message.contains("Could not mix"));
    }

    fn wait(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Saves like Ctrl+S, confirming the Save hitsounds summary when Save leaves moments out.
    fn save_through_summary(app: &mut Application, backups: PathBuf) {
        app.save_with_backups(backups);
        wait(app);
        if app.save_problem_review() {
            app.answer_save_review(true);
            wait(app);
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    fn tail_id(hits: &Hitsounds) -> dawsu_hitsounding::MomentId {
        dawsu_hitsounding::MomentId(hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 1, .. })).unwrap())
    }

    /// HS mode with a soft and a drum clap on the slider's tail, which no object line can hold, and
    /// a finish on the circle, which Save can write.
    fn open_left_out_fixture(scratch: &Scratch, project: bool) -> (Application, PathBuf) {
        let folder = scratch.path("mapset");
        std::fs::create_dir_all(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        let mut app = Application::headless();
        if project {
            app.project_root = Some(scratch.path("projects"));
        }
        open(&mut app, map.clone());
        app.set_hs_mode(true);
        app.document.as_mut().unwrap().composer.apply("Draw claps", |hits| {
            let tail = tail_id(hits);
            hits.add_hit(tail, skin(SampleSet::Soft, Hitsound::Clap));
            hits.add_hit(tail, skin(SampleSet::Drum, Hitsound::Clap));
            hits.add_hit(dawsu_hitsounding::MomentId(0), skin(SampleSet::Normal, Hitsound::Finish));
        });
        (app, map)
    }

    fn line(bytes: &[u8], start: &str) -> String {
        String::from_utf8_lossy(bytes).lines().find(|line| line.starts_with(start)).unwrap().to_string()
    }

    #[test]
    fn ctrl_s_with_left_out_moments_shows_the_summary_and_its_save_writes_the_rest() {
        let scratch = Scratch::new("save-leaves-out");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.save_problem_review(), "Ctrl+S shows the Save hitsounds summary first");
        assert!(app.save_review_files().is_some(), "the summary's Save writes the reviewed plan");
        assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
        assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP, "nothing is written before the summary's Save");
        app.answer_save_review(true);
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        let saved = std::fs::read(&map).unwrap();
        assert_eq!(line(&saved, "100,100,2000,"), line(SLIDER_MAP, "100,100,2000,"), "the slider keeps its line");
        assert_ne!(line(&saved, "256,192,1000,"), line(SLIDER_MAP, "256,192,1000,"), "the circle's finish is written");
        assert!(!app.is_dirty());
        assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0], "the left-out moment stays listed");
        assert!(!app.save_problem_review());
    }

    #[test]
    fn written_follows_a_save_even_with_newer_edits_open_and_reproduces_the_file() {
        let scratch = Scratch::new("written-newer-revision");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        app.answer_save_review(true);
        assert!(app.busy);
        // A newer edit lands while the reviewed plan is written.
        app.document.as_mut().unwrap().composer.apply("Newer edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 55));
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.status.contains("earlier revision"), "{}", app.status);
        let doc = app.document().unwrap();
        let (again, problems, _) = compile_for_save(&doc.compile_base, &doc.written, true, &BTreeSet::new(), true, &|| false, &mut |_, _| {}).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(again.to_bytes(), std::fs::read(&map).unwrap(), "the hits as last written reproduce the file");
        assert!(!doc.written.moment(tail_id(&doc.written)).is_changed(), "the left-out tail is written as opened");
    }

    #[test]
    fn a_node_written_by_one_save_keeps_those_fields_when_the_next_save_leaves_it_out() {
        let scratch = Scratch::new("node-keeps-written");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        app.document.as_mut().unwrap().composer.apply("Drum clap only later", |hits| {
            hits.remove_hit(tail_id(hits), &skin(SampleSet::Drum, Hitsound::Clap));
        });
        save_through_summary(&mut app, scratch.path("backups"));
        let first = std::fs::read(&map).unwrap();
        assert_ne!(line(&first, "100,100,2000,"), line(SLIDER_MAP, "100,100,2000,"), "save 1 writes the soft clap");
        app.document.as_mut().unwrap().composer.apply("Drum clap", |hits| {
            hits.add_hit(tail_id(hits), skin(SampleSet::Drum, Hitsound::Clap));
        });
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(app.output_problems().len(), 1);
        assert_eq!(line(&std::fs::read(&map).unwrap(), "100,100,2000,"), line(&first, "100,100,2000,"));
    }

    /// Under a timing line with custom index 2, a circle cannot ask for the skin's index 0.
    const INDEXED_MAP: &[u8] = b"osu file format v14\r\n\r\n[General]\r\nSampleSet: Soft\r\n\r\n[TimingPoints]\r\n0,500,4,2,2,70,1,0\r\n\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";

    /// The project as it is on disk now.
    fn stored(app: &Application) -> project::SessionSettings {
        let path = &app.document().unwrap().path;
        project::Project::load(app.project_root.as_ref().unwrap(), None, path).unwrap().settings
    }

    fn record_json(app: &Application) -> serde_json::Value {
        serde_json::to_value(stored(app).unwritten).unwrap()
    }

    fn reopen(app: &Application, map: &Path) -> Application {
        let mut reopened = Application::headless();
        reopened.project_root = app.project_root.clone();
        open(&mut reopened, map.to_path_buf());
        reopened
    }

    fn tail_hits(app: &Application) -> Vec<(SampleIdentity, i32)> {
        let hits = app.document().unwrap().composer.hitsounds();
        let tail = hits.moment(tail_id(hits));
        tail.hits().map(|lane| (lane.clone(), tail.hit_volume(lane))).collect()
    }

    #[test]
    fn left_out_moments_are_kept_in_the_project_through_session_writes_and_come_back_on_reopen() {
        let scratch = Scratch::new("record-kept");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        let other = map.with_file_name("b.osu");
        std::fs::write(&other, SLIDER_MAP).unwrap();
        let drawn = tail_hits(&app);
        save_through_summary(&mut app, scratch.path("backups"));
        let record = record_json(&app);
        assert_eq!(record["current"]["moments"].as_array().map(Vec::len), Some(1), "{record}");
        assert_eq!(record["current"]["moments"][0]["object"], 1);
        assert_eq!(record["current"]["moments"][0]["node"], 1);
        // C1: session writes and a difficulty switch keep the record.
        app.zoom = 0.5;
        app.save_session().unwrap();
        open(&mut app, other.clone());
        open(&mut app, map.clone());
        assert_eq!(tail_hits(&app), drawn, "the drawn hits are back on the tail");
        let hits = app.document().unwrap().composer.hitsounds();
        assert!(hits.moment(tail_id(hits)).is_changed(), "the tail still differs from the file");
        assert!(!app.is_dirty());
        assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0], "listed before any Save");
        let reopened = reopen(&app, &map);
        assert_eq!(tail_hits(&reopened), drawn);
        assert_eq!(reopened.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
        assert_eq!(std::fs::read(&other).unwrap(), SLIDER_MAP);
    }

    #[test]
    fn a_digest_mismatch_puts_nothing_back_keeps_the_record_and_names_the_times() {
        let scratch = Scratch::new("record-mismatch");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        save_through_summary(&mut app, scratch.path("backups"));
        let record = record_json(&app);
        let mut edited = std::fs::read(&map).unwrap();
        edited.extend(b"// edited in another program\r\n");
        std::fs::write(&map, &edited).unwrap();
        let reopened = reopen(&app, &map);
        assert!(reopened.warnings.iter().any(|w| w.contains("changed outside dawsu!") && w.contains("00:02.500")), "{:?}", reopened.warnings);
        assert!(!tail_hits(&reopened).iter().any(|(lane, _)| *lane == skin(SampleSet::Drum, Hitsound::Clap)));
        assert_eq!(record_json(&reopened), record, "the record is kept until the next Save");
    }

    #[test]
    fn a_record_that_fails_its_checks_puts_nothing_back_warns_and_is_kept() {
        let scratch = Scratch::new("record-invalid");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        save_through_summary(&mut app, scratch.path("backups"));
        let path = app.project.as_ref().unwrap().path().to_path_buf();
        let reopened = reopen(&app, &map);
        let written = {
            let doc = reopened.document().unwrap();
            let tail = doc.written.moment(tail_id(&doc.written));
            tail.hits().map(|lane| (lane.clone(), tail.hit_volume(lane))).collect::<Vec<_>>()
        };
        assert_ne!(tail_hits(&reopened), written, "the untouched record puts the drawn hits back");
        for (field, value, reason) in [
            ("/settings/unwritten/current/moments/0/hits/0/volume", serde_json::json!(150), "volume 150 is out of range"),
            ("/settings/unwritten/current/moments/0/object", serde_json::json!(99), "an object is missing"),
            ("/settings/unwritten/current/moments/0/time_bits", serde_json::json!(1), "a moment moved"),
            ("/settings/unwritten/current/moments/0/hits/0/lane", serde_json::json!("Nope|x"), "unknown lane"),
        ] {
            let mut file: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let original = file.clone();
            *file.pointer_mut(field).unwrap() = value;
            std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
            let tampered = std::fs::read(&path).unwrap();
            let reopened = reopen(&app, &map);
            assert!(reopened.warnings.iter().any(|w| w.contains("could not be put back") && w.contains(reason)), "{reason}: {:?}", reopened.warnings);
            assert_eq!(tail_hits(&reopened), written, "{reason}: nothing is put back");
            assert!(reopened.output_problems().is_empty(), "{reason}: nothing is listed");
            assert_eq!(std::fs::read(&path).unwrap(), tampered, "{reason}: the record is kept");
            std::fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        }
    }

    #[test]
    fn a_failed_record_write_writes_no_osu() {
        let scratch = Scratch::new("record-stage-fails");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        app.save_session().unwrap();
        // The project's backup folder can't be made, so the staged record can't be written.
        std::fs::write(scratch.path("projects").join("backups"), b"not a folder").unwrap();
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        assert!(app.save_problem_review());
        app.answer_save_review(true);
        wait(&mut app);
        assert!(app.error.is_some());
        assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
        assert!(app.is_dirty());
    }

    #[test]
    fn a_failed_record_update_after_the_write_names_the_left_out_moments() {
        let scratch = Scratch::new("record-promote-fails");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        let drawn = tail_hits(&app);
        // No project file yet: the record is staged by creating it, and only the update after the
        // `.osu` write needs the (blocked) backup folder.
        std::fs::create_dir_all(scratch.path("projects")).unwrap();
        std::fs::write(scratch.path("projects").join("backups"), b"not a folder").unwrap();
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        app.answer_save_review(true);
        wait(&mut app);
        assert_ne!(std::fs::read(&map).unwrap(), SLIDER_MAP, "the difficulty is written");
        assert!(app.error.is_some());
        assert_eq!(app.status, "Difficulty saved. Left-out moments could not be recorded.");
        assert_eq!(tail_hits(&reopen(&app, &map)), drawn, "the staged record still puts them back");
    }

    #[test]
    fn a_crash_after_the_osu_write_restores_the_staged_record_on_reopen() {
        let scratch = Scratch::new("record-crash");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        let drawn = tail_hits(&app);
        save_through_summary(&mut app, scratch.path("backups"));
        // As if dawsu! stopped between the `.osu` write and the promote.
        let path = app.project.as_ref().unwrap().path().to_path_buf();
        let mut file: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let current = file["settings"]["unwritten"]["current"].take();
        file["settings"]["unwritten"] = serde_json::json!({
            "current": null, "pending": current, "before": dawsu_foundation::digest::sha256_hex(SLIDER_MAP),
        });
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(tail_hits(&reopen(&app, &map)), drawn);
    }

    #[test]
    fn fixing_the_moment_and_saving_clears_the_record() {
        let scratch = Scratch::new("record-cleared");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        save_through_summary(&mut app, scratch.path("backups"));
        app.document.as_mut().unwrap().composer.apply("Fix", |hits| {
            hits.remove_hit(tail_id(hits), &skin(SampleSet::Drum, Hitsound::Clap));
        });
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        assert!(!app.save_problem_review(), "nothing is left out, so Save writes straight away");
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(record_json(&app)["current"]["moments"], serde_json::json!([]));
        assert_ne!(line(&std::fs::read(&map).unwrap(), "100,100,2000,"), line(SLIDER_MAP, "100,100,2000,"));
        assert!(app.output_problems().is_empty());
    }

    #[test]
    fn saving_twice_with_a_left_out_moment_changes_neither_file() {
        let scratch = Scratch::new("record-twice");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        save_through_summary(&mut app, scratch.path("backups"));
        let project = app.project.as_ref().unwrap().path().to_path_buf();
        let (osu, json) = (std::fs::read(&map).unwrap(), std::fs::read(&project).unwrap());
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), osu);
        assert_eq!(std::fs::read(&project).unwrap(), json);
        let mut reopened = reopen(&app, &map);
        save_through_summary(&mut reopened, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), osu);
        assert_eq!(std::fs::read(&project).unwrap(), json);
    }

    #[test]
    fn a_body_volume_left_out_in_hs_mode_comes_back_with_its_head() {
        let scratch = Scratch::new("record-body");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.set_hs_mode(true);
        app.document.as_mut().unwrap().composer.apply("Body volume", |hits| hits.set_body_volume(0, 40));
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP, "HS mode can't add the green line a body volume needs");
        assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2000.0, 2000.0], "the body and its head are both listed");
        assert!(app.status.contains("Saved with backup. 2 parts left out."), "{}", app.status);
        let messages: Vec<_> = app.output_problems().into_iter().map(|p| p.message).collect();
        assert_eq!(messages.iter().filter(|m| m.contains("slider body volume")).count(), 1, "the body's row differs from its head's: {messages:?}");
        let record = record_json(&app);
        assert_eq!(record["current"]["bodies"][0]["volume"], 40, "{record}");
        let reopened = reopen(&app, &map);
        let hits = reopened.document().unwrap().composer.hitsounds();
        assert_eq!(hits.bodies()[0].volume(), 40);
        let head = hits.moments().iter().find(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
        assert_eq!(head.volume(), 40);
        let mut edited = std::fs::read(&map).unwrap();
        edited.extend(b"// edited in another program\r\n");
        std::fs::write(&map, &edited).unwrap();
        let changed = reopen(&app, &map);
        assert!(changed.warnings.iter().any(|w| w.contains("00:02.000") && w.contains("slider body volume")), "{:?}", changed.warnings);
    }

    #[test]
    fn per_hit_volumes_of_a_left_out_node_survive_reopen_and_the_next_save() {
        let scratch = Scratch::new("record-gains");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.set_hs_mode(true);
        app.document.as_mut().unwrap().composer.apply("Two volumes", |hits| {
            let tail = tail_id(hits);
            hits.add_hit(tail, skin(SampleSet::Soft, Hitsound::Clap));
            hits.set_hit_volume(tail, &skin(SampleSet::Soft, Hitsound::Clap), 40);
        });
        let drawn = tail_hits(&app);
        save_through_summary(&mut app, scratch.path("backups"));
        let mut reopened = reopen(&app, &map);
        assert_eq!(tail_hits(&reopened), drawn);
        let hits = reopened.document().unwrap().composer.hitsounds();
        assert!(hits.moment(tail_id(hits)).is_changed(), "gain recovery must not make the drawn gains the baseline");
        assert_eq!(reopened.output_problems().len(), 1, "still listed after reopen");
        save_through_summary(&mut reopened, scratch.path("backups"));
        assert_eq!(record_json(&reopened)["current"]["moments"].as_array().map(Vec::len), Some(1));
        assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
    }

    #[test]
    fn undo_after_restoring_a_recovery_copy_keeps_loose_hits() {
        let scratch = Scratch::new("recovery-undo-loose");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        app.data_root = scratch.path("data");
        open(&mut app, map.clone());
        app.document.as_mut().unwrap().composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
        save_through_summary(&mut app, scratch.path("backups"));
        app.document.as_mut().unwrap().composer.apply("Later edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 33));
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert!(restored.error.is_none(), "{:?}", restored.error);
        restored.command("undo");
        let loose = |app: &Application| app.document().unwrap().composer.hitsounds().moments().iter()
            .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject && m.hits().next().is_some()).count();
        assert_eq!(loose(&restored), 1, "the last Save's loose hit is still drawn after undo");
        assert!(!restored.is_dirty());
        let doc = restored.document().unwrap();
        assert!(doc.written.moments().iter().all(|m| m.place() != dawsu_hitsounding::Place::NoObject || m.hits().next().is_none()),
            "loose hits never count as written outside HS mode");
    }

    #[test]
    fn a_body_volume_left_out_in_normal_mode_keeps_the_head_written_beside_it() {
        let scratch = Scratch::new("record-body-green-lines");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.document.as_mut().unwrap().composer.apply("Body 40, head 60", |hits| {
            hits.set_body_volume(0, 40);
            let head = hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
            hits.set_volume(dawsu_hitsounding::MomentId(head), 60);
        });
        save_through_summary(&mut app, scratch.path("backups"));
        let saved = std::fs::read(&map).unwrap();
        let text = String::from_utf8_lossy(&saved);
        assert!(text.contains(",60,0,0\r\n"), "the head's green line is written:\n{text}");
        assert_eq!(line(&saved, "100,100,2000,"), line(SLIDER_MAP, "100,100,2000,"));
        let record = record_json(&app);
        assert_eq!(record["current"]["bodies"].as_array().map(Vec::len), Some(1), "{record}");
        assert_eq!(record["current"]["moments"], serde_json::json!([]), "{record}");
        assert_eq!(app.output_problems().len(), 1);
        let project = app.project.as_ref().unwrap().path().to_path_buf();
        let json = std::fs::read(&project).unwrap();
        let mut reopened = reopen(&app, &map);
        let hits = reopened.document().unwrap().composer.hitsounds();
        assert_eq!(hits.bodies()[0].volume(), 40);
        let head = hits.moments().iter().find(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
        assert_eq!(head.volume(), 60);
        assert_eq!(reopened.output_problems().len(), 1, "listed again on reopen");
        save_through_summary(&mut reopened, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), saved);
        assert_eq!(std::fs::read(&project).unwrap(), json);
    }

    #[test]
    fn a_recovery_copy_taken_after_a_partial_save_restores() {
        let scratch = Scratch::new("recovery-after-partial-save");
        let (mut app, _map) = open_left_out_fixture(&scratch, true);
        app.data_root = scratch.path("data");
        let drawn = tail_hits(&app);
        save_through_summary(&mut app, scratch.path("backups"));
        app.document.as_mut().unwrap().composer.apply("Later edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 33));
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert!(restored.error.is_none(), "{:?}", restored.error);
        let hits = restored.document().unwrap().composer.hitsounds();
        assert_eq!(hits.moment(dawsu_hitsounding::MomentId(0)).volume(), 33);
        assert_eq!(tail_hits(&restored), drawn);
        assert!(restored.is_dirty());
        let doc = restored.document().unwrap();
        assert!(!doc.written.moment(tail_id(&doc.written)).is_changed(), "what was last written comes from the copy's last Save");
    }

    /// Saves with the tail left out, edits on, and restores a recovery copy taken then.
    fn restore_after_partial_save(scratch: &Scratch) -> (Application, Vec<(SampleIdentity, i32)>) {
        let (mut app, _map) = open_left_out_fixture(scratch, true);
        app.data_root = scratch.path("data");
        let drawn = tail_hits(&app);
        save_through_summary(&mut app, scratch.path("backups"));
        app.document.as_mut().unwrap().composer.apply("Later edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 33));
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert!(restored.error.is_none(), "{:?}", restored.error);
        (restored, drawn)
    }

    #[test]
    fn undo_then_save_after_restoring_a_recovery_copy_keeps_the_left_out_hits() {
        let scratch = Scratch::new("recovery-undo-left-out");
        let (mut restored, drawn) = restore_after_partial_save(&scratch);
        restored.command("undo");
        assert_eq!(tail_hits(&restored), drawn, "Undo returns to the hits as drawn at the last Save");
        assert!(!restored.is_dirty());
        save_through_summary(&mut restored, scratch.path("backups"));
        assert_eq!(record_json(&restored)["current"]["moments"].as_array().map(Vec::len), Some(1), "the left-out tail is still kept");
        let map = restored.document().unwrap().path.clone();
        assert_eq!(tail_hits(&reopen(&restored, &map)), drawn);
    }

    #[test]
    fn a_restored_recovery_copy_lists_left_out_moments_straight_away() {
        let scratch = Scratch::new("recovery-lists-left-out");
        let (restored, _) = restore_after_partial_save(&scratch);
        assert_eq!(restored.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
    }

    #[test]
    fn an_old_recovery_copy_without_drawn_hits_still_restores() {
        let scratch = Scratch::new("recovery-old-format");
        let (mut app, _map) = open_left_out_fixture(&scratch, true);
        app.data_root = scratch.path("data");
        save_through_summary(&mut app, scratch.path("backups"));
        app.document.as_mut().unwrap().composer.apply("Later edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 33));
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        // A copy an earlier build wrote has no drawn hits for its last Save.
        let path = scratch.path("data").join("recovery").join(&offer.id);
        let mut copy: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        copy["checkpoint"]["last_save"].as_object_mut().unwrap().remove("drawn");
        std::fs::write(&path, serde_json::to_vec(&copy).unwrap()).unwrap();
        let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert!(restored.error.is_none(), "{:?}", restored.error);
        assert_eq!(restored.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).volume(), 33);
    }

    #[test]
    fn a_recovery_copy_preserves_per_hit_volumes() {
        let scratch = Scratch::new("recovery-per-hit-volumes");
        let (mut app, _map) = open_left_out_fixture(&scratch, true);
        app.data_root = scratch.path("data");
        let clap = skin(SampleSet::Normal, Hitsound::Clap);
        app.document.as_mut().unwrap().composer.apply("Quiet clap", |hits| {
            let m = dawsu_hitsounding::MomentId(0);
            hits.set_volume(m, 100);
            hits.add_hit(m, clap.clone());
            hits.set_hit_volume(m, &clap, 30);
        });
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        let mut restored = Application::create(Preferences::default(), false, None, None, false, scratch.path("data"));
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert!(restored.error.is_none(), "{:?}", restored.error);
        let moment = restored.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(0));
        assert_eq!(moment.volume(), 100);
        assert_eq!(moment.hit_volume(&clap), 30);
    }

    #[test]
    fn an_unrelated_edit_keeps_a_left_out_moment_listed_and_editing_it_drops_it() {
        let scratch = Scratch::new("left-out-listing");
        let (mut app, _map) = open_left_out_fixture(&scratch, true);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(app.output_problems().len(), 1);
        app.document.as_mut().unwrap().composer.apply("Unrelated", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 30));
        assert_eq!(app.output_problems().len(), 1, "an unrelated edit keeps it listed");
        app.document.as_mut().unwrap().composer.apply("Edit the tail", |hits| {
            let tail = tail_id(hits);
            hits.set_hit_volume(tail, &skin(SampleSet::Drum, Hitsound::Clap), 20);
        });
        assert!(app.output_problems().is_empty(), "editing the moment drops it until the next Save");
        app.document.as_mut().unwrap().composer.undo();
        assert_eq!(app.output_problems().len(), 1, "undo brings back what was left out");
        app.set_hs_mode(false);
        assert!(app.output_problems().is_empty(), "the list belongs to the HS mode it was saved in");
    }

    #[test]
    fn a_generate_review_keeps_the_left_out_moments_listed() {
        let scratch = Scratch::new("left-out-generate");
        let (mut app, _map) = open_left_out_fixture(&scratch, true);
        save_through_summary(&mut app, scratch.path("backups"));
        app.generate();
        wait(&mut app);
        assert!(app.output_problems().iter().any(|p| p.time == 2500.0), "the badge stays");
    }

    #[test]
    fn an_hs_group_never_written_makes_no_circle_and_comes_back_as_loose_hits() {
        let scratch = Scratch::new("record-loose-group");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        let other = folder.join("b.osu");
        std::fs::write(&map, INDEXED_MAP).unwrap();
        std::fs::write(&other, INDEXED_MAP).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.set_hs_mode(true);
        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Normal), 80, 0.0);
        composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), INDEXED_MAP, "no 256,192 placeholder circle");
        assert_eq!(stored(&app).unanchored.map(|rows| rows.len()), Some(2));
        app.zoom = 0.5;
        app.save_session().unwrap();
        open(&mut app, other);
        open(&mut app, map.clone());
        let reopened = reopen(&app, &map);
        for app in [&app, &reopened] {
            let loose: Vec<_> = app.document().unwrap().composer.hitsounds().moments().iter()
                .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject).collect();
            assert_eq!(loose.len(), 1);
            assert_eq!(loose[0].hits().count(), 2);
            assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [3000.0]);
        }
    }

    #[test]
    fn an_hs_group_written_before_and_left_out_later_keeps_one_circle() {
        let scratch = Scratch::new("record-written-group");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, INDEXED_MAP).unwrap();
        wav(&folder.join("voice.wav"), 8192);
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.set_hs_mode(true);
        app.document.as_mut().unwrap().composer.add(3000.0, SampleIdentity::File("voice.wav".into()), 80, 0.0);
        save_through_summary(&mut app, scratch.path("backups"));
        let first = std::fs::read(&map).unwrap();
        assert_eq!(app.document().unwrap().file.hit_objects().len(), 2);
        app.document.as_mut().unwrap().composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), first, "the circle keeps its last written hits");
        assert_eq!(record_json(&app)["current"]["moments"][0]["object"], 1);
        let mut reopened = reopen(&app, &map);
        let doc = reopened.document().unwrap();
        assert_eq!(doc.file.hit_objects().len(), 2);
        let circle = doc.composer.hitsounds().moments().iter().find(|m| m.time().ms() == 3000.0).unwrap();
        assert_eq!(circle.hits().count(), 2, "the drawn hits are back on the one circle");
        assert_eq!(project_hits(doc).len(), 0, "and not as loose hits too");
        save_through_summary(&mut reopened, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), first);
    }

    #[test]
    fn loose_hits_saved_outside_hs_mode_never_count_as_a_written_circle() {
        let scratch = Scratch::new("loose-never-a-circle");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, INDEXED_MAP).unwrap();
        wav(&folder.join("voice.wav"), 8192);
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.document.as_mut().unwrap().composer.add(3000.0, SampleIdentity::File("voice.wav".into()), 80, 0.0);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), INDEXED_MAP, "outside HS mode a loose hit stays in the project");
        app.set_hs_mode(true);
        app.document.as_mut().unwrap().composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), INDEXED_MAP, "the group was never a circle, so none is made");
        assert_eq!(app.output_problems().len(), 1);
    }

    #[test]
    fn cancel_or_show_in_the_summary_drops_the_pending_save() {
        let scratch = Scratch::new("summary-cancel");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        app.review_save();
        wait(&mut app);
        assert!(app.save_problem_review());
        assert!(app.save_review_files().is_some());
        app.dismiss_save_problem_review();
        assert!(!app.save_problem_review());
        assert!(app.save_review_files().is_none(), "Review Save must not pop up straight after");
        app.answer_save_review(true);
        wait(&mut app);
        assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
        assert!(app.is_dirty());
    }

    #[test]
    fn without_a_project_save_still_refuses_to_leave_moments_out() {
        let scratch = Scratch::new("no-project-refuses");
        let (mut app, map) = open_left_out_fixture(&scratch, false);
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.as_deref().is_some_and(|e| e.contains("cannot be written")), "{:?}", app.error);
        assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
        assert!(app.is_dirty());
    }

    #[test]
    fn a_pending_open_save_shows_the_summary_first_and_opens_after_its_save() {
        let scratch = Scratch::new("pending-open-summary");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        let other = map.with_file_name("b.osu");
        std::fs::write(&other, SLIDER_MAP).unwrap();
        app.request_open(other.clone());
        assert!(app.pending_open.is_some());
        app.save();
        wait(&mut app);
        assert!(app.error.is_none() && app.is_dirty() && app.save_problem_review(), "{:?}", app.error);
        assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP, "nothing is written before the summary's Save");
        assert!(app.pending_open.is_some());
        app.answer_save_review(true);
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.document().unwrap().path, other.canonicalize().unwrap(), "the pending difficulty opens");
    }

    #[test]
    fn editing_a_left_out_hs_group_drops_it_from_the_list_and_undo_brings_it_back() {
        let scratch = Scratch::new("left-out-group-listing");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, INDEXED_MAP).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.set_hs_mode(true);
        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Normal), 80, 0.0);
        composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Clap), 80, 0.0);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [3000.0]);
        app.document.as_mut().unwrap().composer.add(3000.0, skin(SampleSet::Soft, Hitsound::Finish), 80, 0.0);
        assert!(app.output_problems().is_empty(), "the group no longer holds what Save left out");
        app.document.as_mut().unwrap().composer.undo();
        assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [3000.0]);
    }

    #[test]
    fn a_left_out_tail_beside_a_written_head_green_line_comes_back_on_reopen() {
        let scratch = Scratch::new("left-out-tail-green-line");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.document.as_mut().unwrap().composer.apply("Head 60, empty tail", |hits| {
            let head = hits.moments().iter().position(|m| matches!(m.place(), dawsu_hitsounding::Place::SliderNode { node: 0, .. })).unwrap();
            hits.set_volume(dawsu_hitsounding::MomentId(head), 60);
            let tail = tail_id(hits);
            let lanes: Vec<_> = hits.moment(tail).hits().cloned().collect();
            for lane in lanes {
                hits.remove_hit(tail, &lane);
            }
        });
        let drawn = tail_hits(&app);
        save_through_summary(&mut app, scratch.path("backups"));
        let saved = std::fs::read(&map).unwrap();
        assert_ne!(saved, SLIDER_MAP, "the head's green line is written");
        assert_eq!(app.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
        let reopened = reopen(&app, &map);
        assert!(!reopened.warnings.iter().any(|w| w.contains("left out")), "{:?}", reopened.warnings);
        assert_eq!(tail_hits(&reopened), drawn);
        assert_eq!(reopened.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
    }

    #[test]
    fn an_hs_circle_made_before_a_left_out_tail_shifts_its_record_ordinal_and_reopen_matches() {
        let scratch = Scratch::new("left-out-ordinal-shift");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        app.document.as_mut().unwrap().composer.add(500.0, skin(SampleSet::Normal, Hitsound::Normal), 80, 0.0);
        let drawn = tail_hits(&app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(app.document().unwrap().file.hit_objects().len(), 3);
        assert_eq!(record_json(&app)["current"]["moments"][0]["object"], 2);
        let mut reopened = reopen(&app, &map);
        assert!(!reopened.warnings.iter().any(|w| w.contains("left out")), "{:?}", reopened.warnings);
        assert_eq!(tail_hits(&reopened), drawn);
        assert_eq!(reopened.output_problems().iter().map(|p| p.time).collect::<Vec<_>>(), [2500.0]);
        let before = std::fs::read(&map).unwrap();
        save_through_summary(&mut reopened, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), before);
    }

    #[test]
    fn an_emptied_circle_is_left_out_and_comes_back_empty_on_reopen() {
        let scratch = Scratch::new("left-out-empty-circle");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        std::fs::write(&map, SLIDER_MAP).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        app.document.as_mut().unwrap().composer.apply("Empty the circle", |hits| {
            let lanes: Vec<_> = hits.moment(dawsu_hitsounding::MomentId(0)).hits().cloned().collect();
            for lane in lanes {
                hits.remove_hit(dawsu_hitsounding::MomentId(0), &lane);
            }
        });
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(&map).unwrap(), SLIDER_MAP);
        assert_eq!(app.output_problems().len(), 1);
        let reopened = reopen(&app, &map);
        assert!(!reopened.warnings.iter().any(|w| w.contains("left out")), "{:?}", reopened.warnings);
        assert_eq!(reopened.document().unwrap().composer.hitsounds().moment(dawsu_hitsounding::MomentId(0)).hits().count(), 0);
        assert_eq!(reopened.output_problems().len(), 1);
    }

    #[test]
    fn save_and_save_check_publish_object_progress() {
        let fixture = Scratch::new("save-progress");
        let original = b"osu file format v14\n[General]\nMode:0\n[HitObjects]\n256,192,1000,1,0,0:0:0:50:\n256,192,2000,1,0,0:0:0:60:\n";
        for check_only in [true, false] {
            let path = fixture.path(if check_only { "checked.osu" } else { "saved.osu" });
            std::fs::write(&path, original).unwrap();
            let mut app = Application::headless();
            open(&mut app, path.clone());
            app.document.as_mut().unwrap().composer.apply("Change both objects", |hits| {
                hits.set_volume(dawsu_hitsounding::MomentId(0), 75);
                hits.set_volume(dawsu_hitsounding::MomentId(1), 80);
            });
            if check_only {
                app.review_save();
            } else {
                app.save_with_backups(fixture.path("backups"));
            }
            assert!(app.busy);
            let progress = app.job_progress.as_ref().expect("Save should track object progress");
            let result = app.jobs.as_ref().unwrap().recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(matches!(result, JobResult::SaveCheck { result: Ok(_), .. } | JobResult::Save { result: Ok(_), .. }));
            assert_eq!(*progress.current.lock().unwrap(), Some((2, 2)));
            assert_eq!(std::fs::read(path).unwrap() == original, check_only);
        }
    }

    #[test]
    fn review_progress_reaches_status_while_worker_is_active() {
        let fixture = Scratch::new("review-progress");
        let path = fixture.path("map.osu");
        std::fs::write(&path, b"osu file format v14\n[General]\nMode:0\n[HitObjects]\n").unwrap();
        let mut app = Application::headless();
        open(&mut app, path);
        let (release, wait) = mpsc::channel();
        let (reported, ready) = mpsc::channel();
        app.review_job(move |_, _, _, progress| {
            progress(2, 5);
            reported.send(()).unwrap();
            wait.recv().unwrap();
            Err("test complete".into())
        });
        ready.recv_timeout(Duration::from_secs(3)).unwrap();
        app.poll();
        assert!(app.busy);
        assert!(app.status.contains("2 / 5"), "{}", app.status);
        release.send(()).unwrap();
    }

    #[test]
    fn regenerate_checks_owned_output_reuses_mix_and_backs_up_replacement() {
        let fixture = Scratch::new("checked-regenerate");
        let source = fixture.path("source.osu");
        let projects = fixture.path("sessions");
        let backups = fixture.path("backups");
        let body = b"osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Song\nCreator:Mapper\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:50:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n";
        std::fs::write(&source, body).unwrap();
        for (name, amplitude) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
            wav.extend(amplitude.to_le_bytes());
            std::fs::write(fixture.path(name), wav).unwrap();
        }
        let wait = |app: &mut Application| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let mut app = Application::headless();
        app.project_root = Some(projects.clone());
        open(&mut app, source.clone());
        app.generate();
        wait(&mut app);
        let review = app.output_review.as_ref().unwrap();
        let target = review.files[0].path.clone();
        let mix = review.mix_paths().next().unwrap().to_path_buf();
        assert!(!target.exists() && !mix.exists());
        app.apply_output_in(backups.clone());
        wait(&mut app);
        assert!(app.error.is_none());
        let initial = std::fs::read(&target).unwrap();
        let rendered = std::fs::read(&mix).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(projects);
        open(&mut app, source.clone());
        app.document.as_mut().unwrap().composer.add(1500.0, SampleIdentity::File("one.wav".into()), 60, 1.0);
        app.generate();
        wait(&mut app);
        let replacement = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        assert_eq!(replacement.files[0].expected.as_deref(), Some(initial.as_slice()));
        assert_eq!(replacement.mix_paths().collect::<Vec<_>>(), [mix.as_path()]);
        assert_eq!(std::fs::read(&target).unwrap(), initial);
        app.apply_output_in(backups.clone());
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_ne!(std::fs::read(&target).unwrap(), initial);
        assert_eq!(std::fs::read(&mix).unwrap(), rendered);
        let backup_dir = std::fs::read_dir(&backups).unwrap().next().unwrap().unwrap().path();
        let backup = std::fs::read_dir(backup_dir).unwrap().next().unwrap().unwrap().path();
        assert_eq!(std::fs::read(backup).unwrap(), initial);
        std::fs::write(&target, b"Another editor changed the output").unwrap();
        app.generate();
        wait(&mut app);
        assert!(app.output_review.is_none());
        assert!(app.error.as_deref().unwrap().contains("changed since Generate"));
        assert_eq!(std::fs::read(&target).unwrap(), b"Another editor changed the output");
        assert_eq!(std::fs::read(&source).unwrap(), body);
    }

    #[test]
    fn generate_uses_legacy_rule_to_pair_incompatible_roles_after_reopen() {
        let fixture = Scratch::new("generate-paired-preset");
        let map = fixture.path("Artist - Song (Mapper) [Hard].osu");
        let body = b"osu file format v14\n[General]\nMode:0\n[Metadata]\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,1:0:2:50:\n";
        std::fs::write(&map, body).unwrap();
        let mut layers = Vec::new();
        for (name, amplitude) in [("normal-hitnormal2.wav", 8192_i16), ("soft-hitclap3.wav", 4096), ("drum-hitfinish4.wav", 16384)] {
            let mut wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0".to_vec();
            wav.extend(amplitude.to_le_bytes());
            std::fs::write(fixture.path(name), &wav).unwrap();
            let hash = dawsu_foundation::digest::sha256_hex(&wav).to_ascii_uppercase();
            layers.push(format!("hit:BeatmapFile:{name}@{hash}@balance:1"));
        }
        layers.sort();
        let key = serde_json::to_string(&layers).unwrap();
        let mut settings = ExportSettings::default();
        settings.pair_samples = true;
        settings.rules.insert(key, "soft-hitwhistle".into());
        let path = fixture.path("merge-preset.json");
        let export = serde_json::to_vec_pretty(&settings).unwrap();
        std::fs::write(&path, &export).unwrap();
        let loaded = ExportSettings::read_file(&path).unwrap();
        assert_eq!(loaded, settings);
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        open(&mut app, map.clone());
        app.set_export_settings(loaded).unwrap();
        app.save_session().unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        open(&mut app, map.clone());
        assert_eq!(app.document().unwrap().export_settings(), Some(&settings));
        let d = app.document.as_mut().unwrap();
        for (set, sound, index) in [(SampleSet::Soft, Hitsound::Clap, 3), (SampleSet::Drum, Hitsound::Finish, 4)] {
            d.composer.add(1000.0, SampleIdentity::Standard { set, sound: Sound::Hit(sound), index: dawsu_foundation::CustomIndex::new(index) }, 50, 1.0);
        }
        app.generate();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let plan = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        let names = plan.mix_paths().map(|path| path.file_name().unwrap().to_str().unwrap().to_owned()).collect::<Vec<_>>();
        assert_eq!(names, ["normal-hitnormal5.wav", "soft-hitfinish5.wav", "soft-hitclap5.wav"]);
        assert!(plan.mix_paths().all(|path| !path.exists()));
        assert!(!plan.files[0].path.exists());
        let target = plan.files[0].path.clone();
        app.apply_output_in(fixture.path("backups"));
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        let reopened = Document::read(&target).unwrap();
        let moment = &reopened.composer.hitsounds().moments()[0];
        assert_eq!(moment.volume(), 100);
        assert_eq!(moment.hits().count(), 3);
        for (file, amplitude) in names.iter().zip([0.125, 0.25, 0.0625]) {
            assert_eq!(dawsu_audio::decode(&fixture.path(file)).unwrap().samples, [amplitude; 2]);
        }
        app.generate();
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let replacement = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        assert_eq!(replacement.mix_paths().map(|path| path.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(), names);
        assert_eq!(std::fs::read(&map).unwrap(), body);
        assert_eq!(std::fs::read(&path).unwrap(), export);
    }

    #[test]
    fn bookmark_commands_navigate_imported_and_new_bookmarks_per_difficulty() {
        let fixture = Scratch::new("bookmark-commands");
        let first = fixture.path("first.osu");
        let second = fixture.path("second.osu");
        let map = include_str!("../tests/project-fixture.osu");
        let original = map.replace("[Metadata]", "[Editor]\nBookmarks: 100,500\n[Metadata]");
        std::fs::write(&first, &original).unwrap();
        std::fs::write(&second, map).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        open(&mut app, first.clone());

        app.seek(0.0);
        app.command("nextBookmark");
        assert_eq!(app.position_ms(), 100.0, "next bookmark should navigate to the first map bookmark");
        // The Inspector's pick follows the step: the editor names the bookmark it jumped to, once.
        assert_eq!(app.bookmark_step.take(), Some(100.0));
        app.seek(1500.0);
        app.command("nextBookmark");
        assert_eq!(app.bookmark_step, None, "a step with no bookmark to go to names none");
        app.seek(750.0);
        app.command("addBookmark");
        app.command("previousBookmark");
        assert_eq!(app.position_ms(), 500.0);
        app.command("nextBookmark");
        assert_eq!(app.position_ms(), 750.0);
        app.rename_bookmark(750.0, "Chorus".into()).unwrap();
        app.seek(1250.0);
        app.add_bookmark(Some("Break".into())).unwrap();
        open(&mut app, second.clone());
        app.seek(0.0);
        app.command("nextBookmark");
        assert_eq!(app.position_ms(), 0.0);
        open(&mut app, first.clone());
        let marks = app.document().unwrap().bookmarks();
        assert_eq!(marks[2].label.as_deref(), Some("Chorus"));
        assert_eq!(marks[3].label.as_deref(), Some("Break"));
        app.seek(500.0);
        app.command("nextBookmark");
        assert_eq!(app.position_ms(), 750.0);
        app.rename_bookmark(750.0, " ".into()).unwrap();
        assert_eq!(app.document().unwrap().bookmarks()[2].label, None);
        app.command("removeBookmark");
        for time in [100.0, 500.0, 1250.0] {
            app.remove_bookmark(time).unwrap();
        }
        open(&mut app, first.clone());
        assert!(app.document().unwrap().bookmarks().is_empty(), "the saved empty list overrides imported marks");
        assert_eq!(std::fs::read(&first).unwrap(), original.as_bytes());
        assert_eq!(std::fs::read(&second).unwrap(), map.as_bytes());
    }

    #[test]
    fn add_lane_registers_a_new_sample_before_the_watcher_runs() {
        let fixture = Scratch::new("add-lane-immediate");
        let path = fixture.path("first.osu");
        let map = b"osu file format v14\n[General]\nAudioFilename: song.wav\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
        std::fs::write(&path, map).unwrap();
        let mut app = Application::headless();
        open(&mut app, path.clone());
        let path = path.canonicalize().unwrap();
        let lane = SampleIdentity::File("late.WAV".into());
        std::fs::write(fixture.path("late.WAV"), wav).unwrap();
        std::fs::write(fixture.path("song.wav"), wav).unwrap();
        assert!(!app.document().unwrap().composer.lanes.contains(&lane));
        assert_eq!(app.scan_lane_add_candidates(&path).unwrap(), [lane.clone()]);
        // No poll occurs between copy and explicit Add.
        app.show_lane(&path, &lane).unwrap();
        let doc = app.document().unwrap();
        assert_eq!(doc.composer.lanes[app.active_lane], lane);
        assert!(app.samples.contains_key(&lane));
        assert!(!app.is_dirty());
        assert!(app.scan_lane_add_candidates(&path).unwrap().is_empty());
        app.command("undo");
        assert!(app.document().unwrap().composer.hitsounds().is_lane_hidden(&lane));
        assert_eq!(app.scan_lane_add_candidates(&path).unwrap(), [lane.clone()]);
        app.command("redo");
        assert!(!app.document().unwrap().composer.hitsounds().is_lane_hidden(&lane));
        assert_eq!(std::fs::read(&path).unwrap(), map);
        assert_eq!(std::fs::read(fixture.path("late.WAV")).unwrap(), wav);
    }

    #[test]
    fn deleting_a_lane_preserves_its_file_and_supports_undo_reopen_and_add() {
        let fixture = Scratch::new("delete-lane");
        let path = fixture.path("first.osu");
        let other = fixture.path("second.osu");
        let sample = fixture.path("soft-hitclap.wav");
        let original = b"osu file format v14\r\n[General]\r\nMode:0\r\n[TimingPoints]\r\n0,500,4,2,1,75,1,0\r\n[HitObjects]\r\n256,192,1000,1,8,0:0:0:75:\r\n256,192,2000,1,0,0:0:0:50:\r\n";
        let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
        std::fs::write(&sample, wav).unwrap();
        std::fs::write(&path, original).unwrap();
        std::fs::write(&other, original).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("projects"));
        open(&mut app, path.clone());
        let path = path.canonicalize().unwrap();
        let lane = dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap.wav").unwrap();
        let protected = app.document().unwrap().composer.hitsounds().moments()[1].hits().next().unwrap().clone();
        app.set_normal_protection(true);
        let revision = app.document().unwrap().composer.history.version();
        assert!(app.delete_lane(&path, &protected, revision).is_err());
        assert!(app.delete_lane(&path, &lane, revision + 1).is_err());
        assert!(!app.is_dirty());

        app.active_lane = app.document().unwrap().composer.lanes.iter().position(|row| row == &lane).unwrap();
        let hidden = |app: &Application| app.document().unwrap().composer.hitsounds().is_lane_hidden(&lane);
        let has_hit = |app: &Application| app.document().unwrap().composer.hitsounds().moments()[0].has_hit(&lane);
        app.delete_lane(&path, &lane, revision).unwrap();
        assert!(hidden(&app));
        assert!(!has_hit(&app));
        assert_ne!(app.document().unwrap().composer.lanes[app.active_lane], lane);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        // Simulates a watcher payload that arrives after Delete, including after a later rescan.
        app.register_added_samples(vec![("soft-hitclap.wav".into(), dawsu_audio::decode(&sample).unwrap())]);
        assert!(hidden(&app));
        app.command("undo");
        assert!(!hidden(&app) && has_hit(&app));
        app.command("redo");
        assert!(hidden(&app) && !has_hit(&app));

        app.save_with_backups(fixture.path("backups"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.is_dirty());
        open(&mut app, other.clone());
        assert!(!hidden(&app) && has_hit(&app));
        open(&mut app, path.clone());
        assert!(hidden(&app) && !has_hit(&app));
        assert!(app.lane_add_candidates().contains(&lane));
        app.show_lane(&path, &lane).unwrap();
        assert!(!hidden(&app) && !has_hit(&app));
        assert_eq!(app.document().unwrap().composer.lanes[app.active_lane], lane);
        assert!(!app.is_dirty(), "showing an empty lane never dirties the map");
        app.command("undo");
        assert!(hidden(&app));
        app.command("redo");
        assert!(!hidden(&app));
        assert_eq!(std::fs::read(sample).unwrap(), wav);
        assert_eq!(std::fs::read(other).unwrap(), original);
    }

    #[test]
    fn lane_layout_is_read_per_difficulty_without_writing_the_map() {
        let fixture = Scratch::new("lane-layout");
        let path = fixture.path("first.osu");
        let other = fixture.path("second.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&path, original).unwrap();
        std::fs::write(&other, original).unwrap();
        let projects = fixture.path("projects");
        let mut app = Application::headless();
        app.project_root = Some(projects.clone());
        let mut project = project::Project::load(&projects, None, &path.canonicalize().unwrap()).unwrap();
        let settings = project::SessionSettings {
            lane_layout: Some(project::LaneLayout {
                folder_order: vec!["drum".into(), "normal".into(), "soft".into()],
                collapsed_folders: ["normal".into()].into(),
                collapsed_families: ["soft-hitclap".into()].into(),
                lane_colours: Default::default(),
                ..Default::default()
            }),
            ..Default::default()
        };
        project.save(settings.clone()).unwrap();
        open(&mut app, path.clone());
        assert_eq!(app.document().unwrap().lane_layout(), settings.lane_layout.as_ref());
        let updated = LaneLayout {
            folder_order: vec!["soft".into(), "drum".into(), "normal".into()],
            collapsed_folders: ["drum".into()].into(),
            collapsed_families: ["normal-hitfinish".into()].into(),
            lane_colours: [("Skin|normal-hitclap".into(), "#e65420".into())].into(),
            lane_names: [("Skin|normal-hitclap".into(), "Backup snare".into())].into(),
            lane_order: vec!["Skin|normal-hitclap".into(), "Skin|normal-hitnormal".into()],
        };
        app.set_lane_layout(updated.clone()).unwrap();
        app.save_session().unwrap();
        open(&mut app, other);
        assert!(app.document().unwrap().lane_layout().is_none());
        app.save_session().unwrap();
        open(&mut app, path.clone());
        assert_eq!(app.document().unwrap().lane_layout(), Some(&updated));
        assert_eq!(std::fs::read(path).unwrap().as_slice(), original);
    }

    #[test]
    fn explicit_play_start_keeps_the_return_point_when_seeking() {
        let fixture = Scratch::new("play-start");
        let path = fixture.path("test.osu");
        std::fs::write(&path, include_bytes!("../tests/project-fixture.osu")).unwrap();
        let mut app = Application::headless();
        app.document = Some(Document::read(&path).unwrap());
        app.seek(700.0);
        app.play_pause_from(Some(500.0));
        assert_eq!(app.position_ms(), 500.0);
        assert_eq!(app.return_position, 500.0);
        app.seek(850.0);
        assert_eq!(app.return_position, 500.0);
        app.play_pause_from(None);
        assert_eq!(app.return_position, 850.0);
    }

    #[test]
    fn replacing_a_lane_keeps_the_slot_and_saves_a_playable_new_backing_sample() {
        let scratch = Scratch::new("lane-replacement");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("test.osu");
        let sibling = folder.join("other.osu");
        let original_hit = "256,192,1000,1,0,1:1:1:67:";
        let unrelated_hit = "320,192,1250,1,8,2:2:0:91:";
        let source = include_str!("../tests/project-fixture.osu")
            .replace("256,192,1000,1,8,0:0:0:0:", &format!("{original_hit}\n{unrelated_hit}"));
        let sibling_bytes = source.replace(original_hit, "256,192,1000,1,0,1:1:3:67:");
        std::fs::write(&map, &source).unwrap();
        std::fs::write(&sibling, &sibling_bytes).unwrap();
        let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\x00\x40";
        let mut quieter = wav.to_vec();
        *quieter.last_mut().unwrap() = 0x20;
        let external = scratch.path("replacement.wav");
        let backing = folder.join("normal-hitnormal4.wav");
        std::fs::write(&external, &quieter).unwrap();
        for name in ["normal-hitnormal.wav", "normal-hitnormal2.wav"] {
            std::fs::write(folder.join(name), wav).unwrap();
        }

        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        let old = dawsu_hitsounding::rules::read_sample_file_name("normal-hitnormal.wav").unwrap();
        let new = dawsu_hitsounding::rules::read_sample_file_name("normal-hitnormal4.wav").unwrap();
        app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &old).unwrap();
        let map_key = app.document().unwrap().path.clone();
        assert!(app.replace_lane_sample(&sibling, &old, &external).is_err());
        assert!(app.replace_lane_sample(&map_key, &new, &external).is_err());
        assert!(!backing.exists());
        app.replace_lane_sample(&map_key, &old, &external).unwrap();
        let composer = &app.document().unwrap().composer;
        assert_eq!(composer.lanes[app.active_lane], new);
        assert_eq!(composer.hitsounds().lane_slot(&new), Some(&old));
        assert!(composer.hitsounds().moments()[0].hits().any(|sample| sample == &new));
        assert_eq!(composer.hitsounds().moments()[0].volume(), 67);
        assert!(app.is_dirty());
        assert_eq!(app.clip_for_lane(&new).unwrap().samples[0], 0.25);
        // The new backing sample is a pending file until a Save plays it.
        assert!(!backing.exists());
        assert_eq!(app.document().unwrap().pending.get("normal-hitnormal4.wav").map(|file| std::fs::read(&file.path).unwrap()), Some(quieter.clone()));
        assert_eq!(std::fs::read(&map).unwrap(), source.as_bytes());

        app.command("undo");
        let composer = &app.document().unwrap().composer;
        assert_eq!(composer.lanes[app.active_lane], old);
        assert!(composer.hitsounds().moments()[0].hits().any(|sample| sample == &old));
        assert!(composer.hitsounds().lane_slot(&new).is_none());
        assert!(!app.is_dirty());
        app.command("redo");
        assert_eq!(app.document().unwrap().composer.lanes[app.active_lane], new);

        app.save_with_backups(scratch.path("backups"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.is_dirty());
        let saved = std::fs::read(&map).unwrap();
        assert!(saved.windows(unrelated_hit.len()).any(|window| window == unrelated_hit.as_bytes()));
        assert_eq!(std::fs::read(&sibling).unwrap(), sibling_bytes.as_bytes());
        assert_eq!(std::fs::read(&external).unwrap(), quieter);
        assert_eq!(std::fs::read(folder.join("normal-hitnormal.wav")).unwrap(), wav);
        // The Save wrote the new backing sample because the difficulty plays it.
        assert_eq!(std::fs::read(&backing).unwrap(), quieter);


        let mut reopened = Application::headless();
        reopened.project_root = Some(scratch.path("projects"));
        open(&mut reopened, map);
        let hits = reopened.document().unwrap().composer.hitsounds();
        assert_eq!(hits.lane_slot(&new), Some(&old));
        assert!(hits.moments()[0].hits().any(|sample| sample == &new));
        assert_eq!(reopened.clip_for_lane(&new).unwrap().samples[0], 0.25);
    }

    #[test]
    #[allow(clippy::disallowed_methods)]
    fn add_numbered_layers_skips_reserved_indexes_and_preserves_mapset_files() {
        let scratch = Scratch::new("numbered-layers");
        let folder = scratch.path("mapset");
        let defaults = scratch.path("skin");
        for directory in [&folder, &defaults] {
            std::fs::create_dir(directory).unwrap();
        }
        let map = folder.join("test.osu");
        let sibling = folder.join("other.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        let other = include_str!("../tests/project-fixture.osu")
            .replace("256,192,1000,1,8,0:0:0:0:", "256,192,1000,1,8,1:2:3:50:");
        std::fs::write(&map, original).unwrap();
        std::fs::write(&sibling, &other).unwrap();
        let tone = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
        let occupied = folder.join("SOFT-HITCLAP2.wav");
        std::fs::write(&occupied, tone).unwrap();
        std::fs::write(folder.join("drum-hitfinish7.wav"), tone).unwrap();
        let default_file = defaults.join("soft-hitclap.wav");
        std::fs::write(&default_file, tone).unwrap();

        let mut app = Application::headless();
        app.preferences.default_samples = defaults;
        open(&mut app, map.clone());
        let family = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &family).unwrap();
        let before = dawsu_osu_format::list_folder(&folder).unwrap();
        app.command("addNumberedLayer");
        assert!(app.error.is_none(), "{:?}", app.error);
        // A new layer is a pending file: the mapset holds nothing new until a Save plays it.
        let first = folder.join("soft-hitclap8.wav");
        assert!(!first.exists());
        assert_eq!(app.document().unwrap().pending.get("soft-hitclap8.wav").map(|file| std::fs::read(&file.path).unwrap()), Some(tone.to_vec()));

        let mut quieter = tone.to_vec();
        *quieter.last_mut().unwrap() = 0x20;
        std::fs::write(&default_file, &quieter).unwrap();
        app.command("addNumberedLayer");
        assert!(app.error.is_none(), "{:?}", app.error);
        // The next layer takes the next index, though the first is not on disk.
        assert!(!folder.join("soft-hitclap9.wav").exists());
        assert!(app.document().unwrap().pending.get("soft-hitclap9.wav").is_some());
        assert_eq!(dawsu_osu_format::list_folder(&folder).unwrap(), before);
        assert_eq!(std::fs::read(&occupied).unwrap(), tone);
        assert_eq!(std::fs::read(&map).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&sibling).unwrap(), other);
        assert!(!app.is_dirty());
        let first_lane = dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap8.wav").unwrap();
        let second_lane = dawsu_hitsounding::rules::read_sample_file_name("soft-hitclap9.wav").unwrap();
        {
            let composer = &mut app.document.as_mut().unwrap().composer;
            composer.add(1000.0, first_lane.clone(), 100, 0.0);
            composer.add(1500.0, second_lane.clone(), 80, 0.0);
        }
        assert_eq!(app.clip_for_lane(&first_lane).unwrap().samples, [0.5]);
        assert_eq!(app.clip_for_lane(&second_lane).unwrap().samples, [0.25]);
        let cues = app.scheduled_sounds(app.document().unwrap());
        assert!(cues.iter().any(|c| c.at_ms == 1000.0 && c.clip.samples == [0.5]));
        assert!(cues.iter().any(|c| c.at_ms == 1500.0 && c.clip.samples == [0.25]));
    }

    const TONE: &[u8] = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";

    fn hitnormal(index: u32) -> SampleIdentity {
        SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Normal), index: dawsu_foundation::CustomIndex::new(index) }
    }

    /// The project fixture with two new hitnormal layers, 2 and 3, both pending.
    fn two_layers(scratch: &Scratch, project: bool) -> (Application, PathBuf, PathBuf) {
        two_layers_in(scratch, project, None)
    }

    /// With `data_root`, the session keeps its data apart from the shared headless folder.
    fn two_layers_in(scratch: &Scratch, project: bool, data_root: Option<PathBuf>) -> (Application, PathBuf, PathBuf) {
        let folder = scratch.path("mapset");
        let defaults = scratch.path("skin");
        for directory in [&folder, &defaults] {
            std::fs::create_dir_all(directory).unwrap();
        }
        let map = folder.join("test.osu");
        let text = include_str!("../tests/project-fixture.osu").replace("Title:", "Artist:Artist
Creator:Mapper
Title:");
        std::fs::write(&map, text).unwrap();
        for name in ["normal-hitnormal.wav", "normal-hitclap.wav"] {
            std::fs::write(defaults.join(name), TONE).unwrap();
        }
        let mut app = Application::headless();
        if let Some(data_root) = data_root {
            app.data_root = data_root;
        }
        if project {
            app.project_root = Some(scratch.path("projects"));
        }
        app.preferences.default_samples = defaults;
        open(&mut app, map.clone());
        app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &hitnormal(0)).unwrap();
        app.command("addNumberedLayer");
        app.command("addNumberedLayer");
        assert!(app.error.is_none(), "{:?}", app.error);
        let pending = &app.document().unwrap().pending;
        assert!(pending.get("normal-hitnormal2.wav").is_some() && pending.get("normal-hitnormal3.wav").is_some());
        (app, map, folder)
    }

    #[allow(clippy::disallowed_methods)]
    fn mapset(folder: &Path) -> Vec<String> {
        dawsu_osu_format::list_folder(folder).unwrap()
    }

    /// Layer 2 alone plays on the circle; layer 3 stays unused.
    fn use_layer_2(app: &mut Application) {
        app.document.as_mut().unwrap().composer.apply("Use layer 2", |hits| {
            let circle = dawsu_hitsounding::MomentId(0);
            for lane in hits.moment(circle).hits().cloned().collect::<Vec<_>>() {
                hits.remove_hit(circle, &lane);
            }
            hits.add_hit(circle, hitnormal(2));
        });
    }

    #[test]
    fn save_writes_only_the_pending_files_the_written_difficulty_plays() {
        for hs in [false, true] {
            let scratch = Scratch::new(if hs { "pending-save-hs" } else { "pending-save" });
            let (mut app, map, folder) = two_layers(&scratch, true);
            app.set_hs_mode(hs);
            use_layer_2(&mut app);
            let before = mapset(&folder);
            save_through_summary(&mut app, scratch.path("backups"));
            let after = mapset(&folder);
            assert_eq!(after.iter().filter(|name| !before.contains(name)).collect::<Vec<_>>(), ["normal-hitnormal2.wav"], "hs {hs}");
            assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
            let document = app.document().unwrap();
            assert!(document.pending.get("normal-hitnormal2.wav").is_none());
            assert!(document.pending.get("normal-hitnormal3.wav").is_some(), "the unused layer stays pending");
            assert!(document.files.iter().any(|name| name == "normal-hitnormal2.wav"));
            assert!(stored(&app).pending_samples.is_none(), "nothing the project needs is left pending");
            let reopened = reopen(&app, &map);
            assert!(reopened.document().unwrap().composer.hitsounds().moments()[0].has_hit(&hitnormal(2)));
            assert_eq!(reopened.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
        }
    }

    #[test]
    fn a_mix_source_layer_stays_out_of_the_mapset_and_comes_back_on_reopen() {
        let scratch = Scratch::new("pending-mix-source");
        let (mut app, map, folder) = two_layers(&scratch, true);
        // The skin hitnormal and layer 2 at another volume: Save merges them into one hitnormal file.
        app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
            let normal = dawsu_hitsounding::MomentId(0);
            hits.add_hit(normal, hitnormal(2));
            hits.set_hit_volume(normal, &hitnormal(2), 40);
        });
        let before = mapset(&folder);
        save_through_summary(&mut app, scratch.path("backups"));
        let added: Vec<_> = mapset(&folder).into_iter().filter(|name| !before.contains(name)).collect();
        assert!(!added.iter().any(|name| name.starts_with("normal-hitnormal2") || name.starts_with("normal-hitnormal3")), "{added:?}");
        assert!(!added.is_empty(), "the merged files are written");
        let kept = stored(&app).pending_samples.expect("the project lists the mix source");
        assert_eq!(kept.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(), ["normal-hitnormal2.wav"]);
        let mut reopened = Application::headless();
        reopened.project_root = app.project_root.clone();
        reopened.preferences.default_samples = app.preferences.default_samples.clone();
        open(&mut reopened, map);
        // The skin folder holds only the two sounds the map plays.
        let warnings: Vec<_> = reopened.warnings.iter().filter(|w| !w.starts_with("Sample unavailable")).collect();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(reopened.document().unwrap().composer.hitsounds().moments()[0].has_hit(&hitnormal(2)));
        assert_eq!(reopened.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
    }

    #[test]
    fn a_loose_hit_on_a_layer_keeps_the_layer_pending_in_the_project() {
        let scratch = Scratch::new("pending-loose");
        let (mut app, map, folder) = two_layers(&scratch, true);
        // A hit with no object: the project keeps it, the `.osu` never plays it.
        app.document.as_mut().unwrap().composer.add(5000.0, hitnormal(2), 100, 0.0);
        let before = mapset(&folder);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(mapset(&folder).len(), before.len());
        let kept = stored(&app).pending_samples.expect("the project lists the loose hit's layer");
        assert_eq!(kept.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(), ["normal-hitnormal2.wav"]);
        let reopened = reopen(&app, &map);
        let hits = reopened.document().unwrap().composer.hitsounds();
        assert!(hits.moments().iter().any(|moment| moment.has_hit(&hitnormal(2))));
        assert_eq!(reopened.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
    }

    #[test]
    fn a_left_out_moment_keeps_its_layer_pending_and_reopen_brings_it_back() {
        let scratch = Scratch::new("pending-left-out");
        let (mut app, map) = open_left_out_fixture(&scratch, true);
        let defaults = scratch.path("skin");
        std::fs::create_dir_all(&defaults).unwrap();
        std::fs::write(defaults.join("normal-hitnormal.wav"), TONE).unwrap();
        app.preferences.default_samples = defaults;
        app.active_lane = app.document().unwrap().composer.lanes.iter().position(|lane| lane == &hitnormal(0)).unwrap();
        app.command("addNumberedLayer");
        assert!(app.error.is_none(), "{:?}", app.error);
        // The layer goes on the slider tail, which Save leaves out (F052).
        app.document.as_mut().unwrap().composer.apply("Layer on the tail", |hits| {
            let tail = tail_id(hits);
            hits.add_hit(tail, hitnormal(2));
        });
        let folder = map.parent().unwrap().to_path_buf();
        let before = mapset(&folder);
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(!app.document().unwrap().left_out.is_empty());
        assert_eq!(mapset(&folder), before, "the left-out layer is not written");
        let kept = stored(&app).pending_samples.expect("the project lists the left-out layer");
        assert_eq!(kept.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(), ["normal-hitnormal2.wav"]);
        let reopened = reopen(&app, &map);
        let hits = reopened.document().unwrap().composer.hitsounds();
        assert!(hits.moment(tail_id(hits)).has_hit(&hitnormal(2)));
        assert_eq!(reopened.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
    }

    #[test]
    fn a_file_another_program_put_under_a_pending_name_refuses_save_and_names_it() {
        let scratch = Scratch::new("pending-clash");
        let (mut app, map, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        std::fs::write(folder.join("normal-hitnormal2.wav"), b"another program's file").unwrap();
        let before = (mapset(&folder), std::fs::read(&map).unwrap());
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        let error = app.error.clone().expect("Save refuses");
        assert!(error.contains("normal-hitnormal2.wav"), "{error}");
        assert_eq!((mapset(&folder), std::fs::read(&map).unwrap()), before);
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), b"another program's file");
    }

    #[test]
    fn generate_and_copy_write_the_pending_files_their_output_plays() {
        let scratch = Scratch::new("pending-output");
        let (mut app, map, folder) = two_layers(&scratch, false);
        use_layer_2(&mut app);
        app.generate();
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!folder.join("normal-hitnormal2.wav").exists(), "nothing is written before the review is applied");
        app.apply_output_in(scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
        assert!(!folder.join("normal-hitnormal3.wav").exists(), "the unused layer stays pending");
        assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_none());
        assert!(app.document().unwrap().files.iter().any(|name| name == "normal-hitnormal2.wav"));

        // Copy onto a sibling difficulty with layer 3 on its circle.
        let sibling = folder.join("other.osu");
        std::fs::write(&sibling, include_bytes!("../tests/project-fixture.osu")).unwrap();
        app.document.as_mut().unwrap().composer.apply("Use layer 3", |hits| {
            hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(3));
            hits.remove_hit(dawsu_hitsounding::MomentId(0), &hitnormal(2));
        });
        app.copy_hitsounds(vec![sibling.canonicalize().unwrap()], Default::default());
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        app.apply_output_in(scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(std::fs::read(folder.join("normal-hitnormal3.wav")).unwrap(), TONE);
        let _ = map;
    }

    #[test]
    fn a_restored_recovery_copy_plays_its_pending_layer_and_saves_it() {
        let scratch = Scratch::new("pending-recovery");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        app.poll();
        let offer = app.write_recovery_copy().unwrap().unwrap();
        let mut restored = Application::create(Preferences::default(), false, None, None, false, app.data_root.clone());
        restored.project_root = app.project_root.clone();
        restored.restore_recovery(&offer.id).unwrap();
        wait(&mut restored);
        assert!(restored.error.is_none(), "{:?}", restored.error);
        assert!(restored.document().unwrap().composer.hitsounds().moments()[0].has_hit(&hitnormal(2)));
        assert_eq!(restored.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
        save_through_summary(&mut restored, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
    }

    #[test]
    fn save_refuses_a_name_dawsu_claimed_whose_sound_is_missing() {
        let scratch = Scratch::new("pending-lost");
        let (mut app, map, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        // The editor lost the pending file but its claim is still there: both sides play the
        // skin's sound, which is not what the mapper drew.
        let document = app.document.as_mut().unwrap();
        let root = document.path.parent().unwrap().to_path_buf();
        document.pending = pending::Pending::for_mapset(&app.data_root, &root);
        let before = (mapset(&folder), std::fs::read(&map).unwrap());
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        let error = app.error.clone().expect("Save refuses");
        assert!(error.contains("normal-hitnormal2"), "{error}");
        assert_eq!((mapset(&folder), std::fs::read(&map).unwrap()), before);
    }

    #[test]
    fn another_windows_claim_on_a_name_this_difficulty_looks_up_refuses_save() {
        let scratch = Scratch::new("pending-other-claim");
        let folder = scratch.path("mapset");
        std::fs::create_dir_all(&folder).unwrap();
        let map = folder.join("test.osu");
        // Custom index 1: the circle looks up normal-hitnormal.wav in the mapset first.
        std::fs::write(&map, include_str!("../tests/project-fixture.osu").replace("0,500,4,1,0,100,1,0", "0,500,4,1,1,100,1,0")).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        // Another window made a new sample under that name; it isn't in the mapset yet.
        let mut other = pending::Pending::for_mapset(&app.data_root, &folder.canonicalize().unwrap());
        other.add("normal-hitnormal.wav", TONE).unwrap();
        // Once that window saves it, this difficulty would play it: Save refuses and names it.
        app.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
        let before = std::fs::read(&map).unwrap();
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        let error = app.error.clone().expect("refused");
        assert!(error.contains("normal-hitnormal.wav"), "{error}");
        assert_eq!(std::fs::read(&map).unwrap(), before);
        other.release("normal-hitnormal.wav");
    }

    fn claims(app: &Application, folder: &Path) -> Vec<String> {
        let dir = app.data_root.join("pending-names").join(pending::mapset_key(&folder.canonicalize().unwrap()));
        let mut names: Vec<String> = std::fs::read_dir(dir).map(|entries| entries.flatten()
            .filter(|entry| entry.path().is_file())
            .map(|entry| entry.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn closing_forgets_the_pending_files_nothing_needs_and_frees_their_numbers() {
        let scratch = Scratch::new("pending-close");
        let (mut app, map, folder) = two_layers(&scratch, true);
        // Layer 2 plays a loose hit the project keeps; layer 3 is never used.
        app.document.as_mut().unwrap().composer.add(5000.0, hitnormal(2), 100, 0.0);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(claims(&app, &folder), ["normal-hitnormal2.wav", "normal-hitnormal3.wav"]);
        app.close_document();
        assert_eq!(claims(&app, &folder), ["normal-hitnormal2.wav"]);
        // Reopening keeps layer 2 and gives the next new layer the freed number 3.
        let mut reopened = reopen(&app, &map);
        reopened.preferences.default_samples = app.preferences.default_samples.clone();
        reopened.active_lane = reopened.document().unwrap().composer.lanes.iter().position(|lane| lane == &hitnormal(0)).unwrap();
        reopened.command("addNumberedLayer");
        assert!(reopened.error.is_none(), "{:?}", reopened.error);
        assert!(reopened.document().unwrap().pending.get("normal-hitnormal3.wav").is_some());
    }

    #[test]
    fn opening_releases_claims_a_closed_or_crashed_session_left_behind() {
        let scratch = Scratch::new("pending-stale-claims");
        let (app, map, folder) = two_layers(&scratch, true);
        let claimed = claims(&app, &folder);
        assert_eq!(claimed.len(), 2);
        // The session ends without a close (a crash): its claims stay until the next open.
        let data_root = app.data_root.clone();
        let project_root = app.project_root.clone();
        drop(app);
        let mut other = Application::create(Preferences::default(), false, None, None, false, data_root.clone());
        other.project_root = project_root.clone();
        let live = pending::Presence::enter(&data_root, &folder.canonicalize().unwrap()).unwrap();
        open(&mut other, map.clone());
        assert_eq!(claims(&other, &folder), claimed, "another live session may still use them");
        drop(live);
        other.close_document();
        let mut next = Application::create(Preferences::default(), false, None, None, false, data_root);
        next.project_root = project_root;
        open(&mut next, map);
        assert!(claims(&next, &folder).is_empty(), "{:?}", claims(&next, &folder));
    }

    #[test]
    fn the_project_lists_kept_pending_files_before_the_osu_is_written() {
        let scratch = Scratch::new("pending-staged-record");
        let (mut app, _, _) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        app.document.as_mut().unwrap().composer.add(5000.0, hitnormal(3), 100, 0.0);
        app.save_session().unwrap();
        let document = app.document().unwrap();
        let mut project = app.project.clone().unwrap();
        let plan = prepare_save(
            document.compile_base.clone(), document.composer.hitsounds(), &document.written, document.file.original_bytes().to_vec(),
            &document.path, scratch.path("backups"), false, &BTreeSet::new(), Path::new(""), &document.pending, Some(&project), false,
            &|| false, &mut Vec::new(), &mut |_, _| {},
        ).unwrap();
        // The `.osu` write fails, as a crash right after the project write would leave it.
        std::fs::write(scratch.path("backups"), b"not a folder").unwrap();
        assert!(plan.apply(Some(&mut project), &|| false).is_err());
        let stored = project::Project::load(app.project_root.as_ref().unwrap(), None, &document.path).unwrap().settings;
        let kept = stored.pending_samples.expect("listed before the .osu write");
        assert!(kept.iter().any(|file| file.name == "normal-hitnormal3.wav"));
    }

    #[test]
    fn a_file_another_program_writes_under_a_pending_name_keeps_the_layers_sound() {
        let scratch = Scratch::new("pending-watch-clash");
        let (mut app, _, folder) = two_layers(&scratch, false);
        let mut louder = TONE.to_vec();
        *louder.last_mut().unwrap() = 0x60;
        std::fs::write(folder.join("normal-hitnormal3.wav"), &louder).unwrap();
        let deadline = Instant::now() + Duration::from_millis(2500);
        while Instant::now() < deadline {
            app.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        let document = app.document().unwrap();
        assert!(!document.files.iter().any(|name| name == "normal-hitnormal3.wav"), "still a pending file");
        assert_eq!(app.clip_for_lane(&hitnormal(3)).unwrap().samples, [0.5], "the layer keeps its own sound");
        assert!(app.status.contains("normal-hitnormal3.wav"), "{}", app.status);
        // Reported once: the watcher stops reading and decoding the file every second.
        app.status.clear();
        let deadline = Instant::now() + Duration::from_millis(2500);
        while Instant::now() < deadline {
            app.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!app.status.contains("normal-hitnormal3.wav"), "{}", app.status);
    }

    fn ledger(app: &Application) -> Vec<String> {
        app.document().unwrap().pending.ledger().unwrap().entries().unwrap().into_iter().map(|file| file.name).collect()
    }

    #[test]
    fn the_ledger_lists_each_sample_file_a_save_creates_and_drops_a_taken_back_one() {
        let scratch = Scratch::new("pending-ledger");
        let (mut app, _, _) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        std::fs::write(scratch.path("backups"), b"not a folder").unwrap();
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.take().is_some());
        assert!(ledger(&app).is_empty(), "a taken-back file is not listed");
        std::fs::remove_file(scratch.path("backups")).unwrap();
        save_through_summary(&mut app, scratch.path("backups"));
        let entries = app.document().unwrap().pending.ledger().unwrap().entries().unwrap();
        assert_eq!(entries, [pending::PendingSample { name: "normal-hitnormal2.wav".into(), sha256: dawsu_foundation::digest::sha256_hex(TONE) }]);
    }

    fn back_to_skin(app: &mut Application) {
        app.document.as_mut().unwrap().composer.apply("Back to the skin", |hits| {
            let circle = dawsu_hitsounding::MomentId(0);
            hits.remove_hit(circle, &hitnormal(2));
            hits.add_hit(circle, hitnormal(0));
        });
    }

    /// Layer 2 joins the skin's hitnormal at another volume: the `.osu` plays a mix, and the
    /// layer's file, which a hit still plays, leaves the mapset at the next Save.
    fn mix_with_skin(app: &mut Application) {
        app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
            hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(0));
            hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
        });
    }

    #[test]
    fn a_file_changed_after_its_check_stays_in_the_mapset_as_changed() {
        let scratch = Scratch::new("sweep-changed-before-move");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        fn edit(path: &Path) {
            if path.to_string_lossy().contains("sweep-changed-before-move") {
                std::fs::write(path, b"edited while dawsu! checked it").unwrap();
            }
        }
        mix_with_skin(&mut app);
        let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *sweep::BEFORE_MOVE.lock().unwrap() = Some(edit);
        save_through_summary(&mut app, scratch.path("backups"));
        *sweep::BEFORE_MOVE.lock().unwrap() = None;
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), b"edited while dawsu! checked it");
        let names: Vec<_> = mapset(&folder);
        assert!(names.iter().all(|name| !name.contains("dawsu-moving")), "{names:?}");
        assert!(!ledger(&app).iter().any(|name| name == "normal-hitnormal2.wav"), "no longer dawsu!'s file");
    }

    #[test]
    fn a_file_a_stopped_cleanup_left_aside_comes_back_under_its_name() {
        let scratch = Scratch::new("sweep-stopped-move");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        std::fs::rename(folder.join("normal-hitnormal2.wav"), folder.join(".normal-hitnormal2.wav.dawsu-moving")).unwrap();
        assert!(mapset(&folder).iter().any(|name| name.ends_with(".dawsu-moving")), "the listing shows dot-names");
        app.close_document();
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
        assert!(!mapset(&folder).iter().any(|name| name.ends_with(".dawsu-moving")));
    }

    #[test]
    fn a_file_only_a_lane_names_goes_to_removed_samples_not_pending() {
        let scratch = Scratch::new("sweep-lane-only");
        let (mut app, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        let mut bytes = TONE.to_vec();
        *bytes.last_mut().unwrap() = 0x61;
        std::fs::write(folder.join("normal-hitclap7.wav"), &bytes).unwrap();
        let sha256 = dawsu_foundation::digest::sha256_hex(&bytes);
        app.document().unwrap().pending.ledger().unwrap()
            .add(&[pending::PendingSample { name: "normal-hitclap7.wav".into(), sha256: sha256.clone() }]).unwrap();
        // Closed while another session holds the mapset, so nothing is cleaned up yet.
        let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        app.close_document();
        drop(other);
        open(&mut app, map);
        assert!(app.document().unwrap().composer.hitsounds().lanes().iter()
            .any(|lane| matches!(&lane.source, dawsu_hitsounding::LaneSource::File(name) if name == "normal-hitclap7.wav")), "it has a lane");
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(!folder.join("normal-hitclap7.wav").exists());
        assert_eq!(std::fs::read(app.data_root.join("removed-samples").join(format!("{sha256}.wav"))).unwrap(), bytes);
        assert!(app.document().unwrap().pending.get("normal-hitclap7.wav").is_none(), "not a pending file");
        assert!(!app.document().unwrap().pending.claimed_names().contains("normal-hitclap7.wav"), "no claim");
        assert!(!app.data_root.join("pending-samples").join(format!("{sha256}.wav")).exists());
    }

    #[test]
    fn a_file_a_hit_plays_comes_back_when_the_cleanup_stops_before_the_project_lists_it() {
        let scratch = Scratch::new("sweep-stopped-before-listing");
        let (mut app, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
            hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(0));
            hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
        });
        save_through_summary(&mut app, scratch.path("backups"));
        drop(other);
        assert!(folder.join("normal-hitnormal2.wav").exists(), "kept while the other session was open");
        // The job runs, and dawsu! stops before the editor thread applies what it did.
        app.start_sweep(false);
        let _ = app.sweep.take().unwrap().recv().unwrap();
        assert!(!folder.join("normal-hitnormal2.wav").exists(), "moved aside, waiting for the project");
        let (data_root, project_root) = (app.data_root.clone(), app.project_root.clone());
        drop(app);
        let mut second = Application::headless();
        second.data_root = data_root;
        second.project_root = project_root;
        second.preferences.default_samples = scratch.path("skin");
        open(&mut second, map);
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "back under its name");
        assert!(ledger(&second).iter().any(|name| name == "normal-hitnormal2.wav"), "still dawsu!'s");
        assert_eq!(second.document().unwrap().composer.hitsounds().lane(&hitnormal(2)).map(|lane| &lane.source),
            Some(&dawsu_hitsounding::LaneSource::File("normal-hitnormal2.wav".into())), "its lane plays the file, not the skin");
        second.document.as_mut().unwrap().composer.apply("Mix", |hits| {
            hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(2));
            hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
        });
        save_through_summary(&mut second, scratch.path("backups"));
        let document = second.document().unwrap();
        assert!(folder.join("normal-hitnormal2.wav").exists() || document.pending.get("normal-hitnormal2.wav").is_some(),
            "a hit plays it, so it never goes to removed-samples");
    }

    #[test]
    fn a_file_only_undo_plays_stays_in_the_mapset_and_goes_to_removed_samples_at_close() {
        let scratch = Scratch::new("sweep-undo-only");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "Undo can still play it");
        let copy = format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE));
        assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_none(), "it never left for the data folder");
        let (data_root, project_root) = (app.data_root.clone(), app.project_root.clone());
        app.save_session().unwrap();
        app.close_document();
        assert!(!folder.join("normal-hitnormal2.wav").exists());
        pending::prune_kept_copies(&data_root, project_root.as_deref(), Duration::ZERO);
        assert_eq!(std::fs::read(data_root.join("removed-samples").join(&copy)).unwrap(), TONE, "kept, never deleted");
    }

    #[test]
    fn a_file_left_aside_by_a_crash_is_never_replaced_by_the_next_move() {
        let scratch = Scratch::new("sweep-aside-taken");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        let aside = folder.join(".normal-hitnormal2.wav.dawsu-moving");
        std::fs::write(&aside, b"bytes a crash left aside").unwrap();
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(&aside).unwrap(), b"bytes a crash left aside");
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "it can't move aside, so it stays");
    }

    #[test]
    fn a_damaged_copy_in_removed_samples_is_replaced_before_the_file_leaves() {
        let scratch = Scratch::new("sweep-damaged-trash");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        drop(other);
        let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, b"damaged").unwrap();
        app.close_document();
        assert!(!folder.join("normal-hitnormal2.wav").exists());
        assert_eq!(std::fs::read(&kept).unwrap(), TONE, "removed-samples holds the file's bytes");
        let damaged: Vec<_> = std::fs::read_dir(kept.parent().unwrap()).unwrap().flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".damaged-")).map(|e| std::fs::read(e.path()).unwrap()).collect();
        assert_eq!(damaged, [b"damaged".to_vec()], "the damaged copy is kept aside, not overwritten");
    }

    #[test]
    fn a_file_with_no_hits_left_goes_to_removed_samples_at_close() {
        let scratch = Scratch::new("sweep-trash-at-close");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        // Another session keeps the file in the mapset through this Save.
        let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(folder.join("normal-hitnormal2.wav").exists());
        drop(other);
        let data_root = app.data_root.clone();
        app.close_document();
        let kept = data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
        assert_eq!(std::fs::read(kept).unwrap(), TONE);
        assert!(!folder.join("normal-hitnormal2.wav").exists());
    }

    #[test]
    fn a_difficulty_that_cant_be_read_stops_the_cleanup_and_says_so() {
        let scratch = Scratch::new("sweep-unreadable");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        std::fs::write(folder.join("broken.osu"), "osu file format v14\n[TimingPoints]\nnot a timing point\n").unwrap();
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(folder.join("normal-hitnormal2.wav").exists());
        assert!(app.status.contains("No unused sample files were removed") && app.status.contains("broken.osu"), "{}", app.status);
    }

    #[test]
    fn the_project_lists_a_kept_mix_source_even_when_nothing_moved() {
        let scratch = Scratch::new("sweep-lists-needed");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
            hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(0));
            hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
        });
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(!folder.join("normal-hitnormal2.wav").exists(), "a mix plays it now");
        // Read through the open document's path: open stores it in its full form, which on
        // Windows can differ from `map` (a short 8.3 temp folder name such as RUNNER~1).
        let listed = |app: &Application| stored(app).pending_samples.unwrap_or_default()
            .into_iter().map(|file| file.name).collect::<Vec<_>>();
        assert_eq!(listed(&app), ["normal-hitnormal2.wav"]);
        // The list is lost; the next Save with nothing to move lists it again.
        let mut project = app.project.clone().unwrap();
        let mut settings = project.settings.clone();
        settings.pending_samples = None;
        project.save(settings).unwrap();
        app.project = Some(project);
        app.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 90));
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(listed(&app), ["normal-hitnormal2.wav"]);
    }

    #[test]
    fn two_windows_writing_the_ledger_at_once_keep_each_others_entries() {
        let scratch = Scratch::new("ledger-two-windows");
        let (data, mapset) = (scratch.path("data"), scratch.path("mapset"));
        let threads: Vec<_> = ["a", "b"].into_iter().map(|window| {
            let ledger = pending::Pending::for_mapset(&data, &mapset).ledger().unwrap();
            std::thread::spawn(move || {
                for n in 0..40 {
                    let name = format!("{window}-{n}.wav");
                    ledger.add(&[pending::PendingSample { name: name.clone(), sha256: "0".repeat(64) }]).unwrap();
                    if n % 4 == 3 {
                        ledger.remove(&[name]).unwrap();
                    }
                }
            })
        }).collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let ledger = pending::Pending::for_mapset(&data, &mapset).ledger().unwrap();
        assert_eq!(ledger.entries().unwrap().len(), 60);
    }

    #[test]
    fn a_session_lock_still_being_made_is_left_alone() {
        let scratch = Scratch::new("presence-being-made");
        let (data, mapset) = (scratch.path("data"), scratch.path("mapset"));
        let me = pending::Presence::enter(&data, &mapset).unwrap();
        // Another window's lock file before it locked it: that window is live, and its file stays.
        let folder = data.join("sessions").join(pending::mapset_key(&mapset));
        let making = folder.join("99999-0.lock.tmp");
        std::fs::write(&making, b"").unwrap();
        assert!(me.others_live());
        assert!(making.exists());
        // One left by a crash a while ago is removed and doesn't count.
        std::fs::File::options().write(true).open(&making).unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(120)).unwrap();
        assert!(!me.others_live());
        assert!(!making.exists());
        let other = pending::Presence::enter(&data, &mapset).unwrap();
        assert!(me.others_live());
        let names: Vec<_> = std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().filter(|name| name.ends_with(".lock")).count() == 2, "{names:?}");
        drop(other);
    }

    #[test]
    fn a_session_that_cannot_be_recorded_does_not_open() {
        let scratch = Scratch::new("presence-fails");
        let folder = scratch.path("mapset");
        std::fs::create_dir_all(&folder).unwrap();
        let map = folder.join("test.osu");
        std::fs::write(&map, include_str!("../tests/project-fixture.osu")).unwrap();
        let mut app = Application::headless();
        app.data_root = scratch.path("data");
        std::fs::create_dir_all(&app.data_root).unwrap();
        // A file where the sessions folder belongs.
        std::fs::write(app.data_root.join("sessions"), b"").unwrap();
        app.open_now(OpenSource::Map(map));
        wait(&mut app);
        assert!(app.document().is_none());
        assert!(app.error.as_deref().is_some_and(|error| error.contains("session")), "{:?}", app.error);
    }

    #[test]
    fn kept_copies_no_record_names_are_pruned() {
        let scratch = Scratch::new("prune-kept-copies");
        let (data, projects) = (scratch.path("data"), scratch.path("projects"));
        let samples = data.join("pending-samples");
        std::fs::create_dir_all(&samples).unwrap();
        let sha = |n: u8| format!("{n:064x}");
        for n in 1..=6 {
            std::fs::write(samples.join(format!("{}.wav", sha(n))), [n]).unwrap();
        }
        let write = |path: PathBuf, text: String| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(projects.join("p.json"), format!("{{\"settings\":{{\"pending_samples\":[{{\"name\":\"a.wav\",\"sha256\":\"{}\"}}]}}}}", sha(1)));
        write(data.join("recovery").join("c.json"), format!("{{\"pending\":[{{\"sha256\":\"{}\"}}]}}", sha(2)));
        write(data.join("recovery").join("archive").join("old.json"), format!("{{\"pending\":[{{\"sha256\":\"{}\"}}]}}", sha(3)));
        write(data.join("created-samples").join("k.json"), format!("{{\"files\":[{{\"sha256\":\"{}\"}}]}}", sha(4)));
        write(data.join("pending-names").join("k").join("normal-hitnormal2.wav"), sha(5));
        let removed = pending::prune_kept_copies(&data, Some(&projects), Duration::ZERO);
        assert_eq!(removed, 1);
        for n in 1..=5 {
            assert!(samples.join(format!("{}.wav", sha(n))).exists(), "{n} is named by a record");
        }
        assert!(!samples.join(format!("{}.wav", sha(6))).exists());
        assert_eq!(std::fs::read(data.join("removed-samples").join(format!("{}.wav", sha(6)))).unwrap(), [6], "kept in removed-samples");
        // Without a project folder nothing is known to be unnamed.
        std::fs::write(samples.join(format!("{}.wav", sha(7))), [7]).unwrap();
        assert_eq!(pending::prune_kept_copies(&data, None, Duration::ZERO), 0);
        // A copy younger than the age limit may belong to a window still making its claim.
        assert_eq!(pending::prune_kept_copies(&data, Some(&projects), Duration::from_secs(3600)), 0);
    }

    #[test]
    fn removed_samples_keep_thirty_days_and_a_size_limit_oldest_first() {
        let scratch = Scratch::new("cap-removed-samples");
        let data = scratch.path("data");
        let folder = data.join("removed-samples");
        std::fs::create_dir_all(&folder).unwrap();
        let day = Duration::from_secs(24 * 3600);
        let now = std::time::SystemTime::now();
        for (name, age, size) in [("old.wav", 31, 10), ("a.wav", 3, 40), ("b.wav", 2, 40), ("c.wav", 1, 40)] {
            let path = folder.join(name);
            std::fs::write(&path, vec![0; size]).unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(now - day * age).unwrap();
        }
        assert_eq!(pending::cap_removed_samples(&data, day * 30, 100), 2);
        let mut left: Vec<_> = std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["b.wav", "c.wav"]);
    }

    #[test]
    fn a_reused_copy_in_removed_samples_counts_from_its_latest_removal() {
        let scratch = Scratch::new("removed-reuse-date");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
        std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
        std::fs::write(&kept, TONE).unwrap();
        std::fs::File::options().write(true).open(&kept).unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(31 * 24 * 3600)).unwrap();
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        drop(other);
        let data_root = app.data_root.clone();
        app.close_document();
        wait(&mut app);
        assert!(!folder.join("normal-hitnormal2.wav").exists());
        pending::cap_removed_samples(&data_root, Duration::from_secs(30 * 24 * 3600), u64::MAX);
        assert_eq!(std::fs::read(&kept).unwrap(), TONE, "removed today, kept for 30 days from today");
    }

    #[test]
    fn adding_a_kept_copy_that_is_already_there_refreshes_its_date() {
        let scratch = Scratch::new("pending-copy-date");
        let mut pending = pending::Pending::for_mapset(&scratch.path("data"), &scratch.path("mapset"));
        let copy = scratch.path("data").join("pending-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, TONE).unwrap();
        std::fs::File::options().write(true).open(&copy).unwrap()
            .set_modified(std::time::SystemTime::now() - Duration::from_secs(7200)).unwrap();
        pending.add("normal-hitnormal2.wav", TONE).unwrap();
        let age = std::fs::metadata(&copy).unwrap().modified().unwrap().elapsed().unwrap();
        assert!(age < Duration::from_secs(60), "{age:?}");
    }

    #[test]
    fn a_ledger_that_cant_be_read_stops_the_cleanup_and_says_so() {
        let scratch = Scratch::new("ledger-unreadable");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        let path = app.data_root.join("created-samples").join(format!("{}.json", pending::mapset_key(&folder.canonicalize().unwrap())));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        app.close_document();
        wait(&mut app);
        assert!(folder.join("normal-hitnormal2.wav").exists() || app.status.contains("ledger"), "{}", app.status);
        let ledger = pending::Pending::for_mapset(&scratch.path("data"), &folder.canonicalize().unwrap()).ledger().unwrap();
        assert!(ledger.entries().is_err());
        assert!(ledger.add(&[pending::PendingSample { name: "x.wav".into(), sha256: "0".repeat(64) }]).is_err());
    }

    #[test]
    fn a_second_corrupt_ledger_is_kept_aside_beside_the_first() {
        let scratch = Scratch::new("ledger-corrupt-twice");
        let (app, _, _) = two_layers(&scratch, true);
        let ledger = app.document().unwrap().pending.ledger().unwrap();
        let path = app.data_root.join("created-samples").join(format!("{}.json", pending::mapset_key(app.document().unwrap().path.parent().unwrap())));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for text in [&b"{ first"[..], b"{ second"] {
            std::fs::write(&path, text).unwrap();
            assert!(ledger.entries().unwrap().is_empty());
        }
        let key = pending::mapset_key(app.document().unwrap().path.parent().unwrap());
        let mut aside: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten()
            .filter(|e| { let name = e.file_name().to_string_lossy().into_owned(); name.starts_with(&key) && name.contains("corrupt") }).map(|e| std::fs::read(e.path()).unwrap()).collect();
        aside.sort();
        assert_eq!(aside, [b"{ first".to_vec(), b"{ second".to_vec()]);
    }

    #[test]
    fn an_old_mix_no_ledger_lists_is_cleaned_up_when_nothing_plays_it() {
        let mix = |tag: &str| {
            let bytes = format!("old mix {tag}").into_bytes();
            (format!("dawsu-mix-{}-1.wav", dawsu_foundation::digest::sha256_hex(&bytes)), bytes)
        };
        let scratch = Scratch::new("sweep-old-mixes");
        let (mut app, _, folder) = two_layers(&scratch, true);
        let (unused, unused_bytes) = mix("unused");
        let (other, other_bytes) = mix("other difficulty");
        let (mania, mania_bytes) = mix("mania");
        let (story, story_bytes) = mix("storyboard");
        let (copied, copied_bytes) = mix("recovery copy");
        let (changed, _) = mix("changed");
        for (name, bytes) in [(&unused, &unused_bytes), (&other, &other_bytes), (&mania, &mania_bytes), (&story, &story_bytes), (&copied, &copied_bytes)] {
            std::fs::write(folder.join(name), bytes).unwrap();
        }
        std::fs::write(folder.join(&changed), b"edited by the mapper").unwrap();
        // An old numbered file no ledger lists stays, played or not.
        std::fs::write(folder.join("normal-hitnormal7.wav"), TONE).unwrap();
        let fixture = include_str!("../tests/project-fixture.osu");
        std::fs::write(folder.join("other.osu"), fixture.replace("Version:Hard", "Version:Other").replace("0:0:0:0:", &format!("0:0:0:0:{other}"))).unwrap();
        std::fs::write(folder.join("mania.osu"), fixture.replace("Mode:0", "Mode:3").replace("Version:Hard", "Version:Mania").replace("0:0:0:0:", &format!("0:0:0:0:{mania}"))).unwrap();
        std::fs::write(folder.join("Artist - Title (Mapper).osb"), format!("[Events]
Sample,0,0,\"{story}\",100
")).unwrap();
        // A stopped session's recovery copy lists it as one of its pending files.
        let mut crashed = Application::headless();
        crashed.project_root = Some(scratch.path("projects"));
        open(&mut crashed, folder.join("other.osu"));
        crashed.document.as_mut().unwrap().pending.add(&copied, &copied_bytes).unwrap();
        crashed.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
        crashed.poll();
        crashed.write_recovery_copy().unwrap().unwrap();
        drop(crashed);

        // While another session has the mapset open, nothing leaves.
        let live = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(folder.join(&unused).exists(), "another session is open");
        drop(live);

        app.document.as_mut().unwrap().composer.apply("Louder", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 90));
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(!folder.join(&unused).exists(), "the unused old mix left the mapset");
        let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(&unused_bytes)));
        assert_eq!(std::fs::read(kept).unwrap(), unused_bytes, "kept in removed-samples");
        for (name, why) in [(&other, "another difficulty plays it"), (&mania, "a mania difficulty plays it"), (&story, "the storyboard plays it"),
            (&copied, "a recovery copy lists it"), (&changed, "its bytes are not the ones its name gives")]
        {
            assert!(folder.join(name).exists(), "{why}");
        }
        assert_eq!(std::fs::read(folder.join("normal-hitnormal7.wav")).unwrap(), TONE, "old numbered files stay");
        assert!(ledger(&app).iter().all(|name| !name.starts_with("dawsu-mix-")), "{:?}", ledger(&app));
    }

    #[test]
    fn a_file_a_crashed_sessions_recovery_copy_plays_stays_in_the_mapset() {
        let scratch = Scratch::new("sweep-recovery-copy");
        let (mut crashed, map, folder) = two_layers(&scratch, true);
        use_layer_2(&mut crashed);
        save_through_summary(&mut crashed, scratch.path("backups"));
        crashed.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
        crashed.poll();
        crashed.write_recovery_copy().unwrap().unwrap();
        drop(crashed);
        let mut second = Application::headless();
        second.project_root = Some(scratch.path("projects"));
        open(&mut second, map);
        back_to_skin(&mut second);
        save_through_summary(&mut second, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "the recovery copy still plays it");
    }

    #[test]
    fn a_recovery_copy_that_cant_be_read_stops_the_cleanup_and_is_named() {
        let scratch = Scratch::new("sweep-unreadable-copy");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        std::fs::create_dir_all(app.data_root.join("recovery")).unwrap();
        std::fs::write(app.data_root.join("recovery").join("broken-copy.json"), b"not json").unwrap();
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "the copy could play it");
        assert!(app.status.contains("No unused sample files were removed") && app.status.contains("broken-copy.json"), "{}", app.status);
    }

    /// A file dawsu! wrote that nothing plays, on the ledger: the next cleanup takes it out.
    fn unused_file(app: &Application, folder: &Path) -> (Vec<u8>, String) {
        let mut bytes = TONE.to_vec();
        *bytes.last_mut().unwrap() = 0x61;
        std::fs::write(folder.join("normal-hitclap7.wav"), &bytes).unwrap();
        let sha256 = dawsu_foundation::digest::sha256_hex(&bytes);
        app.document().unwrap().pending.ledger().unwrap()
            .add(&[pending::PendingSample { name: "normal-hitclap7.wav".into(), sha256: sha256.clone() }]).unwrap();
        (bytes, sha256)
    }

    #[test]
    fn a_cleanup_stops_when_a_difficulty_changes_while_it_runs() {
        let scratch = Scratch::new("sweep-map-changed");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        std::fs::write(folder.join("other.osu"), include_str!("../tests/project-fixture.osu").replace("Version:Hard", "Version:Other")).unwrap();
        unused_file(&app, &folder);
        fn edit_other(path: &Path) {
            if path.to_string_lossy().contains("sweep-map-changed") {
                let other = path.with_file_name("other.osu");
                let text = std::fs::read_to_string(&other).unwrap();
                std::fs::write(&other, text + "\n").unwrap();
            }
        }
        app.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
        let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *sweep::BEFORE_MOVE.lock().unwrap() = Some(edit_other);
        save_through_summary(&mut app, scratch.path("backups"));
        *sweep::BEFORE_MOVE.lock().unwrap() = None;
        assert!(folder.join("normal-hitclap7.wav").exists(), "other.osu changed, so nothing moves");
        assert!(ledger(&app).iter().any(|name| name == "normal-hitclap7.wav"), "still dawsu!'s, tried again later");
        app.close_document();
        assert!(!folder.join("normal-hitclap7.wav").exists(), "the next cleanup takes it out");
    }

    #[test]
    fn a_file_only_a_recovery_copys_lane_names_leaves_the_mapset() {
        let scratch = Scratch::new("sweep-recovery-lane-only");
        let (mut crashed, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        let (bytes, sha256) = unused_file(&crashed, &folder);
        // Reopened while another session holds the mapset, so the file gets a lane and stays.
        let other = pending::Presence::enter(&crashed.data_root, &folder.canonicalize().unwrap()).unwrap();
        crashed.close_document();
        open(&mut crashed, map.clone());
        drop(other);
        assert!(crashed.document().unwrap().composer.hitsounds().lanes().iter()
            .any(|lane| matches!(&lane.source, dawsu_hitsounding::LaneSource::File(name) if name == "normal-hitclap7.wav")), "it has a lane");
        crashed.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
        crashed.poll();
        crashed.write_recovery_copy().unwrap().unwrap();
        let (data_root, project_root) = (crashed.data_root.clone(), crashed.project_root.clone());
        drop(crashed);
        let mut second = Application::headless();
        second.data_root = data_root;
        second.project_root = project_root;
        second.preferences.default_samples = scratch.path("skin");
        open(&mut second, map);
        second.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 70));
        // The copy is still there when the cleanup runs, and its lane names the file.
        let copies: Vec<String> = std::fs::read_dir(second.data_root.join("recovery")).unwrap().flatten()
            .filter(|entry| entry.path().is_file()).map(|entry| std::fs::read_to_string(entry.path()).unwrap()).collect();
        assert!(copies.iter().any(|copy| copy.contains("normal-hitclap7.wav")), "the copy names the file");
        save_through_summary(&mut second, scratch.path("backups"));
        assert!(!folder.join("normal-hitclap7.wav").exists(), "a lane with no hit keeps nothing, in a copy too");
        assert_eq!(std::fs::read(second.data_root.join("removed-samples").join(format!("{sha256}.wav"))).unwrap(), bytes);
    }

    #[test]
    fn a_recovery_copy_that_cant_be_read_frees_no_claim() {
        let scratch = Scratch::new("claims-unreadable-copy");
        let (app, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        let claimed = claims(&app, &folder);
        assert_eq!(claimed.len(), 2);
        let (data_root, project_root) = (app.data_root.clone(), app.project_root.clone());
        std::fs::create_dir_all(data_root.join("recovery")).unwrap();
        std::fs::write(data_root.join("recovery").join("broken-copy.json"), b"not json").unwrap();
        // A crash: the claims stay, and the next open can't tell whether the copy needs them.
        drop(app);
        let mut next = Application::create(Preferences::default(), false, None, None, false, data_root.clone());
        next.project_root = project_root.clone();
        open(&mut next, map.clone());
        assert_eq!(claims(&next, &folder), claimed, "at open");
        // Its own unused layer 3 keeps its claim at close too.
        next.close_document();
        let closing = Scratch::new("claims-unreadable-copy-close");
        let (mut third, _, folder) = two_layers_in(&closing, true, Some(closing.path("data")));
        std::fs::create_dir_all(third.data_root.join("recovery")).unwrap();
        std::fs::write(third.data_root.join("recovery").join("broken-copy.json"), b"not json").unwrap();
        third.close_document();
        assert_eq!(claims(&third, &folder), ["normal-hitnormal2.wav", "normal-hitnormal3.wav"], "at close");
    }

    #[test]
    fn close_waits_at_most_the_limit_and_the_cleanup_keeps_the_mapset_marked_open() {
        let scratch = Scratch::new("sweep-slow-close");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        unused_file(&app, &folder);
        fn slow(path: &Path) {
            if path.to_string_lossy().contains("sweep-slow-close") {
                std::thread::sleep(Duration::from_millis(1500));
            }
        }
        app.sweep_wait = Duration::from_millis(100);
        let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *sweep::BEFORE_MOVE.lock().unwrap() = Some(slow);
        let started = Instant::now();
        app.close_document();
        assert!(started.elapsed() < Duration::from_millis(1000), "{:?}", started.elapsed());
        assert!(app.status.contains("Unused sample cleanup is still finishing in the background"), "{}", app.status);
        let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        assert!(other.others_live(), "the cleanup keeps the mapset marked open");
        drop(other);
        let again = Instant::now();
        app.poll_sweep(true);
        assert!(again.elapsed() < Duration::from_millis(50), "a job already given up on is not waited for again: {:?}", again.elapsed());
        finish_sweep(&mut app);
        *sweep::BEFORE_MOVE.lock().unwrap() = None;
        assert!(!folder.join("normal-hitclap7.wav").exists(), "it still finishes");
        assert!(!app.status.contains("still finishing"), "{}", app.status);
        let probe = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        assert!(!probe.others_live(), "and the mapset is free again");
    }

    #[test]
    fn a_file_waiting_to_leave_stays_when_open_stops_waiting_for_the_cleanup() {
        let scratch = Scratch::new("sweep-slow-open");
        let (mut app, map, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        fn slow(path: &Path) {
            if path.to_string_lossy().contains("sweep-slow-open") {
                std::thread::sleep(Duration::from_millis(1000));
            }
        }
        // A mix plays layer 2 now, so this Save's cleanup moves it out; the map opens right after.
        mix_with_skin(&mut app);
        app.request_open(map);
        app.sweep_wait = Duration::from_millis(100);
        let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *sweep::BEFORE_MOVE.lock().unwrap() = Some(slow);
        let opens = app.opens;
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(app.opens, opens + 1, "opened while the cleanup still ran");
        assert!(app.sweep.is_some(), "the job is still running");
        assert!(app.status.contains("Unused sample cleanup is still finishing in the background"), "{}", app.status);
        finish_sweep(&mut app);
        *sweep::BEFORE_MOVE.lock().unwrap() = None;
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "back under its name");
        assert!(!mapset(&folder).iter().any(|name| name.ends_with(".dawsu-moving")));
        assert!(ledger(&app).iter().any(|name| name == "normal-hitnormal2.wav"), "still dawsu!'s; the next Save tries again");
        assert!(!app.status.contains("still finishing"), "{}", app.status);
    }

    /// Polls until the cleanup job is done.
    fn finish_sweep(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.sweep.is_some() {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn when_the_next_map_fails_to_open_the_cleanup_finishes_for_the_map_still_open() {
        let scratch = Scratch::new("sweep-open-fails");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        fn slow(path: &Path) {
            if path.to_string_lossy().contains("sweep-open-fails") {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        mix_with_skin(&mut app);
        app.request_open(scratch.path("missing").join("gone.osu"));
        app.sweep_wait = Duration::from_millis(100);
        let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *sweep::BEFORE_MOVE.lock().unwrap() = Some(slow);
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.take().is_some(), "the open failed");
        finish_sweep(&mut app);
        *sweep::BEFORE_MOVE.lock().unwrap() = None;
        assert!(!folder.join("normal-hitnormal2.wav").exists(), "the same map is still open, so the file leaves as planned");
        assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_some(), "as a pending file");
    }

    #[test]
    fn a_cleanup_stops_when_another_session_opens_the_mapset_while_it_runs() {
        static OTHER: std::sync::Mutex<Option<pending::Presence>> = std::sync::Mutex::new(None);
        fn open_elsewhere(path: &Path) {
            if path.to_string_lossy().contains("sweep-other-opens") {
                let mapset = path.parent().unwrap().canonicalize().unwrap();
                let data_root = mapset.parent().unwrap().join("data");
                *OTHER.lock().unwrap() = Some(pending::Presence::enter(&data_root, &mapset).unwrap());
            }
        }
        let scratch = Scratch::new("sweep-other-opens");
        let (mut app, _, folder) = two_layers_in(&scratch, true, Some(scratch.path("data")));
        unused_file(&app, &folder);
        app.document.as_mut().unwrap().composer.apply("Edit", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 80));
        let _turn = sweep::HOOK_TURN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *sweep::BEFORE_MOVE.lock().unwrap() = Some(open_elsewhere);
        save_through_summary(&mut app, scratch.path("backups"));
        *sweep::BEFORE_MOVE.lock().unwrap() = None;
        assert!(OTHER.lock().unwrap().is_some(), "the hook ran");
        assert!(folder.join("normal-hitclap7.wav").exists(), "another session opened the mapset, so nothing moves");
        *OTHER.lock().unwrap() = None;
        app.close_document();
        assert!(!folder.join("normal-hitclap7.wav").exists(), "the next cleanup takes it out");
    }

    #[test]
    fn an_old_mix_nothing_plays_goes_to_dawsus_removed_samples_folder() {
        let scratch = Scratch::new("sweep-trash");
        let (mut app, _, folder) = two_layers(&scratch, true);
        app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
            hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(2));
            hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
        });
        let before = mapset(&folder);
        save_through_summary(&mut app, scratch.path("backups"));
        let first: Vec<_> = mapset(&folder).into_iter().filter(|name| !before.contains(name)).collect();
        assert!(!first.is_empty());
        let bytes: Vec<_> = first.iter().map(|name| std::fs::read(folder.join(name)).unwrap()).collect();
        app.document.as_mut().unwrap().composer.apply("Louder", |hits| hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 70));
        save_through_summary(&mut app, scratch.path("backups"));
        for (name, bytes) in first.iter().zip(&bytes) {
            assert!(!folder.join(name).exists(), "{name} left the mapset");
            let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(bytes)));
            assert_eq!(&std::fs::read(kept).unwrap(), bytes, "{name} is kept in removed-samples");
        }
    }

    #[test]
    fn nothing_is_cleaned_up_while_another_session_has_the_mapset_open() {
        let scratch = Scratch::new("sweep-live");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        let other = pending::Presence::enter(&app.data_root, &folder.canonicalize().unwrap()).unwrap();
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(folder.join("normal-hitnormal2.wav").exists());
        drop(other);
    }

    #[test]
    fn a_file_changed_before_the_cleanup_is_left_alone_and_no_longer_dawsus() {
        let scratch = Scratch::new("sweep-changed");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        std::fs::write(folder.join("normal-hitnormal2.wav"), b"edited by the mapper").unwrap();
        back_to_skin(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), b"edited by the mapper");
        assert!(ledger(&app).is_empty());
    }

    #[test]
    fn a_save_with_nothing_to_save_and_a_save_of_an_earlier_revision_both_clean_up() {
        let scratch = Scratch::new("sweep-nothing-and-earlier");
        let (mut app, _, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(folder.join("normal-hitnormal2.wav").exists(), "still played");
        mix_with_skin(&mut app);
        app.save_with_backups(scratch.path("backups"));
        // A newer edit arrives while the Save runs.
        app.document.as_mut().unwrap().composer.apply("Newer", |hits| hits.set_volume(dawsu_hitsounding::MomentId(0), 90));
        wait(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.status.starts_with("Saved the earlier revision"), "{}", app.status);
        assert!(!folder.join("normal-hitnormal2.wav").exists());
        assert_eq!(app.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5], "still playable from the data folder");
    }

    #[test]
    fn a_corrupt_ledger_is_kept_aside_and_a_new_one_started() {
        let scratch = Scratch::new("ledger-corrupt");
        let (mut app, _, _) = two_layers(&scratch, true);
        let path = app.data_root.join("created-samples").join(format!("{}.json", pending::mapset_key(app.document().unwrap().path.parent().unwrap())));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(ledger(&app), ["normal-hitnormal2.wav"]);
        let key = pending::mapset_key(app.document().unwrap().path.parent().unwrap());
        let aside: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten()
            .filter(|e| { let name = e.file_name().to_string_lossy().into_owned(); name.starts_with(&key) && name.contains("corrupt") }).collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read(aside[0].path()).unwrap(), b"{ not json");
    }

    #[test]
    fn without_a_project_save_refuses_to_leave_a_new_sample_out() {
        let scratch = Scratch::new("pending-kept-no-project");
        let (mut app, map, folder) = two_layers(&scratch, false);
        app.document.as_mut().unwrap().composer.apply("Mix", |hits| {
            hits.add_hit(dawsu_hitsounding::MomentId(0), hitnormal(2));
            hits.set_hit_volume(dawsu_hitsounding::MomentId(0), &hitnormal(2), 40);
        });
        let before = (mapset(&folder), std::fs::read(&map).unwrap());
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        let error = app.error.clone().expect("refused");
        assert!(error.contains("normal-hitnormal2.wav"), "{error}");
        assert_eq!((mapset(&folder), std::fs::read(&map).unwrap()), before);
    }

    #[test]
    fn a_pending_index_another_difficulty_now_uses_refuses_save() {
        let scratch = Scratch::new("pending-index-taken");
        let (mut app, map, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        std::fs::write(folder.join("other.osu"), include_str!("../tests/project-fixture.osu").replace("0,500,4,1,0,100,1,0", "0,500,4,1,2,100,1,0")).unwrap();
        let before = std::fs::read(&map).unwrap();
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        let error = app.error.clone().expect("refused");
        assert!(error.contains("2") && error.contains("other.osu"), "{error}");
        assert_eq!(std::fs::read(&map).unwrap(), before);
        assert!(!folder.join("normal-hitnormal2.wav").exists());
    }

    #[test]
    fn a_pending_name_must_be_a_bare_file_name() {
        let scratch = Scratch::new("pending-bare-names");
        let mut pending = pending::Pending::for_mapset(&scratch.path("data"), &scratch.path("mapset"));
        for name in ["../x.wav", "a/b.wav", "a\\b.wav", "C:x.wav", "..", ""] {
            assert!(pending.add(name, TONE).is_err(), "{name}");
            assert!(pending.restore(name, &dawsu_foundation::digest::sha256_hex(TONE)).is_err(), "{name}");
        }
        assert!(pending.add("normal-hitnormal2.wav", TONE).is_ok());
    }

    #[test]
    fn slider_tick_and_whistle_lanes_on_pending_files_are_kept() {
        let scratch = Scratch::new("pending-slider-lanes");
        let map = b"osu file format v14\n[General]\nMode:0\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:2\n[TimingPoints]\n0,500,4,1,2,70,1,0\n[HitObjects]\n100,100,1000,2,2,L|300:100,1,280\n";
        let file = OsuFile::read(map.to_vec()).file;
        let mut pending = pending::Pending::for_mapset(&scratch.path("data"), &scratch.path("mapset"));
        pending.add("normal-slidertick2.wav", TONE).unwrap();
        pending.add("normal-sliderwhistle2.wav", TONE).unwrap();
        let folder = vec!["normal-slidertick2.wav".to_string(), "normal-sliderwhistle2.wav".to_string()];
        let Decompiled::Done(hits) = decompile(&file, &folder, &|| false, &mut |_, _| {}) else { panic!() };
        let needed: Vec<_> = pending::needed(&hits, &pending, &[]).into_iter().map(|file| file.name).collect();
        assert_eq!(needed, ["normal-slidertick2.wav", "normal-sliderwhistle2.wav"]);
    }

    #[test]
    fn a_sample_file_dawsu_made_that_nothing_plays_leaves_the_mapset_and_others_stay() {
        let scratch = Scratch::new("pending-sweep");
        let (mut app, map, folder) = two_layers(&scratch, true);
        // Files the sweep must leave alone: one dawsu! never made, one with a dawsu!-style name
        // it didn't make, one a mania difficulty plays and one a storyboard plays (both listed
        // as dawsu!'s).
        let canaries = [("keep.wav", b"mapper".to_vec()), ("normal-hitnormal5.wav", b"older".to_vec()),
            ("soft-hitclap6.wav", TONE.to_vec()), ("sb.wav", TONE.to_vec())];
        for (name, bytes) in &canaries {
            std::fs::write(folder.join(name), bytes).unwrap();
        }
        std::fs::write(folder.join("mania.osu"), "osu file format v14\n[General]\nMode:3\n[TimingPoints]\n0,500,4,2,6,100,1,0\n[HitObjects]\n64,192,1000,1,8,0:0:0:0:\n").unwrap();
        std::fs::write(folder.join("story.osb"), "[Events]\nSample,0,0,\"sb.wav\",100\n").unwrap();
        app.document().unwrap().pending.ledger().unwrap().add(&[
            pending::PendingSample { name: "soft-hitclap6.wav".into(), sha256: dawsu_foundation::digest::sha256_hex(TONE) },
            pending::PendingSample { name: "sb.wav".into(), sha256: dawsu_foundation::digest::sha256_hex(TONE) },
        ]).unwrap();
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert!(folder.join("normal-hitnormal2.wav").exists());
        // The layer's hit goes, but an undo state still plays its file: it stays in the mapset
        // until close (Yoge, 2026-11-21).
        app.document.as_mut().unwrap().composer.apply("Back to the skin", |hits| {
            let circle = dawsu_hitsounding::MomentId(0);
            hits.remove_hit(circle, &hitnormal(2));
            hits.add_hit(circle, hitnormal(0));
        });
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "an undo state plays it");
        assert!(!app.status.contains("Removed"), "{}", app.status);
        assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_none(), "it never went to pending-samples");
        // Undo brings the hit back, playing from the mapset, and Save keeps it.
        app.command("undo");
        assert_eq!(app.clip_for_lane(&hitnormal(2)).unwrap().samples, [0.5]);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
        app.command("redo");
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE, "Undo can still bring it back");
        // Close is strict: the undo history goes, so the file goes to removed-samples.
        app.close_document();
        assert!(!folder.join("normal-hitnormal2.wav").exists());
        let kept = app.data_root.join("removed-samples").join(format!("{}.wav", dawsu_foundation::digest::sha256_hex(TONE)));
        assert_eq!(std::fs::read(kept).unwrap(), TONE, "kept in removed-samples");
        for (name, bytes) in &canaries {
            assert_eq!(&std::fs::read(folder.join(name)).unwrap(), bytes, "{name}");
        }
        let _ = map;
    }

    #[test]
    fn a_failed_output_write_removes_the_pending_files_it_wrote() {
        let scratch = Scratch::new("pending-output-fails");
        let (mut app, _, folder) = two_layers(&scratch, false);
        use_layer_2(&mut app);
        app.generate();
        wait(&mut app);
        std::fs::write(scratch.path("backups"), b"not a folder").unwrap();
        // The generated difficulty's name is taken just before the write, so it fails.
        let target = app.output_review.as_ref().unwrap().files[0].path.clone();
        std::fs::write(&target, b"taken").unwrap();
        app.apply_output_in(scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.is_some());
        assert!(!folder.join("normal-hitnormal2.wav").exists());
        assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_some());
    }

    #[test]
    fn a_failed_osu_write_removes_the_pending_files_it_wrote() {
        let scratch = Scratch::new("pending-write-fails");
        let (mut app, map, folder) = two_layers(&scratch, true);
        use_layer_2(&mut app);
        let before = (mapset(&folder), std::fs::read(&map).unwrap());
        // The backup folder is a file, so the `.osu` write fails after the samples are written.
        std::fs::write(scratch.path("backups"), b"not a folder").unwrap();
        app.save_with_backups(scratch.path("backups"));
        wait(&mut app);
        assert!(app.error.is_some());
        assert_eq!((mapset(&folder), std::fs::read(&map).unwrap()), before);
        assert!(app.document().unwrap().pending.get("normal-hitnormal2.wav").is_some());
    }

    #[test]
    fn a_save_without_a_project_writes_the_pending_file_it_plays() {
        let scratch = Scratch::new("pending-no-project");
        let (mut app, _, folder) = two_layers(&scratch, false);
        use_layer_2(&mut app);
        save_through_summary(&mut app, scratch.path("backups"));
        assert_eq!(std::fs::read(folder.join("normal-hitnormal2.wav")).unwrap(), TONE);
        assert!(!folder.join("normal-hitnormal3.wav").exists());
    }

    #[test]
    fn the_written_file_check_refuses_a_pending_file_the_save_missed() {
        let scratch = Scratch::new("pending-guard");
        let (mut app, _, folder) = two_layers(&scratch, false);
        use_layer_2(&mut app);
        let document = app.document().unwrap();
        let out = compile_snapshot(document, document.composer.hitsounds(), false, false, &|| false).unwrap().0;
        let editor = pending::listing(&folder, &document.pending).unwrap();
        let used = pending::used(&out, &folder, &document.pending, &[]).unwrap();
        assert_eq!(used.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["normal-hitnormal2.wav"]);
        assert!(pending::check_written(&out, &folder, &["normal-hitnormal2.wav".into()], &editor, &document.pending).is_ok());
        let error = pending::check_written(&out, &folder, &[], &editor, &document.pending).unwrap_err();
        assert!(error.contains("normal-hitnormal2.wav"), "{error}");
    }

    #[test]
    fn sample_added_to_open_map_is_playable_without_losing_edits_or_redo() {
        let scratch = Scratch::new("sample-watch");
        let map = scratch.path("test.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let mut app = Application::headless();
        open(&mut app, map.clone());

        let clap = SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        {
            let composer = &mut app.document.as_mut().unwrap().composer;
            composer.add(1500.0, clap.clone(), 80, 0.0);
            composer.add(2000.0, clap, 80, 0.0);
            composer.undo();
        }
        let name = "soft-hitfinish2.wav";
        let wave = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
        std::fs::write(scratch.path(name), wave).unwrap();
        let lane = dawsu_hitsounding::rules::read_sample_file_name(name).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while app.clip_for_lane(&lane).is_none() {
            app.poll();
            assert!(Instant::now() < deadline, "new sample did not become playable");
            std::thread::sleep(Duration::from_millis(10));
        }
        let document = app.document().unwrap();
        assert!(document.files.iter().any(|file| file == name));
        assert!(document.composer.lanes.contains(&lane));
        assert!(document.composer.dirty());
        assert!(document.composer.history.can_redo());
        assert_eq!(document.composer.history.steps(), ["Draw hit", "Draw hit"]);

        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.redo();
        assert!(composer.hitsounds().lane(&lane).is_some());
        assert!(composer.hitsounds().moments().iter().any(|m| m.time().ms() == 2000.0));
        composer.add(1000.0, lane, 100, 0.0);
        let document = app.document().unwrap();
        let cues = app.scheduled_sounds(document);
        assert!(cues.iter().any(|cue| cue.at_ms == 1000.0 && cue.clip.samples == [0.5]));
        assert_eq!(std::fs::read(map).unwrap(), original);
    }

    #[test]
    fn repeating_an_endpoint_shortcut_seeks_to_the_song_boundary() {
        use dawsu_editor::keys::{Key, Mods};
        let scratch = Scratch::new("endpoint-shortcuts");
        let map = scratch.path("Hard.osu");
        let original = b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n";
        std::fs::write(&map, original).unwrap();
        let mut app = Application::headless();
        open(&mut app, map.clone());
        let press = |app: &mut Application, key| app.key(KeyChord::new(key, Mods::NONE));
        press(&mut app, Key::Z);
        assert_eq!(app.position_ms(), 1000.0);
        press(&mut app, Key::Z);
        assert_eq!(app.position_ms(), 0.0);
        press(&mut app, Key::V);
        assert_eq!(app.position_ms(), 2000.0);
        press(&mut app, Key::V);
        assert_eq!(app.position_ms(), 4000.0);
        assert!(!app.is_dirty());

        let lane = SampleIdentity::Standard {
            set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        let composer = &mut app.document.as_mut().unwrap().composer;
        composer.add(800.0, lane.clone(), 40, 0.0);
        composer.add(2200.0, lane, 60, 0.0);
        press(&mut app, Key::Z);
        assert_eq!(app.position_ms(), 800.0);
        app.last_endpoint_jump = Some((true, Instant::now() - Duration::from_millis(401)));
        press(&mut app, Key::Z);
        assert_eq!(app.position_ms(), 800.0);
        press(&mut app, Key::V);
        assert_eq!(app.position_ms(), 2200.0, "changing direction starts a fresh jump");
        assert_eq!(std::fs::read(map).unwrap(), original);
    }

    #[test]
    fn grid_alignment_shortcuts_shift_the_whole_selection_in_one_undo_step() {
        use dawsu_editor::keys::{Key, Mods};
        use dawsu_editor::tempo::SnapMode;

        let scratch = Scratch::new("grid-alignment");
        let map = scratch.path("Hard.osu");
        let original = b"osu file format v14\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n1150,360,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1200,1,0,0:0:0:0:\n256,192,1318,1,0,0:0:0:0:\n";
        std::fs::write(&map, original).unwrap();
        for (mode, next, previous) in [
            (SnapMode::Grid, [1150.0, 1320.0], [1125.0, 1295.0]),
            (SnapMode::GridRelative, [1150.0, 1320.0], [1125.0, 1295.0]),
            (SnapMode::Objects, [1200.0, 1370.0], [1000.0, 1170.0]),
        ] {
            let mut app = Application::headless();
            app.choose_key_preset("DAW");
            open(&mut app, map.clone());
            let lane = SampleIdentity::Standard {
                set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Clap),
                index: dawsu_foundation::CustomIndex::new(0),
            };
            let composer = &mut app.document.as_mut().unwrap().composer;
            composer.snap_mode = mode;
            composer.spacing = SnapSpacing::new(1, 4).unwrap();
            composer.add(1300.0, lane.clone(), 70, 0.0);
            composer.add(1130.0, lane.clone(), 40, 0.0);
            let moments = composer.hitsounds().moments();
            let selected: Vec<_> = moments.iter().enumerate()
                .filter(|(_, moment)| moment.has_hit(&lane))
                .map(|(moment, _)| HitKey { moment, lane: lane.clone() }).collect();
            composer.selected.replace(selected);
            let state = |app: &Application| {
                let mut values: Vec<_> = app.document().unwrap().composer.hitsounds().moments().iter()
                    .filter(|moment| moment.has_hit(&lane))
                    .map(|moment| (moment.time().ms(), moment.hit_volume(&lane))).collect();
                values.sort_by(|a, b| a.0.total_cmp(&b.0));
                values
            };
            let expected = |times: [f64; 2]| vec![(times[0], 40), (times[1], 70)];
            app.seek(500.0);
            app.key(KeyChord::new(Key::Space, Mods::ALT));
            assert_eq!(app.position_ms(), 1130.0);
            let mods = Mods::CTRL.union(Mods::ALT);
            app.key(KeyChord::new(Key::Right, mods));
            assert_eq!(state(&app), expected(next), "{mode:?}");
            app.key(KeyChord::new(Key::Left, mods));
            assert_eq!(state(&app), expected(previous), "{mode:?}");
            if mode == SnapMode::Objects {
                app.key(KeyChord::new(Key::Left, mods));
                assert_eq!(state(&app), expected(previous), "there is no earlier object");
            }
            app.command("undo");
            assert_eq!(state(&app), expected(next));
            app.command("undo");
            assert_eq!(state(&app), expected([1130.0, 1300.0]));
            assert_eq!(std::fs::read(&map).unwrap(), original);
        }
    }

    #[test]
    fn transport_plays_numbered_map_additions_alongside_default_hitnormal() {
        let dir = Scratch::new("addition-playback");
        let folder = dir.path("mapset");
        let defaults = dir.path("skin");
        for path in [&folder, &defaults] {
            std::fs::create_dir(path).unwrap();
        }
        let map = folder.join("test.osu");
        let fixture = include_str!("../tests/project-fixture.osu")
            .replace("256,192,1000,1,8,0:0:0:0:", "256,192,1000,1,14,1:2:2:50:");
        std::fs::write(&map, &fixture).unwrap();
        let header = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0";
        for (path, amplitude) in [
            (defaults.join("normal-hitnormal.wav"), 4096i16),
            (folder.join("soft-hitwhistle2.wav"), 8192),
            (folder.join("soft-hitfinish2.wav"), 12288),
            (folder.join("soft-hitclap2.wav"), 16384),
        ] {
            let mut wav = header.to_vec();
            wav.extend(amplitude.to_le_bytes());
            std::fs::write(path, wav).unwrap();
        }
        let loaded = load(&map, &defaults, &std::env::temp_dir().join("dawsu-headless"), &AtomicBool::new(false), None, None, None).unwrap();
        let mut app = Application::headless();
        app.document = Some(loaded.document);
        app.samples = loaded.samples;
        let mix = |app: &Application| {
            let mut mixer = dawsu_audio::Mixer::new(48_000);
            mixer.song = app.song.clone();
            mixer.song_gain = app.playback_song_gain();
            mixer.cues = app.scheduled_sounds(app.document().unwrap());
            mixer.seek(1000.0);
            mixer.playing = true;
            let mut frame = [0.0];
            mixer.render(&mut frame, 1);
            (mixer.cues.len(), frame[0])
        };
        assert_eq!(mix(&app), (4, 0.625));
        let clap = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: dawsu_foundation::CustomIndex::new(2),
        };
        let clip = app.clip_for_lane(&clap).unwrap();
        assert_eq!(clip.samples[0], 0.5);
        app.document.as_mut().unwrap().composer.listen_muted.push(HitKey {
            moment: 0,
            lane: clap.clone(),
        });
        assert_eq!(mix(&app), (3, 0.375));

        app.document.as_mut().unwrap().composer.listen_muted.clear();
        app.song = Some(Arc::new(AudioClip {
            samples: vec![0.25; 48_002],
            channels: 1,
            sample_rate: 48_000,
        }));
        app.preferences.song_gain = 1.0;
        assert_eq!(mix(&app), (4, 0.875));
        app.set_song_solo(true);
        assert_eq!(mix(&app), (0, 0.25));
        app.solo(clap.clone());
        assert_eq!(mix(&app), (1, 0.5));
        app.preferences.song_muted = true;
        assert_eq!(mix(&app), (1, 0.25), "explicit song mute takes priority");
        app.preferences.song_muted = false;
        app.set_song_solo(false);
        assert_eq!(mix(&app), (1, 0.25), "lane solo suppresses the unsoloed song");
        app.solo(clap);
        assert_eq!(mix(&app), (4, 0.875));
        assert!(!app.is_dirty());
        assert_eq!(std::fs::read(map).unwrap(), fixture.as_bytes());
    }

    #[test]
    fn time_signatures_change_editor_bars_without_resnapping_or_writing_map_bytes() {
        let scratch = Scratch::new("editor-meter");
        let map = scratch.path("Hard.osu");
        let sessions = scratch.path("sessions");
        let original = b"osu file format v14\r\n\r\n[General]\r\nMode:0\r\n\r\n[TimingPoints]\r\n0,500,4,1,1,100,1,0\r\n4000,250,4,1,1,100,1,0\r\n\r\n[HitObjects]\r\n256,192,2000,1,0,0:0:0:0:\r\n";
        std::fs::write(&map, original).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(sessions.clone());
        open(&mut app, map.clone());
        app.set_time_signatures(vec![EditorTimeSignature { bar: 2, numerator: 7, denominator: 8 }]).unwrap();
        let document = app.document().unwrap();
        let point = SongTime::from_ms(3750.0);
        let (bar, meter) = document.editor_bar_at(point);
        assert_eq!((bar.bar, bar.beat, meter), (3, 1, (7, 8)));
        let (bar_at_red, red_meter) = document.editor_bar_at(SongTime::from_ms(4000.0));
        assert_eq!((bar_at_red.bar, bar_at_red.beat, red_meter), (4, 1, (7, 8)));
        assert_eq!(document.composer.tempo.snap(point, SnapSpacing::new(1, 1).unwrap()).ms(), 4000.0);
        assert!(!app.is_dirty());
        app.save_session().unwrap();
        let mut reopened = Application::headless();
        reopened.project_root = Some(sessions);
        open(&mut reopened, map.clone());
        assert_eq!(reopened.document().unwrap().editor_bar_at(point), (bar, meter));
        assert_eq!(std::fs::read(&map).unwrap(), original);
    }

    #[test]
    fn hs_mode_is_restored_per_difficulty_without_writing_map_bytes() {
        let fixture = Scratch::new("hs-mode-state");
        let a = fixture.path("Hard.osu");
        let b = fixture.path("Easy.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        for path in [&a, &b] {
            std::fs::write(path, original).unwrap();
        }
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("sessions"));
        open(&mut app, a.clone());
        app.set_hs_mode(true);
        assert!(!app.is_dirty());
        open(&mut app, b.clone());
        assert!(!app.document().unwrap().hs_mode);
        open(&mut app, a.clone());
        assert!(app.document().unwrap().hs_mode);
        app.set_hs_mode(false);
        app.save_session().unwrap();
        let mut reopened = Application::headless();
        reopened.project_root = app.project_root.clone();
        open(&mut reopened, a.clone());
        assert!(!reopened.document().unwrap().hs_mode);
        for path in [a, b] {
            assert_eq!(std::fs::read(path).unwrap(), original);
        }
    }

    #[test]
    fn new_hitsound_difficulty_opens_with_hs_mode_enabled_by_default() {
        let fixture = Scratch::new("new-diff-hs-mode");
        let source_path = fixture.path("Artist - Title [Hard].osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&source_path, original).unwrap();
        let sessions = fixture.path("sessions");

        let mut app = Application::headless();
        app.project_root = Some(sessions.clone());
        open(&mut app, source_path);
        assert!(!app.document().unwrap().hs_mode);

        app.new_diff("Hitsounds".into());
        let wait = |app: &mut Application| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(app.error.is_none(), "{:?}", app.error);
        };
        wait(&mut app);
        let plan = app.output_review.as_ref().expect("new_diff prepares write plan");
        let created = plan.files[0].path.clone();
        app.apply_output();
        wait(&mut app);
        assert!(created.exists());

        let mut reopened = Application::headless();
        reopened.project_root = Some(sessions);
        open(&mut reopened, created);
        assert!(
            reopened.document().unwrap().hs_mode,
            "new hitsound difficulty should open with hitsounding mode enabled by default"
        );
    }

    #[test]
    fn reopened_hs_ownership_removes_only_created_circles_and_survives_undo() {
        let scratch = Scratch::new("hs-circle-ownership");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("test.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let lane = SampleIdentity::File("voice.wav".into());
        let wav = b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40";
        std::fs::write(folder.join("voice.wav"), wav).unwrap();

        let launch = || {
            let mut app = Application::headless();
            app.project_root = Some(scratch.path("projects"));
            open(&mut app, map.clone());
            app
        };
        let wait = |app: &mut Application| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(app.error.is_none(), "{:?}", app.error);
        };
        let save = |app: &mut Application| {
            app.save_with_backups(scratch.path("backups"));
            wait(app);
        };
        let mut app = launch();
        app.set_hs_mode(true);
        // It rounds to the original, also centered circle's file time.
        app.document.as_mut().unwrap().composer.add(1000.25, lane.clone(), 67, 0.0);
        save(&mut app);
        let saved = std::fs::read(&map).unwrap();
        assert_eq!(app.document().unwrap().file.hit_objects().len(), 2);

        let erase = |app: &mut Application| {
            let composer = &mut app.document.as_mut().unwrap().composer;
            let id = composer.hitsounds().moments().iter().position(|m| m.has_hit(&lane)).unwrap();
            composer.apply("Erase owned hit", |hits| hits.remove_hit(dawsu_hitsounding::MomentId(id), &lane));
        };
        let mut reopened = launch();
        erase(&mut reopened);
        let doc = reopened.document().unwrap();
        let (reviewed, problems, _) = compile_for_save(
            &doc.compile_base, doc.composer.hitsounds(), true, &doc.owned_hs_circles, true, &|| false, &mut |_, _| {},
        ).unwrap();
        assert!(problems.is_empty());
        assert_eq!(reviewed.to_bytes(), original);
        assert_eq!(std::fs::read(&map).unwrap(), saved);
        save(&mut reopened);
        assert_eq!(std::fs::read(&map).unwrap(), original);
        reopened.command("undo");
        save(&mut reopened);
        assert_eq!(std::fs::read(&map).unwrap(), saved);

        let mut external = saved.clone();
        external.extend(b"// This map was edited elsewhere\n");
        std::fs::write(&map, &external).unwrap();
        let mut stale = launch();
        erase(&mut stale);
        stale.save_with_backups(scratch.path("backups"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while stale.busy {
            stale.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        // The circle is not provably HS mode's, and the compiler cannot encode an empty gameplay
        // circle: Save leaves it out with its old hit, lists it, and writes the rest.
        assert!(stale.error.is_none(), "{:?}", stale.error);
        assert!(stale.save_problem_review());
        let problems = stale.output_problems();
        assert_eq!(problems.len(), 1);
        assert!(!problems[0].message.contains("sample mix"), "{}", problems[0].message);
        stale.answer_save_review(true);
        wait(&mut stale);
        assert!(stale.error.is_none(), "{:?}", stale.error);
        assert!(!stale.is_dirty());
        assert_eq!(std::fs::read(&map).unwrap(), external);
        assert_eq!(stale.document().unwrap().file.hit_objects().len(), 2);
        assert_eq!(std::fs::read(folder.join("voice.wav")).unwrap(), wav);
    }

    #[test]
    fn hs_save_mix_reopens_without_duplicate_project_hits() {
        let scratch = Scratch::new("hs-save-mix");
        let folder = scratch.path("mapset");
        std::fs::create_dir(&folder).unwrap();
        let map = folder.join("a.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let wave_header = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0";
        for (name, frames) in [
            ("normal-hitnormal2.wav", [8192i16, 0]),
            ("soft-hitclap3.wav", [0, 8192i16]),
        ] {
            let mut wav = wave_header.to_vec();
            wav.extend(frames.iter().flat_map(|value| value.to_le_bytes()));
            std::fs::write(folder.join(name), wav).unwrap();
        }
        let mut app = Application::headless();
        app.project_root = Some(scratch.path("projects"));
        open(&mut app, map.clone());
        let composer = &mut app.document.as_mut().unwrap().composer;
        // Distinct unanchored moments round to one file time, with two different gains.
        composer.add(10_000.0, SampleIdentity::File("normal-hitnormal2.wav".into()), 80, 0.0);
        composer.add(10_000.25, SampleIdentity::File("soft-hitclap3.wav".into()), 40, 0.0);
        // Save the loose hits once so the HS save must clear the now-materialised project rows.
        let save = |app: &mut Application| {
            app.save_with_backups(scratch.path("backups"));
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(app.error.is_none(), "{:?}", app.error);
        };
        save(&mut app);
        assert_eq!(std::fs::read(&map).unwrap(), original);
        app.set_hs_mode(true);
        save(&mut app);
        assert!(!app.is_dirty());
        let saved = std::fs::read(&map).unwrap();
        assert!(saved.starts_with(original));
        let restart = |app: &mut Application| {
            app.project_root = Some(scratch.path("projects"));
            open(app, map.clone());
        };
        let mut reopened = Application::headless();
        restart(&mut reopened);
        let document = reopened.document().unwrap();
        let hits = document.composer.hitsounds();
        assert_eq!(project_hits(document).len(), 0);
        let [moment] = hits.moments().iter()
            .filter(|moment| moment.time().ms() >= 10_000.0)
            .collect::<Vec<_>>()[..] else {
            panic!("expected exactly one saved circle");
        };
        // Both sample files play as the hitnormal, so Save mixes them into one hitnormal file at
        // the louder one's volume.
        let [SampleIdentity::Standard { set: SampleSet::Normal, sound: Sound::Hit(Hitsound::Normal), index }] = moment.hits().collect::<Vec<_>>()[..] else {
            panic!("expected one hitnormal");
        };
        assert_eq!(moment.volume(), 80);
        let mix = dawsu_audio::decode(&folder.join(format!("normal-hitnormal{}.wav", index.get()))).unwrap();
        assert_eq!(mix.samples.len(), 4);
        for (actual, expected) in mix.samples.iter().zip([0.25, 0.25, 0.125, 0.125]) {
            assert!((actual - expected).abs() < 0.0001, "{actual} vs {expected}");
        }
        save(&mut app);
        save(&mut reopened);
        assert_eq!(std::fs::read(&map).unwrap(), saved);
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 4);
    }


    #[test]
    fn loose_lanes_reopen_as_one_moment_beside_an_unrelated_object() {
        let dir = Scratch::new("grouped-project-hits");
        let path = dir.path("map.osu");
        std::fs::write(&path, include_bytes!("../tests/project-fixture.osu")).unwrap();
        let mut doc = Document::read(&path).unwrap();
        let rows = ["Skin|soft-hitclap", "Skin|soft-hitwhistle"].map(|name| project::ProjectHit {
            lane: name.into(),
            time: 1000.0,
            volume: Some(75),
            group: None,
            default_volume: None,
        });
        restore_project_hits(&mut doc.composer, &rows).unwrap();
        let grouped = project_hits(&doc);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].group, grouped[1].group);
        let mut reopened = Document::read(&path).unwrap();
        restore_project_hits(&mut reopened.composer, &grouped).unwrap();
        let loose: Vec<_> = reopened
            .composer
            .hitsounds()
            .moments()
            .iter()
            .filter(|m| m.place() == dawsu_hitsounding::Place::NoObject)
            .collect();
        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].hits().count(), 2);
    }

    #[test]
    fn legacy_clap_survives_edit_save_reopen_and_deletion() {
        let dir = Scratch::new("legacy-clap");
        let map = dir.path("a.osu");
        let original = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, original).unwrap();
        let path = map.canonicalize().unwrap();
        let legacy = dir.path("legacy");
        let root = dir.path("sessions");
        std::fs::create_dir(&legacy).unwrap();
        let key = path
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_lowercase();
        let hash = dawsu_foundation::digest::sha256_hex(key.as_bytes());
        let source = legacy.join(format!("A-{}.json", hash[..12].to_uppercase()));
        let bytes =
            br#"{"Version":3,"Unanchored":[{"Lane":"Skin|soft-hitclap","Time":250,"Volume":63}]}"#;
        std::fs::write(&source, bytes).unwrap();
        let load_map = || {
            load(
                &path,
                dir.dir(),
                &root,
                &AtomicBool::new(false),
                Some(&root),
                Some(&legacy),
                None,
            )
            .unwrap()
        };
        let mut app = Application::headless();
        let loaded = load_map();
        app.project = loaded.project;
        app.document = Some(loaded.document);
        let hit = project_hits(app.document().unwrap()).pop().unwrap();
        assert_eq!(
            (hit.lane.as_str(), hit.time, hit.volume),
            ("Skin|soft-hitclap", 250.0, Some(63))
        );
        assert!(!app.is_dirty());
        let c = &mut app.document.as_mut().unwrap().composer;
        let last = c.hitsounds().moments().len() - 1;
        let lane = c.hitsounds().moments()[last].hits().next().unwrap().clone();
        c.selected.replace([HitKey { moment: last, lane }]);
        app.selection_volume(77, false);
        app.save();
        let wait = |app: &mut Application| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.busy {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert!(app.error.is_none(), "{:?}", app.error);
        };
        wait(&mut app);
        assert_eq!(project_hits(&load_map().document)[0].volume, Some(77));
        app.document.as_mut().unwrap().composer.remove_selected();
        app.save();
        wait(&mut app);
        assert!(project_hits(&load_map().document).is_empty());
        assert_eq!(std::fs::read(source).unwrap(), bytes);
        assert_eq!(std::fs::read(map).unwrap(), original);
    }

    #[test]
    fn session_conflict_keeps_the_outgoing_map_until_explicit_revert() {
        let fixture = Scratch::new("switch-conflict");
        let map = fixture.path("a.osu");
        let other = fixture.path("b.osu");
        let root = fixture.path("sessions");
        for path in [&map, &other] {
            std::fs::write(path, include_bytes!("../tests/project-fixture.osu")).unwrap();
        }
        let mut app = Application::headless();
        app.project_root = Some(root.clone());
        open(&mut app, map.clone());
        app.song_offset = 5.0;
        app.save_session().unwrap();
        let mut external =
            project::Project::load(&root, None, &map.canonicalize().unwrap()).unwrap();
        external
            .save(project::SessionSettings {
                song_offset: 15.0,
                ..external.settings.clone()
            })
            .unwrap();
        app.song_offset = 10.0;
        app.request_open(other);
        assert!(!app.busy);
        assert_eq!(app.document().unwrap().path, map.canonicalize().unwrap());
        assert_eq!(app.song_offset, 10.0);
        assert!(app.error.take().unwrap().contains("changed on disk"));
        app.revert_requested = true;
        app.confirm_revert();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(app.song_offset, 15.0);
    }

    #[test]
    fn difficulty_switch_protects_inspector_edits() {
        let fixture = Scratch::new("switch-inspector");
        let a = fixture.path("a.osu");
        let b = fixture.path("b.osu");
        for path in [&a, &b] {
            std::fs::write(path, include_bytes!("../tests/project-fixture.osu")).unwrap();
        }
        let mut app = Application::headless();
        open(&mut app, a);
        app.document.as_mut().unwrap().composer.select_all();
        app.request_open(b);
        assert!(app.busy);
        app.selection_volume(7, false);
        assert!(!app.is_dirty(), "the inspector edited an outgoing document");
    }

    #[test]
    fn reverting_reloads_disk_and_requires_an_explicit_confirm() {
        let fixture = Scratch::new("revert-workflow");
        let path = fixture.path("map.osu");
        let data = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&path, data).unwrap();
        let mut app = Application::headless();
        open(&mut app, path.clone());
        app.document.as_mut().unwrap().composer.select_all();
        app.command("volumeDown");
        assert!(app.is_dirty());
        app.command("revert");
        assert!(app.revert_requested);
        assert!(app.is_dirty());
        app.confirm_revert();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.is_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), data);
    }

    #[test]
    fn reopening_a_difficulty_restores_its_session_without_changing_the_map() {
        let fixture = Scratch::new("project-workflow");
        let map = fixture.path("map.osu");
        let other = fixture.path("other.osu");
        let data = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, data).unwrap();
        std::fs::write(&other, data).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("application-data"));
        open(&mut app, map.clone());
        app.song_offset = -42.5;
        app.locator_left = 100.0;
        app.locator_right = 900.0;
        app.cycle = true;
        app.zoom = 0.35;
        app.set_view_start(125.0);
        open(&mut app, other);
        assert_eq!(app.song_offset, DEFAULT_SONG_OFFSET_MS);
        open(&mut app, map.clone());
        assert_eq!(app.song_offset, -42.5);
        assert_eq!(
            (app.locator_left, app.locator_right, app.cycle),
            (100.0, 900.0, true)
        );
        assert_eq!(app.zoom, 0.35);
        assert_eq!(app.document().unwrap().composer.view_start, 125.0);
        assert_eq!(std::fs::read(&map).unwrap(), data);
    }

    #[test]
    fn protect_hitnormals_is_saved_like_the_other_preferences() {
        let fixture = Scratch::new("protect-normals-preference");
        let data = fixture.path("data");
        let mut app = Application::create(Preferences::default(), false, None, None, true, data.clone());
        app.set_normal_protection(true);
        assert!(app.error.is_none(), "{:?}", app.error);
        let (saved, _) = preferences::load(&data.join("preferences.json"), None);
        assert!(saved.protect_hitnormals);
    }

    #[test]
    fn a_snap_spacing_osu_cannot_use_is_reported_and_changes_nothing() {
        let fixture = Scratch::new("mixed-snap");
        let map = fixture.path("map.osu");
        std::fs::write(&map, include_bytes!("../tests/project-fixture.osu")).unwrap();
        let mut app = Application::headless();
        open(&mut app, map);
        let spacing = |app: &Application| {
            let composer = &app.document().unwrap().composer;
            (composer.snap_enabled, composer.spacing.beats(), composer.spacing.divisions())
        };
        let before = spacing(&app);
        app.set_snap_options(!before.0, 2, 3, dawsu_editor::tempo::SnapMode::Grid);
        assert_eq!(
            app.error.take().as_deref(),
            Some("Snap 2/3 is not available. Use a fraction of a beat, like 1/4, or whole beats, like 2/1.")
        );
        assert_eq!(spacing(&app), before);
        app.set_snap_options(true, 1, 3, dawsu_editor::tempo::SnapMode::Grid);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(spacing(&app), (true, 1, 3));
    }

    /// P and Alt+P (Cubase's Locators to Selection and Loop Selection) were in the key list but
    /// did nothing: no command ran for them.
    #[test]
    fn locators_follow_the_selected_hits_or_the_range() {
        let fixture = Scratch::new("locators-to-selection");
        let path = fixture.path("map.osu");
        let map = b"osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,8,0:0:0:0:\n256,192,2500,1,8,0:0:0:0:\n";
        std::fs::write(&path, map).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("sessions"));
        open(&mut app, path.clone());
        app.locator_left = 100.0;
        app.locator_right = 200.0;
        let locators = |app: &Application| (app.locator_left, app.locator_right, app.cycle);

        // Nothing selected and no range: the locators stay where they are.
        app.command("locatorsToSelection");
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(locators(&app), (100.0, 200.0, false));

        let clap = |moment: usize| {
            let moments = app.document().unwrap().composer.hitsounds().moments();
            let lane = moments[moment].hits().find(|lane| matches!(lane,
                SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Clap), .. })).unwrap().clone();
            HitKey { moment, lane }
        };
        let (first, second) = (clap(0), clap(1));
        app.select_hit(first.clone(), false);
        app.select_hit(second, true);
        app.command("locatorsToSelection");
        assert_eq!(locators(&app), (1000.0, 2500.0, false), "the span of the selected hits");

        // One hit has no length: R goes to the next grid line after it.
        app.select_hit(first, false);
        app.command("locatorsToSelection");
        let composer = &app.document().unwrap().composer;
        let next = composer.tempo.grid_step(SongTime::from_ms(1000.0), 1, composer.spacing).ms();
        assert!(next > 1000.0);
        assert_eq!(locators(&app), (1000.0, next, false));

        // A range left from the Range tool does not beat the hits selected since.
        app.document.as_mut().unwrap().composer.range = Some(dawsu_editor::tools::TimeRange {
            from: SongTime::from_ms(1750.0), to: SongTime::from_ms(1250.0), from_row: 0, to_row: 1,
        });
        app.command("locatorsToSelection");
        assert_eq!(locators(&app), (1000.0, next, false));
        // With the Range tool in hand, its range wins.
        app.command("toolRange");
        app.command("locatorsToSelection");
        assert_eq!(locators(&app), (1250.0, 1750.0, false));
        app.command("toolSelect");

        // Alt+P: the same, with Cycle on and playback from L.
        app.document.as_mut().unwrap().composer.range = None;
        app.command("playLocatorRange");
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(locators(&app), (1000.0, next, true));
        assert_eq!(app.playback_start, 1000.0);
        assert_eq!(std::fs::read(&path).unwrap(), map, "the map file is untouched");
    }

    #[test]
    fn fit_zoom_of_a_30_minute_song_survives_zoom_out_and_reopening() {
        let fixture = Scratch::new("fit-zoom");
        let map = fixture.path("map.osu");
        let other = fixture.path("other.osu");
        let data = include_bytes!("../tests/project-fixture.osu");
        std::fs::write(&map, data).unwrap();
        std::fs::write(&other, data).unwrap();
        let mut app = Application::headless();
        app.project_root = Some(fixture.path("application-data"));
        open(&mut app, map.clone());
        // Fit shows a 30-minute song in a 1000 px timeline.
        let fit = 1000.0 / (30.0 * 60_000.0);
        app.zoom = fit;
        app.command("zoomOut");
        assert!(app.zoom <= fit, "Zoom out zoomed in to {}", app.zoom);
        app.zoom = fit;
        open(&mut app, other);
        open(&mut app, map);
        assert_eq!(app.zoom, fit);
    }
}

#[cfg(test)]
mod sample_file_clip {
    use super::*;
    use dawsu_test_support::Scratch;

    fn wave(frames: u16) -> Vec<u8> {
        let data = frames as u32 * 2;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data).to_le_bytes());
        bytes.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data");
        bytes.extend(data.to_le_bytes());
        bytes.extend((0..frames).flat_map(|i| (i as i16 * 100).to_le_bytes()));
        bytes
    }

    #[test]
    fn a_library_sample_is_decoded_once_for_its_audition_and_its_outline_until_the_file_changes() {
        let scratch = Scratch::new("sample-file-clip");
        let path = scratch.path("clap.wav");
        std::fs::write(&path, wave(4)).unwrap();
        let mut app = Application::headless();
        app.audition_sample_file(&path).unwrap();
        let first = app.sample_file_clip(&path).unwrap();
        let (ms, _) = app.sample_file_outline(&path, 4).unwrap();
        assert!(Arc::ptr_eq(&first, &app.sample_file_clip(&path).unwrap()), "the outline reused the audition's clip");
        assert!((ms - 4000.0 / 48000.0).abs() < 1e-9);
        std::fs::write(&path, wave(8)).unwrap();
        let (ms, _) = app.sample_file_outline(&path, 4).unwrap();
        assert!((ms - 8000.0 / 48000.0).abs() < 1e-9, "a changed file is decoded again");
    }
}

#[cfg(test)]
mod local_default_samples {
    use super::*;
    use dawsu_test_support::Scratch;

    #[test]
    fn starting_from_a_data_folder_takes_default_sounds_from_its_preferences_only() {
        let scratch = Scratch::new("data-root-defaults");
        let app = Application::with_data_root(scratch.path("data"));
        assert!(
            app.preferences.default_samples.as_os_str().is_empty(),
            "no folder outside the data folder: {}",
            app.preferences.default_samples.display()
        );
    }

    #[test]
    fn the_desktop_start_uses_the_local_default_sounds_only_when_none_are_saved() {
        let scratch = Scratch::new("local-defaults");
        let local = scratch.path("local");
        std::fs::create_dir(&local).unwrap();

        let mut app = Application::headless();
        app.use_local_default_samples(&scratch.path("missing"));
        assert!(app.preferences.default_samples.as_os_str().is_empty(), "a missing folder is not used");

        app.use_local_default_samples(&local);
        assert_eq!(app.preferences.default_samples, local);

        let saved = scratch.path("saved");
        app.preferences.default_samples = saved.clone();
        app.use_local_default_samples(&local);
        assert_eq!(app.preferences.default_samples, saved, "a saved folder is kept");
    }
}

#[cfg(test)]
mod shipped_default_sounds {
    use super::*;
    use dawsu_foundation::CustomIndex;
    use dawsu_test_support::Scratch;

    fn skin(set: SampleSet, sound: Sound, index: u32) -> Lane {
        Lane { identity: SampleIdentity::Standard { set, sound, index: CustomIndex::new(index) }, source: LaneSource::Skin }
    }

    fn path_for(lane: &Lane, defaults: &Path) -> Result<PathBuf, String> {
        let folder = defaults.parent().unwrap();
        sample_path(lane, folder, &pending::Pending::default(), defaults, &pending::sample_folder(defaults))
    }

    #[test]
    fn a_default_sound_the_folder_lacks_plays_osus_own_shipped_sound() {
        let scratch = Scratch::new("shipped-defaults");
        let defaults = scratch.path("defaults");
        std::fs::create_dir(&defaults).unwrap();
        for (lane, name) in [
            (skin(SampleSet::Soft, Sound::Slider(SliderSound::Slide), 0), "soft-sliderslide.wav"),
            (skin(SampleSet::Drum, Sound::Slider(SliderSound::Tick), 3), "drum-slidertick.wav"),
            (skin(SampleSet::Normal, Sound::Slider(SliderSound::Whistle), 0), "normal-sliderwhistle.wav"),
            (skin(SampleSet::Normal, Sound::Hit(Hitsound::Clap), 0), "normal-hitclap.wav"),
        ] {
            let path = path_for(&lane, &defaults).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(path, default_sounds::path(name).unwrap(), "{name}");
            dawsu_audio::decode(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        // No folder chosen at all: still osu!'s sound.
        let none = scratch.path("not-chosen");
        let slide = skin(SampleSet::Soft, Sound::Slider(SliderSound::Slide), 0);
        assert_eq!(path_for(&slide, &none).unwrap(), default_sounds::path("soft-sliderslide.wav").unwrap());
    }

    #[test]
    fn the_chosen_default_sounds_folder_comes_before_the_shipped_sound() {
        let scratch = Scratch::new("shipped-defaults-order");
        let defaults = scratch.path("defaults");
        std::fs::create_dir(&defaults).unwrap();
        std::fs::write(defaults.join("Soft-SliderSlide.ogg"), b"OggS").unwrap();
        std::fs::write(defaults.join("drum-hitnormal.wav"), b"RIFF").unwrap();
        let slide = skin(SampleSet::Soft, Sound::Slider(SliderSound::Slide), 0);
        assert_eq!(path_for(&slide, &defaults).unwrap(), defaults.join("Soft-SliderSlide.ogg").canonicalize().unwrap());
        let kick = skin(SampleSet::Drum, Sound::Hit(Hitsound::Normal), 0);
        assert_eq!(path_for(&kick, &defaults).unwrap(), defaults.join("drum-hitnormal.wav").canonicalize().unwrap());
    }
}

#[cfg(test)]
mod async_song_decode {
    use super::*;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    fn test_wav(frames: u32) -> Vec<u8> {
        let data = frames * 2;
        let mut wav = b"RIFF".to_vec();
        wav.extend((36 + data).to_le_bytes());
        wav.extend(b"WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data");
        wav.extend(data.to_le_bytes());
        for i in 0..frames {
            let sample = (((i % 100) as f32 / 100.0) * 16_000.0) as i16;
            wav.extend(sample.to_le_bytes());
        }
        wav
    }

    fn wait_open(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline, "open job timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    fn wait_song(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.song_loading {
            app.poll();
            assert!(Instant::now() < deadline, "song decode job timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn document_is_open_and_editable_before_song_ready_and_pending_play_starts_on_arrival() {
        let scratch = Scratch::new("async-song-ready");
        let map = scratch.path("Map.osu");
        std::fs::write(
            &map,
            b"osu file format v14\n[General]\nMode:0\nAudioFilename: song.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
        )
        .unwrap();
        // 4 seconds of audio so song duration (4000 ms) exceeds document duration (3000 ms).
        std::fs::write(scratch.path("song.wav"), test_wav(48_000 * 4)).unwrap();

        let hold = Arc::new(AtomicBool::new(true));
        let mut app = Application::headless();
        app.song_decode_hold = Some(hold.clone());
        app.request_open(map);
        wait_open(&mut app);

        assert!(app.document().is_some());
        assert!(!app.busy);
        assert!(app.song_loading);
        assert!(app.song.is_none());
        assert!(app.waveform.is_empty());
        assert_eq!(app.duration_ms(), 3000.0);

        // Edits work immediately while the song is still decoding.
        let hit_lane = app
            .document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()[0]
            .hits()
            .next()
            .unwrap()
            .clone();
        app.select_hit(HitKey { moment: 0, lane: hit_lane }, false);
        app.selection_volume(55, false);
        assert!(app.is_dirty());
        assert_eq!(
            app.document().unwrap().composer.hitsounds().moments()[0].volume(),
            55
        );

        // Pressing Play while song_loading queues playback rather than starting out of sync.
        app.play_pause_from(Some(250.0));
        assert!(app.pending_play_on_song);
        assert_eq!(app.started_play_count, 0);
        assert_eq!(app.status, "Loading song…");
        assert_eq!(app.position_ms(), 250.0);

        let rev_before = app.waveform_revision();
        hold.store(false, Ordering::Relaxed);
        wait_song(&mut app);

        assert!(!app.song_loading);
        assert!(app.song.is_some());
        assert!(!app.waveform.is_empty());
        assert!(app.waveform_revision() > rev_before);
        assert!(!app.pending_play_on_song);
        assert_eq!(app.started_play_count, 1);
        assert!((app.duration_ms() - 4000.0).abs() < 1.0);
        assert!((app.locator_right - 4000.0).abs() < 1.0);
        assert!(app.is_dirty());
    }

    #[test]
    fn opening_a_second_map_cancels_first_decode_and_drops_late_song_ready() {
        let scratch = Scratch::new("async-song-generation");
        let first = scratch.path("First.osu");
        let second = scratch.path("Second.osu");
        std::fs::write(
            &first,
            b"osu file format v14\n[General]\nMode:0\nAudioFilename: song1.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
        )
        .unwrap();
        std::fs::write(
            &second,
            b"osu file format v14\n[General]\nMode:0\nAudioFilename: song2.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
        )
        .unwrap();
        std::fs::write(scratch.path("song1.wav"), test_wav(48_000)).unwrap();
        std::fs::write(scratch.path("song2.wav"), test_wav(48_000 * 2)).unwrap();

        let hold = Arc::new(AtomicBool::new(true));
        let mut app = Application::headless();
        app.song_decode_hold = Some(hold.clone());
        app.request_open(first);
        wait_open(&mut app);
        let gen1 = app.document_generation();
        let first_cancel = app.song_cancel.clone();
        assert!(!first_cancel.load(Ordering::Relaxed));

        // Release hold as we switch to the second difficulty.
        hold.store(false, Ordering::Relaxed);
        app.request_open(second);
        assert!(first_cancel.load(Ordering::Relaxed), "switching maps cancels old song decode");
        wait_open(&mut app);
        wait_song(&mut app);

        let gen2 = app.document_generation();
        assert_ne!(gen1, gen2);
        let second_wf_len = app.waveform.len();
        let second_rev = app.waveform_revision();
        assert!((app.duration_ms() - 2000.0).abs() < 1.0);

        // Inject a stale SongReady from the first generation and ensure it is dropped cleanly.
        app.pending_play_on_song = false;
        let stale_clip = Arc::new(AudioClip {
            samples: vec![0.5; 48_000 * 5],
            channels: 1,
            sample_rate: 48_000,
        });
        app.apply_song_job_result(JobResult::SongReady {
            generation: gen1,
            song: Some(stale_clip.clone()),
            waveform: vec![WaveBin { peak: 0.9, low: 0.1, mid: 0.2, high: 0.3, amp_l: 0.9, amp_r: 0.9 }; 3],
            warning: Some("stale warning".into()),
        });
        assert_eq!(app.waveform.len(), second_wf_len);
        assert_eq!(app.waveform_revision(), second_rev);
        assert!(!Arc::ptr_eq(app.song.as_ref().unwrap(), &stale_clip));
        assert!(app.warnings.is_empty());
    }

    #[test]
    fn play_from_while_song_loading_waits_and_stop_clears_pending_play() {
        let scratch = Scratch::new("async-song-stop");
        let map = scratch.path("Map.osu");
        std::fs::write(
            &map,
            b"osu file format v14\n[General]\nMode:0\nAudioFilename: song.wav\n[TimingPoints]\n0,500,4,1,1,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
        )
        .unwrap();
        std::fs::write(scratch.path("song.wav"), test_wav(48_000 * 2)).unwrap();

        let hold = Arc::new(AtomicBool::new(true));
        let mut app = Application::headless();
        app.song_decode_hold = Some(hold.clone());
        app.request_open(map);
        wait_open(&mut app);

        app.play_from(400.0);
        assert!(app.pending_play_on_song);
        assert_eq!(app.status, "Loading song…");
        app.stop();
        assert!(!app.pending_play_on_song);

        hold.store(false, Ordering::Relaxed);
        wait_song(&mut app);
        assert_eq!(app.started_play_count, 0);
        assert_eq!(app.status, "Difficulty loaded");
    }
}
