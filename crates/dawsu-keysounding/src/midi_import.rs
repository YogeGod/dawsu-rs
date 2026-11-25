//! Import [`TimedMidi`] tracks into [`KeysoundTrack`]s and [`KeysoundDocument`]s.
//!
//! Follows TDD Responsibility 7 (one track per MIDI track and channel) and Invariant 6
//! (importing the same MIDI file twice gives equal tracks), with General MIDI channel 9
//! (0-indexed, i.e. MIDI Channel 10) automatically flagged as a drum track.

use crate::model::{
    EditorTimeSignature, InstrumentSettings, KeysoundDocument, KeysoundNote, KeysoundTrack,
    SustainSpan, TempoPoint, DEFAULT_TICKS_PER_QUARTER,
};
use dawsu_foundation::{TimeDivision, TimedMidi};
use std::collections::BTreeMap;

/// General MIDI percussion channel (0-indexed; Channel 10 in 1-based MIDI numbering).
pub const MIDI_DRUM_CHANNEL: u8 = 9;

/// Imports a [`TimedMidi`] into a deterministic list of [`KeysoundTrack`]s, splitting each
/// MIDI track by channel so there is one track per `(midi_track_index, channel)`.
pub fn import_midi_tracks(timed: &TimedMidi) -> Vec<KeysoundTrack> {
    let mut grouped: BTreeMap<(usize, u8), Vec<KeysoundNote>> = BTreeMap::new();
    let mut first_program: BTreeMap<(usize, u8), u8> = BTreeMap::new();
    let mut sustain_by_track: BTreeMap<(usize, u8), Vec<SustainSpan>> = BTreeMap::new();

    for track in &timed.tracks {
        for pc in &track.program_changes {
            first_program.entry((track.index, pc.channel & 0x0F)).or_insert(pc.program);
        }
        // Pair CC64 sustain press/release events per channel into display spans.
        let mut active_down: BTreeMap<u8, f64> = BTreeMap::new();
        for ev in &track.sustain_events {
            let ch = ev.channel & 0x0F;
            if ev.pressed {
                active_down.entry(ch).or_insert(ev.time_ms);
            } else if let Some(start_ms) = active_down.remove(&ch) {
                if ev.time_ms > start_ms {
                    sustain_by_track.entry((track.index, ch)).or_default().push(SustainSpan {
                        start_ms,
                        end_ms: ev.time_ms,
                    });
                }
            }
        }
        // Close any CC64 sustain held until end-of-track at that channel's last note end.
        for (ch, start_ms) in active_down {
            let max_end_ms = track
                .notes
                .iter()
                .filter(|n| (n.channel & 0x0F) == ch)
                .map(|n| n.end_ms)
                .fold(start_ms, f64::max);
            if max_end_ms > start_ms {
                sustain_by_track.entry((track.index, ch)).or_default().push(SustainSpan {
                    start_ms,
                    end_ms: max_end_ms,
                });
            }
        }
        for note in &track.notes {
            let ch = note.channel & 0x0F;
            grouped.entry((track.index, ch)).or_default().push(KeysoundNote {
                start_ms: note.start_ms,
                end_ms: note.end_ms.max(note.start_ms),
                pitch: note.pitch.min(127),
                velocity: note.velocity.clamp(1, 127),
                muted: false,
                split_group: None,
            });
        }
    }

    // Count how many channels appear in each MIDI track so single-channel tracks keep their clean name.
    let mut channels_per_midi_track: BTreeMap<usize, usize> = BTreeMap::new();
    for &(track_idx, _) in grouped.keys() {
        *channels_per_midi_track.entry(track_idx).or_insert(0) += 1;
    }

    let mut out = Vec::with_capacity(grouped.len());
    for (ordinal, ((track_idx, channel), mut notes)) in grouped.into_iter().enumerate() {
        let is_drum = channel == MIDI_DRUM_CHANNEL;
        let base_name = timed
            .tracks
            .get(track_idx)
            .and_then(|t| t.name.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let multi_channel = channels_per_midi_track.get(&track_idx).copied().unwrap_or(1) > 1;
        let name = match base_name {
            Some(name) if multi_channel => format!("{name} (Ch {})", channel + 1),
            Some(name) => name.to_string(),
            None if is_drum => format!("Drums (Ch {})", channel + 1),
            None => format!("Track {} (Ch {})", track_idx + 1, channel + 1),
        };

        let program = first_program.get(&(track_idx, channel)).copied();
        notes.sort_by(|a, b| {
            a.start_ms
                .total_cmp(&b.start_ms)
                .then_with(|| a.pitch.cmp(&b.pitch))
                .then_with(|| a.end_ms.total_cmp(&b.end_ms))
        });

        out.push(KeysoundTrack {
            id: (ordinal as u64) + 1,
            name: name.clone(),
            channel,
            is_drum,
            is_drum_sync: false,
            muted: false,
            solo: false,
            instrument: InstrumentSettings {
                name,
                program,
                channel,
                is_drum,
                soundfont_path: None,
                vst3_plugin_uid: None,
                vst3_state_b64: None,
                gain_db: 0.0,
            },
            notes,
            sustain_spans: sustain_by_track.remove(&(track_idx, channel)).unwrap_or_default(),
        });
    }

    out
}

/// Imports a [`TimedMidi`] into a complete [`KeysoundDocument`] including its tempo map and
/// editor-only time signatures.
pub fn import_timed_midi(timed: &TimedMidi) -> KeysoundDocument {
    let ticks_per_quarter = match timed.division {
        TimeDivision::TicksPerQuarter(tpq) => tpq,
        TimeDivision::Smpte { .. } => DEFAULT_TICKS_PER_QUARTER,
    };

    let tempo_points = timed
        .tempo_map
        .changes()
        .iter()
        .map(|c| TempoPoint {
            tick: c.tick,
            time_ms: c.time_ms,
            us_per_quarter: c.us_per_quarter,
        })
        .collect();

    let time_signatures = if timed.time_signatures.is_empty() {
        vec![EditorTimeSignature {
            time_ms: 0.0,
            numerator: 4,
            denominator: 4,
        }]
    } else {
        timed
            .time_signatures
            .iter()
            .map(|ts| EditorTimeSignature {
                time_ms: ts.time_ms,
                numerator: ts.signature.numerator,
                denominator: ts.signature.denominator,
            })
            .collect()
    };

    KeysoundDocument {
        tracks: import_midi_tracks(timed),
        ticks_per_quarter,
        tempo_points,
        time_signatures,
        bookmarks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::{
        MidiEvent, MidiEventKind, MidiFile, MidiFormat, MidiTrack, TimeSignature,
    };

    fn sample_multichannel_midi() -> TimedMidi {
        let track0 = MidiTrack {
            name: Some("Meta".into()),
            events: vec![
                MidiEvent { tick: 0, kind: MidiEventKind::SetTempo { us_per_quarter: 500_000 } },
                MidiEvent {
                    tick: 0,
                    kind: MidiEventKind::TimeSignature(TimeSignature {
                        numerator: 4,
                        denominator: 4,
                        clocks_per_click: 24,
                        thirty_seconds_per_quarter: 8,
                    }),
                },
                MidiEvent { tick: 0, kind: MidiEventKind::EndOfTrack },
            ],
        };

        let track1 = MidiTrack {
            name: Some("Keys & Kit".into()),
            events: vec![
                MidiEvent { tick: 0, kind: MidiEventKind::ProgramChange { channel: 0, program: 4 } },
                MidiEvent { tick: 0, kind: MidiEventKind::NoteOn { channel: 0, pitch: 60, velocity: 100 } },
                MidiEvent { tick: 480, kind: MidiEventKind::NoteOff { channel: 0, pitch: 60, velocity: 0 } },
                MidiEvent { tick: 0, kind: MidiEventKind::NoteOn { channel: 9, pitch: 36, velocity: 120 } },
                MidiEvent { tick: 240, kind: MidiEventKind::NoteOff { channel: 9, pitch: 36, velocity: 0 } },
                MidiEvent { tick: 480, kind: MidiEventKind::EndOfTrack },
            ],
        };

        let midi = MidiFile {
            format: MidiFormat::MultiTrack,
            division: TimeDivision::TicksPerQuarter(480),
            tracks: vec![track0, track1],
        };
        TimedMidi::from_midi(&midi)
    }

    #[test]
    fn imports_one_track_per_midi_track_and_channel_with_drum_detection() {
        let timed = sample_multichannel_midi();
        let doc = import_timed_midi(&timed);

        assert_eq!(doc.tracks.len(), 2);
        assert_eq!(doc.tracks[0].channel, 0);
        assert!(!doc.tracks[0].is_drum);
        assert_eq!(doc.tracks[0].instrument.program, Some(4));
        assert_eq!(doc.tracks[0].name, "Keys & Kit (Ch 1)");
        assert_eq!(doc.tracks[0].notes.len(), 1);
        assert_eq!(doc.tracks[0].notes[0].pitch, 60);

        assert_eq!(doc.tracks[1].channel, 9);
        assert!(doc.tracks[1].is_drum);
        assert!(doc.tracks[1].instrument.is_drum);
        assert_eq!(doc.tracks[1].name, "Keys & Kit (Ch 10)");
        assert_eq!(doc.tracks[1].notes.len(), 1);
        assert_eq!(doc.tracks[1].notes[0].pitch, 36);
    }

    #[test]
    fn importing_same_midi_twice_gives_equal_tracks() {
        let timed = sample_multichannel_midi();
        let first = import_midi_tracks(&timed);
        let second = import_midi_tracks(&timed);
        assert_eq!(first, second);
    }
}
