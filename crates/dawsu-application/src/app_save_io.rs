use crate::document::{PrepareSaveInput, last_written, prepare_save};
use crate::{
    Application, DEFAULT_SONG_OFFSET_MS, Document, ExportSettings, JobProgress, JobResult, Loaded,
    MIN_ZOOM, ManiaImportReport, MidiImportOptions, MidiImportOutcome, OpenSource, OutputProblem,
    PendingSave, RecoveryOffer, ReloadPreserve, SampleHealthReport, SnapshotPreview, gain_recovery,
    generate_receipt, keysound_session, lane_name, midi_import, midi_record, mixes,
    original_snapshot, output, pending, project, recovery, refusals, roll_back, safe_child,
    sample_checks, sample_path, sample_watch, sound_sets, sweep, unwritten, vst3,
};
use dawsu_audio::{AudioClip, AudioEngine};
use dawsu_editor::transport::BookmarkList;
use dawsu_editor_composer::Composer;
use dawsu_foundation::{SampleIdentity, SongTime};
use dawsu_hitsounding::Placement;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

impl Application {
    /// Writes the last completed state, even if the live editor was interrupted mid-edit. Once
    /// it is on disk, this session's timed copy is removed, so one card is left.
    pub fn write_recovery_copy(&mut self) -> Result<Option<RecoveryOffer>, String> {
        let result = self
            .checkpoint
            .as_ref()
            .map(|copy| copy.write(&self.data_root))
            .transpose();
        if let Ok(Some(_)) = &result {
            self.drop_autosave();
        }
        result
    }

    /// The timed recovery copy, from every poll: takes a finished write, drops the copy once Undo
    /// returns to the saved state, and starts a write of a changed dirty difficulty when due.
    #[doc(hidden)]
    pub fn autosave_recovery(&mut self, now: std::time::Instant) {
        self.finish_autosave(false);
        if !self.has_dirty_beatmap() {
            if self.autosave_id.is_some() || self.autosave_job.is_some() {
                self.drop_autosave();
            }
            return;
        }
        if self.autosave_job.is_none()
            && !self.busy
            && !self.is_stroke_open()
            && recovery::autosave_due(self.autosave_last, now, self.preferences.autosave_interval_secs)
        {
            self.start_autosave(now);
        }
    }

    /// Writes the timed recovery copy now, whatever the interval, and waits for it. For tests and
    /// the relaunch smoke check.
    #[doc(hidden)]
    pub fn autosave_recovery_now(&mut self) {
        self.finish_autosave(true);
        if !self.is_stroke_open() {
            self.refresh_recovery_edit();
        }
        if self.has_dirty_beatmap() && !self.busy && !self.is_stroke_open() {
            self.start_autosave(std::time::Instant::now());
            self.finish_autosave(true);
        }
    }

    fn start_autosave(&mut self, now: std::time::Instant) {
        let Some(checkpoint) = &self.checkpoint else { return };
        let key = checkpoint.autosave_key();
        if self.autosaved_key == Some(key) {
            return;
        }
        // A failed write is tried again at the next interval, not on the next poll.
        self.autosave_last = Some(now);
        let snapshot = checkpoint.clone();
        let root = self.data_root.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        #[cfg(test)]
        let hold = self.autosave_write_hold.clone();
        let (send, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let result = snapshot.write_with(&root, || {
                if worker_cancel.load(Ordering::Relaxed) {
                    return Err("The recovery copy was cancelled.".into());
                }
                // Tests hold the writer here, past the cancel check, to race it against a drop.
                #[cfg(test)]
                while hold.as_ref().is_some_and(|hold| hold.load(Ordering::Relaxed))
                    && !worker_cancel.load(Ordering::Relaxed)
                {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Ok(())
            });
            let _ = send.send(result);
        });
        self.autosave_job = Some(recovery::AutosaveJob {
            cancel,
            rx,
            handle: Some(handle),
            generation: self.autosave_generation,
            key,
        });
    }

    /// Takes the writer's answer: a new copy replaces the one before it, which is removed only
    /// now, so a valid copy is on disk at every moment.
    pub(crate) fn finish_autosave(&mut self, wait: bool) {
        let Some(job) = &self.autosave_job else { return };
        let result = if wait {
            job.rx.recv().ok()
        } else {
            match job.rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => None,
            }
        };
        let Some(mut job) = self.autosave_job.take() else { return };
        if let Some(handle) = job.handle.take() {
            let _ = handle.join();
        }
        if job.generation != self.autosave_generation {
            if let Some(Ok(offer)) = result {
                recovery::remove_copy(&self.data_root, &offer.id);
            }
            return;
        }
        match result {
            Some(Ok(offer)) => {
                if let Some(previous) = self.autosave_id.replace(offer.id)
                    && self.restored_recovery.as_ref() != Some(&previous)
                {
                    recovery::remove_copy(&self.data_root, &previous);
                }
                self.autosaved_key = Some(job.key);
                self.autosave_error_logged = false;
            }
            failed => {
                if !std::mem::replace(&mut self.autosave_error_logged, true) {
                    let error = match failed {
                        Some(Err(error)) => error,
                        _ => "the writer stopped.".into(),
                    };
                    self.status = format!("Could not write the recovery copy: {error}");
                    self.bump_view_revision();
                }
            }
        }
    }

    /// Cancels and waits for the timed writer, then removes this session's timed copy, also one
    /// the writer just made. A copy the mapper restored stays. Runs before sample cleanup, which
    /// keeps what a recovery copy names.
    pub(crate) fn drop_autosave(&mut self) {
        if let Some(mut job) = self.autosave_job.take() {
            job.cancel.store(true, Ordering::Relaxed);
            if let Some(handle) = job.handle.take() {
                let _ = handle.join();
            }
            while let Ok(result) = job.rx.try_recv() {
                if let Ok(offer) = result {
                    recovery::remove_copy(&self.data_root, &offer.id);
                }
            }
        }
        if let Some(id) = self.autosave_id.take()
            && self.restored_recovery.as_ref() != Some(&id)
        {
            recovery::remove_copy(&self.data_root, &id);
        }
        self.autosaved_key = None;
        self.autosave_last = None;
        self.autosave_generation += 1;
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
    /// Moves a stopped session's copy to `recovery/archive/`, so its card leaves the start screen.
    pub fn discard_recovery(&mut self, id: &str) -> Result<(), String> {
        if self.busy || self.document.is_some() {
            return Err("Discard a recovery copy from the start screen.".into());
        }
        recovery::archive(&self.data_root, id)?;
        self.bump_view_revision();
        Ok(())
    }
    pub(crate) fn refresh_recovery_edit(&mut self) {
        if let (Some(copy), Some(document)) = (&mut self.checkpoint, &self.document) {
            copy.refresh(document);
        }
        self.record_shared_base_overrides_if_needed();
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
        self.drum_stem_cancel.store(true, Ordering::Relaxed);
        self.drum_stem_job = None;
        self.drum_stem_loading = false;
        self.drum_stem_auto_monitor = false;
        self.drum_stem_generation += 1;
        self.drum_stem_path = None;
        self.drum_stem = None;
        self.drum_stem_waveform = None;
        self.monitor_stem = false;
        self.drum_vst_cancel.store(true, Ordering::Relaxed);
        self.drum_vst_job = None;
        self.drum_vst_loading = false;
        self.drum_vst_generation += 1;
        self.drum_vst_saved = None;
        self.clear_live_drum_vst();
        self.unload_all_mixer_fx();
        self.waveform.clear();
        self.waveform_revision += 1;
        // The timed copy goes before cleanup reads the recovery copies: what it names is kept.
        self.drop_autosave();
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
        self.base_layer_track = None;
        self.checkpoint = None;
        self.osu_watch.clear();
        self.external_osu_changed = false;
        self.tap_recording_armed = false;
        self.tap_recorded_hits.clear();
        self.selection_volume_scope = crate::SelectionVolumeScope::SelectedHits;
        keysound_session::reset_for_map(self, None);
        self.bump_view_revision();
    }

    /// Closes the difficulty without waiting synchronously for the unused-sample cleanup sweep;
    /// any started sweep runs detached in the background and finishes in [`Self::poll`].
    pub fn close_document_nonblocking(&mut self) {
        self.song_cancel.store(true, Ordering::Relaxed);
        self.song_job = None;
        self.song_loading = false;
        self.pending_play_on_song = false;
        self.pending_locator_right = None;
        self.song_generation += 1;
        self.song = None;
        self.drum_stem_cancel.store(true, Ordering::Relaxed);
        self.drum_stem_job = None;
        self.drum_stem_loading = false;
        self.drum_stem_auto_monitor = false;
        self.drum_stem_generation += 1;
        self.drum_stem_path = None;
        self.drum_stem = None;
        self.drum_stem_waveform = None;
        self.monitor_stem = false;
        self.drum_vst_cancel.store(true, Ordering::Relaxed);
        self.drum_vst_job = None;
        self.drum_vst_loading = false;
        self.drum_vst_generation += 1;
        self.drum_vst_saved = None;
        self.clear_live_drum_vst();
        self.unload_all_mixer_fx();
        self.waveform.clear();
        self.waveform_revision += 1;
        self.drop_autosave();
        self.poll_sweep(false);
        if self.sweep.is_none() {
            self.start_sweep(true);
        }
        if self.sweep.is_some() {
            self.sweep_detached = true;
            self.busy = false;
        }
        // As `close_document` does when its sweep outlives the wait: the records on disk say
        // which pending names to keep, so the unused numbers free up.
        self.forget_unneeded_pending(None);
        self.document = None;
        self.base_layer_track = None;
        self.checkpoint = None;
        self.osu_watch.clear();
        self.external_osu_changed = false;
        self.tap_recording_armed = false;
        self.tap_recorded_hits.clear();
        self.selection_volume_scope = crate::SelectionVolumeScope::SelectedHits;
        keysound_session::reset_for_map(self, None);
        self.bump_view_revision();
    }

    /// Forgets the pending files no record lists. `keep` are the listed names when a job read them.
    pub(crate) fn forget_unneeded_pending(&mut self, keep: Option<BTreeSet<String>>) {
        let project_root = self.project_root.clone();
        let Some(document) = self.document.as_mut() else { return };
        let Some(mapset) = document.path.parent().map(Path::to_path_buf) else { return };
        // A recovery copy that can't be read may list any of them: none is forgotten.
        let Some(keep) = keep.or_else(|| recorded_pending(project_root.as_deref(), &self.data_root, &mapset)) else { return };
        document.pending.forget_all_but(&keep);
    }

    pub fn save_session(&mut self) -> Result<(), String> {
        self.write_session(None, None, None, None)
    }

    pub fn set_export_settings(&mut self, settings: ExportSettings) -> Result<(), String> {
        if self.busy {
            return Err("Wait for the current job before changing export settings.".into());
        }
        settings.checked_bytes()?;
        let doc = self.document.as_mut().ok_or("Open a difficulty first.")?;
        doc.export_settings = Some(settings);
        self.output_review = None;
        self.bump_view_revision();
        Ok(())
    }

    pub(crate) fn session_settings(
        &self,
        saved_hits: Option<Vec<project::ProjectHit>>,
        saved_slots: Option<std::collections::BTreeMap<String, String>>,
        saved_ownership: Option<Option<project::HsCircleOwnership>>,
        saved_drum_vst: Option<Option<project::DrumVstSession>>,
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
                (!items.is_empty()
                    || self.project.as_ref().is_some_and(|p| {
                        p.has_saved_file || p.settings.hidden_lanes.is_some()
                    }))
                .then_some(items)
            },
            hs_mode: Some(document.hs_mode),
            starter_mode: document.starter_mode,
            snap_mode: Some(document.composer.snap_mode.into()),
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
            export_index_schema: document.export_index_schema.clone(),
            node_graph_bindings: document.node_graph_bindings.clone(),
            lane_slots: saved_slots.or_else(|| self.project.as_ref().and_then(|p| p.settings.lane_slots.clone())),
            drum_vst: saved_drum_vst.unwrap_or_else(|| {
                self.drum_vst
                    .as_ref()
                    .map(|h| project::DrumVstSession {
                        plugin_path: h.plugin_path.clone(),
                        plugin_uid: h.plugin_uid.clone(),
                        enabled: h.is_enabled(),
                        state_b64: self.drum_vst_state_b64.clone(),
                    })
                    .or_else(|| self.drum_vst_saved.clone())
                    .or_else(|| {
                        self.project
                            .as_ref()
                            .and_then(|p| p.settings.drum_vst.clone())
                    })
            }),
            mixer_fx: (!self.mixer_fx.session().is_empty())
                .then(|| self.mixer_fx.session().clone()),
            drum_stem_path: self
                .drum_stem_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        })
    }

    pub(crate) fn write_session(
        &mut self,
        saved_hits: Option<Vec<project::ProjectHit>>,
        saved_slots: Option<std::collections::BTreeMap<String, String>>,
        saved_ownership: Option<Option<project::HsCircleOwnership>>,
        saved_drum_vst: Option<Option<project::DrumVstSession>>,
    ) -> Result<(), String> {
        let settings =
            self.session_settings(saved_hits, saved_slots, saved_ownership, saved_drum_vst);
        if let (Some(project), Some(settings)) = (&mut self.project, settings) {
            project.save(settings).map_err(|e| format!("Saving session settings: {e}"))?;
        }
        Ok(())
    }

    pub fn save_preferences(&mut self) {
        self.bump_view_revision();
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
        self.bump_view_revision();
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
        self.bump_view_revision();
        Ok(outcome)
    }

    /// Imports hitsounds from an osu!mania (Mode 3) `.osu` difficulty onto the open document's
    /// timeline as one single Undo step.
    pub fn import_mania_hitsound_diff(&mut self, path: &Path) -> Result<ManiaImportReport, String> {
        if self.busy {
            return Err(
                "Wait for the current job to finish before importing mania hitsounds.".into(),
            );
        }
        if self.document.is_none() {
            return Err("Open a difficulty first.".into());
        }
        let bytes = std::fs::read(path)
            .map_err(|e| format!("Could not read mania hitsound file: {e}"))?;
        let hits = dawsu_hitsounding::mania_import::parse_mania_hitsounds_bytes(&bytes)?;
        if hits.is_empty() {
            self.status = "No hitsounds found in that mania difficulty.".into();
            self.bump_view_revision();
            return Ok(ManiaImportReport {
                hits_imported: 0,
                lanes_touched: 0,
                lanes_added: 0,
                matched_moments: 0,
                created_moments: 0,
            });
        }

        const MATCH_TOLERANCE_MS: f64 = 5.0;
        let document = self.document.as_mut().ok_or("Open a difficulty first.")?;
        let existing_lanes: std::collections::BTreeSet<SampleIdentity> = document
            .composer
            .hitsounds()
            .lanes()
            .iter()
            .map(|l| l.identity.clone())
            .collect();
        let mut touched_lanes: std::collections::BTreeSet<SampleIdentity> =
            std::collections::BTreeSet::new();
        let mut matched_moments = 0usize;
        let mut created_moments = 0usize;
        let hits_imported = hits.len();

        document
            .composer
            .apply("Import mania hitsounds", |h| {
                for hit in &hits {
                    touched_lanes.insert(hit.lane.clone());
                    let existing_moment = h
                        .moments()
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| (m.time().ms() - hit.time_ms).abs() <= MATCH_TOLERANCE_MS)
                        .min_by(|(_, a), (_, b)| {
                            let da = (a.time().ms() - hit.time_ms).abs();
                            let db = (b.time().ms() - hit.time_ms).abs();
                            da.total_cmp(&db).then_with(|| {
                                let a_loose = matches!(a.place(), dawsu_hitsounding::Place::NoObject);
                                let b_loose = matches!(b.place(), dawsu_hitsounding::Place::NoObject);
                                a_loose.cmp(&b_loose)
                            })
                        })
                        .map(|(i, _)| dawsu_hitsounding::MomentId(i));

                    let moment = match existing_moment {
                        Some(id) => {
                            matched_moments += 1;
                            id
                        }
                        None => {
                            created_moments += 1;
                            h.add_moment(SongTime::from_ms(hit.time_ms), hit.volume)
                        }
                    };

                    if h.moment(moment).has_hit(&hit.lane) {
                        h.set_hit_volume(moment, &hit.lane, hit.volume);
                    } else {
                        h.add_hit_with_volume(moment, hit.lane.clone(), hit.volume);
                    }
                }
            });

        let mut lanes_added = 0usize;
        for lane in &touched_lanes {
            if !existing_lanes.contains(lane) {
                lanes_added += 1;
            }
            if !document.composer.lanes.contains(lane) {
                document.composer.lanes.push(lane.clone());
            }
        }

        let lanes_touched = touched_lanes.len();
        self.sync_audio();
        self.refresh_recovery_edit();
        self.status = format!(
            "Imported {} {} from {} {} (Mania hitsound diff).",
            hits_imported,
            if hits_imported == 1 { "hit" } else { "hits" },
            lanes_touched,
            if lanes_touched == 1 { "lane" } else { "lanes" },
        );
        self.bump_view_revision();
        Ok(ManiaImportReport {
            hits_imported,
            lanes_touched,
            lanes_added,
            matched_moments,
            created_moments,
        })
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

    fn commit_captured_midi_and_taps(
        &mut self,
        notes: Vec<midi_record::RecordedMidiNote>,
        taps: Vec<midi_record::RecordedTap>,
        drum_map: Option<&dawsu_hitsounding::drums::DrumMap>,
        snap_divisor: Option<u32>,
        retrospective: bool,
    ) -> Result<midi_record::RetrospectiveCaptureStatus, String> {
        let mut committed_hits = 0usize;
        let mut committed_keysounds = 0usize;
        if retrospective {
            if (!notes.is_empty() || !taps.is_empty())
                && let Some(doc) = self.document.as_mut()
            {
                committed_hits = midi_record::commit_retrospective_to_composer(
                    &notes,
                    &taps,
                    &mut doc.composer,
                    drum_map,
                    snap_divisor,
                )?;
            }
        } else {
            if !notes.is_empty()
                && let Some(doc) = self.document.as_mut()
            {
                committed_hits += midi_record::commit_to_composer(
                    &notes,
                    &mut doc.composer,
                    drum_map,
                    snap_divisor,
                )?;
            }
            if !taps.is_empty()
                && let Some(doc) = self.document.as_mut()
            {
                committed_hits += midi_record::commit_taps_to_composer(&taps, &mut doc.composer)?;
            }
        }

        if committed_hits > 0 {
            self.refresh_recovery_edit();
            self.sync_audio();
        }

        if !notes.is_empty() {
            let target_tid = self
                .keysound_history
                .active_track_id
                .filter(|id| self.keysounds.document.tracks.iter().any(|t| t.id == *id))
                .or_else(|| self.keysounds.document.tracks.first().map(|t| t.id));
            if let Some(tid) = target_tid {
                let before = self.keysounds.document.clone();
                let ks_added =
                    midi_record::commit_to_keysounds(&notes, &mut self.keysounds, tid, 200.0);
                if ks_added > 0 {
                    self.keysound_history.record(before);
                    self.keysound_history.active_track_id = Some(tid);
                    committed_keysounds = ks_added;
                }
            }
        }

        Ok(midi_record::RetrospectiveCaptureStatus {
            committed_hits,
            committed_keysounds,
        })
    }

    fn monitor_live_midi_note(&self, pitch: u8, velocity: u8) {
        if let Some(vst) = &self.drum_vst
            && vst.is_enabled()
        {
            vst.preview(pitch, velocity);
            return;
        }
        let Some(doc) = &self.document else {
            return;
        };
        let map = midi_record::build_pitch_to_lane_map(&doc.composer, None);
        if let Some(lane) = map.get(&pitch) {
            self.audition_lane(lane, (f32::from(velocity) / 127.0).clamp(0.05, 1.0));
        }
    }

    /// Selects a persistent hardware MIDI input port without arming recording.
    pub fn select_midi_input_port(&mut self, port_index: Option<usize>) -> Result<Option<String>, String> {
        let name = self.midi_recorder.select_port(port_index)?;
        self.bump_view_revision();
        Ok(name)
    }

    /// Captures buffered unarmed MIDI notes and keyboard taps into the active difficulty /
    /// keysound track in one Undo step.
    pub fn capture_retrospective(
        &mut self,
        drum_map: Option<&dawsu_hitsounding::drums::DrumMap>,
        snap_divisor: Option<u32>,
    ) -> Result<midi_record::RetrospectiveCaptureStatus, String> {
        if self.document.is_none() {
            return Err("Open a difficulty first.".into());
        }
        if self.busy {
            return Err("Wait for the current job before capturing retrospective taps.".into());
        }
        let (notes, taps) = self.midi_recorder.take_retrospective();
        if notes.is_empty() && taps.is_empty() {
            self.status = "Nothing recent in retrospective buffer.".into();
            self.bump_view_revision();
            return Ok(midi_record::RetrospectiveCaptureStatus {
                committed_hits: 0,
                committed_keysounds: 0,
            });
        }
        let summary = self.commit_captured_midi_and_taps(notes, taps, drum_map, snap_divisor, true)?;
        let total = summary.committed_hits.max(summary.committed_keysounds);
        self.status = if total > 0 {
            format!(
                "Captured {total} retrospective hit{}. Press Ctrl+Z to undo.",
                if total == 1 { "" } else { "s" }
            )
        } else {
            "No matching lanes for buffered retrospective events.".into()
        };
        self.bump_view_revision();
        Ok(summary)
    }

    /// Arms or disarms live MIDI recording (and keyboard tap-recording on the timeline);
    /// disarming commits any captured MIDI notes and keyboard taps cleanly.
    pub fn toggle_midi_recording(
        &mut self,
        port_index: Option<usize>,
        drum_map: Option<&dawsu_hitsounding::drums::DrumMap>,
        snap_divisor: Option<u32>,
    ) -> Result<midi_record::MidiRecordStatus, String> {
        if self.tap_recording_armed() {
            let port_name = self.midi_recorder.port_name.clone();
            let notes = if self.midi_recorder.armed {
                let snap = self.clock_snapshot();
                self.midi_recorder.disarm_with_snapshot(
                    snap,
                    self.position_ms(),
                    self.is_playing(),
                    self.playback_speed,
                )
            } else {
                Vec::new()
            };
            self.tap_recording_armed = false;
            let taps = std::mem::take(&mut self.tap_recorded_hits);
            let summary =
                self.commit_captured_midi_and_taps(notes, taps, drum_map, snap_divisor, false)?;
            let committed = summary.committed_hits.max(summary.committed_keysounds);
            self.status = if committed > 0 {
                format!("Recorded {committed} hits.")
            } else {
                "Recording stopped.".into()
            };
            self.bump_view_revision();
            return Ok(midi_record::MidiRecordStatus {
                armed: false,
                port_name,
                buffered_notes: 0,
                committed_notes: committed,
            });
        }
        let port_name = self.midi_recorder.arm(port_index)?;
        self.tap_recording_armed = true;
        self.tap_recorded_hits.clear();
        self.status = match &port_name {
            Some(name) => format!("MIDI recording armed ({name}). Press Space to play and record."),
            None => "Tap recording armed. Press Space and tap A–L or Numpad 1–9.".into(),
        };
        self.bump_view_revision();
        Ok(midi_record::MidiRecordStatus {
            armed: true,
            port_name,
            buffered_notes: 0,
            committed_notes: 0,
        })
    }

    /// True while live keyboard tap-recording (or MIDI recording) is armed.
    pub fn tap_recording_armed(&self) -> bool {
        self.tap_recording_armed || self.midi_recorder.armed
    }

    /// Number of buffered keyboard lane taps waiting to be committed on disarm.
    pub fn tap_recorded_count(&self) -> usize {
        self.tap_recorded_hits.len()
    }

    /// Plays `row`'s sample live immediately; if tap-recording is armed, records the tap at the
    /// current playhead time (otherwise records into the retrospective buffer).
    pub fn tap_lane(&mut self, row: usize) -> bool {
        if self.busy {
            return false;
        }
        let pos_ms = self.position_ms().max(0.0);
        let armed = self.tap_recording_armed();
        let Some((audition_row, primary, recorded_lane, volume)) =
            self.document.as_ref().and_then(|d| {
                let members: Vec<usize> = self
                    .drum_row_groups
                    .as_ref()
                    .and_then(|g| g.members_for_row(row))
                    .map(|rows| rows.to_vec())
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| vec![row]);
                let primary_row = members[0];
                let primary = d.composer.lanes.get(primary_row)?.clone();
                let vol = d.composer.insert_volume.clamp(1, 100);
                let recorded = if armed && members.len() > 1 {
                    let member_ids: Vec<SampleIdentity> = members
                        .iter()
                        .filter_map(|&r| d.composer.lanes.get(r).cloned())
                        .collect();
                    crate::drum_editing::choose_member_lane(
                        d.composer.hitsounds(),
                        &d.file,
                        pos_ms,
                        &member_ids,
                        Some(&primary),
                    )
                } else {
                    primary.clone()
                };
                Some((primary_row, primary, recorded, vol))
            })
        else {
            return false;
        };
        if self.active_lane != audition_row {
            self.active_lane = audition_row;
            self.bump_view_revision();
        }
        self.audition_lane(&primary, volume as f32 / 100.0);
        let tap = midi_record::RecordedTap {
            time_ms: pos_ms,
            lane: recorded_lane,
            volume,
        };
        if armed {
            self.tap_recorded_hits.push(tap);
            self.bump_view_revision();
        } else {
            self.midi_recorder.record_retrospective_tap(tap);
        }
        true
    }

    /// Arms or disarms live keyboard tap-recording on the timeline; disarming commits all
    /// buffered taps (and any armed MIDI notes) cleanly.
    pub fn toggle_tap_recording(&mut self) -> Result<midi_record::TapRecordStatus, String> {
        if self.tap_recording_armed() {
            self.tap_recording_armed = false;
            let notes = if self.midi_recorder.armed {
                let snap = self.clock_snapshot();
                self.midi_recorder.disarm_with_snapshot(
                    snap,
                    self.position_ms(),
                    self.is_playing(),
                    self.playback_speed,
                )
            } else {
                Vec::new()
            };
            let taps = std::mem::take(&mut self.tap_recorded_hits);
            let summary = self.commit_captured_midi_and_taps(notes, taps, None, None, false)?;
            let committed = summary.committed_hits.max(summary.committed_keysounds);
            self.status = if committed > 0 {
                format!(
                    "Recorded {committed} tap{}. Press Ctrl+Z to undo.",
                    if committed == 1 { "" } else { "s" }
                )
            } else {
                "Tap recording stopped.".into()
            };
            self.bump_view_revision();
            return Ok(midi_record::TapRecordStatus {
                armed: false,
                buffered_taps: 0,
                committed_taps: committed,
            });
        }
        self.tap_recording_armed = true;
        self.tap_recorded_hits.clear();
        self.status = "Tap recording armed. Press Space and tap A–L or Numpad 1–9.".into();
        self.bump_view_revision();
        Ok(midi_record::TapRecordStatus {
            armed: true,
            buffered_taps: 0,
            committed_taps: 0,
        })
    }

    pub fn set_reload_osu_after_save(&mut self, enabled: bool) {
        if self.preferences.reload_osu_after_save == enabled {
            return;
        }
        self.preferences.reload_osu_after_save = enabled;
        self.save_preferences();
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

    /// Clears the persistent VST3 scan blacklist and re-scans all discovered plug-ins.
    pub fn retry_blocked_vst3_plugins(&mut self) -> Result<vst3::Vst3ScanReport, String> {
        self.vst3.retry_blocked_plugins().map_err(|e| e.to_string())
    }

    /// Parses and inspects an `.sfz` or `.sf2` SoundFont file, caching it for fast playback.
    pub fn inspect_soundfont_file(
        &mut self,
        path: &Path,
    ) -> Result<dawsu_audio_instruments::SamplerSummary, String> {
        keysound_session::inspect_soundfont(self, path)
    }

    /// Renders and auditions one note from an `.sfz` or `.sf2` SoundFont file.
    pub fn audition_soundfont_note(
        &mut self,
        path: &Path,
        preset_index: usize,
        pitch: u8,
        velocity: u8,
        duration_ms: f64,
    ) -> Result<(), String> {
        keysound_session::audition_soundfont_note(self, path, preset_index, pitch, velocity, duration_ms)
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
        if let Some(plugin) = vst3_plugin_name
            && let Some(track) = self.keysounds.document.tracks.iter_mut().find(|t| t.id == id) {
                track.instrument.name = plugin.to_string();
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
            self.bump_view_revision();
            return;
        }
        self.open_now(OpenSource::Map(path));
    }

    /// Opens the bundled synthetic practice mapset inside an isolated temp directory without
    /// touching `preferences.recent` or user files.
    pub fn request_open_practice_mapset(&mut self) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        if self.is_dirty() {
            self.error = Some("Save or discard your changes before opening the practice mapset.".into());
            self.bump_view_revision();
            return;
        }
        self.start_open(OpenSource::Practice, true);
    }
    pub fn confirm_revert(&mut self) {
        if self.busy || !std::mem::take(&mut self.revert_requested) {
            return;
        }
        self.bump_view_revision();
        if let Some(path) = self.document.as_ref().map(|d| d.path.clone()) {
            self.pending_open = None;
            self.start_open(OpenSource::Map(path), false);
        }
    }

    pub fn revert_document(&mut self) {
        if self.busy || self.document.is_none() {
            return;
        }
        self.revert_requested = true;
        self.confirm_revert();
    }

    pub fn has_original_snapshot(&self) -> bool {
        self.document.is_some() && self.has_original_snapshot.load(Ordering::Relaxed)
    }

    pub fn can_undo_revert(&self) -> bool {
        self.document.is_some() && self.can_undo_revert.load(Ordering::Relaxed)
    }

    pub fn preview_revert_mapset_to_original(&self) -> Result<SnapshotPreview, String> {
        let path = self
            .document
            .as_ref()
            .map(|d| &d.path)
            .ok_or("Open a difficulty first.")?;
        let folder = path.parent().ok_or("Missing mapset folder.")?;
        original_snapshot::preview(&self.data_root, folder)
    }

    pub fn revert_mapset_to_original(&mut self, include_deleted: &[String]) -> Result<usize, String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before reverting the mapset.".into());
        }
        let path = self
            .document
            .as_ref()
            .map(|d| d.path.clone())
            .ok_or("Open a difficulty first.")?;
        let folder = path.parent().ok_or("Missing mapset folder.")?.to_path_buf();
        let restored = original_snapshot::restore_with_options(&self.data_root, &folder, include_deleted)?;
        self.pending_open = None;
        self.revert_requested = false;
        self.start_open(OpenSource::Map(path), false);
        self.sweep_after_open = true;
        self.can_undo_revert.store(
            original_snapshot::has_pre_revert_backup(&self.data_root, &folder),
            Ordering::Relaxed,
        );
        self.status = format!("Reverted {restored} mapset file(s) to original snapshot (pre-revert backup saved).");
        self.bump_view_revision();
        Ok(restored)
    }

    pub fn undo_mapset_revert(&mut self) -> Result<usize, String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before undoing the revert.".into());
        }
        let path = self
            .document
            .as_ref()
            .map(|d| d.path.clone())
            .ok_or("Open a difficulty first.")?;
        let folder = path.parent().ok_or("Missing mapset folder.")?.to_path_buf();
        let restored = original_snapshot::undo_revert(&self.data_root, &folder)?;
        self.pending_open = None;
        self.revert_requested = false;
        self.start_open(OpenSource::Map(path), false);
        self.sweep_after_open = true;
        self.status = format!("Restored {restored} mapset file(s) from pre-revert backup.");
        self.bump_view_revision();
        Ok(restored)
    }

    pub fn discard_and_open(&mut self) {
        if self.busy {
            return;
        }
        if let Some(path) = self.pending_open.take() {
            self.bump_view_revision();
            self.open_now(OpenSource::Map(path));
        }
    }
    pub(crate) fn open_now(&mut self, source: OpenSource) {
        self.start_open(source, true);
    }

    pub fn reload_from_disk(&mut self) {
        self.start_reload_from_disk("Reloaded changes from disk", true);
    }

    pub fn dismiss_external_osu_change(&mut self) {
        self.external_osu_changed = false;
        if let Some(d) = &self.document {
            self.osu_watch.dismiss(&d.path);
        }
        self.status = "Kept your edits. Save will still check for conflicts.".into();
        self.bump_view_revision();
    }

    pub(crate) fn start_reload_from_disk(&mut self, status_note: &'static str, forced: bool) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        let Some(d) = &self.document else {
            return;
        };
        let path = d.path.clone();
        let song_path = d
            .file
            .general("AudioFilename")
            .and_then(|name| d.path.parent().and_then(|folder| safe_child(folder, &name).ok()));
        let preserve = ReloadPreserve {
            position_ms: self.position_ms(),
            view_start: d.composer.view_start,
            zoom: self.zoom,
            row_height: self.row_height,
            was_playing: self.is_playing(),
            playback_speed: self.playback_speed,
            song_path,
            forced,
            status_note,
            snap_mode: d.composer.snap_mode,
            snap_enabled: d.composer.snap_enabled,
            spacing: d.composer.spacing,
            hidden_lanes: d.composer.hitsounds().hidden_lanes().cloned().collect(),
        };
        self.external_osu_changed = false;
        self.start_open_inner(OpenSource::Map(path), false, Some(preserve));
    }

    pub(crate) fn start_open(&mut self, source: OpenSource, keep_settings: bool) {
        self.start_open_inner(source, keep_settings, None);
    }

    pub(crate) fn start_open_inner(
        &mut self,
        source: OpenSource,
        keep_settings: bool,
        preserve: Option<ReloadPreserve>,
    ) {
        // A cleanup the last Save started finishes before the difficulty changes.
        self.poll_sweep(true);
        self.sweep_after_open = false;
        self.cancel_timeline_edit();
        if keep_settings && let Err(error) = self.save_session() {
            self.error = Some(error);
            self.bump_view_revision();
            return;
        }
        let is_live_reload = preserve.is_some();
        if !is_live_reload {
            self.stop();
        }
        self.cancel_controller_stroke();
        self.clear_drum_detection();
        self.clear_node_graph_run();
        if let Some(d) = self.document.as_mut() {
            self.tools.cancel(&mut d.composer);
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        if !is_live_reload {
            self.song_cancel.store(true, Ordering::Relaxed);
            self.song_cancel = Arc::new(AtomicBool::new(false));
            self.song_job = None;
            self.song_loading = false;
            self.pending_play_on_song = false;
            self.pending_locator_right = None;
            self.drum_stem_cancel.store(true, Ordering::Relaxed);
            self.drum_stem_cancel = Arc::new(AtomicBool::new(false));
            self.drum_stem_job = None;
            self.drum_stem_loading = false;
            self.drum_stem_auto_monitor = false;
            self.drum_vst_cancel.store(true, Ordering::Relaxed);
            self.drum_vst_cancel = Arc::new(AtomicBool::new(false));
            self.drum_vst_job = None;
            self.drum_vst_loading = false;
        }
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
        let outgoing_had_autosave = self.autosave_id.is_some() || self.autosave_job.is_some();
        self.status = "Opening map and decoding audio…".into();
        if self.sweep_detached {
            self.status = sweep::with_note(&self.status, sweep::STILL_RUNNING);
        }
        self.bump_view_revision();
        std::thread::spawn(move || {
            let result = (|| {
                let (path, recovery) = match source {
                    OpenSource::Map(path) => (path, None),
                    OpenSource::Recovery(path) => {
                        let copy = recovery::read_copy(&path)?;
                        (copy.source().to_owned(), Some(copy))
                    }
                    OpenSource::Practice => (
                        crate::practice::materialize_practice_mapset(outgoing_mapset.as_deref())?,
                        None,
                    ),
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
                let is_practice = crate::practice::is_practice_mapset_path(&path);
                let practice_project_root = is_practice
                    .then(|| crate::practice::practice_project_root_for(&path))
                    .flatten();
                let effective_project_root = if is_practice {
                    practice_project_root.as_deref()
                } else {
                    project_root.as_deref()
                };
                let effective_legacy_root = if is_practice {
                    None
                } else {
                    legacy_root.as_deref()
                };
                let mut loaded = load(
                    &path,
                    &defaults,
                    &data_root,
                    &cancel,
                    effective_project_root,
                    effective_legacy_root,
                    recovery,
                )?;
                loaded.outgoing_keep = outgoing_mapset.filter(|_| !outgoing_had_autosave).and_then(|mapset| {
                    let outgoing_practice_root = crate::practice::is_practice_mapset_path(&mapset)
                        .then(|| crate::practice::practice_project_root_for(&mapset))
                        .flatten();
                    let outgoing_project_root = if outgoing_practice_root.is_some() {
                        outgoing_practice_root.as_deref()
                    } else {
                        project_root.as_deref()
                    };
                    recorded_pending(outgoing_project_root, &data_root, &mapset)
                });
                Ok(loaded)
            })();
            let _ = send.send(JobResult::Open(outgoing, keep_settings, preserve, result.map(Box::new)));
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
        self.bump_view_revision();
    }
    pub fn collect_retired_audio(&self) -> bool {
        self.audio
            .as_ref()
            .is_some_and(AudioEngine::collect_retired)
    }

    pub(crate) fn track_progress(&mut self, label: &'static str) -> JobProgress {
        let progress = JobProgress { label, current: Arc::new(Mutex::new(None)) };
        self.status = label.into();
        self.job_progress = Some(progress.clone());
        self.bump_view_revision();
        progress
    }

    pub fn poll(&mut self) {
        let stroke_open = self.is_stroke_open();
        if !stroke_open {
            // Edits made outside the refreshing entry points still reach the recovery checkpoint.
            self.refresh_recovery_edit();
        }
        self.autosave_recovery(std::time::Instant::now());
        self.poll_sweep(false);
        self.check_set_versions_when_read();
        self.poll_drum_detection();
        self.poll_node_graph();
        self.poll_async_jobs();
        let clock_snap = self.clock_snapshot();
        let midi_events = self.midi_recorder.poll_with_snapshot(
            clock_snap,
            self.position_ms(),
            self.is_playing(),
            self.playback_speed,
            std::time::Instant::now(),
        );
        if let Some(err) = self.midi_recorder.listener.last_error.take() {
            self.status = err;
            self.bump_view_revision();
        }
        if !self.busy {
            for ev in midi_events {
                if let dawsu_audio_instruments::MidiInputEvent::NoteOn {
                    pitch,
                    velocity,
                    ..
                } = ev
                    && velocity > 0
                {
                    self.monitor_live_midi_note(pitch, velocity);
                }
            }
        }
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
                self.bump_view_revision();
                None
            }
            _ => None,
        };
        if let Some(result) = song_result {
            self.apply_song_job_result(result);
        }
        let stem_result = match self.drum_stem_job.as_ref().map(|r| r.try_recv()) {
            Some(Ok(result)) => {
                self.drum_stem_job = None;
                Some(result)
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.drum_stem_job = None;
                self.drum_stem_loading = false;
                self.drum_stem_auto_monitor = false;
                self.bump_view_revision();
                None
            }
            _ => None,
        };
        if let Some(result) = stem_result {
            self.apply_drum_stem_job_result(result);
        }
        let drum_vst_result = match self.drum_vst_job.as_ref().map(|r| r.try_recv()) {
            Some(Ok(result)) => {
                self.drum_vst_job = None;
                Some(result)
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                self.drum_vst_job = None;
                self.drum_vst_loading = false;
                self.set_drum_vst_unavailable_status("background load stopped");
                self.bump_view_revision();
                None
            }
            _ => None,
        };
        if let Some(result) = drum_vst_result {
            self.apply_drum_vst_job_result(result);
            self.bump_view_revision();
        }
        if !self.cancel.load(Ordering::Relaxed)
            && let Some((done, total, label)) = self
                .job_progress
                .as_ref()
                .and_then(|progress| progress.current.try_lock().ok().and_then(|mut c| c.take()).map(|(d, t)| (d, t, progress.label)))
        {
            self.status = format!("{label} {done} / {total} objects");
            self.bump_view_revision();
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
                self.bump_view_revision();
                None
            }
            _ => None,
        };
        if let Some(result) = result {
            self.job_progress = None;
            self.jobs = None;
            self.busy = false;
            self.bump_view_revision();
            match result {
                JobResult::Open(outgoing, keep_settings, preserve, Ok(loaded)) => {
                    let loaded = *loaded;
                    if self.document.as_ref().map(|d| d.composer.history.version()) != outgoing {
                        self.sweep_after_open = false;
                        if preserve.as_ref().is_some_and(|p| !p.forced) {
                            // The mapper started editing while an automatic reload was in flight.
                            // Never discard their new edits — surface the external-change notice.
                            self.external_osu_changed = true;
                            self.status = "Map changed on disk. Reload or keep your edits.".into();
                            return;
                        }
                        self.pending_open = Some(loaded.document.path);
                        self.status =
                            "The outgoing difficulty changed. Review your edits before switching."
                                .into();
                        return;
                    }
                    if keep_settings && let Err(error) = self.save_session() {
                        self.sweep_after_open = false;
                        self.error = Some(error);
                        return;
                    }
                    let path = loaded.document.path.clone();
                    let is_practice = crate::practice::is_practice_mapset_path(&path);
                    let snap_flag = Arc::new(AtomicBool::new(false));
                    self.has_original_snapshot = snap_flag.clone();
                    let has_undo = !is_practice
                        && path
                            .parent()
                            .is_some_and(|folder| original_snapshot::has_pre_revert_backup(&self.data_root, folder));
                    self.can_undo_revert = Arc::new(AtomicBool::new(has_undo));
                    if !is_practice && let Some(folder) = path.parent().map(Path::to_path_buf) {
                        let data_root = self.data_root.clone();
                        std::thread::spawn(move || {
                            let _ = original_snapshot::capture_once(&data_root, &folder);
                            if original_snapshot::has_snapshot(&data_root, &folder) {
                                snap_flag.store(true, Ordering::Relaxed);
                            }
                        });
                    }
                    // The outgoing difficulty closes here. Its timed copy goes first: the names
                    // read on the open thread may include what that copy listed.
                    let had_autosave = self.autosave_id.is_some() || self.autosave_job.is_some();
                    self.drop_autosave();
                    let keep = if had_autosave { None } else { loaded.outgoing_keep };
                    self.forget_unneeded_pending(keep);
                    let mut checkpoint = (!is_practice || loaded.recovery.is_some())
                        .then(|| recovery::Checkpoint::new(&loaded.document));
                    let restored_id = match (loaded.recovery, checkpoint.as_mut()) {
                        (Some(copy), Some(checkpoint)) => Some(copy.resume(checkpoint)),
                        _ => None,
                    };
                    self.document = Some(loaded.document);
                    self.sync_shared_base_track();
                    // Reviews of the outgoing load must not match this one: history versions restart.
                    self.document_generation += 1;
                    self.opens += 1;
                    self.output_review = None;
                    self.export.clear();
                    self.pending_save_review = None;
                    self.last_endpoint_jump = None;
                    if preserve.is_none() {
                        self.selection_volume_scope = crate::SelectionVolumeScope::SelectedHits;
                    }
                    self.output_problems.clear();
                    self.output_problem_revision = None;
                    self.save_problem_review = false;
                    self.external_osu_changed = false;
                    self.sample_watch = sample_watch::Watch::default();
                    self.osu_watch.reset_for(&path);
                    if let Some(d) = self.document.as_mut() {
                        d.composer.protect_normals = self.preferences.protect_hitnormals;
                        if let Some(keep_view) = &preserve {
                            d.composer.snap_mode = keep_view.snap_mode;
                            d.composer.snap_enabled = keep_view.snap_enabled;
                            d.composer.spacing = keep_view.spacing;
                            d.composer.history.update_external(|hits| {
                                hits.restore_hidden_lanes(keep_view.hidden_lanes.iter().cloned());
                            });
                        }
                    }
                    let keep_song = preserve.as_ref().is_some_and(|p| {
                        p.song_path == loaded.song_path && (self.song.is_some() || self.song_loading)
                    });
                    if !keep_song {
                        self.song_generation = self.document_generation;
                        self.waveform_revision += 1;
                        self.song_cancel.store(true, Ordering::Relaxed);
                        self.song_job = None;
                        self.song_loading = false;
                        self.pending_play_on_song = false;
                        self.pending_locator_right = None;
                        self.song = None;
                        self.waveform.clear();
                        self.drum_stem_generation = self.document_generation;
                        self.drum_stem_cancel.store(true, Ordering::Relaxed);
                        self.drum_stem_job = None;
                        self.drum_stem_loading = false;
                        self.drum_stem_auto_monitor = false;
                        self.drum_stem_path = None;
                        self.drum_stem = None;
                        self.drum_stem_waveform = None;
                        self.monitor_stem = false;
                    }
                    let keep_drum_vst = preserve.is_some() && self.drum_vst.is_some();
                    if !keep_drum_vst {
                        self.drum_vst_generation = self.document_generation;
                        self.drum_vst_cancel.store(true, Ordering::Relaxed);
                        self.drum_vst_job = None;
                        self.drum_vst_loading = false;
                        self.drum_vst_saved = None;
                        self.clear_live_drum_vst();
                    }
                    self.drum.song_loaded();
                    self.node_graph_song_loaded();
                    if preserve.is_none() {
                        keysound_session::reset_for_map(self, Some(&path));
                    }
                    self.samples = loaded.samples;
                    self.warnings = loaded.warnings;
                    if preserve.is_none() {
                        self.playback_speed = 1.0;
                        self.muted.clear();
                        self.solo.clear();
                        self.song_solo = false;
                        self.hitsounds_solo = false;
                        self.lane_gains.clear();
                        self.active_lane = 0;
                        self.restore_active_lane(None);
                    } else {
                        self.restore_active_lane(None);
                    }
                    let keep_mixer_fx = preserve.is_some() && !self.mixer_fx.session().is_empty();
                    if !keep_mixer_fx {
                        self.unload_all_mixer_fx();
                    }
                    if !is_practice {
                        self.preferences.recent.retain(|p| p != &path);
                        self.preferences.recent.insert(0, path);
                        self.preferences.recent.truncate(20);
                    }
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
                    if let Some(keep_view) = &preserve {
                        self.zoom = keep_view.zoom.clamp(MIN_ZOOM, 3.0);
                        self.row_height = keep_view.row_height.clamp(20.0, 65.0);
                        self.set_view_start(keep_view.view_start.clamp(0.0, duration));
                        self.playback_speed = keep_view.playback_speed;
                        if !keep_view.was_playing {
                            self.seek(keep_view.position_ms.clamp(0.0, duration));
                        }
                    } else {
                        self.zoom = settings.zoom.unwrap_or(0.12).clamp(MIN_ZOOM, 3.0);
                        self.row_height = settings.row_height.unwrap_or(30.0).clamp(20.0, 65.0);
                        self.set_view_start(settings.view_start.clamp(0.0, duration));
                        self.seek(0.0);
                    }
                    self.cycle_changed();
                    let base_status = if let Some(keep_view) = &preserve {
                        keep_view.status_note
                    } else if restored_id.is_some() {
                        "Recovery copy restored. Save the difficulty to clear the offer."
                    } else {
                        "Difficulty loaded"
                    };
                    let mut vst_status_note: Option<String> = None;
                    if !keep_drum_vst {
                        if let Some(saved_vst) = settings.drum_vst.clone() {
                            self.drum_vst_saved = Some(saved_vst.clone());
                            if !Path::new(&saved_vst.plugin_path).exists() {
                                vst_status_note = Some(format!(
                                    "Drum VST3 unavailable: plugin not found at {}",
                                    saved_vst.plugin_path
                                ));
                            } else {
                                self.spawn_drum_vst_restore(saved_vst);
                                vst_status_note = Some("Loading drum kit…".into());
                            }
                        } else {
                            self.drum_vst_saved = None;
                            if self.drum_vst.is_some() {
                                self.clear_live_drum_vst();
                            }
                        }
                    }
                    if !keep_mixer_fx {
                        self.spawn_restore_mixer_fx_from_session(settings.mixer_fx.clone());
                    }
                    self.sync_audio();
                    if !keep_song && let Some(song_path) = loaded.song_path {
                        if settings.locator_right.is_none_or(|r| r > duration) {
                            self.pending_locator_right =
                                Some((duration, settings.locator_right, settings.cycle));
                        }
                        self.spawn_song_decode(song_path);
                    }
                    if let Some(stem_path_str) = settings.drum_stem_path.clone() {
                        let stem_path = PathBuf::from(&stem_path_str);
                        if stem_path.exists() {
                            self.drum_stem_path = Some(stem_path.clone());
                            self.spawn_drum_stem_decode(stem_path, false);
                        }
                    }
                    self.status = match vst_status_note {
                        Some(note) => format!("{base_status} ({note})"),
                        None => base_status.into(),
                    };
                    if self.sweep_detached {
                        self.status = sweep::with_note(&self.status, sweep::STILL_RUNNING);
                    }
                    if !is_practice {
                        self.save_preferences();
                    }
                    self.checkpoint = checkpoint;
                    self.restored_recovery = restored_id;
                    // The restored copy already holds this state: no timed copy until an edit.
                    if self.restored_recovery.is_some() {
                        self.autosaved_key = self.checkpoint.as_ref().map(recovery::Checkpoint::autosave_key);
                    }
                    self.ask_about_set_versions();
                    if std::mem::take(&mut self.sweep_after_open) {
                        self.start_sweep(false);
                    }
                }
                JobResult::SongReady { .. } => {
                    self.apply_song_job_result(result);
                }
                JobResult::DrumStemReady { .. } => {
                    self.apply_drum_stem_job_result(result);
                }
                JobResult::DrumVstReady { .. } => {
                    self.apply_drum_vst_job_result(result);
                }
                JobResult::Open(_, _, _, Err(e)) => {
                    self.sweep_after_open = false;
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
                            self.output_review = Some(*plan);
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
                                let plan = *plan;
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
                    Err(e) => {
                        self.export.pending_manifest = None;
                        if self.export.take_write(&Err(e.clone())) {
                            self.status = "Output did not complete".into();
                        } else {
                            self.error = Some(e);
                            self.status = "Output did not complete".into();
                        }
                    }
                    Ok((count, tracking_warning, published, index_schema)) => {
                        self.export.commit_pending_manifest(&self.data_root);
                        self.export.take_write(&Ok(count));
                        self.pending_published(&published, &[]);
                        self.status = format!("Wrote {count} reviewed files with backups");
                        if let Some(warning) = tracking_warning {
                            self.warnings.push(warning);
                            self.status = format!("Wrote {count} reviewed files. Generate replacement could not be recorded.");
                        }
                        self.start_sweep(false);
                        if let Some(d) = self.document.as_mut() {
                            if !index_schema.is_empty() {
                                d.export_index_schema.extend(index_schema);
                            }
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
                    let project = project.map(|p| *p);
                    let settings = settings.map(|s| *s);
                    self.output_problems = diagnostics;
                    if project.is_some() {
                        self.project = project;
                    }
                    match result {
                        Ok(report) => {
                            let report = *report;
                            let mut archive_error = None;
                            // A recovery copy keeps the whole compile input, loose hits included, as
                            // the state Undo returns to.
                            let compiled = report.written;
                            let drawn = report.drawn;
                            let mixed = report.mixed;
                            let cleaned_greenlines = report.cleaned_greenlines > 0;
                            if self.document.as_ref().is_some_and(|d| d.path == path) {
                                if let Some(d) = self.document.as_mut()
                                    && !report.published_mixes.is_empty()
                                {
                                    d.files.extend(report.published_mixes);
                                    d.files.sort();
                                    d.files.dedup();
                                }
                                self.pending_published(&report.published_pending, &report.pending_kept);
                                // The written difficulty replaces the timed copy, before the sweep.
                                self.drop_autosave();
                            }
                            if let Some(d) = self.document.as_mut().filter(|d| d.path == path) {
                                d.file = report.file;
                                self.osu_watch.reset_for(&d.path);
                                self.external_osu_changed = false;
                                self.document_generation += 1;
                                self.export.clear();
                                d.written = last_written(&compiled, hs_mode);
                                d.left_out = report.left_out;
                                // The difficulty is written, even when a step below returns early.
                                self.checkpoint = (!crate::practice::is_practice_mapset_path(&path))
                                    .then(|| recovery::Checkpoint::after_save(d, &compiled, &drawn, mixed.as_ref(), cleaned_greenlines, hs_mode));
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
                            self.checkpoint = (!crate::practice::is_practice_mapset_path(&path))
                                .then(|| recovery::Checkpoint::after_save(document, &compiled, &drawn, mixed.as_ref(), cleaned_greenlines, hs_mode));
                            let left_out = document.left_out.len();
                            let cleaned = report.cleaned_greenlines;
                            self.status = if archive_error.is_some() {
                                "Difficulty saved. Recovery copy is still pending.".into()
                            } else if left_out > 0 {
                                format!("Saved with backup. {left_out} part{} left out.", if left_out == 1 { "" } else { "s" })
                            } else if cleaned > 0 {
                                format!("Saved with backup. Removed {cleaned} redundant greenline{}.", if cleaned == 1 { "" } else { "s" })
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
        self.poll_osu_watch();
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
            for (lane, gain) in self.plan_audition_burst(&requests) {
                self.audition_lane(&lane, gain);
            }
        }
    }

    /// Deduplicates queued audition requests by resolved underlying sample and applies equal-power
    /// gain normalization (capped at `MAX_BURST_AUDITIONS`) so bulk moves across lanes never clip.
    pub(crate) fn plan_audition_burst(&self, requests: &[SampleIdentity]) -> Vec<(SampleIdentity, f32)> {
        const MAX_BURST_AUDITIONS: usize = 4;
        if requests.is_empty() {
            return Vec::new();
        }
        let hits = self.document().map(|d| d.composer.hitsounds());
        let mut seen = HashSet::new();
        let mut unique = Vec::new();
        for lane in requests {
            let resolved = hits
                .and_then(|h| h.lane(lane))
                .map(|m| m.plays_as())
                .unwrap_or_else(|| lane.clone());
            if seen.insert(resolved) {
                unique.push(lane.clone());
                if unique.len() >= MAX_BURST_AUDITIONS {
                    break;
                }
            }
        }
        let gain = 1.0 / (unique.len() as f32).sqrt();
        unique.into_iter().map(|lane| (lane, gain)).collect()
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
        self.bump_view_revision();
    }

    pub fn review_save(&mut self) {
        let backups = self.default_save_backups_dir();
        self.begin_save(backups, true);
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
            self.bump_view_revision();
            return;
        }
        if self.is_stroke_open() || !self.document.as_ref().is_some_and(|doc| {
            doc.path == review.plan.path && doc.composer.history.version() == review.version
                && doc.hs_mode == review.hs
        }) {
            self.error = Some("The difficulty changed. Review Save again.".into());
            self.bump_view_revision();
            return;
        }
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Writing reviewed Save…".into();
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        self.bump_view_revision();
        std::thread::spawn(move || {
            let _ = send.send(review.finish(&|| cancel.load(Ordering::Relaxed)));
        });
    }

    pub fn save(&mut self) {
        let backups = self.default_save_backups_dir();
        self.save_with_backups(backups);
    }

    fn default_save_backups_dir(&self) -> PathBuf {
        if let Some(doc) = &self.document
            && crate::practice::is_practice_mapset_path(&doc.path)
            && let Some(dir) = crate::practice::practice_backups_dir_for(&doc.path)
        {
            return dir;
        }
        self.data_root.join("backups")
    }

    pub(crate) fn save_with_backups(&mut self, backups: PathBuf) {
        self.begin_save(backups, false);
    }

    pub(crate) fn begin_save(&mut self, backups: PathBuf, review: bool) {
        if self.busy {
            return;
        }
        if self.is_stroke_open() {
            self.error = Some("Finish or cancel the current drag before saving.".into());
            self.bump_view_revision();
            return;
        }
        if self.has_dirty_keysounds() {
            if let Err(err) = keysound_session::save_dawsuks_to(self, None, false) {
                self.error = Some(err);
                self.bump_view_revision();
                return;
            }
            if !self.has_dirty_beatmap() {
                self.status = "Saved Keysound project.".into();
                self.bump_view_revision();
                if let Some(path) = self.pending_open.take() {
                    self.open_now(OpenSource::Map(path));
                }
                return;
            }
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
            self.session_settings(Some(saved_hits), Some(saved_slots), None, None)
        });
        let defaults = self.preferences.default_samples.clone();
        let clean_redundant_greenlines = self.preferences.clean_redundant_greenlines;
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
        self.bump_view_revision();
        std::thread::spawn(move || {
            let cancelled = || cancel.load(Ordering::Relaxed);
            let mut diagnostics = Vec::new();
            let result = prepare_save(
                PrepareSaveInput {
                    file,
                    hits: &hits,
                    written: &written,
                    expected,
                    path: &path,
                    backups,
                    hs,
                    owned: &owned,
                    defaults: &defaults,
                    pending: &pending,
                    project: project.as_ref(),
                    review,
                    clean_redundant_greenlines,
                },
                &cancelled,
                &mut diagnostics,
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
                JobResult::SaveCheck { version, path, hs, result: result.map(Box::new), diagnostics }
            } else {
                match result {
                    // Every Save that leaves moments out shows the Save hitsounds summary first.
                    Ok(plan) if !plan.plan.left_out.is_empty() => {
                        JobResult::SaveCheck { version, path, hs, result: Ok(Box::new(plan)), diagnostics }
                    }
                    Ok(plan) => plan.finish(&cancelled),
                    Err(error) => JobResult::Save {
                        version, path, hs_mode: hs, project: project.map(Box::new), settings: None, result: Err(error), diagnostics,
                    },
                }
            };
            let _ = send.send(job);
        });
    }

    pub fn set_auto_check_updates(&mut self, enabled: bool) {
        if self.preferences.auto_check_updates == enabled {
            return;
        }
        self.preferences.auto_check_updates = enabled;
        self.save_preferences();
    }

    pub fn take_siblings_changed(&mut self) -> bool {
        self.osu_watch.take_siblings_changed()
    }

    pub(crate) fn review_job(
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
            self.bump_view_revision();
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
            let _ = send.send(JobResult::Review(result.map(Box::new), diagnostics));
        });
    }
    pub fn generate(&mut self) {
        self.generate_with_options(false, false);
    }

    pub fn generate_with_slider_audio(&mut self, include_slider_audio: bool) {
        self.generate_with_options(include_slider_audio, false);
    }

    pub fn generate_with_options(
        &mut self,
        include_slider_audio: bool,
        auto_fix_staged_mixes: bool,
    ) {
        let defaults = self.preferences.default_samples.clone();
        let clean_redundant_greenlines = self.preferences.clean_redundant_greenlines;
        let settings = self.document.as_ref().and_then(|document| document.export_settings.clone());
        let projects = self.project_root.clone();
        self.review_job(move |source, cancel, diagnostics, progress| {
            let mut plan = output::generate_report_with_options(
                &source,
                &defaults,
                settings.as_ref(),
                projects.as_deref(),
                include_slider_audio,
                auto_fix_staged_mixes,
                &|| cancel.load(Ordering::Relaxed),
                diagnostics,
                progress,
            )?;
            if clean_redundant_greenlines {
                plan.clean_redundant_greenlines();
            }
            Ok(plan)
        });
    }

    /// Audits hitsound audio files in the open mapset folder against osu! Ranking Criteria
    /// and playback rules. Never modifies any file on disk; returns reports for samples with issues.
    pub fn audit_mapset_samples(&self) -> Result<Vec<SampleHealthReport>, String> {
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let root = document.path.parent().ok_or("Missing mapset folder.")?;
        let song_audio = document.file.value("General", "AudioFilename");
        Ok(sample_checks::audit_folder(
            root,
            &document.files,
            song_audio.as_deref(),
        ))
    }
    pub fn new_diff(&mut self, name: String) {
        self.review_job(move |source, _, _, _| output::new_diff(&source, &name));
    }

    /// Creates a new byte-preserving `[Hitsounds]` difficulty cloned from `sibling_path` (keeping
    /// CRLF, BOM, non-UTF-8 lines, timing points, and bookmarks) and optionally seeds its
    /// [`project::Project`] for a beginner starter path (`CustomOnly` or `Default12`).
    ///
    /// When `reuse_existing` is `true` and a hitsound diff already exists in the folder, returns
    /// that path without touching or overwriting its saved project.
    pub fn create_hitsound_diff(
        &self,
        sibling_path: &Path,
        reuse_existing: bool,
        starter_mode: Option<project::HsStarterMode>,
    ) -> Result<PathBuf, String> {
        let (folder, resolved_sibling) = if sibling_path.is_dir() {
            let mut diffs: Vec<PathBuf> = std::fs::read_dir(sibling_path)
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("osu")))
                .collect();
            diffs.sort();
            if diffs.is_empty() {
                return Err("This folder contains no .osu difficulty files.".into());
            }
            // Prefer an osu!standard (mode 0) difficulty as the timing reference when picking from a folder.
            let chosen = diffs
                .iter()
                .find(|p| {
                    std::fs::read(p)
                        .ok()
                        .map(|bytes| dawsu_osu_format::OsuFile::read(bytes).file)
                        .is_some_and(|f| {
                            f.general("Mode")
                                .and_then(|m| m.trim().parse::<u8>().ok())
                                .unwrap_or(0)
                                == 0
                        })
                })
                .cloned()
                .unwrap_or_else(|| diffs[0].clone());
            (sibling_path.to_path_buf(), chosen)
        } else {
            let parent = sibling_path
                .parent()
                .ok_or_else(|| "Difficulty has no parent folder.".to_string())?
                .to_path_buf();
            (parent, sibling_path.to_path_buf())
        };

        let mut sibling_entries: Vec<(String, Option<String>)> = Vec::new();
        let mut osu_paths: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|p| p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("osu")))
            .collect();
        osu_paths.sort();

        for p in &osu_paths {
            let Ok(bytes) = std::fs::read(p) else { continue };
            let file = dawsu_osu_format::OsuFile::read(bytes).file;
            let version = file.value("Metadata", "Version");
            if reuse_existing && is_existing_hs_diff(&file, version.as_deref()) {
                return Ok(p.clone());
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                sibling_entries.push((name.to_string(), version));
            }
        }

        let source_bytes = std::fs::read(&resolved_sibling)
            .map_err(|e| format!("Reading {}: {e}", resolved_sibling.display()))?;
        let source_file = dawsu_osu_format::OsuFile::read(source_bytes).file;
        let source_mode = source_file
            .general("Mode")
            .and_then(|m| m.trim().parse::<u8>().ok())
            .unwrap_or(0);
        if source_mode != 0 {
            return Err("Choose an osu!standard difficulty as the timing reference for a Hitsounds diff.".into());
        }
        let source_name = resolved_sibling
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("map.osu");

        let (planned, file_name) = (1..=999u32)
            .find_map(|n| {
                let version = if n == 1 {
                    dawsu_hitsounding::generate::HITSOUNDS_VERSION.to_string()
                } else {
                    format!("{} {n}", dawsu_hitsounding::generate::HITSOUNDS_VERSION)
                };
                match dawsu_hitsounding::generate::new_hitsound_diff(
                    &source_file,
                    source_name,
                    &version,
                    &sibling_entries,
                ) {
                    dawsu_hitsounding::generate::NewDiff::Planned { file, file_name } => {
                        Some((file, file_name))
                    }
                    _ => None,
                }
            })
            .ok_or("Could not find a free Hitsounds difficulty name in this mapset.")?;

        let target = folder.join(&file_name);
        dawsu_foundation::safe_write::create(&target, &planned.to_bytes())
            .map_err(|e| format!("Writing {file_name}: {e}"))?;

        let canonical_target = target.canonicalize().unwrap_or_else(|_| target.clone());
        if let Some(project_root) = self.project_root.as_deref() {
            let folder_files = pending::sample_folder(&folder);
            let mut proj = project::Project::load(project_root, None, &canonical_target)?;
            let mut settings = proj.settings.clone();
            settings.hs_mode = Some(true);
            if let Some(mode) = starter_mode {
                settings.starter_mode = Some(mode);
                if let dawsu_hitsounding::Decompiled::Done(hits) =
                    dawsu_hitsounding::decompile(&planned, &folder_files, &|| false, &mut |_, _| {})
                {
                    match mode {
                        project::HsStarterMode::CustomOnly => {
                            let all_hidden: std::collections::BTreeSet<String> = hits
                                .lanes()
                                .iter()
                                .map(|lane| project::lane_key(&lane.identity))
                                .collect();
                            settings.hidden_lanes = Some(all_hidden);
                            settings.lane_layout = Some(project::LaneLayout {
                                plain_list: true,
                                ..Default::default()
                            });
                            let mut choices = settings.export_choices.unwrap_or_default();
                            choices.addition_only_normal =
                                dawsu_hitsounding::index_packing::AdditionOnlyNormal::Silent;
                            settings.export_choices = Some(choices);
                        }
                        project::HsStarterMode::Default12 => {
                            let non_skin_hidden: std::collections::BTreeSet<String> = hits
                                .lanes()
                                .iter()
                                .filter(|lane| {
                                    !matches!(
                                        lane.identity,
                                        SampleIdentity::Standard {
                                            sound: dawsu_foundation::Sound::Hit(_),
                                            index,
                                            ..
                                        } if index.get() == 0
                                    )
                                })
                                .map(|lane| project::lane_key(&lane.identity))
                                .collect();
                            settings.hidden_lanes = Some(non_skin_hidden);
                        }
                    }
                }
            }
            proj.save(settings)?;
        }

        Ok(canonical_target)
    }

    /// Generates a standalone `[Hitsounds]` (or `[Hitsounds N]`) difficulty from the currently
    /// open difficulty's hitsounds (preserving stacked / multi-volume moments via staged mixes),
    /// leaving the open difficulty untouched on disk.
    pub fn create_hitsound_diff_from_current(&self) -> Result<PathBuf, String> {
        if self.busy || self.is_stroke_open() {
            return Err("Finish the current operation before creating a Hitsounds diff.".into());
        }
        let doc = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let folder = doc
            .path
            .parent()
            .ok_or("Difficulty has no parent folder.")?;
        let source = output::Source::from_document(doc);
        let source_name = doc
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("map.osu");

        let mut existing_names = std::collections::BTreeSet::new();
        let mut existing_versions = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(folder).map_err(|e| e.to_string())?.flatten() {
            let p = entry.path();
            if !p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("osu")) {
                continue;
            }
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                existing_names.insert(name.to_ascii_lowercase());
            }
            if let Ok(bytes) = std::fs::read(&p) {
                let file = dawsu_osu_format::OsuFile::read(bytes).file;
                if let Some(v) = file.value("Metadata", "Version") {
                    existing_versions.insert(v.to_ascii_lowercase());
                }
            }
        }

        let version = (1..=999u32)
            .map(|n| {
                if n == 1 {
                    dawsu_hitsounding::generate::HITSOUNDS_VERSION.to_string()
                } else {
                    format!("{} {n}", dawsu_hitsounding::generate::HITSOUNDS_VERSION)
                }
            })
            .find(|v| {
                let fname = dawsu_hitsounding::generate::file_name_for(&source.file, source_name, v);
                !existing_versions.contains(&v.to_ascii_lowercase())
                    && !existing_names.contains(&fname.to_ascii_lowercase())
            })
            .ok_or("Could not find a free Hitsounds difficulty name in this mapset.")?;

        let mut diagnostics = Vec::new();
        let mut plan = output::generate_report_for_version(
            &source,
            &version,
            &self.preferences.default_samples,
            doc.export_settings.as_ref(),
            self.project_root.as_deref(),
            false,
            false,
            mixes::MixFormat::Wav16,
            &|| false,
            &mut diagnostics,
            &mut |_, _| {},
        )?;
        if self.preferences.clean_redundant_greenlines {
            plan.clean_redundant_greenlines();
        }
        let target = plan
            .generated_file()
            .map(Path::to_path_buf)
            .ok_or("No generated Hitsounds difficulty in write plan.")?;
        plan.apply(&self.backups_folder(), &|| false)?;

        let canonical_target = target.canonicalize().unwrap_or(target);
        if let Some(project_root) = self.project_root.as_deref() {
            let mut proj = project::Project::load(project_root, None, &canonical_target)?;
            let mut settings = proj.settings.clone();
            settings.hs_mode = Some(true);
            proj.save(settings)?;
        }
        Ok(canonical_target)
    }
}

fn is_existing_hs_diff(file: &dawsu_osu_format::OsuFile, version: Option<&str>) -> bool {
    crate::desktop_services::is_hs_version_name(
        version.unwrap_or(""),
        !file.hit_objects().is_empty(),
    )
}

impl Application {
    pub fn copy_hitsounds(
        &mut self,
        targets: Vec<PathBuf>,
        options: dawsu_hitsounding::copy::CopyOptions,
    ) {
        let defaults = self.preferences.default_samples.clone();
        let clean_redundant_greenlines = self.preferences.clean_redundant_greenlines;
        self.review_job(move |source, cancel, diagnostics, progress| {
            let mut plan = output::copy_report(&source, &defaults, &targets, &options, &|| cancel.load(Ordering::Relaxed), diagnostics, progress)?;
            if clean_redundant_greenlines {
                plan.clean_redundant_greenlines();
            }
            Ok(plan)
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

    pub(crate) fn apply_output_in(&mut self, backups: PathBuf) {
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
            self.bump_view_revision();
            return;
        }
        self.write_reviewed(plan, backups);
    }

    pub(crate) fn write_reviewed(&mut self, plan: output::WritePlan, backups: PathBuf) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.jobs = Some(receive);
        self.busy = true;
        self.status = "Writing reviewed output…".into();
        self.bump_view_revision();
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
                    (count, generate_receipt::record(&plan).err(), plan.pending_names(), plan.export_index_schema)
                });
            let _ = send.send(JobResult::Output(result));
        });
    }

    pub(crate) fn poll_sample_folder(&mut self) {
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
            self.bump_view_revision();
        }
        self.register_added_samples(additions);
    }

    pub(crate) fn poll_osu_watch(&mut self) {
        if self.busy || self.is_stroke_open() {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        if !self
            .osu_watch
            .poll(&document.path, document.file.original_bytes())
        {
            return;
        }
        if !document.composer.dirty() {
            self.start_reload_from_disk("Reloaded changes from disk", false);
        } else {
            self.external_osu_changed = true;
            self.status = "Map changed on disk. Reload or keep your edits.".into();
            self.bump_view_revision();
        }
    }

    /// After a write kept its files: the pending files it wrote are mapset files now, and the
    /// snapshot record lists the sound set files among them and among those the project keeps.
    pub(crate) fn pending_published(&mut self, names: &[String], kept: &[String]) {
        let Some(d) = self.document.as_mut() else { return };
        for name in names {
            d.pending.release(name);
        }
        d.files.extend(names.iter().cloned());
        d.files.sort();
        d.files.dedup();
        if let Err(error) = self.record_written_sets(&[names, kept].concat()) {

            self.error = Some(format!("The files were written. The sound set record could not be updated: {error}"));
            self.bump_view_revision();
        }
    }

    pub(crate) fn register_added_samples(&mut self, additions: Vec<(String, Arc<AudioClip>)>) {

        self.register_samples(additions, true);
    }

    /// Registers decoded samples as lanes. Files on disk join `Document.files`; pending files
    /// only join the hits' folder listing.
    pub(crate) fn register_samples(&mut self, additions: Vec<(String, Arc<AudioClip>)>, on_disk: bool) {
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
        self.bump_view_revision();
    }
}

pub(crate) fn load(
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
    if recovery.is_none()
        && let Some(gain_rec) = project.as_ref().and_then(|p| p.settings.gain_recovery.as_ref())
        && let Err(error) = gain_recovery::restore(&mut document, gain_rec, defaults)
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
    document.starter_mode = project.as_ref().and_then(|p| p.settings.starter_mode);
    if let Some(mode) = project.as_ref().and_then(|p| p.settings.snap_mode) {
        document.composer.snap_mode = mode.into();
    }
    let has_saved_file = project.as_ref().is_some_and(|p| p.has_saved_file);
    if recovery.is_none()
        && !has_saved_file
        && !document.hs_mode
        && !document.composer.hitsounds().moments().is_empty()
    {
        // Path 3 first open (editing an existing gameplay difficulty directly):
        // default to Objects snap and show only lanes that currently have hits
        // (keeping normal-hitnormal Index 0 visible if no lane has hits yet).
        document.composer.snap_mode = dawsu_editor::tempo::SnapMode::Objects;
        let used: std::collections::BTreeSet<SampleIdentity> = document
            .composer
            .hitsounds()
            .moments()
            .iter()
            .flat_map(|moment| moment.hits())
            .cloned()
            .collect();
        let fallback_lane = SampleIdentity::Standard {
            set: dawsu_foundation::SampleSet::Normal,
            sound: dawsu_foundation::Sound::Hit(dawsu_foundation::Hitsound::Normal),
            index: dawsu_foundation::CustomIndex::new(0),
        };
        let to_hide: Vec<SampleIdentity> = document
            .composer
            .hitsounds()
            .lanes()
            .iter()
            .map(|lane| lane.identity.clone())
            .filter(|id| {
                if used.is_empty() {
                    id != &fallback_lane
                } else {
                    !used.contains(id)
                }
            })
            .collect();
        if !to_hide.is_empty() {
            document.composer.history.update_external(|hits| {
                hits.restore_hidden_lanes(to_hide.iter().cloned());
            });
        }
    }
    if let Some(ownership) = project.as_ref().and_then(|p| p.settings.hs_circle_ownership.as_ref()) {
        document.owned_hs_circles = ownership.verified_lines(&document.compile_base);
    }
    if let Some(signatures) = project.as_ref().and_then(|p| p.settings.time_signatures.clone()) {
        document.update_time_signatures(signatures);
    }
    document.export_settings = project.as_ref().and_then(|p| p.settings.export_settings.clone());
    document.drum_lanes = project.as_ref().and_then(|p| p.settings.drum_lanes.clone());
    document.export_choices = project.as_ref().and_then(|p| p.settings.export_choices.clone());
    document.export_index_schema = project.as_ref().map(|p| p.settings.export_index_schema.clone()).unwrap_or_default();
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
    type DecodedSamplesMap = (HashMap<SampleIdentity, Arc<AudioClip>>, Vec<String>);
    let samples_mutex: Mutex<DecodedSamplesMap> =
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
pub(crate) fn recorded_pending(project_root: Option<&Path>, data_root: &Path, mapset: &Path) -> Option<BTreeSet<String>> {
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

pub(crate) fn project_hits(document: &Document) -> Vec<project::ProjectHit> {
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

pub(crate) fn restore_project_hits(
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
            && *count > 0 {
                *count -= 1;
                continue;
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
pub(crate) fn left_out_problem(entry: &roll_back::LeftOut, path: &Path) -> OutputProblem {
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

pub(crate) fn problem_description(problem: &dawsu_hitsounding::ProblemKind, time: f64) -> String {
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
