//! Keysounding Piano Roll editing, live note preview, VST3 instrument loading, and
//! `[Keysounds]` difficulty generation.

use crate::{Application, MidiPreviewNote};
use dawsu_audio::AudioClip;
use dawsu_foundation::safe_write::{BackupPolicy, replace, write_new};
use dawsu_keysounding::generate::plan_generate_keysounds;
use dawsu_keysounding::model::ExportSettings;
use dawsu_keysounding::{KeysoundNote, editing};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

fn synth_note_clip(pitch: u8, velocity: u8, duration_ms: f64) -> Arc<AudioClip> {
    let sample_rate = 48_000u32;
    let frames = ((duration_ms.clamp(40.0, 500.0) / 1000.0) * f64::from(sample_rate)) as usize;
    let freq = 440.0 * 2.0_f64.powf((f64::from(pitch) - 69.0) / 12.0);
    let amp = (f64::from(velocity.clamp(1, 127)) / 127.0) * 0.28;
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f64 / f64::from(sample_rate);
        let attack = (t / 0.005).min(1.0);
        let decay = (-t * 10.0).exp();
        let env = attack * decay;
        let phase = 2.0 * std::f64::consts::PI * freq * t;
        let s = (phase.sin() * 0.75 + (phase * 2.0).sin() * 0.25) * amp * env;
        let v = s as f32;
        samples.push(v);
        samples.push(v);
    }
    Arc::new(AudioClip {
        samples,
        channels: 2,
        sample_rate,
    })
}

pub(crate) fn summary(app: &mut Application) -> Value {
    let loaded = app.vst3.list_loaded();
    json!({
        "targetDifficulty": app.keysounds.target_difficulty,
        "sidecarAvailable": app.vst3.is_sidecar_available(),
        "tracks": app.keysounds.document.tracks.iter().map(|t| json!({
            "id": t.id,
            "name": t.name,
            "channel": t.channel,
            "isDrum": t.is_drum,
            "muted": t.muted,
            "noteCount": t.notes.len(),
            "instrument": t.instrument.name,
            "notes": t.notes.iter().enumerate().map(|(index, n)| json!({
                "index": index,
                "startMs": n.start_ms,
                "endMs": n.end_ms,
                "pitch": n.pitch,
                "velocity": n.velocity,
                "muted": n.muted,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
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

    match kind {
        "view" => {}
        "add_track" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Keysound Track");
            let plugin = payload.get("vst3Plugin").and_then(|v| v.as_str());
            app.keysounds_add_track(name, plugin);
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
            let document = app.document.as_ref().ok_or("Open a difficulty first.")?;
            let file_name = document
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("map.osu")
                .to_string();
            let folder = document
                .path
                .parent()
                .ok_or("Active beatmap has no parent folder.")?;
            let plan = plan_generate_keysounds(
                &app.keysounds.document,
                &document.file,
                &file_name,
                &ExportSettings::default(),
            );
            let out_path = folder.join(&plan.file_name);
            let bytes = plan.file.to_bytes();
            let backed_up = if out_path.exists() {
                let previous = std::fs::read(&out_path).map_err(|e| e.to_string())?;
                let policy = BackupPolicy::new(app.data_root.join("backups"), 20);
                replace(&out_path, &previous, &bytes, &policy)
                    .map_err(|e| e.to_string())?;
                true
            } else {
                write_new(&out_path, &bytes).map_err(|e| e.to_string())?;
                false
            };
            let synced_count = plan.sync.synced.len();
            let unsynced_count = plan.sync.unsynced.len();
            app.status = format!(
                "Generated {} ({} synced notes, {} manifest samples).",
                plan.file_name,
                synced_count,
                plan.manifest.len()
            );
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

    Ok(summary(app))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;
    use std::fs;

    #[test]
    fn piano_roll_crud_preview_and_generate_keysounds_difficulty() {
        let scratch = Scratch::new("keysound-session");
        let map_dir = scratch.path("mapset");
        fs::create_dir_all(&map_dir).unwrap();
        let map_path = map_dir.join("Artist - Song (Mapper) [Hard].osu");
        let osu = b"osu file format v14\r\n[General]\r\nMode:0\r\n[Metadata]\r\nArtist:Artist\r\nTitle:Song\r\nCreator:Mapper\r\nVersion:Hard\r\n[Difficulty]\r\nSliderMultiplier:1.4\r\nSliderTickRate:1\r\n[TimingPoints]\r\n0,500,4,1,0,100,1,0\r\n[HitObjects]\r\n256,192,1000,1,0,0:0:0:0:\r\n";
        fs::write(&map_path, osu).unwrap();

        let mut app = Application::with_data_root(scratch.path("data"));
        app.error = None;
        app.request_open(map_path.clone());
        while app.busy {
            app.poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

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
        let out_path = PathBuf::from(result["generated"]["path"].as_str().unwrap());
        assert!(out_path.is_file());
        assert_eq!(result["generated"]["syncedCount"], 1);
    }
}
