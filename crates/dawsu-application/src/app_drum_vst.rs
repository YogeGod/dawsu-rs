use crate::drum_vst;
use crate::vst3::{
    self, BounceOutput, LoadedPluginSummary, PluginRole, VelocityExportMode, Vst3DrumExportPlan,
    Vst3DrumHit, Vst3Error, Vst3Manager, plan_vst3_drum_export,
};
use crate::{Application, DrumVstSource, JobResult, bounce_plan, lane_name, mixes, pending, project};
use dawsu_audio::AudioClip;
use dawsu_foundation::SampleIdentity;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

impl Application {
    pub(crate) fn clear_live_drum_vst(&mut self) {
        if let Some(a) = &self.audio {
            a.clear_live_source();
        }
        if let Some(vst) = self.drum_vst.take() {
            let _ = self.vst3.unload_plugin(&vst.instance_id);
        }
        self.drum_vst_state_b64 = None;
        self.bump_view_revision();
    }

    pub(crate) fn attach_drum_vst_instance(
        &mut self,
        summary: &LoadedPluginSummary,
        plugin_path: &Path,
        plugin_uid: Option<String>,
        enabled: bool,
        state_b64: Option<String>,
    ) -> Result<(), Vst3Error> {
        let shm = self.vst3.take_shm(&summary.id)?.ok_or_else(|| {
            Vst3Error::Bounce("VST3 shared-memory audio channel unavailable".to_string())
        })?;
        // Detach any outgoing instrument only after the replacement loads so a failure
        // leaves both the live setup and the saved per-map session intact.
        self.clear_live_drum_vst();
        let (mut handle, source) = drum_vst::DrumVstLiveHandle::new(
            summary.id.clone(),
            summary.name.clone(),
            plugin_path.to_string_lossy().into_owned(),
            plugin_uid,
            enabled,
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
        self.drum_vst_state_b64 = state_b64;
        self.bump_view_revision();
        Ok(())
    }

    pub(crate) fn clear_drum_vst_loading_status(&mut self) {
        if self.status.contains(" (Loading drum kit…)") {
            self.status = self.status.replace(" (Loading drum kit…)", "");
        } else if self.status == "Loading drum kit…" {
            self.status = "Difficulty loaded".into();
        }
        self.bump_view_revision();
    }

    pub(crate) fn set_drum_vst_unavailable_status(&mut self, err: &str) {
        let note = format!("Drum VST3 unavailable: {err}");
        if self.status.contains(" (Loading drum kit…)") {
            self.status = self.status.replace(" (Loading drum kit…)", &format!(" ({note})"));
        } else if self.status == "Loading drum kit…" || self.status == "Difficulty loaded" {
            self.status = format!("Difficulty loaded ({note})");
        } else {
            self.status = note;
        }
        self.bump_view_revision();
    }

    pub(crate) fn spawn_drum_vst_restore(&mut self, saved: project::DrumVstSession) {
        self.spawn_drum_vst_load_inner(saved, false);
    }

    pub fn spawn_load_drum_vst(&mut self, plugin_path: PathBuf, plugin_uid: Option<String>) {
        let saved = project::DrumVstSession {
            plugin_path: plugin_path.to_string_lossy().into_owned(),
            plugin_uid,
            enabled: true,
            state_b64: None,
        };
        self.spawn_drum_vst_load_inner(saved, true);
    }

    fn spawn_drum_vst_load_inner(
        &mut self,
        saved: project::DrumVstSession,
        write_session_on_attach: bool,
    ) {
        self.drum_vst_cancel.store(true, Ordering::Relaxed);
        self.drum_vst_generation = self.drum_vst_generation.wrapping_add(1);
        let cancel = Arc::new(AtomicBool::new(false));
        self.drum_vst_cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.drum_vst_job = Some(receive);
        self.drum_vst_loading = true;
        self.async_jobs.drum_vst_write_session_on_attach = write_session_on_attach;
        self.bump_view_revision();
        let generation = self.drum_vst_generation;
        let host_exe = self.vst3.host_executable();
        let shm_name = self.vst3.next_shm_name();
        let pid = std::process::id();
        #[cfg(test)]
        let hold = self.drum_vst_restore_hold.clone();
        let live_sr = self.live_sample_rate();
        std::thread::spawn(move || {
            #[cfg(test)]
            if let Some(hold) = hold {
                while hold.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            if cancel.load(Ordering::Relaxed) {
                let _ = send.send(JobResult::DrumVstReady {
                    generation,
                    saved,
                    result: Err("cancelled".into()),
                });
                return;
            }
            let result = (|| -> Result<
                dawsu_audio_instruments::HostSession<dawsu_audio_instruments::ProcessTransport>,
                String,
            > {
                let host_executable = host_exe.map_err(|e| e.to_string())?;
                let config = dawsu_audio_instruments::Vst3HostConfig {
                    host_executable,
                    shm_name,
                    parent_pid: pid,
                    plugin_path: PathBuf::from(&saved.plugin_path),
                    plugin_uid: saved.plugin_uid.clone(),
                    offline: false,
                    sample_rate_hz: live_sr,
                };
                let mut session = dawsu_audio_instruments::HostSession::spawn(&config)
                    .map_err(|e| e.to_string())?;
                if cancel.load(Ordering::Relaxed) {
                    let _ = session.quit();
                    return Err("cancelled".into());
                }
                if let Some(state_b64) = saved.state_b64.as_deref()
                    && !state_b64.is_empty()
                {
                    let _ = session.set_state(state_b64);
                }
                Ok(session)
            })();
            if cancel.load(Ordering::Relaxed) {
                if let Ok(mut session) = result {
                    let _ = session.quit();
                }
                return;
            }
            let _ = send.send(JobResult::DrumVstReady {
                generation,
                saved,
                result,
            });
        });
    }

    pub(crate) fn apply_drum_vst_job_result(&mut self, result: JobResult) {
        let JobResult::DrumVstReady { generation, saved, result } = result else {
            return;
        };
        if generation != self.drum_vst_generation || self.document.is_none() {
            if let Ok(mut session) = result {
                let _ = session.quit();
            }
            return;
        }
        self.drum_vst_loading = false;
        self.bump_view_revision();
        // An explicit load (`spawn_load_drum_vst`) writes the session once attached and, like
        // `load_drum_vst`, leaves the saved session alone when it fails; a restore keeps it.
        let explicit = std::mem::take(&mut self.async_jobs.drum_vst_write_session_on_attach);
        match result {
            Ok(session) => {
                let plugin_path = PathBuf::from(&saved.plugin_path);
                let summary = self.vst3.insert_loaded_session(
                    &plugin_path,
                    saved.plugin_uid.clone(),
                    PluginRole::Instrument,
                    session,
                );
                let state_b64 = saved
                    .state_b64
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| self.vst3.get_plugin_state(&summary.id).ok());
                match self.attach_drum_vst_instance(
                    &summary,
                    &plugin_path,
                    saved.plugin_uid.clone(),
                    saved.enabled,
                    state_b64,
                ) {
                    Ok(()) => {
                        self.drum_vst_saved = None;
                        self.sync_audio();
                        self.clear_drum_vst_loading_status();
                        if explicit {
                            let _ = self.write_session(None, None, None, None);
                        }
                    }
                    Err(err) => {
                        let _ = self.vst3.unload_plugin(&summary.id);
                        if !explicit {
                            self.drum_vst_saved = Some(saved);
                        }
                        self.set_drum_vst_unavailable_status(&err.to_string());
                    }
                }
            }
            Err(err) => {
                if !explicit {
                    self.drum_vst_saved = Some(saved);
                }
                self.set_drum_vst_unavailable_status(&err);
            }
        }
    }

    /// Loads a VST3 drum instrument into an isolated sidecar and attaches it to live audio playback.
    pub fn load_drum_vst(
        &mut self,
        plugin_path: PathBuf,
        plugin_uid: Option<String>,
    ) -> Result<LoadedPluginSummary, Vst3Error> {
        let sr = self.live_sample_rate();
        let summary = self
            .vst3
            .load_instrument_at_rate(plugin_path.clone(), plugin_uid.clone(), sr)?;
        let state_b64 = self.vst3.get_plugin_state(&summary.id).ok();
        if let Err(err) = self.attach_drum_vst_instance(
            &summary,
            &plugin_path,
            plugin_uid,
            true,
            state_b64,
        ) {
            let _ = self.vst3.unload_plugin(&summary.id);
            return Err(err);
        }
        self.drum_vst_cancel.store(true, Ordering::Relaxed);
        self.drum_vst_job = None;
        self.drum_vst_loading = false;
        self.drum_vst_generation += 1;
        self.drum_vst_saved = None;
        self.sync_audio();
        let _ = self.write_session(None, None, None, None);
        self.bump_view_revision();
        Ok(summary)
    }
    /// Instant ON/OFF toggle for live VST3 drum routing (keeps plugin loaded in memory).
    pub fn set_drum_vst_enabled(&mut self, enabled: bool) {
        if let Some(vst) = &self.drum_vst {
            vst.set_enabled(enabled);
        }
        self.sync_audio();
        let _ = self.write_session(None, None, None, None);
        self.bump_view_revision();
    }

    pub fn drum_vst_search_dirs(&self) -> &[PathBuf] {
        self.vst3.search_dirs()
    }

    pub fn set_drum_vst_search_dirs(&mut self, dirs: Vec<PathBuf>) {
        self.vst3.set_search_dirs(dirs);
        self.bump_view_revision();
    }

    pub fn drum_vst_note_names(&mut self) -> Result<Vec<(u8, String)>, String> {
        let id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or("Load a Drum VST3 instrument first.")?;
        self.vst3.note_names(&id).map_err(|e| e.to_string())
    }

    pub fn open_drum_vst_editor(&mut self) -> Result<(), String> {
        let id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or("Load a Drum VST3 instrument first.")?;
        self.vst3.open_editor(&id).map_err(|e| e.to_string())?;
        self.bump_view_revision();
        Ok(())
    }

    pub fn close_drum_vst_editor(&mut self) -> Result<(), String> {
        let id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or("Load a Drum VST3 instrument first.")?;
        self.vst3.close_editor(&id).map_err(|e| e.to_string())?;
        let _ = self.save_drum_vst_session_state();
        self.bump_view_revision();
        Ok(())
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
            self.bump_view_revision();
        }
    }

    /// Updates lock-free MIDI choke groups (e.g. Hi-Hat Closed/Pedal/Open) on the live drum VST3.
    pub fn set_drum_vst_choke_groups(&self, groups: HashMap<u8, u8>) {
        if let Some(vst) = &self.drum_vst {
            vst.set_choke_groups(groups);
        }
    }

    /// Toggles per-lane VST3 routing on/off; bypassed lanes fall back to sample playback.
    /// In Drums view, applies the toggle uniformly across all merged identical-audio members.
    pub fn toggle_drum_vst_lane(&mut self, row: usize) -> Result<bool, String> {
        let doc = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let primary = doc
            .composer
            .lanes
            .get(row)
            .cloned()
            .ok_or("That lane does not exist.")?;
        let members: Vec<SampleIdentity> = self
            .drum_row_groups
            .as_ref()
            .and_then(|g| g.members_for_row(row))
            .map(|rows| {
                rows.iter()
                    .filter_map(|&r| doc.composer.lanes.get(r).cloned())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| vec![primary.clone()]);
        let vst = self
            .drum_vst
            .as_mut()
            .ok_or("Load a Drum VST3 instrument first.")?;
        let routed = if members.len() > 1 {
            let target_muted = !vst.is_lane_vst_muted(&primary);
            for m in &members {
                vst.set_lane_vst_muted(m, target_muted);
            }
            !target_muted
        } else {
            vst.toggle_lane_vst(&primary)
        };
        self.sync_audio();
        self.bump_view_revision();
        Ok(routed)
    }

    /// Detaches and unloads the active live VST3 drum instrument and clears it from the saved session.
    pub fn unload_drum_vst(&mut self) {
        self.drum_vst_cancel.store(true, Ordering::Relaxed);
        self.drum_vst_job = None;
        self.drum_vst_loading = false;
        self.drum_vst_generation += 1;
        self.drum_vst_saved = None;
        self.clear_live_drum_vst();
        self.sync_audio();
        let _ = self.write_session(None, None, None, Some(None));
        self.bump_view_revision();
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
        let _ = self.write_session(None, None, None, None);
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
        let _ = self.write_session(None, None, None, None);
        Ok(())
    }

    /// Plans a "Render drums to WAVs…" bake over all hits on currently routed VST3 drum lanes,
    /// picking free filenames that do not collide with existing mapset or pending files.
    pub fn plan_drum_vst_render(&self, mode: VelocityExportMode) -> Result<Vst3DrumExportPlan, String> {
        let (plan, _) = self.plan_drum_vst_render_with_sources(mode)?;
        Ok(plan)
    }

    pub(crate) fn plan_drum_vst_render_with_sources(
        &self,
        mode: VelocityExportMode,
    ) -> Result<(Vst3DrumExportPlan, Vec<DrumVstSource>), String> {
        let d = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let vst = self.drum_vst.as_ref().ok_or("Load a Drum VST3 instrument first.")?;
        let routed = vst.routed_notes();
        let mut hits = Vec::new();
        let mut sources = Vec::new();
        for (moment_idx, moment) in d.composer.hitsounds().moments().iter().enumerate() {
            for lane in moment.hits() {
                if let Some(&pitch) = routed.get(lane).filter(|_| !vst.is_lane_vst_muted(lane)) {
                    let orig_vol = moment.hit_volume(lane);
                    let volume = orig_vol.clamp(5, 100) as u8;
                    hits.push(Vst3DrumHit {
                        time_ms: moment.time().ms(),
                        name: lane_name(lane),
                        pitch,
                        volume,
                    });
                    sources.push((moment_idx, lane.clone(), orig_vol));
                }
            }
        }
        let mut plan = plan_vst3_drum_export(&hits, mode);
        if !plan.jobs.is_empty() {
            let root = d.path.parent().ok_or("Map folder unavailable.")?;
            let mut taken = mixes::reserved_mix_names(root, &[], &d.pending)?;
            vst3::assign_free_vst3_filenames(&mut plan, &mut taken)?;
        }
        Ok((plan, sources))
    }

    /// Executes "Render drums to WAVs…" into dawsu!'s render staging folder, registers the WAVs
    /// as pending samples, and switches routed hits onto those WAV lanes in one Undo step.
    pub fn render_drum_vst_to_wavs(
        &mut self,
        mode: VelocityExportMode,
    ) -> Result<(Vst3DrumExportPlan, Vec<BounceOutput>), String> {
        self.render_drum_vst_with_options(mode, bounce_plan::BounceAudioFormat::Wav16, false, true)
    }

    /// Renders routed VST3 drum hits with optional uniform batch peak normalisation, `-60 dBFS`
    /// auto-trim, cooperative cancellation, and Ogg Vorbis or 16-bit WAV output, staging them as
    /// pending mapset samples in one Undo step.
    pub fn render_drum_vst_with_options(
        &mut self,
        mode: VelocityExportMode,
        format: bounce_plan::BounceAudioFormat,
        normalize: bool,
        auto_trim: bool,
    ) -> Result<(Vst3DrumExportPlan, Vec<BounceOutput>), String> {
        self.bounce_cancel.reset();
        let cancel = self.bounce_cancel.clone();
        self.render_drum_vst_to_wavs_formatted(
            mode,
            format,
            normalize,
            move |vst3, instance_id, plan, staging_dir| {
                vst3.execute_vst3_drum_export_with_options(
                    instance_id,
                    plan,
                    staging_dir,
                    44_100,
                    auto_trim,
                    Some(&cancel),
                )
                .map_err(|e| e.to_string())
            },
        )
    }

    /// Renders a non-destructive preview of the planned VST3 drum bounce and returns real measured
    /// duration, sample peak (dBFS), usage count, silence flag, and encoded byte size per sample
    /// without mutating the open difficulty or staging pending samples.
    pub fn preview_drum_vst_render_with_options(
        &mut self,
        mode: VelocityExportMode,
        format: bounce_plan::BounceAudioFormat,
        normalize: bool,
        auto_trim: bool,
    ) -> Result<serde_json::Value, String> {
        self.bounce_cancel.reset();
        let cancel = self.bounce_cancel.clone();
        self.preview_drum_vst_render_with(
            mode,
            format,
            normalize,
            move |vst3, instance_id, plan, staging_dir| {
                vst3.execute_vst3_drum_export_with_options(
                    instance_id,
                    plan,
                    staging_dir,
                    44_100,
                    auto_trim,
                    Some(&cancel),
                )
                .map_err(|e| e.to_string())
            },
        )
    }

    pub(crate) fn preview_drum_vst_render_with<F>(
        &mut self,
        mode: VelocityExportMode,
        format: bounce_plan::BounceAudioFormat,
        normalize: bool,
        render_export: F,
    ) -> Result<serde_json::Value, String>
    where
        F: FnOnce(&mut Vst3Manager, &str, &Vst3DrumExportPlan, &Path) -> Result<Vec<BounceOutput>, String>,
    {
        let (plan, _) = self.plan_drum_vst_render_with_sources(mode)?;
        let mapset_root = self
            .document
            .as_ref()
            .and_then(|d| d.path.parent().map(Path::to_path_buf))
            .ok_or("Map folder unavailable.")?;
        let base_bytes: u64 = std::fs::read_dir(&mapset_root)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .fold(0u64, u64::saturating_add);
        if plan.jobs.is_empty() {
            return Ok(serde_json::json!({
                "baseBytes": base_bytes,
                "bouncedBytes": 0u64,
                "samples": [],
            }));
        }
        let instance_id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or("Load a Drum VST3 instrument first.")?;
        let preview_dir = self
            .data_root
            .join("renders")
            .join(format!("preview-{}", std::process::id()));
        let mut outputs = render_export(&mut self.vst3, &instance_id, &plan, &preview_dir)?;
        let mut plan_out = plan;
        let samples = decode_and_process_drum_outputs(
            &mut outputs,
            &mut plan_out,
            format,
            normalize,
            Some(&preview_dir),
            Some(&self.bounce_cancel),
        )?;
        Ok(build_drum_vst_preview_json(&plan_out, base_bytes, &samples))
    }

    #[cfg(test)]
    pub(crate) fn render_drum_vst_to_wavs_with<F>(
        &mut self,
        mode: VelocityExportMode,
        render_export: F,
    ) -> Result<(Vst3DrumExportPlan, Vec<BounceOutput>), String>
    where
        F: FnOnce(&mut Vst3Manager, &str, &Vst3DrumExportPlan, &Path) -> Result<Vec<BounceOutput>, String>,
    {
        self.render_drum_vst_to_wavs_formatted(
            mode,
            bounce_plan::BounceAudioFormat::Wav16,
            false,
            render_export,
        )
    }
}

#[derive(Debug, Clone)]
pub enum DrumVstBounceOutcome {
    Preview(serde_json::Value),
    Rendered {
        plan: Vst3DrumExportPlan,
        outputs: Vec<BounceOutput>,
    },
}

pub(crate) struct DrumVstProcessedSample {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) clip: Arc<AudioClip>,
}

pub(crate) fn decode_and_process_drum_outputs(
    outputs: &mut [BounceOutput],
    plan: &mut Vst3DrumExportPlan,
    format: bounce_plan::BounceAudioFormat,
    normalize: bool,
    remove_dir: Option<&Path>,
    cancel: Option<&bounce_plan::BounceCancelToken>,
) -> Result<Vec<DrumVstProcessedSample>, String> {
    let mut staged_samples = Vec::with_capacity(outputs.len());
    let read_result = (|| -> Result<(), String> {
        for output in outputs.iter() {
            if cancel.is_some_and(|c| c.is_cancelled()) {
                return Err("Render cancelled.".into());
            }
            let name = output
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| format!("Invalid rendered WAV path: {}", output.path.display()))?
                .to_string();
            let bytes = std::fs::read(&output.path)
                .map_err(|e| format!("Reading rendered WAV {name}: {e}"))?;
            let clip = dawsu_audio::decode(&output.path)?;
            staged_samples.push(DrumVstProcessedSample { name, bytes, clip });
        }
        Ok(())
    })();
    for output in outputs.iter() {
        let _ = std::fs::remove_file(&output.path);
    }
    if let Some(dir) = remove_dir {
        let _ = std::fs::remove_dir_all(dir);
    }
    read_result?;

    let want_ogg = matches!(format, bounce_plan::BounceAudioFormat::OggVorbis);
    if normalize || want_ogg {
        let mut buffers: Vec<Vec<f32>> = staged_samples
            .iter()
            .map(|sample| {
                if sample.clip.channels == 1 {
                    let mut stereo = Vec::with_capacity(sample.clip.samples.len() * 2);
                    for &s in &sample.clip.samples {
                        stereo.push(s);
                        stereo.push(s);
                    }
                    stereo
                } else {
                    sample.clip.samples.clone()
                }
            })
            .collect();
        if normalize {
            let _ = bounce_plan::uniform_batch_normalize(&mut buffers, format.target_peak_dbfs());
        }
        let mut rename_map: HashMap<String, String> = HashMap::new();
        for (idx, (sample, stereo)) in staged_samples.iter_mut().zip(buffers).enumerate() {
            if cancel.is_some_and(|c| c.is_cancelled()) {
                return Err("Render cancelled.".into());
            }
            let sr = sample.clip.sample_rate.max(8_000);
            if want_ogg {
                let ogg_bytes = bounce_plan::encode_vorbis_ogg(&stereo, sr)?;
                let new_name = match sample.name.rsplit_once('.') {
                    Some((stem, _)) => format!("{stem}.ogg"),
                    None => format!("{}.ogg", sample.name),
                };
                rename_map.insert(sample.name.clone(), new_name.clone());
                sample.name = new_name.clone();
                sample.bytes = ogg_bytes;
                if let Some(out) = outputs.get_mut(idx) {
                    out.path = out.path.with_file_name(&new_name);
                }
            } else {
                let data_bytes = (stereo.len() * 2) as u32;
                let mut wav = Vec::with_capacity(44 + data_bytes as usize);
                wav.extend_from_slice(b"RIFF");
                wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
                wav.extend_from_slice(b"WAVEfmt ");
                wav.extend_from_slice(&16u32.to_le_bytes());
                wav.extend_from_slice(&1u16.to_le_bytes());
                wav.extend_from_slice(&2u16.to_le_bytes());
                wav.extend_from_slice(&sr.to_le_bytes());
                wav.extend_from_slice(&(sr * 4).to_le_bytes());
                wav.extend_from_slice(&4u16.to_le_bytes());
                wav.extend_from_slice(&16u16.to_le_bytes());
                wav.extend_from_slice(b"data");
                wav.extend_from_slice(&data_bytes.to_le_bytes());
                for &s in &stereo {
                    let pcm = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
                    wav.extend_from_slice(&pcm.to_le_bytes());
                }
                sample.bytes = wav;
            }
            sample.clip = Arc::new(AudioClip {
                samples: stereo,
                channels: 2,
                sample_rate: sr,
            });
        }
        if !rename_map.is_empty() {
            for job in &mut plan.jobs {
                if let Some(next) = rename_map.get(&job.filename) {
                    job.filename = next.clone();
                }
            }
            for assignment in &mut plan.assignments {
                if let Some(next) = rename_map.get(&assignment.filename) {
                    assignment.filename = next.clone();
                }
            }
        }
    }
    Ok(staged_samples)
}

pub(crate) fn build_drum_vst_preview_json(
    plan: &Vst3DrumExportPlan,
    base_bytes: u64,
    samples: &[DrumVstProcessedSample],
) -> serde_json::Value {
    let mut bounced_bytes = 0u64;
    let mut samples_json = Vec::with_capacity(plan.jobs.len());
    for (idx, sample) in samples.iter().enumerate() {
        let sr = sample.clip.sample_rate.max(8_000);
        let frames = sample.clip.samples.len() / sample.clip.channels.max(1);
        let duration_ms = ((frames as f64 / f64::from(sr)) * 1000.0).round() as u32;
        let peak = sample
            .clip
            .samples
            .iter()
            .copied()
            .filter(|s| s.is_finite())
            .fold(0.0f32, |acc, s| acc.max(s.abs()));
        let silent = peak < bounce_plan::SILENT_PEAK_THRESHOLD;
        let peak_dbfs = if silent { -120.0 } else { 20.0 * peak.log10() };
        let bytes = sample.bytes.len() as u64;
        bounced_bytes = bounced_bytes.saturating_add(bytes);
        let job = plan.jobs.get(idx);
        samples_json.push(serde_json::json!({
            "filename": sample.name,
            "durationMs": duration_ms,
            "peakDbfs": peak_dbfs,
            "usageCount": job.map_or(1, |j| j.hit_count),
            "silent": silent,
            "bytes": bytes,
            "pitch": job.map_or(36, |j| j.pitch),
            "velocity": job.map_or(100, |j| j.midi_velocity),
        }));
    }
    serde_json::json!({
        "baseBytes": base_bytes,
        "bouncedBytes": bounced_bytes,
        "samples": samples_json,
    })
}

impl Application {
    pub(crate) fn finish_rendered_drum_vst_samples(
        &mut self,
        plan: Vst3DrumExportPlan,
        outputs: Vec<BounceOutput>,
        hit_sources: Vec<DrumVstSource>,
        staged_samples: Vec<DrumVstProcessedSample>,
    ) -> Result<(Vst3DrumExportPlan, Vec<BounceOutput>), String> {
        let mut added_names: Vec<String> = Vec::with_capacity(staged_samples.len());
        let mut additions = Vec::with_capacity(staged_samples.len());
        {
            let d = self.document.as_mut().ok_or("Open a difficulty first.")?;
            for sample in staged_samples {
                if let Err(err) = d.pending.add(&sample.name, &sample.bytes) {
                    for added in &added_names {
                        d.pending.release(added);
                    }
                    return Err(err);
                }
                added_names.push(sample.name.clone());
                additions.push((sample.name, sample.clip));
            }
        }
        self.register_samples(additions, false);

        {
            let doc = self.document.as_mut().expect("open document");
            let composer = &mut doc.composer;
            let mut selected = composer.selected.items().iter().cloned().collect::<Vec<_>>();
            composer.history.begin("Render VST3 drums");
            let _ = composer.history.edit(true, |hits| {
                for assignment in &plan.assignments {
                    let (moment_idx, source_lane, orig_vol) = &hit_sources[assignment.hit_index];
                    let moment_id = dawsu_hitsounding::MomentId(*moment_idx);
                    let target_lane = SampleIdentity::File(assignment.filename.clone());
                    if source_lane != &target_lane {
                        hits.remove_hit(moment_id, source_lane);
                        hits.add_hit_with_volume(moment_id, target_lane.clone(), *orig_vol);
                        for sel in &mut selected {
                            if sel.moment == *moment_idx && &sel.lane == source_lane {
                                sel.lane = target_lane.clone();
                            }
                        }
                    }
                }
                Ok::<_, std::convert::Infallible>(())
            });
            composer.history.commit();
            composer.selected.replace(selected);
        }

        self.sync_audio();
        self.status = format!(
            "Staged {} VST3 drum WAV {} and switched {} mapped {} onto lanes (written to mapset on Save).",
            outputs.len(),
            if outputs.len() == 1 { "file" } else { "files" },
            plan.assignments.len(),
            if plan.assignments.len() == 1 { "hit" } else { "hits" },
        );
        self.bump_view_revision();
        Ok((plan, outputs))
    }

    pub(crate) fn render_drum_vst_to_wavs_formatted<F>(
        &mut self,
        mode: VelocityExportMode,
        format: bounce_plan::BounceAudioFormat,
        normalize: bool,
        render_export: F,
    ) -> Result<(Vst3DrumExportPlan, Vec<BounceOutput>), String>
    where
        F: FnOnce(&mut Vst3Manager, &str, &Vst3DrumExportPlan, &Path) -> Result<Vec<BounceOutput>, String>,
    {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before rendering drums.".into());
        }
        let (mut plan, hit_sources) = self.plan_drum_vst_render_with_sources(mode)?;
        if plan.jobs.is_empty() {
            return Err("No hits found on mapped VST3 drum lanes.".into());
        }
        let instance_id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or("Load a Drum VST3 instrument first.")?;
        let mapset_root = self
            .document
            .as_ref()
            .and_then(|d| d.path.parent().map(Path::to_path_buf))
            .ok_or("Map folder unavailable.")?;
        let staging_dir = self
            .data_root
            .join("renders")
            .join(pending::mapset_key(&mapset_root));
        let mut outputs = render_export(&mut self.vst3, &instance_id, &plan, &staging_dir)?;
        let staged_samples =
            decode_and_process_drum_outputs(&mut outputs, &mut plan, format, normalize, None, None)?;
        self.finish_rendered_drum_vst_samples(plan, outputs, hit_sources, staged_samples)
    }
}
