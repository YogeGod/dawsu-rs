//! Live MIDI input recording into hitsound lanes and keysound tracks.

use dawsu_audio_instruments::midi_in::{MidiInputEvent, MidiInputRouter, list_midi_input_ports};
use dawsu_editor::tempo::{SnapSpacing, TempoMap};
use dawsu_editor_composer::{BatchHit, BatchOptions, Composer, Policy};
use dawsu_foundation::{SampleIdentity, SongTime};
use dawsu_hitsounding::drums::{DrumMap, render_notes};
use dawsu_keysounding::{KeysoundNote, KeysoundProject};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::mpsc;

/// One captured MIDI Note-On event timestamped against the editor playback clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordedMidiNote {
    pub time_ms: f64,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
}

/// Summary returned to the UI when toggling or querying MIDI recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiRecordStatus {
    pub armed: bool,
    pub port_name: Option<String>,
    pub buffered_notes: usize,
    pub committed_notes: usize,
}

/// Manages live MIDI input capture for the active editor session.
pub struct MidiRecorder {
    pub armed: bool,
    pub port_index: Option<usize>,
    pub port_name: Option<String>,
    router: Option<MidiInputRouter>,
    rx: Option<mpsc::Receiver<MidiInputEvent>>,
    recorded: Vec<RecordedMidiNote>,
}

impl Default for MidiRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiRecorder {
    pub fn new() -> Self {
        Self {
            armed: false,
            port_index: None,
            port_name: None,
            router: None,
            rx: None,
            recorded: Vec::new(),
        }
    }

    /// Arms MIDI recording. If a hardware MIDI port is available at `port_index` (or port `0`),
    /// opens it via `midir`; otherwise arms cleanly in software/virtual mode.
    pub fn arm(&mut self, port_index: Option<usize>) -> Result<Option<String>, String> {
        self.recorded.clear();
        let ports = list_midi_input_ports();
        let chosen_idx = port_index.or(if ports.is_empty() { None } else { Some(0) });
        if let Some(idx) = chosen_idx {
            if let Some(name) = ports.get(idx).cloned() {
                let mut router = MidiInputRouter::new();
                let (tx, rx) = mpsc::channel();
                router.connect_port_channel(&name, tx)?;
                self.router = Some(router);
                self.rx = Some(rx);
                self.port_index = Some(idx);
                self.port_name = Some(name.clone());
                self.armed = true;
                return Ok(Some(name));
            }
        }
        self.router = None;
        self.rx = None;
        self.port_index = None;
        self.port_name = None;
        self.armed = true;
        Ok(None)
    }

    /// Disarms MIDI recording, closes any open input port, and returns the captured notes.
    pub fn disarm(&mut self) -> Vec<RecordedMidiNote> {
        self.armed = false;
        self.router = None;
        self.rx = None;
        std::mem::take(&mut self.recorded)
    }

    pub fn buffered_count(&self) -> usize {
        self.recorded.len()
    }

    /// Drains live MIDI input events. Captures `NoteOn` timestamps when `armed && playing`.
    pub fn poll(&mut self, position_ms: f64, playing: bool) -> Vec<MidiInputEvent> {
        let Some(rx) = self.rx.as_ref() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        if self.armed && playing {
            for &ev in &events {
                if let MidiInputEvent::NoteOn {
                    channel,
                    pitch,
                    velocity,
                } = ev
                {
                    if velocity > 0 {
                        self.recorded.push(RecordedMidiNote {
                            time_ms: position_ms.max(0.0),
                            pitch,
                            velocity,
                            channel,
                        });
                    }
                }
            }
        }
        events
    }

    /// Records a simulated or virtual MIDI event at `time_ms` when armed.
    pub fn record_event(&mut self, time_ms: f64, event: MidiInputEvent) {
        if !self.armed {
            return;
        }
        if let MidiInputEvent::NoteOn {
            channel,
            pitch,
            velocity,
        } = event
        {
            if velocity > 0 {
                self.recorded.push(RecordedMidiNote {
                    time_ms: time_ms.max(0.0),
                    pitch,
                    velocity,
                    channel,
                });
            }
        }
    }
}

fn quantize_ms(tempo: &TempoMap, time_ms: f64, snap_divisor: Option<u32>) -> f64 {
    let Some(divisor) = snap_divisor.filter(|d| *d > 0) else {
        return time_ms;
    };
    let Ok(spacing) = SnapSpacing::fraction(divisor) else {
        return time_ms;
    };
    tempo.snap(SongTime::from_ms(time_ms), spacing).ms().max(0.0)
}

fn map_pitch_to_lane(
    pitch: u8,
    note_to_lane: &HashMap<u8, SampleIdentity>,
) -> Option<SampleIdentity> {
    note_to_lane.get(&pitch).cloned()
}

/// Commits recorded MIDI notes into `composer` as a single `"Record MIDI"` undo step.
pub fn commit_to_composer(
    notes: &[RecordedMidiNote],
    composer: &mut Composer,
    drum_map: Option<&DrumMap>,
    snap_divisor: Option<u32>,
) -> Result<usize, String> {
    if notes.is_empty() {
        return Ok(0);
    }
    let real_lanes = composer.hitsounds().lanes();
    let mut note_to_lane: HashMap<u8, SampleIdentity> = render_notes(drum_map, real_lanes)
        .into_iter()
        .map(|(identity, midi_note)| (midi_note, identity))
        .collect();
    // Map remaining visible lanes sequentially from middle C (60) upward (tdd-application.md Resp 35).
    let mut next_note: u8 = 60;
    for lane in &composer.lanes {
        if note_to_lane.values().any(|existing| existing == lane) {
            continue;
        }
        while next_note < 127 && note_to_lane.contains_key(&next_note) {
            next_note += 1;
        }
        if !note_to_lane.contains_key(&next_note) {
            note_to_lane.insert(next_note, lane.clone());
            next_note = next_note.saturating_add(1);
        }
    }

    let mut hits = Vec::with_capacity(notes.len());
    for note in notes {
        let Some(lane) = map_pitch_to_lane(note.pitch, &note_to_lane) else {
            continue;
        };
        let time_ms = quantize_ms(&composer.tempo, note.time_ms, snap_divisor);
        let strength = (f64::from(note.velocity) / 127.0).clamp(0.05, 1.0);
        let grid = snap_divisor
            .filter(|d| *d > 0)
            .map(|d| composer.grid_line(time_ms, d).0);
        hits.push(BatchHit {
            time: time_ms,
            lane,
            strength,
            grid,
        });
    }
    if hits.is_empty() {
        return Ok(0);
    }
    let options = BatchOptions {
        object_ms: 0.0,
        dedupe_ms: 2.0,
        shift_ms: 0.0,
        range: None,
        policy: Policy::Classic {
            filter_grid: false,
            include_unmatched: true,
        },
    };
    let report = composer
        .add_batch("Record MIDI", &hits, &options)
        .map_err(|refusal| format!("Record MIDI failed: {refusal:?}"))?;
    Ok(report.counts.added)
}

/// Commits recorded pitched MIDI notes into a [`KeysoundProject`] track.
pub fn commit_to_keysounds(
    notes: &[RecordedMidiNote],
    project: &mut KeysoundProject,
    track_id: u64,
    default_duration_ms: f64,
) -> usize {
    let Some(track) = project.document.tracks.iter_mut().find(|t| t.id == track_id) else {
        return 0;
    };
    let dur = default_duration_ms.max(20.0);
    let mut added = 0usize;
    for note in notes {
        track.notes.push(KeysoundNote::new(
            note.time_ms,
            note.time_ms + dur,
            note.pitch,
            note.velocity.clamp(1, 127),
        ));
        added += 1;
    }
    track.sort_notes();
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_hitsounding::{Decompiled, decompile};
    use dawsu_osu_format::OsuFile;

    #[test]
    fn simulated_midi_recording_quantizes_commits_one_undo_step_and_captures_keysounds() {
        let map = "osu file format v14\n[General]\nMode:0\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n";
        let osu = OsuFile::read(map.as_bytes().to_vec()).file;
        let hitsounds = match decompile(&osu, &[], &|| false, &mut |_, _| {}) {
            Decompiled::Done(hs) => hs,
            _ => panic!("minimal osu file must decompile"),
        };
        let mut composer = Composer::new(hitsounds);
        composer.tempo = TempoMap::new(vec![dawsu_editor::tempo::TimingSection {
            start: SongTime::from_ms(0.0),
            beat_length_ms: 500.0,
            beats_per_bar: 4,
        }]);

        let mut rec = MidiRecorder::new();
        rec.arm(None).unwrap();
        assert!(rec.armed);
        // 120 BPM = 500 ms/beat; 1/4 snap = 125 ms. 1120 ms snaps to 1125 ms.
        rec.record_event(
            1120.0,
            MidiInputEvent::NoteOn {
                channel: 9,
                pitch: 36,
                velocity: 100,
            },
        );
        rec.record_event(
            1490.0,
            MidiInputEvent::NoteOn {
                channel: 9,
                pitch: 38,
                velocity: 80,
            },
        );
        // Velocity 0 NoteOn is ignored as NoteOff.
        rec.record_event(
            1600.0,
            MidiInputEvent::NoteOn {
                channel: 9,
                pitch: 36,
                velocity: 0,
            },
        );

        let captured = rec.disarm();
        assert!(!rec.armed);
        assert_eq!(captured.len(), 2);

        let before = composer.hitsounds().moments().len();
        let added = commit_to_composer(&captured, &mut composer, None, Some(4)).unwrap();
        assert_eq!(added, 2);
        assert_eq!(composer.hitsounds().moments().len(), before + 2);
        let times: Vec<f64> = composer
            .hitsounds()
            .moments()
            .iter()
            .map(|m| m.time().ms())
            .collect();
        assert!(times.iter().any(|t| (*t - 1125.0).abs() < 1e-6));
        assert!(times.iter().any(|t| (*t - 1500.0).abs() < 1e-6));

        composer.undo();
        assert_eq!(composer.hitsounds().moments().len(), before);

        // Also commits pitched notes to a KeysoundProject track.
        let mut ks = KeysoundProject::new("audio.mp3", "Keysounds");
        let tid = ks.document.add_track("Piano", 0, false);
        assert_eq!(commit_to_keysounds(&captured, &mut ks, tid, 200.0), 2);
        assert_eq!(ks.document.tracks[0].notes.len(), 2);
    }
}
