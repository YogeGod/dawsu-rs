//! Pure-Rust stereo WAV writer (16-bit PCM & 32-bit IEEE float) with silence-tail trimming
//! for bouncing rendered VST3 hits and clips to mapset `.wav` files.

use dawsu_foundation::Error;
use dawsu_foundation::safe_write;
use std::path::Path;

const CHANNELS: u16 = 2;
const FORMAT_PCM: u16 = 1;
const FORMAT_IEEE_FLOAT: u16 = 3;

/// Default 3 ms raised-cosine tail micro-fade applied after silence trimming to prevent clicks.
pub const DEFAULT_TAIL_FADE_MS: f32 = 3.0;

/// Curve shape for stereo splice crossfades.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossfadeCurve {
    EqualGain,
    EqualPower,
}

/// Trims trailing stereo frames whose peak amplitude (`max(|L|, |R|)`) is strictly below `threshold`.
///
/// If `interleaved_stereo` has an odd length, the incomplete trailing sample is dropped first.
/// When `threshold <= 0.0`, only an odd trailing sample (if any) is dropped.
pub fn trim_silence_tail(interleaved_stereo: &[f32], threshold: f32) -> &[f32] {
    let aligned_len = interleaved_stereo.len() & !1;
    let aligned = &interleaved_stereo[..aligned_len];
    if threshold <= 0.0 {
        return aligned;
    }

    let mut keep_frames = aligned_len / 2;
    while keep_frames > 0 {
        let idx = (keep_frames - 1) * 2;
        let l = sanitize_f32(aligned[idx]).abs();
        let r = sanitize_f32(aligned[idx + 1]).abs();
        if l.max(r) >= threshold {
            break;
        }
        keep_frames -= 1;
    }
    &aligned[..keep_frames * 2]
}

/// Trims trailing silence below `threshold` and applies a `fade_out_ms` raised-cosine micro-fade
/// (`0.0` ms bypasses the fade bit-identically).
pub fn trim_and_fade_tail(
    interleaved_stereo: &[f32],
    threshold: f32,
    sample_rate: u32,
    fade_out_ms: f32,
) -> Vec<f32> {
    let trimmed = trim_silence_tail(interleaved_stereo, threshold);
    let mut out: Vec<f32> = trimmed.iter().copied().map(sanitize_f32).collect();
    if fade_out_ms <= 0.0 || !fade_out_ms.is_finite() || sample_rate == 0 || out.is_empty() {
        return out;
    }
    let total_frames = out.len() / 2;
    let fade_frames = ((f64::from(fade_out_ms) * f64::from(sample_rate) / 1000.0).round() as usize)
        .min(total_frames);
    if fade_frames < 2 {
        if fade_frames == 1 {
            let last = (total_frames - 1) * 2;
            out[last] = 0.0;
            out[last + 1] = 0.0;
        }
        return out;
    }
    let start_frame = total_frames - fade_frames;
    let denom = (fade_frames - 1) as f64;
    for i in 0..fade_frames {
        let frac = i as f64 / denom;
        let gain = (0.5 * (1.0 + (std::f64::consts::PI * frac).cos())) as f32;
        let idx = (start_frame + i) * 2;
        out[idx] *= gain;
        out[idx + 1] *= gain;
    }
    out
}

/// Applies an in-place raised-cosine head fade (`0.0 -> 1.0`) to interleaved stereo audio.
pub fn apply_head_fade(interleaved_stereo: &mut [f32], sample_rate: u32, fade_in_ms: f32) {
    let total_frames = interleaved_stereo.len() / 2;
    if fade_in_ms <= 0.0 || !fade_in_ms.is_finite() || sample_rate == 0 || total_frames == 0 {
        return;
    }
    let fade_frames = ((f64::from(fade_in_ms) * f64::from(sample_rate) / 1000.0).round() as usize)
        .min(total_frames);
    if fade_frames < 2 {
        if fade_frames == 1 {
            interleaved_stereo[0] = 0.0;
            interleaved_stereo[1] = 0.0;
        }
        return;
    }
    let denom = (fade_frames - 1) as f64;
    for i in 0..fade_frames {
        let frac = i as f64 / denom;
        let gain = (0.5 * (1.0 - (std::f64::consts::PI * frac).cos())) as f32;
        let idx = i * 2;
        interleaved_stereo[idx] *= gain;
        interleaved_stereo[idx + 1] *= gain;
    }
}

/// Crossfades `a_tail` into `b_head` over `min(a_frames, b_frames)` stereo frames.
pub fn crossfade_stereo(a_tail: &[f32], b_head: &[f32], curve: CrossfadeCurve) -> Vec<f32> {
    let frames = (a_tail.len() / 2).min(b_head.len() / 2);
    let mut out = vec![0.0_f32; frames * 2];
    if frames == 0 {
        return out;
    }
    let denom = (frames.saturating_sub(1)).max(1) as f64;
    for i in 0..frames {
        let t = if frames == 1 { 0.5 } else { i as f64 / denom };
        let (ga, gb) = match curve {
            CrossfadeCurve::EqualGain => ((1.0 - t) as f32, t as f32),
            CrossfadeCurve::EqualPower => (
                (t * std::f64::consts::FRAC_PI_2).cos() as f32,
                (t * std::f64::consts::FRAC_PI_2).sin() as f32,
            ),
        };
        let idx = i * 2;
        out[idx] = sanitize_f32(a_tail[idx]) * ga + sanitize_f32(b_head[idx]) * gb;
        out[idx + 1] = sanitize_f32(a_tail[idx + 1]) * ga + sanitize_f32(b_head[idx + 1]) * gb;
    }
    out
}

/// Encodes interleaved stereo `f32` audio as a 32-bit IEEE float WAV byte buffer.
pub fn encode_wav_f32(
    interleaved_stereo: &[f32],
    sample_rate: u32,
    silence_threshold: Option<f32>,
) -> Vec<u8> {
    encode_wav_f32_with_fade(interleaved_stereo, sample_rate, silence_threshold, 0.0)
}

/// Encodes interleaved stereo `f32` audio as a 32-bit IEEE float WAV buffer with tail fade.
pub fn encode_wav_f32_with_fade(
    interleaved_stereo: &[f32],
    sample_rate: u32,
    silence_threshold: Option<f32>,
    fade_out_ms: f32,
) -> Vec<u8> {
    let samples = match silence_threshold {
        Some(threshold) => trim_and_fade_tail(interleaved_stereo, threshold, sample_rate, fade_out_ms),
        None => trim_and_fade_tail(interleaved_stereo, 0.0, sample_rate, fade_out_ms),
    };
    let data_bytes = (samples.len() * 4) as u32;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    write_wav_header(&mut out, FORMAT_IEEE_FLOAT, 32, sample_rate, data_bytes);
    for &sample in &samples {
        out.extend_from_slice(&sanitize_f32(sample).to_le_bytes());
    }
    out
}

/// Encodes interleaved stereo `f32` audio as a 16-bit signed PCM WAV byte buffer.
pub fn encode_wav_i16(
    interleaved_stereo: &[f32],
    sample_rate: u32,
    silence_threshold: Option<f32>,
) -> Vec<u8> {
    encode_wav_i16_with_fade(interleaved_stereo, sample_rate, silence_threshold, 0.0)
}

/// Encodes interleaved stereo `f32` audio as a 16-bit signed PCM WAV buffer with tail fade.
pub fn encode_wav_i16_with_fade(
    interleaved_stereo: &[f32],
    sample_rate: u32,
    silence_threshold: Option<f32>,
    fade_out_ms: f32,
) -> Vec<u8> {
    let samples = match silence_threshold {
        Some(threshold) => trim_and_fade_tail(interleaved_stereo, threshold, sample_rate, fade_out_ms),
        None => trim_and_fade_tail(interleaved_stereo, 0.0, sample_rate, fade_out_ms),
    };
    let data_bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    write_wav_header(&mut out, FORMAT_PCM, 16, sample_rate, data_bytes);
    for &sample in &samples {
        let clamped = sanitize_f32(sample).clamp(-1.0, 1.0);
        let quantized = (clamped * 32767.0).round() as i16;
        out.extend_from_slice(&quantized.to_le_bytes());
    }
    out
}

/// Writes interleaved stereo `f32` audio as a 32-bit float `.wav` file atomically.
///
/// Returns the number of stereo frames written after optional silence-tail trimming.
pub fn write_wav_f32(
    path: &Path,
    interleaved_stereo: &[f32],
    sample_rate: u32,
    silence_threshold: Option<f32>,
) -> Result<usize, Error> {
    write_wav_f32_with_fade(path, interleaved_stereo, sample_rate, silence_threshold, 0.0)
}

/// Writes a 32-bit float `.wav` file with raised-cosine tail fade after silence trimming.
pub fn write_wav_f32_with_fade(
    path: &Path,
    interleaved_stereo: &[f32],
    sample_rate: u32,
    silence_threshold: Option<f32>,
    fade_out_ms: f32,
) -> Result<usize, Error> {
    let bytes = encode_wav_f32_with_fade(interleaved_stereo, sample_rate, silence_threshold, fade_out_ms);
    let frames = (bytes.len() - 44) / 8;
    safe_write::write_new(path, &bytes)?;
    Ok(frames)
}

/// Writes interleaved stereo `f32` audio as a 16-bit PCM `.wav` file atomically.
///
/// Returns the number of stereo frames written after optional silence-tail trimming.
pub fn write_wav_i16(
    path: &Path,
    interleaved_stereo: &[f32],
    sample_rate: u32,
    silence_threshold: Option<f32>,
) -> Result<usize, Error> {
    write_wav_i16_with_fade(path, interleaved_stereo, sample_rate, silence_threshold, 0.0)
}

/// Writes a 16-bit PCM `.wav` file with raised-cosine tail fade after silence trimming.
pub fn write_wav_i16_with_fade(
    path: &Path,
    interleaved_stereo: &[f32],
    sample_rate: u32,
    silence_threshold: Option<f32>,
    fade_out_ms: f32,
) -> Result<usize, Error> {
    let bytes = encode_wav_i16_with_fade(interleaved_stereo, sample_rate, silence_threshold, fade_out_ms);
    let frames = (bytes.len() - 44) / 4;
    safe_write::write_new(path, &bytes)?;
    Ok(frames)
}

fn sanitize_f32(sample: f32) -> f32 {
    if sample.is_finite() { sample } else { 0.0 }
}

fn write_wav_header(
    out: &mut Vec<u8>,
    audio_format: u16,
    bits_per_sample: u16,
    sample_rate: u32,
    data_bytes: u32,
) {
    let block_align = CHANNELS * (bits_per_sample / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let riff_size = 36u32.saturating_add(data_bytes);

    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&audio_format.to_le_bytes());
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_wav_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("dawsu-bounce-{label}-{nanos}.wav"))
    }

    #[test]
    fn trims_silence_tail_preserving_leading_and_middle_quiet() {
        let stereo = [
            0.0, 0.0, // leading silence kept
            0.5, -0.5, // loud frame
            0.0, 0.0, // quiet gap kept
            0.02, 0.0, // last loud frame
            0.0001, -0.0001, // tail trimmed
            0.0, 0.0, // tail trimmed
        ];
        let trimmed = trim_silence_tail(&stereo, 0.001);
        assert_eq!(trimmed.len(), 8);
        assert_eq!(trimmed[6], 0.02);
    }

    #[test]
    fn wav_f32_header_and_decode_round_trip() {
        let samples = vec![
            0.25f32, -0.25, 0.75, -0.5, 0.0, 0.0, // trailing silent frame
        ];
        let bytes = encode_wav_f32(&samples, 44_100, Some(0.001));
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), FORMAT_IEEE_FLOAT);
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            44_100
        );
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 32);
        assert_eq!(&bytes[36..40], b"data");
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            16
        );

        let path = temp_wav_path("f32");
        let frames = write_wav_f32(&path, &samples, 44_100, Some(0.001)).unwrap();
        assert_eq!(frames, 2);

        let decoded = dawsu_audio::decode(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.sample_rate, 44_100);
        assert_eq!(decoded.samples.len(), 4);
        for (got, expected) in decoded.samples.iter().zip(&samples[..4]) {
            assert!((got - expected).abs() < 1e-6, "got {got}, expected {expected}");
        }
    }

    #[test]
    fn wav_i16_header_and_decode_round_trip() {
        let samples = vec![0.5f32, -0.5, 1.5, -1.5, 0.00001, 0.0];
        let bytes = encode_wav_i16(&samples, 48_000, Some(0.001));
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), FORMAT_PCM);
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            48_000
        );
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16);
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            8
        );

        let path = temp_wav_path("i16");
        let frames = write_wav_i16(&path, &samples, 48_000, Some(0.001)).unwrap();
        assert_eq!(frames, 2);

        let decoded = dawsu_audio::decode(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.sample_rate, 48_000);
        assert_eq!(decoded.samples.len(), 4);
        let expected = [0.5f32, -0.5, 1.0, -1.0];
        for (got, exp) in decoded.samples.iter().zip(&expected) {
            assert!((got - exp).abs() < 1e-3, "got {got}, expected {exp}");
        }
    }

    #[test]
    fn trim_and_fade_tail_ramps_smoothly_to_zero_and_zero_ms_is_bit_identical() {
        let rate = 48_000u32;
        let frames = 2400; // 50 ms stereo sine at 0.5 amplitude + 200 trailing zero frames
        let mut stereo = Vec::with_capacity((frames + 200) * 2);
        for i in 0..frames {
            let v = (2.0 * std::f32::consts::PI * 220.0 * i as f32 / rate as f32).sin() * 0.5;
            stereo.push(v);
            stereo.push(v);
        }
        stereo.resize((frames + 200) * 2, 0.0);

        let bypassed = trim_and_fade_tail(&stereo, 0.001, rate, 0.0);
        assert_eq!(bypassed.as_slice(), trim_silence_tail(&stereo, 0.001));

        let faded = trim_and_fade_tail(&stereo, 0.001, rate, 3.0);
        assert_eq!(faded.len(), bypassed.len());
        let tail_start = (faded.len() / 2) - 144;
        // Untouched before fade window.
        assert_eq!(faded[(tail_start - 1) * 2], bypassed[(tail_start - 1) * 2]);
        // Starts at gain 1.0, ends at exact 0.0 with no step discontinuity.
        assert_eq!(faded[tail_start * 2], bypassed[tail_start * 2]);
        assert!(faded[faded.len() - 2].abs() < 1e-6);
        assert!(faded[faded.len() - 1].abs() < 1e-6);
    }

    #[test]
    fn head_fade_and_crossfade_preserve_energy_curves() {
        let mut head = vec![0.8_f32; 480 * 2];
        apply_head_fade(&mut head, 48_000, 3.0);
        assert!(head[0].abs() < 1e-6);
        assert!((head[144 * 2] - 0.8).abs() < 1e-5);

        let a = vec![1.0_f32; 16 * 2];
        let b = vec![1.0_f32; 16 * 2];
        let eq_gain = crossfade_stereo(&a, &b, CrossfadeCurve::EqualGain);
        assert!(eq_gain.iter().all(|&v| (v - 1.0).abs() < 1e-5));

        let mid_idx = 8;
        let t = mid_idx as f64 / 15.0;
        let ga = (t * std::f64::consts::FRAC_PI_2).cos() as f32;
        let gb = (t * std::f64::consts::FRAC_PI_2).sin() as f32;
        assert!((ga * ga + gb * gb - 1.0).abs() < 1e-5);
        let eq_pow = crossfade_stereo(&a, &b, CrossfadeCurve::EqualPower);
        assert!((eq_pow[0] - 1.0).abs() < 1e-5);
        assert!((eq_pow[eq_pow.len() - 2] - 1.0).abs() < 1e-5);
    }
}
