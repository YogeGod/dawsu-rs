//! Pure-Rust Standard MIDI File (SMF Format 0 and Format 1) parser and millisecond timing converter.
//!
//! Uses only the Rust standard library (foundation invariant 1).

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// Default MIDI tempo when no Set Tempo meta-event is present (500,000 µs/quarter = 120 BPM).
pub const DEFAULT_US_PER_QUARTER: u32 = 500_000;

/// Errors returned while parsing a Standard MIDI File.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiError {
    UnexpectedEof,
    InvalidHeader(&'static str),
    UnsupportedFormat(u16),
    InvalidVariableLength,
    MissingRunningStatus { offset: usize },
    InvalidStatus { status: u8, offset: usize },
    TruncatedTrack { track: usize },
}

impl fmt::Display for MidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MidiError::UnexpectedEof => write!(f, "unexpected end of MIDI data"),
            MidiError::InvalidHeader(msg) => write!(f, "invalid MIDI header: {msg}"),
            MidiError::UnsupportedFormat(fmt_id) => write!(f, "unsupported SMF format {fmt_id} (only Format 0 and Format 1 are supported)"),
            MidiError::InvalidVariableLength => write!(f, "invalid MIDI variable-length quantity"),
            MidiError::MissingRunningStatus { offset } => write!(f, "data byte encountered with no running status at offset {offset}"),
            MidiError::InvalidStatus { status, offset } => write!(f, "invalid MIDI status byte 0x{status:02X} at offset {offset}"),
            MidiError::TruncatedTrack { track } => write!(f, "MIDI track {track} is truncated"),
        }
    }
}

impl std::error::Error for MidiError {}

/// SMF file format (Format 0 or Format 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiFormat {
    SingleTrack,
    MultiTrack,
}

/// Time division from the SMF header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeDivision {
    /// Metrical time: ticks per quarter note (PPQN).
    TicksPerQuarter(u16),
    /// SMPTE timecode: frames per second (24, 25, 29 for 29.97, or 30) and ticks per frame.
    Smpte { fps: u8, ticks_per_frame: u8 },
}

/// A parsed raw MIDI event with its absolute tick position in its track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiEvent {
    pub tick: u64,
    pub kind: MidiEventKind,
}

/// Payload of a MIDI event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiEventKind {
    NoteOff { channel: u8, pitch: u8, velocity: u8 },
    NoteOn { channel: u8, pitch: u8, velocity: u8 },
    PolyPressure { channel: u8, pitch: u8, pressure: u8 },
    ControlChange { channel: u8, controller: u8, value: u8 },
    ProgramChange { channel: u8, program: u8 },
    ChannelPressure { channel: u8, pressure: u8 },
    PitchBend { channel: u8, value: u16 },
    TrackName(String),
    SetTempo { us_per_quarter: u32 },
    TimeSignature(TimeSignature),
    EndOfTrack,
    SysEx(Vec<u8>),
    Meta { meta_type: u8, data: Vec<u8> },
}

/// A MIDI time signature meta-event (`0xFF 0x58`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u8,
    /// Actual denominator (e.g. 4 for 4/4, 8 for 6/8), decoded from `2^dd`.
    pub denominator: u16,
    pub clocks_per_click: u8,
    pub thirty_seconds_per_quarter: u8,
}

/// One parsed raw MIDI track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiTrack {
    pub name: Option<String>,
    pub events: Vec<MidiEvent>,
}

/// A parsed Standard MIDI File.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiFile {
    pub format: MidiFormat,
    pub division: TimeDivision,
    pub tracks: Vec<MidiTrack>,
}

impl MidiFile {
    /// Parses a Standard MIDI File (Format 0 or Format 1) from raw bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, MidiError> {
        let mut cursor = Cursor::new(bytes);

        let header_magic = cursor.read_bytes(4)?;
        if header_magic != b"MThd" {
            return Err(MidiError::InvalidHeader("missing MThd magic"));
        }
        let header_len = cursor.read_u32_be()? as usize;
        if header_len < 6 {
            return Err(MidiError::InvalidHeader("header length shorter than 6 bytes"));
        }
        let format_raw = cursor.read_u16_be()?;
        let format = match format_raw {
            0 => MidiFormat::SingleTrack,
            1 => MidiFormat::MultiTrack,
            other => return Err(MidiError::UnsupportedFormat(other)),
        };
        let declared_tracks = cursor.read_u16_be()? as usize;
        let division_raw = cursor.read_u16_be()?;
        let division = if (division_raw & 0x8000) == 0 {
            let tpq = division_raw & 0x7FFF;
            if tpq == 0 {
                return Err(MidiError::InvalidHeader("ticks per quarter note cannot be zero"));
            }
            TimeDivision::TicksPerQuarter(tpq)
        } else {
            let fps_signed = ((division_raw >> 8) as u8) as i8;
            let fps = fps_signed.unsigned_abs();
            let ticks_per_frame = (division_raw & 0x00FF) as u8;
            if fps == 0 || ticks_per_frame == 0 {
                return Err(MidiError::InvalidHeader("SMPTE fps and ticks per frame must be non-zero"));
            }
            TimeDivision::Smpte { fps, ticks_per_frame }
        };

        if header_len > 6 {
            cursor.skip(header_len - 6)?;
        }

        let mut tracks = Vec::with_capacity(declared_tracks);
        while tracks.len() < declared_tracks && !cursor.is_eof() {
            let chunk_id = cursor.read_bytes(4)?;
            let chunk_len = cursor.read_u32_be()? as usize;
            let chunk_data = cursor.read_bytes(chunk_len).map_err(|_| MidiError::TruncatedTrack { track: tracks.len() })?;
            if chunk_id != b"MTrk" {
                continue;
            }
            let track = parse_track(chunk_data)?;
            tracks.push(track);
        }

        if tracks.len() < declared_tracks {
            return Err(MidiError::TruncatedTrack { track: tracks.len() });
        }

        Ok(MidiFile { format, division, tracks })
    }
}

fn parse_track(bytes: &[u8]) -> Result<MidiTrack, MidiError> {
    let mut cursor = Cursor::new(bytes);
    let mut events = Vec::new();
    let mut name: Option<String> = None;
    let mut tick: u64 = 0;
    let mut running_status: Option<u8> = None;

    while !cursor.is_eof() {
        let delta = cursor.read_vlq()?;
        tick = tick.saturating_add(delta as u64);

        let peek = cursor.peek_u8()?;
        let status = if peek >= 0x80 {
            let b = cursor.read_u8()?;
            if b < 0xF0 {
                running_status = Some(b);
            } else {
                running_status = None;
            }
            b
        } else {
            running_status.ok_or(MidiError::MissingRunningStatus { offset: cursor.pos })?
        };

        let kind = match status & 0xF0 {
            0x80 => {
                let pitch = cursor.read_u8()? & 0x7F;
                let velocity = cursor.read_u8()? & 0x7F;
                MidiEventKind::NoteOff { channel: status & 0x0F, pitch, velocity }
            }
            0x90 => {
                let pitch = cursor.read_u8()? & 0x7F;
                let velocity = cursor.read_u8()? & 0x7F;
                MidiEventKind::NoteOn { channel: status & 0x0F, pitch, velocity }
            }
            0xA0 => {
                let pitch = cursor.read_u8()? & 0x7F;
                let pressure = cursor.read_u8()? & 0x7F;
                MidiEventKind::PolyPressure { channel: status & 0x0F, pitch, pressure }
            }
            0xB0 => {
                let controller = cursor.read_u8()? & 0x7F;
                let value = cursor.read_u8()? & 0x7F;
                MidiEventKind::ControlChange { channel: status & 0x0F, controller, value }
            }
            0xC0 => {
                let program = cursor.read_u8()? & 0x7F;
                MidiEventKind::ProgramChange { channel: status & 0x0F, program }
            }
            0xD0 => {
                let pressure = cursor.read_u8()? & 0x7F;
                MidiEventKind::ChannelPressure { channel: status & 0x0F, pressure }
            }
            0xE0 => {
                let lsb = (cursor.read_u8()? & 0x7F) as u16;
                let msb = (cursor.read_u8()? & 0x7F) as u16;
                MidiEventKind::PitchBend { channel: status & 0x0F, value: lsb | (msb << 7) }
            }
            0xF0 => match status {
                0xF0 | 0xF7 => {
                    let len = cursor.read_vlq()? as usize;
                    let data = cursor.read_bytes(len)?.to_vec();
                    MidiEventKind::SysEx(data)
                }
                0xFF => {
                    let meta_type = cursor.read_u8()?;
                    let len = cursor.read_vlq()? as usize;
                    let data = cursor.read_bytes(len)?;
                    match meta_type {
                        0x03 => {
                            let s = String::from_utf8_lossy(data).into_owned();
                            if name.is_none() {
                                name = Some(s.clone());
                            }
                            MidiEventKind::TrackName(s)
                        }
                        0x2F => MidiEventKind::EndOfTrack,
                        0x51 if data.len() == 3 => {
                            let us = ((data[0] as u32) << 16) | ((data[1] as u32) << 8) | (data[2] as u32);
                            MidiEventKind::SetTempo { us_per_quarter: us.max(1) }
                        }
                        0x58 if data.len() == 4 => {
                            let denom_shift = data[1].min(15);
                            MidiEventKind::TimeSignature(TimeSignature {
                                numerator: data[0],
                                denominator: 1u16 << denom_shift,
                                clocks_per_click: data[2],
                                thirty_seconds_per_quarter: data[3],
                            })
                        }
                        _ => MidiEventKind::Meta { meta_type, data: data.to_vec() },
                    }
                }
                _ => return Err(MidiError::InvalidStatus { status, offset: cursor.pos.saturating_sub(1) }),
            },
            _ => return Err(MidiError::InvalidStatus { status, offset: cursor.pos.saturating_sub(1) }),
        };

        let is_eot = matches!(kind, MidiEventKind::EndOfTrack);
        events.push(MidiEvent { tick, kind });
        if is_eot {
            break;
        }
    }

    Ok(MidiTrack { name, events })
}

/// A single tempo change at an absolute tick position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoChange {
    pub tick: u64,
    pub time_ms: f64,
    pub us_per_quarter: u32,
}

impl TempoChange {
    /// Tempo in beats (quarter notes) per minute.
    pub fn bpm(self) -> f64 {
        60_000_000.0 / (self.us_per_quarter as f64)
    }
}

/// Converts between MIDI ticks and millisecond timestamps using a parsed tempo map.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    division: TimeDivision,
    changes: Vec<TempoChange>,
}

impl TempoMap {
    /// Builds a tempo map from a time division and an arbitrary sequence of `(tick, us_per_quarter)` points.
    pub fn from_changes(division: TimeDivision, raw_changes: impl IntoIterator<Item = (u64, u32)>) -> Self {
        let mut by_tick: BTreeMap<u64, u32> = BTreeMap::new();
        for (tick, us) in raw_changes {
            by_tick.insert(tick, us.max(1));
        }
        by_tick.entry(0).or_insert(DEFAULT_US_PER_QUARTER);

        let mut changes: Vec<TempoChange> = Vec::with_capacity(by_tick.len());
        let mut acc_ms = 0.0_f64;
        let mut prev_tick = 0_u64;
        let mut prev_us = DEFAULT_US_PER_QUARTER;

        for (i, (tick, us_per_quarter)) in by_tick.into_iter().enumerate() {
            if i > 0 {
                acc_ms += delta_ticks_to_ms(division, tick - prev_tick, prev_us);
            }
            changes.push(TempoChange { tick, time_ms: acc_ms, us_per_quarter });
            prev_tick = tick;
            prev_us = us_per_quarter;
        }

        Self { division, changes }
    }

    pub fn division(&self) -> TimeDivision {
        self.division
    }

    pub fn changes(&self) -> &[TempoChange] {
        &self.changes
    }

    /// Converts an absolute MIDI tick to milliseconds.
    pub fn tick_to_ms(&self, tick: u64) -> f64 {
        match self.division {
            TimeDivision::Smpte { fps, ticks_per_frame } => {
                let effective_fps = if fps == 29 { 29.97 } else { fps as f64 };
                (tick as f64) * 1000.0 / (effective_fps * (ticks_per_frame as f64))
            }
            TimeDivision::TicksPerQuarter(_) => {
                let idx = match self.changes.binary_search_by_key(&tick, |c| c.tick) {
                    Ok(i) => i,
                    Err(i) => i.saturating_sub(1),
                };
                let seg = self.changes[idx];
                seg.time_ms + delta_ticks_to_ms(self.division, tick - seg.tick, seg.us_per_quarter)
            }
        }
    }

    /// Converts a millisecond timestamp to a fractional MIDI tick.
    pub fn ms_to_tick(&self, ms: f64) -> f64 {
        if ms <= 0.0 {
            return 0.0;
        }
        match self.division {
            TimeDivision::Smpte { fps, ticks_per_frame } => {
                let effective_fps = if fps == 29 { 29.97 } else { fps as f64 };
                ms * effective_fps * (ticks_per_frame as f64) / 1000.0
            }
            TimeDivision::TicksPerQuarter(tpq) => {
                let idx = self.changes.partition_point(|c| c.time_ms <= ms).saturating_sub(1);
                let seg = self.changes[idx];
                let delta_ms = (ms - seg.time_ms).max(0.0);
                (seg.tick as f64) + delta_ms * (tpq as f64) * 1000.0 / (seg.us_per_quarter as f64)
            }
        }
    }

    /// Returns the BPM in effect at `tick`.
    pub fn bpm_at_tick(&self, tick: u64) -> f64 {
        let idx = match self.changes.binary_search_by_key(&tick, |c| c.tick) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        self.changes[idx].bpm()
    }
}

fn delta_ticks_to_ms(division: TimeDivision, delta_ticks: u64, us_per_quarter: u32) -> f64 {
    match division {
        TimeDivision::TicksPerQuarter(tpq) => {
            (delta_ticks as f64) * (us_per_quarter as f64) / ((tpq as f64) * 1000.0)
        }
        TimeDivision::Smpte { fps, ticks_per_frame } => {
            let effective_fps = if fps == 29 { 29.97 } else { fps as f64 };
            (delta_ticks as f64) * 1000.0 / (effective_fps * (ticks_per_frame as f64))
        }
    }
}

/// A paired MIDI note with both tick and millisecond boundaries.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiNote {
    pub channel: u8,
    pub pitch: u8,
    pub velocity: u8,
    pub start_tick: u64,
    pub end_tick: u64,
    pub start_ms: f64,
    pub end_ms: f64,
}

/// A program change event with millisecond timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedProgramChange {
    pub tick: u64,
    pub time_ms: f64,
    pub channel: u8,
    pub program: u8,
}

/// A Sustain Pedal (CC64) event with millisecond timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedSustainEvent {
    pub tick: u64,
    pub time_ms: f64,
    pub channel: u8,
    pub value: u8,
    pub pressed: bool,
}

/// A time signature event with millisecond timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimedTimeSignature {
    pub tick: u64,
    pub time_ms: f64,
    pub signature: TimeSignature,
}

/// One track converted to millisecond-timed notes and controller events.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedTrack {
    pub index: usize,
    pub name: Option<String>,
    pub notes: Vec<MidiNote>,
    pub program_changes: Vec<TimedProgramChange>,
    pub sustain_events: Vec<TimedSustainEvent>,
}

/// A MIDI file with all events converted to millisecond timestamps via its tempo map.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedMidi {
    pub format: MidiFormat,
    pub division: TimeDivision,
    pub tempo_map: TempoMap,
    pub time_signatures: Vec<TimedTimeSignature>,
    pub tracks: Vec<TimedTrack>,
}

#[derive(Debug, Clone, Copy)]
struct ActiveNote {
    start_tick: u64,
    velocity: u8,
    released_under_sustain: bool,
}

impl TimedMidi {
    /// Parses SMF bytes and immediately converts all events into millisecond timestamps.
    pub fn parse(bytes: &[u8]) -> Result<Self, MidiError> {
        let midi = MidiFile::parse(bytes)?;
        Ok(Self::from_midi(&midi))
    }

    /// Converts a parsed [`MidiFile`] into millisecond timestamps using its tempo map.
    pub fn from_midi(midi: &MidiFile) -> Self {
        let mut raw_tempos = Vec::new();
        let mut raw_time_sigs: Vec<(u64, TimeSignature)> = Vec::new();

        for track in &midi.tracks {
            for ev in &track.events {
                match ev.kind {
                    MidiEventKind::SetTempo { us_per_quarter } => {
                        raw_tempos.push((ev.tick, us_per_quarter));
                    }
                    MidiEventKind::TimeSignature(sig) => {
                        raw_time_sigs.push((ev.tick, sig));
                    }
                    _ => {}
                }
            }
        }

        let tempo_map = TempoMap::from_changes(midi.division, raw_tempos);

        raw_time_sigs.sort_by_key(|(tick, _)| *tick);
        let time_signatures = raw_time_sigs
            .into_iter()
            .map(|(tick, signature)| TimedTimeSignature {
                tick,
                time_ms: tempo_map.tick_to_ms(tick),
                signature,
            })
            .collect();

        let tracks = midi
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| build_timed_track(index, track, &tempo_map))
            .collect();

        Self {
            format: midi.format,
            division: midi.division,
            tempo_map,
            time_signatures,
            tracks,
        }
    }
}

fn build_timed_track(index: usize, track: &MidiTrack, tempo_map: &TempoMap) -> TimedTrack {
    let mut notes = Vec::new();
    let mut program_changes = Vec::new();
    let mut sustain_events = Vec::new();

    // Active notes keyed by (channel, pitch).
    let mut active: BTreeMap<(u8, u8), VecDeque<ActiveNote>> = BTreeMap::new();
    let mut sustain_down = [false; 16];
    let mut last_tick: u64 = 0;

    let emit_note = |notes: &mut Vec<MidiNote>, channel: u8, pitch: u8, active_note: ActiveNote, end_tick: u64| {
        let end_tick = end_tick.max(active_note.start_tick);
        notes.push(MidiNote {
            channel,
            pitch,
            velocity: active_note.velocity,
            start_tick: active_note.start_tick,
            end_tick,
            start_ms: tempo_map.tick_to_ms(active_note.start_tick),
            end_ms: tempo_map.tick_to_ms(end_tick),
        });
    };

    for ev in &track.events {
        last_tick = last_tick.max(ev.tick);
        match ev.kind {
            MidiEventKind::NoteOn { channel, pitch, velocity } if velocity > 0 => {
                let queue = active.entry((channel, pitch)).or_default();
                // If an earlier note on the same pitch was being held only by sustain, close it on retrigger.
                if let Some(front) = queue.front()
                    && front.released_under_sustain
                    && let Some(old) = queue.pop_front()
                {
                    emit_note(&mut notes, channel, pitch, old, ev.tick);
                }
                queue.push_back(ActiveNote {
                    start_tick: ev.tick,
                    velocity,
                    released_under_sustain: false,
                });
            }
            MidiEventKind::NoteOff { channel, pitch, .. }
            | MidiEventKind::NoteOn { channel, pitch, velocity: 0 } => {
                if let Some(queue) = active.get_mut(&(channel, pitch)) {
                    if sustain_down[usize::from(channel & 0x0F)] {
                        if let Some(note) = queue.iter_mut().find(|n| !n.released_under_sustain) {
                            note.released_under_sustain = true;
                        }
                    } else if let Some(old) = queue.pop_front() {
                        emit_note(&mut notes, channel, pitch, old, ev.tick);
                    }
                }
            }
            MidiEventKind::ProgramChange { channel, program } => {
                program_changes.push(TimedProgramChange {
                    tick: ev.tick,
                    time_ms: tempo_map.tick_to_ms(ev.tick),
                    channel,
                    program,
                });
            }
            MidiEventKind::ControlChange { channel, controller: 64, value } => {
                let pressed = value >= 64;
                let ch_idx = usize::from(channel & 0x0F);
                let was_down = sustain_down[ch_idx];
                sustain_down[ch_idx] = pressed;
                sustain_events.push(TimedSustainEvent {
                    tick: ev.tick,
                    time_ms: tempo_map.tick_to_ms(ev.tick),
                    channel,
                    value,
                    pressed,
                });
                if was_down && !pressed {
                    // Release all notes on this channel that were held only by sustain.
                    for (&(ch, pitch), queue) in &mut active {
                        if ch != channel {
                            continue;
                        }
                        let mut kept = VecDeque::with_capacity(queue.len());
                        while let Some(note) = queue.pop_front() {
                            if note.released_under_sustain {
                                emit_note(&mut notes, ch, pitch, note, ev.tick);
                            } else {
                                kept.push_back(note);
                            }
                        }
                        *queue = kept;
                    }
                }
            }
            _ => {}
        }
    }

    // Close any dangling Note On events at the end of the track.
    for ((channel, pitch), mut queue) in active {
        while let Some(note) = queue.pop_front() {
            emit_note(&mut notes, channel, pitch, note, last_tick);
        }
    }

    notes.sort_by(|a, b| {
        a.start_tick
            .cmp(&b.start_tick)
            .then_with(|| a.pitch.cmp(&b.pitch))
            .then_with(|| a.channel.cmp(&b.channel))
    });

    TimedTrack {
        index,
        name: track.name.clone(),
        notes,
        program_changes,
        sustain_events,
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek_u8(&self) -> Result<u8, MidiError> {
        self.bytes.get(self.pos).copied().ok_or(MidiError::UnexpectedEof)
    }

    fn read_u8(&mut self) -> Result<u8, MidiError> {
        let b = self.peek_u8()?;
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], MidiError> {
        let end = self.pos.checked_add(len).ok_or(MidiError::UnexpectedEof)?;
        if end > self.bytes.len() {
            return Err(MidiError::UnexpectedEof);
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn skip(&mut self, len: usize) -> Result<(), MidiError> {
        let _ = self.read_bytes(len)?;
        Ok(())
    }

    fn read_u16_be(&mut self) -> Result<u16, MidiError> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn read_u32_be(&mut self) -> Result<u32, MidiError> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_vlq(&mut self) -> Result<u32, MidiError> {
        let mut value: u32 = 0;
        for _ in 0..4 {
            let b = self.read_u8()?;
            value = (value << 7) | ((b & 0x7F) as u32);
            if (b & 0x80) == 0 {
                return Ok(value);
            }
        }
        Err(MidiError::InvalidVariableLength)
    }
}

fn write_vlq(out: &mut Vec<u8>, mut value: u64) {
    let mut buf = [0u8; 8];
    let mut n = 0;
    buf[n] = (value & 0x7F) as u8;
    n += 1;
    value >>= 7;
    while value > 0 {
        buf[n] = ((value & 0x7F) as u8) | 0x80;
        n += 1;
        value >>= 7;
    }
    for i in (0..n).rev() {
        out.push(buf[i]);
    }
}

fn encode_track(track: &MidiTrack) -> Vec<u8> {
    let mut events: Vec<(u64, usize, &MidiEventKind)> = track
        .events
        .iter()
        .enumerate()
        .filter(|(_, e)| !matches!(e.kind, MidiEventKind::EndOfTrack))
        .map(|(i, e)| (e.tick, i, &e.kind))
        .collect();
    events.sort_by_key(|(tick, idx, _)| (*tick, *idx));

    let mut body = Vec::new();
    let mut last_tick = 0u64;

    // Emit TrackName at tick 0 if set and not already present as an event.
    let has_track_name = events
        .iter()
        .any(|(_, _, k)| matches!(k, MidiEventKind::TrackName(_)));
    if !has_track_name {
        if let Some(name) = &track.name {
            let bytes = name.as_bytes();
            let len = bytes.len().min(u32::MAX as usize);
            write_vlq(&mut body, 0);
            body.extend_from_slice(&[0xFF, 0x03]);
            write_vlq(&mut body, len as u64);
            body.extend_from_slice(&bytes[..len]);
        }
    }

    for (tick, _, kind) in events {
        let delta = tick.saturating_sub(last_tick);
        last_tick = tick;
        write_vlq(&mut body, delta);
        match kind {
            MidiEventKind::NoteOff { channel, pitch, velocity } => {
                body.push(0x80 | (channel & 0x0F));
                body.push(pitch & 0x7F);
                body.push(velocity & 0x7F);
            }
            MidiEventKind::NoteOn { channel, pitch, velocity } => {
                body.push(0x90 | (channel & 0x0F));
                body.push(pitch & 0x7F);
                body.push(velocity & 0x7F);
            }
            MidiEventKind::PolyPressure { channel, pitch, pressure } => {
                body.push(0xA0 | (channel & 0x0F));
                body.push(pitch & 0x7F);
                body.push(pressure & 0x7F);
            }
            MidiEventKind::ControlChange { channel, controller, value } => {
                body.push(0xB0 | (channel & 0x0F));
                body.push(controller & 0x7F);
                body.push(value & 0x7F);
            }
            MidiEventKind::ProgramChange { channel, program } => {
                body.push(0xC0 | (channel & 0x0F));
                body.push(program & 0x7F);
            }
            MidiEventKind::ChannelPressure { channel, pressure } => {
                body.push(0xD0 | (channel & 0x0F));
                body.push(pressure & 0x7F);
            }
            MidiEventKind::PitchBend { channel, value } => {
                let v = value & 0x3FFF;
                body.push(0xE0 | (channel & 0x0F));
                body.push((v & 0x7F) as u8);
                body.push(((v >> 7) & 0x7F) as u8);
            }
            MidiEventKind::TrackName(name) => {
                let bytes = name.as_bytes();
                body.extend_from_slice(&[0xFF, 0x03]);
                write_vlq(&mut body, bytes.len() as u64);
                body.extend_from_slice(bytes);
            }
            MidiEventKind::SetTempo { us_per_quarter } => {
                let b = us_per_quarter.to_be_bytes();
                body.extend_from_slice(&[0xFF, 0x51, 0x03, b[1], b[2], b[3]]);
            }
            MidiEventKind::TimeSignature(sig) => {
                let dd = (sig.denominator.max(1) as f64).log2().round().clamp(0.0, 255.0) as u8;
                body.extend_from_slice(&[
                    0xFF,
                    0x58,
                    0x04,
                    sig.numerator,
                    dd,
                    sig.clocks_per_click,
                    sig.thirty_seconds_per_quarter,
                ]);
            }
            MidiEventKind::EndOfTrack => {}
            MidiEventKind::SysEx(data) => {
                body.push(0xF0);
                write_vlq(&mut body, data.len() as u64);
                body.extend_from_slice(data);
            }
            MidiEventKind::Meta { meta_type, data } => {
                body.push(0xFF);
                body.push(*meta_type);
                write_vlq(&mut body, data.len() as u64);
                body.extend_from_slice(data);
            }
        }
    }

    // End of Track
    write_vlq(&mut body, 0);
    body.extend_from_slice(&[0xFF, 0x2F, 0x00]);
    body
}

impl MidiFile {
    /// Encodes this [`MidiFile`] into Standard MIDI File bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let format_raw: u16 = match self.format {
            MidiFormat::SingleTrack => 0,
            MidiFormat::MultiTrack => 1,
        };
        let division_raw: u16 = match self.division {
            TimeDivision::TicksPerQuarter(ppq) => ppq & 0x7FFF,
            TimeDivision::Smpte { fps, ticks_per_frame } => {
                let signed_fps = -(fps as i8) as u8;
                ((signed_fps as u16) << 8) | (ticks_per_frame as u16)
            }
        };
        let mut out = Vec::new();
        out.extend_from_slice(b"MThd");
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&format_raw.to_be_bytes());
        out.extend_from_slice(&(self.tracks.len() as u16).to_be_bytes());
        out.extend_from_slice(&division_raw.to_be_bytes());
        for track in &self.tracks {
            let chunk = encode_track(track);
            out.extend_from_slice(b"MTrk");
            out.extend_from_slice(&(chunk.len() as u32).to_be_bytes());
            out.extend_from_slice(&chunk);
        }
        out
    }
}

impl TimedMidi {
    /// Converts this millisecond-timed MIDI back into a Standard MIDI File.
    pub fn to_midi(&self) -> MidiFile {
        let mut tracks = Vec::with_capacity(self.tracks.len() + 1);
        let mut tempo_events = Vec::new();
        for tc in self.tempo_map.changes() {
            tempo_events.push(MidiEvent {
                tick: tc.tick,
                kind: MidiEventKind::SetTempo {
                    us_per_quarter: tc.us_per_quarter,
                },
            });
        }
        for ts in &self.time_signatures {
            tempo_events.push(MidiEvent {
                tick: ts.tick,
                kind: MidiEventKind::TimeSignature(ts.signature),
            });
        }
        tracks.push(MidiTrack {
            name: Some("Tempo".into()),
            events: tempo_events,
        });

        for t in &self.tracks {
            if t.notes.is_empty() && t.program_changes.is_empty() && t.sustain_events.is_empty() {
                continue;
            }
            let mut events = Vec::new();
            for pc in &t.program_changes {
                events.push(MidiEvent {
                    tick: self.tempo_map.ms_to_tick(pc.time_ms).round().max(0.0) as u64,
                    kind: MidiEventKind::ProgramChange {
                        channel: pc.channel,
                        program: pc.program,
                    },
                });
            }
            for note in &t.notes {
                let start_tick = self.tempo_map.ms_to_tick(note.start_ms).round().max(0.0) as u64;
                let end_tick = self
                    .tempo_map
                    .ms_to_tick(note.end_ms)
                    .round()
                    .max(start_tick as f64 + 1.0) as u64;
                events.push(MidiEvent {
                    tick: start_tick,
                    kind: MidiEventKind::NoteOn {
                        channel: note.channel,
                        pitch: note.pitch,
                        velocity: note.velocity.clamp(1, 127),
                    },
                });
                events.push(MidiEvent {
                    tick: end_tick,
                    kind: MidiEventKind::NoteOff {
                        channel: note.channel,
                        pitch: note.pitch,
                        velocity: 0,
                    },
                });
            }
            tracks.push(MidiTrack {
                name: t.name.clone(),
                events,
            });
        }

        MidiFile {
            format: MidiFormat::MultiTrack,
            division: self.division,
            tracks,
        }
    }

    /// Encodes this [`TimedMidi`] directly into Standard MIDI File bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_midi().to_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_vlq(out: &mut Vec<u8>, mut v: u32) {
        let mut buf = [0u8; 4];
        let mut n = 0;
        buf[n] = (v & 0x7F) as u8;
        n += 1;
        v >>= 7;
        while v > 0 {
            buf[n] = ((v & 0x7F) as u8) | 0x80;
            n += 1;
            v >>= 7;
        }
        for i in (0..n).rev() {
            out.push(buf[i]);
        }
    }

    fn build_track_chunk(body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + body.len());
        out.extend_from_slice(b"MTrk");
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(body);
        out
    }

    fn build_smf(format: u16, division: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"MThd");
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&format.to_be_bytes());
        out.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        out.extend_from_slice(&division.to_be_bytes());
        for t in tracks {
            out.extend_from_slice(&build_track_chunk(t));
        }
        out
    }

    fn approx_eq(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-6, "expected {a} ≈ {b}");
    }

    #[test]
    fn parses_format_1_multitrack_with_tempo_changes_and_time_signature() {
        // Track 0: tempo map (120 BPM at tick 0 -> 60 BPM at tick 480) and 3/4 time signature.
        let mut t0 = Vec::new();
        // Track name "Tempo"
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xFF, 0x03, 5]);
        t0.extend_from_slice(b"Tempo");
        // Time signature 3/4 (dd=2 -> 2^2 = 4)
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xFF, 0x58, 4, 3, 2, 24, 8]);
        // Tempo 120 BPM = 500,000 us = 0x07A120
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xFF, 0x51, 3, 0x07, 0xA1, 0x20]);
        // At tick 480 (1 quarter note = 500 ms), change to 60 BPM = 1,000,000 us = 0x0F4240
        push_vlq(&mut t0, 480);
        t0.extend_from_slice(&[0xFF, 0x51, 3, 0x0F, 0x42, 0x40]);
        // End of track
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xFF, 0x2F, 0]);

        // Track 1: Piano track with Program Change and 2 notes across the tempo change.
        let mut t1 = Vec::new();
        push_vlq(&mut t1, 0);
        t1.extend_from_slice(&[0xFF, 0x03, 5]);
        t1.extend_from_slice(b"Piano");
        // Program Change ch 0 -> 1
        push_vlq(&mut t1, 0);
        t1.extend_from_slice(&[0xC0, 1]);
        // Note On C4 (60) vel 100 at tick 0
        push_vlq(&mut t1, 0);
        t1.extend_from_slice(&[0x90, 60, 100]);
        // Note Off C4 at tick 480 (= 500 ms)
        push_vlq(&mut t1, 480);
        t1.extend_from_slice(&[0x80, 60, 0]);
        // Note On E4 (64) vel 90 at tick 480 (= 500 ms)
        push_vlq(&mut t1, 0);
        t1.extend_from_slice(&[0x90, 64, 90]);
        // Note Off E4 at tick 960 (= 500 + 1000 = 1500 ms at 60 BPM)
        push_vlq(&mut t1, 480);
        t1.extend_from_slice(&[0x80, 64, 0]);
        push_vlq(&mut t1, 0);
        t1.extend_from_slice(&[0xFF, 0x2F, 0]);

        let bytes = build_smf(1, 480, &[t0, t1]);
        let timed = TimedMidi::parse(&bytes).expect("valid format 1 midi");

        assert_eq!(timed.format, MidiFormat::MultiTrack);
        assert_eq!(timed.division, TimeDivision::TicksPerQuarter(480));
        assert_eq!(timed.time_signatures.len(), 1);
        assert_eq!(timed.time_signatures[0].signature.numerator, 3);
        assert_eq!(timed.time_signatures[0].signature.denominator, 4);

        assert_eq!(timed.tempo_map.changes().len(), 2);
        approx_eq(timed.tempo_map.bpm_at_tick(0), 120.0);
        approx_eq(timed.tempo_map.bpm_at_tick(480), 60.0);
        approx_eq(timed.tempo_map.ms_to_tick(1000.0), 720.0);

        assert_eq!(timed.tracks.len(), 2);
        assert_eq!(timed.tracks[0].name.as_deref(), Some("Tempo"));
        assert_eq!(timed.tracks[1].name.as_deref(), Some("Piano"));
        assert_eq!(timed.tracks[1].program_changes.len(), 1);
        assert_eq!(timed.tracks[1].program_changes[0].program, 1);

        let notes = &timed.tracks[1].notes;
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].pitch, 60);
        assert_eq!(notes[0].velocity, 100);
        approx_eq(notes[0].start_ms, 0.0);
        approx_eq(notes[0].end_ms, 500.0);

        assert_eq!(notes[1].pitch, 64);
        assert_eq!(notes[1].velocity, 90);
        approx_eq(notes[1].start_ms, 500.0);
        approx_eq(notes[1].end_ms, 1500.0);
    }

    #[test]
    fn running_status_and_note_on_velocity_zero_as_note_off() {
        // Format 0, 480 TPQ, default 120 BPM (500 ms per 480 ticks).
        let mut t0 = Vec::new();
        // Note On ch 2, pitch 60, vel 80 at tick 0
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0x92, 60, 80]);
        // Running status: Note On ch 2, pitch 64, vel 85 at tick 240
        push_vlq(&mut t0, 240);
        t0.extend_from_slice(&[64, 85]);
        // Running status: Note On ch 2, pitch 60, vel 0 (= Note Off) at tick 480
        push_vlq(&mut t0, 240);
        t0.extend_from_slice(&[60, 0]);
        // Running status: Note On ch 2, pitch 64, vel 0 (= Note Off) at tick 720
        push_vlq(&mut t0, 240);
        t0.extend_from_slice(&[64, 0]);
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xFF, 0x2F, 0]);

        let bytes = build_smf(0, 480, &[t0]);
        let timed = TimedMidi::parse(&bytes).expect("valid format 0 midi");

        assert_eq!(timed.format, MidiFormat::SingleTrack);
        let notes = &timed.tracks[0].notes;
        assert_eq!(notes.len(), 2);

        assert_eq!(notes[0].channel, 2);
        assert_eq!(notes[0].pitch, 60);
        approx_eq(notes[0].start_ms, 0.0);
        approx_eq(notes[0].end_ms, 500.0);

        assert_eq!(notes[1].channel, 2);
        assert_eq!(notes[1].pitch, 64);
        approx_eq(notes[1].start_ms, 250.0);
        approx_eq(notes[1].end_ms, 750.0);
    }

    #[test]
    fn sustain_pedal_cc64_extends_note_until_release() {
        let mut t0 = Vec::new();
        // Sustain down at tick 0 (CC 64 = 127)
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xB0, 64, 127]);
        // Note On pitch 60 at tick 0
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0x90, 60, 100]);
        // Note Off pitch 60 at tick 240 (held by sustain)
        push_vlq(&mut t0, 240);
        t0.extend_from_slice(&[0x80, 60, 0]);
        // Sustain up at tick 960 (CC 64 = 0)
        push_vlq(&mut t0, 720);
        t0.extend_from_slice(&[0xB0, 64, 0]);
        push_vlq(&mut t0, 0);
        t0.extend_from_slice(&[0xFF, 0x2F, 0]);

        let bytes = build_smf(0, 480, &[t0]);
        let timed = TimedMidi::parse(&bytes).expect("valid sustain midi");

        assert_eq!(timed.tracks[0].sustain_events.len(), 2);
        assert!(timed.tracks[0].sustain_events[0].pressed);
        assert!(!timed.tracks[0].sustain_events[1].pressed);

        assert_eq!(timed.tracks[0].notes.len(), 1);
        let note = &timed.tracks[0].notes[0];
        assert_eq!(note.start_tick, 0);
        assert_eq!(note.end_tick, 960);
        approx_eq(note.start_ms, 0.0);
        approx_eq(note.end_ms, 1000.0);
    }

    #[test]
    fn rejects_invalid_header_and_unsupported_format() {
        assert_eq!(MidiFile::parse(b"NOT_MIDI"), Err(MidiError::InvalidHeader("missing MThd magic")));
        let fmt2 = build_smf(2, 480, &[]);
        assert_eq!(MidiFile::parse(&fmt2), Err(MidiError::UnsupportedFormat(2)));
    }

    #[test]
    fn encodes_and_roundtrips_timed_midi_with_tempo_changes() {
        let timed = TimedMidi {
            format: MidiFormat::MultiTrack,
            division: TimeDivision::TicksPerQuarter(480),
            tempo_map: TempoMap::from_changes(
                TimeDivision::TicksPerQuarter(480),
                [(0, 500_000), (480, 1_000_000)],
            ),
            time_signatures: vec![TimedTimeSignature {
                tick: 0,
                time_ms: 0.0,
                signature: TimeSignature {
                    numerator: 4,
                    denominator: 4,
                    clocks_per_click: 24,
                    thirty_seconds_per_quarter: 8,
                },
            }],
            tracks: vec![TimedTrack {
                index: 0,
                name: Some("Keys".into()),
                notes: vec![
                    MidiNote {
                        channel: 0,
                        pitch: 60,
                        velocity: 100,
                        start_tick: 0,
                        end_tick: 480,
                        start_ms: 0.0,
                        end_ms: 500.0,
                    },
                    MidiNote {
                        channel: 0,
                        pitch: 64,
                        velocity: 90,
                        start_tick: 480,
                        end_tick: 960,
                        start_ms: 500.0,
                        end_ms: 1500.0,
                    },
                ],
                program_changes: vec![TimedProgramChange {
                    tick: 0,
                    time_ms: 0.0,
                    channel: 0,
                    program: 5,
                }],
                sustain_events: Vec::new(),
            }],
        };

        let bytes = timed.to_bytes();
        let reparsed = TimedMidi::parse(&bytes).expect("round-trip parse");
        assert_eq!(reparsed.tracks.len(), 2);
        assert_eq!(reparsed.tracks[1].name.as_deref(), Some("Keys"));
        assert_eq!(reparsed.tracks[1].notes.len(), 2);
        assert_eq!(reparsed.tracks[1].notes[0].pitch, 60);
        assert_eq!(reparsed.tracks[1].notes[0].velocity, 100);
        approx_eq(reparsed.tracks[1].notes[0].start_ms, 0.0);
        approx_eq(reparsed.tracks[1].notes[0].end_ms, 500.0);
        assert_eq!(reparsed.tracks[1].notes[1].pitch, 64);
        approx_eq(reparsed.tracks[1].notes[1].start_ms, 500.0);
        approx_eq(reparsed.tracks[1].notes[1].end_ms, 1500.0);
    }
}
