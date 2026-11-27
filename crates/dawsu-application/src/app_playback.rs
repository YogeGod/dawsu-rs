use crate::drum_vst;
use crate::waveform::WaveBin;
use crate::{
    Application, Document, JobResult, OutputBackend, OutputDevice, Preferences,
    open_preferred_audio, sample_watch, toggle, waveform,
};
use dawsu_audio::{AudioClip, AudioEngine, AudioTelemetryHandle, ScheduledSound};
use dawsu_editor::tempo::{SnapMode, TempoMap};
use dawsu_foundation::{SampleIdentity, SongTime};
use dawsu_hitsounding::cue_list_filtered;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

/// Musical position/tempo/meter snapshot evaluated at a given song timestamp (ms).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MusicalSample {
    pub bpm_centi: u32,
    pub meter_num: u16,
    pub meter_den: u16,
    pub bar: i64,
    pub beat: u32,
    pub tick: u16,
}

/// Lock-free snapshot of document timing so push telemetry computes live red-line transitions
/// identically to `Document::editor_bar_at` without touching `Application`.
#[derive(Clone, Debug)]
pub struct MusicalTimingSnapshot {
    tempo: TempoMap,
    editor_bars: Vec<crate::editor_meter::EditorBarSection>,
    pub snap_label: String,
}

impl MusicalTimingSnapshot {
    pub fn at(&self, position_ms: f64) -> MusicalSample {
        let time = SongTime::from_ms(position_ms);
        let section = self.tempo.section_at(time);
        let bpm = if section.beat_length_ms > 0.0 && section.beat_length_ms.is_finite() {
            (60_000.0 / section.beat_length_ms).clamp(0.0, 4_000_000.0)
        } else {
            0.0
        };
        let bpm_centi = (bpm * 100.0).round() as u32;
        let index = self.editor_bars.partition_point(|bar| bar.start <= time.ms());
        let (bar_beat, (num, den)) = if let Some(bar_sec) = self.editor_bars.get(index.saturating_sub(1)) {
            (bar_sec.position(time.ms()), (bar_sec.numerator, bar_sec.denominator))
        } else {
            (self.tempo.bar_beat_at(time), (section.beats_per_bar, 4))
        };
        let tick = ((bar_beat.beat_fraction * 480.0 + 1e-9).floor() as u32).min(479) as u16;
        MusicalSample {
            bpm_centi,
            meter_num: num.min(u16::MAX as u32) as u16,
            meter_den: den.min(u16::MAX as u32) as u16,
            bar: bar_beat.bar,
            beat: bar_beat.beat,
            tick,
        }
    }
}

impl Application {
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

    pub fn clock_snapshot(&self) -> Option<dawsu_audio::ClockSnapshot> {
        self.audio.as_ref().and_then(AudioEngine::clock_snapshot)
    }

    pub fn select_device(&mut self, name: String) {
        let mut preferences = self.preferences.clone();
        preferences.audio_backend = OutputBackend::System;
        preferences.audio_device_id = None;
        preferences.device = Some(name);
        if let Err(error) = self.configure_audio_output(preferences) {
            self.error = Some(error);
            self.bump_view_revision();
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

    pub(crate) fn configure_audio_output(&mut self, mut preferences: Preferences) -> Result<(), String> {
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
        if let Some(engine) = &self.audio {
            engine.set_spectrum_tap_enabled(self.spectrum_enabled);
        }
        self.preferences = preferences;
        self.error = None;
        self.sync_audio();
        self.push_mixer_fx_graph();
        self.seek(cursor);
        self.cycle_changed();
        self.gains_changed();
        self.save_preferences();
        self.bump_view_revision();
        Ok(())
    }

    pub(crate) fn spawn_song_decode(&mut self, song_path: PathBuf) {
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

    pub(crate) fn apply_song_job_result(&mut self, result: JobResult) {
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
        self.bump_view_revision();
    }

    pub fn active_song_clip(&self) -> Option<Arc<AudioClip>> {
        if self.monitor_stem && self.drum_stem.is_some() {
            self.drum_stem.clone()
        } else {
            self.song.clone()
        }
    }

    pub fn active_waveform(&self) -> &[WaveBin] {
        if self.monitor_stem
            && let Some(wf) = self.drum_stem_waveform.as_deref()
        {
            return wf;
        }
        &self.waveform
    }

    pub fn has_drum_stem(&self) -> bool {
        self.drum_stem.is_some()
    }

    pub fn is_monitoring_stem(&self) -> bool {
        self.monitor_stem && self.drum_stem.is_some()
    }

    pub fn drum_stem_path(&self) -> Option<&Path> {
        self.drum_stem_path.as_deref()
    }

    pub fn drum_stem_name(&self) -> Option<String> {
        if self.drum_stem.is_none() && !self.drum_stem_loading {
            return None;
        }
        self.drum_stem_path
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
    }

    pub fn load_drum_stem(&mut self, path: PathBuf) {
        if self.document.is_none() {
            return;
        }
        let was_monitoring = self.monitor_stem;
        self.drum_stem_path = Some(path.clone());
        self.drum_stem = None;
        self.drum_stem_waveform = None;
        self.monitor_stem = false;
        if was_monitoring {
            self.waveform_revision += 1;
            self.sync_audio();
        }
        self.status = "Loading drum stem…".into();
        self.spawn_drum_stem_decode(path, true);
        let _ = self.save_session();
        self.bump_view_revision();
    }

    pub fn clear_drum_stem(&mut self) {
        self.drum_stem_cancel.store(true, Ordering::Relaxed);
        self.drum_stem_job = None;
        self.drum_stem_loading = false;
        self.drum_stem_auto_monitor = false;
        self.drum_stem_generation += 1;
        let had_stem = self.drum_stem_path.is_some() || self.drum_stem.is_some();
        let was_monitoring = self.monitor_stem;
        self.drum_stem_path = None;
        self.drum_stem = None;
        self.drum_stem_waveform = None;
        self.monitor_stem = false;
        if was_monitoring {
            self.waveform_revision += 1;
            self.sync_audio();
        }
        if had_stem {
            self.status = "Drum stem cleared".into();
            let _ = self.save_session();
        }
        self.bump_view_revision();
    }

    pub fn toggle_monitor_stem(&mut self) -> bool {
        if self.document.is_none() {
            return false;
        }
        if self.drum_stem.is_none() {
            self.status = if self.drum_stem_loading {
                "Loading drum stem…".into()
            } else {
                "No drum stem attached — click + Stem to load one".into()
            };
            self.bump_view_revision();
            return false;
        }
        self.monitor_stem = !self.monitor_stem;
        self.waveform_revision += 1;
        self.sync_audio();
        self.status = if self.monitor_stem {
            let name = self.drum_stem_name().unwrap_or_else(|| "stem".into());
            format!("Monitoring drum stem ({name})")
        } else {
            "Monitoring song".into()
        };
        self.bump_view_revision();
        self.monitor_stem
    }

    pub(crate) fn spawn_drum_stem_decode(&mut self, path: PathBuf, auto_monitor: bool) {
        self.drum_stem_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.drum_stem_cancel = cancel.clone();
        let (send, receive) = mpsc::channel();
        self.drum_stem_job = Some(receive);
        self.drum_stem_loading = true;
        self.drum_stem_auto_monitor = auto_monitor;
        self.drum_stem_generation += 1;
        let generation = self.drum_stem_generation;
        std::thread::spawn(move || {
            if cancel.load(Ordering::Relaxed) {
                let _ = send.send(JobResult::DrumStemReady {
                    generation,
                    path,
                    clip: None,
                    waveform: Vec::new(),
                    warning: None,
                });
                return;
            }
            let (clip, waveform, warning) =
                match dawsu_audio::decode_with_cancel(&path, &|| cancel.load(Ordering::Relaxed)) {
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
                            (None, Vec::new(), Some(format!("Drum stem unavailable: {e}")))
                        }
                    }
                };
            let _ = send.send(JobResult::DrumStemReady {
                generation,
                path,
                clip,
                waveform,
                warning,
            });
        });
    }

    pub(crate) fn apply_drum_stem_job_result(&mut self, result: JobResult) {
        let JobResult::DrumStemReady { generation, path, clip, waveform, warning } = result else {
            return;
        };
        if generation != self.drum_stem_generation || self.document.is_none() {
            return;
        }
        self.drum_stem_loading = false;
        let auto_monitor = std::mem::take(&mut self.drum_stem_auto_monitor);
        if let Some(clip) = clip {
            self.drum_stem_path = Some(path);
            self.drum_stem = Some(clip);
            self.drum_stem_waveform = Some(Arc::new(waveform));
            if auto_monitor {
                self.monitor_stem = true;
                self.waveform_revision += 1;
                self.sync_audio();
                // Set once the WAV has decoded, so nothing reports "ready" before it is.
                self.status = "Drum stem ready. Press T to flip between Song and Drums.".into();
            } else if self.status == "Loading drum stem…" {
                self.status = "Drum stem ready (press T to toggle)".into();
            }
        } else {
            self.drum_stem_path = None;
            self.drum_stem = None;
            self.drum_stem_waveform = None;
            self.monitor_stem = false;
            if let Some(w) = warning {
                self.warnings.push(w);
            }
            if self.status == "Loading drum stem…" {
                self.status = "Drum stem unavailable".into();
            }
        }
        self.bump_view_revision();
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
                self.bump_view_revision();
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
            self.bump_view_revision();
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
            self.bump_view_revision();
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
    pub(crate) fn playback_song_gain(&self) -> f32 {
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
        self.bump_view_revision();
    }
    pub fn cycle_changed(&mut self) {
        if let Some(a) = &self.audio {
            a.cycle(
                self.cycle
                    .then_some((self.locator_left, self.locator_right)),
            );
            a.offset(self.song_offset);
        }
        self.bump_view_revision();
    }
    pub fn lane_clip(&self, lane: &SampleIdentity) -> Option<Arc<AudioClip>> {
        self.clip_for_lane(lane).cloned()
    }
    pub(crate) fn clip_for_lane(&self, lane: &SampleIdentity) -> Option<&Arc<AudioClip>> {
        let model = self.document()?.composer.hitsounds().lane(lane)?;
        self.samples.get(&model.plays_as())
    }
    pub fn audition(&self, lane: &SampleIdentity) {
        self.audition_lane(lane, 1.0);
    }
    pub(crate) fn audition_lane(&self, lane: &SampleIdentity, gain: f32) {
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
    pub(crate) fn sample_file_clip(&mut self, source: &Path) -> Result<Arc<AudioClip>, String> {
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
        self.bump_view_revision();
    }
    pub fn solo(&mut self, lane: SampleIdentity) {
        toggle(&mut self.solo, lane);
        self.sync_audio();
        self.bump_view_revision();
    }
    pub fn song_solo(&self) -> bool {
        self.song_solo
    }

    pub fn set_song_solo(&mut self, on: bool) {
        if self.document.is_some() && self.song_solo != on {
            self.song_solo = on;
            self.sync_audio();
            self.bump_view_revision();
        }
    }

    pub fn hitsounds_solo(&self) -> bool {
        self.hitsounds_solo
    }

    pub fn audio_peaks(&self) -> (f32, f32, f32) {
        self.audio.as_ref().map_or((0.0, 0.0, 0.0), |a| a.peaks())
    }

    pub fn audio_telemetry(&self) -> Option<AudioTelemetryHandle> {
        self.audio.as_ref().map(AudioEngine::telemetry_handle)
    }

    pub fn spectrum_tap(&self) -> Option<Arc<dawsu_audio::SpectrumTap>> {
        self.audio.as_ref().map(AudioEngine::spectrum_tap)
    }

    pub fn spectrum_enabled(&self) -> bool {
        self.spectrum_enabled
    }

    pub fn set_spectrum_tap_enabled(&mut self, enabled: bool) {
        self.spectrum_enabled = enabled;
        if let Some(engine) = &self.audio {
            engine.set_spectrum_tap_enabled(enabled);
        }
        self.bump_view_revision();
    }

    pub fn musical_timing_snapshot(&self) -> Option<MusicalTimingSnapshot> {
        self.document.as_ref().map(|document| {
            let composer = &document.composer;
            let snap_label = if composer.snap_enabled {
                if composer.snap_mode == SnapMode::Objects {
                    "Objects".into()
                } else {
                    format!("{}/{}", composer.spacing.beats(), composer.spacing.divisions())
                }
            } else {
                "Snap off".into()
            };
            MusicalTimingSnapshot {
                tempo: composer.tempo.clone(),
                editor_bars: document.editor_bars.clone(),
                snap_label,
            }
        })
    }
    pub fn set_hitsounds_solo(&mut self, on: bool) {
        if self.document.is_some() && self.hitsounds_solo != on {
            self.hitsounds_solo = on;
            self.sync_audio();
            self.bump_view_revision();
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
        self.bump_view_revision();
    }

    pub fn set_speed(&mut self, speed: f64) {
        if self.busy || self.document.is_none() {
            return;
        }
        if !speed.is_finite() || !(0.25..=1.5).contains(&speed) {
            self.error = Some("Playback speed must be between 25% and 150%".into());
            self.bump_view_revision();
            return;
        }
        let speed = if (speed - 1.0).abs() < 0.001 { 1.0 } else { speed };
        self.playback_speed = speed;
        self.sync_audio();
        self.status = format!("Playback speed: {:.0}%", speed * 100.0);
        self.bump_view_revision();
    }

    pub fn set_keep_pitch(&mut self, enabled: bool) {
        if self.preferences.keep_pitch == enabled {
            return;
        }
        self.preferences.keep_pitch = enabled;
        if let Some(a) = &self.audio {
            a.keep_pitch(enabled);
        }
        self.save_preferences();
        self.status = if enabled {
            "Keep pitch during speed changes: On".into()
        } else {
            "Keep pitch during speed changes: Off".into()
        };
        self.bump_view_revision();
    }


    pub fn grid_seek(&mut self, direction: i32) {
        self.grid_seek_steps(direction, 1);
    }
    pub(crate) fn grid_seek_steps(&mut self, direction: i32, count: u8) {
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

    #[cfg(test)]
    pub(crate) fn scheduled_sounds(&self, d: &Document) -> Vec<ScheduledSound> {
        self.scheduled_audio_and_midi(d).0
    }
    pub(crate) fn scheduled_audio_and_midi(&self, d: &Document) -> (Vec<ScheduledSound>, Vec<drum_vst::DrumMidiCue>) {
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
                    strip_id: self.mixer_fx.strip_id_for_sample(cue.sample()),
                });
            }
        }
        (sounds, midi_cues)
    }
    pub(crate) fn sync_audio(&mut self) {
        if let Some(d) = &self.document {
            self.cached_version = d.composer.history.version();
            self.cached_listen_version = d.composer.listen_version;
            let (cues, midi_cues) = self.scheduled_audio_and_midi(d);
            if let Some(vst) = &self.drum_vst {
                vst.set_schedule(midi_cues);
            }
            if let Some(a) = &self.audio {
                a.keep_pitch(self.preferences.keep_pitch);
                a.load_at_speed(
                    self.active_song_clip(),
                    cues,
                    self.playback_speed,
                );
                a.gains(self.playback_song_gain(), self.preferences.master_gain);
                a.hitsound_gain(self.playback_hitsound_gain());
            }
        }
    }

}
