//! Keysounding Piano Roll editing, live note preview, VST3 instrument loading, and
//! `[Keysounds]` difficulty generation.

use crate::pending::{Ledger, PendingSample};
use crate::bounce_plan::{
    BounceAudioFormat, calculate_osz_budget, encode_vorbis_ogg, uniform_batch_normalize,
};
use crate::{Application, MidiPreviewNote};
use dawsu_audio::AudioClip;
use dawsu_foundation::digest::sha256_hex;
use dawsu_foundation::safe_write::{self, BackupPolicy, replace};
use dawsu_keysounding::chords::{
    ChordResolution, detect_chords, resolve_all_chords, resolve_chord,
};
use dawsu_keysounding::generate::{GeneratePlan, plan_generate_keysounds};
use dawsu_keysounding::model::{ExportSettings, KeysoundDocument};
use dawsu_keysounding::{KeysoundNote, editing};
use dawsu_osu_format::{check_map_path, find_case_insensitive, list_folder};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_KEYSOUND_UNDO: usize = 100;
const MAX_DAWSUKS_BYTES: u64 = 16 * 1024 * 1024;

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

    pub fn mark_saved(&mut self, path: PathBuf) {
        self.project_path = Some(path);
        self.saved_cursor = Some(self.cursor);
        self.dirty = false;
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

fn default_dawsuks_path(app: &Application) -> Option<PathBuf> {
    let doc = app.document.as_ref()?;
    let key = sha256_hex(doc.path.to_string_lossy().as_bytes());
    let dir = app.data_root.join("keysounds-projects");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join(format!("{}.dawsuks", &key[..16])))
}

fn synth_chord_clip(pitches: &[u8], velocity: u8, duration_ms: f64) -> Arc<AudioClip> {
    let notes: &[u8] = if pitches.is_empty() { &[60] } else { pitches };
    let sample_rate = 48_000u32;
    let frames = ((duration_ms.clamp(40.0, 500.0) / 1000.0) * f64::from(sample_rate)) as usize;
    let amp = (f64::from(velocity.clamp(1, 127)) / 127.0) * (0.28 / (notes.len() as f64).sqrt());
    let freqs: Vec<f64> = notes
        .iter()
        .map(|&p| 440.0 * 2.0_f64.powf((f64::from(p.min(127)) - 69.0) / 12.0))
        .collect();
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f64 / f64::from(sample_rate);
        let attack = (t / 0.005).min(1.0);
        let decay = (-t * 10.0).exp();
        let env = attack * decay;
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

fn synth_note_clip(pitch: u8, velocity: u8, duration_ms: f64) -> Arc<AudioClip> {
    synth_chord_clip(&[pitch], velocity, duration_ms)
}

pub(crate) fn summary(app: &mut Application) -> Value {
    let loaded = app.vst3.list_loaded();
    let active_id = app
        .keysound_history
        .active_track_id
        .filter(|id| app.keysounds.document.tracks.iter().any(|t| t.id == *id))
        .or_else(|| app.keysounds.document.tracks.first().map(|t| t.id));
    app.keysound_history.active_track_id = active_id;
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
            json!({
                "id": t.id,
                "name": t.name,
                "channel": t.channel,
                "isDrum": t.is_drum,
                "muted": t.muted,
                "noteCount": t.notes.len(),
                "instrument": t.instrument.name,
                "sustainSpans": if is_active { json!(t.sustain_spans) } else { json!([]) },
                "notes": if is_active {
                    t.notes.iter().enumerate().map(|(index, n)| json!({
                        "index": index,
                        "startMs": n.start_ms,
                        "endMs": n.end_ms,
                        "pitch": n.pitch,
                        "velocity": n.velocity,
                        "muted": n.muted,
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
            | "add_note"
            | "move_note"
            | "resize_note"
            | "delete_note"
            | "set_velocity"
            | "quantize_notes"
            | "transpose_notes"
            | "resolve_chord"
            | "resolve_all_chords"
    );
    let snapshot_before = mutating.then(|| app.keysounds.document.clone());

    match kind {
        "view" => {}
        "select_track" => {
            app.keysound_history.active_track_id = track_id;
        }
        "undo" => {
            app.keysound_history.undo(&mut app.keysounds.document);
        }
        "redo" => {
            app.keysound_history.redo(&mut app.keysounds.document);
        }
        "save_dawsuks" => {
            let path = match payload.get("path").and_then(|v| v.as_str()) {
                Some(raw) => {
                    let p = PathBuf::from(raw);
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
            if path.exists() {
                let prev = std::fs::read(&path).map_err(|e| e.to_string())?;
                let policy = BackupPolicy::new(app.data_root.join("backups"), 10);
                let _ = replace(&path, &prev, json_text.as_bytes(), &policy).map_err(|e| e.to_string())?;
            } else {
                safe_write::create(&path, json_text.as_bytes()).map_err(|e| e.to_string())?;
            }
            app.keysound_history.mark_saved(path);
        }
        "load_dawsuks" => {
            let path = payload
                .get("path")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
                .ok_or("Missing .dawsuks path")?;
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
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let loaded = dawsu_keysounding::KeysoundProject::from_json(&text).map_err(|e| e.to_string())?;
            app.keysounds = loaded;
            app.keysound_history = KeysoundUndoHistory {
                project_path: Some(path),
                active_track_id: app.keysounds.document.tracks.first().map(|t| t.id),
                ..KeysoundUndoHistory::default()
            };
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
            let document = app.document.as_ref().ok_or("Open a difficulty first.")?;
            let settings = export_settings_from_payload(payload);
            let file_name = document
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("map.osu");
            let plan = plan_generate_keysounds(&app.keysounds.document, &document.file, file_name, &settings);
            let base_bytes = document
                .path
                .parent()
                .map(mapset_disk_bytes)
                .unwrap_or(0);
            let is_ogg = settings.sample_extension.eq_ignore_ascii_case("ogg");
            let target_dbfs = if is_ogg {
                BounceAudioFormat::OggVorbis.target_peak_dbfs()
            } else {
                BounceAudioFormat::Wav16.target_peak_dbfs()
            };
            let mut raw_clips: Vec<Vec<f32>> = plan
                .manifest
                .iter()
                .map(|entry| synth_chord_clip(&entry.chord_pitches, entry.velocity, entry.duration_ms).samples.clone())
                .collect();
            let norm = uniform_batch_normalize(&mut raw_clips, target_dbfs);
            let mut bounced_bytes = 0u64;
            let mut samples_json = Vec::with_capacity(plan.manifest.len());
            // Encode exact Ogg bytes for small manifests; estimate bitrate cleanly for large sets
            // so preview never stalls the editor thread.
            let exact_ogg_preview = is_ogg && plan.manifest.len() <= 8;
            for (idx, entry) in plan.manifest.iter().enumerate() {
                let bytes = if exact_ogg_preview {
                    encode_vorbis_ogg(&raw_clips[idx], 48_000).map(|b| b.len() as u64).unwrap_or(0)
                } else if is_ogg {
                    ((entry.duration_ms / 1000.0) * 16_000.0).round().max(1_400.0) as u64
                } else {
                    44 + (raw_clips[idx].len() as u64) * 2
                };
                bounced_bytes = bounced_bytes.saturating_add(bytes);
                let silent = norm.silent_clip_indices.contains(&idx);
                let peak = raw_clips[idx].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
                let peak_dbfs = if peak > 1e-6 { 20.0 * peak.log10() } else { -99.0 };
                samples_json.push(json!({
                    "filename": entry.filename,
                    "durationMs": entry.duration_ms.round() as u32,
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
                app.keysounds.document.delete_track(id);
            }
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
            let loaded = app
                .vst3
                .load_instrument(PathBuf::from(plugin_path), plugin_uid)
                .map_err(|e| e.to_string())?;
            if let Some(track) = app.keysounds.document.tracks.iter_mut().find(|t| t.id == id) {
                track.instrument.name = format!("{plugin_name} ({})", loaded.id);
            }
        }
        "preview_note" => {
            let pitch = payload.get("pitch").and_then(|v| v.as_u64()).unwrap_or(60).min(127) as u8;
            let velocity = payload.get("velocity").and_then(|v| v.as_u64()).unwrap_or(100).clamp(1, 127) as u8;
            let duration_ms = payload.get("durationMs").and_then(|v| v.as_f64()).unwrap_or(140.0);
            let loaded_id = app.vst3.list_loaded().into_iter().next().map(|p| p.id);
            if let Some(instance_id) = loaded_id {
                let _ = app.vst3.send_preview_note(
                    &instance_id,
                    MidiPreviewNote {
                        channel: 0,
                        note: pitch,
                        velocity,
                        duration_ms: duration_ms.clamp(20.0, 4000.0).round() as u32,
                    },
                );
            }
            if let Some(audio) = app.audio.as_ref() {
                audio.audition(synth_note_clip(pitch, velocity, duration_ms), 0.8);
            }
        }
        "generate_keysounds" => {
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
            let apply_batch_norm = payload
                .get("normalize")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| {
                    payload
                        .get("format")
                        .and_then(|v| v.as_str())
                        .is_some_and(|f| f == "ogg_vorbis" || f == "ogg")
                });
            // Render and write each manifest sample before writing [Keysounds].osu so no dangling
            // sample references are ever emitted.
            let (written_names, guard) = prepare_and_write_wavs(
                &folder,
                &mut plan,
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
            return Ok(out);
        }
        other => return Err(format!("Unknown keysounds action: {other}")),
    }

    if let Some(before) = snapshot_before
        && app.keysounds.document != before
    {
        app.keysound_history.record(before);
    }

    Ok(summary(app))
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

struct CreatedKeysoundsGuard {
    created: Vec<(PathBuf, Vec<u8>)>,
    ledger: Option<(Ledger, Vec<String>, Vec<PendingSample>)>,
}

impl CreatedKeysoundsGuard {
    fn keep(mut self) {
        self.created.clear();
        self.ledger = None;
    }

    fn rollback(&mut self) {
        let mut still_ours = Vec::new();
        for (path, wav) in self.created.drain(..).rev() {
            let result = match std::fs::read(&path) {
                Ok(bytes) if bytes == wav => std::fs::remove_file(&path),
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
    plan: &mut GeneratePlan,
    pending_taken: &BTreeSet<String>,
    ledger: Option<&Ledger>,
    apply_batch_norm: bool,
) -> Result<(Vec<String>, CreatedKeysoundsGuard), String> {
    let disk_files = list_folder(folder).map_err(|e| e.to_string())?;
    let mut reserved_names = pending_taken.clone();
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    let mut to_write: Vec<(String, Vec<u8>, String)> = Vec::new();

    let mut raw_clips: Vec<Vec<f32>> = plan
        .manifest
        .iter()
        .map(|entry| {
            synth_chord_clip(&entry.chord_pitches, entry.velocity, entry.duration_ms)
                .samples
                .clone()
        })
        .collect();
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

    for (entry, samples) in plan.manifest.iter().zip(raw_clips.into_iter()) {
        let is_ogg = entry.filename.to_ascii_lowercase().ends_with(".ogg");
        let wav_bytes = if is_ogg {
            encode_vorbis_ogg(&samples, 48_000)?
        } else {
            let clip = AudioClip {
                samples,
                channels: 2,
                sample_rate: 48_000,
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
                    if std::fs::read(&existing_path)
                        .is_ok_and(|bytes| sha256_hex(&bytes) == digest)
                    {
                        return Some(Ok((existing_name.to_string(), None)));
                    }
                    return None;
                }
                let direct_path = folder.join(&candidate);
                if direct_path.symlink_metadata().is_ok() {
                    if std::fs::read(&direct_path)
                        .is_ok_and(|bytes| sha256_hex(&bytes) == digest)
                    {
                        return Some(Ok((candidate, None)));
                    }
                    return None;
                }
                if candidate_stem_taken_elsewhere(&candidate, &reserved_names, &disk_files) {
                    return None;
                }
                Some(Ok((candidate, Some((wav_bytes.clone(), digest.clone())))))
            })
            .ok_or_else(|| format!("No free sample filename for {}", entry.filename))??;

        let (chosen_name, write_payload) = chosen;
        reserved_names.insert(chosen_name.to_ascii_lowercase());
        if chosen_name != entry.filename {
            renames.insert(entry.filename.clone(), chosen_name.clone());
        }
        if let Some((bytes, sha256)) = write_payload {
            to_write.push((chosen_name, bytes, sha256));
        }
    }

    let ledger_entries: Vec<PendingSample> = to_write
        .iter()
        .map(|(name, _, sha256)| PendingSample {
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
    for (name, bytes, _) in to_write {
        let wav_path = folder.join(&name);
        safe_write::create(&wav_path, &bytes).map_err(|e| format!("Writing {name}: {e}"))?;
        guard.created.push((wav_path, bytes));
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
        let scratch = Scratch::new(name);
        let map_dir = scratch.path("mapset");
        fs::create_dir_all(&map_dir).unwrap();
        let map_path = map_dir.join("Artist - Song (Mapper) [Hard].osu");
        let osu = format!(
            "osu file format v14\r\n[General]\r\nMode:0\r\n[Metadata]\r\nArtist:Artist\r\nTitle:Song\r\nCreator:Mapper\r\nVersion:Hard\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n[TimingPoints]\r\n0,500,4,1,0,100,1,0\r\n[HitObjects]\r\n{hit_objects}"
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
}
