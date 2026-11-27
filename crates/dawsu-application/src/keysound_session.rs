//! Keysounding Piano Roll editing, live note preview, VST3 instrument loading, and
//! `[Keysounds]` difficulty generation.

use crate::pending::{Ledger, PendingSample};
use crate::bounce_plan::{
    BounceAudioFormat, BounceCancelToken, calculate_osz_budget, encode_vorbis_ogg,
    uniform_batch_normalize,
};
use crate::{Application, MidiPreviewNote};
use dawsu_audio::AudioClip;
use dawsu_audio_instruments::{MidiEvent, SampleInstrument, SamplerClip, SamplerNote};
use dawsu_foundation::digest::sha256_hex;
use dawsu_foundation::safe_write::{self, BackupPolicy, WriteOutcome, replace};
use dawsu_keysounding::chords::{
    ChordResolution, detect_chords, resolve_all_chords, resolve_chord,
};
use dawsu_keysounding::generate::{
    GeneratePlan, SampleManifestEntry, plan_generate_keysounds, preview_sync, sync_flags,
};
use dawsu_editor::tempo::Snap;
use dawsu_foundation::SongTime;
use dawsu_hitsounding::Place;
use dawsu_keysounding::model::{ExportSettings, KeysoundDocument, KeysoundTrack};
use dawsu_keysounding::{KeysoundNote, KeysoundProject, StrumDirection, editing};
use dawsu_osu_format::{check_map_path, find_case_insensitive, list_folder};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_KEYSOUND_UNDO: usize = 100;
const MAX_DAWSUKS_BYTES: u64 = 16 * 1024 * 1024;
const KEYSOUND_RENDER_RATE: u32 = 48_000;

/// Bounded Undo/Redo, revision counter, dirty guard, and active-track scope for Keysounding.
pub struct KeysoundUndoHistory {
    undo_stack: VecDeque<KeysoundDocument>,
    redo_stack: Vec<KeysoundDocument>,
    pub revision: u64,
    /// Monotonic edit cursor relative to the last save/load (`Some(0)` means clean).
    saved_cursor: Option<i64>,
    cursor: i64,
    pub dirty: bool,
    pub active_track_id: Option<u64>,
    pub project_path: Option<PathBuf>,
    /// Exact bytes last loaded from or written to `project_path` (for optimistic-concurrency save).
    pub saved_bytes: Option<Vec<u8>>,
    /// Per-track loaded VST3 instance IDs (`track_id -> vst3 instance_id`).
    pub track_instances: BTreeMap<u64, String>,
    /// Cached parsed `.sfz` / `.sf2` instruments so Piano Roll previews and bounces don't re-parse.
    pub soundfont_cache: std::collections::HashMap<PathBuf, Arc<SampleInstrument>>,
}

impl Default for KeysoundUndoHistory {
    fn default() -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            revision: 0,
            saved_cursor: Some(0),
            cursor: 0,
            dirty: false,
            active_track_id: None,
            project_path: None,
            saved_bytes: None,
            track_instances: BTreeMap::new(),
            soundfont_cache: std::collections::HashMap::new(),
        }
    }
}

impl KeysoundUndoHistory {
    pub fn record(&mut self, before: KeysoundDocument) {
        if let Some(saved) = self.saved_cursor
            && saved > self.cursor
        {
            // Redo branch containing the saved state is being discarded.
            self.saved_cursor = None;
        }
        if self.undo_stack.len() >= MAX_KEYSOUND_UNDO {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(before);
        self.redo_stack.clear();
        self.cursor += 1;
        self.revision = self.revision.saturating_add(1);
        self.dirty = self.saved_cursor != Some(self.cursor);
    }

    pub fn undo(&mut self, current: &mut KeysoundDocument) -> bool {
        let Some(prev) = self.undo_stack.pop_back() else {
            return false;
        };
        let now = std::mem::replace(current, prev);
        self.redo_stack.push(now);
        self.cursor -= 1;
        self.revision = self.revision.saturating_add(1);
        self.dirty = self.saved_cursor != Some(self.cursor);
        true
    }

    pub fn redo(&mut self, current: &mut KeysoundDocument) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let now = std::mem::replace(current, next);
        self.undo_stack.push_back(now);
        self.cursor += 1;
        self.revision = self.revision.saturating_add(1);
        self.dirty = self.saved_cursor != Some(self.cursor);
        true
    }

    pub fn mark_saved(&mut self, path: PathBuf, bytes: Vec<u8>) {
        self.project_path = Some(path);
        self.saved_bytes = Some(bytes);
        self.saved_cursor = Some(self.cursor);
        self.dirty = false;
        self.revision = self.revision.saturating_add(1);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

fn validate_dawsuks_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err("Choose a .dawsuks file path first.".to_string());
    }
    if !path.is_absolute() {
        return Err("Keysound project path must be an absolute path.".to_string());
    }
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("Project path may not contain parent directory segments (..).".to_string());
    }
    let is_dawsuks = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dawsuks"));
    if !is_dawsuks {
        return Err("Keysound projects must use the .dawsuks file extension.".to_string());
    }
    Ok(())
}

pub(crate) fn default_dawsuks_path_for(data_root: &Path, map_path: &Path) -> PathBuf {
    let key = sha256_hex(map_path.to_string_lossy().as_bytes());
    data_root
        .join("keysounds-projects")
        .join(format!("{}.dawsuks", &key[..16]))
}

fn default_dawsuks_path(app: &Application) -> Option<PathBuf> {
    let doc = app.document.as_ref()?;
    let path = default_dawsuks_path_for(&app.data_root, &doc.path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    Some(path)
}

fn load_cached_soundfont(
    app: &mut Application,
    path: &Path,
) -> Result<Arc<SampleInstrument>, String> {
    if let Some(cached) = app.keysound_history.soundfont_cache.get(path) {
        return Ok(cached.clone());
    }
    let loaded = Arc::new(SampleInstrument::load(path)?);
    app.keysound_history
        .soundfont_cache
        .insert(path.to_path_buf(), loaded.clone());
    Ok(loaded)
}

fn decode_sfz_sample(path: &Path) -> Result<SamplerClip, String> {
    let clip = dawsu_audio::decode(path)?;
    Ok(SamplerClip {
        samples: clip.samples.clone(),
        channels: clip.channels as u16,
        sample_rate: clip.sample_rate,
    })
}

pub(crate) fn render_soundfont_chord(
    instrument: &SampleInstrument,
    preset_index: usize,
    pitches: &[u8],
    velocity: u8,
    duration_ms: f64,
    sample_rate: u32,
) -> Result<Vec<f32>, String> {
    let notes: Vec<SamplerNote> = pitches
        .iter()
        .map(|&pitch| SamplerNote {
            start_ms: 0.0,
            duration_ms: duration_ms.clamp(20.0, 4000.0),
            pitch,
            velocity: velocity.clamp(1, 127),
        })
        .collect();
    let audio = instrument.render_notes_with(preset_index, &notes, sample_rate, decode_sfz_sample)?;
    Ok(audio.samples)
}

pub(crate) fn inspect_soundfont(
    app: &mut Application,
    path: &Path,
) -> Result<dawsu_audio_instruments::SamplerSummary, String> {
    let inst = load_cached_soundfont(app, path)?;
    Ok(inst.summary())
}

pub(crate) fn audition_soundfont_note(
    app: &mut Application,
    path: &Path,
    preset_index: usize,
    pitch: u8,
    velocity: u8,
    duration_ms: f64,
) -> Result<(), String> {
    let inst = load_cached_soundfont(app, path)?;
    let samples = render_soundfont_chord(
        &inst,
        preset_index,
        &[pitch.min(127)],
        velocity.clamp(1, 127),
        duration_ms,
        KEYSOUND_RENDER_RATE,
    )?;
    if samples.is_empty() {
        return Err("No playable sample mapped to that key in this SoundFont.".into());
    }
    if let Some(audio) = app.audio.as_ref() {
        audio.audition(
            Arc::new(AudioClip {
                samples,
                channels: 2,
                sample_rate: KEYSOUND_RENDER_RATE,
            }),
            0.9,
        );
    }
    Ok(())
}

/// Resets Keysound state when switching or closing maps, unloading any Keysounds VST3 instances
/// and automatically loading a previously saved bound `.dawsuks` project for `map_path` if present.
pub(crate) fn reset_for_map(app: &mut Application, map_path: Option<&Path>) {
    let old_instances: Vec<String> = app
        .keysound_history
        .track_instances
        .values()
        .cloned()
        .collect();
    for inst_id in old_instances {
        let _ = app.vst3.unload_plugin(&inst_id);
    }
    let audio_hint = map_path
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    app.keysounds = KeysoundProject::new(audio_hint, "Keysounds");
    app.keysound_history = KeysoundUndoHistory::default();

    if let Some(path) = map_path {
        let bound = default_dawsuks_path_for(&app.data_root, path);
        if bound.is_file()
            && let Ok(meta) = std::fs::metadata(&bound)
            && meta.len() <= MAX_DAWSUKS_BYTES
            && let Ok(raw_bytes) = std::fs::read(&bound)
            && let Ok(text) = std::str::from_utf8(&raw_bytes)
            && let Ok(loaded) = KeysoundProject::from_json(text)
        {
            let first_id = loaded.document.tracks.first().map(|t| t.id);
            app.keysounds = loaded;
            app.keysound_history = KeysoundUndoHistory {
                project_path: Some(bound),
                saved_bytes: Some(raw_bytes),
                active_track_id: first_id,
                ..KeysoundUndoHistory::default()
            };
        }
    }
}

/// Saves the open Keysound project safely using optimistic concurrency against `saved_bytes`.
pub(crate) fn save_dawsuks_to(
    app: &mut Application,
    explicit_path: Option<PathBuf>,
    allow_overwrite_unloaded: bool,
) -> Result<PathBuf, String> {
    let path = match explicit_path {
        Some(p) => {
            validate_dawsuks_path(&p)?;
            p
        }
        None => app
            .keysound_history
            .project_path
            .clone()
            .or_else(|| default_dawsuks_path(app))
            .ok_or("Open a difficulty or choose a .dawsuks path first.")?,
    };
    validate_dawsuks_path(&path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json_text = app.keysounds.to_json().map_err(|e| e.to_string())?;
    let json_bytes = json_text.into_bytes();
    if path.exists() {
        let expected = if app.keysound_history.project_path.as_deref() == Some(path.as_path()) {
            app.keysound_history.saved_bytes.clone()
        } else if allow_overwrite_unloaded {
            std::fs::read(&path).ok()
        } else {
            None
        };
        let Some(expected_bytes) = expected else {
            return Err(
                "A .dawsuks project already exists on disk and was not loaded into this session. Open it first or choose another path."
                    .into(),
            );
        };
        let policy = BackupPolicy::new(app.data_root.join("backups"), 10);
        match replace(&path, &expected_bytes, &json_bytes, &policy).map_err(|e| e.to_string())? {
            WriteOutcome::Saved { .. } => {
                app.keysound_history.mark_saved(path.clone(), json_bytes);
                Ok(path)
            }
            WriteOutcome::ChangedOnDisk => Err(
                "Keysound project changed on disk since it was loaded. Reopen it or save under another name."
                    .into(),
            ),
        }
    } else {
        safe_write::create(&path, &json_bytes).map_err(|e| e.to_string())?;
        app.keysound_history.mark_saved(path.clone(), json_bytes);
        Ok(path)
    }
}

/// Loads a `.dawsuks` project from disk, updating `saved_bytes` baseline and clearing dirty state.
pub(crate) fn load_dawsuks_from(app: &mut Application, path: PathBuf) -> Result<PathBuf, String> {
    validate_dawsuks_path(&path)?;
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Project path must be a regular .dawsuks file.".to_string());
    }
    if meta.len() > MAX_DAWSUKS_BYTES {
        return Err(format!(
            "Project file is too large ({} bytes, limit {} bytes).",
            meta.len(),
            MAX_DAWSUKS_BYTES
        ));
    }
    let raw_bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&raw_bytes)
        .map_err(|e| format!("Invalid UTF-8 in .dawsuks project: {e}"))?;
    let loaded = KeysoundProject::from_json(text).map_err(|e| e.to_string())?;
    let old_instances: Vec<String> = app
        .keysound_history
        .track_instances
        .values()
        .cloned()
        .collect();
    for inst_id in old_instances {
        let _ = app.vst3.unload_plugin(&inst_id);
    }
    let first_id = loaded.document.tracks.first().map(|t| t.id);
    app.keysounds = loaded;
    app.keysound_history = KeysoundUndoHistory {
        project_path: Some(path.clone()),
        saved_bytes: Some(raw_bytes),
        active_track_id: first_id,
        ..KeysoundUndoHistory::default()
    };
    Ok(path)
}

fn synth_chord_clip(pitches: &[u8], velocity: u8, duration_ms: f64) -> Arc<AudioClip> {
    let notes: &[u8] = if pitches.is_empty() { &[60] } else { pitches };
    let sample_rate = KEYSOUND_RENDER_RATE;
    let dur_ms = duration_ms.clamp(40.0, 10_000.0);
    let frames = ((dur_ms / 1000.0) * f64::from(sample_rate)) as usize;
    let dur_sec = (dur_ms / 1000.0).max(0.04);
    let decay_rate = (3.5 / dur_sec).clamp(0.8, 10.0);
    let amp = (f64::from(velocity.clamp(1, 127)) / 127.0) * (0.28 / (notes.len() as f64).sqrt());
    let freqs: Vec<f64> = notes
        .iter()
        .map(|&p| 440.0 * 2.0_f64.powf((f64::from(p.min(127)) - 69.0) / 12.0))
        .collect();
    let mut samples = Vec::with_capacity(frames * 2);
    let release_frames = ((0.015 * f64::from(sample_rate)) as usize).min(frames / 4).max(1);
    for i in 0..frames {
        let t = i as f64 / f64::from(sample_rate);
        let attack = (t / 0.005).min(1.0);
        let decay = (-t * decay_rate).exp();
        let release = if frames.saturating_sub(i) < release_frames {
            frames.saturating_sub(i) as f64 / release_frames as f64
        } else {
            1.0
        };
        let env = attack * decay * release;
        let mut sum = 0.0f64;
        for &freq in &freqs {
            let phase = 2.0 * std::f64::consts::PI * freq * t;
            sum += phase.sin() * 0.75 + (phase * 2.0).sin() * 0.25;
        }
        let v = (sum * amp * env).clamp(-1.0, 1.0) as f32;
        samples.push(v);
        samples.push(v);
    }
    Arc::new(AudioClip {
        samples,
        channels: 2,
        sample_rate,
    })
}

pub(crate) enum PreparedKeysoundTrack {
    SoundFont {
        path: PathBuf,
        cached: Option<Arc<SampleInstrument>>,
        preset_index: usize,
    },
    Vst3 {
        host_exe: PathBuf,
        shm_name: String,
        plugin_path: PathBuf,
        plugin_uid: Option<String>,
        saved_state: Option<String>,
        channel: u8,
    },
    Synth,
}

pub(crate) struct PreparedKeysoundRenderJob {
    pub(crate) manifest: Vec<SampleManifestEntry>,
    pub(crate) track_targets: BTreeMap<u64, (Vec<usize>, PreparedKeysoundTrack)>,
}

pub(crate) fn prepare_keysound_render_job(
    app: &mut Application,
    manifest: &[SampleManifestEntry],
) -> Result<PreparedKeysoundRenderJob, String> {
    let mut by_track: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (idx, entry) in manifest.iter().enumerate() {
        by_track.entry(entry.track_id).or_default().push(idx);
    }
    let mut track_targets = BTreeMap::new();
    for (tid, indices) in by_track {
        let live_id = app.keysound_history.track_instances.get(&tid).cloned();
        let track_meta = app
            .keysounds
            .document
            .tracks
            .iter()
            .find(|t| t.id == tid)
            .map(|t| {
                (
                    t.channel,
                    t.instrument.soundfont_path.clone(),
                    t.instrument.vst3_plugin_uid.clone(),
                    t.instrument.vst3_state_b64.clone(),
                    t.instrument.program.unwrap_or(0) as usize,
                )
            });

        let target = if let Some(sf_path_str) = track_meta.as_ref().and_then(|m| m.1.as_deref())
            && SampleInstrument::is_soundfont_path(Path::new(sf_path_str))
            && Path::new(sf_path_str).is_file()
        {
            let path = PathBuf::from(sf_path_str);
            let cached = app.keysound_history.soundfont_cache.get(&path).cloned();
            let preset_index = track_meta.as_ref().map_or(0, |m| m.4);
            PreparedKeysoundTrack::SoundFont {
                path,
                cached,
                preset_index,
            }
        } else {
            let vst3_info = live_id
                .as_deref()
                .and_then(|id| app.vst3.instance_info(id))
                .or_else(|| {
                    track_meta.as_ref().and_then(|(_, path, uid, _, _)| {
                        let raw = path.as_deref()?;
                        if raw.to_ascii_lowercase().ends_with(".vst3") || Path::new(raw).exists() {
                            Some((PathBuf::from(raw), uid.clone()))
                        } else {
                            None
                        }
                    })
                });
            if let Some((plugin_path, plugin_uid)) = vst3_info
                && let Ok(host_exe) = app.vst3.host_executable()
            {
                let channel = track_meta.as_ref().map_or(0, |m| m.0);
                let live_state = live_id
                    .as_deref()
                    .and_then(|id| app.vst3.get_plugin_state(id).ok())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        track_meta
                            .as_ref()
                            .and_then(|m| m.3.as_deref())
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                    });
                PreparedKeysoundTrack::Vst3 {
                    host_exe,
                    shm_name: app.vst3.next_shm_name(),
                    plugin_path,
                    plugin_uid,
                    saved_state: live_state,
                    channel,
                }
            } else {
                PreparedKeysoundTrack::Synth
            }
        };
        track_targets.insert(tid, (indices, target));
    }
    Ok(PreparedKeysoundRenderJob {
        manifest: manifest.to_vec(),
        track_targets,
    })
}

pub(crate) type RenderedManifestClips = (Vec<Vec<f32>>, Vec<(PathBuf, Arc<SampleInstrument>)>);

pub(crate) fn render_manifest_clips_standalone(
    job: &PreparedKeysoundRenderJob,
    cancel: &BounceCancelToken,
) -> Result<RenderedManifestClips, String> {
    let mut clips: Vec<Option<Vec<f32>>> = vec![None; job.manifest.len()];
    let mut newly_loaded_sf = Vec::new();
    for (indices, target) in job.track_targets.values() {
        if cancel.is_cancelled() {
            return Err("Render cancelled.".into());
        }
        match target {
            PreparedKeysoundTrack::SoundFont {
                path,
                cached,
                preset_index,
            } => {
                let instrument = match cached {
                    Some(inst) => inst.clone(),
                    None => {
                        let loaded = Arc::new(SampleInstrument::load(path)?);
                        newly_loaded_sf.push((path.clone(), loaded.clone()));
                        loaded
                    }
                };
                for &idx in indices {
                    if cancel.is_cancelled() {
                        return Err("Render cancelled.".into());
                    }
                    let entry = &job.manifest[idx];
                    let samples = render_soundfont_chord(
                        &instrument,
                        *preset_index,
                        &entry.chord_pitches,
                        entry.velocity,
                        entry.duration_ms,
                        KEYSOUND_RENDER_RATE,
                    )?;
                    clips[idx] = Some(if samples.is_empty() {
                        synth_chord_clip(&entry.chord_pitches, entry.velocity, entry.duration_ms)
                            .samples
                            .clone()
                    } else {
                        samples
                    });
                }
            }
            PreparedKeysoundTrack::Vst3 {
                host_exe,
                shm_name,
                plugin_path,
                plugin_uid,
                saved_state,
                channel,
            } => {
                let batches: Vec<Vec<MidiEvent>> = indices
                    .iter()
                    .map(|&idx| {
                        let entry = &job.manifest[idx];
                        chord_midi_events(
                            *channel,
                            &entry.chord_pitches,
                            entry.velocity,
                            entry.duration_ms,
                            KEYSOUND_RENDER_RATE,
                        )
                    })
                    .collect();
                let tmp_dir = std::env::temp_dir().join(format!(
                    "dawsu-ks-vst3-{}-{}",
                    std::process::id(),
                    shm_name
                ));
                let rendered = crate::vst3::render_midi_batches_standalone(
                    host_exe.clone(),
                    shm_name.clone(),
                    tmp_dir,
                    plugin_path.clone(),
                    plugin_uid.clone(),
                    saved_state.clone(),
                    &batches,
                    KEYSOUND_RENDER_RATE,
                    Some(cancel),
                )
                .map_err(|e| e.to_string())?;
                for (&idx, buf) in indices.iter().zip(rendered) {
                    clips[idx] = Some(buf);
                }
            }
            PreparedKeysoundTrack::Synth => {
                for &idx in indices {
                    if cancel.is_cancelled() {
                        return Err("Render cancelled.".into());
                    }
                    let entry = &job.manifest[idx];
                    clips[idx] = Some(
                        synth_chord_clip(&entry.chord_pitches, entry.velocity, entry.duration_ms)
                            .samples
                            .clone(),
                    );
                }
            }
        }
    }
    Ok((
        clips.into_iter().map(|c| c.unwrap_or_default()).collect(),
        newly_loaded_sf,
    ))
}

pub(crate) fn build_keysounds_preview_value(
    plan: &GeneratePlan,
    mut raw_clips: Vec<Vec<f32>>,
    base_bytes: u64,
    apply_batch_norm: bool,
    is_ogg: bool,
    cancel: &BounceCancelToken,
) -> Result<Value, String> {
    let target_dbfs = if is_ogg {
        BounceAudioFormat::OggVorbis.target_peak_dbfs()
    } else {
        BounceAudioFormat::Wav16.target_peak_dbfs()
    };
    let silent_indices = if apply_batch_norm {
        uniform_batch_normalize(&mut raw_clips, target_dbfs).silent_clip_indices
    } else {
        raw_clips
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
                    < crate::bounce_plan::SILENT_PEAK_THRESHOLD
            })
            .map(|(i, _)| i)
            .collect()
    };
    let mut bounced_bytes = 0u64;
    let mut samples_json = Vec::with_capacity(plan.manifest.len());
    let exact_ogg_preview = is_ogg && plan.manifest.len() <= 8;
    for (idx, entry) in plan.manifest.iter().enumerate() {
        if cancel.is_cancelled() {
            return Err("Render cancelled.".into());
        }
        let bytes = if exact_ogg_preview {
            encode_vorbis_ogg(&raw_clips[idx], KEYSOUND_RENDER_RATE)
                .map(|b| b.len() as u64)
                .unwrap_or(0)
        } else if is_ogg {
            let dur_sec = raw_clips[idx].len() as f64 / (2.0 * f64::from(KEYSOUND_RENDER_RATE));
            (dur_sec * 16_000.0).round().max(1_400.0) as u64
        } else {
            44 + (raw_clips[idx].len() as u64) * 2
        };
        bounced_bytes = bounced_bytes.saturating_add(bytes);
        let silent = silent_indices.contains(&idx);
        let peak = raw_clips[idx].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let peak_dbfs = if peak > 1e-6 { 20.0 * peak.log10() } else { -99.0 };
        let dur_ms = ((raw_clips[idx].len() as f64 / (2.0 * f64::from(KEYSOUND_RENDER_RATE))) * 1000.0)
            .round() as u32;
        samples_json.push(json!({
            "filename": entry.filename,
            "durationMs": dur_ms,
            "peakDbfs": peak_dbfs,
            "usageCount": entry.usage_count,
            "silent": silent,
            "bytes": bytes,
            "pitches": entry.chord_pitches,
        }));
    }
    let budget = calculate_osz_budget(base_bytes, bounced_bytes);
    Ok(json!({
        "sourceNotes": plan.sync.synced.len(),
        "unsyncedNotes": plan.sync.unsynced.len(),
        "baseBytes": base_bytes,
        "bouncedBytes": bounced_bytes,
        "budget": budget,
        "samples": samples_json,
        "normalized": apply_batch_norm,
    }))
}

pub(crate) struct StagedKeysoundGenerate {
    pub(crate) doc_generation: u64,
    pub(crate) open_path: PathBuf,
    pub(crate) out_path: PathBuf,
    pub(crate) plan: GeneratePlan,
    pub(crate) written_names: Vec<String>,
    pub(crate) guard: CreatedKeysoundsGuard,
    pub(crate) newly_loaded_sf: Vec<(PathBuf, Arc<SampleInstrument>)>,
}

pub(crate) fn commit_staged_keysounds(
    app: &mut Application,
    stage: StagedKeysoundGenerate,
) -> Result<Value, String> {
    let StagedKeysoundGenerate {
        doc_generation,
        open_path,
        out_path,
        plan,
        written_names,
        guard,
        newly_loaded_sf,
    } = stage;
    for (path, inst) in newly_loaded_sf {
        app.keysound_history.soundfont_cache.entry(path).or_insert(inst);
    }
    let still_valid = app.document_generation == doc_generation
        && app.document.as_ref().is_some_and(|d| d.path == open_path);
    if !still_valid {
        drop(guard);
        return Err("Active difficulty changed while generating keysounds.".into());
    }
    let bytes = plan.file.to_bytes();
    let backed_up = if out_path.exists() {
        let previous = std::fs::read(&out_path).map_err(|e| e.to_string())?;
        let policy = BackupPolicy::new(app.data_root.join("backups"), 20);
        match replace(&out_path, &previous, &bytes, &policy).map_err(|e| e.to_string())? {
            dawsu_foundation::safe_write::WriteOutcome::Saved { .. } => true,
            dawsu_foundation::safe_write::WriteOutcome::ChangedOnDisk => {
                drop(guard);
                return Err(format!(
                    "{} changed on disk during write. Try again.",
                    plan.file_name
                ));
            }
        }
    } else {
        safe_write::create(&out_path, &bytes).map_err(|e| e.to_string())?;
        false
    };
    guard.keep();
    if let Some(doc) = app.document.as_mut() {
        if !doc.difficulties.iter().any(|p| p == &out_path) {
            doc.difficulties.push(out_path.clone());
            doc.difficulties.sort();
        }
        if !written_names.is_empty() {
            doc.files.extend(written_names);
            doc.files.sort();
            doc.files.dedup();
        }
    }
    let synced_count = plan.sync.synced.len();
    let unsynced_count = plan.sync.unsynced.len();
    app.status = format!(
        "Generated {} ({} synced notes, {} manifest samples).",
        plan.file_name,
        synced_count,
        plan.manifest.len()
    );
    if !app.busy {
        app.start_sweep(false);
    }
    let mut out = summary(app);
    out["generated"] = json!({
        "fileName": plan.file_name,
        "path": out_path.to_string_lossy(),
        "syncedCount": synced_count,
        "unsyncedCount": unsynced_count,
        "manifestCount": plan.manifest.len(),
        "backedUp": backed_up,
    });
    app.bump_view_revision();
    Ok(out)
}

pub(crate) fn export_settings_from_payload_pub(payload: &Value) -> ExportSettings {
    export_settings_from_payload(payload)
}

pub(crate) fn resolve_apply_batch_norm_pub(payload: &Value) -> bool {
    resolve_apply_batch_norm(payload)
}

pub(crate) fn mapset_disk_bytes_pub(folder: &Path) -> u64 {
    mapset_disk_bytes(folder)
}

pub(crate) fn prepare_and_write_wavs_pub(
    folder: &Path,
    backup_root: &Path,
    plan: &mut GeneratePlan,
    raw_clips: Vec<Vec<f32>>,
    pending_taken: &BTreeSet<String>,
    ledger: Option<&Ledger>,
    apply_batch_norm: bool,
) -> Result<(Vec<String>, CreatedKeysoundsGuard), String> {
    prepare_and_write_wavs(
        folder,
        backup_root,
        plan,
        raw_clips,
        pending_taken,
        ledger,
        apply_batch_norm,
    )
}

pub(crate) fn synth_note_clip(pitch: u8, velocity: u8, duration_ms: f64) -> Arc<AudioClip> {
    synth_chord_clip(&[pitch], velocity, duration_ms)
}

fn resolve_apply_batch_norm(payload: &Value) -> bool {
    payload
        .get("normalize")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn chord_midi_events(
    channel: u8,
    pitches: &[u8],
    velocity: u8,
    duration_ms: f64,
    sample_rate: u32,
) -> Vec<MidiEvent> {
    let notes: &[u8] = if pitches.is_empty() { &[60] } else { pitches };
    let ch = channel & 0x0F;
    let vel = velocity.clamp(1, 127);
    let off_frame = ((duration_ms.clamp(20.0, 10_000.0) / 1000.0) * f64::from(sample_rate.max(1)))
        .round()
        .max(1.0) as u64;
    let mut events = Vec::with_capacity(notes.len() * 2);
    for &p in notes {
        events.push(MidiEvent {
            frame: 0,
            status: 0x90 | ch,
            data1: p.min(127),
            data2: vel,
        });
    }
    for &p in notes {
        events.push(MidiEvent {
            frame: off_frame,
            status: 0x80 | ch,
            data1: p.min(127),
            data2: 0,
        });
    }
    events
}

/// Renders every manifest entry through its track's assigned VST3 instrument when available,
/// falling back to the built-in synth only for unassigned tracks or when the sidecar is unavailable.
fn render_manifest_clips(
    app: &mut Application,
    manifest: &[SampleManifestEntry],
    cancel: &BounceCancelToken,
) -> Result<Vec<Vec<f32>>, String> {
    let mut clips: Vec<Option<Vec<f32>>> = vec![None; manifest.len()];
    let mut by_track: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (idx, entry) in manifest.iter().enumerate() {
        by_track.entry(entry.track_id).or_default().push(idx);
    }

    for (tid, indices) in by_track {
        if cancel.is_cancelled() {
            return Err("Render cancelled.".into());
        }
        let live_id = app.keysound_history.track_instances.get(&tid).cloned();
        let track_meta = app
            .keysounds
            .document
            .tracks
            .iter()
            .find(|t| t.id == tid)
            .map(|t| {
                (
                    t.channel,
                    t.instrument.soundfont_path.clone(),
                    t.instrument.vst3_plugin_uid.clone(),
                    t.instrument.vst3_state_b64.clone(),
                    t.instrument.program.unwrap_or(0) as usize,
                )
            });

        // Pure-Rust SoundFont / SFZ rendering works on all platforms without a VST3 sidecar.
        if let Some(sf_path_str) = track_meta.as_ref().and_then(|m| m.1.as_deref())
            && SampleInstrument::is_soundfont_path(Path::new(sf_path_str))
            && Path::new(sf_path_str).is_file()
        {
            let preset_index = track_meta.as_ref().map_or(0, |m| m.4);
            let instrument = load_cached_soundfont(app, Path::new(sf_path_str))?;
            for idx in indices {
                if cancel.is_cancelled() {
                    return Err("Render cancelled.".into());
                }
                let entry = &manifest[idx];
                let samples = render_soundfont_chord(
                    &instrument,
                    preset_index,
                    &entry.chord_pitches,
                    entry.velocity,
                    entry.duration_ms,
                    KEYSOUND_RENDER_RATE,
                )?;
                clips[idx] = Some(if samples.is_empty() {
                    synth_chord_clip(&entry.chord_pitches, entry.velocity, entry.duration_ms)
                        .samples
                        .clone()
                } else {
                    samples
                });
            }
            continue;
        }

        let vst3_info = live_id
            .as_deref()
            .and_then(|id| app.vst3.instance_info(id))
            .or_else(|| {
                track_meta.as_ref().and_then(|(_, path, uid, _, _)| {
                    let raw = path.as_deref()?;
                    if raw.to_ascii_lowercase().ends_with(".vst3") || Path::new(raw).exists() {
                        Some((PathBuf::from(raw), uid.clone()))
                    } else {
                        None
                    }
                })
            });

        let channel = track_meta.as_ref().map_or(0, |m| m.0);
        let saved_state = track_meta.as_ref().and_then(|m| m.3.as_deref());

        if let Some((plugin_path, plugin_uid)) = vst3_info
            && app.vst3.is_sidecar_available()
        {
            let batches: Vec<Vec<MidiEvent>> = indices
                .iter()
                .map(|&idx| {
                    let entry = &manifest[idx];
                    chord_midi_events(
                        channel,
                        &entry.chord_pitches,
                        entry.velocity,
                        entry.duration_ms,
                        KEYSOUND_RENDER_RATE,
                    )
                })
                .collect();
            let rendered = app
                .vst3
                .render_midi_batches_via_vst3(
                    crate::vst3::Vst3RenderTarget {
                        live_instance_id: live_id.as_deref(),
                        plugin_path: &plugin_path,
                        plugin_uid,
                        saved_state_b64: saved_state,
                    },
                    &batches,
                    KEYSOUND_RENDER_RATE,
                    Some(cancel),
                )
                .map_err(|e| e.to_string())?;
            for (&idx, buf) in indices.iter().zip(rendered) {
                clips[idx] = Some(buf);
            }
            continue;
        }

        for idx in indices {
            if cancel.is_cancelled() {
                return Err("Render cancelled.".into());
            }
            let entry = &manifest[idx];
            clips[idx] = Some(
                synth_chord_clip(&entry.chord_pitches, entry.velocity, entry.duration_ms)
                    .samples
                    .clone(),
            );
        }
    }

    Ok(clips.into_iter().map(|c| c.unwrap_or_default()).collect())
}

fn is_track_audible(doc: &KeysoundDocument, track_id: u64) -> bool {
    let Some(track) = doc.tracks.iter().find(|t| t.id == track_id) else {
        return false;
    };
    if track.muted {
        return false;
    }
    let any_solo = doc.tracks.iter().any(|t| t.solo);
    !any_solo || track.solo
}

pub(crate) fn summary(app: &mut Application) -> Value {
    let loaded = app.vst3.list_loaded();
    let active_id = app
        .keysound_history
        .active_track_id
        .filter(|id| app.keysounds.document.tracks.iter().any(|t| t.id == *id))
        .or_else(|| app.keysounds.document.tracks.first().map(|t| t.id));
    app.keysound_history.active_track_id = active_id;
    // Which notes Generate would place on an object, so the roll can show the rest as left out.
    let synced = app
        .document
        .as_ref()
        .map(|d| sync_flags(&app.keysounds.document, &d.file, &ExportSettings::default()))
        .unwrap_or_default();
    let chord_conflicts = app
        .document
        .as_ref()
        .map(|d| detect_chords(&app.keysounds.document, &d.file, 10.0))
        .unwrap_or_default();
    json!({
        "targetDifficulty": app.keysounds.target_difficulty,
        "sidecarAvailable": app.vst3.is_sidecar_available(),
        "revision": app.keysound_history.revision,
        "canUndo": app.keysound_history.can_undo(),
        "canRedo": app.keysound_history.can_redo(),
        "dirty": app.keysound_history.dirty,
        "activeTrackId": active_id,
        "chordConflicts": chord_conflicts,
        "tracks": app.keysounds.document.tracks.iter().map(|t| {
            let is_active = Some(t.id) == active_id;
            let is_audible = is_track_audible(&app.keysounds.document, t.id);
            let sf_presets: Vec<String> = t
                .instrument
                .soundfont_path
                .as_deref()
                .filter(|p| SampleInstrument::is_soundfont_path(Path::new(p)))
                .and_then(|p| app.keysound_history.soundfont_cache.get(Path::new(p)))
                .map(|inst| inst.summary().presets)
                .unwrap_or_default();
            let is_soundfont = t
                .instrument
                .soundfont_path
                .as_deref()
                .is_some_and(|p| SampleInstrument::is_soundfont_path(Path::new(p)));
            json!({
                "id": t.id,
                "name": t.name,
                "channel": t.channel,
                "isDrum": t.is_drum,
                "isDrumSync": t.is_drum_sync,
                "muted": t.muted,
                "solo": t.solo,
                "noteCount": t.notes.len(),
                "instrument": t.instrument.name,
                "vst3Path": t.instrument.soundfont_path,
                "isSoundfont": is_soundfont,
                "soundfontPreset": t.instrument.program.unwrap_or(0),
                "soundfontPresets": sf_presets,
                "sustainSpans": if is_active { json!(t.sustain_spans) } else { json!([]) },
                "notes": if is_active || is_audible {
                    t.notes.iter().enumerate().map(|(index, n)| json!({
                        "index": index,
                        "startMs": n.start_ms,
                        "endMs": n.end_ms,
                        "pitch": n.pitch,
                        "velocity": n.velocity,
                        "muted": n.muted,
                        // true or false; null when Generate never considers the note.
                        "synced": synced.get(&(t.id, index)),
                    })).collect::<Vec<_>>()
                } else {
                    Vec::new()
                },
            })
        }).collect::<Vec<_>>(),
        "vst3Plugins": app.vst3.scanned_plugins().iter().map(|p| json!({
            "id": p.uid,
            "name": p.name,
            "vendor": p.vendor,
            "path": p.path.to_string_lossy(),
            "isInstrument": p.is_instrument,
        })).collect::<Vec<_>>(),
        "loadedPlugins": loaded,
    })
}

pub(crate) fn handle(app: &mut Application, payload: &Value) -> Result<Value, String> {
    let kind = payload.get("kind").and_then(|v| v.as_str()).unwrap_or("view");
    let track_id = payload.get("trackId").and_then(|v| v.as_u64());

    let mutating = matches!(
        kind,
        "add_track"
            | "delete_track"
            | "rename_track"
            | "snap_notes_to_grid"
            | "sync_drum_track"
            | "add_note"
            | "move_note"
            | "resize_note"
            | "delete_note"
            | "delete_notes"
            | "set_velocity"
            | "quantize_notes"
            | "transpose_notes"
            | "strum_notes"
            | "humanise_notes"
            | "velocity_ramp"
            | "trim_overlaps"
            | "sync_to_objects"
            | "toggle_mute"
            | "toggle_solo"
            | "resolve_chord"
            | "resolve_all_chords"
            | "load_vst3"
            | "load_soundfont"
            | "set_soundfont_preset"
    );
    let snapshot_before = mutating.then(|| app.keysounds.document.clone());
    let mut sync_result_json: Option<Value> = None;

    match kind {
        "view" => {}
        "cancel_bounce" => {
            app.bounce_cancel.cancel();
            return Ok(json!({ "cancelled": true }));
        }
        "select_track" => {
            if app.keysound_history.active_track_id != track_id {
                app.keysound_history.active_track_id = track_id;
                app.keysound_history.revision = app.keysound_history.revision.saturating_add(1);
                app.bump_view_revision();
            }
        }
        "undo" => {
            if app.keysound_history.undo(&mut app.keysounds.document) {
                app.bump_view_revision();
            }
        }
        "redo" => {
            if app.keysound_history.redo(&mut app.keysounds.document) {
                app.bump_view_revision();
            }
        }
        "save_dawsuks" => {
            let explicit_path = payload.get("path").and_then(|v| v.as_str()).map(PathBuf::from);
            let allow_overwrite = payload
                .get("overwrite")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let saved = save_dawsuks_to(app, explicit_path, allow_overwrite)?;
            app.status = format!("Saved Keysound project ({}).", saved.display());
            app.bump_view_revision();
        }
        "load_dawsuks" => {
            let path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
                .ok_or("Missing .dawsuks path")?;
            let loaded_path = load_dawsuks_from(app, path)?;
            app.status = format!("Opened Keysound project ({}).", loaded_path.display());
            app.keysound_history.revision = app.keysound_history.revision.saturating_add(1);
            app.bump_view_revision();
        }
        "resolve_chord" => {
            let policy = match payload.get("policy").and_then(|v| v.as_str()).unwrap_or("flatten_polyphonic") {
                "top_note_only" | "top_only" => ChordResolution::TopNoteOnly,
                _ => ChordResolution::FlattenPolyphonic,
            };
            let target_time = payload.get("timeMs").and_then(|v| v.as_f64());
            let target_track = track_id;
            if let Some(doc) = app.document.as_ref() {
                let conflicts = detect_chords(&app.keysounds.document, &doc.file, 10.0);
                if let Some(conflict) = conflicts.iter().find(|c| {
                    target_time.is_none_or(|t| (c.time_ms - t).abs() <= 2.0)
                        && target_track.is_none_or(|tid| c.track_id == tid)
                }) && resolve_chord(&mut app.keysounds.document, conflict, policy)
                {
                    app.status = format!("Resolved chord {}.", conflict.suggested_name);
                }
            }
        }
        "resolve_all_chords" => {
            let policy = match payload.get("policy").and_then(|v| v.as_str()).unwrap_or("flatten_polyphonic") {
                "top_note_only" | "top_only" => ChordResolution::TopNoteOnly,
                _ => ChordResolution::FlattenPolyphonic,
            };
            if let Some(doc) = app.document.as_ref() {
                let count = resolve_all_chords(&mut app.keysounds.document, &doc.file, 10.0, policy);
                if count > 0 {
                    app.status = format!("Resolved {count} chord conflict(s).");
                }
            }
        }
        "preview_bounce" => {
            app.bounce_cancel.reset();
            let cancel = app.bounce_cancel.clone();
            let (plan, base_bytes, is_ogg) = {
                let document = app.document.as_ref().ok_or("Open a difficulty first.")?;
                let settings = export_settings_from_payload(payload);
                let file_name = document
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("map.osu");
                let plan = plan_generate_keysounds(
                    &app.keysounds.document,
                    &document.file,
                    file_name,
                    &settings,
                );
                let base_bytes = document
                    .path
                    .parent()
                    .map(mapset_disk_bytes)
                    .unwrap_or(0);
                let is_ogg = settings.sample_extension.eq_ignore_ascii_case("ogg");
                (plan, base_bytes, is_ogg)
            };
            let target_dbfs = if is_ogg {
                BounceAudioFormat::OggVorbis.target_peak_dbfs()
            } else {
                BounceAudioFormat::Wav16.target_peak_dbfs()
            };
            let apply_batch_norm = resolve_apply_batch_norm(payload);
            let mut raw_clips = render_manifest_clips(app, &plan.manifest, &cancel)?;
            let silent_indices = if apply_batch_norm {
                uniform_batch_normalize(&mut raw_clips, target_dbfs).silent_clip_indices
            } else {
                raw_clips
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        c.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
                            < crate::bounce_plan::SILENT_PEAK_THRESHOLD
                    })
                    .map(|(i, _)| i)
                    .collect()
            };
            let mut bounced_bytes = 0u64;
            let mut samples_json = Vec::with_capacity(plan.manifest.len());
            let exact_ogg_preview = is_ogg && plan.manifest.len() <= 8;
            for (idx, entry) in plan.manifest.iter().enumerate() {
                if cancel.is_cancelled() {
                    return Err("Render cancelled.".into());
                }
                let bytes = if exact_ogg_preview {
                    encode_vorbis_ogg(&raw_clips[idx], KEYSOUND_RENDER_RATE)
                        .map(|b| b.len() as u64)
                        .unwrap_or(0)
                } else if is_ogg {
                    let dur_sec = raw_clips[idx].len() as f64 / (2.0 * f64::from(KEYSOUND_RENDER_RATE));
                    (dur_sec * 16_000.0).round().max(1_400.0) as u64
                } else {
                    44 + (raw_clips[idx].len() as u64) * 2
                };
                bounced_bytes = bounced_bytes.saturating_add(bytes);
                let silent = silent_indices.contains(&idx);
                let peak = raw_clips[idx].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
                let peak_dbfs = if peak > 1e-6 { 20.0 * peak.log10() } else { -99.0 };
                let dur_ms = ((raw_clips[idx].len() as f64 / (2.0 * f64::from(KEYSOUND_RENDER_RATE))) * 1000.0).round() as u32;
                samples_json.push(json!({
                    "filename": entry.filename,
                    "durationMs": dur_ms,
                    "peakDbfs": peak_dbfs,
                    "usageCount": entry.usage_count,
                    "silent": silent,
                    "bytes": bytes,
                    "pitches": entry.chord_pitches,
                }));
            }
            let budget = calculate_osz_budget(base_bytes, bounced_bytes);
            let mut out = summary(app);
            out["bouncePreview"] = json!({
                "sourceNotes": plan.sync.synced.len(),
                "unsyncedNotes": plan.sync.unsynced.len(),
                "baseBytes": base_bytes,
                "bouncedBytes": bounced_bytes,
                "budget": budget,
                "samples": samples_json,
                "normalized": apply_batch_norm,
            });
            return Ok(out);
        }
        "add_track" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Keysound Track");
            let plugin = payload.get("vst3Plugin").and_then(|v| v.as_str());
            let new_id = app.keysounds_add_track(name, plugin);
            app.keysound_history.active_track_id = Some(new_id);
        }
        "delete_track" => {
            if let Some(id) = track_id {
                if let Some(inst_id) = app.keysound_history.track_instances.remove(&id) {
                    let _ = app.vst3.unload_plugin(&inst_id);
                }
                app.keysounds.document.delete_track(id);
            }
        }
        "rename_track" => {
            let id = track_id.ok_or("Missing trackId")?;
            let name = payload.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
            if name.is_empty() {
                return Err("Track name is empty".to_string());
            }
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.name = name.to_string();
        }
        "snap_notes_to_grid" => {
            let id = track_id.ok_or("Missing trackId")?;
            let composer = &app.document.as_ref().ok_or("Open a difficulty first")?.composer;
            if !composer.snap_enabled {
                return Err("Snap is off".to_string());
            }
            let snap = Snap { on: true, mode: composer.snap_mode, spacing: composer.spacing };
            let mut objects: Vec<SongTime> = composer
                .hitsounds()
                .moments()
                .iter()
                .filter(|moment| moment.place() != Place::NoObject)
                .map(|moment| moment.time())
                .collect();
            objects.sort_by(|a, b| a.ms().total_cmp(&b.ms()));
            let tempo = composer.tempo.clone();
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            for note in &mut track.notes {
                let length = note.duration_ms();
                // Whole milliseconds, as the roll's own drag results are.
                note.start_ms = snap.time(&tempo, &objects, SongTime::from_ms(note.start_ms)).ms().round();
                note.end_ms = note.start_ms + length;
            }
            track.sort_notes();
        }
        "sync_drum_track" => {
            let track_name = payload
                .get("trackName")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Drum Map (VST3)");
            let plugin_path = payload
                .get("pluginPath")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            let plugin_uid = payload
                .get("pluginUid")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let plugin_name = payload
                .get("pluginName")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("VST3 Drum Kit");
            let mut parsed_notes: Vec<KeysoundNote> = Vec::new();
            if let Some(arr) = payload.get("notes").and_then(|v| v.as_array()) {
                for item in arr {
                    let start_ms = item.get("startMs").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
                    let end_ms = item
                        .get("endMs")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(start_ms + 120.0)
                        .max(start_ms + 20.0);
                    let pitch = item.get("pitch").and_then(|v| v.as_u64()).unwrap_or(36).min(127) as u8;
                    let velocity = item.get("velocity").and_then(|v| v.as_u64()).unwrap_or(100).clamp(1, 127) as u8;
                    parsed_notes.push(KeysoundNote::new(start_ms, end_ms, pitch, velocity));
                }
            }

            // Never delete or overwrite a user-created track by display name: only update the
            // dedicated `is_drum_sync == true` mirror track.
            let sync_id = if let Some(existing) = app.keysounds.document.tracks.iter().find(|t| t.is_drum_sync) {
                existing.id
            } else {
                let next_id = app
                    .keysounds
                    .document
                    .tracks
                    .iter()
                    .map(|t| t.id)
                    .max()
                    .map_or(1, |m| m + 1);
                let mut track = KeysoundTrack::new(next_id, track_name, 9, true);
                track.is_drum_sync = true;
                app.keysounds.document.tracks.push(track);
                next_id
            };

            if let Some(path) = plugin_path {
                let needs_load = app
                    .keysounds
                    .document
                    .tracks
                    .iter()
                    .find(|t| t.id == sync_id)
                    .is_some_and(|t| t.instrument.soundfont_path.as_deref() != Some(path))
                    || !app.keysound_history.track_instances.contains_key(&sync_id);
                if needs_load {
                    let sr = app.live_sample_rate();
                    match app
                        .vst3
                        .load_instrument_at_rate(PathBuf::from(path), plugin_uid.clone(), sr)
                    {
                        Ok(loaded) => {
                            if let Some(prev_inst) = app
                                .keysound_history
                                .track_instances
                                .insert(sync_id, loaded.id.clone())
                            {
                                let _ = app.vst3.unload_plugin(&prev_inst);
                            }
                            let state = app.vst3.get_plugin_state(&loaded.id).ok();
                            if let Some(track) =
                                app.keysounds.document.tracks.iter_mut().find(|t| t.id == sync_id)
                            {
                                track.instrument.name = format!("{plugin_name} ({})", loaded.id);
                                track.instrument.soundfont_path = Some(path.to_string());
                                track.instrument.vst3_plugin_uid = plugin_uid;
                                track.instrument.vst3_state_b64 = state;
                            }
                        }
                        Err(err) => {
                            if let Some(before) = snapshot_before {
                                app.keysounds.document = before;
                            }
                            return Err(err.to_string());
                        }
                    }
                }
            }

            if let Some(track) = app.keysounds.document.tracks.iter_mut().find(|t| t.id == sync_id) {
                track.notes = parsed_notes;
                track.sort_notes();
            }
            app.keysound_history.active_track_id = Some(sync_id);
        }
        "add_note" => {
            let id = track_id.ok_or("Missing trackId")?;
            let start_ms = payload.get("startMs").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
            let end_ms = payload
                .get("endMs")
                .and_then(|v| v.as_f64())
                .unwrap_or(start_ms + 200.0)
                .max(start_ms + 20.0);
            let pitch = payload.get("pitch").and_then(|v| v.as_u64()).unwrap_or(60).min(127) as u8;
            let velocity = payload.get("velocity").and_then(|v| v.as_u64()).unwrap_or(100).clamp(1, 127) as u8;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.notes.push(KeysoundNote::new(start_ms, end_ms, pitch, velocity));
            track.sort_notes();
        }
        "move_note" => {
            let id = track_id.ok_or("Missing trackId")?;
            let index = payload.get("noteIndex").and_then(|v| v.as_u64()).ok_or("Missing noteIndex")? as usize;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            let note = track.notes.get_mut(index).ok_or("Note index out of range")?;
            let dur = note.duration_ms().max(20.0);
            if let Some(start_ms) = payload.get("startMs").and_then(|v| v.as_f64()) {
                note.start_ms = start_ms.max(0.0);
                note.end_ms = note.start_ms + dur;
            }
            if let Some(pitch) = payload.get("pitch").and_then(|v| v.as_u64()) {
                note.pitch = pitch.min(127) as u8;
            }
            track.sort_notes();
        }
        "resize_note" => {
            let id = track_id.ok_or("Missing trackId")?;
            let index = payload.get("noteIndex").and_then(|v| v.as_u64()).ok_or("Missing noteIndex")? as usize;
            let end_ms = payload.get("endMs").and_then(|v| v.as_f64()).ok_or("Missing endMs")?;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            let note = track.notes.get_mut(index).ok_or("Note index out of range")?;
            note.end_ms = end_ms.max(note.start_ms + 20.0);
        }
        "delete_note" => {
            let id = track_id.ok_or("Missing trackId")?;
            let index = payload.get("noteIndex").and_then(|v| v.as_u64()).ok_or("Missing noteIndex")? as usize;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            if index < track.notes.len() {
                track.notes.remove(index);
            }
        }
        // The roll's Erase drag: every note it crossed, as one undo step.
        "delete_notes" => {
            let id = track_id.ok_or("Missing trackId")?;
            let mut indices: Vec<usize> = payload
                .get("noteIndices")
                .and_then(|v| v.as_array())
                .ok_or("Missing noteIndices")?
                .iter()
                .filter_map(|v| v.as_u64().map(|i| i as usize))
                .collect();
            indices.sort_unstable_by(|a, b| b.cmp(a));
            indices.dedup();
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            for index in indices {
                if index < track.notes.len() {
                    track.notes.remove(index);
                }
            }
        }
        "set_velocity" => {
            let id = track_id.ok_or("Missing trackId")?;
            let index = payload.get("noteIndex").and_then(|v| v.as_u64()).ok_or("Missing noteIndex")? as usize;
            let velocity = payload.get("velocity").and_then(|v| v.as_u64()).unwrap_or(100).clamp(1, 127) as u8;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.notes = editing::set_velocity(&track.notes, Some(&[index]), velocity);
        }
        "quantize_notes" => {
            let id = track_id.ok_or("Missing trackId")?;
            let step_ms = payload.get("stepMs").and_then(|v| v.as_f64()).unwrap_or(125.0).max(5.0);
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.notes = editing::quantize_notes(&track.notes, None, step_ms, 0.0, 1.0);
            track.sort_notes();
        }
        "transpose_notes" => {
            let id = track_id.ok_or("Missing trackId")?;
            let semitones = payload
                .get("semitones")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .clamp(-127, 127) as i8;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.notes = editing::transpose_notes(&track.notes, None, semitones);
        }
        "toggle_mute" => {
            let id = track_id.ok_or("Missing trackId")?;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.muted = !track.muted;
        }
        "toggle_solo" => {
            let id = track_id.ok_or("Missing trackId")?;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.solo = !track.solo;
        }
        "trim_overlaps" => {
            let id = track_id.ok_or("Missing trackId")?;
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.notes = editing::trim_overlaps(&track.notes);
            track.sort_notes();
        }
        "strum_notes" => {
            let id = track_id.ok_or("Missing trackId")?;
            let spread_ms = payload
                .get("spreadMs")
                .and_then(|v| v.as_f64())
                .unwrap_or(20.0)
                .clamp(1.0, 250.0);
            let direction = payload
                .get("direction")
                .and_then(|v| v.as_str())
                .map(StrumDirection::from_id)
                .unwrap_or_default();
            let tolerance_ms = payload
                .get("chordToleranceMs")
                .and_then(|v| v.as_f64())
                .unwrap_or(15.0)
                .clamp(0.0, 100.0);
            let indices: Option<Vec<usize>> = payload
                .get("noteIndices")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|n| n.as_u64().map(|i| i as usize)).collect());
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.notes = editing::strum_notes(
                &track.notes,
                indices.as_deref(),
                spread_ms,
                direction,
                tolerance_ms,
            );
            track.sort_notes();
        }
        "humanise_notes" => {
            let id = track_id.ok_or("Missing trackId")?;
            let timing_ms = payload
                .get("timingMs")
                .and_then(|v| v.as_f64())
                .unwrap_or(10.0)
                .clamp(0.0, 100.0);
            let velocity_range = payload
                .get("velocityRange")
                .and_then(|v| v.as_u64())
                .unwrap_or(12)
                .min(64) as u8;
            let seed = payload
                .get("seed")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| app.keysound_history.revision.wrapping_add(1));
            let indices: Option<Vec<usize>> = payload
                .get("noteIndices")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|n| n.as_u64().map(|i| i as usize)).collect());
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.notes = editing::humanise_notes(
                &track.notes,
                indices.as_deref(),
                timing_ms,
                velocity_range,
                seed,
            );
            track.sort_notes();
        }
        "velocity_ramp" => {
            let id = track_id.ok_or("Missing trackId")?;
            let start_ms = payload
                .get("startMs")
                .and_then(|v| v.as_f64())
                .ok_or("Missing startMs")?;
            let end_ms = payload
                .get("endMs")
                .and_then(|v| v.as_f64())
                .ok_or("Missing endMs")?;
            let start_vel = payload
                .get("startVelocity")
                .and_then(|v| v.as_u64())
                .unwrap_or(64)
                .clamp(1, 127) as u8;
            let end_vel = payload
                .get("endVelocity")
                .and_then(|v| v.as_u64())
                .unwrap_or(110)
                .clamp(1, 127) as u8;
            let curve = payload
                .get("curve")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let track = app
                .keysounds
                .document
                .tracks
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or("Keysound track not found")?;
            track.notes = editing::apply_velocity_ramp(
                &track.notes,
                start_ms,
                end_ms,
                start_vel,
                end_vel,
                curve,
            );
        }
        "sync_to_objects" => {
            let document = app.document.as_ref().ok_or("Open a difficulty first.")?;
            let mut settings = export_settings_from_payload(payload);
            if let Some(max_shift) = payload.get("maxShiftMs").and_then(|v| v.as_f64())
                && max_shift.is_finite()
                && max_shift >= 0.0
            {
                settings.sync_tolerance_ms = max_shift;
            }
            let sync = if let Some(id) = track_id {
                let mut track = app
                    .keysounds
                    .document
                    .tracks
                    .iter()
                    .find(|t| t.id == id)
                    .cloned()
                    .ok_or("Keysound track not found")?;
                track.muted = false;
                track.solo = false;
                let mut scoped = app.keysounds.document.clone();
                scoped.tracks = vec![track];
                preview_sync(&scoped, &document.file, &settings)
            } else {
                preview_sync(&app.keysounds.document, &document.file, &settings)
            };
            let mut touched_tracks = BTreeSet::new();
            for placed in &sync.synced {
                if let Some(track) = app
                    .keysounds
                    .document
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == placed.track_id)
                    && let Some(note) = track.notes.get_mut(placed.note_index)
                {
                    let dur = note.duration_ms();
                    let new_start = placed.target_time_ms.max(0.0);
                    note.start_ms = new_start;
                    note.end_ms = new_start + dur;
                    touched_tracks.insert(placed.track_id);
                }
            }
            for tid in touched_tracks {
                if let Some(track) = app
                    .keysounds
                    .document
                    .tracks
                    .iter_mut()
                    .find(|t| t.id == tid)
                {
                    track.sort_notes();
                }
            }
            sync_result_json = Some(json!({
                "syncedCount": sync.synced.len(),
                "unsyncedCount": sync.unsynced.len(),
            }));
        }
        "scan_vst3" => {
            app.scan_vst3_plugins(&[])?;
        }
        "load_vst3" => {
            let id = track_id.ok_or("Missing trackId")?;
            let plugin_path = payload
                .get("pluginPath")
                .and_then(|v| v.as_str())
                .ok_or("Missing pluginPath")?;
            let plugin_uid = payload
                .get("pluginUid")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let plugin_name = payload
                .get("pluginName")
                .and_then(|v| v.as_str())
                .unwrap_or("VST3 Instrument");
            let sr = app.live_sample_rate();
            let loaded = app
                .vst3
                .load_instrument_at_rate(PathBuf::from(plugin_path), plugin_uid.clone(), sr)
                .map_err(|e| e.to_string())?;
            if let Some(prev_inst) = app
                .keysound_history
                .track_instances
                .insert(id, loaded.id.clone())
            {
                let _ = app.vst3.unload_plugin(&prev_inst);
            }
            let state = app.vst3.get_plugin_state(&loaded.id).ok();
            if let Some(track) = app.keysounds.document.tracks.iter_mut().find(|t| t.id == id) {
                track.instrument.name = format!("{plugin_name} ({})", loaded.id);
                track.instrument.soundfont_path = Some(plugin_path.to_string());
                track.instrument.vst3_plugin_uid = plugin_uid;
                track.instrument.vst3_state_b64 = state;
            }
            app.keysound_history.active_track_id = Some(id);
        }
        "load_soundfont" => {
            let id = track_id.ok_or("Missing trackId")?;
            let path_str = payload
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or("Missing SoundFont path")?;
            let preset_index = payload
                .get("presetIndex")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(255) as u8;
            let inst = load_cached_soundfont(app, Path::new(path_str))?;
            let info = inst.summary();
            if let Some(prev_inst) = app.keysound_history.track_instances.remove(&id) {
                let _ = app.vst3.unload_plugin(&prev_inst);
            }
            if let Some(track) = app.keysounds.document.tracks.iter_mut().find(|t| t.id == id) {
                track.instrument.name = format!("{} ({})", info.name, info.format.to_uppercase());
                track.instrument.soundfont_path = Some(path_str.to_string());
                track.instrument.program = Some(preset_index);
                track.instrument.vst3_plugin_uid = None;
                track.instrument.vst3_state_b64 = None;
            }
            app.keysound_history.active_track_id = Some(id);
            app.status = format!("Loaded SoundFont {} ({} regions).", info.name, info.region_count);
        }
        "set_soundfont_preset" => {
            let id = track_id.ok_or("Missing trackId")?;
            let preset_index = payload
                .get("presetIndex")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .min(255) as u8;
            if let Some(track) = app.keysounds.document.tracks.iter_mut().find(|t| t.id == id) {
                track.instrument.program = Some(preset_index);
            }
        }
        "preview_note" => {
            let pitch = payload.get("pitch").and_then(|v| v.as_u64()).unwrap_or(60).min(127) as u8;
            let velocity = payload.get("velocity").and_then(|v| v.as_u64()).unwrap_or(100).clamp(1, 127) as u8;
            let duration_ms = payload.get("durationMs").and_then(|v| v.as_f64()).unwrap_or(140.0);
            let target_track_id = track_id.or(app.keysound_history.active_track_id);
            if let Some(tid) = target_track_id
                && !is_track_audible(&app.keysounds.document, tid)
            {
                return Ok(json!({ "previewed": true }));
            }
            let live_id = target_track_id
                .and_then(|tid| app.keysound_history.track_instances.get(&tid).cloned());
            let track_meta = target_track_id.and_then(|tid| {
                app.keysounds.document.tracks.iter().find(|t| t.id == tid).map(|t| {
                    (
                        t.channel,
                        t.instrument.soundfont_path.clone(),
                        t.instrument.vst3_plugin_uid.clone(),
                        t.instrument.vst3_state_b64.clone(),
                        t.instrument.program.unwrap_or(0) as usize,
                    )
                })
            });
            // SoundFont / SFZ direct preview (cross-platform, zero sidecar needed).
            if let Some(sf_path_str) = track_meta.as_ref().and_then(|m| m.1.as_deref())
                && SampleInstrument::is_soundfont_path(Path::new(sf_path_str))
                && Path::new(sf_path_str).is_file()
            {
                let preset_index = track_meta.as_ref().map_or(0, |m| m.4);
                if let Ok(inst) = load_cached_soundfont(app, Path::new(sf_path_str))
                    && let Ok(samples) = render_soundfont_chord(
                        &inst,
                        preset_index,
                        &[pitch],
                        velocity,
                        duration_ms,
                        KEYSOUND_RENDER_RATE,
                    )
                    && !samples.is_empty()
                {
                    if let Some(audio) = app.audio.as_ref() {
                        audio.audition(
                            Arc::new(AudioClip {
                                samples,
                                channels: 2,
                                sample_rate: KEYSOUND_RENDER_RATE,
                            }),
                            0.9,
                        );
                    }
                    return Ok(json!({ "previewed": true }));
                }
            }
            let channel = track_meta.as_ref().map_or(0, |m| m.0);
            let mut vst3_clip: Option<Arc<AudioClip>> = None;
            if let Some(ref instance_id) = live_id {
                let _ = app.vst3.send_preview_note(
                    instance_id,
                    MidiPreviewNote {
                        channel,
                        note: pitch,
                        velocity,
                        duration_ms: duration_ms.clamp(20.0, 4000.0).round() as u32,
                    },
                );
            }
            let vst3_info = live_id
                .as_deref()
                .and_then(|id| app.vst3.instance_info(id))
                .or_else(|| {
                    track_meta.as_ref().and_then(|(_, path, uid, _, _)| {
                        let raw = path.as_deref()?;
                        if raw.to_ascii_lowercase().ends_with(".vst3") || Path::new(raw).exists() {
                            Some((PathBuf::from(raw), uid.clone()))
                        } else {
                            None
                        }
                    })
                });
            if let Some((plugin_path, plugin_uid)) = vst3_info
                && app.vst3.is_sidecar_available()
            {
                let events =
                    chord_midi_events(channel, &[pitch], velocity, duration_ms, KEYSOUND_RENDER_RATE);
                let saved_state = track_meta.as_ref().and_then(|m| m.3.as_deref());
                if let Ok(mut rendered) = app.vst3.render_midi_batches_via_vst3(
                    crate::vst3::Vst3RenderTarget {
                        live_instance_id: live_id.as_deref(),
                        plugin_path: &plugin_path,
                        plugin_uid,
                        saved_state_b64: saved_state,
                    },
                    &[events],
                    KEYSOUND_RENDER_RATE,
                    None,
                ) && let Some(samples) = rendered.pop()
                    && !samples.is_empty()
                {
                    vst3_clip = Some(Arc::new(AudioClip {
                        samples,
                        channels: 2,
                        sample_rate: KEYSOUND_RENDER_RATE,
                    }));
                }
            }
            if let Some(audio) = app.audio.as_ref() {
                if let Some(clip) = vst3_clip {
                    audio.audition(clip, 0.9);
                } else if live_id.is_none() {
                    audio.audition(synth_note_clip(pitch, velocity, duration_ms), 0.8);
                }
            }
            // The caller only wants the sound; building the whole summary per key press is waste.
            return Ok(json!({ "previewed": true }));
        }
        "generate_keysounds" => {
            app.bounce_cancel.reset();
            let cancel = app.bounce_cancel.clone();
            // Finish any earlier cleanup job before checking and writing mapset files.
            app.poll_sweep(true);
            let (open_path, file_name, folder, pending_taken, ledger, mut plan) = {
                let document = app.document.as_ref().ok_or("Open a difficulty first.")?;
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
                let settings = export_settings_from_payload(payload);
                let plan = plan_generate_keysounds(
                    &app.keysounds.document,
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
            if out_path == open_path {
                return Err("Cannot overwrite the currently open difficulty with [Keysounds].".into());
            }
            let apply_batch_norm = resolve_apply_batch_norm(payload);
            let raw_clips = render_manifest_clips(app, &plan.manifest, &cancel)?;
            if cancel.is_cancelled() {
                return Err("Render cancelled.".into());
            }
            // Render and write each manifest sample before writing [Keysounds].osu so no dangling
            // sample references are ever emitted.
            let (written_names, guard) = prepare_and_write_wavs(
                &folder,
                &app.data_root.join("backups"),
                &mut plan,
                raw_clips,
                &pending_taken,
                ledger.as_ref(),
                apply_batch_norm,
            )?;
            let bytes = plan.file.to_bytes();
            let backed_up = if out_path.exists() {
                let previous = std::fs::read(&out_path).map_err(|e| e.to_string())?;
                let policy = BackupPolicy::new(app.data_root.join("backups"), 20);
                match replace(&out_path, &previous, &bytes, &policy).map_err(|e| e.to_string())? {
                    dawsu_foundation::safe_write::WriteOutcome::Saved { .. } => true,
                    dawsu_foundation::safe_write::WriteOutcome::ChangedOnDisk => {
                        return Err(format!(
                            "{} changed on disk during write. Try again.",
                            plan.file_name
                        ));
                    }
                }
            } else {
                safe_write::create(&out_path, &bytes).map_err(|e| e.to_string())?;
                false
            };
            guard.keep();
            if let Some(doc) = app.document.as_mut() {
                if !doc.difficulties.iter().any(|p| p == &out_path) {
                    doc.difficulties.push(out_path.clone());
                    doc.difficulties.sort();
                }
                if !written_names.is_empty() {
                    doc.files.extend(written_names);
                    doc.files.sort();
                    doc.files.dedup();
                }
            }
            let synced_count = plan.sync.synced.len();
            let unsynced_count = plan.sync.unsynced.len();
            app.status = format!(
                "Generated {} ({} synced notes, {} manifest samples).",
                plan.file_name,
                synced_count,
                plan.manifest.len()
            );
            if !app.busy {
                app.start_sweep(false);
            }
            let mut out = summary(app);
            out["generated"] = json!({
                "fileName": plan.file_name,
                "path": out_path.to_string_lossy(),
                "syncedCount": synced_count,
                "unsyncedCount": unsynced_count,
                "manifestCount": plan.manifest.len(),
                "backedUp": backed_up,
            });
            app.bump_view_revision();
            return Ok(out);
        }
        other => return Err(format!("Unknown keysounds action: {other}")),
    }

    if let Some(before) = snapshot_before
        && app.keysounds.document != before
    {
        app.keysound_history.record(before);
        app.bump_view_revision();
    } else if matches!(kind, "load_vst3" | "load_soundfont" | "set_soundfont_preset" | "sync_to_objects" | "resolve_chord" | "resolve_all_chords" | "preview_bounce") {
        app.bump_view_revision();
    }

    let mut out = summary(app);
    if let Some(sync_result) = sync_result_json {
        out["syncResult"] = sync_result;
    }
    Ok(out)
}

fn export_settings_from_payload(payload: &Value) -> ExportSettings {
    let mut settings = ExportSettings::default();
    if let Some(v) = payload.get("velocityBuckets").and_then(|v| v.as_u64()) {
        settings.velocity_buckets = v.clamp(1, 16) as u8;
    }
    if let Some(d) = payload.get("durationBucketMs").and_then(|v| v.as_f64())
        && d.is_finite()
        && d >= 10.0
    {
        settings.duration_bucket_ms = d.clamp(10.0, 2000.0);
    }
    if let Some(fc) = payload.get("flattenChords").and_then(|v| v.as_bool()) {
        settings.flatten_chords = fc;
    }
    if let Some(fmt) = payload.get("format").and_then(|v| v.as_str()) {
        settings.sample_extension = if fmt == "ogg_vorbis" || fmt == "ogg" {
            "ogg".to_string()
        } else {
            "wav".to_string()
        };
    }
    settings
}

fn mapset_disk_bytes(folder: &Path) -> u64 {
    std::fs::read_dir(folder)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .fold(0u64, u64::saturating_add)
}

fn encode_clip_wav(clip: &AudioClip) -> Vec<u8> {
    let channels = clip.channels.max(1) as u16;
    let sample_rate = clip.sample_rate.max(1);
    let data_bytes = (clip.samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * u32::from(channels) * 2;
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = channels * 2;
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    for &sample in &clip.samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let pcm = (clamped * 32767.0).round() as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    wav
}

pub(crate) struct CreatedKeysoundsGuard {
    created: Vec<(PathBuf, Vec<u8>, Option<Vec<u8>>)>,
    ledger: Option<(Ledger, Vec<String>, Vec<PendingSample>)>,
}

impl CreatedKeysoundsGuard {
    fn keep(mut self) {
        self.created.clear();
        self.ledger = None;
    }

    fn rollback(&mut self) {
        let mut still_ours = Vec::new();
        for (path, wav, prior) in self.created.drain(..).rev() {
            let result = match std::fs::read(&path) {
                Ok(bytes) if bytes == wav => match prior {
                    Some(old) => std::fs::write(&path, old),
                    None => std::fs::remove_file(&path),
                },
                Err(error) => Err(error),
                Ok(_) => continue,
            };
            if let Err(error) = result
                && error.kind() != std::io::ErrorKind::NotFound
            {
                let changed = std::fs::read(&path).is_ok_and(|bytes| bytes != wav);
                if !changed
                    && let Some(name) = path.file_name()
                {
                    still_ours.push(name.to_string_lossy().into_owned());
                }
            }
        }
        if let Some((ledger, names, replaced)) = self.ledger.take() {
            let gone: Vec<String> = names
                .into_iter()
                .filter(|name| !still_ours.iter().any(|ours| ours.eq_ignore_ascii_case(name)))
                .collect();
            let _ = ledger.update(|entries| {
                entries.retain(|entry| !gone.iter().any(|name| name.eq_ignore_ascii_case(&entry.name)));
                entries.extend(
                    replaced
                        .into_iter()
                        .filter(|earlier| gone.iter().any(|name| name.eq_ignore_ascii_case(&earlier.name))),
                );
            });
        }
    }
}

impl Drop for CreatedKeysoundsGuard {
    fn drop(&mut self) {
        self.rollback();
    }
}

fn numbered_sample_name(filename: &str, suffix: u32) -> String {
    if suffix <= 1 {
        return filename.to_string();
    }
    match filename.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}_{suffix}.{ext}"),
        None => format!("{filename}_{suffix}"),
    }
}

fn candidate_stem_taken_elsewhere(
    candidate: &str,
    reserved_names: &BTreeSet<String>,
    disk_files: &[String],
) -> bool {
    let lower = candidate.to_ascii_lowercase();
    let (stem, current_ext) = lower.rsplit_once('.').unwrap_or((lower.as_str(), ""));
    ["wav", "ogg", "mp3"].iter().any(|ext| {
        let probe = format!("{stem}.{ext}");
        reserved_names.contains(&probe)
            || (*ext != current_ext && find_case_insensitive(disk_files, &probe).is_some())
    })
}

#[allow(clippy::disallowed_methods)]
fn prepare_and_write_wavs(
    folder: &Path,
    backup_root: &Path,
    plan: &mut GeneratePlan,
    mut raw_clips: Vec<Vec<f32>>,
    pending_taken: &BTreeSet<String>,
    ledger: Option<&Ledger>,
    apply_batch_norm: bool,
) -> Result<(Vec<String>, CreatedKeysoundsGuard), String> {
    let disk_files = list_folder(folder).map_err(|e| e.to_string())?;
    let ledger_hashes: BTreeMap<String, String> = ledger
        .and_then(|l| l.entries().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|entry| (entry.name.to_ascii_lowercase(), entry.sha256))
        .collect();
    let mut reserved_names = pending_taken.clone();
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    type PendingKeysoundWrite = (String, Vec<u8>, String, Option<Vec<u8>>);
    let mut to_write: Vec<PendingKeysoundWrite> = Vec::new();

    if apply_batch_norm {
        let is_ogg_batch = plan
            .manifest
            .first()
            .is_some_and(|e| e.filename.to_ascii_lowercase().ends_with(".ogg"));
        let target_dbfs = if is_ogg_batch {
            BounceAudioFormat::OggVorbis.target_peak_dbfs()
        } else {
            BounceAudioFormat::Wav16.target_peak_dbfs()
        };
        let _ = uniform_batch_normalize(&mut raw_clips, target_dbfs);
    }

    for (entry, samples) in plan.manifest.iter().zip(raw_clips) {
        let is_ogg = entry.filename.to_ascii_lowercase().ends_with(".ogg");
        let wav_bytes = if is_ogg {
            encode_vorbis_ogg(&samples, KEYSOUND_RENDER_RATE)?
        } else {
            let clip = AudioClip {
                samples,
                channels: 2,
                sample_rate: KEYSOUND_RENDER_RATE,
            };
            encode_clip_wav(&clip)
        };
        let digest = sha256_hex(&wav_bytes);

        let chosen = (1u32..)
            .find_map(|suffix| {
                let candidate = numbered_sample_name(&entry.filename, suffix);
                if let Err(e) = check_map_path(&candidate) {
                    return Some(Err(crate::refusals::file_name(&e)));
                }
                let lower = candidate.to_ascii_lowercase();
                if reserved_names.contains(&lower) {
                    return None;
                }
                if let Some(existing_name) = find_case_insensitive(&disk_files, &candidate) {
                    let existing_path = folder.join(existing_name);
                    if let Ok(existing_bytes) = std::fs::read(&existing_path) {
                        let existing_sha = sha256_hex(&existing_bytes);
                        if existing_sha == digest {
                            return Some(Ok((existing_name.to_string(), None)));
                        }
                        if ledger_hashes
                            .get(&existing_name.to_ascii_lowercase())
                            .is_some_and(|recorded| recorded == &existing_sha)
                        {
                            return Some(Ok((
                                existing_name.to_string(),
                                Some((wav_bytes.clone(), digest.clone(), Some(existing_bytes))),
                            )));
                        }
                    }
                    return None;
                }
                let direct_path = folder.join(&candidate);
                if direct_path.symlink_metadata().is_ok() {
                    if let Ok(existing_bytes) = std::fs::read(&direct_path) {
                        let existing_sha = sha256_hex(&existing_bytes);
                        if existing_sha == digest {
                            return Some(Ok((candidate, None)));
                        }
                        if ledger_hashes
                            .get(&lower)
                            .is_some_and(|recorded| recorded == &existing_sha)
                        {
                            return Some(Ok((
                                candidate,
                                Some((wav_bytes.clone(), digest.clone(), Some(existing_bytes))),
                            )));
                        }
                    }
                    return None;
                }
                if candidate_stem_taken_elsewhere(&candidate, &reserved_names, &disk_files) {
                    return None;
                }
                Some(Ok((candidate, Some((wav_bytes.clone(), digest.clone(), None)))))
            })
            .ok_or_else(|| format!("No free sample filename for {}", entry.filename))??;

        let (chosen_name, write_payload) = chosen;
        reserved_names.insert(chosen_name.to_ascii_lowercase());
        if chosen_name != entry.filename {
            renames.insert(entry.filename.clone(), chosen_name.clone());
        }
        if let Some((bytes, sha256, prior)) = write_payload {
            to_write.push((chosen_name, bytes, sha256, prior));
        }
    }

    let ledger_entries: Vec<PendingSample> = to_write
        .iter()
        .map(|(name, _, sha256, _)| PendingSample {
            name: name.clone(),
            sha256: sha256.clone(),
        })
        .collect();
    let ledger_state = match (ledger, ledger_entries.is_empty()) {
        (Some(ledger), false) => {
            let replaced = ledger.add(&ledger_entries)?;
            let names = ledger_entries.into_iter().map(|entry| entry.name).collect();
            Some((ledger.clone(), names, replaced))
        }
        _ => None,
    };

    let mut guard = CreatedKeysoundsGuard {
        created: Vec::with_capacity(to_write.len()),
        ledger: ledger_state,
    };
    let mut written_names = Vec::with_capacity(to_write.len());
    let policy = BackupPolicy::new(backup_root.to_path_buf(), 20);
    for (name, bytes, _, prior) in to_write {
        let wav_path = folder.join(&name);
        if let Some(ref old_bytes) = prior {
            match replace(&wav_path, old_bytes, &bytes, &policy)
                .map_err(|e| format!("Updating {name}: {e}"))?
            {
                dawsu_foundation::safe_write::WriteOutcome::Saved { .. } => {}
                dawsu_foundation::safe_write::WriteOutcome::ChangedOnDisk => {
                    return Err(format!("{name} changed on disk during write. Try again."));
                }
            }
        } else {
            safe_write::create(&wav_path, &bytes).map_err(|e| format!("Writing {name}: {e}"))?;
        }
        guard.created.push((wav_path, bytes, prior));
        written_names.push(name);
    }

    plan.remap_sample_filenames(&renames);
    Ok((written_names, guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_osu_format::OsuFile;
    use dawsu_test_support::Scratch;
    use std::fs;

    fn poll_until_idle(app: &mut Application) {
        while app.busy {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    fn setup_mapset(name: &str, hit_objects: &str) -> (Scratch, PathBuf, Application) {
        setup_mapset_with_timing(name, "0,500,4,1,0,100,1,0", hit_objects)
    }

    fn setup_mapset_with_timing(name: &str, timing: &str, hit_objects: &str) -> (Scratch, PathBuf, Application) {
        let scratch = Scratch::new(name);
        let map_dir = scratch.path("mapset");
        fs::create_dir_all(&map_dir).unwrap();
        let map_path = map_dir.join("Artist - Song (Mapper) [Hard].osu");
        let osu = format!(
            "osu file format v14\r\n[General]\r\nMode:0\r\n[Metadata]\r\nArtist:Artist\r\nTitle:Song\r\nCreator:Mapper\r\nVersion:Hard\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n[TimingPoints]\r\n{timing}\r\n[HitObjects]\r\n{hit_objects}"
        );
        fs::write(&map_path, osu.as_bytes()).unwrap();

        let mut app = Application::with_data_root(scratch.path("data"));
        app.error = None;
        app.request_open(map_path);
        poll_until_idle(&mut app);
        (scratch, map_dir, app)
    }

    #[test]
    fn piano_roll_crud_preview_and_generate_keysounds_difficulty() {
        let (_scratch, _map_dir, mut app) =
            setup_mapset("keysound-session", "256,192,1000,1,0,0:0:0:0:\r\n");

        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Piano" })).unwrap();
        let tid = res["tracks"][0]["id"].as_u64().unwrap();

        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1005.0, "endMs": 1205.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();
        handle(
            &mut app,
            &json!({ "kind": "quantize_notes", "trackId": tid, "stepMs": 125.0 }),
        )
        .unwrap();
        handle(
            &mut app,
            &json!({ "kind": "transpose_notes", "trackId": tid, "semitones": 2 }),
        )
        .unwrap();
        let res = handle(
            &mut app,
            &json!({ "kind": "preview_note", "trackId": tid, "pitch": 62, "velocity": 100 }),
        )
        .unwrap();
        // An audition answers with a small acknowledgement, not the whole summary.
        assert_eq!(res, json!({ "previewed": true }));
        let res = handle(&mut app, &json!({ "kind": "view" })).unwrap();
        assert_eq!(res["tracks"][0]["notes"][0]["pitch"], 62);
        assert_eq!(res["tracks"][0]["notes"][0]["startMs"], 1000.0);

        let result = handle(&mut app, &json!({ "kind": "generate_keysounds" })).unwrap();
        poll_until_idle(&mut app);
        let out_path = PathBuf::from(result["generated"]["path"].as_str().unwrap());
        assert!(out_path.is_file());
        assert_eq!(result["generated"]["syncedCount"], 1);
    }

    #[test]
    fn generate_keysounds_avoids_overwriting_existing_sample_with_different_bytes() {
        let (_scratch, map_dir, mut app) =
            setup_mapset("keysound-collision", "256,192,1000,1,0,0:0:0:0:\r\n");

        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Piano" })).unwrap();
        let tid = res["tracks"][0]["id"].as_u64().unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1000.0, "endMs": 1150.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();

        let doc = app.document.as_ref().unwrap();
        let expected_plan = plan_generate_keysounds(
            &app.keysounds.document,
            &doc.file,
            "Artist - Song (Mapper) [Hard].osu",
            &ExportSettings::default(),
        );
        assert_eq!(expected_plan.manifest.len(), 1);
        let expected_first_name = expected_plan.manifest[0].filename.clone();
        let preexisting_bytes = b"RIFFuser-preexisting-sample";
        let preexisting_path = map_dir.join(&expected_first_name);
        fs::write(&preexisting_path, preexisting_bytes).unwrap();

        let result = handle(&mut app, &json!({ "kind": "generate_keysounds" })).unwrap();
        poll_until_idle(&mut app);

        assert_eq!(fs::read(&preexisting_path).unwrap(), preexisting_bytes);
        let expected_fallback = numbered_sample_name(&expected_first_name, 2);
        let fallback_path = map_dir.join(&expected_fallback);
        assert!(fallback_path.is_file(), "expected fallback WAV {expected_fallback}");

        let out_path = PathBuf::from(result["generated"]["path"].as_str().unwrap());
        let generated_osu = OsuFile::read(fs::read(&out_path).unwrap()).file;
        let objects = generated_osu.hit_objects();
        let hit_sample_name = objects[0]
            .hit_sample()
            .and_then(|s| s.filename.as_deref());
        assert_eq!(hit_sample_name, Some(expected_fallback.as_str()));

        let ledger_names: Vec<String> = app
            .document
            .as_ref()
            .unwrap()
            .pending
            .ledger()
            .unwrap()
            .entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(ledger_names, vec![expected_fallback.clone()]);

        // Regenerating with the same note reuses the identical fallback file without creating _3.wav.
        let fallback_bytes = fs::read(&fallback_path).unwrap();
        handle(&mut app, &json!({ "kind": "generate_keysounds" })).unwrap();
        poll_until_idle(&mut app);
        assert_eq!(fs::read(&fallback_path).unwrap(), fallback_bytes);
        assert!(!map_dir.join(numbered_sample_name(&expected_first_name, 3)).exists());
    }

    #[test]
    fn regenerate_with_fewer_notes_records_and_sweeps_unused_keysound_wav() {
        let (scratch, map_dir, mut app) = setup_mapset(
            "keysound-sweep",
            "256,192,1000,1,0,0:0:0:0:\r\n256,192,1500,1,0,0:0:0:0:\r\n",
        );

        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Piano" })).unwrap();
        let tid = res["tracks"][0]["id"].as_u64().unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1000.0, "endMs": 1150.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1500.0, "endMs": 1650.0, "pitch": 64, "velocity": 100 }),
        )
        .unwrap();

        let first = handle(&mut app, &json!({ "kind": "generate_keysounds" })).unwrap();
        poll_until_idle(&mut app);
        assert_eq!(first["generated"]["manifestCount"], 2);

        let ledger = app.document.as_ref().unwrap().pending.ledger().unwrap();
        let initial_entries = ledger.entries().unwrap();
        assert_eq!(initial_entries.len(), 2);
        let kept_entry = initial_entries
            .iter()
            .find(|e| e.name.contains("_p060_"))
            .unwrap()
            .clone();
        let dropped_entry = initial_entries
            .iter()
            .find(|e| e.name.contains("_p064_"))
            .unwrap()
            .clone();
        assert!(map_dir.join(&kept_entry.name).is_file());
        assert!(map_dir.join(&dropped_entry.name).is_file());

        // Remove the second note (pitch 64) and regenerate with fewer notes.
        handle(&mut app, &json!({ "kind": "delete_note", "trackId": tid, "noteIndex": 1 })).unwrap();
        let second = handle(&mut app, &json!({ "kind": "generate_keysounds" })).unwrap();
        poll_until_idle(&mut app);
        assert_eq!(second["generated"]["manifestCount"], 1);

        assert!(map_dir.join(&kept_entry.name).is_file());
        assert!(
            !map_dir.join(&dropped_entry.name).exists(),
            "unused keysound WAV {} should have been swept from mapset",
            dropped_entry.name
        );
        assert!(
            scratch
                .path("data")
                .join("removed-samples")
                .join(format!("{}.wav", dropped_entry.sha256))
                .is_file(),
            "swept keysound WAV should be preserved in removed-samples"
        );
        assert!(app.status.contains("Removed 1 unused sample file"), "{}", app.status);

        let remaining_names: Vec<String> = ledger.entries().unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(remaining_names, vec![kept_entry.name]);
    }

    #[test]
    fn save_dawsuks_detects_external_disk_change_and_keeps_dirty() {
        let (scratch, _map_dir, mut app) =
            setup_mapset("keysound-save-race", "256,192,1000,1,0,0:0:0:0:\r\n");
        let proj_path = scratch.path("my.dawsuks");
        handle(&mut app, &json!({ "kind": "add_track", "name": "Piano" })).unwrap();
        assert!(app.is_dirty(), "keysound-only edit must mark app dirty");

        handle(&mut app, &json!({ "kind": "save_dawsuks", "path": proj_path.to_string_lossy() }))
            .unwrap();
        assert!(!app.is_dirty());

        // External process modifies the .dawsuks file on disk.
        let external = b"{\"version\":1,\"external\":true}";
        fs::write(&proj_path, external).unwrap();

        handle(&mut app, &json!({ "kind": "add_track", "name": "Bass" })).unwrap();
        assert!(app.is_dirty());

        let err = handle(&mut app, &json!({ "kind": "save_dawsuks" })).unwrap_err();
        assert!(err.contains("changed on disk"), "unexpected err: {err}");
        assert!(app.is_dirty(), "failed save must keep dirty flag true");
        assert_eq!(fs::read(&proj_path).unwrap(), external, "external bytes must stay intact");
    }

    #[test]
    fn switching_maps_resets_keysounds_and_auto_restores_bound_project() {
        let (scratch, map_dir, mut app) =
            setup_mapset("keysound-map-switch", "256,192,1000,1,0,0:0:0:0:\r\n");
        let map_a = app.document.as_ref().unwrap().path.clone();
        let map_b = map_dir.join("Artist - Song (Mapper) [Normal].osu");
        fs::write(
            &map_b,
            b"osu file format v14\r\n[General]\r\nMode:0\r\n[Metadata]\r\nArtist:Artist\r\nTitle:Song\r\nCreator:Mapper\r\nVersion:Normal\r\n[TimingPoints]\r\n0,500,4,1,0,100,1,0\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n",
        )
        .unwrap();

        // Add & save keysounds on map A via app.save() (keysound-only dirty save!).
        handle(&mut app, &json!({ "kind": "add_track", "name": "Map A Lead" })).unwrap();
        assert!(app.is_dirty());
        app.save();
        poll_until_idle(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(!app.is_dirty(), "app.save() should save dirty keysounds and clear dirty");

        // Switch to map B -> map A's tracks and project_path must NOT leak into map B!
        app.request_open(map_b.clone());
        poll_until_idle(&mut app);
        assert!(app.keysounds.document.tracks.is_empty());
        assert!(app.keysound_history.project_path.is_none());

        // Switch back to map A -> bound .dawsuks auto-restores cleanly!
        app.request_open(map_a);
        poll_until_idle(&mut app);
        assert_eq!(app.keysounds.document.tracks.len(), 1);
        assert_eq!(app.keysounds.document.tracks[0].name, "Map A Lead");
        assert!(!app.is_dirty());
        let _ = scratch;
    }

    #[test]
    fn sync_drum_track_never_deletes_user_track_with_same_display_name() {
        let (_scratch, _map_dir, mut app) =
            setup_mapset("keysound-drum-sync", "256,192,1000,1,0,0:0:0:0:\r\n");

        // User manually creates a track literally named "Drum Map (VST3)".
        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Drum Map (VST3)" })).unwrap();
        let user_tid = res["tracks"][0]["id"].as_u64().unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": user_tid, "startMs": 500.0, "endMs": 700.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();

        // Sync drum map -> must create a separate `is_drum_sync: true` track and leave user track intact!
        let synced = handle(
            &mut app,
            &json!({
                "kind": "sync_drum_track",
                "trackName": "Drum Map (VST3)",
                "notes": [{ "startMs": 1000, "endMs": 1120, "pitch": 45, "velocity": 110 }]
            }),
        )
        .unwrap();
        assert_eq!(synced["tracks"].as_array().unwrap().len(), 2);
        let user_track = app.keysounds.document.tracks.iter().find(|t| t.id == user_tid).unwrap();
        assert!(!user_track.is_drum_sync);
        assert_eq!(user_track.notes.len(), 1);
        assert_eq!(user_track.notes[0].pitch, 60);

        let drum_track = app.keysounds.document.tracks.iter().find(|t| t.is_drum_sync).unwrap();
        assert_eq!(drum_track.notes.len(), 1);
        assert_eq!(drum_track.notes[0].pitch, 45);

        // Single undo reverts the entire atomic drum sync in one step.
        handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(app.keysounds.document.tracks.len(), 1);
        assert_eq!(app.keysounds.document.tracks[0].id, user_tid);
    }

    #[test]
    fn wav_preview_bounce_and_generate_keysounds_match_normalization_and_duration() {
        let (_scratch, map_dir, mut app) =
            setup_mapset("keysound-wav-norm-parity", "256,192,1000,1,0,0:0:0:0:\r\n");
        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Lead" })).unwrap();
        let tid = res["tracks"][0]["id"].as_u64().unwrap();
        // 1200 ms sustained note (> old 500 ms cap!).
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1000.0, "endMs": 2200.0, "pitch": 60, "velocity": 64 }),
        )
        .unwrap();

        let preview = handle(&mut app, &json!({ "kind": "preview_bounce", "format": "wav16" })).unwrap();
        let preview_sample = &preview["bouncePreview"]["samples"][0];
        let preview_peak = preview_sample["peakDbfs"].as_f64().unwrap();
        let preview_dur = preview_sample["durationMs"].as_u64().unwrap();
        assert!(preview_dur >= 1000, "sustained note must not be capped at 500 ms, got {preview_dur} ms");

        let generated = handle(&mut app, &json!({ "kind": "generate_keysounds", "format": "wav16" })).unwrap();
        poll_until_idle(&mut app);
        let filename = preview_sample["filename"].as_str().unwrap();
        let wav_path = map_dir.join(filename);
        let decoded = dawsu_audio::decode(&wav_path).unwrap();
        let actual_peak = decoded.samples.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let actual_peak_dbfs = 20.0 * f64::from(actual_peak).log10();
        assert!(
            (actual_peak_dbfs - preview_peak).abs() < 0.2,
            "WAV preview ({preview_peak:.2} dBFS) and committed WAV ({actual_peak_dbfs:.2} dBFS) must match"
        );
        let _ = generated;
    }

    #[test]
    fn midi_record_commits_to_active_keysound_track_with_undo() {
        let (_scratch, _map_dir, mut app) =
            setup_mapset("keysound-midi-active", "256,192,1000,1,0,0:0:0:0:\r\n");
        let r1 = handle(&mut app, &json!({ "kind": "add_track", "name": "Track 1" })).unwrap();
        let t1 = r1["tracks"][0]["id"].as_u64().unwrap();
        let r2 = handle(&mut app, &json!({ "kind": "add_track", "name": "Track 2" })).unwrap();
        let t2 = r2["tracks"][1]["id"].as_u64().unwrap();
        handle(&mut app, &json!({ "kind": "select_track", "trackId": t2 })).unwrap();

        // Arm, inject one note, disarm via toggle_midi_recording.
        app.toggle_midi_recording(None, None, None).unwrap();
        app.midi_recorder.record_event(
            1000.0,
            dawsu_audio_instruments::MidiInputEvent::NoteOn {
                channel: 0,
                pitch: 64,
                velocity: 100,
            },
        );
        app.toggle_midi_recording(None, None, None).unwrap();

        assert_eq!(app.keysounds.document.tracks.iter().find(|t| t.id == t1).unwrap().notes.len(), 0);
        assert_eq!(app.keysounds.document.tracks.iter().find(|t| t.id == t2).unwrap().notes.len(), 1);
        assert!(app.keysound_history.can_undo());

        handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(app.keysounds.document.tracks.iter().find(|t| t.id == t2).unwrap().notes.len(), 0);
    }

    #[test]
    fn sfz_soundfont_loads_previews_and_bounces_keysounds_without_vst3_sidecar() {
        let (scratch, map_dir, mut app) =
            setup_mapset("keysound-sfz-bounce", "256,192,1000,1,0,0:0:0:0:\r\n");
        let sfz_dir = scratch.dir().join("sfz-bank");
        fs::create_dir_all(&sfz_dir).unwrap();
        let wav_path = sfz_dir.join("tone.wav");
        let mut stereo = Vec::with_capacity(4800 * 2);
        for i in 0..4800 {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin() * 0.7;
            stereo.push(v);
            stereo.push(v);
        }
        dawsu_audio_instruments::write_wav_i16(&wav_path, &stereo, 48_000, None).unwrap();
        let sfz_path = sfz_dir.join("lead.sfz");
        fs::write(&sfz_path, "<region> lokey=36 hikey=84 pitch_keycenter=69 sample=tone.wav\n").unwrap();

        let summary_inspected = app.inspect_soundfont_file(&sfz_path).unwrap();
        assert_eq!(summary_inspected.format, "sfz");
        assert_eq!(summary_inspected.region_count, 1);

        let r = handle(&mut app, &json!({ "kind": "add_track", "name": "SFZ Lead" })).unwrap();
        let tid = r["tracks"][0]["id"].as_u64().unwrap();
        let loaded = handle(
            &mut app,
            &json!({
                "kind": "load_soundfont",
                "trackId": tid,
                "path": sfz_path.to_string_lossy(),
                "presetIndex": 0
            }),
        )
        .unwrap();
        assert_eq!(loaded["tracks"][0]["isSoundfont"], json!(true));

        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1000.0, "endMs": 1150.0, "pitch": 69, "velocity": 110 }),
        )
        .unwrap();
        // Preview works cleanly without a VST3 sidecar.
        handle(&mut app, &json!({ "kind": "preview_note", "trackId": tid, "pitch": 69, "velocity": 100 })).unwrap();

        let preview = handle(&mut app, &json!({ "kind": "preview_bounce", "format": "wav16" })).unwrap();
        let filename = preview["bouncePreview"]["samples"][0]["filename"].as_str().unwrap().to_string();
        handle(&mut app, &json!({ "kind": "generate_keysounds", "format": "wav16" })).unwrap();
        poll_until_idle(&mut app);

        let bounced = dawsu_audio::decode(&map_dir.join(&filename)).unwrap();
        let peak = bounced.samples.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!(peak > 0.05, "SFZ bounced WAV must contain non-zero rendered audio, got peak {peak}");
    }

    #[test]
    fn live_tap_lane_and_toggle_tap_recording_commit_in_one_undo_step() {
        let (_scratch, _map_dir, mut app) =
            setup_mapset("live-tap-record", "256,192,1000,1,0,0:0:0:0:\r\n");
        app.reset_fresh_open_for_tests();
        let before = app.document().unwrap().composer.hitsounds().moments().len();

        // Tapping while unarmed auditions only and does not buffer or edit the map.
        app.seek(1122.0);
        assert!(app.tap_lane(0));
        assert_eq!(app.tap_recorded_count(), 0);
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), before);

        // Arm, tap two lanes at different playhead times, then disarm -> commits as one Undo step.
        let armed = app.toggle_tap_recording().unwrap();
        assert!(armed.armed);
        assert!(app.tap_recording_armed());

        app.seek(1122.0);
        assert!(app.tap_lane(0));
        app.seek(1495.0);
        assert!(app.tap_lane(1));
        assert_eq!(app.tap_recorded_count(), 2);

        let done = app.toggle_tap_recording().unwrap();
        assert!(!done.armed);
        assert_eq!(done.committed_taps, 2);
        assert_eq!(app.tap_recorded_count(), 0);
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), before + 2);

        app.command("undo");
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), before);
    }

    #[test]
    fn record_button_stops_tap_recording_armed_from_the_playback_menu() {
        let (_scratch, _map_dir, mut app) =
            setup_mapset("record-stops-tap", "256,192,1000,1,0,0:0:0:0:\r\n");
        app.reset_fresh_open_for_tests();
        let before = app.document().unwrap().composer.hitsounds().moments().len();

        // Playback › Tap record arms; the transport's Record button must stop and commit.
        assert!(app.toggle_tap_recording().unwrap().armed);
        app.seek(1122.0);
        assert!(app.tap_lane(0));
        let done = app.toggle_midi_recording(None, None, None).unwrap();
        assert!(!done.armed);
        assert_eq!(done.committed_notes, 1);
        assert!(!app.tap_recording_armed());
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), before + 1);
    }

    #[test]
    fn toggle_tap_disarm_commits_armed_midi_and_unarmed_taps_capture_retrospectively() {
        let (_scratch, _map_dir, mut app) =
            setup_mapset("retro-and-disarm", "256,192,1000,1,0,0:0:0:0:\r\n");
        app.reset_fresh_open_for_tests();
        let before = app.document().unwrap().composer.hitsounds().moments().len();

        // 1. Disarming via toggle_tap_recording when midi_recorder is armed commits armed MIDI notes.
        app.toggle_midi_recording(None, None, None).unwrap();
        app.midi_recorder.record_event(
            1122.0,
            dawsu_audio_instruments::MidiInputEvent::NoteOn {
                channel: 9,
                pitch: 36,
                velocity: 100,
            },
        );
        let done = app.toggle_tap_recording().unwrap();
        assert!(!done.armed);
        assert_eq!(done.committed_taps, 1);
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), before + 1);
        app.command("undo");
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), before);

        // Drain anything already in retrospective from step 1, then test unarmed capture.
        let _ = app.midi_recorder.take_retrospective();
        app.seek(1495.0);
        assert!(app.tap_lane(1));
        app.midi_recorder.record_retrospective_event(
            1998.0,
            dawsu_audio_instruments::MidiInputEvent::NoteOn {
                channel: 9,
                pitch: 36,
                velocity: 110,
            },
        );
        let captured = app.capture_retrospective(None, None).unwrap();
        assert_eq!(captured.committed_hits, 2);
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), before + 2);

        // Single Undo removes both the retrospective tap and MIDI hit together.
        app.command("undo");
        assert_eq!(app.document().unwrap().composer.hitsounds().moments().len(), before);
    }

    #[test]
    fn toggle_mute_and_toggle_solo_control_audibility_and_undo() {
        let (_scratch, _map_dir, mut app) = setup_mapset(
            "keysound-mute-solo",
            "256,192,1000,1,0,0:0:0:0:\r\n256,192,2000,1,0,0:0:0:0:\r\n",
        );

        let r1 = handle(&mut app, &json!({ "kind": "add_track", "name": "Lead" })).unwrap();
        let t1 = r1["tracks"][0]["id"].as_u64().unwrap();
        let r2 = handle(&mut app, &json!({ "kind": "add_track", "name": "Pad" })).unwrap();
        let t2 = r2["tracks"][1]["id"].as_u64().unwrap();

        assert_eq!(r2["tracks"][0]["muted"], json!(false));
        assert_eq!(r2["tracks"][0]["solo"], json!(false));
        assert_eq!(r2["tracks"][1]["muted"], json!(false));
        assert_eq!(r2["tracks"][1]["solo"], json!(false));

        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": t1, "startMs": 1000.0, "endMs": 1200.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": t2, "startMs": 2000.0, "endMs": 2200.0, "pitch": 64, "velocity": 100 }),
        )
        .unwrap();

        // Mute track 1 -> only track 2 is audible in preview_bounce.
        let muted = handle(&mut app, &json!({ "kind": "toggle_mute", "trackId": t1 })).unwrap();
        assert_eq!(muted["tracks"][0]["muted"], json!(true));
        assert_eq!(muted["tracks"][0]["solo"], json!(false));
        assert!(!is_track_audible(&app.keysounds.document, t1));
        assert!(is_track_audible(&app.keysounds.document, t2));
        let preview = handle(&mut app, &json!({ "kind": "preview_bounce", "format": "wav16" })).unwrap();
        assert_eq!(preview["bouncePreview"]["sourceNotes"], json!(1));

        // Undo unmutes track 1.
        let unmuted = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(unmuted["tracks"][0]["muted"], json!(false));
        assert!(is_track_audible(&app.keysounds.document, t1));

        // Solo track 1 -> only track 1 is audible.
        let soloed = handle(&mut app, &json!({ "kind": "toggle_solo", "trackId": t1 })).unwrap();
        assert_eq!(soloed["tracks"][0]["solo"], json!(true));
        assert_eq!(soloed["tracks"][1]["solo"], json!(false));
        assert!(is_track_audible(&app.keysounds.document, t1));
        assert!(!is_track_audible(&app.keysounds.document, t2));
        let preview = handle(&mut app, &json!({ "kind": "preview_bounce", "format": "wav16" })).unwrap();
        assert_eq!(preview["bouncePreview"]["sourceNotes"], json!(1));

        // Muting the only soloed track silences everything (soloed && !muted).
        handle(&mut app, &json!({ "kind": "toggle_mute", "trackId": t1 })).unwrap();
        assert!(!is_track_audible(&app.keysounds.document, t1));
        assert!(!is_track_audible(&app.keysounds.document, t2));
        let preview = handle(&mut app, &json!({ "kind": "preview_bounce", "format": "wav16" })).unwrap();
        assert_eq!(preview["bouncePreview"]["sourceNotes"], json!(0));

        // Two undos restore both mute and solo.
        let after_undo1 = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(after_undo1["tracks"][0]["muted"], json!(false));
        assert_eq!(after_undo1["tracks"][0]["solo"], json!(true));
        let after_undo2 = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(after_undo2["tracks"][0]["solo"], json!(false));
        assert!(is_track_audible(&app.keysounds.document, t1));
        assert!(is_track_audible(&app.keysounds.document, t2));
    }

    #[test]
    fn sync_to_objects_moves_notes_preserving_duration_and_supports_undo() {
        let (_scratch, _map_dir, mut app) = setup_mapset(
            "keysound-sync-to-objects",
            "256,192,1000,1,0,0:0:0:0:\r\n256,192,2000,1,0,0:0:0:0:\r\n",
        );

        let r = handle(&mut app, &json!({ "kind": "add_track", "name": "Piano" })).unwrap();
        let tid = r["tracks"][0]["id"].as_u64().unwrap();

        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1015.0, "endMs": 1265.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1980.0, "endMs": 2130.0, "pitch": 64, "velocity": 100 }),
        )
        .unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 3500.0, "endMs": 3700.0, "pitch": 67, "velocity": 100 }),
        )
        .unwrap();

        let synced = handle(
            &mut app,
            &json!({ "kind": "sync_to_objects", "trackId": tid, "maxShiftMs": 30.0 }),
        )
        .unwrap();
        assert_eq!(synced["syncResult"]["syncedCount"], json!(2));
        assert_eq!(synced["syncResult"]["unsyncedCount"], json!(1));
        assert_eq!(synced["dirty"], json!(true));
        assert_eq!(synced["canUndo"], json!(true));

        let notes = synced["tracks"][0]["notes"].as_array().unwrap();
        assert_eq!(notes[0]["startMs"], json!(1000.0));
        assert_eq!(notes[0]["endMs"], json!(1250.0));
        assert_eq!(notes[1]["startMs"], json!(2000.0));
        assert_eq!(notes[1]["endMs"], json!(2150.0));
        assert_eq!(notes[2]["startMs"], json!(3500.0));
        assert_eq!(notes[2]["endMs"], json!(3700.0));

        // Undo reverts all synced note positions in one step.
        let undone = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        let undone_notes = undone["tracks"][0]["notes"].as_array().unwrap();
        assert_eq!(undone_notes[0]["startMs"], json!(1015.0));
        assert_eq!(undone_notes[0]["endMs"], json!(1265.0));
        assert_eq!(undone_notes[1]["startMs"], json!(1980.0));
        assert_eq!(undone_notes[1]["endMs"], json!(2130.0));
    }

    #[test]
    fn trim_overlaps_shortens_overlapping_same_pitch_notes_and_supports_undo() {
        let (_scratch, _map_dir, mut app) = setup_mapset(
            "keysound-trim-overlaps",
            "256,192,1000,1,0,0:0:0:0:\r\n",
        );

        let r = handle(&mut app, &json!({ "kind": "add_track", "name": "Synth" })).unwrap();
        let tid = r["tracks"][0]["id"].as_u64().unwrap();

        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1000.0, "endMs": 1500.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1100.0, "endMs": 1400.0, "pitch": 64, "velocity": 100 }),
        )
        .unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": 1250.0, "endMs": 1600.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();

        let trimmed = handle(&mut app, &json!({ "kind": "trim_overlaps", "trackId": tid })).unwrap();
        assert_eq!(trimmed["dirty"], json!(true));
        assert_eq!(trimmed["canUndo"], json!(true));
        let notes = trimmed["tracks"][0]["notes"].as_array().unwrap();
        assert_eq!(notes[0]["startMs"], json!(1000.0));
        assert_eq!(notes[0]["endMs"], json!(1250.0));
        assert_eq!(notes[1]["startMs"], json!(1100.0));
        assert_eq!(notes[1]["endMs"], json!(1400.0));
        assert_eq!(notes[2]["startMs"], json!(1250.0));
        assert_eq!(notes[2]["endMs"], json!(1600.0));

        let undone = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        let undone_notes = undone["tracks"][0]["notes"].as_array().unwrap();
        assert_eq!(undone_notes[0]["startMs"], json!(1000.0));
        assert_eq!(undone_notes[0]["endMs"], json!(1500.0));
    }

    #[test]
    fn summary_emits_audible_sibling_track_notes_for_ghosts() {
        let (_scratch, _map_dir, mut app) = setup_mapset(
            "keysound-ghost-notes",
            "256,192,1000,1,0,0:0:0:0:\r\n",
        );

        let r1 = handle(&mut app, &json!({ "kind": "add_track", "name": "Lead" })).unwrap();
        let t1 = r1["tracks"][0]["id"].as_u64().unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": t1, "startMs": 500.0, "endMs": 750.0, "pitch": 60, "velocity": 100 }),
        )
        .unwrap();

        let r2 = handle(&mut app, &json!({ "kind": "add_track", "name": "Pad" })).unwrap();
        let t2 = r2["tracks"][1]["id"].as_u64().unwrap();
        handle(
            &mut app,
            &json!({ "kind": "add_note", "trackId": t2, "startMs": 1000.0, "endMs": 1500.0, "pitch": 64, "velocity": 90 }),
        )
        .unwrap();

        // Active track is t2, but t1 is audible so summary includes t1's notes too.
        let view = handle(&mut app, &json!({ "kind": "view" })).unwrap();
        assert_eq!(view["activeTrackId"], json!(t2));
        assert_eq!(view["tracks"][0]["notes"].as_array().unwrap().len(), 1);
        assert_eq!(view["tracks"][1]["notes"].as_array().unwrap().len(), 1);

        // Muting t1 suppresses its notes in summary.
        let muted = handle(&mut app, &json!({ "kind": "toggle_mute", "trackId": t1 })).unwrap();
        assert!(muted["tracks"][0]["notes"].as_array().unwrap().is_empty());
        assert_eq!(muted["tracks"][1]["notes"].as_array().unwrap().len(), 1);

        // Unmute t1, then solo t2 (active): sibling t1 is excluded by solo while active t2 stays visible.
        handle(&mut app, &json!({ "kind": "toggle_mute", "trackId": t1 })).unwrap();
        let solo = handle(&mut app, &json!({ "kind": "toggle_solo", "trackId": t2 })).unwrap();
        assert!(solo["tracks"][0]["notes"].as_array().unwrap().is_empty());
        assert_eq!(solo["tracks"][1]["notes"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn strum_humanise_and_velocity_ramp_mutate_with_single_undo_steps() {
        let (_scratch, _map_dir, mut app) = setup_mapset(
            "keysound-strum-humanise-ramp",
            "256,192,1000,1,0,0:0:0:0:\r\n",
        );

        let r = handle(&mut app, &json!({ "kind": "add_track", "name": "Keys" })).unwrap();
        let tid = r["tracks"][0]["id"].as_u64().unwrap();

        for pitch in [60u8, 64, 67] {
            handle(
                &mut app,
                &json!({ "kind": "add_note", "trackId": tid, "startMs": 1000.0, "endMs": 1250.0, "pitch": pitch, "velocity": 80 }),
            )
            .unwrap();
        }

        // 1. Strum
        let strummed = handle(
            &mut app,
            &json!({ "kind": "strum_notes", "trackId": tid, "spreadMs": 20.0, "direction": "up" }),
        )
        .unwrap();
        let notes = strummed["tracks"][0]["notes"].as_array().unwrap();
        assert_eq!(notes[0]["startMs"], json!(1000.0));
        assert_eq!(notes[1]["startMs"], json!(1020.0));
        assert_eq!(notes[2]["startMs"], json!(1040.0));

        // 2. Velocity ramp across [1000, 1040]
        let ramped = handle(
            &mut app,
            &json!({
                "kind": "velocity_ramp",
                "trackId": tid,
                "startMs": 1000.0,
                "endMs": 1040.0,
                "startVelocity": 40,
                "endVelocity": 120,
                "curve": 0.0
            }),
        )
        .unwrap();
        let rnotes = ramped["tracks"][0]["notes"].as_array().unwrap();
        assert_eq!(rnotes[0]["velocity"], json!(40));
        assert_eq!(rnotes[1]["velocity"], json!(80));
        assert_eq!(rnotes[2]["velocity"], json!(120));

        // Undo velocity ramp cleanly in one step.
        let undone_ramp = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(undone_ramp["tracks"][0]["notes"][0]["velocity"], json!(80));
        assert_eq!(undone_ramp["tracks"][0]["notes"][2]["velocity"], json!(80));

        // 3. Humanise with fixed seed & undo in one step.
        let humanised = handle(
            &mut app,
            &json!({ "kind": "humanise_notes", "trackId": tid, "timingMs": 5.0, "velocityRange": 10, "seed": 77 }),
        )
        .unwrap();
        let hnotes = humanised["tracks"][0]["notes"].as_array().unwrap();
        assert_ne!(hnotes, undone_ramp["tracks"][0]["notes"].as_array().unwrap());

        let undone_humanise = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(
            undone_humanise["tracks"][0]["notes"][1]["startMs"],
            json!(1020.0)
        );
        let undone_strum = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(
            undone_strum["tracks"][0]["notes"][2]["startMs"],
            json!(1000.0)
        );
    }

    fn add_test_note(app: &mut Application, tid: u64, start_ms: f64, end_ms: f64) -> Value {
        handle(
            app,
            &json!({ "kind": "add_note", "trackId": tid, "startMs": start_ms, "endMs": end_ms, "pitch": 60, "velocity": 100 }),
        )
        .unwrap()
    }

    #[test]
    fn summary_marks_each_note_synced_unsynced_or_not_considered() {
        let (_scratch, _map_dir, mut app) =
            setup_mapset("keysound-synced-flag", "256,192,1000,1,0,0:0:0:0:\r\n");
        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Lead" })).unwrap();
        let tid = res["tracks"][0]["id"].as_u64().unwrap();
        add_test_note(&mut app, tid, 1000.0, 1100.0);
        let res = add_test_note(&mut app, tid, 1500.0, 1600.0);
        assert_eq!(res["tracks"][0]["notes"][0]["synced"], json!(true), "a note on an object");
        assert_eq!(res["tracks"][0]["notes"][1]["synced"], json!(false), "a note 500 ms from any object");

        // The flags are what Generate then does.
        let result = handle(&mut app, &json!({ "kind": "generate_keysounds" })).unwrap();
        poll_until_idle(&mut app);
        assert_eq!(result["generated"]["syncedCount"], 1);
        assert_eq!(result["generated"]["unsyncedCount"], 1);

        // Generate never considers a muted track, so its notes are neither synced nor unsynced.
        let res = handle(&mut app, &json!({ "kind": "toggle_mute", "trackId": tid })).unwrap();
        assert_eq!(res["tracks"][0]["notes"][0]["synced"], Value::Null);
        assert_eq!(res["tracks"][0]["notes"][1]["synced"], Value::Null);
    }

    #[test]
    fn rename_track_changes_the_name_rejects_an_empty_one_and_undoes() {
        let (_scratch, _map_dir, mut app) =
            setup_mapset("keysound-rename-track", "256,192,1000,1,0,0:0:0:0:\r\n");
        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Lead" })).unwrap();
        let tid = res["tracks"][0]["id"].as_u64().unwrap();

        let res = handle(&mut app, &json!({ "kind": "rename_track", "trackId": tid, "name": "  Bass  " })).unwrap();
        assert_eq!(res["tracks"][0]["name"], "Bass");

        let err = handle(&mut app, &json!({ "kind": "rename_track", "trackId": tid, "name": "   " })).unwrap_err();
        assert_eq!(err, "Track name is empty");
        let err = handle(&mut app, &json!({ "kind": "rename_track", "trackId": tid + 99, "name": "X" })).unwrap_err();
        assert_eq!(err, "Keysound track not found");

        let res = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(res["tracks"][0]["name"], "Lead");
    }

    #[test]
    fn delete_notes_removes_several_in_one_undo_step() {
        let (_scratch, _map_dir, mut app) = setup_mapset_with_timing(
            "keysound-delete-notes",
            "0,500,4,1,0,100,1,0",
            "256,192,1000,1,0,0:0:0:0:
",
        );
        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Lead" })).unwrap();
        let tid = res["tracks"][0]["id"].as_u64().unwrap();
        for start in [0.0, 250.0, 500.0, 750.0] {
            add_test_note(&mut app, tid, start, start + 200.0);
        }
        // Out of order, repeated and out of range: each listed note goes once, the rest stay.
        let res = handle(&mut app, &json!({ "kind": "delete_notes", "trackId": tid, "noteIndices": [3, 1, 3, 9] })).unwrap();
        let starts: Vec<f64> = res["tracks"][0]["notes"].as_array().unwrap().iter()
            .map(|n| n["startMs"].as_f64().unwrap()).collect();
        assert_eq!(starts, vec![0.0, 500.0]);
        let res = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(res["tracks"][0]["notes"].as_array().unwrap().len(), 4, "one Undo brings both back");
        assert!(handle(&mut app, &json!({ "kind": "delete_notes", "trackId": tid })).is_err(), "noteIndices is required");
    }

    #[test]
    fn snap_notes_to_grid_follows_each_tempo_section_and_keeps_lengths() {
        // Second red line at 2150 ms with a 400 ms beat: its 1/4 grid is 2150, 2250, 2350, ...
        let (_scratch, _map_dir, mut app) = setup_mapset_with_timing(
            "keysound-snap-grid",
            "0,500,4,1,0,100,1,0\r\n2150,400,4,1,0,100,1,0",
            "256,192,1000,1,0,0:0:0:0:\r\n256,192,2150,1,0,0:0:0:0:\r\n",
        );
        let res = handle(&mut app, &json!({ "kind": "add_track", "name": "Lead" })).unwrap();
        let tid = res["tracks"][0]["id"].as_u64().unwrap();
        add_test_note(&mut app, tid, 1040.0, 1240.0);
        add_test_note(&mut app, tid, 2330.0, 2500.5);

        app.set_snap_options(true, 1, 4, dawsu_editor::tempo::SnapMode::Grid);
        let res = handle(&mut app, &json!({ "kind": "snap_notes_to_grid", "trackId": tid })).unwrap();
        let notes = &res["tracks"][0]["notes"];
        assert_eq!(notes[0]["startMs"], json!(1000.0), "first section: 125 ms grid from 0");
        assert_eq!(notes[0]["endMs"], json!(1200.0));
        assert_eq!(notes[1]["startMs"], json!(2350.0), "second section: 100 ms grid from 2150");
        assert_eq!(notes[1]["endMs"], json!(2520.5), "the length is kept");

        // One Undo puts both notes back.
        let res = handle(&mut app, &json!({ "kind": "undo" })).unwrap();
        assert_eq!(res["tracks"][0]["notes"][0]["startMs"], json!(1040.0));
        assert_eq!(res["tracks"][0]["notes"][1]["startMs"], json!(2330.0));

        // Objects mode snaps to the nearest object.
        app.set_snap_options(true, 1, 4, dawsu_editor::tempo::SnapMode::Objects);
        let res = handle(&mut app, &json!({ "kind": "snap_notes_to_grid", "trackId": tid })).unwrap();
        assert_eq!(res["tracks"][0]["notes"][0]["startMs"], json!(1000.0));
        assert_eq!(res["tracks"][0]["notes"][1]["startMs"], json!(2150.0));

        // With Snap off there is nothing to snap to.
        app.set_snap_options(false, 1, 4, dawsu_editor::tempo::SnapMode::Grid);
        let err = handle(&mut app, &json!({ "kind": "snap_notes_to_grid", "trackId": tid })).unwrap_err();
        assert_eq!(err, "Snap is off");
    }
}
