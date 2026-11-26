//! Zero-allocation MIDI 1.0 channel/note/controller state tracker for RT audio callbacks.
//!
//! Tracks active ref-counted notes, sustain/sostenuto/soft pedals, continuous controllers,
//! program changes, and pitch bend across 16 channels so stop, seek, Cycle wrap, mute, and
//! bypass flush cleanly with zero hanging notes or stuck sustain pedals.

use crate::host::ShmEvent;

/// Fixed-size, RT-safe MIDI channel and voice state tracker.
#[derive(Debug, Clone)]
pub struct MidiStateTracker {
    active_notes: [[u8; 128]; 16],
    sustain_pedal: [bool; 16],
    sostenuto_pedal: [bool; 16],
    soft_pedal: [bool; 16],
    cc_values: [[u8; 128]; 16],
    cc_set: [[bool; 128]; 16],
    program: [Option<u8>; 16],
    pitch_bend: [Option<(u8, u8)>; 16],
}

impl Default for MidiStateTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiStateTracker {
    pub fn new() -> Self {
        Self {
            active_notes: [[0; 128]; 16],
            sustain_pedal: [false; 16],
            sostenuto_pedal: [false; 16],
            soft_pedal: [false; 16],
            cc_values: [[0; 128]; 16],
            cc_set: [[false; 128]; 16],
            program: [None; 16],
            pitch_bend: [None; 16],
        }
    }

    pub fn reset_all(&mut self) {
        self.active_notes = [[0; 128]; 16];
        self.sustain_pedal = [false; 16];
        self.sostenuto_pedal = [false; 16];
        self.soft_pedal = [false; 16];
        self.cc_values = [[0; 128]; 16];
        self.cc_set = [[false; 128]; 16];
        self.program = [None; 16];
        self.pitch_bend = [None; 16];
    }

    /// Updates state from one channel voice MIDI message (`status`, `data1`, `data2`).
    pub fn track_event(&mut self, status: u8, data1: u8, data2: u8) {
        let kind = status & 0xF0;
        let ch = usize::from(status & 0x0F);
        let d1 = data1 & 0x7F;
        let d2 = data2 & 0x7F;
        match kind {
            0x90 => {
                let slot = &mut self.active_notes[ch][usize::from(d1)];
                if d2 > 0 {
                    *slot = slot.saturating_add(1);
                } else {
                    *slot = slot.saturating_sub(1);
                }
            }
            0x80 => {
                let slot = &mut self.active_notes[ch][usize::from(d1)];
                *slot = slot.saturating_sub(1);
            }
            0xB0 => {
                match d1 {
                    64 => self.sustain_pedal[ch] = d2 >= 64,
                    66 => self.sostenuto_pedal[ch] = d2 >= 64,
                    67 => self.soft_pedal[ch] = d2 >= 64,
                    120 | 123 => {
                        self.active_notes[ch].fill(0);
                        self.sustain_pedal[ch] = false;
                        self.sostenuto_pedal[ch] = false;
                        self.soft_pedal[ch] = false;
                    }
                    _ => {}
                }
                if d1 <= 119 {
                    self.cc_values[ch][usize::from(d1)] = d2;
                    self.cc_set[ch][usize::from(d1)] = true;
                }
            }
            0xC0 => {
                self.program[ch] = Some(d1);
            }
            0xE0 => {
                self.pitch_bend[ch] = Some((d1, d2));
            }
            _ => {}
        }
    }

    pub fn active_note_count(&self) -> usize {
        self.active_notes
            .iter()
            .flat_map(|ch| ch.iter())
            .filter(|&&c| c > 0)
            .count()
    }

    pub fn is_note_active(&self, channel: u8, pitch: u8) -> bool {
        self.active_notes[usize::from(channel & 0x0F)][usize::from(pitch & 0x7F)] > 0
    }

    pub fn is_sustain_held(&self, channel: u8) -> bool {
        self.sustain_pedal[usize::from(channel & 0x0F)]
    }

    /// Emits pedal-off CCs, explicit `Note Off` for every active pitch on `channel`, and
    /// `CC 123` / `CC 120` safety reset messages at sample `offset`.
    pub fn flush_channel(
        &mut self,
        channel: u8,
        offset: i32,
        out: &mut Vec<ShmEvent>,
        max_events: usize,
    ) {
        let ch_u8 = channel & 0x0F;
        let ch = usize::from(ch_u8);
        let push = |out: &mut Vec<ShmEvent>, status: u8, d1: u8, d2: u8| {
            if out.len() < max_events {
                out.push(ShmEvent {
                    offset,
                    status,
                    data1: d1,
                    data2: d2,
                    pad: 0,
                });
            }
        };

        if self.sustain_pedal[ch] {
            self.sustain_pedal[ch] = false;
            self.cc_values[ch][64] = 0;
            push(out, 0xB0 | ch_u8, 64, 0);
        }
        if self.sostenuto_pedal[ch] {
            self.sostenuto_pedal[ch] = false;
            self.cc_values[ch][66] = 0;
            push(out, 0xB0 | ch_u8, 66, 0);
        }
        if self.soft_pedal[ch] {
            self.soft_pedal[ch] = false;
            self.cc_values[ch][67] = 0;
            push(out, 0xB0 | ch_u8, 67, 0);
        }

        for (pitch, count) in self.active_notes[ch].iter_mut().enumerate() {
            if *count > 0 {
                *count = 0;
                push(out, 0x80 | ch_u8, pitch as u8, 0);
            }
        }

        push(out, 0xB0 | ch_u8, 123, 0);
        push(out, 0xB0 | ch_u8, 120, 0);
    }

    /// Flushes every channel that has any active note or held pedal.
    pub fn flush_all_notes(&mut self, offset: i32, out: &mut Vec<ShmEvent>, max_events: usize) {
        for ch in 0..16u8 {
            let idx = usize::from(ch);
            let any_active = self.active_notes[idx].iter().any(|&c| c > 0)
                || self.sustain_pedal[idx]
                || self.sostenuto_pedal[idx]
                || self.soft_pedal[idx];
            if any_active {
                self.flush_channel(ch, offset, out, max_events);
            }
        }
    }

    /// Emits Bank Select MSB/LSB -> Program Change -> CCs (`0..=119`) -> Pitch Bend at `offset`.
    pub fn emit_chase_at_offset(
        &self,
        offset: i32,
        out: &mut Vec<ShmEvent>,
        max_events: usize,
    ) {
        let push = |out: &mut Vec<ShmEvent>, status: u8, d1: u8, d2: u8| {
            if out.len() < max_events {
                out.push(ShmEvent {
                    offset,
                    status,
                    data1: d1,
                    data2: d2,
                    pad: 0,
                });
            }
        };

        for ch in 0..16u8 {
            let idx = usize::from(ch);
            if self.cc_set[idx][0] {
                push(out, 0xB0 | ch, 0, self.cc_values[idx][0]);
            }
            if self.cc_set[idx][32] {
                push(out, 0xB0 | ch, 32, self.cc_values[idx][32]);
            }
            if let Some(prog) = self.program[idx] {
                push(out, 0xC0 | ch, prog, 0);
            }
            for cc in 0..=119usize {
                if cc == 0 || cc == 32 {
                    continue;
                }
                if self.cc_set[idx][cc] {
                    push(out, 0xB0 | ch, cc as u8, self.cc_values[idx][cc]);
                }
            }
            if let Some((lsb, msb)) = self.pitch_bend[idx] {
                push(out, 0xE0 | ch, lsb, msb);
            }
        }
    }

    /// Reconstructs controller, bank, program, and pitch-bend state up to `target_ms` and
    /// emits chase events at offset `0`.
    pub fn chase_from_timeline(
        events: &[(f64, u8, u8, u8)],
        target_ms: f64,
        out: &mut Vec<ShmEvent>,
        max_events: usize,
    ) {
        let mut tracker = Self::new();
        for &(at_ms, status, d1, d2) in events {
            if at_ms <= target_ms {
                tracker.track_event(status, d1, d2);
            }
        }
        tracker.emit_chase_at_offset(0, out, max_events);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refcounted_notes_stay_active_until_matching_note_off() {
        let mut tracker = MidiStateTracker::new();
        tracker.track_event(0x90, 60, 100);
        tracker.track_event(0x90, 60, 110);
        assert!(tracker.is_note_active(0, 60));
        assert_eq!(tracker.active_note_count(), 1);

        // First Note Off decrements refcount from 2 to 1; pitch remains active.
        tracker.track_event(0x80, 60, 0);
        assert!(tracker.is_note_active(0, 60));

        // Second Note Off clears active note.
        tracker.track_event(0x90, 60, 0);
        assert!(!tracker.is_note_active(0, 60));
        assert_eq!(tracker.active_note_count(), 0);
    }

    #[test]
    fn flush_emits_pedal_off_explicit_note_offs_and_cc120_123() {
        let mut tracker = MidiStateTracker::new();
        tracker.track_event(0xB0, 64, 127);
        tracker.track_event(0x90, 60, 90);
        tracker.track_event(0x90, 64, 95);
        tracker.track_event(0x90, 67, 100);
        assert!(tracker.is_sustain_held(0));
        assert_eq!(tracker.active_note_count(), 3);

        let mut out = Vec::new();
        tracker.flush_all_notes(0, &mut out, 64);

        assert!(!tracker.is_sustain_held(0));
        assert_eq!(tracker.active_note_count(), 0);
        assert_eq!(out[0].status, 0xB0);
        assert_eq!((out[0].data1, out[0].data2), (64, 0));
        let note_offs: Vec<u8> = out
            .iter()
            .filter(|e| e.status == 0x80)
            .map(|e| e.data1)
            .collect();
        assert_eq!(note_offs, vec![60, 64, 67]);
        assert_eq!(out.iter().filter(|e| e.status == 0xB0 && e.data1 == 123).count(), 1);
        assert_eq!(out.iter().filter(|e| e.status == 0xB0 && e.data1 == 120).count(), 1);
    }

    #[test]
    fn chase_emits_bank_program_cc_pitch_bend_before_target_only() {
        let timeline = [
            (100.0, 0xB1, 0, 2),
            (100.0, 0xB1, 32, 5),
            (150.0, 0xC1, 42, 0),
            (200.0, 0xB1, 1, 88),
            (250.0, 0xE1, 0x10, 0x50),
            // After target_ms -> ignored.
            (500.0, 0xC1, 99, 0),
            (600.0, 0xB1, 1, 12),
        ];
        let mut out = Vec::new();
        MidiStateTracker::chase_from_timeline(&timeline, 300.0, &mut out, 64);
        assert_eq!(out.len(), 5);
        assert!(out.iter().all(|e| e.offset == 0));
        assert_eq!((out[0].status, out[0].data1, out[0].data2), (0xB1, 0, 2));
        assert_eq!((out[1].status, out[1].data1, out[1].data2), (0xB1, 32, 5));
        assert_eq!((out[2].status, out[2].data1), (0xC1, 42));
        assert_eq!((out[3].status, out[3].data1, out[3].data2), (0xB1, 1, 88));
        assert_eq!((out[4].status, out[4].data1, out[4].data2), (0xE1, 0x10, 0x50));
    }
}
