//! Live MIDI input recording into hitsound lanes and keysound tracks.

use dawsu_audio::ClockSnapshot;
use dawsu_audio_instruments::midi_in::{
    MidiInputEvent, MidiInputRouter, TimestampedMidiEvent, list_midi_input_ports,
};
use dawsu_editor::tempo::{SnapSpacing, TempoMap};
use dawsu_editor::tools::ToolTarget;
use dawsu_editor_composer::{BatchHit, BatchOptions, Composer, Policy};
use dawsu_foundation::{SampleIdentity, SongTime};
use dawsu_hitsounding::drums::{DrumMap, render_notes};
use dawsu_keysounding::{KeysoundNote, KeysoundProject};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, TryRecvError};
use std::time::Instant;

const RETROSPECTIVE_RING_CAPACITY: usize = 512;

/// One captured MIDI Note-On event timestamped against the editor playback clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordedMidiNote {
    pub time_ms: f64,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
    pub duration_ms: Option<f64>,
}

/// One live keyboard lane tap captured against the editor playback clock.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedTap {
    pub time_ms: f64,
    pub lane: SampleIdentity,
    pub volume: i32,
}

/// Summary returned after capturing buffered retrospective MIDI notes / taps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrospectiveCaptureStatus {
    pub committed_hits: usize,
    pub committed_keysounds: usize,
}

/// Persistent MIDI input listener that keeps the chosen port open across arm/disarm cycles
/// and recovers cleanly if a hardware MIDI device disconnects.
#[derive(Default)]
pub struct MidiListener {
    pub port_index: Option<usize>,
    pub port_name: Option<String>,
    pub last_error: Option<String>,
    router: Option<MidiInputRouter>,
    rx: Option<mpsc::Receiver<TimestampedMidiEvent>>,
}

impl MidiListener {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_connected(&self) -> bool {
        self.rx.is_some()
    }

    pub fn disconnect(&mut self) {
        self.router = None;
        self.rx = None;
        self.port_index = None;
        self.port_name = None;
    }

    /// Opens `port_index` (or port `0` when `None` and hardware ports exist) persistently.
    pub fn select_port(&mut self, port_index: Option<usize>) -> Result<Option<String>, String> {
        let ports = list_midi_input_ports();
        let chosen_idx = port_index.or(if ports.is_empty() { None } else { Some(0) });
        if let Some(idx) = chosen_idx
            && let Some(name) = ports.get(idx).cloned()
        {
            if self.rx.is_some() && self.port_index == Some(idx) && self.port_name.as_deref() == Some(&name) {
                return Ok(Some(name));
            }
            let mut router = MidiInputRouter::new();
            let (tx, rx) = mpsc::channel();
            router.connect_port_timestamped_channel(&name, tx)?;
            self.router = Some(router);
            self.rx = Some(rx);
            self.port_index = Some(idx);
            self.port_name = Some(name.clone());
            self.last_error = None;
            return Ok(Some(name));
        }
        self.disconnect();
        Ok(None)
    }

    #[cfg(test)]
    pub(crate) fn attach_test_receiver(&mut self, rx: mpsc::Receiver<TimestampedMidiEvent>) {
        self.router = None;
        self.rx = Some(rx);
        self.last_error = None;
    }

    /// Drains all queued timestamped events, recovering cleanly on hot-unplug disconnects.
    pub fn drain(&mut self) -> Vec<TimestampedMidiEvent> {
        let Some(rx) = self.rx.as_ref() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(ev) => events.push(ev),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if disconnected {
            let lost = self.port_name.clone();
            self.router = None;
            self.rx = None;
            self.last_error = Some(match lost {
                Some(name) => format!("MIDI input \"{name}\" disconnected."),
                None => "MIDI input disconnected.".into(),
            });
        }
        events
    }
}

/// Bounded circular buffer preserving recent unarmed MIDI notes and keyboard taps.
#[derive(Debug, Default)]
pub struct RetrospectiveRing {
    midi: VecDeque<RecordedMidiNote>,
    taps: VecDeque<RecordedTap>,
}

impl RetrospectiveRing {
    pub fn push_midi_note_on(&mut self, note: RecordedMidiNote) {
        if self.midi.len() >= RETROSPECTIVE_RING_CAPACITY {
            self.midi.pop_front();
        }
        self.midi.push_back(note);
    }

    pub fn close_midi_note(&mut self, channel: u8, pitch: u8, end_ms: f64) {
        if let Some(open) = self
            .midi
            .iter_mut()
            .rev()
            .find(|n| n.channel == channel && n.pitch == pitch && n.duration_ms.is_none())
        {
            open.duration_ms = Some((end_ms - open.time_ms).max(1.0));
        }
    }

    pub fn push_tap(&mut self, tap: RecordedTap) {
        if self.taps.len() >= RETROSPECTIVE_RING_CAPACITY {
            self.taps.pop_front();
        }
        self.taps.push_back(tap);
    }

    pub fn midi_len(&self) -> usize {
        self.midi.len()
    }

    pub fn tap_len(&self) -> usize {
        self.taps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.midi.is_empty() && self.taps.is_empty()
    }

    pub fn clear(&mut self) {
        self.midi.clear();
        self.taps.clear();
    }

    pub fn take_all(&mut self) -> (Vec<RecordedMidiNote>, Vec<RecordedTap>) {
        (self.midi.drain(..).collect(), self.taps.drain(..).collect())
    }
}

/// Summary returned when toggling live keyboard tap-recording on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TapRecordStatus {
    pub armed: bool,
    pub buffered_taps: usize,
    pub committed_taps: usize,
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

/// Manages live MIDI input capture (plus continuous unarmed retrospective capture).
pub struct MidiRecorder {
    pub armed: bool,
    pub port_index: Option<usize>,
    pub port_name: Option<String>,
    pub listener: MidiListener,
    pub retrospective: RetrospectiveRing,
    recorded: Vec<RecordedMidiNote>,
    last_clock: Option<(Instant, f64, bool, f64)>,
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
            listener: MidiListener::new(),
            retrospective: RetrospectiveRing::default(),
            recorded: Vec::new(),
            last_clock: None,
        }
    }

    /// Selects a persistent hardware MIDI port without arming recording.
    pub fn select_port(&mut self, port_index: Option<usize>) -> Result<Option<String>, String> {
        let name = self.listener.select_port(port_index)?;
        self.port_index = self.listener.port_index;
        self.port_name = self.listener.port_name.clone();
        Ok(name)
    }

    /// Arms MIDI recording, keeping or opening the persistent MIDI input listener cleanly.
    pub fn arm(&mut self, port_index: Option<usize>) -> Result<Option<String>, String> {
        self.recorded.clear();
        self.last_clock = None;
        let name = if port_index.is_some() || !self.listener.is_connected() {
            self.listener.select_port(port_index)?
        } else {
            self.listener.port_name.clone()
        };
        self.port_index = self.listener.port_index;
        self.port_name = self.listener.port_name.clone();
        self.armed = true;
        Ok(name)
    }

    /// Disarms MIDI recording after draining queued events while keeping `listener` open.
    pub fn disarm(&mut self) -> Vec<RecordedMidiNote> {
        if let Some((last_now, last_pos, last_playing, last_speed)) = self.last_clock {
            let now = Instant::now();
            let elapsed_ms = if last_playing {
                now.saturating_duration_since(last_now).as_secs_f64() * 1000.0 * last_speed
            } else {
                0.0
            };
            let _ = self.poll_with_clock(last_pos + elapsed_ms, last_playing, last_speed, now);
        }
        self.armed = false;
        self.last_clock = None;
        std::mem::take(&mut self.recorded)
    }

    /// Disarms MIDI recording after draining queued events against the exact current playback clock.
    pub fn disarm_with_clock(
        &mut self,
        position_ms: f64,
        playing: bool,
        speed: f64,
    ) -> Vec<RecordedMidiNote> {
        self.disarm_with_snapshot(None, position_ms, playing, speed)
    }

    /// Disarms MIDI recording using an optional audio-callback DLL [`ClockSnapshot`].
    pub fn disarm_with_snapshot(
        &mut self,
        snapshot: Option<ClockSnapshot>,
        position_ms: f64,
        playing: bool,
        speed: f64,
    ) -> Vec<RecordedMidiNote> {
        let _ = self.poll_with_snapshot(snapshot, position_ms, playing, speed, Instant::now());
        self.armed = false;
        self.last_clock = None;
        std::mem::take(&mut self.recorded)
    }

    pub fn buffered_count(&self) -> usize {
        self.recorded.len()
    }

    /// Drains live MIDI input events at `1.0×` playback speed.
    pub fn poll(&mut self, position_ms: f64, playing: bool) -> Vec<MidiInputEvent> {
        self.poll_with_clock(position_ms, playing, 1.0, Instant::now())
    }

    /// Drains live MIDI input events using linear fallback timing (`poll_with_snapshot(None, ...)`).
    pub fn poll_with_clock(
        &mut self,
        position_ms: f64,
        playing: bool,
        speed: f64,
        now: Instant,
    ) -> Vec<MidiInputEvent> {
        self.poll_with_snapshot(None, position_ms, playing, speed, now)
    }

    /// Drains live MIDI input events using the audio callback's DLL [`ClockSnapshot`] when
    /// available, updating both the continuous [`RetrospectiveRing`] and armed buffer.
    pub fn poll_with_snapshot(
        &mut self,
        snapshot: Option<ClockSnapshot>,
        position_ms: f64,
        playing: bool,
        speed: f64,
        now: Instant,
    ) -> Vec<MidiInputEvent> {
        let eff_speed = if speed.is_finite() && speed > 0.0 {
            speed
        } else {
            1.0
        };
        let prev_clock = self.last_clock;
        self.last_clock = Some((now, position_ms.max(0.0), playing, eff_speed));

        let stamped_events = self.listener.drain();
        self.port_index = self.listener.port_index;
        self.port_name = self.listener.port_name.clone();

        let mut events = Vec::with_capacity(stamped_events.len());
        for stamped in stamped_events {
            events.push(stamped.event);
            let mut event_ms = if let Some(snap) = snapshot.filter(|s| s.playing) {
                snap.instant_to_song_ms(stamped.received_at)
            } else if playing {
                let age_ms = now
                    .saturating_duration_since(stamped.received_at)
                    .as_secs_f64()
                    * 1000.0
                    * eff_speed;
                (position_ms - age_ms).max(0.0)
            } else {
                position_ms.max(0.0)
            };
            if playing
                && let Some((prev_instant, prev_pos, prev_playing, _)) = prev_clock
                && !prev_playing
                && stamped.received_at >= prev_instant
            {
                event_ms = event_ms.max(prev_pos);
            }

            match stamped.event {
                MidiInputEvent::NoteOn {
                    channel,
                    pitch,
                    velocity,
                } if velocity > 0 => {
                    let note = RecordedMidiNote {
                        time_ms: event_ms,
                        pitch,
                        velocity,
                        channel,
                        duration_ms: None,
                    };
                    self.retrospective.push_midi_note_on(note);
                    if self.armed && playing {
                        self.recorded.push(note);
                    }
                }
                MidiInputEvent::NoteOn {
                    channel,
                    pitch,
                    velocity: 0,
                }
                | MidiInputEvent::NoteOff { channel, pitch } => {
                    self.retrospective.close_midi_note(channel, pitch, event_ms);
                    if self.armed && playing {
                        Self::close_open_note(&mut self.recorded, channel, pitch, event_ms);
                    }
                }
                _ => {}
            }
        }
        events
    }

    fn close_open_note(notes: &mut [RecordedMidiNote], channel: u8, pitch: u8, end_ms: f64) {
        if let Some(open) = notes
            .iter_mut()
            .rev()
            .find(|n| n.channel == channel && n.pitch == pitch && n.duration_ms.is_none())
        {
            open.duration_ms = Some((end_ms - open.time_ms).max(1.0));
        }
    }

    /// Records a simulated or virtual MIDI event at `time_ms` into retrospective (always)
    /// and armed capture (when armed).
    pub fn record_event(&mut self, time_ms: f64, event: MidiInputEvent) {
        self.record_retrospective_event(time_ms, event);
        if !self.armed {
            return;
        }
        let clamped_ms = time_ms.max(0.0);
        match event {
            MidiInputEvent::NoteOn {
                channel,
                pitch,
                velocity,
            } if velocity > 0 => {
                self.recorded.push(RecordedMidiNote {
                    time_ms: clamped_ms,
                    pitch,
                    velocity,
                    channel,
                    duration_ms: None,
                });
            }
            MidiInputEvent::NoteOn {
                channel,
                pitch,
                velocity: 0,
            }
            | MidiInputEvent::NoteOff { channel, pitch } => {
                Self::close_open_note(&mut self.recorded, channel, pitch, clamped_ms);
            }
            _ => {}
        }
    }

    /// Records an unarmed virtual MIDI event directly into [`RetrospectiveRing`].
    pub fn record_retrospective_event(&mut self, time_ms: f64, event: MidiInputEvent) {
        let clamped_ms = time_ms.max(0.0);
        match event {
            MidiInputEvent::NoteOn {
                channel,
                pitch,
                velocity,
            } if velocity > 0 => {
                self.retrospective.push_midi_note_on(RecordedMidiNote {
                    time_ms: clamped_ms,
                    pitch,
                    velocity,
                    channel,
                    duration_ms: None,
                });
            }
            MidiInputEvent::NoteOn {
                channel,
                pitch,
                velocity: 0,
            }
            | MidiInputEvent::NoteOff { channel, pitch } => {
                self.retrospective.close_midi_note(channel, pitch, clamped_ms);
            }
            _ => {}
        }
    }

    /// Records a live unarmed keyboard tap into [`RetrospectiveRing`].
    pub fn record_retrospective_tap(&mut self, tap: RecordedTap) {
        self.retrospective.push_tap(tap);
    }

    /// Drains all buffered retrospective MIDI notes and keyboard taps.
    pub fn take_retrospective(&mut self) -> (Vec<RecordedMidiNote>, Vec<RecordedTap>) {
        self.retrospective.take_all()
    }
}

pub(crate) fn build_pitch_to_lane_map(
    composer: &Composer,
    drum_map: Option<&DrumMap>,
) -> HashMap<u8, SampleIdentity> {
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
        if let std::collections::hash_map::Entry::Vacant(e) = note_to_lane.entry(next_note) {
            e.insert(lane.clone());
            next_note = next_note.saturating_add(1);
        }
    }
    note_to_lane
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
    let note_to_lane = build_pitch_to_lane_map(composer, drum_map);

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

fn push_dedup_planned(
    planned: &mut Vec<(f64, SampleIdentity, i32)>,
    composer: &Composer,
    time_ms: f64,
    lane: &SampleIdentity,
    volume: i32,
) {
    if composer.hitsounds().lane(lane).is_none() {
        return;
    }
    let duplicate = planned
        .iter()
        .any(|(t, l, _)| l == lane && (*t - time_ms).abs() <= 2.0)
        || composer
            .hitsounds()
            .moments()
            .iter()
            .any(|m| (m.time().ms() - time_ms).abs() <= 0.5 && m.has_hit(lane));
    if !duplicate {
        planned.push((time_ms, lane.clone(), volume));
    }
}

fn insert_planned_hits(
    action_label: &'static str,
    composer: &mut Composer,
    planned: &[(f64, SampleIdentity, i32)],
) -> Result<usize, String> {
    if planned.is_empty() {
        return Ok(0);
    }
    if composer.history.is_transaction_open() {
        return Err("Finish the current edit before committing taps.".into());
    }
    let mut added_keys = Vec::with_capacity(planned.len());
    let _ = composer.history.apply(action_label, true, |h| {
        for (time_ms, lane, vol) in planned {
            let existing = h
                .moments()
                .iter()
                .enumerate()
                .filter(|(_, m)| (m.time().ms() - *time_ms).abs() <= 0.5)
                .min_by(|(_, a), (_, b)| {
                    (a.time().ms() - *time_ms)
                        .abs()
                        .total_cmp(&(b.time().ms() - *time_ms).abs())
                })
                .map(|(i, _)| dawsu_hitsounding::MomentId(i));
            let id = match existing {
                Some(id) => id,
                None => h.add_moment(SongTime::from_ms(*time_ms), *vol),
            };
            if !h.moment(id).has_hit(lane) {
                h.add_hit_with_volume(id, lane.clone(), *vol);
                added_keys.push(dawsu_editor_composer::HitKey {
                    moment: id.0,
                    lane: lane.clone(),
                });
            }
        }
        Ok::<_, std::convert::Infallible>(())
    });
    if !added_keys.is_empty() {
        composer.selected.replace(added_keys.clone());
    }
    Ok(added_keys.len())
}

/// Commits live keyboard lane taps into `composer` as a single `"Record taps"` undo step,
/// respecting the composer's active snap mode (Grid, Grid relative, Objects, or Snap off)
/// and preserving each tap's recorded volume.
pub fn commit_taps_to_composer(
    taps: &[RecordedTap],
    composer: &mut Composer,
) -> Result<usize, String> {
    if taps.is_empty() {
        return Ok(0);
    }
    let mut planned = Vec::with_capacity(taps.len());
    for tap in taps {
        let raw_time = tap.time_ms.max(0.0);
        let time_ms = if composer.snap_enabled {
            composer.snap(SongTime::from_ms(raw_time)).ms().max(0.0)
        } else {
            raw_time
        };
        push_dedup_planned(&mut planned, composer, time_ms, &tap.lane, tap.volume.clamp(1, 100));
    }
    insert_planned_hits("Record taps", composer, &planned)
}

/// Commits buffered retrospective MIDI notes and keyboard taps together into `composer` as
/// a single `"Capture retrospective"` Undo step.
pub fn commit_retrospective_to_composer(
    notes: &[RecordedMidiNote],
    taps: &[RecordedTap],
    composer: &mut Composer,
    drum_map: Option<&DrumMap>,
    snap_divisor: Option<u32>,
) -> Result<usize, String> {
    if notes.is_empty() && taps.is_empty() {
        return Ok(0);
    }
    let note_to_lane = build_pitch_to_lane_map(composer, drum_map);
    let mut planned = Vec::with_capacity(notes.len() + taps.len());
    for note in notes {
        let Some(lane) = map_pitch_to_lane(note.pitch, &note_to_lane) else {
            continue;
        };
        let raw_ms = note.time_ms.max(0.0);
        let time_ms = if snap_divisor.is_some() {
            quantize_ms(&composer.tempo, raw_ms, snap_divisor)
        } else if composer.snap_enabled {
            composer.snap(SongTime::from_ms(raw_ms)).ms().max(0.0)
        } else {
            raw_ms
        };
        let vol = ((f64::from(note.velocity) / 127.0) * 100.0)
            .round()
            .clamp(5.0, 100.0) as i32;
        push_dedup_planned(&mut planned, composer, time_ms, &lane, vol);
    }
    for tap in taps {
        let raw_ms = tap.time_ms.max(0.0);
        let time_ms = if snap_divisor.is_some() {
            quantize_ms(&composer.tempo, raw_ms, snap_divisor)
        } else if composer.snap_enabled {
            composer.snap(SongTime::from_ms(raw_ms)).ms().max(0.0)
        } else {
            raw_ms
        };
        push_dedup_planned(&mut planned, composer, time_ms, &tap.lane, tap.volume.clamp(1, 100));
    }
    insert_planned_hits("Capture retrospective", composer, &planned)
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
    let mut added = 0usize;
    for note in notes {
        let dur = note.duration_ms.unwrap_or(default_duration_ms).max(20.0);
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

    #[test]
    fn poll_preserves_distinct_capture_timestamps_and_disarm_drains_tail() {
        use std::time::Duration;

        let (tx, rx) = mpsc::channel();
        let mut rec = MidiRecorder::new();
        rec.armed = true;
        rec.listener.attach_test_receiver(rx);

        let poll_now = Instant::now();
        // Two hits arrived 80 ms and 20 ms BEFORE this poll at song time 1000 ms (1.0x speed).
        tx.send(TimestampedMidiEvent {
            received_at: poll_now - Duration::from_millis(80),
            event: MidiInputEvent::NoteOn {
                channel: 0,
                pitch: 60,
                velocity: 100,
            },
        })
        .unwrap();
        tx.send(TimestampedMidiEvent {
            received_at: poll_now - Duration::from_millis(20),
            event: MidiInputEvent::NoteOn {
                channel: 0,
                pitch: 64,
                velocity: 95,
            },
        })
        .unwrap();

        let drained = rec.poll_with_clock(1000.0, true, 1.0, poll_now);
        assert_eq!(drained.len(), 2);

        // A final hit arrives right before disarming with no intermediate poll.
        tx.send(TimestampedMidiEvent {
            received_at: Instant::now(),
            event: MidiInputEvent::NoteOn {
                channel: 0,
                pitch: 67,
                velocity: 110,
            },
        })
        .unwrap();

        let notes = rec.disarm_with_clock(1050.0, true, 1.0);
        assert_eq!(notes.len(), 3, "final queued hit must be drained before closing port");
        assert!((notes[0].time_ms - 920.0).abs() < 2.0, "got {}", notes[0].time_ms);
        assert!((notes[1].time_ms - 980.0).abs() < 2.0, "got {}", notes[1].time_ms);
        assert!((notes[2].time_ms - 1050.0).abs() < 25.0, "got {}", notes[2].time_ms);
        assert!(notes[1].time_ms - notes[0].time_ms > 50.0, "burst timestamps must stay distinct");
    }

    #[test]
    fn keyboard_taps_snap_deduplicate_and_commit_in_one_undo_step() {
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
        composer.snap_enabled = true;
        composer.spacing = SnapSpacing::fraction(4).unwrap();

        let lane_a = composer.lanes[0].clone();
        let lane_b = composer.lanes[1].clone();
        let before = composer.hitsounds().moments().len();
        let taps = vec![
            RecordedTap { time_ms: 1122.0, lane: lane_a.clone(), volume: 80 },
            // Duplicate within 2 ms of the snapped 1125 ms tap on the same lane -> deduplicated.
            RecordedTap { time_ms: 1126.0, lane: lane_a.clone(), volume: 80 },
            RecordedTap { time_ms: 1495.0, lane: lane_b.clone(), volume: 65 },
        ];
        let added = commit_taps_to_composer(&taps, &mut composer).unwrap();
        assert_eq!(added, 2);
        assert_eq!(composer.hitsounds().moments().len(), before + 2);
        let moments = composer.hitsounds().moments();
        let m1 = moments.iter().find(|m| (m.time().ms() - 1125.0).abs() < 1e-6).unwrap();
        assert!(m1.has_hit(&lane_a));
        assert_eq!(m1.hit_volume(&lane_a), 80);
        let m2 = moments.iter().find(|m| (m.time().ms() - 1500.0).abs() < 1e-6).unwrap();
        assert!(m2.has_hit(&lane_b));
        assert_eq!(m2.hit_volume(&lane_b), 65);

        // One Undo step removes every tapped hit together.
        composer.undo();
        assert_eq!(composer.hitsounds().moments().len(), before);
    }

    #[test]
    fn unarmed_persistent_listener_feeds_retrospective_note_durations_and_recovers_on_unplug() {
        use dawsu_audio::{AudioClockDll, monotonic_rel_ns};
        use std::time::Duration;

        let (tx, rx) = mpsc::channel();
        let mut rec = MidiRecorder::new();
        rec.listener.attach_test_receiver(rx);
        rec.arm(None).unwrap();
        rec.disarm();
        assert!(!rec.armed);
        assert!(rec.listener.is_connected());

        let mut dll = AudioClockDll::new(0.1);
        let base_now = Instant::now();
        let snap = dll.update(monotonic_rel_ns(base_now), 512, 48_000, 1000.0, 1.0, true);

        tx.send(TimestampedMidiEvent {
            received_at: base_now + Duration::from_millis(20),
            event: MidiInputEvent::NoteOn {
                channel: 0,
                pitch: 36,
                velocity: 110,
            },
        })
        .unwrap();
        tx.send(TimestampedMidiEvent {
            received_at: base_now + Duration::from_millis(160),
            event: MidiInputEvent::NoteOff {
                channel: 0,
                pitch: 36,
            },
        })
        .unwrap();

        let drained = rec.poll_with_snapshot(Some(snap), 1000.0, true, 1.0, base_now + Duration::from_millis(180));
        assert_eq!(drained.len(), 2);
        assert_eq!(rec.retrospective.midi_len(), 1);

        // Hot-unplug recovers cleanly without panicking.
        drop(tx);
        assert!(rec.poll_with_snapshot(Some(snap), 1200.0, true, 1.0, Instant::now()).is_empty());
        assert!(!rec.listener.is_connected());
        assert!(rec.listener.last_error.is_some());

        let (notes, taps) = rec.take_retrospective();
        assert_eq!(notes.len(), 1);
        assert!(taps.is_empty());
        assert!((notes[0].time_ms - 1020.0).abs() < 1.0);
        let dur = notes[0].duration_ms.expect("NoteOff duration recorded");
        assert!((dur - 140.0).abs() < 1.0, "got dur={dur}");

        // Commits into KeysoundProject preserving captured NoteOff duration.
        let mut ks = KeysoundProject::new("audio.mp3", "Keysounds");
        let tid = ks.document.add_track("Lead", 0, false);
        assert_eq!(commit_to_keysounds(&notes, &mut ks, tid, 200.0), 1);
        assert!((ks.document.tracks[0].notes[0].duration_ms() - 140.0).abs() < 1.0);
    }

    #[test]
    fn commit_retrospective_merges_midi_and_keyboard_taps_in_one_undo_step() {
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
        composer.snap_enabled = true;
        composer.spacing = SnapSpacing::fraction(4).unwrap();

        let before = composer.hitsounds().moments().len();
        let notes = vec![RecordedMidiNote {
            time_ms: 1122.0,
            pitch: 36,
            velocity: 100,
            channel: 9,
            duration_ms: Some(150.0),
        }];
        let taps = vec![RecordedTap {
            time_ms: 1498.0,
            lane: composer.lanes[1].clone(),
            volume: 75,
        }];
        let committed = commit_retrospective_to_composer(&notes, &taps, &mut composer, None, None).unwrap();
        assert_eq!(committed, 2);
        assert_eq!(composer.hitsounds().moments().len(), before + 2);

        composer.undo();
        assert_eq!(composer.hitsounds().moments().len(), before);
    }
}
