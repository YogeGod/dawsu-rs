//! Cross-platform live MIDI hardware input via `midir` (WinMM, CoreMIDI, ALSA).
//!
//! Domain-free: decodes raw MIDI 1.0 channel voice bytes into [`MidiInputEvent`]
//! and manages port enumeration and live connections via [`MidiInputRouter`].

use serde::{Deserialize, Serialize};
use std::sync::{Arc, mpsc};
use std::time::Instant;

/// A decoded live MIDI 1.0 channel voice event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MidiInputEvent {
    NoteOn { channel: u8, pitch: u8, velocity: u8 },
    NoteOff { channel: u8, pitch: u8 },
    ControlChange { channel: u8, controller: u8, value: u8 },
    /// Signed 14-bit pitch bend value in `-8192..=8191` (`0` is center).
    PitchBend { channel: u8, value: i16 },
}

/// A decoded MIDI event paired with its monotonic capture timestamp (`Instant::now()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampedMidiEvent {
    pub received_at: Instant,
    pub event: MidiInputEvent,
}

/// Decodes a raw MIDI 1.0 message into a [`MidiInputEvent`].
///
/// Normalizes `NoteOn` with velocity `0` into [`MidiInputEvent::NoteOff`].
/// Returns `None` for malformed, truncated, or unsupported messages (e.g. SysEx, clock).
pub fn decode_midi_bytes(msg: &[u8]) -> Option<MidiInputEvent> {
    let status = *msg.first()?;
    if status < 0x80 || status >= 0xF0 {
        return None;
    }
    let kind = status & 0xF0;
    let channel = status & 0x0F;
    match kind {
        0x80 => {
            if msg.len() < 3 {
                return None;
            }
            Some(MidiInputEvent::NoteOff {
                channel,
                pitch: msg[1] & 0x7F,
            })
        }
        0x90 => {
            if msg.len() < 3 {
                return None;
            }
            let pitch = msg[1] & 0x7F;
            let velocity = msg[2] & 0x7F;
            if velocity == 0 {
                Some(MidiInputEvent::NoteOff { channel, pitch })
            } else {
                Some(MidiInputEvent::NoteOn {
                    channel,
                    pitch,
                    velocity,
                })
            }
        }
        0xB0 => {
            if msg.len() < 3 {
                return None;
            }
            Some(MidiInputEvent::ControlChange {
                channel,
                controller: msg[1] & 0x7F,
                value: msg[2] & 0x7F,
            })
        }
        0xE0 => {
            if msg.len() < 3 {
                return None;
            }
            let lsb = (msg[1] & 0x7F) as i16;
            let msb = (msg[2] & 0x7F) as i16;
            let raw = lsb | (msb << 7);
            Some(MidiInputEvent::PitchBend {
                channel,
                value: raw - 8192,
            })
        }
        _ => None,
    }
}

/// Returns the names of all currently available MIDI input ports.
///
/// Returns an empty `Vec` gracefully if no MIDI backend or devices are available.
pub fn list_midi_input_ports() -> Vec<String> {
    let Ok(midi_in) = midir::MidiInput::new("dawsu-scan") else {
        return Vec::new();
    };
    midi_in
        .ports()
        .iter()
        .filter_map(|port| midi_in.port_name(port).ok())
        .collect()
}

/// Manages one or more live MIDI input connections and forwards decoded events.
///
/// All open connections are closed automatically when dropped.
#[derive(Default)]
pub struct MidiInputRouter {
    connections: Vec<midir::MidiInputConnection<()>>,
}

impl MidiInputRouter {
    /// Creates an empty router with no active connections.
    pub fn new() -> Self {
        Self {
            connections: Vec::new(),
        }
    }

    /// Number of currently open MIDI input connections.
    pub fn connected_count(&self) -> usize {
        self.connections.len()
    }

    /// Closes all active MIDI input connections.
    pub fn disconnect_all(&mut self) {
        self.connections.clear();
    }

    /// Connects to a specific MIDI input port by name and forwards decoded events with arrival
    /// timestamps (`Instant::now()`) to `callback`.
    pub fn connect_port_timestamped<F>(&mut self, port_name: &str, callback: F) -> Result<(), String>
    where
        F: Fn(TimestampedMidiEvent) + Send + 'static,
    {
        let midi_in = midir::MidiInput::new("dawsu-in")
            .map_err(|e| format!("Could not initialize MIDI input: {e}"))?;
        let ports = midi_in.ports();
        let port = ports
            .iter()
            .find(|p| midi_in.port_name(p).ok().as_deref() == Some(port_name))
            .ok_or_else(|| format!("MIDI input port \"{port_name}\" not found."))?;
        let connection = midi_in
            .connect(
                port,
                "dawsu-midi-in",
                move |_stamp, message, _| {
                    let received_at = Instant::now();
                    if let Some(event) = decode_midi_bytes(message) {
                        callback(TimestampedMidiEvent { received_at, event });
                    }
                },
                (),
            )
            .map_err(|e| format!("Could not connect to MIDI port \"{port_name}\": {e}"))?;
        self.connections.push(connection);
        Ok(())
    }

    /// Connects to a specific MIDI input port by name and forwards decoded events to `callback`.
    pub fn connect_port<F>(&mut self, port_name: &str, callback: F) -> Result<(), String>
    where
        F: Fn(MidiInputEvent) + Send + 'static,
    {
        self.connect_port_timestamped(port_name, move |stamped| callback(stamped.event))
    }

    /// Connects to every available MIDI input port and forwards decoded events to `callback`.
    ///
    /// Returns the number of ports successfully connected.
    pub fn connect_all<F>(&mut self, callback: F) -> Result<usize, String>
    where
        F: Fn(MidiInputEvent) + Send + Sync + 'static,
    {
        let names = list_midi_input_ports();
        let shared = Arc::new(callback);
        let mut connected = 0;
        for name in names {
            let cb = Arc::clone(&shared);
            if self.connect_port(&name, move |ev| cb(ev)).is_ok() {
                connected += 1;
            }
        }
        Ok(connected)
    }

    /// Connects to a specific MIDI input port and sends decoded timestamped events to `tx`.
    pub fn connect_port_timestamped_channel(
        &mut self,
        port_name: &str,
        tx: mpsc::Sender<TimestampedMidiEvent>,
    ) -> Result<(), String> {
        self.connect_port_timestamped(port_name, move |ev| {
            let _ = tx.send(ev);
        })
    }

    /// Connects to a specific MIDI input port and sends decoded events to `tx`.
    pub fn connect_port_channel(
        &mut self,
        port_name: &str,
        tx: mpsc::Sender<MidiInputEvent>,
    ) -> Result<(), String> {
        self.connect_port(port_name, move |ev| {
            let _ = tx.send(ev);
        })
    }

    /// Connects to all available MIDI input ports and sends decoded events to `tx`.
    pub fn connect_all_channel(
        &mut self,
        tx: mpsc::Sender<MidiInputEvent>,
    ) -> Result<usize, String> {
        let names = list_midi_input_ports();
        let mut connected = 0;
        for name in names {
            let sender = tx.clone();
            if self
                .connect_port(&name, move |ev| {
                    let _ = sender.send(ev);
                })
                .is_ok()
            {
                connected += 1;
            }
        }
        Ok(connected)
    }
}

impl Drop for MidiInputRouter {
    fn drop(&mut self) {
        self.disconnect_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_note_on() {
        assert_eq!(
            decode_midi_bytes(&[0x99, 36, 110]),
            Some(MidiInputEvent::NoteOn {
                channel: 9,
                pitch: 36,
                velocity: 110,
            })
        );
    }

    #[test]
    fn normalizes_note_on_velocity_zero_to_note_off() {
        assert_eq!(
            decode_midi_bytes(&[0x90, 60, 0]),
            Some(MidiInputEvent::NoteOff {
                channel: 0,
                pitch: 60,
            })
        );
    }

    #[test]
    fn decodes_note_off() {
        assert_eq!(
            decode_midi_bytes(&[0x82, 48, 64]),
            Some(MidiInputEvent::NoteOff {
                channel: 2,
                pitch: 48,
            })
        );
    }

    #[test]
    fn decodes_control_change() {
        assert_eq!(
            decode_midi_bytes(&[0xB0, 64, 127]),
            Some(MidiInputEvent::ControlChange {
                channel: 0,
                controller: 64,
                value: 127,
            })
        );
    }

    #[test]
    fn decodes_pitch_bend() {
        // Center (0x2000 = 8192 -> 0)
        assert_eq!(
            decode_midi_bytes(&[0xE0, 0x00, 0x40]),
            Some(MidiInputEvent::PitchBend {
                channel: 0,
                value: 0,
            })
        );
        // Minimum (0x0000 -> -8192)
        assert_eq!(
            decode_midi_bytes(&[0xE3, 0x00, 0x00]),
            Some(MidiInputEvent::PitchBend {
                channel: 3,
                value: -8192,
            })
        );
        // Maximum (0x3FFF = 16383 -> +8191)
        assert_eq!(
            decode_midi_bytes(&[0xE1, 0x7F, 0x7F]),
            Some(MidiInputEvent::PitchBend {
                channel: 1,
                value: 8191,
            })
        );
    }

    #[test]
    fn rejects_malformed_or_short_bytes() {
        assert_eq!(decode_midi_bytes(&[]), None);
        assert_eq!(decode_midi_bytes(&[0x90]), None);
        assert_eq!(decode_midi_bytes(&[0x90, 60]), None);
        assert_eq!(decode_midi_bytes(&[0x80, 60]), None);
        assert_eq!(decode_midi_bytes(&[0xB0, 64]), None);
        assert_eq!(decode_midi_bytes(&[0xE0, 0x00]), None);
        // Data byte without status
        assert_eq!(decode_midi_bytes(&[0x40, 0x3C, 0x64]), None);
        // System / realtime / unsupported channel messages
        assert_eq!(decode_midi_bytes(&[0xF8]), None);
        assert_eq!(decode_midi_bytes(&[0xF0, 0x7E, 0xF7]), None);
        assert_eq!(decode_midi_bytes(&[0xC0, 1]), None);
    }

    #[test]
    fn list_midi_input_ports_is_safe_on_any_machine() {
        let ports = list_midi_input_ports();
        // Should never panic; every returned name is a valid UTF-8 String.
        let _count = ports.len();
    }
}
