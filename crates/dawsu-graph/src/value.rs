//! Runtime values flowing across typed graph sockets.

use crate::ids::{AnalysisRange, SocketType};
use dawsu_foundation::digest::sha256_hex;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Planar decoded PCM buffer over `[start_ms, ...)`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
    pub start_ms: f64,
    pub silent: bool,
    /// Optional full-source mono peak supplied by source nodes for `normaliseOver = "song"`.
    pub full_source_peak: Option<f32>,
}

impl AudioBuffer {
    pub fn frame_count(&self) -> usize {
        self.channels.iter().map(Vec::len).min().unwrap_or(0)
    }

    pub fn byte_size(&self) -> usize {
        self.channels.iter().map(|c| c.len() * std::mem::size_of::<f32>()).sum()
    }
}

/// Magnitude spectrogram stored row-major as `frames * bins`.
#[derive(Debug, Clone, PartialEq)]
pub struct Spectrogram {
    pub magnitudes: Vec<f32>,
    pub frames: usize,
    pub bins: usize,
    pub window: usize,
    pub hop_samples: usize,
    pub sample_rate: u32,
    pub start_ms: f64,
}

impl Spectrogram {
    pub fn hop_ms(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.hop_samples as f64 * 1000.0 / f64::from(self.sample_rate)
        }
    }

    pub fn byte_size(&self) -> usize {
        self.magnitudes.len() * std::mem::size_of::<f32>()
    }
}

/// Time-aligned scalar curve (one value per `hop_ms`, with frame 0 at `start_ms`).
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub values: Vec<f64>,
    pub hop_ms: f64,
    pub start_ms: f64,
}

impl Curve {
    pub fn byte_size(&self) -> usize {
        self.values.len() * std::mem::size_of::<f64>()
    }
}

/// A discrete timestamped event with optional per-event curve measurements.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphEvent {
    pub time_ms: f64,
    /// Curve-frame timestamp before any onset bias is added, used by `events.measure`.
    pub detected_at_ms: f64,
    pub frame: usize,
    pub strength: f64,
    pub tag: String,
    pub measures: BTreeMap<String, f64>,
}

/// Canonical event list ordered by `time_ms`, then `tag`, then `frame`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventList {
    pub events: Vec<GraphEvent>,
}

impl EventList {
    pub fn sorted(mut events: Vec<GraphEvent>) -> Self {
        events.sort_by(|a, b| {
            a.time_ms
                .total_cmp(&b.time_ms)
                .then_with(|| a.tag.cmp(&b.tag))
                .then_with(|| a.frame.cmp(&b.frame))
        });
        Self { events }
    }

    pub fn byte_size(&self) -> usize {
        self.events
            .iter()
            .map(|e| {
                std::mem::size_of::<GraphEvent>()
                    + e.tag.len()
                    + e.measures.keys().map(|k| k.len() + 16).sum::<usize>()
            })
            .sum()
    }
}

/// One uninherited red timing line in song time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingLine {
    pub start_ms: f64,
    pub beat_length_ms: f64,
}

/// Read-only snapshot of a map's red lines for beat snapping and grid sampling.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimingGrid {
    lines: Vec<TimingLine>,
}

impl TimingGrid {
    /// Sorts lines by `start_ms` (later line at the exact same time wins) and keeps valid positive beat lengths.
    pub fn new(mut lines: Vec<TimingLine>) -> Self {
        lines.retain(|l| l.start_ms.is_finite() && l.beat_length_ms.is_finite() && l.beat_length_ms > 0.0);
        lines.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
        lines.dedup_by(|later, earlier| {
            let same = later.start_ms == earlier.start_ms;
            if same {
                *earlier = *later;
            }
            same
        });
        Self { lines }
    }

    pub fn lines(&self) -> &[TimingLine] {
        &self.lines
    }

    /// Snaps `time_ms` to the nearest `1/divisor` beat grid line, returning `(snapped_ms, step_ms)`.
    /// Returns `(time_ms, f64::INFINITY)` when the map has no red lines or `time_ms` precedes the first red line.
    pub fn snap(&self, time_ms: f64, divisor: u32) -> (f64, f64) {
        let Some(first) = self.lines.first() else {
            return (time_ms, f64::INFINITY);
        };
        if !time_ms.is_finite() || time_ms < first.start_ms {
            return (time_ms, f64::INFINITY);
        }
        self.snap_with_first(time_ms, divisor)
    }

    /// Snaps `time_ms` using the active section (or `lines[0]` if `time_ms` precedes the first red line,
    /// e.g. after lag compensation shifts an already-mapped event left of `lines[0].start_ms`).
    pub fn snap_with_first(&self, time_ms: f64, divisor: u32) -> (f64, f64) {
        let Some(first) = self.lines.first() else {
            return (time_ms, f64::INFINITY);
        };
        if !time_ms.is_finite() {
            return (time_ms, f64::INFINITY);
        }
        let idx = if time_ms < first.start_ms {
            0
        } else {
            self.lines
                .partition_point(|l| l.start_ms <= time_ms)
                .saturating_sub(1)
        };
        let section = self.lines[idx];
        let step = section.beat_length_ms / f64::from(divisor.max(1));
        let beats = (time_ms.max(0.0) - section.start_ms) / step;
        let mut snapped = section.start_ms + beats.round() * step;
        if snapped < 0.0 {
            snapped += step;
        }
        if let Some(next) = self.lines.get(idx + 1)
            && (time_ms - next.start_ms).abs() <= (time_ms - snapped).abs()
        {
            return (next.start_ms, step);
        }
        (snapped, step)
    }

    /// Generates all `1/divisor` grid timestamps falling inside `[range.from_ms, range.to_ms)`.
    pub fn grid_times_in_range(&self, range: AnalysisRange, divisor: u32) -> Vec<f64> {
        self.grid_times_in_range_with_cancel(range, divisor, &|| false)
            .unwrap_or_default()
    }

    /// Generates all `1/divisor` grid timestamps in `[range.from_ms, range.to_ms)` while checking `cancel`.
    pub fn grid_times_in_range_with_cancel(
        &self,
        range: AnalysisRange,
        divisor: u32,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<f64>, ()> {
        let mut out = Vec::new();
        let divisor = f64::from(divisor.max(1));
        for (idx, section) in self.lines.iter().enumerate() {
            let section_end = self
                .lines
                .get(idx + 1)
                .map_or(range.to_ms, |next| next.start_ms.min(range.to_ms));
            let win_start = section.start_ms.max(range.from_ms);
            if section_end <= win_start {
                continue;
            }
            let step = section.beat_length_ms / divisor;
            if !step.is_finite() || step <= 1e-6 {
                continue;
            }
            let first_n = ((win_start - section.start_ms) / step - 1e-9)
                .ceil()
                .max(0.0) as u64;
            let mut n = first_n;
            let mut steps = 0usize;
            loop {
                if steps & 1023 == 0 && cancel() {
                    return Err(());
                }
                let t = section.start_ms + n as f64 * step;
                if t >= section_end - 1e-9 || t >= range.to_ms {
                    break;
                }
                if t >= range.from_ms - 1e-9 {
                    out.push(t.max(range.from_ms));
                }
                n += 1;
                steps += 1;
            }
        }
        Ok(out)
    }

    /// Deterministic SHA-256 hash of all timing lines for source cache keys.
    pub fn content_hash(&self) -> String {
        let mut bytes = Vec::with_capacity(self.lines.len() * 16);
        let norm = |v: f64| if v == 0.0 { 0.0_f64.to_bits() } else { v.to_bits() };
        for line in &self.lines {
            bytes.extend_from_slice(&norm(line.start_ms).to_le_bytes());
            bytes.extend_from_slice(&norm(line.beat_length_ms).to_le_bytes());
        }
        sha256_hex(&bytes)
    }
}

/// Typed runtime value stored on a socket.
#[derive(Debug, Clone, PartialEq)]
pub enum SocketValue {
    Audio(Arc<AudioBuffer>),
    Spectrogram(Arc<Spectrogram>),
    Curve(Arc<Curve>),
    Events(Arc<EventList>),
    Timing(Arc<TimingGrid>),
    Range(AnalysisRange),
    Number(f64),
    Integer(i64),
    Bool(bool),
    Enum(String),
}

impl SocketValue {
    pub fn socket_type(&self) -> SocketType {
        match self {
            SocketValue::Audio(_) => SocketType::Audio,
            SocketValue::Spectrogram(_) => SocketType::Spectrogram,
            SocketValue::Curve(_) => SocketType::Curve,
            SocketValue::Events(_) => SocketType::Events,
            SocketValue::Timing(_) => SocketType::Timing,
            SocketValue::Range(_) => SocketType::Range,
            SocketValue::Number(_) => SocketType::Number,
            SocketValue::Integer(_) => SocketType::Integer,
            SocketValue::Bool(_) => SocketType::Bool,
            SocketValue::Enum(_) => SocketType::Enum,
        }
    }

    pub fn byte_size(&self) -> usize {
        match self {
            SocketValue::Audio(v) => v.byte_size(),
            SocketValue::Spectrogram(v) => v.byte_size(),
            SocketValue::Curve(v) => v.byte_size(),
            SocketValue::Events(v) => v.byte_size(),
            SocketValue::Timing(v) => v.lines.len() * std::mem::size_of::<TimingLine>(),
            SocketValue::Range(_) => std::mem::size_of::<AnalysisRange>(),
            SocketValue::Number(_) => std::mem::size_of::<f64>(),
            SocketValue::Integer(_) => std::mem::size_of::<i64>(),
            SocketValue::Bool(_) => std::mem::size_of::<bool>(),
            SocketValue::Enum(v) => v.len(),
        }
    }
}
