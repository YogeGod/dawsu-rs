//! Display data belongs to Application, outside the audio callback.
//!
//! Follows osu!lazer's `osu.Framework.Audio.Track.Waveform` algorithm:
//! 1. 1 ms resolution stereo peak points (`amp_l`, `amp_r`, clamped to `[0, 1]`).
//! 2. 1024-sample Hann-windowed radix-2 FFT for Low (20–100 Hz), Mid (100–2000 Hz),
//!    and High (2000–12000 Hz) intensities.
//! 3. Gaussian weighted mean resampling down to at most `MAX_BINS` (45,000) bins.
//! 4. Track-wide normalization by `low_max`, `mid_max`, `high_max`.
use dawsu_audio::AudioClip;

const FFT_SIZE: usize = 1024;
const FFT_BINS: usize = FFT_SIZE / 2;
const LOW_MIN_HZ: f32 = 20.0;
const MID_MIN_HZ: f32 = 100.0;
const HIGH_MIN_HZ: f32 = 2000.0;
const HIGH_MAX_HZ: f32 = 12000.0;
const MAX_BINS: usize = 45_000;
const KERNEL_WIDTH_FACTOR: usize = 3;

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
}

#[derive(Clone, Copy, Default)]
struct RawPoint {
    amp_l: f32,
    amp_r: f32,
    low: f32,
    mid: f32,
    high: f32,
}

fn eval_gaussian(x: f32, sigma: f32) -> f32 {
    const INV_SQRT_2_PI: f32 = 0.398_942_3;
    INV_SQRT_2_PI * (-0.5 * x * x / (sigma * sigma)).exp() / sigma
}

fn resample_gaussian(points: &[RawPoint], target_count: usize) -> Vec<RawPoint> {
    if points.is_empty() || target_count == 0 {
        return Vec::new();
    }
    let points_per_generated = points.len() as f32 / target_count as f32;
    let kernel_width = (points_per_generated * KERNEL_WIDTH_FACTOR as f32) as usize + 1;
    let filter: Vec<f32> = (0..=kernel_width)
        .map(|i| eval_gaussian(i as f32, points_per_generated))
        .collect();

    let mut out = Vec::with_capacity(target_count);
    for generated_idx in 0..target_count {
        let center = (generated_idx as f32 * points_per_generated) as isize;
        let start_idx = center - kernel_width as isize;
        let end_idx = center + kernel_width as isize;

        let mut acc = RawPoint::default();
        let mut total_weight = 0.0_f32;

        let lo = start_idx.max(0) as usize;
        let hi = (end_idx.min(points.len() as isize)).max(0) as usize;
        for j in lo..hi {
            let dist = (j as isize - center).unsigned_abs().min(kernel_width);
            let w = filter[dist];
            total_weight += w;
            let p = &points[j];
            acc.amp_l += w * p.amp_l;
            acc.amp_r += w * p.amp_r;
            acc.low += w * p.low;
            acc.mid += w * p.mid;
            acc.high += w * p.high;
        }

        if total_weight > 0.0 {
            let inv = 1.0 / total_weight;
            acc.amp_l *= inv;
            acc.amp_r *= inv;
            acc.low *= inv;
            acc.mid *= inv;
            acc.high *= inv;
        }
        out.push(acc);
    }
    out
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
        for i in 1..FFT_SIZE {
            let mut bit = FFT_SIZE >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j ^= bit;
            rev[i] = j;
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

    // Pass 1: 1 ms stereo peaks (parallelized in point chunks).
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
                        pt.amp_l = amp_l.min(1.0);
                        pt.amp_r = amp_r.min(1.0);
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
            pt.amp_l = amp_l.min(1.0);
            pt.amp_r = amp_r.min(1.0);
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

    // Pass 3: Gaussian weighted mean downsampling when track exceeds MAX_BINS.
    let resampled = if points.len() > MAX_BINS {
        resample_gaussian(&points, MAX_BINS)
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
}
