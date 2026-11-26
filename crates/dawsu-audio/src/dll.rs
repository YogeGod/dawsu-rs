//! Lock-free 2nd-order Fons Adriaensen Delay-Locked Loop (`Instant` <-> sample clock)
//! for sample-accurate MIDI timestamp translation without heap allocation on the audio thread.

use crate::ring::CachePadded;
use std::f64::consts::{PI, SQRT_2};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering, fence};
use std::time::Instant;

static MONOTONIC_EPOCH: OnceLock<Instant> = OnceLock::new();

/// Maps an [`Instant`] into signed nanoseconds relative to a lazy process-local epoch.
/// Performs zero heap allocations once the epoch `OnceLock` is seeded off-thread.
pub fn monotonic_rel_ns(at: Instant) -> i64 {
    let origin = *MONOTONIC_EPOCH.get_or_init(Instant::now);
    if at >= origin {
        i64::try_from(at.duration_since(origin).as_nanos()).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(origin.duration_since(at).as_nanos()).unwrap_or(i64::MAX)
    }
}

/// Immutable snapshot of the audio callback DLL clock for off-thread MIDI timestamping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockSnapshot {
    pub anchor_instant_ns: i64,
    pub anchor_song_ms: f64,
    pub anchor_sample: u64,
    pub rate: u32,
    pub speed: f64,
    pub filtered_period_s: f64,
    pub period_frames: u32,
    pub playing: bool,
    pub generation: u64,
}

impl ClockSnapshot {
    /// Converts a monotonic nanosecond capture timestamp (`monotonic_rel_ns`) into song time (ms).
    pub fn instant_ns_to_song_ms(&self, event_ns: i64) -> f64 {
        if !self.playing {
            return self.anchor_song_ms.max(0.0);
        }
        let dt_s = (event_ns - self.anchor_instant_ns) as f64 * 1e-9;
        let rate_f = f64::from(self.rate.max(1));
        let eff_rate = if self.filtered_period_s > 0.0
            && self.filtered_period_s.is_finite()
            && self.period_frames > 0
        {
            f64::from(self.period_frames) / self.filtered_period_s
        } else {
            rate_f
        };
        let speed = if self.speed.is_finite() && self.speed > 0.0 {
            self.speed
        } else {
            1.0
        };
        let delta_ms = dt_s * eff_rate * 1000.0 * speed / rate_f;
        let song_ms = self.anchor_song_ms + delta_ms;
        if song_ms.is_finite() {
            song_ms.max(0.0)
        } else {
            self.anchor_song_ms.max(0.0)
        }
    }

    /// Converts an `std::time::Instant` into song time (ms) using this DLL snapshot.
    pub fn instant_to_song_ms(&self, at: Instant) -> f64 {
        self.instant_ns_to_song_ms(monotonic_rel_ns(at))
    }
}

/// Fons Adriaensen 2nd-order critically damped DLL (`bandwidth = 0.1 Hz` default).
pub struct AudioClockDll {
    bw_hz: f64,
    initialized: bool,
    t0: f64,
    t1: f64,
    e2: f64,
    last_period_frames: u32,
    last_rate: u32,
    sample_counter: u64,
    generation: u64,
}

impl Default for AudioClockDll {
    fn default() -> Self {
        Self::new(0.1)
    }
}

impl AudioClockDll {
    pub fn new(bw_hz: f64) -> Self {
        Self {
            bw_hz: if bw_hz.is_finite() && bw_hz > 0.0 {
                bw_hz
            } else {
                0.1
            },
            initialized: false,
            t0: 0.0,
            t1: 0.0,
            e2: 0.0,
            last_period_frames: 0,
            last_rate: 0,
            sample_counter: 0,
            generation: 0,
        }
    }

    /// Resets the DLL cleanly across transport transitions (Play, Seek, Cycle wrap, speed change).
    pub fn reset(&mut self, generation: u64) {
        self.initialized = false;
        self.sample_counter = 0;
        self.generation = generation;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Steps the DLL once per audio callback and returns the smoothed [`ClockSnapshot`].
    pub fn update(
        &mut self,
        now_ns: i64,
        period_frames: u32,
        sample_rate: u32,
        song_ms: f64,
        speed: f64,
        playing: bool,
    ) -> ClockSnapshot {
        let frames = period_frames.max(1);
        let rate = sample_rate.max(1);
        let t_nom = f64::from(frames) / f64::from(rate);
        let t_now = now_ns as f64 * 1e-9;

        let reinit = !self.initialized
            || self.last_period_frames != frames
            || self.last_rate != rate
            || (t_now - self.t1).abs() > (t_nom * 4.0).max(0.25);

        if reinit {
            self.initialized = true;
            self.last_period_frames = frames;
            self.last_rate = rate;
            self.e2 = t_nom;
            self.t0 = t_now;
            self.t1 = t_now + t_nom;
            self.sample_counter = 0;
        } else {
            let omega = (2.0 * PI * self.bw_hz * t_nom).clamp(1e-6, 0.5);
            let b = SQRT_2 * omega;
            let c = omega * omega;
            let err = t_now - self.t1;
            self.t0 = self.t1;
            self.t1 += b * err + self.e2;
            self.e2 = (self.e2 + c * err).clamp(t_nom * 0.5, t_nom * 2.0);
            self.sample_counter = self.sample_counter.saturating_add(u64::from(frames));
        }

        ClockSnapshot {
            anchor_instant_ns: (self.t0 * 1e9).round() as i64,
            anchor_song_ms: if song_ms.is_finite() {
                song_ms.max(0.0)
            } else {
                0.0
            },
            anchor_sample: self.sample_counter,
            rate,
            speed: if speed.is_finite() && speed > 0.0 {
                speed
            } else {
                1.0
            },
            filtered_period_s: self.e2,
            period_frames: frames,
            playing,
            generation: self.generation,
        }
    }
}

/// Safe lock-free single-writer seqlock exposing [`ClockSnapshot`] to editor threads.
pub struct ClockSnapshotSlot {
    seq: CachePadded<AtomicU64>,
    has_value: AtomicBool,
    anchor_instant_ns: AtomicI64,
    anchor_song_ms_bits: AtomicU64,
    anchor_sample: AtomicU64,
    rate: AtomicU32,
    speed_bits: AtomicU64,
    filtered_period_s_bits: AtomicU64,
    period_frames: AtomicU32,
    playing: AtomicBool,
    generation: AtomicU64,
}

impl Default for ClockSnapshotSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockSnapshotSlot {
    pub fn new() -> Self {
        Self {
            seq: CachePadded::new(AtomicU64::new(0)),
            has_value: AtomicBool::new(false),
            anchor_instant_ns: AtomicI64::new(0),
            anchor_song_ms_bits: AtomicU64::new(0),
            anchor_sample: AtomicU64::new(0),
            rate: AtomicU32::new(48_000),
            speed_bits: AtomicU64::new(1.0_f64.to_bits()),
            filtered_period_s_bits: AtomicU64::new(0),
            period_frames: AtomicU32::new(512),
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }

    pub fn store(&self, snap: ClockSnapshot) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Release);
        fence(Ordering::Release);
        self.anchor_instant_ns
            .store(snap.anchor_instant_ns, Ordering::Relaxed);
        self.anchor_song_ms_bits
            .store(snap.anchor_song_ms.to_bits(), Ordering::Relaxed);
        self.anchor_sample
            .store(snap.anchor_sample, Ordering::Relaxed);
        self.rate.store(snap.rate, Ordering::Relaxed);
        self.speed_bits
            .store(snap.speed.to_bits(), Ordering::Relaxed);
        self.filtered_period_s_bits
            .store(snap.filtered_period_s.to_bits(), Ordering::Relaxed);
        self.period_frames
            .store(snap.period_frames, Ordering::Relaxed);
        self.playing.store(snap.playing, Ordering::Relaxed);
        self.generation.store(snap.generation, Ordering::Relaxed);
        self.has_value.store(true, Ordering::Relaxed);
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    pub fn load(&self) -> Option<ClockSnapshot> {
        for _ in 0..16 {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let has = self.has_value.load(Ordering::Relaxed);
            let snap = ClockSnapshot {
                anchor_instant_ns: self.anchor_instant_ns.load(Ordering::Relaxed),
                anchor_song_ms: f64::from_bits(self.anchor_song_ms_bits.load(Ordering::Relaxed)),
                anchor_sample: self.anchor_sample.load(Ordering::Relaxed),
                rate: self.rate.load(Ordering::Relaxed),
                speed: f64::from_bits(self.speed_bits.load(Ordering::Relaxed)),
                filtered_period_s: f64::from_bits(
                    self.filtered_period_s_bits.load(Ordering::Relaxed),
                ),
                period_frames: self.period_frames.load(Ordering::Relaxed),
                playing: self.playing.load(Ordering::Relaxed),
                generation: self.generation.load(Ordering::Relaxed),
            };
            fence(Ordering::Acquire);
            let s2 = self.seq.load(Ordering::Acquire);
            if s1 == s2 {
                return has.then_some(snap);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dll_tracks_sub_half_ms_under_4ms_jitter_and_resets_generation() {
        let rate = 48_000u32;
        let frames = 512u32;
        let t_nom_ns = f64::from(frames) * 1e9 / f64::from(rate);
        let mut dll = AudioClockDll::new(0.1);
        let mut max_settled_err_ns = 0i64;
        let mut max_song_err_ms = 0.0_f64;

        // Deterministic +-4 ms alternating jitter across 700 callbacks (~7.5 s).
        let jitter_pattern_ns: [f64; 8] = [
            0.0, 3_900_000.0, -3_800_000.0, 2_500_000.0, -4_000_000.0, 1_700_000.0,
            -2_200_000.0, 3_200_000.0,
        ];
        for k in 0..700usize {
            let true_t_ns = k as f64 * t_nom_ns;
            let jitter_ns = if k == 0 {
                0.0
            } else {
                jitter_pattern_ns[k % jitter_pattern_ns.len()]
            };
            let now_ns = (true_t_ns + jitter_ns).round() as i64;
            let true_song_ms = k as f64 * f64::from(frames) * 1000.0 / f64::from(rate);
            let snap = dll.update(now_ns, frames, rate, true_song_ms, 1.0, true);
            if k >= 450 {
                let err_ns = (snap.anchor_instant_ns - true_t_ns.round() as i64).abs();
                max_settled_err_ns = max_settled_err_ns.max(err_ns);

                // Query an event arriving 4.0 ms into this callback.
                let query_ns = true_t_ns.round() as i64 + 4_000_000;
                let got_ms = snap.instant_ns_to_song_ms(query_ns);
                let expected_ms = true_song_ms + 4.0;
                max_song_err_ms = max_song_err_ms.max((got_ms - expected_ms).abs());
            }
        }
        assert!(
            max_settled_err_ns < 500_000,
            "expected < 0.5 ms DLL phase error under +-4 ms jitter, got {} ms",
            max_settled_err_ns as f64 * 1e-6
        );
        assert!(
            max_song_err_ms < 0.5,
            "expected < 0.5 ms interpolated song error, got {max_song_err_ms}"
        );

        // Reset bumps generation and restarts cleanly after a Seek/Cycle jump without drift leak.
        dll.reset(3);
        assert_eq!(dll.generation(), 3);
        let jump_ns = 10_000_000_000_i64;
        let snap = dll.update(jump_ns, frames, rate, 12_345.0, 1.5, true);
        assert_eq!(snap.generation, 3);
        assert!((snap.instant_ns_to_song_ms(jump_ns + 10_000_000) - 12_360.0).abs() < 0.05);
    }

    #[test]
    fn clock_snapshot_slot_seqlock_round_trip() {
        let slot = ClockSnapshotSlot::new();
        assert!(slot.load().is_none());
        let snap = ClockSnapshot {
            anchor_instant_ns: 42_000_000,
            anchor_song_ms: 1500.25,
            anchor_sample: 2048,
            rate: 48_000,
            speed: 1.25,
            filtered_period_s: 512.0 / 48_000.0,
            period_frames: 512,
            playing: true,
            generation: 7,
        };
        slot.store(snap);
        assert_eq!(slot.load(), Some(snap));
    }
}
