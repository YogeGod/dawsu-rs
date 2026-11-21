//! Display data belongs to Application, outside the audio callback.
//!
//! Follows osu!lazer's `osu.Framework.Audio.Track.Waveform` algorithm:
//! 1. 1 ms resolution stereo peak points (`amp_l`, `amp_r`, clamped to `[0, 1]`).
//! 2. 1024-sample Hann-windowed FFT (`rustfft`) for Low (20–100 Hz), Mid (100–2000 Hz),
//!    and High (2000–12000 Hz) intensities.
//! 3. Gaussian weighted mean resampling down to at most `MAX_BINS` (12,000) bins.
//! 4. Track-wide normalization by `low_max`, `mid_max`, `high_max`.
use dawsu_audio::AudioClip;
use rustfft::{FftPlanner, num_complex::Complex};

const FFT_SIZE: usize = 1024;
const FFT_BINS: usize = FFT_SIZE / 2;
const LOW_MIN_HZ: f32 = 20.0;
const MID_MIN_HZ: f32 = 100.0;
const HIGH_MIN_HZ: f32 = 2000.0;
const HIGH_MAX_HZ: f32 = 12000.0;
const MAX_BINS: usize = 12_000;
const KERNEL_WIDTH_FACTOR: usize = 3;

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

fn compute_intensity(magnitudes: &[f32], sample_rate: f32, start_freq: f32, end_freq: f32) -> f32 {
    let start_bin = ((FFT_BINS as f32 * 2.0 * start_freq / sample_rate) as usize).min(magnitudes.len());
    let end_bin = ((FFT_BINS as f32 * 2.0 * end_freq / sample_rate) as usize).min(magnitudes.len());
    let mut sum = 0.0_f32;
    for &mag in &magnitudes[start_bin..end_bin] {
        sum += mag;
    }
    sum
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

    // Pass 1: 1 ms resolution stereo peak points.
    let frames_per_point = (audio.sample_rate as usize / 1000).max(1);
    let second_ch = if channels > 1 { 1 } else { 0 };
    let point_count = frames.div_ceil(frames_per_point);
    let mut points = Vec::with_capacity(point_count);

    for point_idx in 0..point_count {
        let start_frame = point_idx * frames_per_point;
        let end_frame = (start_frame + frames_per_point).min(frames);
        let mut amp_l = 0.0_f32;
        let mut amp_r = 0.0_f32;
        for f in start_frame..end_frame {
            let base = f * channels;
            amp_l = amp_l.max(audio.samples[base].abs());
            amp_r = amp_r.max(audio.samples[base + second_ch].abs());
        }
        points.push(RawPoint {
            amp_l: amp_l.min(1.0),
            amp_r: amp_r.min(1.0),
            low: 0.0,
            mid: 0.0,
            high: 0.0,
        });
    }

    // Pass 2: 1024-sample Hann-windowed FFT stepped across the track.
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|n| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (FFT_SIZE - 1) as f32).cos())
        })
        .collect();
    let mut fft_buf = vec![Complex::new(0.0_f32, 0.0_f32); FFT_SIZE];
    let mut scratch = vec![Complex::new(0.0_f32, 0.0_f32); fft.get_inplace_scratch_len()];
    let mut magnitudes = vec![0.0_f32; FFT_BINS];
    let sample_rate = audio.sample_rate as f32;
    let inv_channels = 1.0 / channels as f32;

    let mut current_point = 0usize;
    let mut frame_pos = 0usize;
    while frame_pos < frames {
        let available = (frames - frame_pos).min(FFT_SIZE);
        for i in 0..available {
            let base = (frame_pos + i) * channels;
            let mut mono = 0.0_f32;
            for c in 0..channels {
                mono += audio.samples[base + c];
            }
            fft_buf[i] = Complex::new(mono * inv_channels * window[i], 0.0);
        }
        for slot in &mut fft_buf[available..FFT_SIZE] {
            *slot = Complex::new(0.0, 0.0);
        }

        fft.process_with_scratch(&mut fft_buf, &mut scratch);

        for (mag, c) in magnitudes.iter_mut().zip(fft_buf.iter().take(FFT_BINS)) {
            *mag = c.norm();
        }

        let low = compute_intensity(&magnitudes, sample_rate, LOW_MIN_HZ, MID_MIN_HZ);
        let mid = compute_intensity(&magnitudes, sample_rate, MID_MIN_HZ, HIGH_MIN_HZ);
        let high = compute_intensity(&magnitudes, sample_rate, HIGH_MIN_HZ, HIGH_MAX_HZ);

        frame_pos += available;
        while current_point < points.len() && current_point * frames_per_point < frame_pos {
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

        // 20-second mono track (20,000 ms) resamples down to MAX_BINS (12,000).
        let long = AudioClip {
            sample_rate: 8_000,
            channels: 1,
            samples: vec![0.5; 8_000 * 20],
        };
        let long_bins = analyse(&long);
        assert_eq!(long_bins.len(), MAX_BINS);
        assert!((long_bins[6000].peak - 0.5).abs() < 0.02);
    }
}
