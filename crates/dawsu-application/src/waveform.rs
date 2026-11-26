//! Display data belongs to Application, outside the audio callback.
//!
//! 1. 1 ms resolution stereo peak points (`amp_l`, `amp_r`, clamped to `[0, 1]`)
//!    and gated BS.1770 4x true-peak measurement (`true_peak`, unclamped).
//! 2. 1024-sample Hann-windowed radix-2 FFT for Low (20–100 Hz), Mid (100–2000 Hz),
//!    and High (2000–12000 Hz) intensities.
//! 3. Peak-preserving resampling down to at most `MAX_BINS` (45,000) bins and $R=4$
//!    mipmap pyramid pooling so transients survive across all zoom levels.
//! 4. Track-wide normalization by `low_max`, `mid_max`, `high_max`.
use dawsu_audio::AudioClip;
use std::sync::OnceLock;

const FFT_SIZE: usize = 1024;
const FFT_BINS: usize = FFT_SIZE / 2;
const LOW_MIN_HZ: f32 = 20.0;
const MID_MIN_HZ: f32 = 100.0;
const HIGH_MIN_HZ: f32 = 2000.0;
const HIGH_MAX_HZ: f32 = 12000.0;
const MAX_BINS: usize = 45_000;
/// Gate 4x BS.1770 FIR evaluation to windows whose sample peak can exceed -0.3 dBTP (~-3.7 dBFS).
const TRUE_PEAK_GATE: f32 = 0.65;
const FIR_TAPS: usize = 12;
const FIR_CENTER: isize = 5;
const PYRAMID_MIN_BINS: usize = 256;
const PYRAMID_MAX_LEVELS: usize = 8;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0_f64;
    let mut term = 1.0_f64;
    let half_x = x * 0.5;
    for k in 1..=25 {
        let kf = k as f64;
        term *= (half_x / kf) * (half_x / kf);
        sum += term;
        if term < 1e-16 * sum {
            break;
        }
    }
    sum
}

/// Three polyphase fractional-delay kernels (phases 0.25, 0.50, 0.75) of a 48-tap
/// 4x oversampling Kaiser-windowed sinc filter modeled after ITU-R BS.1770-4.
fn true_peak_fir_phases() -> &'static [[f32; FIR_TAPS]; 3] {
    static PHASES: OnceLock<[[f32; FIR_TAPS]; 3]> = OnceLock::new();
    PHASES.get_or_init(|| {
        const BETA: f64 = 5.5;
        const HALF_WIDTH: f64 = 6.0;
        let i0_beta = bessel_i0(BETA);
        let mut table = [[0.0_f32; FIR_TAPS]; 3];
        for (phase_idx, row) in table.iter_mut().enumerate() {
            let frac = (phase_idx + 1) as f64 * 0.25;
            let mut weights = [0.0_f64; FIR_TAPS];
            let mut sum = 0.0_f64;
            for (k, slot) in weights.iter_mut().enumerate() {
                let x = (k as f64 - FIR_CENTER as f64) - frac;
                let sinc = if x.abs() < 1e-12 {
                    1.0
                } else {
                    let pi_x = std::f64::consts::PI * x;
                    pi_x.sin() / pi_x
                };
                let ratio = (x / HALF_WIDTH).abs().min(1.0);
                let win = bessel_i0(BETA * (1.0 - ratio * ratio).sqrt()) / i0_beta;
                let w = sinc * win;
                *slot = w;
                sum += w;
            }
            for (dst, &w) in row.iter_mut().zip(weights.iter()) {
                *dst = (w / sum) as f32;
            }
        }
        table
    })
}

/// Computes the 4x oversampled BS.1770 true-peak linear magnitude across `start_frame..end_frame`
/// in interleaved `samples`, gating polyphase FIR evaluation on segments whose sample peak >= 0.65.
pub(crate) fn true_peak_4x_interleaved(
    samples: &[f32],
    channels: usize,
    start_frame: usize,
    end_frame: usize,
) -> f32 {
    if channels == 0 || start_frame >= end_frame {
        return 0.0;
    }
    let total_frames = samples.len() / channels;
    let end = end_frame.min(total_frames);
    if start_frame >= end {
        return 0.0;
    }

    let mut max_peak = 0.0_f32;
    for f in start_frame..end {
        let base = f * channels;
        for ch in 0..channels {
            let v = samples[base + ch];
            if v.is_finite() {
                max_peak = max_peak.max(v.abs());
            }
        }
    }
    if max_peak < TRUE_PEAK_GATE {
        return max_peak;
    }

    let phases = true_peak_fir_phases();
    for f in start_frame..end {
        for ch in 0..channels {
            if f >= FIR_CENTER as usize && f + FIR_TAPS - FIR_CENTER as usize <= total_frames {
                let base_start = (f - FIR_CENTER as usize) * channels + ch;
                for phase in phases {
                    let mut acc = 0.0_f32;
                    for (k, &coef) in phase.iter().enumerate() {
                        acc += samples[base_start + k * channels] * coef;
                    }
                    max_peak = max_peak.max(acc.abs());
                }
            } else {
                for phase in phases {
                    let mut acc = 0.0_f32;
                    for (k, &coef) in phase.iter().enumerate() {
                        let idx = f as isize + (k as isize - FIR_CENTER);
                        if idx >= 0 && (idx as usize) < total_frames {
                            let s = samples[idx as usize * channels + ch];
                            if s.is_finite() {
                                acc += s * coef;
                            }
                        }
                    }
                    max_peak = max_peak.max(acc.abs());
                }
            }
        }
    }
    max_peak
}

/// In-place radix-2 Cooley-Tukey forward FFT on a power-of-two buffer of `(re, im)` pairs.
pub(crate) fn fft_radix2_inplace(buf: &mut [(f32, f32)]) {
    let n = buf.len();
    debug_assert!(n.is_power_of_two());
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            buf.swap(i, j);
        }
    }
    let mut len = 2usize;
    while len <= n {
        let half = len >> 1;
        let angle = -2.0 * std::f32::consts::PI / len as f32;
        let (w_len_im, w_len_re) = angle.sin_cos();
        for chunk in buf.chunks_exact_mut(len) {
            let (mut w_re, mut w_im) = (1.0_f32, 0.0_f32);
            for k in 0..half {
                let (u_re, u_im) = chunk[k];
                let (v_re, v_im) = chunk[k + half];
                let t_re = v_re * w_re - v_im * w_im;
                let t_im = v_re * w_im + v_im * w_re;
                chunk[k] = (u_re + t_re, u_im + t_im);
                chunk[k + half] = (u_re - t_re, u_im - t_im);
                let next_re = w_re * w_len_re - w_im * w_len_im;
                w_im = w_re * w_len_im + w_im * w_len_re;
                w_re = next_re;
            }
        }
        len <<= 1;
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WaveBin {
    pub peak: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
    pub amp_l: f32,
    pub amp_r: f32,
    pub true_peak: f32,
}

#[derive(Clone, Copy, Default)]
struct RawPoint {
    amp_l: f32,
    amp_r: f32,
    true_peak: f32,
    low: f32,
    mid: f32,
    high: f32,
}

fn resample_peak_preserving(points: &[RawPoint], target_count: usize) -> Vec<RawPoint> {
    if points.is_empty() || target_count == 0 {
        return Vec::new();
    }
    let len = points.len();
    let mut out = Vec::with_capacity(target_count);
    for generated_idx in 0..target_count {
        let lo = (generated_idx * len) / target_count;
        let hi = (((generated_idx + 1) * len) / target_count)
            .max(lo + 1)
            .min(len);
        let mut amp_l = 0.0_f32;
        let mut amp_r = 0.0_f32;
        let mut true_peak = 0.0_f32;
        let mut w_low = 0.0_f32;
        let mut w_mid = 0.0_f32;
        let mut w_high = 0.0_f32;
        let mut w_sum = 0.0_f32;
        let mut s_low = 0.0_f32;
        let mut s_mid = 0.0_f32;
        let mut s_high = 0.0_f32;
        let slice = &points[lo..hi];
        for p in slice {
            amp_l = amp_l.max(p.amp_l);
            amp_r = amp_r.max(p.amp_r);
            true_peak = true_peak.max(p.true_peak);
            let w = p.amp_l.max(p.amp_r);
            w_low += p.low * w;
            w_mid += p.mid * w;
            w_high += p.high * w;
            w_sum += w;
            s_low += p.low;
            s_mid += p.mid;
            s_high += p.high;
        }
        let inv_n = 1.0 / slice.len() as f32;
        let (low, mid, high) = if w_sum > 1e-6 {
            let inv_w = 1.0 / w_sum;
            (w_low * inv_w, w_mid * inv_w, w_high * inv_w)
        } else {
            (s_low * inv_n, s_mid * inv_n, s_high * inv_n)
        };
        out.push(RawPoint {
            amp_l,
            amp_r,
            true_peak,
            low,
            mid,
            high,
        });
    }
    out
}

/// Pools a level of [`WaveBin`] values by factor R=4, preserving peak, amp_l, amp_r and true_peak
/// while amplitude-weighting spectral colour bands.
pub fn pool_wave_bins_r4(level: &[WaveBin]) -> Vec<WaveBin> {
    if level.len() <= 1 {
        return level.to_vec();
    }
    let out_len = level.len().div_ceil(4);
    let mut out = Vec::with_capacity(out_len);
    for chunk in level.chunks(4) {
        let mut peak = 0.0_f32;
        let mut amp_l = 0.0_f32;
        let mut amp_r = 0.0_f32;
        let mut true_peak = 0.0_f32;
        let mut w_low = 0.0_f32;
        let mut w_mid = 0.0_f32;
        let mut w_high = 0.0_f32;
        let mut w_sum = 0.0_f32;
        let mut s_low = 0.0_f32;
        let mut s_mid = 0.0_f32;
        let mut s_high = 0.0_f32;
        for b in chunk {
            peak = peak.max(b.peak);
            amp_l = amp_l.max(b.amp_l);
            amp_r = amp_r.max(b.amp_r);
            true_peak = true_peak.max(b.true_peak);
            let w = b.amp_l.max(b.amp_r).max(b.peak);
            w_low += b.low * w;
            w_mid += b.mid * w;
            w_high += b.high * w;
            w_sum += w;
            s_low += b.low;
            s_mid += b.mid;
            s_high += b.high;
        }
        let inv_n = 1.0 / chunk.len() as f32;
        let (low, mid, high) = if w_sum > 1e-6 {
            let inv_w = 1.0 / w_sum;
            (w_low * inv_w, w_mid * inv_w, w_high * inv_w)
        } else {
            (s_low * inv_n, s_mid * inv_n, s_high * inv_n)
        };
        out.push(WaveBin {
            peak,
            low,
            mid,
            high,
            amp_l,
            amp_r,
            true_peak,
        });
    }
    out
}

/// Builds an R=4 peak-preserving mipmap pyramid rooted at `base`.
/// Level 0 is `base` itself; coarsened levels are appended while `len > 256` (up to 8 levels).
pub fn build_pyramid_levels(base: &[WaveBin]) -> Vec<Vec<WaveBin>> {
    if base.is_empty() {
        return Vec::new();
    }
    let mut levels = Vec::with_capacity(PYRAMID_MAX_LEVELS);
    levels.push(base.to_vec());
    while levels.len() < PYRAMID_MAX_LEVELS {
        let Some(prev) = levels.last() else { break };
        if prev.len() <= PYRAMID_MIN_BINS {
            break;
        }
        levels.push(pool_wave_bins_r4(prev));
    }
    levels
}

pub fn analyse(audio: &AudioClip) -> Vec<WaveBin> {
    let channels = audio.channels.max(1);
    let frames = audio.samples.len() / channels;
    if frames == 0 || audio.sample_rate == 0 {
        return Vec::new();
    }

    // Pass 1 & Pass 2: parallelized across CPU cores for fast multi-minute track analysis.
    let frames_per_point = (audio.sample_rate as usize / 1000).max(1);
    let second_ch = if channels > 1 { 1 } else { 0 };
    let point_count = frames.div_ceil(frames_per_point);
    let mut points = vec![RawPoint::default(); point_count];

    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|n| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (FFT_SIZE - 1) as f32).cos())
        })
        .collect();
    // Precompute radix-2 stage twiddle factors once for FFT_SIZE (eliminates sin_cos per chunk).
    let twiddles: Vec<Vec<(f32, f32)>> = {
        let mut stages = Vec::new();
        let mut len = 2usize;
        while len <= FFT_SIZE {
            let half = len >> 1;
            let mut stage = Vec::with_capacity(half);
            let step = -2.0 * std::f32::consts::PI / len as f32;
            for k in 0..half {
                let (im, re) = (step * k as f32).sin_cos();
                stage.push((re, im));
            }
            stages.push(stage);
            len <<= 1;
        }
        stages
    };
    let bit_rev: Vec<usize> = {
        let mut rev = vec![0usize; FFT_SIZE];
        let mut j = 0usize;
        for slot in rev.iter_mut().skip(1) {
            let mut bit = FFT_SIZE >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j ^= bit;
            *slot = j;
        }
        rev
    };

    let sample_rate = audio.sample_rate as f32;
    let inv_channels = 1.0 / channels as f32;
    let low_start = ((FFT_BINS as f32 * 2.0 * LOW_MIN_HZ / sample_rate) as usize).min(FFT_BINS);
    let mid_start = ((FFT_BINS as f32 * 2.0 * MID_MIN_HZ / sample_rate) as usize).min(FFT_BINS);
    let high_start = ((FFT_BINS as f32 * 2.0 * HIGH_MIN_HZ / sample_rate) as usize).min(FFT_BINS);
    let high_end = ((FFT_BINS as f32 * 2.0 * HIGH_MAX_HZ / sample_rate) as usize).min(FFT_BINS);

    let fft_blocks = frames.div_ceil(FFT_SIZE);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8);

    // Pass 1: 1 ms stereo peaks and gated 4x BS.1770 true-peak (parallelized in point chunks).
    if point_count >= 2048 && threads > 1 {
        let chunk_pts = point_count.div_ceil(threads);
        std::thread::scope(|scope| {
            for (chunk_idx, slice) in points.chunks_mut(chunk_pts).enumerate() {
                let base_pt = chunk_idx * chunk_pts;
                let samples = &audio.samples;
                scope.spawn(move || {
                    for (local, pt) in slice.iter_mut().enumerate() {
                        let start_frame = (base_pt + local) * frames_per_point;
                        let end_frame = (start_frame + frames_per_point).min(frames);
                        let mut amp_l = 0.0_f32;
                        let mut amp_r = 0.0_f32;
                        for f in start_frame..end_frame {
                            let base = f * channels;
                            amp_l = amp_l.max(samples[base].abs());
                            amp_r = amp_r.max(samples[base + second_ch].abs());
                        }
                        let raw_peak = amp_l.max(amp_r);
                        pt.amp_l = amp_l.min(1.0);
                        pt.amp_r = amp_r.min(1.0);
                        pt.true_peak = if raw_peak >= TRUE_PEAK_GATE {
                            true_peak_4x_interleaved(samples, channels, start_frame, end_frame)
                        } else {
                            raw_peak
                        };
                    }
                });
            }
        });
    } else {
        for (point_idx, pt) in points.iter_mut().enumerate() {
            let start_frame = point_idx * frames_per_point;
            let end_frame = (start_frame + frames_per_point).min(frames);
            let mut amp_l = 0.0_f32;
            let mut amp_r = 0.0_f32;
            for f in start_frame..end_frame {
                let base = f * channels;
                amp_l = amp_l.max(audio.samples[base].abs());
                amp_r = amp_r.max(audio.samples[base + second_ch].abs());
            }
            let raw_peak = amp_l.max(amp_r);
            pt.amp_l = amp_l.min(1.0);
            pt.amp_r = amp_r.min(1.0);
            pt.true_peak = if raw_peak >= TRUE_PEAK_GATE {
                true_peak_4x_interleaved(&audio.samples, channels, start_frame, end_frame)
            } else {
                raw_peak
            };
        }
    }

    // Pass 2: 1024-sample Hann-windowed FFTs (parallelized across block chunks).
    let mut block_bands = vec![(0.0_f32, 0.0_f32, 0.0_f32); fft_blocks];
    let run_block_range = |start_block: usize, out_slice: &mut [(f32, f32, f32)]| {
        let mut fft_buf = vec![(0.0_f32, 0.0_f32); FFT_SIZE];
        let mut magnitudes = vec![0.0_f32; FFT_BINS];
        for (offset, dst) in out_slice.iter_mut().enumerate() {
            let frame_pos = (start_block + offset) * FFT_SIZE;
            let available = (frames - frame_pos).min(FFT_SIZE);
            for i in 0..available {
                let base = (frame_pos + i) * channels;
                let mut mono = 0.0_f32;
                for c in 0..channels {
                    mono += audio.samples[base + c];
                }
                fft_buf[bit_rev[i]] = (mono * inv_channels * window[i], 0.0);
            }
            for i in available..FFT_SIZE {
                fft_buf[bit_rev[i]] = (0.0, 0.0);
            }
            let mut len = 2usize;
            for stage in &twiddles {
                let half = len >> 1;
                for chunk in fft_buf.chunks_exact_mut(len) {
                    for (k, &(w_re, w_im)) in stage.iter().enumerate() {
                        let (u_re, u_im) = chunk[k];
                        let (v_re, v_im) = chunk[k + half];
                        let t_re = v_re * w_re - v_im * w_im;
                        let t_im = v_re * w_im + v_im * w_re;
                        chunk[k] = (u_re + t_re, u_im + t_im);
                        chunk[k + half] = (u_re - t_re, u_im - t_im);
                    }
                }
                len <<= 1;
            }
            for (mag, &(re, im)) in magnitudes.iter_mut().zip(fft_buf.iter().take(FFT_BINS)) {
                *mag = (re * re + im * im).sqrt();
            }
            let low: f32 = magnitudes[low_start..mid_start].iter().sum();
            let mid: f32 = magnitudes[mid_start..high_start].iter().sum();
            let high: f32 = magnitudes[high_start..high_end].iter().sum();
            *dst = (low, mid, high);
        }
    };

    if fft_blocks >= 64 && threads > 1 {
        let chunk_blocks = fft_blocks.div_ceil(threads);
        std::thread::scope(|scope| {
            for (chunk_idx, slice) in block_bands.chunks_mut(chunk_blocks).enumerate() {
                let start_block = chunk_idx * chunk_blocks;
                scope.spawn(move || run_block_range(start_block, slice));
            }
        });
    } else {
        run_block_range(0, &mut block_bands);
    }

    let mut current_point = 0usize;
    for (block_idx, &(low, mid, high)) in block_bands.iter().enumerate() {
        let block_end = ((block_idx + 1) * FFT_SIZE).min(frames);
        while current_point < points.len() && current_point * frames_per_point < block_end {
            let p = &mut points[current_point];
            p.low = low;
            p.mid = mid;
            p.high = high;
            current_point += 1;
        }
    }

    // Pass 3: Peak-preserving max-pooling when track exceeds MAX_BINS.
    let resampled = if points.len() > MAX_BINS {
        resample_peak_preserving(&points, MAX_BINS)
    } else {
        points
    };

    // Pass 4: Track-wide normalization by low_max, mid_max, high_max (with a -26 dB floor
    // relative to the dominant band so single-tone signals don't amplify window sidelobes).
    let mut low_max = 0.0_f32;
    let mut mid_max = 0.0_f32;
    let mut high_max = 0.0_f32;
    for p in &resampled {
        low_max = low_max.max(p.low);
        mid_max = mid_max.max(p.mid);
        high_max = high_max.max(p.high);
    }
    let overall_max = low_max.max(mid_max).max(high_max);
    let floor = overall_max * 0.05;
    let low_denom = low_max.max(floor);
    let mid_denom = mid_max.max(floor);
    let high_denom = high_max.max(floor);

    resampled
        .into_iter()
        .map(|p| {
            let amp_l = p.amp_l.clamp(0.0, 1.0);
            let amp_r = p.amp_r.clamp(0.0, 1.0);
            WaveBin {
                peak: amp_l.max(amp_r),
                low: if low_denom > 0.0 { (p.low / low_denom).clamp(0.0, 1.0) } else { 0.0 },
                mid: if mid_denom > 0.0 { (p.mid / mid_denom).clamp(0.0, 1.0) } else { 0.0 },
                high: if high_denom > 0.0 { (p.high / high_denom).clamp(0.0, 1.0) } else { 0.0 },
                amp_l,
                amp_r,
                true_peak: p.true_peak.max(amp_l).max(amp_r),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(f: f32) -> AudioClip {
        AudioClip {
            sample_rate: 48000,
            channels: 1,
            samples: (0..48000)
                .map(|n| (2.0 * std::f32::consts::PI * f * n as f32 / 48000.0).sin())
                .collect(),
        }
    }

    #[test]
    fn frequency_colours_come_from_the_audio() {
        let bass = analyse(&wave(70.0));
        let treble = analyse(&wave(8000.0));
        let energy = |bins: &[WaveBin]| {
            bins.iter()
                .fold((0.0, 0.0), |(l, h), b| (l + b.low, h + b.high))
        };
        let (l, h) = energy(&bass);
        assert!(l > h * 3.0, "bass low={l}, high={h}");
        let (l, h) = energy(&treble);
        assert!(h > l * 3.0, "treble low={l}, high={h}");
        assert!(bass.len() <= MAX_BINS);
    }

    #[test]
    fn stereo_split_and_long_track_gaussian_resampling() {
        // Stereo: left = 0.8, right = 0.25.
        let mut samples = Vec::with_capacity(4800 * 2);
        for _ in 0..4800 {
            samples.push(0.8);
            samples.push(0.25);
        }
        let bins = analyse(&AudioClip {
            sample_rate: 48_000,
            channels: 2,
            samples,
        });
        assert_eq!(bins.len(), 100);
        assert!((bins[50].amp_l - 0.8).abs() < 1e-4);
        assert!((bins[50].amp_r - 0.25).abs() < 1e-4);
        assert!((bins[50].peak - 0.8).abs() < 1e-4);

        // 60-second mono track (60,000 ms) resamples down to MAX_BINS (45,000).
        let long = AudioClip {
            sample_rate: 8_000,
            channels: 1,
            samples: vec![0.5; 8_000 * 60],
        };
        let long_bins = analyse(&long);
        assert_eq!(long_bins.len(), MAX_BINS);
        assert!((long_bins[MAX_BINS / 2].peak - 0.5).abs() < 0.02);
    }

    #[test]
    fn true_peak_fir_phases_have_unit_dc_and_symmetry() {
        let phases = true_peak_fir_phases();
        for (idx, phase) in phases.iter().enumerate() {
            let sum: f32 = phase.iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "phase {idx} DC sum = {sum}");
        }
        for k in 0..FIR_TAPS {
            assert!(
                (phases[0][k] - phases[2][FIR_TAPS - 1 - k]).abs() < 1e-5,
                "phase asymmetry at tap {k}: {} vs {}",
                phases[0][k],
                phases[2][FIR_TAPS - 1 - k]
            );
        }
    }

    #[test]
    fn impulse_in_five_minute_track_survives_all_pyramid_levels() {
        // 5-minute track @ 16 kHz = 300,000 ms (> MAX_BINS), with a single +1.0 impulse.
        let sample_rate = 16_000u32;
        let frames = sample_rate as usize * 300;
        let mut samples = vec![0.0_f32; frames];
        samples[frames / 2 + 7] = 1.0;
        let bins = analyse(&AudioClip {
            sample_rate,
            channels: 1,
            samples,
        });
        assert_eq!(bins.len(), MAX_BINS);
        let levels = build_pyramid_levels(&bins);
        assert!(levels.len() >= 4, "expected multi-level pyramid, got {}", levels.len());
        for (lvl_idx, lvl) in levels.iter().enumerate() {
            let max_peak = lvl.iter().map(|b| b.peak).fold(0.0_f32, f32::max);
            assert!(
                max_peak >= 0.99,
                "pyramid level {lvl_idx} (len={}) attenuated impulse to {max_peak}",
                lvl.len()
            );
        }
    }

    #[test]
    fn true_peak_detects_nyquist_inter_sample_overshoot() {
        // +0.5 dBTP fs/4 sine sampled at pi/4 offset: discrete sample magnitude ~0.749 (< 1.0),
        // continuous true peak ~1.059 (> 1.0).
        let frames = 480usize;
        let amp = 10.0_f32.powf(0.5 / 20.0);
        let samples: Vec<f32> = (0..frames)
            .map(|n| amp * (std::f32::consts::FRAC_PI_2 * n as f32 - std::f32::consts::FRAC_PI_4).cos())
            .collect();
        let bins = analyse(&AudioClip {
            sample_rate: 48_000,
            channels: 1,
            samples,
        });
        let mid_bin = &bins[bins.len() / 2];
        assert!(mid_bin.peak < 0.80, "discrete peak expected ~0.749, got {}", mid_bin.peak);
        assert!(
            mid_bin.true_peak >= 1.04 && (mid_bin.true_peak - amp).abs() < 0.02,
            "true_peak expected near +0.5 dBTP ({amp:.4}), got {}",
            mid_bin.true_peak
        );
    }
}
