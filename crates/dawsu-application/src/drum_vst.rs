//! Real-time VST3 drum instrument playback, MIDI Channel 10 scheduling, live audition,
//! and lane-to-MIDI routing (parity with v3.1.4 `DrumInstrumentPlayback` / `MainWindow.DrumVst`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, mpsc};

use dawsu_audio::LiveAudioSource;
use dawsu_audio_instruments::{MAX_EVENTS, ShmChannel, ShmEvent};
use dawsu_foundation::{Hitsound, SampleIdentity, Sound};

/// Fixed MIDI Channel 10 (0-indexed 9) status bytes, matching v3.1.4 `DrumInstrumentPlayback`.
pub const NOTE_OFF_CH10: u8 = 0x89;
pub const NOTE_ON_CH10: u8 = 0x99;
pub const CC_CH10: u8 = 0xB9;
pub const CC_ALL_SOUND_OFF: u8 = 120;
pub const CC_ALL_NOTES_OFF: u8 = 123;

/// One scheduled drum MIDI trigger on the timeline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumMidiCue {
    pub at_ms: f64,
    pub note: u8,
    pub velocity: u8,
}

/// Trait over the stereo audio/MIDI block processor so `DrumVstLiveSource` can be tested
/// without spawning a real VST3 sidecar binary.
pub trait DrumBlockProcessor: Send {
    fn process_block(
        &mut self,
        sample_rate: f64,
        events: &[ShmEvent],
        left: &mut [f32],
        right: &mut [f32],
    ) -> bool;
}

impl DrumBlockProcessor for ShmChannel {
    fn process_block(
        &mut self,
        sample_rate: f64,
        events: &[ShmEvent],
        left: &mut [f32],
        right: &mut [f32],
    ) -> bool {
        ShmChannel::process_block(self, sample_rate, events, left, right)
    }
}

/// UI/Application-side handle for controlling a live VST3 drum instrument without locking
/// the audio thread.
#[derive(Debug)]
pub struct DrumVstLiveHandle {
    pub instance_id: String,
    pub plugin_name: String,
    pub plugin_path: String,
    pub plugin_uid: Option<String>,
    enabled: Arc<AtomicBool>,
    schedule: Arc<RwLock<Vec<DrumMidiCue>>>,
    preview_tx: mpsc::SyncSender<(u8, u8)>,
    routed_notes: HashMap<SampleIdentity, u8>,
}

impl DrumVstLiveHandle {
    pub fn new(
        instance_id: String,
        plugin_name: String,
        plugin_path: String,
        plugin_uid: Option<String>,
        enabled: bool,
        processor: Box<dyn DrumBlockProcessor>,
    ) -> (Self, DrumVstLiveSource) {
        let enabled_flag = Arc::new(AtomicBool::new(enabled));
        let schedule = Arc::new(RwLock::new(Vec::new()));
        let (preview_tx, preview_rx) = mpsc::sync_channel(128);
        let handle = Self {
            instance_id,
            plugin_name,
            plugin_path,
            plugin_uid,
            enabled: enabled_flag.clone(),
            schedule: schedule.clone(),
            preview_tx,
            routed_notes: HashMap::new(),
        };
        let source = DrumVstLiveSource {
            processor,
            enabled: enabled_flag,
            schedule,
            preview_rx,
            events: Vec::with_capacity(128),
            active_releases: Vec::with_capacity(32),
            needs_reset: false,
            last_block_end_ms: None,
        };
        (handle, source)
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    pub fn set_schedule(&self, mut cues: Vec<DrumMidiCue>) {
        cues.sort_by(|a, b| a.at_ms.total_cmp(&b.at_ms));
        if let Ok(mut guard) = self.schedule.write() {
            *guard = cues;
        }
    }

    pub fn set_routed_notes(&mut self, notes: HashMap<SampleIdentity, u8>) {
        self.routed_notes = notes;
    }

    pub fn routed_notes(&self) -> &HashMap<SampleIdentity, u8> {
        &self.routed_notes
    }

    pub fn routed_note_for(&self, lane: &SampleIdentity) -> Option<u8> {
        if !self.is_enabled() {
            return None;
        }
        self.routed_notes.get(lane).copied()
    }

    pub fn preview(&self, note: u8, velocity: u8) {
        if self.is_enabled() {
            let _ = self.preview_tx.try_send((note & 0x7F, velocity.clamp(1, 127)));
        }
    }
}

/// Audio-thread `LiveAudioSource` driving a live VST3 drum instrument in 512-frame blocks.
pub struct DrumVstLiveSource {
    processor: Box<dyn DrumBlockProcessor>,
    enabled: Arc<AtomicBool>,
    schedule: Arc<RwLock<Vec<DrumMidiCue>>>,
    preview_rx: mpsc::Receiver<(u8, u8)>,
    events: Vec<ShmEvent>,
    /// `(midi_note, frames_remaining_until_note_off)` — 100 ms release window like v3.1.4.
    active_releases: Vec<(u8, usize)>,
    needs_reset: bool,
    last_block_end_ms: Option<f64>,
}

impl DrumVstLiveSource {
    fn push_event(&mut self, offset: usize, status: u8, data1: u8, data2: u8) {
        if self.events.len() < MAX_EVENTS {
            self.events.push(ShmEvent {
                offset: offset as i32,
                status,
                data1: data1 & 0x7F,
                data2: data2 & 0x7F,
                pad: 0,
            });
        }
    }

    fn trigger_note(&mut self, offset: usize, note: u8, velocity: u8, release_frames: usize) {
        let note = note & 0x7F;
        let velocity = velocity.clamp(1, 127);
        // Retrigger on an active note: emit Note Off right before Note On at the same offset.
        if let Some(existing) = self.active_releases.iter_mut().find(|(n, _)| *n == note) {
            existing.1 = offset + release_frames;
            self.push_event(offset, NOTE_OFF_CH10, note, 0);
        } else {
            self.active_releases.push((note, offset + release_frames));
        }
        self.push_event(offset, NOTE_ON_CH10, note, velocity);
    }
}

impl LiveAudioSource for DrumVstLiveSource {
    fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        sample_rate: u32,
        is_playing: bool,
    ) {
        self.process_timeline_block(left, right, sample_rate, is_playing, 0.0, 1.0);
    }

    fn process_timeline_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        sample_rate: u32,
        is_playing: bool,
        position_ms: f64,
        speed: f64,
    ) {
        let frames = left.len().min(right.len());
        if frames == 0 {
            return;
        }
        if !self.enabled.load(Ordering::Relaxed) {
            // Drain any stale preview events while disabled and return silence immediately.
            while self.preview_rx.try_recv().is_ok() {}
            if !self.active_releases.is_empty() || self.needs_reset {
                self.active_releases.clear();
                self.needs_reset = false;
            }
            self.last_block_end_ms = None;
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
            return;
        }

        self.events.clear();
        let rate = sample_rate.max(1) as f64;
        let release_frames = ((sample_rate / 10) as usize).max(1); // 100 ms like v3.1.4

        if self.needs_reset {
            self.needs_reset = false;
            self.active_releases.clear();
            self.last_block_end_ms = None;
            self.push_event(0, CC_CH10, CC_ALL_SOUND_OFF, 0);
            self.push_event(0, CC_CH10, CC_ALL_NOTES_OFF, 0);
        }

        // Advance pending Note Offs due inside this block.
        let mut i = 0;
        while i < self.active_releases.len() {
            let (note, rem) = self.active_releases[i];
            if rem < frames {
                self.push_event(rem, NOTE_OFF_CH10, note, 0);
                self.active_releases.swap_remove(i);
            } else {
                self.active_releases[i].1 = rem - frames;
                i += 1;
            }
        }

        // Live preview / audition hits (work both while playing and while stopped).
        while let Ok((note, velocity)) = self.preview_rx.try_recv() {
            self.trigger_note(0, note, velocity, release_frames);
        }

        // Timeline playback triggers inside `[window_start, window_end)`.
        if is_playing {
            let eff_speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
            let block_ms = frames as f64 * eff_speed * 1000.0 / rate;
            let window_start = match self.last_block_end_ms {
                Some(prev) if (position_ms - prev).abs() <= block_ms => prev,
                _ => position_ms,
            };
            let window_end = window_start + block_ms;
            self.last_block_end_ms = Some(window_end);

            let schedule = self.schedule.clone();
            if let Ok(guard) = schedule.try_read() {
                let start_idx = guard.partition_point(|c| c.at_ms < window_start);
                for cue in &guard[start_idx..] {
                    if cue.at_ms >= window_end {
                        break;
                    }
                    let rel_ms = (cue.at_ms - window_start).max(0.0);
                    let offset = ((rel_ms * rate / (eff_speed * 1000.0)).round() as usize)
                        .min(frames.saturating_sub(1));
                    self.trigger_note(offset, cue.note, cue.velocity, release_frames);
                }
            }
        } else {
            self.last_block_end_ms = None;
        }

        // Keep events ordered by sample offset for the VST3 host without allocating.
        // Insertion sort is stable (preserving Note Off before Note On at equal offsets)
        // and performs zero heap allocations even above `slice::sort`'s merge threshold.
        for idx in 1..self.events.len() {
            let event = self.events[idx];
            let mut j = idx;
            while j > 0 && self.events[j - 1].offset > event.offset {
                self.events[j] = self.events[j - 1];
                j -= 1;
            }
            self.events[j] = event;
        }
        let _ = self
            .processor
            .process_block(rate, &self.events, &mut left[..frames], &mut right[..frames]);
    }

    fn reset(&mut self) {
        self.needs_reset = true;
        self.last_block_end_ms = None;
    }
}

/// Default GM drum MIDI note for skin lanes (matching v3.1.4 `MainWindow.DrumVst.cs:39-43`).
pub fn default_skin_midi_note(lane: &SampleIdentity) -> Option<u8> {
    match lane {
        SampleIdentity::Standard { sound, index, .. } if index.get() == 0 => match sound {
            Sound::Hit(Hitsound::Normal) => Some(36),
            Sound::Hit(Hitsound::Clap) => Some(38),
            Sound::Hit(Hitsound::Whistle) => Some(42),
            Sound::Hit(Hitsound::Finish) => Some(49),
            _ => None,
        },
        _ => None,
    }
}

/// Converts a normalized hit gain (`0.0..=1.0+`) into MIDI velocity (`1..=127`).
pub fn gain_to_midi_velocity(gain: f32) -> u8 {
    ((gain.clamp(0.0, 1.0) * 127.0).round() as i32).clamp(1, 127) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::{CustomIndex, SampleSet};
    use std::sync::Mutex;

    struct RecordingProcessor {
        calls: Arc<Mutex<Vec<Vec<ShmEvent>>>>,
        fill_value: f32,
    }

    impl RecordingProcessor {
        fn new(fill_value: f32) -> (Self, Arc<Mutex<Vec<Vec<ShmEvent>>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    calls: calls.clone(),
                    fill_value,
                },
                calls,
            )
        }
    }

    impl DrumBlockProcessor for RecordingProcessor {
        fn process_block(
            &mut self,
            _sample_rate: f64,
            events: &[ShmEvent],
            left: &mut [f32],
            right: &mut [f32],
        ) -> bool {
            self.calls.lock().unwrap().push(events.to_vec());
            left.fill(self.fill_value);
            right.fill(self.fill_value);
            true
        }
    }

    #[test]
    fn schedules_timeline_cues_previews_retriggers_and_resets_cc120_123() {
        let (proc, _calls) = RecordingProcessor::new(0.25);
        let (handle, mut source) = DrumVstLiveHandle::new(
            "vst3-1".into(),
            "Test Kit".into(),
            "kit.vst3".into(),
            None,
            true,
            Box::new(proc),
        );
        handle.set_schedule(vec![
            DrumMidiCue { at_ms: 0.0, note: 36, velocity: 100 },
            DrumMidiCue { at_ms: 5.0, note: 36, velocity: 120 },
        ]);
        let mut l = [0.0f32; 512];
        let mut r = [0.0f32; 512];
        // Block 1: playing at 0 ms (44100 Hz -> 512 frames ~= 11.61 ms), fires both kicks with retrigger Note Off.
        source.process_timeline_block(&mut l, &mut r, 44_100, true, 0.0, 1.0);
        assert_eq!(l[0], 0.25);

        // Preview while stopped still renders audio and emits Note On at offset 0.
        handle.preview(38, 95);
        source.process_timeline_block(&mut l, &mut r, 44_100, false, 0.0, 1.0);
        assert_eq!(l[0], 0.25);

        // Reset emits CC 120 + CC 123.
        source.reset();
        source.process_timeline_block(&mut l, &mut r, 44_100, false, 0.0, 1.0);

        // Toggling OFF returns silence immediately.
        handle.set_enabled(false);
        source.process_timeline_block(&mut l, &mut r, 44_100, true, 0.0, 1.0);
        assert_eq!(l[0], 0.0);
        assert_eq!(r[0], 0.0);
    }

    #[test]
    fn jitter_at_fractional_speeds_fires_each_cue_exactly_once() {
        const SAMPLE_RATE: u32 = 44_100;
        const FRAMES: usize = 512;
        let jitter_pattern = [0.0, -3.0, 3.0, -2.5, 2.8, -1.9, 2.9];

        for speed in [0.75_f64, 1.5_f64] {
            let (proc, calls) = RecordingProcessor::new(0.0);
            let (handle, mut source) = DrumVstLiveHandle::new(
                "vst3-jitter".into(),
                "Test Kit".into(),
                "kit.vst3".into(),
                None,
                true,
                Box::new(proc),
            );

            let block_ms = FRAMES as f64 * speed * 1000.0 / SAMPLE_RATE as f64;
            // Place cues near block boundaries (including inside the +-3 ms jitter band) and mid-block.
            let mut cues = Vec::new();
            let mut note_on_counts: HashMap<(u8, u8), usize> = HashMap::new();
            for i in 0..24 {
                let at_ms = match i % 3 {
                    0 => (i as f64 / 3.0 + 1.0) * block_ms - 0.4,
                    1 => (i as f64 / 3.0 + 1.0) * block_ms + 0.4,
                    _ => (i as f64 / 3.0 + 0.5) * block_ms,
                };
                let note = 36 + (i as u8 % 16);
                let velocity = 60 + i as u8;
                cues.push(DrumMidiCue { at_ms, note, velocity });
                note_on_counts.insert((note, velocity), 0);
            }
            let max_at_ms = cues.iter().map(|c| c.at_ms).fold(0.0_f64, f64::max);
            handle.set_schedule(cues);

            let mut l = [0.0f32; FRAMES];
            let mut r = [0.0f32; FRAMES];
            let total_blocks = (max_at_ms / block_ms).ceil() as usize + 2;
            for block_idx in 0..total_blocks {
                let nominal = block_idx as f64 * block_ms;
                let jitter = if block_idx == 0 {
                    0.0
                } else {
                    jitter_pattern[block_idx % jitter_pattern.len()]
                };
                let pos = (nominal + jitter).max(0.0);
                source.process_timeline_block(&mut l, &mut r, SAMPLE_RATE, true, pos, speed);
            }

            let recorded = calls.lock().unwrap();
            for block_events in recorded.iter() {
                for ev in block_events {
                    if ev.status == NOTE_ON_CH10 {
                        *note_on_counts.entry((ev.data1, ev.data2)).or_insert(0) += 1;
                    }
                }
            }

            for ((note, velocity), count) in &note_on_counts {
                assert_eq!(
                    *count, 1,
                    "speed={speed}: expected cue (note={note}, vel={velocity}) to fire once, got {count}"
                );
            }
        }
    }

    #[test]
    fn insertion_sort_preserves_note_off_before_note_on_with_many_events() {
        const SAMPLE_RATE: u32 = 44_100;
        const FRAMES: usize = 512;
        let (proc, calls) = RecordingProcessor::new(0.0);
        let (handle, mut source) = DrumVstLiveHandle::new(
            "vst3-sort".into(),
            "Test Kit".into(),
            "kit.vst3".into(),
            None,
            true,
            Box::new(proc),
        );

        let block_ms = FRAMES as f64 * 1000.0 / SAMPLE_RATE as f64;
        // Block 0: trigger 12 notes so `active_releases` holds pending Note Offs expiring in block 8.
        // In block 8 (at 8 * block_ms), those Note Offs fire at various offsets while previews (offset 0)
        // and retriggered timeline cues (at offsets before, equal to, and after those Note Offs)
        // produce > 30 unsorted events with shared offsets.
        let mut cues = Vec::new();
        for i in 0..12u8 {
            let note = 36 + i;
            let initial_ms = (i as f64) * 0.5;
            cues.push(DrumMidiCue { at_ms: initial_ms, note, velocity: 90 });

            // Re-trigger the same note twice at identical timestamps inside block 8.
            let block8_ms = 8.0 * block_ms + (11 - i) as f64 * 0.6;
            cues.push(DrumMidiCue { at_ms: block8_ms, note, velocity: 100 });
            cues.push(DrumMidiCue { at_ms: block8_ms, note, velocity: 110 });
        }
        handle.set_schedule(cues);

        let mut l = [0.0f32; FRAMES];
        let mut r = [0.0f32; FRAMES];
        for block_idx in 0..8 {
            source.process_timeline_block(
                &mut l,
                &mut r,
                SAMPLE_RATE,
                true,
                block_idx as f64 * block_ms,
                1.0,
            );
        }

        // Queue previews at offset 0 on already-active notes so Note Off + Note On appear at offset 0 too.
        handle.preview(36, 95);
        handle.preview(37, 96);
        source.process_timeline_block(&mut l, &mut r, SAMPLE_RATE, true, 8.0 * block_ms, 1.0);

        let recorded = calls.lock().unwrap();
        let block8_events = &recorded[8];
        assert!(
            block8_events.len() > 30,
            "expected > 30 events in block 8, got {}",
            block8_events.len()
        );
        assert!(
            block8_events.windows(2).all(|w| w[0].offset <= w[1].offset),
            "events must be sorted by non-decreasing offset: {block8_events:?}"
        );

        // Verify that for every retriggered note at the same offset, Note Off precedes Note On.
        for note in 36..48u8 {
            let mut offsets_seen: HashMap<i32, Vec<u8>> = HashMap::new();
            for ev in block8_events.iter().filter(|e| e.data1 == note) {
                offsets_seen.entry(ev.offset).or_default().push(ev.status);
            }
            for (offset, statuses) in offsets_seen {
                if statuses.contains(&NOTE_OFF_CH10) && statuses.contains(&NOTE_ON_CH10) {
                    let first_off = statuses.iter().position(|&s| s == NOTE_OFF_CH10).unwrap();
                    let first_on = statuses.iter().position(|&s| s == NOTE_ON_CH10).unwrap();
                    assert!(
                        first_off < first_on,
                        "note {note} at offset {offset}: Note Off must precede Note On, got {statuses:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn skin_defaults_match_v3_1_4() {
        let mk = |sound| SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(sound),
            index: CustomIndex::new(0),
        };
        assert_eq!(default_skin_midi_note(&mk(Hitsound::Normal)), Some(36));
        assert_eq!(default_skin_midi_note(&mk(Hitsound::Clap)), Some(38));
        assert_eq!(default_skin_midi_note(&mk(Hitsound::Whistle)), Some(42));
        assert_eq!(default_skin_midi_note(&mk(Hitsound::Finish)), Some(49));
        let custom = SampleIdentity::Standard {
            set: SampleSet::Normal,
            sound: Sound::Hit(Hitsound::Normal),
            index: CustomIndex::new(2),
        };
        assert_eq!(default_skin_midi_note(&custom), None);
    }
}
