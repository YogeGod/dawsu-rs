//! Real-time VST3 drum instrument playback, MIDI Channel 10 scheduling, live audition,
//! and lane-to-MIDI routing (parity with v3.1.4 `DrumInstrumentPlayback` / `MainWindow.DrumVst`).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use dawsu_audio::{LiveAudioSource, SpscConsumer, SpscProducer, spsc_ring};
use dawsu_audio_instruments::{MAX_EVENTS, MidiStateTracker, ShmChannel};
pub use dawsu_audio_instruments::ShmEvent;
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

const PREVIEW_RING_CAPACITY: usize = 256;
const MAX_ACTIVE_RELEASES: usize = 128;

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
    /// Lock-free 128-note choke group table (`0` = no choke group, `1..=255` = choke group ID).
    choke_groups: Arc<[AtomicU8; 128]>,
    preview_tx: Mutex<SpscProducer<(u8, u8), PREVIEW_RING_CAPACITY>>,
    routed_notes: HashMap<SampleIdentity, u8>,
    muted_vst_lanes: HashSet<SampleIdentity>,
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
        let choke_groups: Arc<[AtomicU8; 128]> = Arc::new(std::array::from_fn(|_| AtomicU8::new(0)));
        let (preview_tx, preview_rx) = spsc_ring::<(u8, u8), PREVIEW_RING_CAPACITY>();
        let handle = Self {
            instance_id,
            plugin_name,
            plugin_path,
            plugin_uid,
            enabled: enabled_flag.clone(),
            schedule: schedule.clone(),
            choke_groups: choke_groups.clone(),
            preview_tx: Mutex::new(preview_tx),
            routed_notes: HashMap::new(),
            muted_vst_lanes: HashSet::new(),
        };
        let source = DrumVstLiveSource {
            processor,
            enabled: enabled_flag,
            schedule,
            choke_groups,
            preview_rx,
            events: Vec::with_capacity(MAX_EVENTS),
            active_releases: Vec::with_capacity(MAX_ACTIVE_RELEASES),
            midi_state: MidiStateTracker::new(),
            was_enabled: enabled,
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

    /// Maps MIDI note numbers (`0..=127`) to a choke group ID (`1..=255`, `0` clears).
    pub fn set_choke_groups(&self, groups: HashMap<u8, u8>) {
        for note in 0..128u8 {
            let gid = groups.get(&note).copied().unwrap_or(0);
            self.choke_groups[note as usize].store(gid, Ordering::Relaxed);
        }
    }

    pub fn set_routed_notes(&mut self, notes: HashMap<SampleIdentity, u8>) {
        self.routed_notes = notes;
    }

    pub fn routed_notes(&self) -> &HashMap<SampleIdentity, u8> {
        &self.routed_notes
    }

    pub fn muted_vst_lanes(&self) -> &HashSet<SampleIdentity> {
        &self.muted_vst_lanes
    }

    pub fn is_lane_vst_muted(&self, lane: &SampleIdentity) -> bool {
        self.muted_vst_lanes.contains(lane)
    }

    pub fn set_lane_vst_muted(&mut self, lane: &SampleIdentity, muted: bool) {
        if muted {
            self.muted_vst_lanes.insert(lane.clone());
        } else {
            self.muted_vst_lanes.remove(lane);
        }
    }

    /// Toggles VST3 routing off/on for one lane. Returns `true` when routed to VST3,
    /// `false` when bypassed back to sample playback.
    pub fn toggle_lane_vst(&mut self, lane: &SampleIdentity) -> bool {
        if self.muted_vst_lanes.remove(lane) {
            true
        } else {
            self.muted_vst_lanes.insert(lane.clone());
            false
        }
    }

    pub fn routed_note_for(&self, lane: &SampleIdentity) -> Option<u8> {
        if !self.is_enabled() || self.muted_vst_lanes.contains(lane) {
            return None;
        }
        self.routed_notes.get(lane).copied()
    }

    pub fn preview(&self, note: u8, velocity: u8) {
        if self.is_enabled() {
            let mut tx = self.preview_tx.lock().unwrap_or_else(|e| e.into_inner());
            let _ = tx.try_push((note & 0x7F, velocity.clamp(1, 127)));
        }
    }
}

/// Audio-thread `LiveAudioSource` driving a live VST3 drum instrument in 512-frame blocks.
pub struct DrumVstLiveSource {
    processor: Box<dyn DrumBlockProcessor>,
    enabled: Arc<AtomicBool>,
    schedule: Arc<RwLock<Vec<DrumMidiCue>>>,
    choke_groups: Arc<[AtomicU8; 128]>,
    preview_rx: SpscConsumer<(u8, u8), PREVIEW_RING_CAPACITY>,
    events: Vec<ShmEvent>,
    /// `(midi_note, frames_remaining_until_note_off)` — 100 ms release window like v3.1.4.
    active_releases: Vec<(u8, usize)>,
    midi_state: MidiStateTracker,
    was_enabled: bool,
    needs_reset: bool,
    last_block_end_ms: Option<f64>,
}

impl DrumVstLiveSource {
    fn push_event(&mut self, offset: usize, status: u8, data1: u8, data2: u8) {
        let d1 = data1 & 0x7F;
        let d2 = data2 & 0x7F;
        self.midi_state.track_event(status, d1, d2);
        if self.events.len() < MAX_EVENTS {
            self.events.push(ShmEvent {
                offset: offset as i32,
                status,
                data1: d1,
                data2: d2,
                pad: 0,
            });
        }
    }

    fn trigger_note(
        &mut self,
        offset: usize,
        note: u8,
        velocity: u8,
        release_frames: usize,
        sample_rate: u32,
    ) {
        let note = note & 0x7F;
        let velocity = velocity.clamp(1, 127);
        // Lock-free choke group lookup on the real-time audio thread: notes in a choke group hold
        // open for 4 s so a later peer trigger (e.g. closed hat after open hat) cuts it cleanly.
        let gid = self.choke_groups[note as usize].load(Ordering::Relaxed);
        let effective_release = if gid != 0 {
            let mut i = 0;
            while i < self.active_releases.len() {
                let (active_note, _) = self.active_releases[i];
                if active_note != note
                    && self.choke_groups[(active_note & 0x7F) as usize].load(Ordering::Relaxed) == gid
                {
                    self.push_event(offset, NOTE_OFF_CH10, active_note, 0);
                    self.active_releases.swap_remove(i);
                } else {
                    i += 1;
                }
            }
            (sample_rate.max(1) as usize).saturating_mul(4).max(release_frames)
        } else {
            release_frames
        };
        // Retrigger on an active note: emit Note Off right before Note On at the same offset.
        if let Some(existing) = self.active_releases.iter_mut().find(|(n, _)| *n == note) {
            existing.1 = offset + effective_release;
            self.push_event(offset, NOTE_OFF_CH10, note, 0);
        } else if self.active_releases.len() < self.active_releases.capacity() {
            self.active_releases.push((note, offset + effective_release));
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
        let rate = sample_rate.max(1) as f64;
        if !self.enabled.load(Ordering::Relaxed) {
            // Drain any stale preview events while disabled and flush active notes cleanly once.
            while self.preview_rx.try_pop().is_some() {}
            if self.was_enabled {
                self.was_enabled = false;
                self.events.clear();
                self.midi_state.flush_channel(9, 0, &mut self.events, MAX_EVENTS);
                if !self.events.is_empty() {
                    let _ = self.processor.process_block(
                        rate,
                        &self.events,
                        &mut left[..frames],
                        &mut right[..frames],
                    );
                }
            }
            if !self.active_releases.is_empty() || self.needs_reset {
                self.active_releases.clear();
                self.needs_reset = false;
            }
            self.last_block_end_ms = None;
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
            return;
        }
        self.was_enabled = true;

        self.events.clear();
        let release_frames = ((sample_rate / 10) as usize).max(1); // 100 ms like v3.1.4

        if self.needs_reset {
            self.needs_reset = false;
            self.active_releases.clear();
            self.last_block_end_ms = None;
            self.midi_state.flush_channel(9, 0, &mut self.events, MAX_EVENTS);
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
        while let Some((note, velocity)) = self.preview_rx.try_pop() {
            self.trigger_note(0, note, velocity, release_frames, sample_rate);
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
                    self.trigger_note(offset, cue.note, cue.velocity, release_frames, sample_rate);
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

    #[test]
    fn bypass_and_cycle_reset_flush_active_notes_before_next_trigger() {
        let (proc, calls) = RecordingProcessor::new(0.25);
        let (handle, mut source) = DrumVstLiveHandle::new(
            "vst3-flush".into(),
            "Test Kit".into(),
            "kit.vst3".into(),
            None,
            true,
            Box::new(proc),
        );
        handle.set_schedule(vec![DrumMidiCue { at_ms: 100.0, note: 36, velocity: 110 }]);
        let mut l = [0.0f32; 512];
        let mut r = [0.0f32; 512];

        // Trigger note 36 at 100 ms (cycle.left), then reset right before wrapping back to 100 ms.
        source.process_timeline_block(&mut l, &mut r, 48_000, true, 100.0, 1.0);
        source.reset();
        source.process_timeline_block(&mut l, &mut r, 48_000, true, 100.0, 1.0);

        // Now mute live VST while note 36 is active -> emits explicit Note Off + CC 123/120 once, then goes idle.
        handle.set_enabled(false);
        source.process_timeline_block(&mut l, &mut r, 48_000, true, 110.0, 1.0);
        assert_eq!(l[0], 0.0);
        source.process_timeline_block(&mut l, &mut r, 48_000, true, 120.0, 1.0);

        let recorded = calls.lock().unwrap();
        assert_eq!(recorded.len(), 3, "second muted block must not call processor");
        // Block 1 (after reset + retrigger at 100 ms): NoteOff(36), CC 123, CC 120, then NoteOn(36).
        let wrap_block = &recorded[1];
        assert_eq!(wrap_block[0].status, NOTE_OFF_CH10);
        assert_eq!(wrap_block[0].data1, 36);
        assert!(wrap_block.iter().any(|e| e.status == CC_CH10 && e.data1 == CC_ALL_NOTES_OFF));
        assert!(wrap_block.iter().any(|e| e.status == CC_CH10 && e.data1 == CC_ALL_SOUND_OFF));
        assert_eq!(wrap_block.last().unwrap().status, NOTE_ON_CH10);
        assert_eq!(wrap_block.last().unwrap().data1, 36);

        // Block 2 (mute flush): NoteOff(36) before CC 123 / CC 120.
        let mute_block = &recorded[2];
        assert_eq!(mute_block[0].status, NOTE_OFF_CH10);
        assert_eq!(mute_block[0].data1, 36);
    }
}
