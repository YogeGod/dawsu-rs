//! Shared offline bounce planner, uniform batch peak normalization, Ogg Vorbis encoding,
//! choke-group fade-out, waveform thumbnail peaks, and `.osz` 10 MB ranking budget math
//! shared by Drums VST3 Bounce (D4) and Keysounding Bounce (K4).

use serde::{Deserialize, Serialize};
use std::num::{NonZeroU8, NonZeroU32};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

/// osu! ranked `.osz` size ceiling in bytes (10.0 MiB).
pub const OSZ_RANKING_LIMIT_BYTES: u64 = 10 * 1024 * 1024;
/// Per-note render watchdog timeout so a hung VST3 sidecar never stalls a bounce forever.
pub const PER_NOTE_RENDER_TIMEOUT: Duration = Duration::from_secs(15);
/// Minimum peak below which a rendered clip is flagged as silent.
pub const SILENT_PEAK_THRESHOLD: f32 = 1e-6;
/// Anti-click fade-out length in milliseconds applied when a choke group cuts a sample early.
pub const CHOKE_FADE_OUT_MS: f64 = 10.0;

/// Cancellation token shared between the UI cancel command and background bounce threads.
#[derive(Clone, Default)]
pub struct BounceCancelToken(Arc<AtomicBool>);

impl BounceCancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn reset(&self) {
        self.0.store(false, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Output audio format for bounced drum/keysound samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BounceAudioFormat {
    #[default]
    OggVorbis,
    Wav16,
}

impl BounceAudioFormat {
    /// Target sample-peak ceiling in `dBFS` (sample peak, not `dBTP`).
    pub const fn target_peak_dbfs(self) -> f32 {
        match self {
            Self::Wav16 => -1.0,
            Self::OggVorbis => -1.5,
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Wav16 => "wav",
            Self::OggVorbis => "ogg",
        }
    }
}

/// Result of applying ONE uniform batch gain across an entire kit or instrument so velocity
/// dynamics (ghost notes vs. hard hits) stay intact.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchNormReport {
    pub applied_gain_linear: f32,
    pub applied_gain_db: f32,
    pub peak_before_dbfs: f32,
    pub target_dbfs: f32,
    pub silent_clip_indices: Vec<usize>,
}

/// Normalizes all clips in `clips` by ONE shared linear gain factor so the loudest clip hits
/// `target_dbfs` while quieter velocity tiers preserve their exact relative loudness.
/// Non-finite (`NaN`/`Inf`) samples are sanitized to `0.0` so a misbehaving plugin cannot zero out the kit.
pub fn uniform_batch_normalize(clips: &mut [Vec<f32>], target_dbfs: f32) -> BatchNormReport {
    let mut batch_peak = 0.0f32;
    let mut silent_clip_indices = Vec::new();
    for (idx, clip) in clips.iter_mut().enumerate() {
        let mut clip_peak = 0.0f32;
        for sample in clip.iter_mut() {
            if !sample.is_finite() {
                *sample = 0.0;
            }
            clip_peak = clip_peak.max(sample.abs());
        }
        if clip_peak < SILENT_PEAK_THRESHOLD {
            silent_clip_indices.push(idx);
        }
        batch_peak = batch_peak.max(clip_peak);
    }
    if batch_peak < SILENT_PEAK_THRESHOLD || !batch_peak.is_finite() {
        return BatchNormReport {
            applied_gain_linear: 1.0,
            applied_gain_db: 0.0,
            peak_before_dbfs: f32::NEG_INFINITY,
            target_dbfs,
            silent_clip_indices,
        };
    }
    let target_linear = 10.0f32.powf(target_dbfs / 20.0);
    let gain = target_linear / batch_peak;
    for clip in clips.iter_mut() {
        for sample in clip.iter_mut() {
            *sample = (*sample * gain).clamp(-1.0, 1.0);
        }
    }
    BatchNormReport {
        applied_gain_linear: gain,
        applied_gain_db: 20.0 * gain.log10(),
        peak_before_dbfs: 20.0 * batch_peak.log10(),
        target_dbfs,
        silent_clip_indices,
    }
}

/// Truncates an interleaved stereo clip to `choke_duration_ms` with a 10 ms anti-click fade-out.
pub fn apply_choke_fade(interleaved_stereo: &mut Vec<f32>, sample_rate: u32, choke_duration_ms: u32) {
    let channels = 2usize;
    if !interleaved_stereo.len().is_multiple_of(channels) {
        interleaved_stereo.pop();
    }
    let max_frames = ((f64::from(choke_duration_ms) / 1000.0) * f64::from(sample_rate.max(1))).round() as usize;
    let current_frames = interleaved_stereo.len() / channels;
    if max_frames == 0 || max_frames >= current_frames {
        return;
    }
    interleaved_stereo.truncate(max_frames * channels);
    let fade_frames = ((CHOKE_FADE_OUT_MS / 1000.0) * f64::from(sample_rate.max(1)))
        .round()
        .max(1.0) as usize;
    let fade_len = fade_frames.min(max_frames);
    let fade_start = max_frames - fade_len;
    for f in 0..fade_len {
        let gain = 1.0 - ((f + 1) as f32 / fade_len as f32);
        let base = (fade_start + f) * channels;
        interleaved_stereo[base] *= gain;
        interleaved_stereo[base + 1] *= gain;
    }
}

/// Quantizes an optional choke duration in milliseconds to 25 ms buckets for bounce deduplication,
/// ensuring choked open hats get their own key and never truncate unchoked open hats.
pub fn quantize_choke_ms(choke_duration_ms: Option<u32>) -> Option<u32> {
    choke_duration_ms.map(|ms| {
        let clamped = ms.clamp(25, 4000);
        ((clamped + 12) / 25) * 25
    })
}

/// Extracts `bars` normalized peak bars (`0.0..=1.0`) for SVG waveform audition cards.
pub fn extract_waveform_peaks(interleaved_stereo: &[f32], bars: usize) -> Vec<f32> {
    let bars = bars.clamp(8, 128);
    if interleaved_stereo.is_empty() {
        return vec![0.0; bars];
    }
    let chunk = (interleaved_stereo.len() / bars).max(1);
    (0..bars)
        .map(|i| {
            let start = i * chunk;
            let end = ((i + 1) * chunk).min(interleaved_stereo.len());
            if start >= interleaved_stereo.len() {
                0.0
            } else {
                interleaved_stereo[start..end]
                    .iter()
                    .fold(0.0f32, |acc, &s| acc.max(s.abs()))
                    .min(1.0)
            }
        })
        .collect()
}

/// Encodes interleaved stereo f32 PCM into Ogg Vorbis bytes reusing `vorbis_rs`.
pub fn encode_vorbis_ogg(interleaved_stereo: &[f32], sample_rate: u32) -> Result<Vec<u8>, String> {
    if !(8_000..=192_000).contains(&sample_rate) {
        return Err(format!("Unsupported Ogg Vorbis sample rate: {sample_rate} Hz (expected 8000–192000 Hz)"));
    }
    let frames = interleaved_stereo.len() / 2;
    if frames == 0 {
        return Err("Cannot encode empty audio clip to Ogg Vorbis".to_string());
    }
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for pair in interleaved_stereo.as_chunks::<2>().0 {
        let l = if pair[0].is_finite() { pair[0].clamp(-1.0, 1.0) } else { 0.0 };
        let r = if pair[1].is_finite() { pair[1].clamp(-1.0, 1.0) } else { 0.0 };
        left.push(l);
        right.push(r);
    }
    let mut out = Vec::new();
    let sr = NonZeroU32::new(sample_rate).ok_or("Invalid sample rate")?;
    let ch = NonZeroU8::new(2).ok_or("Invalid channel count")?;
    let mut encoder = VorbisEncoderBuilder::new_with_serial(sr, ch, &mut out, 0)
        .bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
            target_quality: 0.5,
        })
        .build()
        .map_err(|e| format!("Vorbis encoder build failed: {e}"))?;
    encoder
        .encode_audio_block([left.as_slice(), right.as_slice()])
        .map_err(|e| format!("Vorbis encode failed: {e}"))?;
    encoder
        .finish()
        .map_err(|e| format!("Vorbis finish failed: {e}"))?;
    Ok(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStatus {
    Safe,
    Caution,
    OverLimit,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OszBudgetSummary {
    pub base_bytes: u64,
    pub bounced_bytes: u64,
    pub total_bytes: u64,
    pub limit_bytes: u64,
    pub percent_used: f32,
    pub status: BudgetStatus,
}

pub fn calculate_osz_budget(base_bytes: u64, bounced_bytes: u64) -> OszBudgetSummary {
    let total_bytes = base_bytes.saturating_add(bounced_bytes);
    let percent_used = ((total_bytes as f64 / OSZ_RANKING_LIMIT_BYTES as f64) * 100.0) as f32;
    let status = if total_bytes > OSZ_RANKING_LIMIT_BYTES {
        BudgetStatus::OverLimit
    } else if percent_used >= 85.0 {
        BudgetStatus::Caution
    } else {
        BudgetStatus::Safe
    };
    OszBudgetSummary {
        base_bytes,
        bounced_bytes,
        total_bytes,
        limit_bytes: OSZ_RANKING_LIMIT_BYTES,
        percent_used,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_batch_normalize_preserves_velocity_dynamics_and_flags_silence() {
        let mut clips = vec![
            vec![0.1, -0.1, 0.05, -0.05], // ghost note (half loudness)
            vec![0.2, -0.2, 0.1, -0.1],   // hard hit
            vec![0.0, 0.0, 0.0, 0.0],     // silent render
        ];
        let report = uniform_batch_normalize(&mut clips, -1.0);
        assert_eq!(report.silent_clip_indices, vec![2]);
        let ghost_peak = clips[0].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let hard_peak = clips[1].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        let expected_hard = 10.0f32.powf(-1.0 / 20.0);
        assert!((hard_peak - expected_hard).abs() < 1e-4);
        assert!(((ghost_peak / hard_peak) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn choke_fade_truncates_with_smooth_tail_and_quantizes_dedup_key() {
        let mut samples = vec![0.8f32; 48_000 * 2]; // 1000 ms stereo
        apply_choke_fade(&mut samples, 48_000, 100);
        assert_eq!(samples.len(), 4_800 * 2);
        assert!(samples[samples.len() - 1].abs() < 0.01);
        assert_eq!(quantize_choke_ms(Some(118)), Some(125));
        assert_eq!(quantize_choke_ms(None), None);
    }

    #[test]
    fn vorbis_ogg_encodes_valid_ogg_header_and_budget_thresholds() {
        let samples = vec![0.25f32; 4800 * 2];
        let ogg = encode_vorbis_ogg(&samples, 48_000).unwrap();
        assert!(ogg.starts_with(b"OggS"));
        let safe = calculate_osz_budget(4 * 1024 * 1024, 1024 * 1024);
        assert_eq!(safe.status, BudgetStatus::Safe);
        let over = calculate_osz_budget(9 * 1024 * 1024, 2 * 1024 * 1024);
        assert_eq!(over.status, BudgetStatus::OverLimit);
    }

    #[test]
    fn guards_non_finite_samples_odd_buffers_and_empty_ogg() {
        let mut clips = vec![vec![f32::NAN, f32::INFINITY, 0.25, -0.25]];
        let report = uniform_batch_normalize(&mut clips, -1.0);
        assert!(report.applied_gain_linear.is_finite() && report.applied_gain_linear > 1.0);
        assert!(clips[0].iter().all(|s| s.is_finite()));

        let mut odd = vec![0.5f32; 48_000 * 2 + 1];
        apply_choke_fade(&mut odd, 48_000, 100);
        assert_eq!(odd.len(), 4_800 * 2);

        assert!(encode_vorbis_ogg(&[], 48_000).is_err());
        assert!(encode_vorbis_ogg(&[0.1; 1000], 4_000).is_err());
    }
}
