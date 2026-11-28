//! The Export window's preview, kept apart from main's job, error, problem list and review.
use crate::export_manifest::{self, ExportWriteMode, SingleSampleFormat};
use crate::{Application, OutputProblem, output};
use dawsu_hitsounding::copy::CopyOptions;
use dawsu_hitsounding::index_packing::AdditionOnlyNormal;
use dawsu_hitsounding::tick_render::TickPolicy;
use dawsu_hitsounding::volume_envelope::{VolumeEnvelope, apply_envelope_to_osu};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

fn default_true() -> bool {
    true
}

/// What the Export window asks to write: "Into the mapset" (Direct Copy/Generate) or an isolated
/// standalone Hitsound-only folder outside the mapset.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    pub targets: Vec<PathBuf>,
    pub copy: CopyOptions,
    /// Also write the base hitsound difficulty, as Generate does.
    pub base: bool,
    /// Generate's slider slide, whistle and tick audio, for the base difficulty.
    pub slider_audio: bool,
    /// Opt-in: auto-fix osu! RC health issues (late impact, <25 ms, <1024 B, bloated silence)
    /// on newly rendered/staged mixes. Default `false` preserves exact raw PCM output.
    pub auto_fix_staged_mixes: bool,
    /// Audio format for generated/copied hitsound mix files.
    pub mix_format: crate::MixFormat,
    /// Direct mapset write (with backups) or isolated standalone hitsound-only folder.
    pub write_mode: ExportWriteMode,
    /// Optional explicit directory for `ExportWriteMode::HitsoundOnly`.
    pub hitsound_only_dir: Option<PathBuf>,
    /// Keep unchanged single custom samples byte-identical (`Source`) or re-encode to `mix_format`.
    pub single_format: SingleSampleFormat,
    /// What hitnormal accompanies addition-only moments (whistle/finish/clap without a normal).
    pub addition_only_normal: AdditionOnlyNormal,
    /// Whether unmatched slider ticks are ignored or rendered as short green-line `slidertick` samples.
    pub tick_policy: TickPolicy,
    /// Allow repeat exports to allocate new custom indices when new mixes appear.
    pub allow_schema_growth: bool,
    /// Automatically archive stale dawsu!-created sample mixes on repeat direct exports.
    pub auto_cleanup_stale: bool,
    /// Opt-in: allow cleanup to also archive existing mapper-owned unused samples (with backup first).
    pub take_over_samples: bool,
    /// Apply the mapset's or difficulty's green-line-only master volume envelope during export.
    pub apply_volume_envelope: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            copy: CopyOptions::default(),
            base: false,
            slider_audio: false,
            auto_fix_staged_mixes: false,
            mix_format: crate::MixFormat::default(),
            write_mode: ExportWriteMode::Direct,
            hitsound_only_dir: None,
            single_format: SingleSampleFormat::Source,
            addition_only_normal: AdditionOnlyNormal::Difficulty,
            tick_policy: TickPolicy::Ignore,
            allow_schema_growth: true,
            auto_cleanup_stale: true,
            take_over_samples: false,
            apply_volume_envelope: true,
        }
    }
}

/// What the Export window last chose for a map, kept with the session. Older builds ignore it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportChoices {
    /// File names of the difficulties left out; every other one is written into.
    pub left_out: Vec<String>,
    /// Match within, in ms.
    pub match_within: f64,
    pub copy_volumes: bool,
    /// Also write the base hitsound difficulty. Choices saved before it existed read as off.
    #[serde(default)]
    pub base: bool,
    #[serde(default)]
    pub slider_audio: bool,
    /// Use "later moment wins" instead of the default Merge/Union for nearby source hits.
    #[serde(default)]
    pub later_wins: bool,
    /// Audio format for generated/copied hitsound mix files. Choices saved before it existed read as 16-bit WAV.
    #[serde(default)]
    pub mix_format: crate::MixFormat,
    /// Automatic slider-end (and optional spinner-end) muting choices.
    #[serde(default)]
    pub slider_end_mute: dawsu_hitsounding::copy::SliderEndMuteOptions,
    /// Optional fallback sound for unmatched target gameplay object edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_sound: Option<dawsu_hitsounding::copy::FallbackSound>,
    /// Auto-fix ranking criteria issues on staged sample mixes (trim leading silence, pad <25 ms).
    #[serde(default)]
    pub auto_fix_staged_mixes: bool,
    /// Direct mapset write or standalone hitsound-only folder export.
    #[serde(default)]
    pub write_mode: ExportWriteMode,
    /// Pass-through unchanged single custom samples (`Source`) or match `mix_format`.
    #[serde(default)]
    pub single_format: SingleSampleFormat,
    /// Hitnormal policy for addition-only moments.
    #[serde(default)]
    pub addition_only_normal: AdditionOnlyNormal,
    /// Native green-line slider tick rendering policy.
    #[serde(default)]
    pub tick_policy: TickPolicy,
    /// Allow new custom index allocation on repeat exports.
    #[serde(default = "default_true")]
    pub allow_schema_growth: bool,
    /// Automatically archive stale dawsu!-owned sample mixes on repeat direct exports.
    #[serde(default = "default_true")]
    pub auto_cleanup_stale: bool,
    /// Opt-in: include unused pre-existing samples in cleanup (backed up first).
    #[serde(default)]
    pub take_over_samples: bool,
    /// Apply the green-line-only master volume curve on export.
    #[serde(default = "default_true")]
    pub apply_volume_envelope: bool,
}

impl Default for ExportChoices {
    fn default() -> Self {
        Self {
            left_out: Vec::new(),
            match_within: 5.0,
            copy_volumes: true,
            base: false,
            slider_audio: false,
            later_wins: false,
            mix_format: crate::MixFormat::Wav16,
            slider_end_mute: dawsu_hitsounding::copy::SliderEndMuteOptions::default(),
            fallback_sound: None,
            auto_fix_staged_mixes: false,
            write_mode: ExportWriteMode::Direct,
            single_format: SingleSampleFormat::Source,
            addition_only_normal: AdditionOnlyNormal::Difficulty,
            tick_policy: TickPolicy::Ignore,
            allow_schema_growth: true,
            auto_cleanup_stale: true,
            take_over_samples: false,
            apply_volume_envelope: true,
        }
    }
}

/// Which window a preview belongs to, so main never shows it as its own review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewOrigin {
    Export,
}

pub struct ExportPreview {
    pub id: u64,
    pub origin: PreviewOrigin,
    pub plan: output::WritePlan,
    pub options: ExportOptions,
    /// The load or Save of the difficulty it was made from. The history version starts
    /// again at every load, so path and version alone cannot tell two loads apart.
    generation: u64,
    /// Hitsounding mode when it was made: switching it changes what the source writes.
    hs_mode: bool,
    /// The merge settings it was made with: changing them changes what the base difficulty writes.
    settings: Option<crate::ExportSettings>,
    /// Whether redundant greenline cleanup was enabled when previewed.
    clean_redundant_greenlines: bool,
}

type PreviewResult = (Result<output::WritePlan, String>, Vec<OutputProblem>);

struct Running {
    id: u64,
    options: ExportOptions,
    generation: u64,
    hs_mode: bool,
    settings: Option<crate::ExportSettings>,
    clean_redundant_greenlines: bool,
    handle: JoinHandle<()>,
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<PreviewResult>,
    progress: Arc<Mutex<Option<(usize, usize)>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingManifestCommit {
    pub(crate) mapset_root: PathBuf,
    pub(crate) backups: PathBuf,
    pub(crate) staged_manifest_entries: Vec<(String, String)>,
    pub(crate) tick_writes: BTreeMap<String, Vec<dawsu_hitsounding::tick_render::TickWrite>>,
    pub(crate) active_sample_names: BTreeSet<String>,
    pub(crate) retained_names: BTreeSet<String>,
    pub(crate) auto_cleanup_stale: bool,
    pub(crate) take_over_samples: bool,
}

#[derive(Default)]
pub(crate) struct Slot {
    last_id: u64,
    /// The id, the difficulty it was asked for, and the options.
    waiting: Option<(u64, PathBuf, ExportOptions)>,
    running: Option<Running>,
    /// Cancelled previews still finishing. The next one starts after they end.
    ending: Vec<JoinHandle<()>>,
    preview: Option<ExportPreview>,
    problems: Vec<OutputProblem>,
    error: Option<String>,
    /// An Export write is running on main's job.
    pub(crate) writing: bool,
    /// Manifest & stale-cleanup metadata deferred until the background Direct write succeeds.
    pub(crate) pending_manifest: Option<PendingManifestCommit>,
    /// The id of the last Export write started; `written` and `write_error` belong to it.
    write_id: Option<u64>,
    /// Where the last hitsound-only Export wrote, for Open folder. Kept when Export closes.
    last_hitsound_only_dir: Option<PathBuf>,
    /// The last Export write's result: files written, or why nothing was.
    written: Option<usize>,
    write_error: Option<String>,
}

impl Slot {
    pub(crate) fn has_active_work(&self) -> bool {
        self.running.is_some()
            || self.waiting.is_some()
            || self.writing
            || self.ending.iter().any(|handle| !handle.is_finished())
    }

    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.cancel.store(true, Ordering::Relaxed);
            self.ending.push(running.handle);
        }
        self.waiting = None;
    }

    /// Stops the preview and forgets the finished one, its problems and its error.
    pub(crate) fn clear(&mut self) {
        self.stop();
        self.preview = None;
        self.problems.clear();
        self.error = None;
        self.writing = false;
        self.pending_manifest = None;
        self.written = None;
        self.write_error = None;
    }

    /// Commits ownership manifest entries and archives stale dawsu!-owned samples AFTER the
    /// background Direct export write has succeeded.
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn commit_pending_manifest(&mut self, data_root: &Path) {
        let Some(commit) = self.pending_manifest.take() else {
            return;
        };
        let mut manifest = export_manifest::load_manifest(data_root, &commit.mapset_root);
        for (name, digest) in commit.staged_manifest_entries {
            manifest.owned_samples.insert(name, digest);
        }
        for (diff_name, writes) in commit.tick_writes {
            manifest.slider_history.record_ticks(&diff_name, &writes);
        }
        let mut retained = commit.retained_names;
        if let Ok(files) = dawsu_osu_format::list_folder(&commit.mapset_root)
            && let Ok((used_indexes, used_names)) =
                crate::layers::reservations(&commit.mapset_root, &files)
        {
            retained.extend(used_names);
            for file in &files {
                let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
                if let Some(dawsu_foundation::SampleIdentity::Standard { index, .. }) =
                    dawsu_hitsounding::rules::read_sample_file_name(base)
                    && used_indexes.contains(&index.get())
                {
                    retained.insert(base.to_ascii_lowercase());
                }
            }
        }
        if commit.auto_cleanup_stale {
            let _ = export_manifest::cleanup_stale_samples(
                &export_manifest::CleanupRoots {
                    mapset_root: &commit.mapset_root,
                    data_root,
                    backups_root: &commit.backups,
                },
                &mut manifest,
                &commit.active_sample_names,
                &retained,
                commit.take_over_samples,
                &BTreeSet::new(),
            );
        }
        let _ = export_manifest::save_manifest(data_root, &commit.mapset_root, &manifest);
    }

    /// Takes a finished write's result for the Export window. False when the window closed
    /// during the write, so main reports it instead.
    pub(crate) fn take_write(&mut self, result: &Result<usize, String>) -> bool {
        if !std::mem::take(&mut self.writing) {
            return false;
        }
        match result {
            Ok(count) => self.written = Some(*count),
            Err(error) => self.write_error = Some(error.clone()),
        }
        true
    }
}

/// Why a finished preview no longer matches the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stale {
    /// Another difficulty, an older edit, a reload or another hitsounding mode.
    Map,
    /// The merge settings the base difficulty was previewed with changed.
    MergeSettings,
}

pub struct ExportStatus<'a> {
    /// The id of the latest preview request, finished or not.
    pub requested: Option<u64>,
    pub preview: Option<&'a ExportPreview>,
    /// The preview was made from another difficulty or an older edit.
    pub stale: bool,
    pub stale_reason: Option<Stale>,
    /// Options wait for main's job or a drag to end.
    pub waiting: bool,
    pub running: bool,
    pub progress: Option<(usize, usize)>,
    pub problems: &'a [OutputProblem],
    pub error: Option<&'a str>,
    pub writing: bool,
    pub write_id: Option<u64>,
    pub written: Option<usize>,
    pub write_error: Option<&'a str>,
}

impl Application {
    /// Previews one Export target difficulty's planned hitsounds over the song without writing files.
    pub fn listen_export_target(&mut self, target_path: &std::path::Path) -> Result<(), String> {
        let targets = self.export_targets();
        let canonical_target = targets
            .into_iter()
            .find(|p| p == target_path)
            .ok_or_else(|| "Choose a target difficulty in this mapset to preview.".to_string())?;
        let osu = if let Some(planned) = self
            .export
            .preview
            .as_ref()
            .and_then(|p| p.plan.files.iter().find(|f| f.path == canonical_target))
        {
            planned.file.clone()
        } else {
            let bytes = std::fs::read(&canonical_target).map_err(|e| e.to_string())?;
            dawsu_osu_format::OsuFile::read(bytes).file
        };
        let folder_files = self
            .document
            .as_ref()
            .map(|d| d.files.clone())
            .unwrap_or_default();
        let hits = match dawsu_hitsounding::decompile(&osu, &folder_files, &|| false, &mut |_, _| {}) {
            dawsu_hitsounding::Decompiled::Done(h) => h,
            other => return Err(crate::refusals::open(&other)),
        };
        let raw_cues = dawsu_hitsounding::cue_list_filtered(&hits, &[], &[], |_, _| false);
        let mut staged_clips: std::collections::HashMap<String, Arc<dawsu_audio::AudioClip>> =
            std::collections::HashMap::new();
        if let Some(preview) = self.export.preview.as_ref() {
            for mix in preview.plan.mixes.iter().chain(preview.plan.pending.iter()) {
                if let Some(name) = mix.path.file_name().and_then(|n| n.to_str()) {
                    staged_clips.insert(name.to_ascii_lowercase(), Arc::new(mix.clip()));
                }
            }
        }
        let mut sounds = Vec::with_capacity(raw_cues.len());
        for cue in raw_cues {
            let staged = match cue.sample() {
                dawsu_foundation::SampleIdentity::File(name) => {
                    staged_clips.get(&name.to_ascii_lowercase()).cloned()
                }
                _ => None,
            };
            if let Some(clip) = staged.or_else(|| self.samples.get(cue.sample()).cloned()) {
                sounds.push(dawsu_audio::ScheduledSound {
                    at_ms: cue.time().ms(),
                    end_ms: cue.loop_end().map(|t| t.ms()),
                    clip,
                    gain: cue.gain() as f32,
                    strip_id: cue.strip_id(),
                });
            }
        }
        if let Some(a) = &self.audio {
            a.load_at_speed(self.active_song_clip(), sounds, self.playback_speed);
            a.gains(self.playback_song_gain(), self.preferences.master_gain);
            a.hitsound_gain(self.playback_hitsound_gain());
        }
        self.play_from(self.position_ms());
        Ok(())
    }

    /// Stops Export target preview playback and restores the open difficulty's audio cues.
    pub fn stop_export_listen(&mut self) {
        if self.is_playing() {
            self.stop();
        }
        self.sync_audio();
    }

    /// Queues a preview and answers at once with its id. A newer request drops the
    /// finished preview and any running one, so Export is never run on old options.
    pub fn request_export_preview(&mut self, options: ExportOptions) -> Result<u64, String> {
        let source = self.document.as_ref().ok_or("Open a difficulty first.")?.path.clone();
        let targets = self.export_targets();
        // Never the source or the difficulty Generate made, whatever the window sends.
        if options.targets.iter().any(|path| !targets.contains(path)) {
            return Err("Export writes only into the other difficulties of this mapset.".into());
        }
        let left_out = targets.iter()
            .filter(|path| !options.targets.contains(path))
            .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .collect();
        if let Some(document) = self.document.as_mut() {
            document.export_choices = Some(ExportChoices {
                left_out,
                match_within: options.copy.leniency,
                copy_volumes: options.copy.copy_volumes,
                base: options.base,
                slider_audio: options.slider_audio,
                later_wins: options.copy.collision_policy == dawsu_hitsounding::copy::CopyCollisionPolicy::LaterWins,
                mix_format: options.mix_format,
                slider_end_mute: options.copy.slider_end_mute.clone(),
                fallback_sound: options.copy.fallback_sound,
                auto_fix_staged_mixes: options.auto_fix_staged_mixes,
                write_mode: options.write_mode,
                single_format: options.single_format,
                addition_only_normal: options.addition_only_normal,
                tick_policy: options.tick_policy,
                allow_schema_growth: options.allow_schema_growth,
                auto_cleanup_stale: options.auto_cleanup_stale,
                take_over_samples: options.take_over_samples,
                apply_volume_envelope: options.apply_volume_envelope,
            });
        }
        self.export.stop();
        self.export.preview = None;
        self.export.problems.clear();
        self.export.error = None;
        self.export.last_id += 1;
        let id = self.export.last_id;
        // Nothing chosen to write: the choice is kept, and there is nothing to preview.
        if !options.targets.is_empty() || options.base {
            self.export.waiting = Some((id, source, options));
            self.start_export_preview();
        }
        Ok(id)
    }

    pub fn cancel_export_preview(&mut self) {
        self.export.clear();
    }

    /// The folder the last hitsound-only Export wrote, if one has run this session.
    pub fn last_hitsound_only_export(&self) -> Option<&Path> {
        self.export.last_hitsound_only_dir.as_deref()
    }

    fn preview_is_current(&self, preview: &ExportPreview) -> bool {
        self.staleness(preview).is_none()
    }

    fn staleness(&self, preview: &ExportPreview) -> Option<Stale> {
        let document = self.document.as_ref().filter(|d| {
            preview.origin == PreviewOrigin::Export
                && preview.generation == self.document_generation
                && d.path == preview.plan.source && d.composer.history.version() == preview.plan.version
                && d.hs_mode == preview.hs_mode
                && self.preferences.clean_redundant_greenlines == preview.clean_redundant_greenlines
        });
        match document {
            None => Some(Stale::Map),
            // Merge settings change only what the base difficulty writes; Copy does not use them yet.
            Some(d) if preview.plan.generated.is_some() && d.export_settings != preview.settings => {
                Some(Stale::MergeSettings)
            }
            Some(_) => None,
        }
    }

    pub fn export_status(&self) -> ExportStatus<'_> {
        let slot = &self.export;
        ExportStatus {
            requested: (slot.last_id > 0).then_some(slot.last_id),
            preview: slot.preview.as_ref(),
            stale: slot.preview.as_ref().is_some_and(|preview| !self.preview_is_current(preview)),
            stale_reason: slot.preview.as_ref().and_then(|preview| self.staleness(preview)),
            waiting: slot.waiting.is_some(),
            running: slot.running.is_some(),
            progress: slot.running.as_ref()
                .and_then(|running| running.progress.lock().ok().and_then(|current| *current)),
            problems: &slot.problems,
            error: slot.error.as_deref(),
            writing: slot.writing,
            write_id: slot.write_id,
            written: slot.written,
            write_error: slot.write_error.as_deref(),
        }
    }

    /// The difficulties Export can write into: the mapset's others, without the source
    /// and without the difficulty Generate made from this source.
    pub fn export_targets(&self) -> Vec<PathBuf> {
        let Some(d) = self.document.as_ref() else {
            return Vec::new();
        };
        let generated = self.project_root.as_deref()
            .and_then(|projects| crate::generate_receipt::output_path(projects, &d.path));
        d.difficulties.iter()
            .filter(|path| **path != d.path && Some(path.as_path()) != generated.as_deref())
            .cloned()
            .collect()
    }

    pub fn backups_folder(&self) -> PathBuf {
        self.data_root.join("backups")
    }

    /// Writes the preview with this id. Refused unless it is the latest preview and
    /// the difficulty is still the one, at the same edit, that it was made from.
    pub fn run_export(&mut self, preview_id: u64) -> Result<u64, String> {
        self.run_export_in(preview_id, self.backups_folder())
    }

    /// Answers with the write's id, which the window matches against `write_id`.
    pub(crate) fn run_export_in(&mut self, preview_id: u64, backups: PathBuf) -> Result<u64, String> {
        if self.busy {
            return Err("Wait for the current job to finish.".into());
        }
        if self.is_stroke_open() {
            return Err("Finish or cancel the current drag before exporting.".into());
        }
        let matches = self.export.preview.as_ref()
            .is_some_and(|preview| preview.id == preview_id && self.preview_is_current(preview));
        if !matches {
            return Err("The map or the options changed since this preview. Preview again.".into());
        }
        if !self.export.preview.as_ref().is_some_and(|p| p.plan.verification.passed) {
            let reason = self
                .export
                .preview
                .as_ref()
                .and_then(|p| {
                    p.plan.verification.first_error.clone().or_else(|| {
                        p.plan.verification.files.iter().find(|f| !f.passed).and_then(|f| {
                            f.checks.iter().find(|c| !c.passed).map(|c| {
                                if c.detail.is_empty() {
                                    format!("{} ({})", f.file_name, c.label)
                                } else {
                                    format!("{} ({}: {})", f.file_name, c.label, c.detail)
                                }
                            })
                        })
                    })
                })
                .unwrap_or_else(|| "Read-back verification failed.".into());
            return Err(format!("Read-back check failed before writing: {reason}"));
        }
        let is_hitsound_only = self
            .export
            .preview
            .as_ref()
            .is_some_and(|p| p.options.write_mode == ExportWriteMode::HitsoundOnly);
        if is_hitsound_only {
            self.export
                .preview
                .as_ref()
                .expect("checked above")
                .plan
                .verify_planned_files_for_export(true)?;
        }
        let mut preview = self.export.preview.take().expect("checked above");
        let export_options = preview.options.clone();
        if export_options.write_mode == ExportWriteMode::HitsoundOnly {
            let mapset_root = preview
                .plan
                .source
                .parent()
                .ok_or("Missing mapset folder.")?
                .to_path_buf();
            let folder_label = mapset_root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "mapset".into());
            let out_dir = export_options.hitsound_only_dir.clone().unwrap_or_else(|| {
                self.data_root
                    .join("exports")
                    .join(format!("{folder_label}-hitsound-only"))
            });
            let osu_files: Vec<(String, Vec<u8>)> = preview
                .plan
                .files
                .iter()
                .filter_map(|f| {
                    let name = f.path.file_name()?.to_string_lossy().into_owned();
                    Some((name, f.file.to_bytes()))
                })
                .collect();
            let mut sample_files: Vec<(String, Vec<u8>)> = Vec::new();
            for mix in preview.plan.mixes.iter().chain(preview.plan.pending.iter()) {
                if let Some(name) = mix.path.file_name().and_then(|n| n.to_str()) {
                    sample_files.push((name.to_owned(), mix.wav_bytes().to_vec()));
                }
            }
            let result = export_manifest::write_hitsound_only_directory(
                &mapset_root,
                &out_dir,
                &osu_files,
                &sample_files,
            )?;
            let count = result.osu_files.len();
            self.export.writing = false;
            self.export.written = Some(count);
            self.export.write_error = None;
            let id = self.export.write_id.map_or(1, |last| last + 1);
            self.export.write_id = Some(id);
            self.export.last_hitsound_only_dir = Some(result.output_dir.clone());
            self.status = format!(
                "Exported {count} hitsound-only file(s) to {} (mapset untouched).",
                result.output_dir.display()
            );
            self.bump_view_revision();
            return Ok(id);
        }

        // Direct write: defer ownership manifest commit & stale cleanup until the write succeeds.
        let mapset_root = preview.plan.source.parent().map(PathBuf::from);
        let staged_manifest_entries: Vec<(String, String)> = preview
            .plan
            .mixes
            .iter()
            .filter_map(|m| {
                let name = m.path.file_name()?.to_string_lossy().to_ascii_lowercase();
                Some((name, m.digest()))
            })
            .collect();
        let mut active_sample_names: BTreeSet<String> = staged_manifest_entries
            .iter()
            .map(|(n, _)| n.clone())
            .collect();
        for p in &preview.plan.pending {
            if let Some(name) = p.path.file_name().and_then(|n| n.to_str()) {
                active_sample_names.insert(name.to_ascii_lowercase());
            }
        }
        let retained_names: BTreeSet<String> = self
            .document
            .as_ref()
            .map(|d| d.pending.taken_names())
            .unwrap_or_default();
        let tick_writes = std::mem::take(&mut preview.plan.tick_writes);
        self.export.pending_manifest = mapset_root.map(|root| PendingManifestCommit {
            mapset_root: root,
            backups: backups.clone(),
            staged_manifest_entries,
            tick_writes,
            active_sample_names,
            retained_names,
            auto_cleanup_stale: export_options.auto_cleanup_stale,
            take_over_samples: export_options.take_over_samples,
        });
        self.write_reviewed(preview.plan, backups);
        self.export.writing = true;
        self.export.written = None;
        self.export.write_error = None;
        let id = self.export.write_id.map_or(1, |last| last + 1);
        self.export.write_id = Some(id);
        Ok(id)
    }

    /// Collects a finished preview and starts waiting options once main is free.
    pub(crate) fn poll_export(&mut self) {
        let mut changed = false;
        if let Some(running) = &self.export.running {
            match running.receive.try_recv() {
                Ok((result, problems)) => {
                    let (id, generation, hs_mode, clean_redundant_greenlines) = (
                        running.id, running.generation, running.hs_mode, running.clean_redundant_greenlines,
                    );
                    let settings = running.settings.clone();
                    let options = running.options.clone();
                    self.export.running = None;
                    self.export.problems = problems;
                    match result {
                        Ok(plan) => {
                            self.export.preview = Some(ExportPreview {
                                id, origin: PreviewOrigin::Export, plan, options, generation, hs_mode, settings,
                                clean_redundant_greenlines,
                            });
                        }
                        Err(error) => self.export.error = Some(error),
                    }
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.export.running = None;
                    self.export.error = Some("The preview stopped unexpectedly. Preview again.".into());
                    changed = true;
                }
            }
        }
        if changed {
            self.bump_view_revision();
        }
        self.start_export_preview();
    }

    fn start_export_preview(&mut self) {
        self.export.ending.retain(|handle| !handle.is_finished());
        if self.export.running.is_some() || !self.export.ending.is_empty() || self.busy || self.is_stroke_open() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some((id, asked_for, options)) = self.export.waiting.take() else {
            return;
        };
        if asked_for != document.path {
            self.export.error = Some("Another difficulty was opened. Preview again.".into());
            self.bump_view_revision();
            return;
        }
        let source = output::Source::from_document(document);
        let defaults = self.preferences.default_samples.clone();
        let clean_redundant_greenlines = self.preferences.clean_redundant_greenlines;
        // As Generate in main: the map's merge settings, and the receipts in the project folder.
        let settings = document.export_settings.clone();
        let projects = self.project_root.clone();
        let thread_settings = settings.clone();
        let shared_envelope = if options.apply_volume_envelope {
            self.shared_base_volume_envelope(None)
        } else {
            VolumeEnvelope::default()
        };
        let diff_envelopes: BTreeMap<String, VolumeEnvelope> = if options.apply_volume_envelope {
            options
                .targets
                .iter()
                .filter_map(|t| {
                    let name = t.file_name()?.to_string_lossy().to_ascii_lowercase();
                    let env = self.shared_base_volume_envelope(Some(&name));
                    (!env.is_empty()).then_some((name, env))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let prev_schema_len = document.export_index_schema.len();
        let manifest_history = source
            .path
            .parent()
            .map(|root| export_manifest::load_manifest(&self.data_root, root).slider_history)
            .unwrap_or_default();
        let thread_options = options.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(None));
        let (send, receive) = mpsc::channel();
        let generation = self.document_generation;
        let hs_mode = document.hs_mode;
        let (thread_cancel, thread_progress) = (cancel.clone(), progress.clone());
        let handle = std::thread::spawn(move || {
            let (cancel, progress) = (thread_cancel, thread_progress);
            let mut problems = Vec::new();
            let result = output::export_report(
                &source,
                &output::ExportConfig {
                    defaults: &defaults,
                    settings: thread_settings.as_ref(),
                    projects: projects.as_deref(),
                    options: &thread_options,
                },
                &|| cancel.load(Ordering::Relaxed),
                &mut problems,
                &mut |done, total| {
                    if let Ok(mut current) = progress.lock() {
                        *current = Some((done, total));
                    }
                },
            )
            .and_then(|mut plan| {
                if !thread_options.allow_schema_growth
                    && prev_schema_len > 0
                    && plan.export_index_schema.len() > prev_schema_len
                {
                    return Err(
                        "Export needed new custom sample indices, but schema growth is locked. Turn on Allow schema growth to allocate new indices."
                            .into(),
                    );
                }
                let mut pre_envelope_errors: std::collections::HashMap<PathBuf, (Option<f64>, String)> =
                    std::collections::HashMap::new();
                if thread_options.apply_volume_envelope {
                    let mut total_env_changes = 0usize;
                    let folder_with_writes = plan.folder_listing_after_writes();
                    for planned_file in &mut plan.files {
                        let key = planned_file
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_ascii_lowercase())
                            .unwrap_or_default();
                        let env = diff_envelopes
                            .get(&key)
                            .filter(|e| !e.is_empty())
                            .unwrap_or(&shared_envelope);
                        if !env.is_empty() {
                            let (patched, changed) = apply_envelope_to_osu(&planned_file.file, env);
                            if changed > 0 {
                                let pre_bytes = planned_file.file.to_bytes();
                                let pre_expected = planned_file.expected_hitsounds().cloned();
                                if let Some(ref pre_exp) = pre_expected
                                    && let Err(mismatch) = dawsu_hitsounding::verify_compiled_osu(
                                        &pre_bytes,
                                        pre_exp,
                                        &folder_with_writes,
                                    )
                                {
                                    let msg = mismatch.time_ms.map_or_else(
                                        || mismatch.detail.clone(),
                                        |t| format!("Pre-curve mismatch at {t:.0} ms: {}", mismatch.detail),
                                    );
                                    pre_envelope_errors
                                        .insert(planned_file.path.clone(), (mismatch.time_ms, msg));
                                }
                                planned_file.replace_file(patched);
                                total_env_changes += changed;
                                let post_bytes = planned_file.file.to_bytes();
                                let post_read = dawsu_osu_format::OsuFile::read(post_bytes);
                                match dawsu_hitsounding::decompile(
                                    &post_read.file,
                                    &folder_with_writes,
                                    &|| cancel.load(Ordering::Relaxed),
                                    &mut |_, _| {},
                                ) {
                                    dawsu_hitsounding::Decompiled::Done(post_hits) => {
                                        if let Some(ref pre_exp) = pre_expected
                                            && let Err(mismatch) =
                                                dawsu_hitsounding::verify_envelope_preserved(
                                                    pre_exp,
                                                    &post_hits,
                                                )
                                        {
                                            let msg = mismatch.time_ms.map_or_else(
                                                || mismatch.detail.clone(),
                                                |t| {
                                                    format!(
                                                        "Volume curve mismatch at {t:.0} ms: {}",
                                                        mismatch.detail
                                                    )
                                                },
                                            );
                                            pre_envelope_errors
                                                .entry(planned_file.path.clone())
                                                .or_insert((mismatch.time_ms, msg));
                                        }
                                        planned_file.set_verify_hitsounds(post_hits);
                                    }
                                    dawsu_hitsounding::Decompiled::UnsupportedMode { mode } => {
                                        pre_envelope_errors
                                            .entry(planned_file.path.clone())
                                            .or_insert_with(|| {
                                                (
                                                    None,
                                                    format!(
                                                        "Volume curve output parsed as unsupported game mode {mode}."
                                                    ),
                                                )
                                            });
                                    }
                                    dawsu_hitsounding::Decompiled::Cancelled => {
                                        return Err("Export preview cancelled.".into());
                                    }
                                }
                            }
                        }
                    }
                    if total_env_changes > 0 {
                        plan.details.push(format!(
                            "Applied master volume curve ({total_env_changes} green-line update(s), 0 extra sample mixes)."
                        ));
                    }
                }
                if thread_options.addition_only_normal != AdditionOnlyNormal::Difficulty {
                    plan.details.push(format!(
                        "Addition-only hitnormal policy: {:?}.",
                        thread_options.addition_only_normal
                    ));
                }
                if thread_options.tick_policy == TickPolicy::Render
                    && let Some(root) = source.path.parent()
                {
                    let mut occupied = crate::mixes::occupied_indexes(root, &source.pending)?;
                    for mix in &plan.mixes {
                        if let Some(fname) = mix.path.file_name().and_then(|n| n.to_str())
                            && let Some(dawsu_foundation::SampleIdentity::Standard { index, .. }) =
                                dawsu_hitsounding::rules::read_sample_file_name(fname)
                        {
                            occupied.insert(index.get());
                        }
                    }
                    let mut next_tick_idx = dawsu_hitsounding::tick_render::DEFAULT_BODY_SAMPLE_FIRST_INDEX
                        .max(thread_options.copy.start_index);
                    let mut tick_idx_by_digest: std::collections::HashMap<String, u32> =
                        std::collections::HashMap::new();
                    let mut total_ticks = 0usize;
                    let mut source_moments: Vec<&dawsu_hitsounding::Moment> =
                        source.hits.moments().iter().collect();
                    source_moments.sort_by(|a, b| a.time().ms().total_cmp(&b.time().ms()));

                    for planned_file in &mut plan.files {
                        let diff_name = planned_file
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let (restored, _) =
                            manifest_history.restore_ticks(&diff_name, &planned_file.file);
                        planned_file.replace_file(restored);

                        let target_hits = match dawsu_hitsounding::decompile(
                            &planned_file.file,
                            &source.folder_files,
                            &|| cancel.load(Ordering::Relaxed),
                            &mut |_, _| {},
                        ) {
                            dawsu_hitsounding::Decompiled::Done(h) => h,
                            _ => continue,
                        };
                        let node_times: Vec<f64> =
                            target_hits.moments().iter().map(|m| m.time().ms()).collect();
                        let mut writes: Vec<dawsu_hitsounding::tick_render::TickWrite> = Vec::new();
                        for body in target_hits.bodies() {
                            for tick in body.ticks() {
                                let tick_ms = tick.ms();
                                if node_times
                                    .iter()
                                    .any(|&nt| (nt - tick_ms).abs() <= thread_options.copy.leniency)
                                {
                                    continue;
                                }
                                let mut layers: Vec<(dawsu_foundation::SampleIdentity, i32)> =
                                    Vec::new();
                                let mut max_vol = 0i32;
                                for sm in &source_moments {
                                    if (sm.time().ms() - tick_ms).abs() <= thread_options.copy.leniency {
                                        for lane in sm.hits() {
                                            let v = sm.hit_volume(lane).clamp(5, 100);
                                            max_vol = max_vol.max(v);
                                            layers.push((lane.clone(), v));
                                        }
                                    }
                                }
                                if layers.is_empty() {
                                    continue;
                                }
                                let norm_layers: Vec<(dawsu_foundation::SampleIdentity, i32)> = layers
                                    .iter()
                                    .map(|(l, v)| {
                                        let rel = ((*v as f64 / max_vol.max(5) as f64) * 100.0)
                                            .round()
                                            .clamp(5.0, 100.0) as i32;
                                        (l.clone(), rel)
                                    })
                                    .collect();
                                let (ext, bytes) = crate::mixes::render_tick_sample(
                                    &source.hits,
                                    &norm_layers,
                                    root,
                                    &source.pending,
                                    &defaults,
                                    thread_options.mix_format,
                                    thread_options.auto_fix_staged_mixes,
                                    thread_options.single_format,
                                    &|| cancel.load(Ordering::Relaxed),
                                )?;
                                let digest = dawsu_foundation::digest::sha256_hex(&bytes);
                                let custom_index = if let Some(&existing_idx) =
                                    tick_idx_by_digest.get(&digest)
                                {
                                    existing_idx
                                } else {
                                    while next_tick_idx <= i32::MAX as u32
                                        && occupied.contains(&next_tick_idx)
                                    {
                                        next_tick_idx += 1;
                                    }
                                    if next_tick_idx > i32::MAX as u32 {
                                        return Err("No free custom sample index available for slider tick audio.".into());
                                    }
                                    let idx = next_tick_idx;
                                    occupied.insert(idx);
                                    next_tick_idx += 1;
                                    tick_idx_by_digest.insert(digest, idx);
                                    plan.mixes.push(crate::mixes::StagedMix::indexed_bytes(
                                        root,
                                        &format!("normal-slidertick{idx}.{ext}"),
                                        idx,
                                        bytes,
                                    ));
                                    idx
                                };
                                writes.push(dawsu_hitsounding::tick_render::TickWrite {
                                    time_ms: tick_ms,
                                    sample_set: dawsu_foundation::SampleSet::Normal,
                                    custom_index,
                                    volume: max_vol.clamp(5, 100),
                                });
                            }
                        }
                        if !writes.is_empty() {
                            let (patched, applied) =
                                dawsu_hitsounding::tick_render::apply_tick_greenlines(
                                    &planned_file.file,
                                    &writes,
                                );
                            planned_file.replace_file(patched);
                            total_ticks += applied;
                            plan.tick_writes.insert(diff_name, writes);
                        }
                    }
                    plan.details.push(format!(
                        "Rendered {total_ticks} slider tick green line(s) (with +5 ms revert)."
                    ));
                }
                if thread_options.write_mode == ExportWriteMode::HitsoundOnly {
                    plan.details.push(
                        "Hitsound-only folder export: real mapset files will not be modified."
                            .into(),
                    );
                }
                if clean_redundant_greenlines {
                    plan.clean_redundant_greenlines();
                }
                let is_hitsound_only = thread_options.write_mode == ExportWriteMode::HitsoundOnly;
                let _ = plan.verify_and_finalize(&source, &pre_envelope_errors, is_hitsound_only);
                Ok(plan)
            });
            let _ = send.send((result, problems));
        });
        self.export.running = Some(Running {
            id, options, generation, hs_mode, settings, clean_redundant_greenlines, handle, cancel, receive, progress,
        });
        self.bump_view_revision();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::SampleIdentity;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    const MAP: &str = "osu file format v14\n\n[General]\nMode:0\n\n[Metadata]\nVersion:DIFF\n\n[Events]\nEVENTS\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";

    struct Mapset {
        scratch: Scratch,
        source: PathBuf,
        target: PathBuf,
        target_bytes: String,
    }

    fn mapset(name: &str) -> Mapset {
        let scratch = Scratch::new(name);
        let source = scratch.path("Source.osu");
        let target = scratch.path("Target.osu");
        std::fs::write(&source, MAP.replace("DIFF", "Source")
            .replace("EVENTS", "Sample,1005,0,\"near.wav\",70\nSample,1200,0,\"far.wav\",60")).unwrap();
        let target_bytes = MAP.replace("DIFF", "Target").replace("EVENTS", "Sample,1400,0,\"target.wav\",80");
        std::fs::write(&target, &target_bytes).unwrap();
        let source = source.canonicalize().unwrap();
        let target = target.canonicalize().unwrap();
        Mapset { scratch, source, target, target_bytes }
    }

    fn opened(source: &std::path::Path) -> Application {
        let mut app = Application::headless();
        // Backups and receipts stay in the fixture, never in the real data folder.
        let data = source.parent().unwrap().join("data");
        app.data_root = data.clone();
        app.project_root = Some(data.join("projects"));
        app.request_open(source.to_path_buf());
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        app
    }

    fn wait_main(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline, "main job did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_preview(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll();
            let status = app.export_status();
            if !status.running && !status.waiting {
                return;
            }
            assert!(Instant::now() < deadline, "preview did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn save(app: &mut Application) {
        app.save();
        wait_main(app);
        if app.save_review_pending() {
            app.answer_save_review(true);
            wait_main(app);
        }
        assert!(app.error.is_none() && !app.is_dirty(), "{:?}", app.error);
    }

    fn reopen(app: &mut Application, path: &std::path::Path) {
        app.request_open(path.to_path_buf());
        wait_main(app);
        assert_eq!(app.document().unwrap().path, path);
    }

    fn edit(app: &mut Application) {
        app.document.as_mut().unwrap().composer.add(1500.0, SampleIdentity::File("far.wav".into()), 60, 1.0);
    }

    /// Edits until the history version is back at `version`, as after the same number of steps.
    fn edit_to(app: &mut Application, version: u64) {
        let mut time = 1700.0;
        loop {
            let composer = &mut app.document.as_mut().unwrap().composer;
            let now = composer.history.version();
            if now >= version {
                break;
            }
            // An added hit takes two steps, Undo one.
            if version - now == 1 && composer.history.can_undo() {
                composer.history.undo();
            } else {
                composer.add(time, SampleIdentity::File("far.wav".into()), 60, 1.0);
                time += 100.0;
            }
        }
        assert_eq!(app.document().unwrap().composer.history.version(), version);
    }

    fn options(targets: &[PathBuf]) -> ExportOptions {
        ExportOptions {
            targets: targets.to_vec(),
            copy: CopyOptions { leniency: 5.0, copy_storyboarded_samples: true, ..CopyOptions::default() },
            ..ExportOptions::default()
        }
    }

    fn plan_parts(plan: &output::WritePlan) -> impl PartialEq + std::fmt::Debug {
        (
            plan.label.clone(), plan.source.clone(), plan.version, plan.details.clone(),
            plan.mix_paths().map(PathBuf::from).collect::<Vec<_>>(),
            (0..plan.mixes.len()).map(|index| plan.mix_clip(index).unwrap().samples).collect::<Vec<_>>(),
            plan.files.iter().map(|file| (file.path.clone(), file.file.to_bytes(), file.expected.clone()))
                .collect::<Vec<_>>(),
        )
    }

    fn problem_parts(problems: &[OutputProblem]) -> Vec<(PathBuf, PathBuf, String, String)> {
        problems.iter().map(|problem| {
            (problem.target.clone(), problem.timeline.clone(), problem.time.to_string(), problem.message.clone())
        }).collect()
    }

    /// The preview plans the same writes, sample mixes and problems as the editor's Copy.
    fn assert_parity(source: &std::path::Path, options: ExportOptions) -> Application {
        let mut app = opened(source);
        let defaults = source.parent().unwrap().join("defaults");
        std::fs::create_dir_all(&defaults).unwrap();
        app.preferences.default_samples = defaults;
        let id = app.request_export_preview(options.clone()).unwrap();
        wait_preview(&mut app);
        app.copy_hitsounds(options.targets.clone(), options.copy.clone());
        wait_main(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let preview = status.preview.expect("a finished preview");
        assert_eq!((preview.id, preview.origin), (id, PreviewOrigin::Export));
        assert!(!status.stale);
        let copy = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        assert_eq!(plan_parts(&preview.plan), plan_parts(copy));
        assert_eq!(problem_parts(status.problems), problem_parts(&app.output_problems));
        assert!(!preview.plan.details.is_empty());
        assert_ne!(Some(preview.plan.files[0].file.to_bytes()), preview.plan.files[0].expected, "the preview changes the target");
        app
    }

    #[test]
    fn export_preview_plans_the_same_writes_as_copy() {
        let set = mapset("export-parity");
        let app = assert_parity(&set.source, options(std::slice::from_ref(&set.target)));
        assert_eq!(app.export_status().preview.unwrap().plan.mixes.len(), 0);
    }

    #[test]
    fn export_preview_stages_the_same_sample_mixes_as_copy() {
        // Two samples at one moment need a mix in the target (tests/copy_mixes.rs).
        let scratch = Scratch::new("export-parity-mix");
        let header = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Test\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n";
        let source = scratch.path("source.osu");
        let target = scratch.path("target.osu");
        std::fs::write(&source, format!("{header}256,192,1000,1,0,0:0:0:40:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n")).unwrap();
        std::fs::write(&target, format!("{header}100,100,1000,1,0,0:0:0:0:\n")).unwrap();
        for (name, value) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            for _ in 0..2 {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let options = ExportOptions { targets: vec![target.canonicalize().unwrap()], ..ExportOptions::default() };
        let mut app = assert_parity(&source.canonicalize().unwrap(), options);
        let preview = app.export_status().preview.unwrap();
        assert!(!preview.plan.mixes.is_empty());
        let (id, waves) = (preview.id, preview.plan.mix_paths().map(PathBuf::from).collect::<Vec<_>>());
        assert!(waves.iter().all(|wave| !wave.exists()));
        app.output_review = None;
        app.run_export(id).unwrap();
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(waves.iter().all(|wave| wave.exists()), "the export publishes its mixes");
    }

    #[test]
    fn export_preview_never_uses_main_job_error_or_review() {
        let set = mapset("export-apart");
        let mut app = opened(&set.source);
        app.status = "untouched".into();
        app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        assert!(!app.busy);
        assert!(app.jobs.is_none());
        wait_preview(&mut app);
        assert!(app.export_status().preview.is_some());
        assert!(app.output_review.is_none());
        assert_eq!(app.status, "untouched");

        app.request_export_preview(options(&[])).unwrap();
        wait_preview(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None, "every difficulty left out is a choice, not an error");
        assert!(status.preview.is_none());
        assert!(app.error.is_none());
        assert!(app.output_problems.is_empty());
    }

    #[test]
    fn export_preview_waits_for_main_job_and_close_cancels_it() {
        let set = mapset("export-wait");
        let mut app = opened(&set.source);
        app.busy = true;
        app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        app.poll_export();
        let status = app.export_status();
        assert!(status.waiting && !status.running);
        assert_eq!(status.error, None);
        app.busy = false;
        app.poll_export();
        assert!(app.export_status().running || app.export_status().preview.is_some());
        app.cancel_export_preview();
        let status = app.export_status();
        assert!(!status.running && !status.waiting);
        app.poll_export();
        assert!(app.export_status().preview.is_none());

        app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        app.cancel_job();
        wait_preview(&mut app);
        assert!(app.export_status().preview.is_some());
        app.cancel_export_preview();
        let status = app.export_status();
        assert!(status.preview.is_none() && status.problems.is_empty());
    }

    #[test]
    fn export_runs_only_the_latest_preview_of_the_current_edit() {
        let set = mapset("export-run");
        let backups = set.scratch.path("backups");
        let mut app = opened(&set.source);
        let first = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        assert!(app.run_export_in(first + 1, backups.clone()).is_err());

        let second = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        assert!(second > first);
        assert!(app.run_export_in(first, backups.clone()).is_err());
        wait_preview(&mut app);
        assert!(app.run_export_in(first, backups.clone()).is_err());

        app.document.as_mut().unwrap().composer.add(1500.0, SampleIdentity::File("far.wav".into()), 60, 1.0);
        assert!(app.export_status().stale);
        assert!(app.run_export_in(second, backups.clone()).is_err());
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);

        let third = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        app.run_export_in(third, backups.clone()).unwrap();
        assert!(app.output_review.is_none());
        assert!(app.export_status().preview.is_none());
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        let written = std::fs::read_to_string(&set.target).unwrap();
        assert!(written.contains("Sample,1200,0,\"far.wav\",60"), "{written}");
        assert!(app.run_export_in(third, backups).is_err());
    }

    #[test]
    fn waiting_options_are_dropped_when_another_difficulty_opens() {
        let set = mapset("export-switch");
        let mut app = opened(&set.source);
        app.busy = true;
        app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        app.busy = false;
        app.request_open(set.target.clone());
        wait_main(&mut app);
        wait_preview(&mut app);
        let status = app.export_status();
        assert!(status.preview.is_none() && !status.waiting && !status.running);
        assert_eq!(status.error, None);
        assert!(app.error.is_none());
    }

    #[test]
    fn export_refuses_a_preview_of_another_difficulty() {
        let set = mapset("export-other-map");
        let mut app = opened(&set.source);
        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        let version = app.export_status().preview.unwrap().plan.version;
        app.request_open(set.target.clone());
        wait_main(&mut app);
        assert_eq!(app.document().unwrap().path, set.target);
        assert_eq!(app.document().unwrap().composer.history.version(), version);
        assert!(app.export_status().preview.is_none());
        assert!(app.run_export_in(id, set.scratch.path("backups")).is_err());
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);
    }

    #[test]
    fn reopening_or_reverting_the_same_difficulty_refuses_an_older_preview() {
        let set = mapset("export-reload");
        let backups = set.scratch.path("backups");
        let mut app = opened(&set.source);
        edit_to(&mut app, 4);
        let version = app.document().unwrap().composer.history.version();
        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        app.copy_hitsounds(vec![set.target.clone()], options(&[]).copy);
        wait_main(&mut app);
        assert!(app.output_review.is_some());
        save(&mut app);
        reopen(&mut app, &set.target);
        reopen(&mut app, &set.source);
        edit_to(&mut app, version);
        assert!(app.run_export_in(id, backups.clone()).is_err());
        assert!(app.output_review.is_none(), "main's old review does not survive a reload");
        app.apply_output_in(backups.clone());
        wait_main(&mut app);
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);

        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        let version = app.document().unwrap().composer.history.version();
        app.revert_requested = true;
        app.confirm_revert();
        wait_main(&mut app);
        edit_to(&mut app, version);
        assert!(app.run_export_in(id, backups).is_err());
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);
    }

    #[test]
    fn saving_the_difficulty_refuses_a_preview_made_before_the_save() {
        let set = mapset("export-save");
        let mut app = opened(&set.source);
        edit(&mut app);
        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        save(&mut app);
        assert!(app.run_export_in(id, set.scratch.path("backups")).is_err());
        assert_eq!(std::fs::read_to_string(&set.target).unwrap(), set.target_bytes);
    }

    #[test]
    fn export_preview_counts_each_target_as_copy_planned_it() {
        let set = mapset("export-counts");
        let app = assert_parity(&set.source, options(std::slice::from_ref(&set.target)));
        let preview = app.export_status().preview.unwrap();
        assert_eq!(preview.plan.targets.len(), 1);
        let counts = &preview.plan.targets[0];
        assert_eq!(counts.path, set.target);
        // The same numbers Copy's review line shows.
        let line = format!("Target.osu: {} matched, {} unmatched source moments", counts.matched, counts.unmatched);
        assert_eq!(preview.plan.details[0], line);
        assert_eq!((counts.collisions, counts.kept_from_silent, counts.muted), (0, 0, 0));
    }

    #[test]
    fn a_failed_export_write_reports_to_export_not_main() {
        let set = mapset("export-write-error");
        let mut app = opened(&set.source);
        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        // Another program changes the target after the preview: the write refuses.
        std::fs::write(&set.target, set.target_bytes.replace("Target", "Changed")).unwrap();
        app.run_export_in(id, set.scratch.path("backups")).unwrap();
        assert!(app.export_status().writing);
        wait_main(&mut app);
        let status = app.export_status();
        assert!(!status.writing);
        assert!(status.write_error.is_some_and(|error| error.contains("changed since this review")), "{:?}", status.write_error);
        assert_eq!(status.written, None);
        assert!(app.error.is_none(), "{:?}", app.error);

        // A new preview keeps the write result until the next write.
        app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        assert!(app.export_status().write_error.is_some());
        wait_preview(&mut app);
        let id = app.export_status().preview.unwrap().id;
        app.run_export_in(id, set.scratch.path("backups")).unwrap();
        assert_eq!(app.export_status().write_error, None);
        wait_main(&mut app);
        assert_eq!(app.export_status().written, Some(1));
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn a_write_still_running_when_export_closes_reports_to_main() {
        let set = mapset("export-write-closed");
        let mut app = opened(&set.source);
        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        std::fs::write(&set.target, "changed").unwrap();
        app.run_export_in(id, set.scratch.path("backups")).unwrap();
        app.cancel_export_preview();
        wait_main(&mut app);
        assert!(app.export_status().write_error.is_none());
        assert!(app.error.as_deref().is_some_and(|error| error.contains("changed since this review")), "{:?}", app.error);
    }

    #[test]
    fn export_choices_are_kept_per_map_with_the_session() {
        let set = mapset("export-choices");
        let mut app = opened(&set.source);
        assert_eq!(app.document().unwrap().export_choices(), None);
        let mut chosen = options(&[]);
        chosen.copy.leniency = 3.0;
        chosen.copy.copy_volumes = false;
        app.request_export_preview(chosen).unwrap();
        let expected = ExportChoices {
            left_out: vec!["Target.osu".into()],
            match_within: 3.0,
            copy_volumes: false,
            ..ExportChoices::default()
        };
        assert_eq!(app.document().unwrap().export_choices(), Some(&expected));
        wait_preview(&mut app);
        app.save_session().unwrap();

        let mut app = opened(&set.source);
        assert_eq!(app.document().unwrap().export_choices(), Some(&expected));
        app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        assert_eq!(app.document().unwrap().export_choices().unwrap().left_out, Vec::<String>::new());
    }

    #[test]
    fn a_session_without_export_choices_still_loads() {
        let old: crate::project::SessionSettings =
            serde_json::from_str(r#"{"song_offset":0.0,"view_start":0.0}"#).unwrap();
        assert_eq!(old.export_choices, None);
        let json = serde_json::to_value(&old).unwrap();
        assert!(json.get("export_choices").is_none(), "an unset field is not written");
    }

    #[test]
    fn export_refuses_a_target_outside_the_mapset_list() {
        let set = mapset("export-foreign-target");
        let mut app = opened(&set.source);
        let elsewhere = set.scratch.path("elsewhere.osu");
        std::fs::write(&elsewhere, &set.target_bytes).unwrap();
        for target in [set.source.clone(), elsewhere.canonicalize().unwrap()] {
            let error = app.request_export_preview(options(&[target])).unwrap_err();
            assert_eq!(error, "Export writes only into the other difficulties of this mapset.");
        }
        assert_eq!(app.document().unwrap().export_choices(), None, "a refused request stores nothing");
        assert!(!app.export_status().running && !app.export_status().waiting);
    }

    #[test]
    fn every_difficulty_left_out_stores_the_choice_without_a_preview() {
        let set = mapset("export-all-left-out");
        let mut app = opened(&set.source);
        app.request_export_preview(options(&[])).unwrap();
        let status = app.export_status();
        assert!(!status.running && !status.waiting && status.preview.is_none() && status.error.is_none());
        assert_eq!(app.document().unwrap().export_choices().unwrap().left_out, ["Target.osu"]);
    }

    #[test]
    fn switching_hitsounding_mode_makes_the_preview_stale() {
        let set = mapset("export-hs-mode");
        let mut app = opened(&set.source);
        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        assert!(!app.export_status().stale);
        app.document.as_mut().unwrap().hs_mode ^= true;
        assert!(app.export_status().stale);
        assert!(app.run_export_in(id, set.scratch.path("backups")).is_err());
    }

    #[test]
    fn each_export_write_has_its_own_id() {
        let set = mapset("export-write-id");
        let mut app = opened(&set.source);
        assert_eq!(app.export_status().write_id, None);
        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        let first = app.run_export_in(id, set.scratch.path("backups")).unwrap();
        assert_eq!(app.export_status().write_id, Some(first));
        wait_main(&mut app);
        let id = app.request_export_preview(options(std::slice::from_ref(&set.target))).unwrap();
        wait_preview(&mut app);
        let second = app.run_export_in(id, set.scratch.path("backups")).unwrap();
        assert!(second > first);
        assert_eq!(app.export_status().write_id, Some(second));
    }

    #[test]
    fn export_targets_leave_out_the_source_and_the_generate_output() {
        let set = mapset("export-targets");
        // Generate names its output from the artist and title.
        let source = std::fs::read_to_string(&set.source).unwrap()
            .replace("Version:Source", "Artist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Source");
        std::fs::write(&set.source, source).unwrap();
        let mut app = opened(&set.source);
        app.generate();
        wait_main(&mut app);
        let generated = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error)).files[0].path.clone();
        app.apply_output_in(set.scratch.path("backups"));
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(app.document().unwrap().difficulties.contains(&generated));
        assert_eq!(app.export_targets(), std::slice::from_ref(&set.target));
    }

    // Slice 2: the base hitsound difficulty (Generate) in the same write as Copy.

    const MIXED: &str = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:VERSION\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n";

    /// A source with two samples at one moment, so Generate and Copy both need a sample mix,
    /// and two other difficulties.
    fn mixed_mapset(name: &str) -> (Scratch, PathBuf, PathBuf, PathBuf) {
        let scratch = Scratch::new(name);
        let source = scratch.path("source.osu");
        std::fs::write(&source, format!("{}256,192,1000,1,0,0:0:0:40:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n", MIXED.replace("VERSION", "Source"))).unwrap();
        for target in ["target", "other"] {
            std::fs::write(scratch.path(&format!("{target}.osu")), format!("{}100,100,1000,1,0,0:0:0:0:\n", MIXED.replace("VERSION", target))).unwrap();
        }
        for (name, value) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            for _ in 0..2 {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let canonical = |name: &str| scratch.path(name).canonicalize().unwrap();
        let (source, target, other) = (canonical("source.osu"), canonical("target.osu"), canonical("other.osu"));
        (scratch, source, target, other)
    }

    /// Generate written, then Copy written, as the editor's two old buttons do, without the
    /// editor's polling in between (it would add the new WAVs as samples to the source).
    fn generate_then_copy(source: &std::path::Path, target: PathBuf, copy: &CopyOptions, slider_audio: bool) {
        let app = opened(source);
        let snapshot = output::Source::from_document(app.document().unwrap());
        let defaults = app.preferences.default_samples.clone();
        let backups = app.data_root.join("backups");
        let generate = output::generate_report(
            &snapshot, &defaults, None, app.project_root.as_deref(), slider_audio, &|| false,
            &mut Vec::new(), &mut |_, _| {},
        ).unwrap();
        generate.apply(&backups, &|| false).unwrap();
        crate::generate_receipt::record(&generate).unwrap();
        let copy = output::copy_report(&snapshot, &defaults, &[target], copy, &|| false, &mut Vec::new(), &mut |_, _| {})
            .unwrap();
        copy.apply(&backups, &|| false).unwrap();
    }

    fn with_base(targets: &[PathBuf], slider_audio: bool) -> ExportOptions {
        ExportOptions { base: true, slider_audio, ..options(targets) }
    }

    /// Every file in the mapset folder by name, without the editor's data folder.
    fn folder_files(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(root).unwrap().map(|entry| entry.unwrap().path())
            .filter(|path| path.is_file())
            .map(|path| (path.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&path).unwrap()))
            .collect()
    }

    fn wavs(root: &std::path::Path) -> Vec<String> {
        folder_files(root).into_keys().filter(|name| name.starts_with("dawsu-mix-")).collect()
    }

    fn run_preview(app: &mut Application, options: ExportOptions) {
        // The editor adds new WAVs in the mapset folder as samples about once a second, which
        // makes a preview stale as it would in the window. Preview again until one is current.
        for _ in 0..20 {
            app.request_export_preview(options.clone()).unwrap();
            wait_preview(app);
            if !app.export_status().stale {
                break;
            }
        }
        let status = app.export_status();
        assert_eq!(status.error, None);
        let id = status.preview.expect("a finished preview").id;
        let backups = app.data_root.join("backups");
        app.run_export_in(id, backups).unwrap();
        wait_main(app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.export_status().write_error, None);
    }

    #[test]
    fn base_only_export_plans_the_same_writes_as_generate() {
        let (_scratch, source, _, _) = mixed_mapset("export-base-parity");
        let mut app = opened(&source);
        let id = app.request_export_preview(with_base(&[], true)).unwrap();
        wait_preview(&mut app);
        app.generate_with_slider_audio(true);
        wait_main(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let preview = status.preview.expect("a base-only preview");
        assert_eq!(preview.id, id);
        let generate = app.output_review.as_ref().unwrap_or_else(|| panic!("{:?}", app.error));
        assert_eq!(plan_parts(&preview.plan), plan_parts(generate));
        assert_eq!(problem_parts(status.problems), problem_parts(&app.output_problems));
        assert!(preview.plan.includes_slider_audio());
        assert!(!preview.plan.mixes.is_empty(), "the fixture needs a sample mix");
        assert_eq!(preview.plan.generated_file(), Some(preview.plan.files[0].path.as_path()));
    }

    #[test]
    fn base_and_targets_write_as_generate_then_copy_and_leave_out_the_rest() {
        let (_export_set, source, target, other) = mixed_mapset("export-base-copy");
        let other_bytes = std::fs::read(&other).unwrap();
        let mut app = opened(&source);
        run_preview(&mut app, with_base(std::slice::from_ref(&target), false));
        let exported = folder_files(source.parent().unwrap());

        let (_set, reference, reference_target, _) = mixed_mapset("export-base-copy-reference");
        generate_then_copy(&reference, reference_target, &options(&[]).copy, false);

        assert_eq!(exported, folder_files(reference.parent().unwrap()));
        assert_eq!(exported["other.osu"], other_bytes, "a left-out difficulty is not touched");
        assert_eq!(wavs(source.parent().unwrap()).len(), 2, "Generate and Copy each wrote their mix");
    }

    #[test]
    fn the_receipt_lists_only_the_base_and_its_mixes_and_re_export_adds_no_wav() {
        let (_scratch, source, target, _) = mixed_mapset("export-base-receipt");
        let mut app = opened(&source);
        app.request_export_preview(with_base(std::slice::from_ref(&target), false)).unwrap();
        wait_preview(&mut app);
        let plan = &app.export_status().preview.unwrap().plan;
        let generated = plan.generated_file().unwrap().to_path_buf();
        let (base_mixes, copy_mixes) = plan.mixes.split_at(plan.generated.as_ref().unwrap().1);
        let names = |mixes: &[crate::mixes::StagedMix]| mixes.iter()
            .map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        let (base_names, copy_names) = (names(base_mixes), names(copy_mixes));
        assert!(!base_names.is_empty() && !copy_names.is_empty());
        run_preview(&mut app, with_base(std::slice::from_ref(&target), false));

        let projects = app.project_root.clone().unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(
            &std::fs::read(crate::generate_receipt::path(&projects, &source)).unwrap()).unwrap();
        assert_eq!(receipt["file_name"], generated.file_name().unwrap().to_string_lossy().as_ref());
        assert_eq!(receipt["mixes"], serde_json::json!(base_names), "Copy's mixes are not Generate's");
        let targets = app.export_targets();
        assert_eq!(targets.len(), 2, "the base difficulty is never a target");
        assert!(targets.iter().all(|path| path.file_name() != generated.file_name()));

        let before = wavs(source.parent().unwrap());
        app.request_export_preview(with_base(std::slice::from_ref(&target), false)).unwrap();
        wait_preview(&mut app);
        let plan = &app.export_status().preview.unwrap().plan;
        assert!(plan.mixes.iter().all(|mix| mix.is_reused()), "every mix is already on disk");
        run_preview(&mut app, with_base(&[target], false));
        assert_eq!(wavs(source.parent().unwrap()), before);
    }

    #[test]
    fn a_failed_export_write_rolls_back_the_base_difficulty_too() {
        let (scratch, source, target, _) = mixed_mapset("export-base-rollback");
        let target_bytes = std::fs::read(&target).unwrap();
        let app = opened(&source);
        let plan = output::export_report(
            &output::Source::from_document(app.document().unwrap()),
            &output::ExportConfig {
                defaults: std::path::Path::new(""),
                settings: None,
                projects: app.project_root.as_deref(),
                options: &with_base(std::slice::from_ref(&target), false),
            },
            &|| false,
            &mut Vec::new(), &mut |_, _| {},
        ).unwrap();
        let generated = plan.generated_file().unwrap().to_path_buf();
        let waves = plan.mix_paths().map(PathBuf::from).collect::<Vec<_>>();
        assert_eq!(waves.len(), 2);
        // The write stops once the base difficulty is on disk, before Copy's target.
        let error = plan.apply(&scratch.path("backups"), &|| generated.exists()).unwrap_err();
        assert!(error.contains("Previous targets were restored"), "{error}");
        assert!(!generated.exists());
        assert!(waves.iter().all(|wave| !wave.exists()));
        assert_eq!(std::fs::read(&target).unwrap(), target_bytes);
        assert!(!crate::generate_receipt::path(app.project_root.as_deref().unwrap(), &source).exists());
    }

    #[test]
    fn export_refuses_the_base_difficulty_as_a_copy_target() {
        let (_scratch, source, target, _) = mixed_mapset("export-base-twice");
        let mut app = opened(&source);
        run_preview(&mut app, with_base(&[target], false));
        let generated = crate::generate_receipt::output_path(app.project_root.as_deref().unwrap(), &source).unwrap();
        let generated = generated.canonicalize().unwrap();
        let error = output::export_report(
            &output::Source::from_document(app.document().unwrap()),
            &output::ExportConfig {
                defaults: std::path::Path::new(""),
                settings: None,
                projects: app.project_root.as_deref(),
                options: &with_base(&[generated], false),
            },
            &|| false,
            &mut Vec::new(), &mut |_, _| {},
        ).err().unwrap();
        assert_eq!(error, "The base hitsound difficulty cannot also be a Copy target.");
    }

    #[test]
    fn merge_settings_changed_in_the_editor_make_the_preview_stale() {
        let (_scratch, source, _, _) = mixed_mapset("export-base-settings");
        let mut app = opened(&source);
        let id = app.request_export_preview(with_base(&[], false)).unwrap();
        wait_preview(&mut app);
        assert!(!app.export_status().stale);
        let settings = crate::ExportSettings { pair_samples: true, ..crate::ExportSettings::default() };
        app.set_export_settings(settings.clone()).unwrap();
        assert!(app.export_status().stale);
        assert_eq!(app.export_status().stale_reason, Some(Stale::MergeSettings));
        let backups = app.data_root.join("backups");
        assert!(app.run_export_in(id, backups).is_err());

        // Copy does not use merge settings yet, so a Copy-only preview stays current.
        let (_scratch, source, target, _) = mixed_mapset("export-copy-settings");
        let mut app = opened(&source);
        app.request_export_preview(options(&[target])).unwrap();
        wait_preview(&mut app);
        app.set_export_settings(settings).unwrap();
        assert!(!app.export_status().stale);
        assert_eq!(app.export_status().stale_reason, None);
        edit(&mut app);
        assert_eq!(app.export_status().stale_reason, Some(Stale::Map));
    }

    #[test]
    fn the_base_choice_is_kept_and_older_choices_read_it_as_off() {
        let (_scratch, source, target, _) = mixed_mapset("export-base-choice");
        let mut app = opened(&source);
        app.request_export_preview(with_base(&[target], true)).unwrap();
        let choices = app.document().unwrap().export_choices().unwrap();
        assert!(choices.base && choices.slider_audio);
        let old: ExportChoices =
            serde_json::from_str(r#"{"left_out":[],"match_within":5.0,"copy_volumes":true}"#).unwrap();
        assert!(!old.base && !old.slider_audio && !old.slider_end_mute.enabled);
        assert_eq!(old.mix_format, crate::MixFormat::Wav16);
        let roundtrip: ExportChoices = serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
        assert_eq!(roundtrip.mix_format, crate::MixFormat::Wav16);
    }

    /// The sample index files Generate stages under merge settings, after `already` and from `start`.
    fn indexed_mixes(source: &std::path::Path, already: &[crate::mixes::StagedMix], start: u32) -> Vec<crate::mixes::StagedMix> {
        let document = crate::Document::read(source).unwrap();
        let hits = document.composer.hitsounds();
        let dawsu_hitsounding::generate::GenerateOutcome::Planned { file, problems, .. } = dawsu_hitsounding::generate::generate(
            hits, &document.file, "source.osu", &[], &|| false, &mut |_, _| {},
        ) else {
            panic!("Generate planned nothing");
        };
        let settings = crate::ExportSettings::default();
        crate::mixes::stage_in_file(
            hits, file, &problems, source.parent().unwrap(), std::path::Path::new(""), &[],
            dawsu_hitsounding::Placement::OnObjects, already, Some(&settings), &[], start,
            crate::MixFormat::Wav16, &crate::pending::Pending::default(), &|| false,
        ).unwrap().1.staged
    }

    fn index_names(mixes: &[crate::mixes::StagedMix]) -> Vec<String> {
        mixes.iter().map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn merge_settings_mixes_skip_indexes_staged_earlier_and_honour_the_start_index() {
        let (_scratch, source, _, _) = mixed_mapset("export-indexes");
        let first = indexed_mixes(&source, &[], 2);
        assert!(!first.is_empty() && index_names(&first).iter().all(|name| name.ends_with("2.wav")), "{:?}", index_names(&first));
        let second = indexed_mixes(&source, &first, 2);
        assert!(index_names(&second).iter().all(|name| name.ends_with("3.wav")), "{:?}", index_names(&second));
        let custom = indexed_mixes(&source, &[], 7);
        assert!(index_names(&custom).iter().all(|name| name.ends_with("7.wav")), "{:?}", index_names(&custom));
        let both = first.into_iter().chain(second).collect::<Vec<_>>();
        let published = crate::mixes::publish(&both, None, &|| false).unwrap();
        drop(published);
    }

    /// A source slider whose tick plays soft-slidertick.wav, and a source circle on that tick
    /// with the same file: Generate's slider audio and Copy's timed tick sound render the same WAV.
    fn timed_mapset(name: &str) -> (Scratch, PathBuf, PathBuf) {
        let scratch = Scratch::new(name);
        let header = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:VERSION\n[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]\n0,500,4,2,1,100,1,0\n[HitObjects]\n";
        let slider = "100,100,1000,2,0,L|380:100,1,280,0|0,0:0|0:0,0:0:0:0:\n";
        std::fs::write(scratch.path("source.osu"), format!("{}{slider}256,192,1500,1,0,0:0:0:0:soft-slidertick.wav\n", header.replace("VERSION", "Source"))).unwrap();
        std::fs::write(scratch.path("target.osu"), format!("{}{slider}", header.replace("VERSION", "Target"))).unwrap();
        for (name, value) in [("soft-slidertick.wav", 8192_i16), ("soft-sliderslide.wav", 4096)] {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            for _ in 0..2 {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let canonical = |name: &str| scratch.path(name).canonicalize().unwrap();
        let (source, target) = (canonical("source.osu"), canonical("target.osu"));
        (scratch, source, target)
    }

    fn timed_copy() -> CopyOptions {
        CopyOptions {
            mode: dawsu_hitsounding::copy::CopyMode::OverwriteOnlyDefined,
            copy_to_slider_ticks: true, copy_to_slider_slides: true, ..CopyOptions::default()
        }
    }

    #[test]
    fn copy_never_points_a_target_at_a_wav_generate_owns() {
        let (_scratch, source, target) = timed_mapset("export-timed-owned");
        let mut app = opened(&source);
        let options = ExportOptions { targets: vec![target.clone()], copy: timed_copy(), base: true, slider_audio: true, ..ExportOptions::default() };
        app.request_export_preview(options.clone()).unwrap();
        wait_preview(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let plan = &status.preview.unwrap().plan;
        let count = plan.generated.as_ref().unwrap().1;
        let target_file = &plan.files.iter().find(|file| file.path == target).unwrap().file;
        let used = target_file.storyboard_samples().into_iter().map(|event| event.path).collect::<Vec<_>>();
        let generated = plan.mixes[..count].iter()
            .map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(!used.is_empty());
        assert!(used.iter().all(|name| !generated.contains(name)), "{used:?} uses {generated:?}");

        run_preview(&mut app, options);
        let exported = folder_files(source.parent().unwrap());
        let (_set, reference, reference_target) = timed_mapset("export-timed-owned-reference");
        generate_then_copy(&reference, reference_target, &timed_copy(), true);
        assert_eq!(exported, folder_files(reference.parent().unwrap()));
    }

    /// Every .wav in the mapset folder with its bytes.
    fn wav_files(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        folder_files(root).into_iter().filter(|(name, _)| name.ends_with(".wav")).collect()
    }

    fn index_of(name: &str) -> u32 {
        let stem = name.strip_suffix(".wav").unwrap();
        stem[stem.trim_end_matches(|c: char| c.is_ascii_digit()).len()..].parse().unwrap()
    }

    #[test]
    fn with_merge_settings_base_and_copy_never_share_an_index_and_re_export_adds_none() {
        // Generate mixes the circle under merge settings (indexed); Copy mixes a slider tail (indexed).
        let scratch = Scratch::new("export-merge-indexes");
        let header = MIXED.replace("[TimingPoints]", "[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n[TimingPoints]");
        let source = scratch.path("source.osu");
        std::fs::write(&source, format!("{}256,192,1500,1,0,0:0:0:40:one.wav\n300,192,1500,1,0,0:0:0:80:two.wav\n", header.replace("VERSION", "Source"))).unwrap();
        let target = scratch.path("target.osu");
        std::fs::write(&target, format!("{}256,192,1000,2,0,L|396:192,1,140,0|0,1:0|1:0,1:0:0:0:\n", header.replace("VERSION", "Target"))).unwrap();
        for (name, value) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let mut wav = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x04\0\0\0".to_vec();
            for _ in 0..2 {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let (source, target) = (source.canonicalize().unwrap(), target.canonicalize().unwrap());
        let root = source.parent().unwrap().to_path_buf();
        let mut app = opened(&source);
        app.set_export_settings(crate::ExportSettings::default()).unwrap();
        let copy = |start_index| CopyOptions {
            mode: dawsu_hitsounding::copy::CopyMode::OverwriteOnlyDefined, start_index, ..CopyOptions::default()
        };
        let export = |start_index| ExportOptions { targets: vec![target.clone()], copy: copy(start_index), base: true, slider_audio: false, ..ExportOptions::default() };
        let names = |mixes: &[crate::mixes::StagedMix]| mixes.iter()
            .map(|mix| mix.path.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();

        // A custom start index is honoured where it is free.
        app.request_export_preview(export(10)).unwrap();
        wait_preview(&mut app);
        assert_eq!(app.export_status().error, None);
        let plan = &app.export_status().preview.unwrap().plan;
        let count = plan.generated.as_ref().unwrap().1;
        let copy_names = names(&plan.mixes[count..]);
        assert_eq!(copy_names.iter().map(|name| index_of(name)).collect::<Vec<_>>(), [10], "{copy_names:?}");

        // Start 2 is Generate's: Copy takes the next free index, and no index holds two WAVs.
        let before = wav_files(&root);
        app.request_export_preview(export(2)).unwrap();
        wait_preview(&mut app);
        let plan = &app.export_status().preview.unwrap().plan;
        let count = plan.generated.as_ref().unwrap().1;
        let (base_names, copy_names) = (names(&plan.mixes[..count]), names(&plan.mixes[count..]));
        let indexes = |names: &[String]| names.iter().map(|name| index_of(name)).collect::<std::collections::BTreeSet<_>>();
        assert_eq!(indexes(&base_names), [2].into(), "{base_names:?}");
        assert_eq!(indexes(&copy_names), [3].into(), "{copy_names:?}");
        run_preview(&mut app, export(2));
        let written = wav_files(&root);
        assert_eq!(written.len(), before.len() + base_names.len() + copy_names.len(), "one file per staged mix");

        let projects = app.project_root.clone().unwrap();
        let receipt: serde_json::Value = serde_json::from_slice(
            &std::fs::read(crate::generate_receipt::path(&projects, &source)).unwrap()).unwrap();
        assert_eq!(receipt["mixes"], serde_json::json!(base_names), "only the base difficulty's mixes");

        app.request_export_preview(export(2)).unwrap();
        wait_preview(&mut app);
        assert!(app.export_status().preview.unwrap().plan.mixes.iter().all(|mix| mix.is_reused()));
        run_preview(&mut app, export(2));
        assert_eq!(wav_files(&root), written, "re-export adds and changes no WAV");
    }

    /// Asks for a base-only preview and answers its refusal; nothing in the mapset changes.
    fn base_refusal(app: &mut Application) -> String {
        let root = app.document().unwrap().path.parent().unwrap().to_path_buf();
        let before = folder_files(&root);
        app.request_export_preview(with_base(&[], false)).unwrap();
        wait_preview(app);
        let status = app.export_status();
        assert!(status.preview.is_none());
        let error = status.error.expect("Generate refused").to_owned();
        assert_eq!(folder_files(&root), before, "nothing is written");
        error
    }

    #[test]
    fn a_foreign_hitsounds_difficulty_refuses_the_base() {
        let (scratch, source, _, _) = mixed_mapset("export-base-foreign");
        std::fs::write(scratch.path("theirs.osu"), format!("{}100,100,1000,1,0,0:0:0:0:\n", MIXED.replace("VERSION", "Hitsounds"))).unwrap();
        let mut app = opened(&source);
        assert_eq!(base_refusal(&mut app), "A difficulty in this mapset is already called \"Hitsounds\". Rename it, then generate again.");
    }

    #[test]
    fn a_source_called_hitsounds_refuses_the_base() {
        let (scratch, _, _, _) = mixed_mapset("export-base-source-hitsounds");
        let source = scratch.path("Artist - Title (Mapper) [Hitsounds].osu");
        std::fs::write(&source, format!("{}256,192,1000,1,0,0:0:0:40:one.wav\n", MIXED.replace("VERSION", "Hitsounds"))).unwrap();
        let mut app = opened(&source.canonicalize().unwrap());
        assert_eq!(base_refusal(&mut app), "A difficulty already has the Generate output name. Only an earlier output tracked by this editor can be replaced.");
    }

    #[test]
    fn a_base_difficulty_edited_outside_is_kept() {
        let (_scratch, source, _, _) = mixed_mapset("export-base-edited");
        let mut app = opened(&source);
        run_preview(&mut app, with_base(&[], false));
        let generated = crate::generate_receipt::output_path(app.project_root.as_deref().unwrap(), &source).unwrap();
        let edited = std::fs::read_to_string(&generated).unwrap().replace("Version:Hitsounds", "Version:Hitsounds\nTags:edited");
        std::fs::write(&generated, &edited).unwrap();
        assert_eq!(base_refusal(&mut app), "The output changed since Generate. The existing difficulty was kept.");
        assert_eq!(std::fs::read_to_string(&generated).unwrap(), edited);
    }

    #[test]
    fn export_leaves_out_a_removed_lane_and_its_hits() {
        let scratch = Scratch::new("export-removed-lane");
        let header = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Test\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n";
        let source = scratch.path("source.osu");
        let target = scratch.path("target.osu");
        // A finish on the map's own normal-hitfinish.wav, and a whistle.
        std::fs::write(&source, format!("{header}256,192,1000,1,4,1:0:1:0:\n256,192,1500,1,2,0:0:0:0:\n")).unwrap();
        std::fs::write(&target, format!("{header}100,100,1000,1,0,0:0:0:0:\n100,100,1500,1,0,0:0:0:0:\n")).unwrap();
        std::fs::write(scratch.path("normal-hitfinish.wav"),
            b"RIFF\x26\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0w\x01\0\x02\0\x10\0data\x02\0\0\0\0\x40").unwrap();
        let mut app = opened(&source.canonicalize().unwrap());
        let lane = dawsu_hitsounding::rules::read_sample_file_name("normal-hitfinish.wav").unwrap();
        let path = app.document().unwrap().path.clone();
        let revision = app.document().unwrap().composer.history.version();
        app.delete_lane(&path, &lane, revision).unwrap();
        let options = ExportOptions { targets: vec![target.canonicalize().unwrap()], ..ExportOptions::default() };
        app.request_export_preview(options).unwrap();
        wait_preview(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let preview = status.preview.expect("a finished preview");
        let written = String::from_utf8(preview.plan.files[0].file.to_bytes()).unwrap();
        assert!(written.contains("100,100,1000,1,0,"), "no finish at 1000: {written}");
        assert!(written.contains("100,100,1500,1,2,"), "the whistle stays: {written}");
        assert!(preview.plan.mixes.is_empty());
    }

    #[test]
    fn export_with_ogg_vorbis_and_wav32f_stages_publishes_and_reuses_mixes() {
        let scratch = Scratch::new("export-mix-formats");
        let header = "osu file format v14\n[General]\nMode:0\n[Metadata]\nArtist:Artist\nTitle:Title\nCreator:Mapper\nVersion:Test\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n";
        let source = scratch.path("source.osu");
        let target = scratch.path("target.osu");
        std::fs::write(&source, format!("{header}256,192,1000,1,0,0:0:0:40:one.wav\n300,192,1000,1,0,0:0:0:80:two.wav\n")).unwrap();
        std::fs::write(&target, format!("{header}100,100,1000,1,0,0:0:0:0:\n")).unwrap();
        for (name, value) in [("one.wav", 8192_i16), ("two.wav", 16384)] {
            let frames = 2048usize;
            let data_size = (frames * 2) as u32;
            let mut wav = Vec::with_capacity(44 + data_size as usize);
            wav.extend(b"RIFF");
            wav.extend((36 + data_size).to_le_bytes());
            wav.extend(b"WAVEfmt ");
            wav.extend(16u32.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(48_000u32.to_le_bytes());
            wav.extend((48_000u32 * 2).to_le_bytes());
            wav.extend(2u16.to_le_bytes());
            wav.extend(16u16.to_le_bytes());
            wav.extend(b"data");
            wav.extend(data_size.to_le_bytes());
            for _ in 0..frames {
                wav.extend(value.to_le_bytes());
            }
            std::fs::write(scratch.path(name), wav).unwrap();
        }
        let source = source.canonicalize().unwrap();
        let target = target.canonicalize().unwrap();
        let mut app = opened(&source);

        // 1. Ogg Vorbis: stages .ogg, publishes, decodes cleanly, and re-export reuses it.
        let ogg_opts = ExportOptions {
            targets: vec![target.clone()],
            mix_format: crate::MixFormat::OggVorbis,
            ..ExportOptions::default()
        };
        let id = app.request_export_preview(ogg_opts.clone()).unwrap();
        wait_preview(&mut app);
        assert_eq!(app.export_status().error, None);
        let ogg_paths: Vec<PathBuf> = app.export_status().preview.unwrap().plan.mix_paths().map(PathBuf::from).collect();
        assert_eq!(ogg_paths.len(), 1);
        assert_eq!(ogg_paths[0].extension().and_then(|e| e.to_str()), Some("ogg"));
        assert_eq!(app.document().unwrap().export_choices().unwrap().mix_format, crate::MixFormat::OggVorbis);
        app.run_export(id).unwrap();
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(ogg_paths[0].exists());
        assert_eq!(&std::fs::read(&ogg_paths[0]).unwrap()[0..4], b"OggS");
        assert!(dawsu_audio::decode(&ogg_paths[0]).is_ok());

        // Re-exporting identical mix with OggVorbis reuses the existing .ogg file byte-for-byte.
        app.request_export_preview(ogg_opts).unwrap();
        wait_preview(&mut app);
        let preview = app.export_status().preview.unwrap();
        assert_eq!(preview.plan.mixes.len(), 1);
        assert!(preview.plan.mixes[0].is_reused(), "identical Ogg Vorbis mix should be reused");

        // 2. 32-bit float WAV: stages IEEE float WAV (format tag = 3) and publishes cleanly.
        let wav32f_opts = ExportOptions {
            targets: vec![target],
            mix_format: crate::MixFormat::Wav32f,
            ..ExportOptions::default()
        };
        let id = app.request_export_preview(wav32f_opts).unwrap();
        wait_preview(&mut app);
        assert_eq!(app.export_status().error, None);
        let float_paths: Vec<PathBuf> = app.export_status().preview.unwrap().plan.mix_paths().map(PathBuf::from).collect();
        assert_eq!(float_paths.len(), 1);
        assert_eq!(float_paths[0].extension().and_then(|e| e.to_str()), Some("wav"));
        app.run_export(id).unwrap();
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        let bytes = std::fs::read(&float_paths[0]).unwrap();
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 3, "IEEE float format tag");
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 32, "32 bits per sample");
        assert!(dawsu_audio::decode(&float_paths[0]).is_ok());
    }

    #[test]
    fn hitsound_only_export_writes_isolated_folder_and_leaves_mapset_untouched() {
        let map = mapset("export-hitsound-only");
        std::fs::write(map.scratch.path("song.mp3"), b"fake-audio-bytes").unwrap();
        let src_with_audio = std::fs::read_to_string(&map.source)
            .unwrap()
            .replace("[General]\nMode:0", "[General]\nAudioFilename: song.mp3\nMode:0");
        std::fs::write(&map.source, src_with_audio).unwrap();
        let tgt_with_audio = std::fs::read_to_string(&map.target)
            .unwrap()
            .replace("[General]\nMode:0", "[General]\nAudioFilename: song.mp3\nMode:0");
        std::fs::write(&map.target, &tgt_with_audio).unwrap();

        let out_dir = map.scratch.path("standalone-hs-only");
        let mut app = opened(&map.source);
        let opts = ExportOptions {
            targets: vec![map.target.clone()],
            write_mode: ExportWriteMode::HitsoundOnly,
            hitsound_only_dir: Some(out_dir.clone()),
            ..ExportOptions::default()
        };
        let id = app.request_export_preview(opts).unwrap();
        wait_preview(&mut app);
        assert_eq!(app.export_status().error, None);
        app.run_export(id).unwrap();

        // Isolated folder has target .osu and copied song; real mapset target is untouched.
        assert!(out_dir.join("Target.osu").is_file());
        assert!(out_dir.join("song.mp3").is_file());
        assert_eq!(
            std::fs::read_to_string(&map.target).unwrap(),
            tgt_with_audio,
            "real mapset target must stay byte-identical in HitsoundOnly mode"
        );
    }

    #[test]
    fn hitsound_only_export_is_the_last_export_open_folders_shows() {
        let map = mapset("export-last-export-folder");
        let out_dir = map.scratch.path("last-hs-only");
        let mut app = opened(&map.source);
        assert_eq!(app.last_hitsound_only_export(), None, "nothing exported yet");
        let opts = ExportOptions {
            targets: vec![map.target.clone()],
            write_mode: ExportWriteMode::HitsoundOnly,
            hitsound_only_dir: Some(out_dir.clone()),
            ..ExportOptions::default()
        };
        let id = app.request_export_preview(opts.clone()).unwrap();
        wait_preview(&mut app);
        app.run_export(id).unwrap();
        assert_eq!(app.last_hitsound_only_export(), Some(out_dir.as_path()));
        // Closing Export (ExportClose) forgets the preview, never where the files went.
        app.cancel_export_preview();
        assert_eq!(app.last_hitsound_only_export(), Some(out_dir.as_path()));
        // A Direct preview afterwards keeps it too.
        app.request_export_preview(ExportOptions { write_mode: ExportWriteMode::Direct, ..opts }).unwrap();
        assert_eq!(app.last_hitsound_only_export(), Some(out_dir.as_path()));
    }

    #[test]
    fn export_choices_sent_just_before_close_are_kept() {
        // The Export window's close sends a pending preview, then ExportClose at once:
        // request_export_preview stores the choices before it starts the background preview.
        let map = mapset("export-choices-before-close");
        let mut app = opened(&map.source);
        let opts = ExportOptions { targets: vec![map.target.clone()], base: true, ..ExportOptions::default() };
        app.request_export_preview(opts).unwrap();
        app.cancel_export_preview();
        let choices = app.document().unwrap().export_choices().unwrap();
        assert!(choices.base);
        assert!(choices.left_out.is_empty());
    }

    #[test]
    fn export_applies_volume_envelope_without_creating_extra_mixes() {
        let map = mapset("export-volume-envelope");
        let mut app = opened(&map.source);
        app.shared_base_set_volume_envelope(
            None,
            VolumeEnvelope {
                points: vec![
                    dawsu_hitsounding::volume_envelope::VolumePoint {
                        time_ms: 0,
                        volume: 50,
                        curve: dawsu_hitsounding::volume_envelope::CurveKind::Step,
                    },
                ],
            },
        )
        .unwrap();

        let opts = ExportOptions {
            targets: vec![map.target.clone()],
            apply_volume_envelope: true,
            ..ExportOptions::default()
        };
        let _id = app.request_export_preview(opts).unwrap();
        wait_preview(&mut app);
        let status = app.export_status();
        assert_eq!(status.error, None);
        let preview = status.preview.unwrap();
        assert_eq!(preview.plan.mixes.len(), 0, "volume curve must create 0 extra sample mixes");
        let target_plan = preview.plan.files.iter().find(|f| f.path == map.target).unwrap();
        assert!(
            target_plan.file.timing_points().iter().any(|tp| tp.volume() == Some(50)),
            "expected green/timing line scaled to 50% by volume curve"
        );
    }

    #[test]
    fn export_renders_slider_ticks_and_restores_on_repeat_export() {
        let scratch = Scratch::new("export-tick-render");
        // Write a real 16-bit PCM WAV sample for the source tick sound.
        let mut wav = Vec::from(dawsu_hitsounding::index_packing::STANDARD_SILENT_WAV);
        let samples = [0i16, 12000, -12000, 8000, -8000, 0, 0, 0];
        let data_bytes = (samples.len() * 2) as u32;
        wav[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
        wav[40..44].copy_from_slice(&data_bytes.to_le_bytes());
        for s in samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(scratch.path("tick.wav"), &wav).unwrap();

        // Source has a hit at 1500 ms (matches the target slider's midpoint tick!).
        let source_osu = concat!(
            "osu file format v14\n\n",
            "[General]\nMode:0\n\n",
            "[Metadata]\nVersion:Source\n\n",
            "[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n",
            "[TimingPoints]\n0,500,4,1,0,100,1,0\n\n",
            "[HitObjects]\n",
            "256,192,1000,1,0,0:0:0:80:\n",
            "256,192,1500,1,0,0:0:0:85:tick.wav\n",
        );
        // Target has a 2-beat slider starting at 1000 ms -> tick at 1500 ms!
        let target_osu = concat!(
            "osu file format v14\n\n",
            "[General]\nMode:0\n\n",
            "[Metadata]\nVersion:Target\n\n",
            "[Difficulty]\nSliderMultiplier:1.4\nSliderTickRate:1\n\n",
            "[TimingPoints]\n0,500,4,1,0,100,1,0\n\n",
            "[HitObjects]\n",
            "100,192,1000,2,0,L|380:192,1,280,0|0,0:0|0:0,0:0:0:0:\n",
        );
        let source = scratch.path("Source.osu");
        let target = scratch.path("Target.osu");
        std::fs::write(&source, source_osu).unwrap();
        std::fs::write(&target, target_osu).unwrap();
        let source = source.canonicalize().unwrap();
        let target = target.canonicalize().unwrap();

        let mut app = opened(&source);
        let opts = ExportOptions {
            targets: vec![target.clone()],
            tick_policy: TickPolicy::Render,
            ..ExportOptions::default()
        };
        let id = app.request_export_preview(opts.clone()).unwrap();
        wait_preview(&mut app);
        let preview = app.export_status().preview.unwrap();
        assert!(
            preview.plan.mixes.iter().any(|m| m.path.to_string_lossy().contains("normal-slidertick")),
            "expected staged normal-slidertick custom file"
        );
        let planned_target = preview.plan.files.iter().find(|f| f.path == target).unwrap();
        let tps = planned_target.file.timing_points();
        assert!(
            tps.iter().any(|tp| (tp.time() - 1500.0).abs() <= 1.0 && tp.sample_index().unwrap_or(0) >= 100),
            "expected tick green line at 1500 ms with custom index >= 100, got: {tps:?}"
        );
        assert!(
            tps.iter().any(|tp| (tp.time() - 1505.0).abs() <= 1.0),
            "expected +5 ms revert green line at 1505 ms"
        );

        app.run_export(id).unwrap();
        wait_main(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);

        // Repeat export restores previous tick green lines cleanly without duplicating them.
        let id2 = app.request_export_preview(opts).unwrap();
        wait_preview(&mut app);
        let preview2 = app.export_status().preview.unwrap();
        let planned_target2 = preview2.plan.files.iter().find(|f| f.path == target).unwrap();
        let tick_lines_count = planned_target2
            .file
            .timing_points()
            .iter()
            .filter(|tp| (tp.time() - 1500.0).abs() <= 1.0 && tp.sample_index().unwrap_or(0) >= 100)
            .count();
        assert_eq!(tick_lines_count, 1, "repeat export must restore prior ticks instead of stacking duplicates (id2={id2})");
    }

    #[test]
    fn failed_direct_export_keeps_previously_owned_samples_and_unselected_diff_samples() {
        let map = mapset("export-manifest-deferred");
        let stale_path = map.scratch.path("dawsu-mix-old.wav");
        let sibling_used_path = map.scratch.path("dawsu-mix-sibling.wav");
        std::fs::write(&stale_path, b"RIFFold").unwrap();
        std::fs::write(&sibling_used_path, b"RIFFsib").unwrap();
        let sibling_osu = map.scratch.path("Sibling.osu");
        std::fs::write(
            &sibling_osu,
            MAP.replace("DIFF", "Sibling")
                .replace("EVENTS", "Sample,1000,0,\"dawsu-mix-sibling.wav\",80"),
        )
        .unwrap();

        let mut app = opened(&map.source);
        let root = map.source.parent().unwrap().to_path_buf();
        let manifest = export_manifest::ExportOwnershipManifest {
            version: export_manifest::MANIFEST_VERSION,
            owned_samples: BTreeMap::from([
                (
                    "dawsu-mix-old.wav".into(),
                    dawsu_foundation::digest::sha256_hex(b"RIFFold"),
                ),
                (
                    "dawsu-mix-sibling.wav".into(),
                    dawsu_foundation::digest::sha256_hex(b"RIFFsib"),
                ),
            ]),
            ..Default::default()
        };
        export_manifest::save_manifest(&app.data_root, &root, &manifest).unwrap();

        // 1. If Target is modified externally before write completes, Export fails and MUST NOT delete owned samples.
        let id = app
            .request_export_preview(ExportOptions {
                targets: vec![map.target.clone()],
                ..ExportOptions::default()
            })
            .unwrap();
        wait_preview(&mut app);
        let modified_target = format!("{}// external edit\n", map.target_bytes);
        std::fs::write(&map.target, &modified_target).unwrap();
        app.run_export(id).unwrap();
        wait_main(&mut app);
        assert!(app.export_status().write_error.is_some());
        assert!(
            stale_path.is_file(),
            "failed export must not clean up previously owned samples"
        );
        assert!(sibling_used_path.is_file());

        // 2. Re-preview and succeed: stale owned sample is removed, while unselected sibling sample is kept!
        let id2 = app
            .request_export_preview(ExportOptions {
                targets: vec![map.target.clone()],
                ..ExportOptions::default()
            })
            .unwrap();
        wait_preview(&mut app);
        app.run_export(id2).unwrap();
        wait_main(&mut app);
        assert_eq!(app.export_status().write_error, None);
        assert!(!stale_path.is_file(), "stale unused dawsu-mix should be archived");
        assert!(
            sibling_used_path.is_file(),
            "sample still referenced by unselected sibling diff must be preserved"
        );
    }

    #[test]
    fn export_preview_volume_envelope_and_lt5_untouched_pass_verification() {
        let scratch = Scratch::new("export-envelope-verify");
        let source_path = scratch.path("Source.osu").canonicalize().unwrap_or_else(|_| scratch.path("Source.osu"));
        let target_path = scratch.path("Target.osu");
        let src_bytes = "osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Source\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,2,0:0:0:80:\n256,192,2000,1,0,0:0:0:3:\n";
        let dst_bytes = "osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Target\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:3:\n";
        std::fs::write(&source_path, src_bytes).unwrap();
        std::fs::write(&target_path, dst_bytes).unwrap();
        let source_canon = source_path.canonicalize().unwrap();
        let target_canon = target_path.canonicalize().unwrap();

        let mut app = opened(&source_canon);
        app.shared_base_set_volume_envelope(
            None,
            VolumeEnvelope {
                points: vec![dawsu_hitsounding::volume_envelope::VolumePoint {
                    time_ms: 0,
                    volume: 75,
                    curve: dawsu_hitsounding::volume_envelope::CurveKind::Step,
                }],
            },
        )
        .unwrap();
        let id = app
            .request_export_preview(ExportOptions {
                targets: vec![target_canon],
                apply_volume_envelope: true,
                ..ExportOptions::default()
            })
            .unwrap();
        wait_preview(&mut app);
        let preview = app.export.preview.as_ref().expect("preview should finish");
        assert!(
            preview.plan.verification.passed,
            "expected envelope preview to verify cleanly, got: {:?}",
            preview.plan.verification.files
        );
        let diff = preview.plan.files[0].diff_summary();
        assert!(diff.added > 0);
        assert!(preview.plan.files[0].verification().is_some_and(|v| v.passed));
        app.run_export(id).unwrap();
        wait_main(&mut app);
        assert_eq!(app.export_status().write_error, None);
    }

    #[test]
    fn export_preview_tick_render_and_clean_greenlines_pass_verification() {
        let scratch = Scratch::new("export-tick-clean-verify");
        let source_path = scratch.path("Source.osu");
        let target_path = scratch.path("Target.osu");
        let mut wav = Vec::from(dawsu_hitsounding::index_packing::STANDARD_SILENT_WAV);
        let samples = [0i16, 10000, -10000, 5000, -5000, 0];
        let data_bytes = (samples.len() * 2) as u32;
        wav[4..8].copy_from_slice(&(36 + data_bytes).to_le_bytes());
        wav[40..44].copy_from_slice(&data_bytes.to_le_bytes());
        for s in samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(scratch.path("normal-hitclap.wav"), &wav).unwrap();
        std::fs::write(scratch.path("normal-hitnormal.wav"), &wav).unwrap();
        let src_bytes = "osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Source\n\n[Difficulty]\nSliderTickRate:1\nSliderMultiplier:1.4\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n500,-100,4,1,0,100,0,0\n500,-100,4,1,0,100,0,0\n\n[HitObjects]\n256,192,1000,1,2,0:0:0:80:\n256,192,1250,1,2,0:0:0:80:\n";
        let dst_bytes = "osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Target\n\n[Difficulty]\nSliderTickRate:1\nSliderMultiplier:1.4\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,2,0,B|356:192,1,280\n";
        std::fs::write(&source_path, src_bytes).unwrap();
        std::fs::write(&target_path, dst_bytes).unwrap();
        let source_canon = source_path.canonicalize().unwrap();
        let target_canon = target_path.canonicalize().unwrap();

        let mut app = opened(&source_canon);
        let id = app
            .request_export_preview(ExportOptions {
                targets: vec![target_canon],
                tick_policy: dawsu_hitsounding::tick_render::TickPolicy::Render,
                ..ExportOptions::default()
            })
            .unwrap();
        wait_preview(&mut app);
        let preview = app.export.preview.as_ref().expect("preview should exist");
        assert!(
            preview.plan.verification.passed,
            "tick render + cleanup should pass read-back verification: {:?}",
            preview.plan.verification.files
        );
        app.run_export(id).unwrap();
        wait_main(&mut app);
        assert_eq!(app.export_status().write_error, None);
    }

    #[test]
    fn export_preview_preserves_legacy_map_warnings_and_blocks_corrupted_write() {
        let scratch = Scratch::new("export-legacy-warn-and-gate");
        let source_path = scratch.path("Source.osu");
        let target_path = scratch.path("Target.osu");
        let src_bytes = "osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Source\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,2,0:0:0:0:\n";
        // Include a pre-existing malformed line in [Events] so baseline has a legacy ReadWarning.
        let dst_bytes = "osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Target\n\n[Events]\nmalformed_legacy_event\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        std::fs::write(&source_path, src_bytes).unwrap();
        std::fs::write(&target_path, dst_bytes).unwrap();
        let source_canon = source_path.canonicalize().unwrap();
        let target_canon = target_path.canonicalize().unwrap();

        let mut app = opened(&source_canon);
        let id = app
            .request_export_preview(ExportOptions {
                targets: vec![target_canon],
                ..ExportOptions::default()
            })
            .unwrap();
        wait_preview(&mut app);
        {
            let preview = app.export.preview.as_mut().expect("preview should finish");
            assert!(
                preview.plan.verification.passed,
                "legacy baseline warning must not fail verification: {:?}",
                preview.plan.verification.files
            );
            // Simulate a failed verification check to test hard pre-write gate & modal preservation.
            preview.plan.verification.passed = false;
            preview.plan.verification.failed_files = 1;
            if let Some(first) = preview.plan.verification.files.first_mut() {
                first.passed = false;
                first.checks[2].passed = false;
                first.checks[2].detail = "Injected read-back mismatch".into();
            }
        }
        let err = app
            .run_export_in(id, scratch.path("backups"))
            .expect_err("export must block write when verification fails");
        assert!(err.contains("Read-back check failed"), "unexpected error: {err}");
        assert!(
            app.export.preview.is_some(),
            "failed export must preserve preview so modal keeps diagnostics"
        );
    }

    #[test]
    fn export_preview_hitsound_only_with_custom_wav_passes_verification() {
        let scratch = Scratch::new("export-hsonly-custom-wav");
        let source_path = scratch.path("Source.osu");
        let target_path = scratch.path("Target.osu");
        std::fs::write(
            scratch.path("drum-hitclap5.wav"),
            dawsu_hitsounding::index_packing::STANDARD_SILENT_WAV,
        )
        .unwrap();
        let src_bytes = "osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Source\n\n[Events]\n\n[TimingPoints]\n0,500,4,3,5,100,1,0\n\n[HitObjects]\n256,192,1000,1,2,0:0:0:0:\n";
        let dst_bytes = "osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Target\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        std::fs::write(&source_path, src_bytes).unwrap();
        std::fs::write(&target_path, dst_bytes).unwrap();
        let source_canon = source_path.canonicalize().unwrap();
        let target_canon = target_path.canonicalize().unwrap();

        let out_dir = scratch.path("isolated-out");
        let mut app = opened(&source_canon);
        let id = app
            .request_export_preview(ExportOptions {
                targets: vec![target_canon.clone()],
                write_mode: export_manifest::ExportWriteMode::HitsoundOnly,
                hitsound_only_dir: Some(out_dir.clone()),
                ..ExportOptions::default()
            })
            .unwrap();
        wait_preview(&mut app);
        let preview = app.export.preview.as_ref().expect("preview should finish");
        assert!(
            preview.plan.verification.passed,
            "HitsoundOnly with existing mapset sample must verify cleanly: {:?}",
            preview.plan.verification.files
        );
        let samples_check = preview.plan.verification.files[0]
            .checks
            .iter()
            .find(|c| c.id == "samples")
            .unwrap();
        assert!(samples_check.detail.contains("hitsound-only"));
        app.run_export_in(id, scratch.path("backups")).unwrap();
        assert!(out_dir.join("Target.osu").is_file());

        // Also confirm HitsoundOnly blocks run_export_in if read-back verification fails.
        let blocked_dir = scratch.path("blocked-out");
        let blocked_id = app
            .request_export_preview(ExportOptions {
                targets: vec![target_canon.clone()],
                write_mode: export_manifest::ExportWriteMode::HitsoundOnly,
                hitsound_only_dir: Some(blocked_dir.clone()),
                ..ExportOptions::default()
            })
            .unwrap();
        wait_preview(&mut app);
        if let Some(preview) = app.export.preview.as_mut() {
            preview.plan.verification.passed = false;
            preview.plan.verification.failed_files = 1;
            if let Some(first) = preview.plan.verification.files.first_mut() {
                first.passed = false;
                first.checks[0].passed = false;
                first.checks[0].detail = "Injected HitsoundOnly header failure".into();
            }
        }
        let err = app
            .run_export_in(blocked_id, scratch.path("backups"))
            .expect_err("HitsoundOnly must also block writes when verification fails");
        assert!(err.contains("Read-back check failed"), "unexpected error: {err}");
        assert!(
            !blocked_dir.exists(),
            "blocked HitsoundOnly export must not create the output folder"
        );
        assert!(
            app.export.preview.is_some(),
            "blocked HitsoundOnly export must preserve preview diagnostics"
        );

        // Post-preview tampering of HitsoundOnly PlannedFile (valid header, wrong hitsound)
        // must be caught by run_export_in even if cached plan.verification.passed is true.
        let tampered_dir = scratch.path("tampered-out");
        let tampered_id = app
            .request_export_preview(ExportOptions {
                targets: vec![target_canon.clone()],
                write_mode: export_manifest::ExportWriteMode::HitsoundOnly,
                hitsound_only_dir: Some(tampered_dir.clone()),
                ..ExportOptions::default()
            })
            .unwrap();
        wait_preview(&mut app);
        if let Some(preview) = app.export.preview.as_mut() {
            let tampered_osu = b"osu file format v14\n\n[General]\nMode:0\nAudioFilename:a.mp3\n\n[Metadata]\nVersion:Target\n\n[Events]\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
            preview.plan.files[0].replace_file(dawsu_osu_format::OsuFile::read(tampered_osu.to_vec()).file);
        }
        let err = app
            .run_export_in(tampered_id, scratch.path("backups"))
            .expect_err("HitsoundOnly must block post-preview hitsound tampering");
        assert!(err.contains("Save verification stopped"), "unexpected error: {err}");
        assert!(!tampered_dir.exists(), "tampered HitsoundOnly must not write directory");
        assert!(app.export.preview.is_some(), "tampered HitsoundOnly must preserve preview");
    }
}
