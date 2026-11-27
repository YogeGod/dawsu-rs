//! Non-blocking background job spawners and `Application::poll()` completion handlers
//! for long-running sidecar, bounce, sound-set, and mixer FX operations.

use crate::app_drum_vst::{
    DrumVstBounceOutcome, DrumVstProcessedSample, build_drum_vst_preview_json,
    decode_and_process_drum_outputs,
};
use crate::bounce_plan::BounceAudioFormat;
use crate::keysound_session::{
    StagedKeysoundGenerate, build_keysounds_preview_value, commit_staged_keysounds,
    export_settings_from_payload_pub, mapset_disk_bytes_pub, prepare_and_write_wavs_pub,
    prepare_keysound_render_job, render_manifest_clips_standalone, resolve_apply_batch_norm_pub,
    summary as keysounds_summary_json,
};
use crate::mixer_fx::{FxSlotConfig, MixerFxSession, MixerFxSnapshot};
use crate::sound_sets::{SaveSetError, execute_save_sound_set};
use crate::vst3::{
    BounceOutput, PluginRole, VelocityExportMode, Vst3Blacklist, Vst3DrumExportPlan, Vst3Error,
    Vst3ScanReport, run_offline_drum_export, scan_dirs_with_blacklist,
};
use crate::{Application, DrumVstSource, pending};
use dawsu_audio_instruments::SampleInstrument;
use dawsu_audio_instruments::host::{HostSession, ProcessTransport, Vst3HostConfig};
use dawsu_keysounding::generate::{GeneratePlan, plan_generate_keysounds};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

type DrumVstBounceWorkerOutput = Result<DrumVstBounceWorkerDone, String>;

enum DrumVstBounceWorkerDone {
    Preview {
        generation: u64,
        payload: Value,
    },
    Rendered {
        generation: u64,
        plan: Vst3DrumExportPlan,
        outputs: Vec<BounceOutput>,
        hit_sources: Vec<DrumVstSource>,
        staged_samples: Vec<DrumVstProcessedSample>,
    },
}

struct KeysoundsVst3WorkerOutput {
    generation: u64,
    track_id: u64,
    plugin_path: PathBuf,
    plugin_uid: Option<String>,
    plugin_name: String,
    result: Result<(HostSession<ProcessTransport>, Option<String>), String>,
}

type KeysoundsBounceWorkerOutput = Result<KeysoundsBounceWorkerDone, String>;

enum KeysoundsBounceWorkerDone {
    Preview {
        generation: u64,
        preview: Value,
        newly_loaded_sf: Vec<(PathBuf, Arc<SampleInstrument>)>,
    },
    Generated(Box<StagedKeysoundGenerate>),
}

struct MixerFxAddWorkerOutput {
    generation: u64,
    strip_key: String,
    config: FxSlotConfig,
    result: Result<(HostSession<ProcessTransport>, Option<String>), String>,
}

type RestoreSlotOutcome = (
    FxSlotConfig,
    Result<(HostSession<ProcessTransport>, Option<String>), String>,
);

type Vst3ScanResult = Result<(Vst3ScanReport, Vst3Blacklist, bool), Vst3Error>;

struct MixerFxRestoreWorkerOutput {
    seq: u64,
    generation: u64,
    outcomes: Vec<RestoreSlotOutcome>,
}

#[derive(Default)]
pub(crate) struct AsyncJobsState {
    pub(crate) drum_vst_write_session_on_attach: bool,
    vst3_scan: Option<mpsc::Receiver<Vst3ScanResult>>,
    vst3_scan_result: Option<Result<Vst3ScanReport, Vst3Error>>,
    drum_vst_bounce: Option<mpsc::Receiver<DrumVstBounceWorkerOutput>>,
    drum_vst_bounce_result: Option<Result<DrumVstBounceOutcome, String>>,
    keysounds_vst3_load: Option<mpsc::Receiver<KeysoundsVst3WorkerOutput>>,
    keysounds_vst3_load_result: Option<Result<Value, String>>,
    keysounds_bounce: Option<mpsc::Receiver<KeysoundsBounceWorkerOutput>>,
    keysounds_bounce_result: Option<Result<Value, String>>,
    mixer_fx_add: Option<mpsc::Receiver<MixerFxAddWorkerOutput>>,
    mixer_fx_restore: Option<mpsc::Receiver<MixerFxRestoreWorkerOutput>>,
    mixer_fx_restore_seq: u64,
    mixer_fx_result: Option<Result<MixerFxSnapshot, String>>,
    sound_set_save: Option<mpsc::Receiver<Result<(String, usize), SaveSetError>>>,
    sound_set_save_result: Option<Result<(), SaveSetError>>,
}

impl AsyncJobsState {
    pub(crate) fn is_active(&self) -> bool {
        self.vst3_scan.is_some()
            || self.drum_vst_bounce.is_some()
            || self.keysounds_vst3_load.is_some()
            || self.keysounds_bounce.is_some()
            || self.mixer_fx_add.is_some()
            || self.mixer_fx_restore.is_some()
            || self.sound_set_save.is_some()
    }

    /// Both bounces share `Application::bounce_cancel`: a second one would reset a pending
    /// cancel and replace the first one's receiver, dropping its result.
    fn refuse_second_bounce(&self) -> Result<(), String> {
        if self.drum_vst_bounce.is_some() || self.keysounds_bounce.is_some() {
            return Err("Wait for the current bounce to finish or cancel it first.".into());
        }
        Ok(())
    }
}

fn try_take_channel<T>(slot: &mut Option<mpsc::Receiver<T>>) -> Option<Result<T, ()>> {
    let rx = slot.as_ref()?;
    match rx.try_recv() {
        Ok(val) => {
            *slot = None;
            Some(Ok(val))
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            *slot = None;
            Some(Err(()))
        }
        Err(mpsc::TryRecvError::Empty) => None,
    }
}

impl Application {
    pub fn spawn_vst3_scan(&mut self, retry_blocked: bool) {
        if self.async_jobs.vst3_scan.is_some() {
            return;
        }
        let host_exe_res = self.vst3.host_executable();
        let dirs = self.vst3.search_dirs().to_vec();
        let (blacklist, reset_dirty) = self.vst3.blacklist_snapshot(retry_blocked);
        let (send, recv) = mpsc::channel();
        self.async_jobs.vst3_scan = Some(recv);
        self.async_jobs.vst3_scan_result = None;
        self.bump_view_revision();
        std::thread::spawn(move || {
            let outcome = scan_dirs_with_blacklist(host_exe_res, &dirs, blacklist)
                .map(|(report, bl, scan_dirty)| (report, bl, reset_dirty || scan_dirty));
            let _ = send.send(outcome);
        });
    }

    pub fn take_vst3_scan_result(&mut self) -> Option<Result<Vst3ScanReport, Vst3Error>> {
        self.async_jobs.vst3_scan_result.take()
    }

    pub fn spawn_drum_vst_bounce(
        &mut self,
        mode: VelocityExportMode,
        format: BounceAudioFormat,
        normalize: bool,
        auto_trim: bool,
        preview_only: bool,
    ) -> Result<(), String> {
        self.async_jobs.refuse_second_bounce()?;
        if !preview_only && (self.busy || self.is_stroke_open()) {
            return Err("Finish the current operation before rendering drums.".into());
        }
        let (mut plan, hit_sources) = self.plan_drum_vst_render_with_sources(mode)?;
        let mapset_root = self
            .document
            .as_ref()
            .and_then(|d| d.path.parent().map(Path::to_path_buf))
            .ok_or("Map folder unavailable.")?;
        if plan.jobs.is_empty() {
            if !preview_only {
                return Err("No hits found on mapped VST3 drum lanes.".into());
            }
            let base_bytes = mapset_disk_bytes_pub(&mapset_root);
            self.async_jobs.drum_vst_bounce_result = Some(Ok(DrumVstBounceOutcome::Preview(
                json!({
                    "baseBytes": base_bytes,
                    "bouncedBytes": 0u64,
                    "samples": [],
                }),
            )));
            self.bump_view_revision();
            return Ok(());
        }
        let instance_id = self
            .drum_vst
            .as_ref()
            .map(|v| v.instance_id.clone())
            .ok_or("Load a Drum VST3 instrument first.")?;
        let (plugin_path, plugin_uid) = self
            .vst3
            .instance_info(&instance_id)
            .ok_or_else(|| format!("VST3 plug-in instance '{instance_id}' is not loaded."))?;
        let state_b64 = self.vst3.get_plugin_state(&instance_id).ok();
        let host_exe = self.vst3.host_executable().map_err(|e| e.to_string())?;
        let shm_name = self.vst3.next_shm_name();
        let generation = self.document_generation;
        let base_bytes = if preview_only {
            mapset_disk_bytes_pub(&mapset_root)
        } else {
            0
        };
        let work_dir = if preview_only {
            self.data_root
                .join("renders")
                .join(format!("preview-{}", std::process::id()))
        } else {
            self.data_root
                .join("renders")
                .join(pending::mapset_key(&mapset_root))
        };
        self.bounce_cancel.reset();
        let cancel = self.bounce_cancel.clone();
        let (send, recv) = mpsc::channel();
        self.async_jobs.drum_vst_bounce = Some(recv);
        self.async_jobs.drum_vst_bounce_result = None;
        self.bump_view_revision();

        std::thread::spawn(move || {
            let outcome = (|| -> Result<DrumVstBounceWorkerDone, String> {
                let mut outputs = run_offline_drum_export(
                    host_exe,
                    shm_name,
                    plugin_path,
                    plugin_uid,
                    state_b64,
                    &plan,
                    &work_dir,
                    44_100,
                    auto_trim,
                    Some(&cancel),
                )
                .map_err(|e| e.to_string())?;
                let remove_dir = preview_only.then_some(work_dir.as_path());
                let staged_samples = decode_and_process_drum_outputs(
                    &mut outputs,
                    &mut plan,
                    format,
                    normalize,
                    remove_dir,
                    Some(&cancel),
                )?;
                if cancel.is_cancelled() {
                    return Err("Render cancelled.".into());
                }
                if preview_only {
                    let payload = build_drum_vst_preview_json(&plan, base_bytes, &staged_samples);
                    Ok(DrumVstBounceWorkerDone::Preview { generation, payload })
                } else {
                    Ok(DrumVstBounceWorkerDone::Rendered {
                        generation,
                        plan,
                        outputs,
                        hit_sources,
                        staged_samples,
                    })
                }
            })();
            let _ = send.send(outcome);
        });
        Ok(())
    }

    pub fn take_drum_vst_bounce_result(&mut self) -> Option<Result<DrumVstBounceOutcome, String>> {
        self.async_jobs.drum_vst_bounce_result.take()
    }

    pub fn spawn_keysounds_load_vst3(
        &mut self,
        track_id: u64,
        plugin_path: PathBuf,
        plugin_uid: Option<String>,
        plugin_name: String,
    ) -> Result<(), String> {
        if self.async_jobs.keysounds_vst3_load.is_some() {
            return Err("Wait for the current VST3 instrument to finish loading.".into());
        }
        if !self.keysounds.document.tracks.iter().any(|t| t.id == track_id) {
            return Err("Keysound track not found".into());
        }
        let host_executable = self.vst3.host_executable().map_err(|e| e.to_string())?;
        let shm_name = self.vst3.next_shm_name();
        let sr = self.live_sample_rate();
        let generation = self.document_generation;
        let pid = std::process::id();
        let (send, recv) = mpsc::channel();
        self.async_jobs.keysounds_vst3_load = Some(recv);
        self.async_jobs.keysounds_vst3_load_result = None;
        self.bump_view_revision();

        std::thread::spawn(move || {
            let result = (|| {
                let config = Vst3HostConfig {
                    host_executable,
                    shm_name,
                    parent_pid: pid,
                    plugin_path: plugin_path.clone(),
                    plugin_uid: plugin_uid.clone(),
                    offline: false,
                    sample_rate_hz: if sr > 0 { sr } else { 44_100 },
                };
                let mut session = HostSession::spawn(&config).map_err(|e| e.to_string())?;
                let state = session.get_state().ok();
                Ok((session, state))
            })();
            let _ = send.send(KeysoundsVst3WorkerOutput {
                generation,
                track_id,
                plugin_path,
                plugin_uid,
                plugin_name,
                result,
            });
        });
        Ok(())
    }

    pub fn take_keysounds_vst3_load_result(&mut self) -> Option<Result<Value, String>> {
        self.async_jobs.keysounds_vst3_load_result.take()
    }

    pub fn spawn_keysounds_bounce(
        &mut self,
        payload: Value,
        preview_only: bool,
    ) -> Result<(), String> {
        self.async_jobs.refuse_second_bounce()?;
        if !preview_only {
            if self.busy || self.is_stroke_open() {
                return Err("Finish the current operation before generating keysounds.".into());
            }
            self.poll_sweep(false);
            if self.sweep.is_some() {
                return Err(
                    "Wait for the current unused sample cleanup to finish before generating keysounds."
                        .into(),
                );
            }
        }
        let settings = export_settings_from_payload_pub(&payload);
        let apply_batch_norm = resolve_apply_batch_norm_pub(&payload);
        let generation = self.document_generation;
        let (open_path, file_name, folder, pending_taken, ledger, plan) = {
            let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
            let open_path = document.path.clone();
            let file_name = document
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("map.osu")
                .to_string();
            let folder = document
                .path
                .parent()
                .ok_or("Active beatmap has no parent folder.")?
                .to_path_buf();
            let plan = plan_generate_keysounds(
                &self.keysounds.document,
                &document.file,
                &file_name,
                &settings,
            );
            (
                open_path,
                file_name,
                folder,
                document.pending.taken_names(),
                document.pending.ledger(),
                plan,
            )
        };
        let _ = file_name;
        let out_path = folder.join(&plan.file_name);
        if !preview_only && out_path == open_path {
            return Err("Cannot overwrite the currently open difficulty with [Keysounds].".into());
        }
        let is_ogg = settings.sample_extension.eq_ignore_ascii_case("ogg");
        let base_bytes = if preview_only {
            mapset_disk_bytes_pub(&folder)
        } else {
            0
        };
        let backup_root = self.data_root.join("backups");
        let render_job = prepare_sound_manifest_job_safe(self, &plan)?;
        self.bounce_cancel.reset();
        let cancel = self.bounce_cancel.clone();
        let (send, recv) = mpsc::channel();
        self.async_jobs.keysounds_bounce = Some(recv);
        self.async_jobs.keysounds_bounce_result = None;
        self.bump_view_revision();

        std::thread::spawn(move || {
            let outcome = (|| -> Result<KeysoundsBounceWorkerDone, String> {
                let (raw_clips, newly_loaded_sf) =
                    render_manifest_clips_standalone(&render_job, &cancel)?;
                if cancel.is_cancelled() {
                    return Err("Render cancelled.".into());
                }
                if preview_only {
                    let preview = build_keysounds_preview_value(
                        &plan,
                        raw_clips,
                        base_bytes,
                        apply_batch_norm,
                        is_ogg,
                        &cancel,
                    )?;
                    Ok(KeysoundsBounceWorkerDone::Preview {
                        generation,
                        preview,
                        newly_loaded_sf,
                    })
                } else {
                    let mut plan = plan;
                    let (written_names, guard) = prepare_and_write_wavs_pub(
                        &folder,
                        &backup_root,
                        &mut plan,
                        raw_clips,
                        &pending_taken,
                        ledger.as_ref(),
                        apply_batch_norm,
                    )?;
                    if cancel.is_cancelled() {
                        drop(guard);
                        return Err("Render cancelled.".into());
                    }
                    Ok(KeysoundsBounceWorkerDone::Generated(Box::new(
                        StagedKeysoundGenerate {
                            doc_generation: generation,
                            open_path,
                            out_path,
                            plan,
                            written_names,
                            guard,
                            newly_loaded_sf,
                        },
                    )))
                }
            })();
            let _ = send.send(outcome);
        });
        Ok(())
    }

    pub fn take_keysounds_bounce_result(&mut self) -> Option<Result<Value, String>> {
        self.async_jobs.keysounds_bounce_result.take()
    }

    pub fn spawn_mixer_fx_add_insert(
        &mut self,
        strip: &str,
        plugin_path: String,
        plugin_uid: Option<String>,
        name: Option<String>,
    ) -> Result<(), String> {
        if self.async_jobs.mixer_fx_add.is_some() || self.async_jobs.mixer_fx_restore.is_some() {
            return Err("Wait for the current mixer FX operation to finish.".into());
        }
        if self.mixer_fx.has_test_host() {
            let res = self.mixer_fx_add_insert(strip, plugin_path, plugin_uid, name);
            self.async_jobs.mixer_fx_result = Some(res.clone());
            self.bump_view_revision();
            return res.map(|_| ());
        }
        let (strip_key, config) =
            self.prepare_mixer_fx_add_insert(strip, plugin_path, plugin_uid, name)?;
        let host_exe = self.vst3.host_executable().map_err(|e| e.to_string())?;
        let shm_name = self.vst3.next_shm_name();
        let sr = self.live_sample_rate();
        let generation = self.document_generation;
        let pid = std::process::id();
        let (send, recv) = mpsc::channel();
        self.async_jobs.mixer_fx_add = Some(recv);
        self.async_jobs.mixer_fx_result = None;
        self.bump_view_revision();

        std::thread::spawn(move || {
            let result = crate::mixer_fx::spawn_effect_session(crate::mixer_fx::SpawnEffectParams {
                host_executable: host_exe,
                shm_name,
                parent_pid: pid,
                plugin_path: PathBuf::from(&config.plugin_path),
                plugin_uid: config.plugin_uid.clone(),
                sample_rate_hz: sr,
                active_state: None,
                bypassed: config.bypassed,
            });
            let _ = send.send(MixerFxAddWorkerOutput {
                generation,
                strip_key,
                config,
                result,
            });
        });
        Ok(())
    }

    pub fn spawn_restore_mixer_fx_from_session(&mut self, saved: Option<MixerFxSession>) {
        self.async_jobs.mixer_fx_restore_seq =
            self.async_jobs.mixer_fx_restore_seq.wrapping_add(1);
        self.async_jobs.mixer_fx_restore = None;
        if self.mixer_fx.has_test_host() {
            self.restore_mixer_fx_from_session(saved);
            return;
        }
        let desired_slots = self.begin_mixer_fx_restore_session(saved);
        if desired_slots.is_empty() {
            return;
        }
        let host_exe = match self.vst3.host_executable() {
            Ok(path) => path,
            Err(err) => {
                let reason = err.to_string();
                let outcomes = desired_slots
                    .into_iter()
                    .map(|slot| (slot, Err(reason.clone())))
                    .collect();
                self.finish_spawned_restore_slots(outcomes);
                return;
            }
        };
        let sr = self.live_sample_rate();
        let generation = self.document_generation;
        let seq = self.async_jobs.mixer_fx_restore_seq;
        let pid = std::process::id();
        let specs: Vec<(FxSlotConfig, String)> = desired_slots
            .into_iter()
            .map(|slot| {
                let shm = self.vst3.next_shm_name();
                (slot, shm)
            })
            .collect();
        let (send, recv) = mpsc::channel();
        self.async_jobs.mixer_fx_restore = Some(recv);
        self.bump_view_revision();

        std::thread::spawn(move || {
            let mut outcomes = Vec::with_capacity(specs.len());
            for (slot, shm_name) in specs {
                let active_state = match slot.active_bank {
                    crate::mixer_fx::AbBank::A => slot.state_a_b64.clone(),
                    crate::mixer_fx::AbBank::B => {
                        slot.state_b_b64.clone().or_else(|| slot.state_a_b64.clone())
                    }
                };
                let res = crate::mixer_fx::spawn_effect_session(crate::mixer_fx::SpawnEffectParams {
                    host_executable: host_exe.clone(),
                    shm_name,
                    parent_pid: pid,
                    plugin_path: PathBuf::from(&slot.plugin_path),
                    plugin_uid: slot.plugin_uid.clone(),
                    sample_rate_hz: sr,
                    active_state,
                    bypassed: slot.bypassed,
                });
                outcomes.push((slot, res));
            }
            let _ = send.send(MixerFxRestoreWorkerOutput {
                seq,
                generation,
                outcomes,
            });
        });
    }

    pub fn take_mixer_fx_job_result(&mut self) -> Option<Result<MixerFxSnapshot, String>> {
        self.async_jobs.mixer_fx_result.take()
    }

    pub fn spawn_save_sound_set_from_selection(
        &mut self,
        map: PathBuf,
        name: String,
    ) -> Result<(), SaveSetError> {
        if self.async_jobs.sound_set_save.is_some() {
            return Err(SaveSetError::Refused(
                "Wait for the current sound set save to finish.".into(),
            ));
        }
        let prepared = self.prepare_save_sound_set_from_selection(&map, &name)?;
        let (send, recv) = mpsc::channel();
        self.async_jobs.sound_set_save = Some(recv);
        self.async_jobs.sound_set_save_result = None;
        self.bump_view_revision();
        std::thread::spawn(move || {
            let _ = send.send(execute_save_sound_set(prepared));
        });
        Ok(())
    }

    pub fn take_save_sound_set_result(&mut self) -> Option<Result<(), SaveSetError>> {
        self.async_jobs.sound_set_save_result.take()
    }

    pub(crate) fn poll_async_jobs(&mut self) {
        if let Some(msg) = try_take_channel(&mut self.async_jobs.vst3_scan) {
            self.async_jobs.vst3_scan_result = Some(match msg {
                Ok(Ok((report, blacklist, dirty))) => {
                    self.vst3.apply_scan_result(&report, blacklist, dirty);
                    Ok(report)
                }
                Ok(Err(err)) => Err(err),
                Err(()) => Err(Vst3Error::Bounce("VST3 scan worker stopped unexpectedly.".into())),
            });
            self.bump_view_revision();
        }

        if let Some(msg) = try_take_channel(&mut self.async_jobs.drum_vst_bounce) {
            self.async_jobs.drum_vst_bounce_result = Some(match msg {
                Ok(Ok(DrumVstBounceWorkerDone::Preview { generation, payload })) => {
                    if self.bounce_cancel.is_cancelled() {
                        Err("Render cancelled.".into())
                    } else if generation != self.document_generation || self.document.is_none() {
                        Err("Difficulty changed while previewing Drum VST3.".into())
                    } else {
                        Ok(DrumVstBounceOutcome::Preview(payload))
                    }
                }
                Ok(Ok(DrumVstBounceWorkerDone::Rendered {
                    generation,
                    plan,
                    outputs,
                    hit_sources,
                    staged_samples,
                })) => {
                    if self.bounce_cancel.is_cancelled() {
                        Err("Render cancelled.".into())
                    } else if generation != self.document_generation || self.document.is_none() {
                        Err("Difficulty changed while rendering Drum VST3.".into())
                    } else if self.busy || self.is_stroke_open() {
                        Err("Finish the current operation before committing rendered drums.".into())
                    } else {
                        self.finish_rendered_drum_vst_samples(
                            plan,
                            outputs,
                            hit_sources,
                            staged_samples,
                        )
                        .map(|(plan, outputs)| DrumVstBounceOutcome::Rendered { plan, outputs })
                    }
                }
                Ok(Err(err)) => Err(err),
                Err(()) => Err("Drum VST3 bounce worker stopped unexpectedly.".into()),
            });
            self.bump_view_revision();
        }

        if let Some(msg) = try_take_channel(&mut self.async_jobs.keysounds_vst3_load) {
            self.async_jobs.keysounds_vst3_load_result = Some(match msg {
                Ok(done) => {
                    let stale = done.generation != self.document_generation
                        || !self.keysounds.document.tracks.iter().any(|t| t.id == done.track_id);
                    match done.result {
                        Ok((mut session, _state)) if stale => {
                            let _ = session.quit();
                            Err("Keysound track changed before VST3 finished loading.".into())
                        }
                        Ok((session, state)) => {
                            let before = self.keysounds.document.clone();
                            let loaded = self.vst3.insert_loaded_session(
                                &done.plugin_path,
                                done.plugin_uid.clone(),
                                PluginRole::Instrument,
                                session,
                            );
                            if let Some(prev_inst) = self
                                .keysound_history
                                .track_instances
                                .insert(done.track_id, loaded.id.clone())
                            {
                                let _ = self.vst3.unload_plugin(&prev_inst);
                            }
                            if let Some(track) = self
                                .keysounds
                                .document
                                .tracks
                                .iter_mut()
                                .find(|t| t.id == done.track_id)
                            {
                                track.instrument.name =
                                    format!("{} ({})", done.plugin_name, loaded.id);
                                track.instrument.soundfont_path =
                                    Some(done.plugin_path.to_string_lossy().into_owned());
                                track.instrument.vst3_plugin_uid = done.plugin_uid;
                                track.instrument.vst3_state_b64 = state;
                            }
                            self.keysound_history.active_track_id = Some(done.track_id);
                            if self.keysounds.document != before {
                                self.keysound_history.record(before);
                            } else {
                                self.keysound_history.revision =
                                    self.keysound_history.revision.saturating_add(1);
                            }
                            Ok(keysounds_summary_json(self))
                        }
                        Err(err) => Err(err),
                    }
                }
                Err(()) => Err("Keysounds VST3 load worker stopped unexpectedly.".into()),
            });
            self.bump_view_revision();
        }

        if let Some(msg) = try_take_channel(&mut self.async_jobs.keysounds_bounce) {
            self.async_jobs.keysounds_bounce_result = Some(match msg {
                Ok(Ok(KeysoundsBounceWorkerDone::Preview {
                    generation,
                    preview,
                    newly_loaded_sf,
                })) => {
                    for (path, inst) in newly_loaded_sf {
                        self.keysound_history
                            .soundfont_cache
                            .entry(path)
                            .or_insert(inst);
                    }
                    if self.bounce_cancel.is_cancelled() {
                        Err("Render cancelled.".into())
                    } else if generation != self.document_generation || self.document.is_none() {
                        Err("Difficulty changed while previewing keysounds.".into())
                    } else {
                        let mut out = keysounds_summary_json(self);
                        out["bouncePreview"] = preview;
                        Ok(out)
                    }
                }
                Ok(Ok(KeysoundsBounceWorkerDone::Generated(stage))) => {
                    if self.bounce_cancel.is_cancelled() {
                        drop(stage.guard);
                        Err("Render cancelled.".into())
                    } else {
                        commit_staged_keysounds(self, *stage)
                    }
                }
                Ok(Err(err)) => Err(err),
                Err(()) => Err("Keysounds bounce worker stopped unexpectedly.".into()),
            });
            self.bump_view_revision();
        }

        if let Some(msg) = try_take_channel(&mut self.async_jobs.mixer_fx_add) {
            self.async_jobs.mixer_fx_result = Some(match msg {
                Ok(done) => {
                    if done.generation != self.document_generation || self.document.is_none() {
                        if let Ok((mut session, _)) = done.result {
                            let _ = session.quit();
                        }
                        Err("Difficulty changed while loading mixer FX.".into())
                    } else {
                        self.finish_spawned_add_insert(done.strip_key, done.config, done.result)
                    }
                }
                Err(()) => Err("Mixer FX load worker stopped unexpectedly.".into()),
            });
            self.bump_view_revision();
        }

        if let Some(msg) = try_take_channel(&mut self.async_jobs.mixer_fx_restore) {
            if let Ok(done) = msg {
                if done.seq != self.async_jobs.mixer_fx_restore_seq
                    || done.generation != self.document_generation
                    || self.document.is_none()
                {
                    for (_, res) in done.outcomes {
                        if let Ok((mut session, _)) = res {
                            let _ = session.quit();
                        }
                    }
                } else {
                    self.finish_spawned_restore_slots(done.outcomes);
                }
            }
            self.bump_view_revision();
        }

        if let Some(msg) = try_take_channel(&mut self.async_jobs.sound_set_save) {
            self.async_jobs.sound_set_save_result = Some(match msg {
                Ok(Ok((saved_name, silent))) => {
                    self.apply_save_sound_set_success(&saved_name, silent);
                    Ok(())
                }
                Ok(Err(err)) => Err(err),
                Err(()) => Err(SaveSetError::Refused(
                    "Sound set save stopped unexpectedly.".into(),
                )),
            });
            self.bump_view_revision();
        }
    }
}

fn prepare_sound_manifest_job_safe(
    app: &mut Application,
    plan: &GeneratePlan,
) -> Result<crate::keysound_session::PreparedKeysoundRenderJob, String> {
    prepare_keysound_render_job(app, &plan.manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorSchemePreference, Preferences};
    use dawsu_editor_composer::VolumeScope;
    use dawsu_foundation::SampleIdentity;
    use dawsu_test_support::Scratch;
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::{Duration, Instant};

    const MINIMAL_MAP: &str = "\
osu file format v14

[General]
Mode: 0

[Metadata]
Title: Async Test
Artist: Tester
Creator: Mapper
Version: Normal

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,0,1,0,0:0:0:0:
256,192,500,1,0,0:0:0:0:
";

    fn poll_until(app: &mut Application, done: impl Fn(&mut Application) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            app.poll();
            if done(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out polling async job");
    }

    #[test]
    fn legacy_v13_preferences_deserialize_defaults_and_new_setters_roundtrip() {
        let legacy_json = r#"{
            "recent": [],
            "songs_folder": "",
            "default_samples": "",
            "device": null,
            "song_gain": 0.7,
            "song_muted": false,
            "master_gain": 0.2,
            "audition": true,
            "follow": false,
            "return_on_stop": true,
            "show_browser": true,
            "show_inspector": true,
            "show_controller": true,
            "used_only": false,
            "lane_names": true,
            "object_lines": false,
            "classic_browser": false,
            "theme": 0,
            "key_preset": "DAW",
            "key_overrides": {},
            "protect_hitnormals": false,
            "clean_redundant_greenlines": false
        }"#;
        let prefs: Preferences = serde_json::from_str(legacy_json).expect("legacy prefs");
        assert_eq!(prefs.ui_scale_pct, 100);
        assert_eq!(prefs.color_scheme, None);
        assert_eq!(prefs.midi_input_port, None);
        assert!(!prefs.setup_checklist_dismissed);
        assert!(!prefs.practice_tried);
        assert_eq!(prefs.preview_skin, None);
        assert_eq!(prefs.autosave_interval_secs, 30, "a missing field is the 30 s default, not Off");

        let mut app = Application::headless();
        let mut custom = BTreeMap::new();
        custom.insert("accent".to_string(), [210.0, 80.0, 90.0]);
        let scheme = Some(ColorSchemePreference {
            preset: Some("midnight".into()),
            custom_hsb: custom,
        });
        app.set_ui_scale_pct(250);
        assert_eq!(app.preferences().ui_scale_pct, 200);
        app.set_ui_scale_pct(50);
        assert_eq!(app.preferences().ui_scale_pct, 90);
        app.set_color_scheme(scheme.clone());
        app.set_midi_input_port(Some("Launchpad".into()));
        app.set_setup_checklist_dismissed(true);
        app.set_practice_tried(true);
        app.set_preview_skin(Some("classic".into()));
        app.set_autosave_interval_secs(999);
        assert_eq!(app.preferences().autosave_interval_secs, 120);
        app.set_autosave_interval_secs(5);
        assert_eq!(app.preferences().autosave_interval_secs, 15);

        let roundtrip: Preferences =
            serde_json::from_str(&serde_json::to_string(app.preferences()).unwrap()).unwrap();
        assert_eq!(roundtrip.ui_scale_pct, 90);
        assert_eq!(roundtrip.color_scheme, scheme);
        assert_eq!(roundtrip.midi_input_port.as_deref(), Some("Launchpad"));
        assert!(roundtrip.setup_checklist_dismissed);
        assert!(roundtrip.practice_tried);
        assert_eq!(roundtrip.preview_skin.as_deref(), Some("classic"));
        assert_eq!(roundtrip.autosave_interval_secs, 15);
    }

    #[test]
    fn view_state_token_bumps_when_persist_preferences_is_false_across_audited_mutators() {
        let scratch = Scratch::new("slice0-view-token");
        let map = scratch.path("Normal.osu");
        std::fs::write(&map, MINIMAL_MAP).unwrap();

        let mut app = Application::headless();
        app.open_now(crate::OpenSource::Map(map.clone()));
        poll_until(&mut app, |a| a.document().is_some() && !a.is_busy());

        macro_rules! assert_token_bumps {
            ($label:literal, $op:expr) => {{
                let before = app.view_state_token();
                $op;
                let after = app.view_state_token();
                assert_ne!(before, after, "ViewStateToken did not bump after {}", $label);
            }};
        }

        assert_token_bumps!("save_preferences(!persist)", app.save_preferences());
        assert_token_bumps!("set_step_pads_mode", app.set_step_pads_mode(true));
        assert_token_bumps!("set_normal_protection", app.set_normal_protection(true));
        assert_token_bumps!(
            "set_reload_osu_after_save",
            app.set_reload_osu_after_save(true)
        );
        assert_token_bumps!("set_auto_check_updates", app.set_auto_check_updates(false));
        assert_token_bumps!("set_ui_scale_pct", app.set_ui_scale_pct(125));
        assert_token_bumps!(
            "set_setup_checklist_dismissed",
            app.set_setup_checklist_dismissed(true)
        );
        assert_token_bumps!("set_practice_tried", app.set_practice_tried(true));
        assert_token_bumps!("set_autosave_interval_secs", app.set_autosave_interval_secs(15));
        assert_token_bumps!(
            "set_preview_skin",
            app.set_preview_skin(Some("default".into()))
        );
        assert_token_bumps!(
            "set_selection_lane_scope",
            app.set_selection_lane_scope(Some(BTreeSet::from([SampleIdentity::File(
                "kick.wav".into()
            )])))
            .unwrap()
        );
        assert_token_bumps!(
            "set_export_settings",
            app.set_export_settings(crate::ExportSettings::default())
                .unwrap()
        );
        assert_token_bumps!(
            "set_drum_options",
            app.set_drum_options(crate::DrumOptions::default()).unwrap()
        );
        assert_token_bumps!(
            "keysound_history.mark_saved",
            app.keysound_history
                .mark_saved(scratch.path("p.dawsuks"), vec![1, 2])
        );
        let t1 = app.keysounds_add_track("Lead", None);
        let t2 = app.keysounds_add_track("Pad", None);
        assert_ne!(t1, t2);
        assert_token_bumps!(
            "keysounds_action select_track",
            crate::keysound_session::handle(
                &mut app,
                &json!({ "kind": "select_track", "trackId": t2 })
            )
            .unwrap()
        );

        app.set_song_loading_for_test(true);
        assert_token_bumps!("play_pause_from(song_loading)", app.play_pause_from(Some(10.0)));
        assert_token_bumps!("play_from(song_loading)", app.play_from(20.0));
        app.set_song_loading_for_test(false);

        assert_token_bumps!("close_document", app.close_document());
        let _ = VolumeScope::All;
    }

    #[test]
    fn a_failed_explicit_drum_vst_load_keeps_the_saved_session_as_the_sync_load_does() {
        let scratch = Scratch::new("slice0-drum-vst-load-fail");
        let map = scratch.path("Normal.osu");
        std::fs::write(&map, MINIMAL_MAP).unwrap();
        let mut app = Application::headless_with_projects(scratch.path("data"));
        app.open_now(crate::OpenSource::Map(map));
        poll_until(&mut app, |a| a.document().is_some() && !a.is_busy());
        assert!(app.drum_vst_saved.is_none());

        app.spawn_load_drum_vst(scratch.path("missing.vst3"), None);
        assert!(app.has_active_background_jobs());
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        assert!(app.drum_vst().is_none());
        assert!(
            app.drum_vst_saved.is_none(),
            "a plug-in that never loaded must not become the session's drum VST3"
        );
        assert!(!app.async_jobs.drum_vst_write_session_on_attach);
    }

    #[test]
    fn is_listening_and_has_active_background_jobs_report_accurately() {
        let mut app = Application::headless();
        assert!(!app.is_listening());
        assert!(!app.has_active_background_jobs());

        app.set_busy_for_test(true);
        assert!(app.has_active_background_jobs());
        app.set_busy_for_test(false);
        assert!(!app.has_active_background_jobs());

        app.set_song_loading_for_test(true);
        assert!(app.has_active_background_jobs());
        app.set_song_loading_for_test(false);
        assert!(!app.has_active_background_jobs());

        app.drum.sink = Some(vec![true]);
        assert!(app.is_listening());
        app.drum.sink = Some(vec![false]);
        assert!(!app.is_listening());
    }

    #[test]
    fn nonblocking_vst3_scan_keysounds_bounce_cancel_save_set_and_close() {
        let scratch = Scratch::new("slice0-async-jobs");
        let empty_vst_dir = scratch.path("vst3-empty");
        std::fs::create_dir_all(&empty_vst_dir).unwrap();
        let lib_dir = scratch.path("library");
        std::fs::create_dir_all(&lib_dir).unwrap();
        let sample_wav = scratch.path("kick.wav");
        let clip = dawsu_audio::AudioClip {
            samples: vec![0.25, 0.25, -0.25, -0.25],
            channels: 2,
            sample_rate: 44_100,
        };
        let wav_bytes = {
            let mut v = Vec::new();
            let data_bytes = (clip.samples.len() * 2) as u32;
            v.extend_from_slice(b"RIFF");
            v.extend_from_slice(&(36 + data_bytes).to_le_bytes());
            v.extend_from_slice(b"WAVEfmt ");
            v.extend_from_slice(&16u32.to_le_bytes());
            v.extend_from_slice(&1u16.to_le_bytes());
            v.extend_from_slice(&2u16.to_le_bytes());
            v.extend_from_slice(&44_100u32.to_le_bytes());
            v.extend_from_slice(&(44_100u32 * 4).to_le_bytes());
            v.extend_from_slice(&4u16.to_le_bytes());
            v.extend_from_slice(&16u16.to_le_bytes());
            v.extend_from_slice(b"data");
            v.extend_from_slice(&data_bytes.to_le_bytes());
            for s in clip.samples {
                v.extend_from_slice(&((s * 32767.0).round() as i16).to_le_bytes());
            }
            v
        };
        std::fs::write(&sample_wav, &wav_bytes).unwrap();
        let map = scratch.path("Normal.osu");
        std::fs::write(
            &map,
            "\
osu file format v14

[General]
Mode: 0

[Metadata]
Title: Async Test
Artist: Tester
Creator: Mapper
Version: Normal

[TimingPoints]
0,500,4,1,0,100,1,0

[HitObjects]
256,192,0,1,0,0:0:0:0:kick.wav
",
        )
        .unwrap();

        let mut app = Application::headless_with_projects(scratch.path("data"));
        app.open_now(crate::OpenSource::Map(map.clone()));
        poll_until(&mut app, |a| a.document().is_some() && !a.is_busy());
        app.reset_fresh_open_for_tests();
        let map_canonical = app.document().unwrap().path.clone();

        // 1. Non-blocking VST3 scan
        app.set_drum_vst_search_dirs(vec![empty_vst_dir]);
        app.spawn_vst3_scan(false);
        assert!(app.has_active_background_jobs());
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        let scan = app.take_vst3_scan_result().expect("scan result").expect("ok");
        assert!(scan.plugins.is_empty());

        // 2. Non-blocking keysounds preview & immediate cancellation
        let tid = app.keysounds_add_track("Piano", None);
        crate::keysound_session::handle(
            &mut app,
            &json!({
                "kind": "add_note",
                "trackId": tid,
                "startMs": 0.0,
                "endMs": 200.0,
                "pitch": 60,
                "velocity": 100
            }),
        )
        .unwrap();
        app.spawn_keysounds_bounce(json!({}), true).unwrap();
        assert!(app.has_active_background_jobs());
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        let preview = app
            .take_keysounds_bounce_result()
            .expect("preview result")
            .expect("preview ok");
        assert!(preview.get("bouncePreview").is_some());

        // A second bounce while one runs is refused: both share `bounce_cancel`, so starting
        // another would un-cancel the first and drop its result.
        app.spawn_keysounds_bounce(json!({}), true).unwrap();
        let second = app.spawn_keysounds_bounce(json!({}), true);
        assert!(second.is_err(), "a second bounce must be refused while one runs");
        app.bounce_cancel.cancel();
        assert!(app.spawn_keysounds_bounce(json!({}), true).is_err());
        assert!(app.bounce_cancel.is_cancelled(), "a refused spawn leaves the cancel standing");
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        let _ = app.take_keysounds_bounce_result().expect("first bounce result");

        app.spawn_keysounds_bounce(json!({}), false).unwrap();
        app.bounce_cancel.cancel();
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        let cancelled = app
            .take_keysounds_bounce_result()
            .expect("cancelled bounce")
            .unwrap_err();
        assert!(cancelled.contains("cancelled"));

        // 3. Non-blocking sound set save
        app.update_preferences(|p| p.hitsound_library = Some(lib_dir.clone()));
        let rows: Vec<usize> = (0..app.document().unwrap().composer.lanes.len()).collect();
        app.select_box(-1.0, 1000.0, &rows, false, None);
        app.spawn_save_sound_set_from_selection(map_canonical.clone(), "KickSet".into())
            .unwrap();
        assert!(app.has_active_background_jobs());
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        app.take_save_sound_set_result()
            .expect("sound set save result")
            .expect("saved");
        assert!(app.sound_set_library().unwrap().sets.iter().any(|s| s.name == "KickSet"));

        // 4. Non-blocking close_document_nonblocking
        app.close_document_nonblocking();
        assert!(app.document().is_none());
        poll_until(&mut app, |a| !a.has_active_background_jobs());
    }

    #[test]
    fn open_restores_persisted_mixer_fx_nonblockingly_and_reports_unavailable_without_hanging() {
        let scratch = Scratch::new("slice0-mixer-fx-open-restore");
        let projects = scratch.path("projects");
        let map = scratch.path("Normal.osu");
        std::fs::write(&map, MINIMAL_MAP).unwrap();

        let missing_vst = scratch.path("missing-reverb.vst3");
        let mut proj = crate::project::Project::load(&projects, None, &map).unwrap();
        let mut settings = proj.settings.clone();
        let mut session = MixerFxSession::default();
        session.strips.insert(
            "master".into(),
            vec![FxSlotConfig {
                slot_id: "fx-1".into(),
                plugin_path: missing_vst.to_string_lossy().into_owned(),
                plugin_uid: None,
                name: "Missing Reverb".into(),
                bypassed: false,
                wet_dry: 1.0,
                active_bank: crate::mixer_fx::AbBank::A,
                state_a_b64: None,
                state_b_b64: None,
                pinned: false,
            }],
        );
        settings.mixer_fx = Some(session);
        proj.save(settings).unwrap();

        let mut app = Application::headless_with_projects(scratch.path("data"));
        app.project_root = Some(projects.clone());
        app.vst3 = crate::vst3::Vst3Manager::with_search_dirs(Vec::new())
            .with_host_executable(scratch.path("missing-dawsu-vst3-host.exe"));

        app.open_now(crate::OpenSource::Map(map.clone()));
        poll_until(&mut app, |a| a.document().is_some() && !a.is_busy());
        assert!(app.document().is_some());
        poll_until(&mut app, |a| !a.has_active_background_jobs());

        let snap = app.mixer_fx_snapshot();
        let slots = snap.strips.get("master").expect("master slot restored");
        assert_eq!(slots.len(), 1);
        assert!(!slots[0].available);
        assert!(
            slots[0]
                .unavailable_reason
                .as_deref()
                .is_some_and(|r| !r.is_empty() && r != "Not initialized"),
            "unexpected reason: {:?}",
            slots[0].unavailable_reason
        );
        app.close_document_nonblocking();
        poll_until(&mut app, |a| !a.has_active_background_jobs());

        // Also test async worker restore path when `host_executable` exists on disk.
        let dummy_host = scratch.path("dummy-restore-host.exe");
        std::fs::write(&dummy_host, b"").unwrap();
        let mut async_app = Application::headless_with_projects(scratch.path("data2"));
        async_app.project_root = Some(projects);
        async_app.vst3 = crate::vst3::Vst3Manager::with_search_dirs(Vec::new())
            .with_host_executable(dummy_host);
        async_app.open_now(crate::OpenSource::Map(map));
        poll_until(&mut async_app, |a| a.document().is_some() && !a.is_busy());
        poll_until(&mut async_app, |a| !a.has_active_background_jobs());
        let snap2 = async_app.mixer_fx_snapshot();
        let slots2 = snap2.strips.get("master").expect("worker restore slot");
        assert_eq!(slots2.len(), 1);
        assert!(!slots2[0].available);
        assert!(
            slots2[0]
                .unavailable_reason
                .as_deref()
                .is_some_and(|r| !r.is_empty() && r != "Not initialized"),
            "unexpected worker reason: {:?}",
            slots2[0].unavailable_reason
        );
    }

    #[test]
    fn async_mixer_fx_add_insert_single_flight_and_missing_plugin_error() {
        let scratch = Scratch::new("slice0-mixer-fx-add-async");
        let map = scratch.path("Normal.osu");
        std::fs::write(&map, MINIMAL_MAP).unwrap();

        let dummy_host = scratch.path("dummy-vst3-host.exe");
        std::fs::write(&dummy_host, b"").unwrap();
        let mut app = Application::headless_with_projects(scratch.path("data"));
        app.vst3 = crate::vst3::Vst3Manager::with_search_dirs(Vec::new())
            .with_host_executable(dummy_host);
        app.open_now(crate::OpenSource::Map(map));
        poll_until(&mut app, |a| a.document().is_some() && !a.is_busy());

        let missing = scratch.path("missing-eq.vst3").to_string_lossy().into_owned();
        app.spawn_mixer_fx_add_insert("master", missing.clone(), None, Some("EQ".into()))
            .unwrap();
        assert!(app.has_active_background_jobs());
        assert!(
            app.spawn_mixer_fx_add_insert("master", missing, None, Some("EQ2".into()))
                .is_err(),
            "concurrent mixer FX add must be refused while one is in flight"
        );
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        let err = app
            .take_mixer_fx_job_result()
            .expect("mixer FX add outcome")
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn task04_guards_sweep_keysounds_vst3_drum_bounce_scan_saveset_and_keymap() {
        let scratch = Scratch::new("slice0-task04-guards");
        let map = scratch.path("Normal.osu");
        std::fs::write(&map, MINIMAL_MAP).unwrap();

        let dummy_host = scratch.path("dummy-vst3-host.exe");
        std::fs::write(&dummy_host, b"").unwrap();
        let mut app = Application::headless_with_projects(scratch.path("data"));
        app.vst3 = crate::vst3::Vst3Manager::with_search_dirs(Vec::new())
            .with_host_executable(dummy_host);
        app.open_now(crate::OpenSource::Map(map.clone()));
        poll_until(&mut app, |a| a.document().is_some() && !a.is_busy());

        // 1. keymap_mut bumps ViewStateToken
        let before = app.view_state_token();
        let _ = app.keymap_mut();
        assert_ne!(before, app.view_state_token());

        // 2. Non-preview spawn_keysounds_bounce refuses active sweep immediately without blocking
        //    and leaves a standing bounce_cancel untouched.
        let (sweep_tx, sweep_rx) = mpsc::channel();
        app.sweep = Some(sweep_rx);
        app.bounce_cancel.cancel();
        let t0 = Instant::now();
        let refused = app.spawn_keysounds_bounce(json!({}), false);
        assert!(refused.is_err(), "must refuse while sweep is running");
        assert!(
            t0.elapsed() < Duration::from_secs(1),
            "must not block waiting on sweep"
        );
        assert!(app.bounce_cancel.is_cancelled());
        drop(sweep_tx);
        app.sweep = None;
        app.bounce_cancel.reset();

        // 3. spawn_drum_vst_bounce refuses missing drum instrument and concurrent keysounds bounce
        assert!(
            app.spawn_drum_vst_bounce(
                VelocityExportMode::ExactPerVolume,
                BounceAudioFormat::Wav16,
                true,
                true,
                true
            )
            .is_err()
        );
        let (bounce_tx, bounce_rx) = mpsc::channel();
        app.async_jobs.keysounds_bounce = Some(bounce_rx);
        assert!(
            app.spawn_drum_vst_bounce(
                VelocityExportMode::ExactPerVolume,
                BounceAudioFormat::Wav16,
                true,
                true,
                true
            )
            .is_err()
        );
        drop(bounce_tx);
        app.async_jobs.keysounds_bounce = None;

        // 4. spawn_keysounds_load_vst3 validates track, blocks concurrent loads, and settles error
        assert!(
            app.spawn_keysounds_load_vst3(
                999_999,
                scratch.path("missing.vst3"),
                None,
                "Missing".into()
            )
            .is_err()
        );
        let tid = app.keysounds_add_track("Lead", None);
        app.spawn_keysounds_load_vst3(tid, scratch.path("missing.vst3"), None, "Missing".into())
            .unwrap();
        assert!(
            app.spawn_keysounds_load_vst3(
                tid,
                scratch.path("missing2.vst3"),
                None,
                "Missing2".into()
            )
            .is_err()
        );
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        let err = app
            .take_keysounds_vst3_load_result()
            .expect("keysounds vst3 load result")
            .unwrap_err();
        assert!(!err.is_empty());

        // 5. spawn_vst3_scan skips when a scan is already active
        let (scan_tx, scan_rx) = mpsc::channel();
        app.async_jobs.vst3_scan = Some(scan_rx);
        app.spawn_vst3_scan(false);
        assert!(
            scan_tx
                .send(Ok((
                    Vst3ScanReport {
                        discovered_paths: Vec::new(),
                        plugins: Vec::new(),
                        skipped: Vec::new(),
                        blocked: Vec::new(),
                        sidecar_available: false,
                    },
                    Vst3Blacklist::default(),
                    false
                )))
                .is_ok(),
            "existing scan channel must remain intact when duplicate spawn_vst3_scan is ignored"
        );
        poll_until(&mut app, |a| !a.has_active_background_jobs());
        assert!(app.take_vst3_scan_result().is_some());

        // 6. spawn_save_sound_set_from_selection refuses when already saving
        let (ss_tx, ss_rx) = mpsc::channel();
        app.async_jobs.sound_set_save = Some(ss_rx);
        let save_err = app
            .spawn_save_sound_set_from_selection(map, "DupSet".into())
            .unwrap_err();
        assert!(matches!(save_err, SaveSetError::Refused(_)));
        drop(ss_tx);
        app.async_jobs.sound_set_save = None;
    }
}
