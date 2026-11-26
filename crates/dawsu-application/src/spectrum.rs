//! Calibrated FFT Spectrum & Pitch analyzer for opt-in mix diagnostics.
//!
//! Operates completely off the real-time audio thread on mono samples drained from
//! `dawsu_audio::SpectrumTap`:
//! - 4096-point FFT (75% overlap, hop = 1024) using `dawsu_foundation::fft::fft`.
//! - 4-term Blackman-Harris 92 dB windowing.
//! - Linear-power `|X[k]|^2` averaging across overlapping hops (never averaging dB).
//! - O(1) prefix-sum 1/12-octave smoothing across 128 log-spaced bins (20 Hz..20 kHz).
//! - Parabolic log-magnitude peak tracking for sub-bin pitch (`Hz`, MIDI note, cents).

use dawsu_foundation::fft::fft;

pub const SPECTRUM_FFT_SIZE: usize = 4096;
pub const SPECTRUM_HOP: usize = 1024;
pub const SPECTRUM_BINS: usize = 128;
pub const SPECTRUM_MIN_HZ: f32 = 20.0;
pub const SPECTRUM_MAX_HZ: f32 = 20_000.0;
pub const SPECTRUM_FLOOR_DB: f32 = -120.0;
const EMA_ALPHA: f64 = 0.35;
const PITCH_THRESHOLD_DB: f32 = -80.0;

#[derive(Clone, Debug, PartialEq)]
pub struct SpectrumFrame {
    pub bins_db: [f32; SPECTRUM_BINS],
    pub peak_hz: f32,
    pub peak_db: f32,
    pub midi_note: Option<u8>,
    pub cents: i16,
}

impl Default for SpectrumFrame {
    fn default() -> Self {
        Self {
            bins_db: [SPECTRUM_FLOOR_DB; SPECTRUM_BINS],
            peak_hz: 0.0,
            peak_db: SPECTRUM_FLOOR_DB,
            midi_note: None,
            cents: 0,
        }
    }
}

/// Applies spectral tilt around 1 kHz (`+3.0` for pink-flat, `+4.5` for mix balance).
pub fn apply_tilt(db: f32, freq_hz: f32, tilt_db_per_oct: f32) -> f32 {
    if !db.is_finite() || freq_hz <= 0.0 || tilt_db_per_oct == 0.0 {
        return db;
    }
    db + tilt_db_per_oct * (freq_hz / 1000.0).log2()
}

pub fn bin_center_hz(index: usize) -> f32 {
    let t = index as f32 / (SPECTRUM_BINS.saturating_sub(1).max(1)) as f32;
    SPECTRUM_MIN_HZ * (SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ).powf(t)
}

pub fn hz_to_note(hz: f32) -> Option<(u8, i16)> {
    if !hz.is_finite() || !(16.0..=22_000.0).contains(&hz) {
        return None;
    }
    let midi_f = 69.0 + 12.0 * (hz / 440.0).log2();
    let midi_rounded = midi_f.round();
    if !(0.0..=127.0).contains(&midi_rounded) {
        return None;
    }
    let cents = ((midi_f - midi_rounded) * 100.0).round() as i16;
    Some((midi_rounded as u8, cents.clamp(-50, 50)))
}

pub struct SpectrumAnalyzer {
    window: Vec<f64>,
    inv_coherent_sq: f64,
    ring: Vec<f32>,
    write_pos: usize,
    filled: usize,
    since_hop: usize,
    re: Vec<f64>,
    im: Vec<f64>,
    avg_power: Vec<f64>,
    prefix: Vec<f64>,
    has_power: bool,
    pub last_frame: SpectrumFrame,
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl SpectrumAnalyzer {
    pub fn new() -> Self {
        // 4-term Blackman-Harris -92 dB sidelobe window.
        let mut window = Vec::with_capacity(SPECTRUM_FFT_SIZE);
        let mut win_sum = 0.0_f64;
        let denom = (SPECTRUM_FFT_SIZE - 1) as f64;
        for n in 0..SPECTRUM_FFT_SIZE {
            let phase = 2.0 * std::f64::consts::PI * n as f64 / denom;
            let w = 0.35875
                - 0.48829 * phase.cos()
                + 0.14128 * (2.0 * phase).cos()
                - 0.01168 * (3.0 * phase).cos();
            win_sum += w;
            window.push(w);
        }
        let inv_coherent_sq = 4.0 / (win_sum * win_sum).max(1e-24);
        let bins = SPECTRUM_FFT_SIZE / 2 + 1;
        Self {
            window,
            inv_coherent_sq,
            ring: vec![0.0; SPECTRUM_FFT_SIZE],
            write_pos: 0,
            filled: 0,
            since_hop: 0,
            re: vec![0.0; SPECTRUM_FFT_SIZE],
            im: vec![0.0; SPECTRUM_FFT_SIZE],
            avg_power: vec![0.0; bins],
            prefix: vec![0.0; bins + 1],
            has_power: false,
            last_frame: SpectrumFrame::default(),
        }
    }

    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.write_pos = 0;
        self.filled = 0;
        self.since_hop = 0;
        self.avg_power.fill(0.0);
        self.has_power = false;
        self.last_frame = SpectrumFrame::default();
    }

    /// Feeds incoming mono samples and returns `Some(SpectrumFrame)` whenever an FFT hop
    /// completes (zero allocations).
    pub fn push_samples(&mut self, samples: &[f32], sample_rate: u32) -> Option<SpectrumFrame> {
        if samples.is_empty() || sample_rate == 0 {
            return None;
        }
        let mut updated = false;
        for &s in samples {
            self.ring[self.write_pos] = if s.is_finite() { s } else { 0.0 };
            self.write_pos = (self.write_pos + 1) % SPECTRUM_FFT_SIZE;
            if self.filled < SPECTRUM_FFT_SIZE {
                self.filled += 1;
            }
            self.since_hop += 1;
            if self.filled == SPECTRUM_FFT_SIZE && self.since_hop >= SPECTRUM_HOP {
                self.since_hop = 0;
                self.compute_hop(sample_rate);
                updated = true;
            }
        }
        updated.then(|| self.last_frame.clone())
    }

    fn compute_hop(&mut self, sample_rate: u32) {
        let mut pos = self.write_pos;
        for i in 0..SPECTRUM_FFT_SIZE {
            self.re[i] = f64::from(self.ring[pos]) * self.window[i];
            self.im[i] = 0.0;
            pos += 1;
            if pos == SPECTRUM_FFT_SIZE {
                pos = 0;
            }
        }
        fft(&mut self.re, &mut self.im);

        let num_bins = self.avg_power.len();
        for k in 0..num_bins {
            let norm = if k == 0 || k == SPECTRUM_FFT_SIZE / 2 {
                self.inv_coherent_sq * 0.25
            } else {
                self.inv_coherent_sq
            };
            let mag_sq = (self.re[k] * self.re[k] + self.im[k] * self.im[k]) * norm;
            if !self.has_power {
                self.avg_power[k] = mag_sq;
            } else {
                self.avg_power[k] = self.avg_power[k] * (1.0 - EMA_ALPHA) + mag_sq * EMA_ALPHA;
            }
        }
        self.has_power = true;

        self.prefix[0] = 0.0;
        for (i, &p) in self.avg_power.iter().enumerate() {
            self.prefix[i + 1] = self.prefix[i] + p;
        }

        let df = f64::from(sample_rate) / SPECTRUM_FFT_SIZE as f64;
        let max_bin = SPECTRUM_FFT_SIZE / 2;
        let half_twelfth = 2.0_f64.powf(1.0 / 24.0);
        let mut bins_db = [SPECTRUM_FLOOR_DB; SPECTRUM_BINS];
        for (i, slot) in bins_db.iter_mut().enumerate() {
            let center = f64::from(bin_center_hz(i));
            let f_lo = center / half_twelfth;
            let f_hi = center * half_twelfth;
            let lo = (f_lo / df).ceil().max(1.0) as usize;
            let hi = ((f_hi / df).floor() as usize).min(max_bin);
            let power = if hi >= lo && lo <= max_bin {
                (self.prefix[hi + 1] - self.prefix[lo]) / (hi - lo + 1) as f64
            } else {
                let exact = (center / df).clamp(1.0, max_bin as f64);
                let b0 = (exact.floor() as usize).min(max_bin - 1);
                let frac = exact - b0 as f64;
                self.avg_power[b0] * (1.0 - frac) + self.avg_power[b0 + 1] * frac
            };
            let db = if power > 1e-12 {
                10.0 * power.log10() as f32
            } else {
                SPECTRUM_FLOOR_DB
            };
            *slot = db.max(SPECTRUM_FLOOR_DB);
        }

        let min_search_bin = ((20.0 / df).ceil() as usize).clamp(1, max_bin - 1);
        let max_search_bin = ((20_000.0 / df).floor() as usize).clamp(min_search_bin, max_bin - 1);
        let mut best_k = min_search_bin;
        let mut best_p = 0.0_f64;
        for (k, &p) in self
            .avg_power
            .iter()
            .enumerate()
            .take(max_search_bin + 1)
            .skip(min_search_bin)
        {
            if p > best_p {
                best_p = p;
                best_k = k;
            }
        }
        let peak_db = if best_p > 1e-12 {
            (10.0 * best_p.log10() as f32).max(SPECTRUM_FLOOR_DB)
        } else {
            SPECTRUM_FLOOR_DB
        };
        let (peak_hz, midi_note, cents) = if peak_db >= PITCH_THRESHOLD_DB && best_k > 0 && best_k < max_bin {
            let alpha = (self.avg_power[best_k - 1] + 1e-18).ln();
            let beta = (self.avg_power[best_k] + 1e-18).ln();
            let gamma = (self.avg_power[best_k + 1] + 1e-18).ln();
            let denom = alpha - 2.0 * beta + gamma;
            let delta = if denom.abs() > 1e-12 {
                (0.5 * (alpha - gamma) / denom).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            let hz = ((best_k as f64 + delta) * df) as f32;
            let (midi, cents) = hz_to_note(hz).map_or((None, 0), |(m, c)| (Some(m), c));
            (hz, midi, cents)
        } else {
            (0.0, None, 0)
        };

        self.last_frame = SpectrumFrame {
            bins_db,
            peak_hz,
            peak_db,
            midi_note,
            cents,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_440hz_detects_a4_within_one_hz() {
        let rate = 48_000u32;
        let mut analyzer = SpectrumAnalyzer::new();
        let samples: Vec<f32> = (0..8192)
            .map(|n| (2.0 * std::f32::consts::PI * 440.0 * n as f32 / rate as f32).sin())
            .collect();
        let frame = analyzer.push_samples(&samples, rate).expect("expected FFT frame");
        assert!(
            (frame.peak_hz - 440.0).abs() < 1.0,
            "expected 440 Hz +-1 Hz, got {} Hz",
            frame.peak_hz
        );
        assert_eq!(frame.midi_note, Some(69));
        assert!(frame.cents.abs() <= 4, "cents={}", frame.cents);
        assert!(frame.peak_db > -3.0 && frame.peak_db <= 0.5, "peak_db={}", frame.peak_db);
    }

    #[test]
    fn pink_noise_reads_flat_within_1db_with_pink_tilt_and_4_5db_tilt_adds_exact_slope() {
        // Paul Kellet economic pink-noise filter seeded deterministically; Welch-average linear
        // power across emitted hops so single-frame chi-squared variance does not mask bias.
        let rate = 48_000u32;
        let total = 48_000 * 8;
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut b0 = 0.0_f64;
        let mut b1 = 0.0_f64;
        let mut b2 = 0.0_f64;
        let mut b3 = 0.0_f64;
        let mut b4 = 0.0_f64;
        let mut b5 = 0.0_f64;
        let mut b6 = 0.0_f64;
        let mut samples = Vec::with_capacity(total);
        for _ in 0..total {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let white = ((state >> 32) as i32 as f64) / f64::from(i32::MAX);
            b0 = 0.99886 * b0 + white * 0.0555179;
            b1 = 0.99332 * b1 + white * 0.0750759;
            b2 = 0.96900 * b2 + white * 0.1538520;
            b3 = 0.86650 * b3 + white * 0.3104856;
            b4 = 0.55000 * b4 + white * 0.5329522;
            b5 = -0.7616 * b5 - white * 0.0168980;
            let pink = (b0 + b1 + b2 + b3 + b4 + b5 + b6 + white * 0.5362) * 0.05;
            b6 = white * 0.115926;
            samples.push(pink as f32);
        }

        let mut analyzer = SpectrumAnalyzer::new();
        let mut power_sum = [0.0_f64; SPECTRUM_BINS];
        let mut hop_count = 0usize;
        let mut last_frame = None;
        // Skip first 48_000 samples (1 s filter warmup) so the IIR pink filter is in steady state.
        for chunk in samples[48_000..].chunks(SPECTRUM_HOP) {
            if let Some(frame) = analyzer.push_samples(chunk, rate) {
                for (acc, &db) in power_sum.iter_mut().zip(frame.bins_db.iter()) {
                    *acc += 10.0_f64.powf(f64::from(db) / 10.0);
                }
                hop_count += 1;
                last_frame = Some(frame);
            }
        }
        assert!(hop_count > 100);
        let frame = last_frame.unwrap();

        // Check midband flatness under +3.0 dB/oct pink compensation (300 Hz..6000 Hz).
        let mut tilted_3db = Vec::new();
        for (i, &sum_p) in power_sum.iter().enumerate() {
            let hz = bin_center_hz(i);
            if (300.0..=6000.0).contains(&hz) {
                let db = (10.0 * (sum_p / hop_count as f64).log10()) as f32;
                tilted_3db.push(apply_tilt(db, hz, 3.0));
            }
        }
        let mean = tilted_3db.iter().sum::<f32>() / tilted_3db.len() as f32;
        let max_dev = tilted_3db
            .iter()
            .map(|&v| (v - mean).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_dev <= 1.0, "pink noise +3 dB/oct deviation = {max_dev} dB");

        // Verify +4.5 dB/oct tilt adds exactly +4.5 dB per octave relative to 0 dB tilt.
        let low_1k = apply_tilt(frame.bins_db[64], 1000.0, 4.5) - frame.bins_db[64];
        let oct_2k = apply_tilt(frame.bins_db[80], 2000.0, 4.5) - frame.bins_db[80];
        let oct_500 = apply_tilt(frame.bins_db[48], 500.0, 4.5) - frame.bins_db[48];
        assert!(low_1k.abs() < 1e-5);
        assert!((oct_2k - 4.5).abs() < 1e-4);
        assert!((oct_500 + 4.5).abs() < 1e-4);
    }
}
