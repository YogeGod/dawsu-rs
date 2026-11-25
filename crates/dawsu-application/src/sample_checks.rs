//! osu! Ranking Criteria (RC) and playback health checks for hitsound samples, plus opt-in
//! auto-fix for newly staged PCM WAV mixes.
//!
//! Existing mapset files are never mutated without explicit consent: checks surface plain-English
//! warnings during output review or on-demand mapset audits, and auto-fix is strictly opt-in.

use serde::Serialize;
use std::fs;
use std::path::Path;

/// osu! Ranking Criteria minimum length for non-silent hitsound samples.
pub const MIN_DURATION_MS: f64 = 25.0;
/// Maximum delay from sample start to 50% of peak amplitude before a hit feels noticeably late.
pub const MAX_IMPACT_DELAY_MS: f64 = 5.0;
/// Minimum byte size for non-blank samples in osu!stable (smaller non-silent files are skipped).
pub const MIN_STABLE_BYTES: usize = 1024;
/// Standard compact silent 16-bit PCM WAV header size (0 data bytes).
pub const STANDARD_SILENT_WAV_BYTES: usize = 44;
/// Peak amplitude below -90 dBFS (`10^(-90/20)`) is treated as completely silent.
pub const SILENCE_PEAK_THRESHOLD: f32 = 3.162_277_7e-5;
/// Threshold (-60 dBFS, `10^(-60/20)`) used when trimming leading dead air.
pub const TRIM_SILENCE_THRESHOLD: f32 = 1.0e-3;
/// Short pre-transient margin preserved when trimming leading silence so attacks never click.
pub const ANTI_CLICK_MARGIN_MS: f64 = 0.5;

/// A single Ranking Criteria or osu! playback health issue found on a hitsound sample.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SampleHealthIssue {
    /// Non-silent audio shorter than 25 ms.
    TooShort { duration_ms: f64 },
    /// Time from sample start to 50% of peak amplitude exceeds 5 ms.
    LateImpact { impact_ms: f64 },
    /// Non-blank file smaller than 1024 bytes (osu!stable silently drops it).
    TooSmallForStable { bytes: usize },
    /// Completely silent audio (peak < -90 dBFS) that is not a compact 44-byte blank WAV.
    NonStandardSilence { bytes: usize },
}

impl SampleHealthIssue {
    /// Plain-English warning for osu! mappers.
    pub fn plain_english(&self, name: &str) -> String {
        match *self {
            Self::TooShort { duration_ms } => format!(
                "{name} is only {duration_ms:.1} ms long (under 25 ms) — may fail osu! Ranking Criteria or sound cut off."
            ),
            Self::LateImpact { impact_ms } => format!(
                "{name} starts {impact_ms:.1} ms late (leading silence) — may feel off-beat in osu!"
            ),
            Self::TooSmallForStable { bytes } => format!(
                "{name} is only {bytes} bytes (under 1024 bytes) — osu!stable may silently drop it."
            ),
            Self::NonStandardSilence { bytes } => format!(
                "{name} is silent audio ({bytes} bytes) — replace with a standard 44-byte blank WAV."
            ),
        }
    }
}

/// Health check summary for a single hitsound sample file or staged WAV.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SampleHealthReport {
    pub name: String,
    pub bytes: usize,
    pub duration_ms: f64,
    pub peak: f32,
    pub impact_ms: Option<f64>,
    pub issues: Vec<SampleHealthIssue>,
}

impl SampleHealthReport {
    pub fn is_healthy(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn warnings(&self) -> Vec<String> {
        self.issues
            .iter()
            .map(|issue| issue.plain_english(&self.name))
            .collect()
    }
}

/// Output of `auto_fix_wav_pcm`.
#[derive(Clone, Debug, PartialEq)]
pub struct FixedWav {
    pub wav: Vec<u8>,
    pub sample_rate: u32,
    pub channels: usize,
    pub trimmed_ms: f64,
    pub padded_ms: f64,
    pub replaced_with_silent_wav: bool,
}

/// Inspects interleaved floating-point PCM samples against osu! RC and playback rules.
pub fn inspect_pcm(
    name: impl Into<String>,
    samples: &[f32],
    sample_rate: u32,
    channels: usize,
    file_bytes: usize,
    is_wav: bool,
) -> SampleHealthReport {
    let name = name.into();
    let channels = channels.max(1);
    let sample_rate = sample_rate.max(1);
    let frames = samples.len() / channels;
    let duration_ms = frames as f64 * 1000.0 / f64::from(sample_rate);
    let usable = &samples[..frames * channels];
    let peak = usable.iter().fold(0.0_f32, |acc, &s| {
        if s.is_finite() { acc.max(s.abs()) } else { acc }
    });

    let mut issues = Vec::new();
    let is_silent = frames == 0 || peak < SILENCE_PEAK_THRESHOLD;
    if is_silent {
        if !(is_wav && file_bytes == STANDARD_SILENT_WAV_BYTES) {
            issues.push(SampleHealthIssue::NonStandardSilence { bytes: file_bytes });
        }
        return SampleHealthReport {
            name,
            bytes: file_bytes,
            duration_ms,
            peak,
            impact_ms: None,
            issues,
        };
    }

    let half_peak = peak * 0.5;
    let impact_frame = (0..frames)
        .find(|&frame| {
            let start = frame * channels;
            usable[start..start + channels]
                .iter()
                .any(|&s| s.is_finite() && s.abs() >= half_peak)
        })
        .unwrap_or(0);
    let impact_ms = impact_frame as f64 * 1000.0 / f64::from(sample_rate);

    if duration_ms < MIN_DURATION_MS {
        issues.push(SampleHealthIssue::TooShort { duration_ms });
    }
    if impact_ms > MAX_IMPACT_DELAY_MS {
        issues.push(SampleHealthIssue::LateImpact { impact_ms });
    }
    if file_bytes < MIN_STABLE_BYTES {
        issues.push(SampleHealthIssue::TooSmallForStable { bytes: file_bytes });
    }

    SampleHealthReport {
        name,
        bytes: file_bytes,
        duration_ms,
        peak,
        impact_ms: Some(impact_ms),
        issues,
    }
}

/// Inspects an in-memory PCM WAV buffer (such as a newly staged mix).
/// Returns `None` if `wav` is not a recognized PCM WAV container.
pub fn inspect_wav_bytes(name: impl Into<String>, wav: &[u8]) -> Option<SampleHealthReport> {
    let parsed = parse_pcm_wav(wav)?;
    Some(inspect_pcm(
        name,
        &parsed.samples,
        parsed.sample_rate,
        parsed.channels,
        wav.len(),
        true,
    ))
}

/// Inspects a WAV, OGG, or MP3 sample file on disk.
pub fn inspect_file(path: &Path) -> Result<SampleHealthReport, String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let file_bytes = fs::metadata(path)
        .map_err(|error| format!("Could not read metadata for {name}: {error}"))?
        .len() as usize;
    let is_wav = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"));

    // Symphonia may reject 0-frame 44-byte blank WAVs; check them directly first.
    if is_wav && file_bytes == STANDARD_SILENT_WAV_BYTES
        && let Ok(bytes) = fs::read(path)
            && let Some(report) = inspect_wav_bytes(&name, &bytes)
        {
            return Ok(report);
        }

    let clip = dawsu_audio::decode(path)
        .map_err(|error| format!("Could not decode {name}: {error}"))?;
    Ok(inspect_pcm(
        name,
        &clip.samples,
        clip.sample_rate,
        clip.channels,
        file_bytes,
        is_wav,
    ))
}

/// Audits all hitsound audio files (`.wav`, `.ogg`, `.mp3`) in `root`, skipping `ignore_song_audio`
/// (case-insensitive) when provided. Returns only samples that have at least one RC health issue.
pub fn audit_folder(
    root: &Path,
    files: &[String],
    ignore_song_audio: Option<&str>,
) -> Vec<SampleHealthReport> {
    let ignore = ignore_song_audio.map(|s| s.replace('\\', "/").to_ascii_lowercase());
    let mut reports = Vec::new();
    for relative in files {
        let normalized = relative.replace('\\', "/");
        if ignore
            .as_deref()
            .is_some_and(|song| normalized.eq_ignore_ascii_case(song))
        {
            continue;
        }
        let lower = normalized.to_ascii_lowercase();
        if !(lower.ends_with(".wav") || lower.ends_with(".ogg") || lower.ends_with(".mp3")) {
            continue;
        }
        let Ok(path) = crate::safe_child(root, &normalized) else {
            continue;
        };
        if let Ok(report) = inspect_file(&path)
            && !report.is_healthy()
        {
            reports.push(report);
        }
    }
    reports
}

/// Applies safe osu! RC auto-fixes to interleaved PCM samples and encodes a 16-bit PCM WAV:
/// - Completely silent (`peak < -90 dBFS`) -> standard 44-byte silent PCM WAV.
/// - `LateImpact` (> 5 ms) -> trims leading silence below -60 dBFS, preserving a 0.5 ms anti-click margin.
/// - `TooShort` (< 25 ms) or `< 1024` bytes -> pads trailing zeros so duration >= 25 ms and size >= 1024 bytes.
pub fn auto_fix_wav_pcm(
    samples: &[f32],
    sample_rate: u32,
    channels: usize,
) -> Result<FixedWav, String> {
    let channels = channels.max(1);
    let sample_rate = sample_rate.max(1);
    let frames = samples.len() / channels;
    let usable = &samples[..frames * channels];
    let peak = usable.iter().fold(0.0_f32, |acc, &s| {
        if s.is_finite() { acc.max(s.abs()) } else { acc }
    });

    if frames == 0 || peak < SILENCE_PEAK_THRESHOLD {
        return Ok(FixedWav {
            wav: silent_wav_44(sample_rate, channels),
            sample_rate,
            channels,
            trimmed_ms: 0.0,
            padded_ms: 0.0,
            replaced_with_silent_wav: true,
        });
    }

    let half_peak = peak * 0.5;
    let impact_frame = (0..frames)
        .find(|&frame| {
            let start = frame * channels;
            usable[start..start + channels]
                .iter()
                .any(|&s| s.is_finite() && s.abs() >= half_peak)
        })
        .unwrap_or(0);
    let impact_ms = impact_frame as f64 * 1000.0 / f64::from(sample_rate);

    let mut start_frame = 0usize;
    if impact_ms > MAX_IMPACT_DELAY_MS {
        let first_audible = (0..=impact_frame)
            .find(|&frame| {
                let start = frame * channels;
                usable[start..start + channels]
                    .iter()
                    .any(|&s| s.is_finite() && s.abs() >= TRIM_SILENCE_THRESHOLD)
            })
            .unwrap_or(impact_frame);
        let margin_frames =
            (ANTI_CLICK_MARGIN_MS * f64::from(sample_rate) / 1000.0).round() as usize;
        start_frame = first_audible.saturating_sub(margin_frames);
    }

    let trimmed_ms = start_frame as f64 * 1000.0 / f64::from(sample_rate);
    let mut out_samples = usable[start_frame * channels..].to_vec();

    let min_duration_frames =
        (MIN_DURATION_MS * f64::from(sample_rate) / 1000.0).ceil() as usize;
    let min_data_bytes = MIN_STABLE_BYTES.saturating_sub(STANDARD_SILENT_WAV_BYTES);
    let bytes_per_frame = channels.saturating_mul(2).max(2);
    let min_size_frames = min_data_bytes.div_ceil(bytes_per_frame);
    let target_frames = min_duration_frames.max(min_size_frames);

    let current_frames = out_samples.len() / channels;
    let padded_frames = target_frames.saturating_sub(current_frames);
    if padded_frames > 0 {
        out_samples.resize(target_frames * channels, 0.0);
    }
    let padded_ms = padded_frames as f64 * 1000.0 / f64::from(sample_rate);

    let wav = encode_pcm16_wav(&out_samples, sample_rate, channels)?;
    Ok(FixedWav {
        wav,
        sample_rate,
        channels,
        trimmed_ms,
        padded_ms,
        replaced_with_silent_wav: false,
    })
}

/// Auto-fixes an in-memory PCM WAV buffer if it has any RC health issue.
/// Returns `Ok(None)` when the WAV is already healthy or not a recognized PCM WAV.
pub fn auto_fix_wav_bytes(wav: &[u8]) -> Result<Option<FixedWav>, String> {
    let Some(parsed) = parse_pcm_wav(wav) else {
        return Ok(None);
    };
    let report = inspect_pcm(
        "",
        &parsed.samples,
        parsed.sample_rate,
        parsed.channels,
        wav.len(),
        true,
    );
    if report.is_healthy() {
        return Ok(None);
    }
    auto_fix_wav_pcm(&parsed.samples, parsed.sample_rate, parsed.channels).map(Some)
}

/// Builds a standard 44-byte silent 16-bit PCM WAV header (0 data bytes).
pub fn silent_wav_44(sample_rate: u32, channels: usize) -> Vec<u8> {
    let channels_u16 = (channels.clamp(1, 2)) as u16;
    let rate = sample_rate.max(1);
    let mut wav = Vec::with_capacity(STANDARD_SILENT_WAV_BYTES);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&36u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels_u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * u32::from(channels_u16) * 2).to_le_bytes());
    wav.extend_from_slice(&(channels_u16 * 2).to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&0u32.to_le_bytes());
    wav
}

/// Encodes interleaved f32 samples into a standard 16-bit PCM WAV buffer.
pub fn encode_pcm16_wav(
    samples: &[f32],
    sample_rate: u32,
    channels: usize,
) -> Result<Vec<u8>, String> {
    let channels_u16 = u16::try_from(channels.max(1)).map_err(|_| "Invalid channel count")?;
    let rate = sample_rate.max(1);
    let data_size = u32::try_from(samples.len().saturating_mul(2))
        .ok()
        .filter(|size| *size <= u32::MAX - 36)
        .ok_or("Sample mix is too long for a WAV file")?;
    let mut wav = Vec::new();
    wav.try_reserve_exact(44 + data_size as usize)
        .map_err(|error| error.to_string())?;
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels_u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * u32::from(channels_u16) * 2).to_le_bytes());
    wav.extend_from_slice(&(channels_u16 * 2).to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for &value in samples {
        let clamped = if value.is_finite() {
            value.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        wav.extend_from_slice(&((clamped * 32768.0).round() as i16).to_le_bytes());
    }
    Ok(wav)
}

struct ParsedWav {
    samples: Vec<f32>,
    sample_rate: u32,
    channels: usize,
}

fn parse_pcm_wav(wav: &[u8]) -> Option<ParsedWav> {
    if wav.len() < 44 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut offset = 12usize;
    let mut audio_format = 0u16;
    let mut channels = 0usize;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut data_slice: Option<&[u8]> = None;

    while offset + 8 <= wav.len() {
        let chunk_id = &wav[offset..offset + 4];
        let chunk_size = u32::from_le_bytes(wav[offset + 4..offset + 8].try_into().ok()?) as usize;
        let payload_start = offset + 8;
        let payload_end = payload_start.checked_add(chunk_size)?.min(wav.len());
        let payload = &wav[payload_start..payload_end];

        if chunk_id == b"fmt " && payload.len() >= 16 {
            audio_format = u16::from_le_bytes([payload[0], payload[1]]);
            channels = usize::from(u16::from_le_bytes([payload[2], payload[3]]));
            sample_rate = u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
            bits_per_sample = u16::from_le_bytes([payload[14], payload[15]]);
        } else if chunk_id == b"data" {
            data_slice = Some(payload);
            break;
        }
        let padded = chunk_size.saturating_add(chunk_size & 1);
        offset = payload_start.checked_add(padded)?;
    }

    if channels == 0 || sample_rate == 0 {
        return None;
    }
    let data = data_slice?;
    let samples = match (audio_format, bits_per_sample) {
        (1, 16) => data
            .as_chunks::<2>().0.iter()
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
            .collect(),
        (1, 8) => data
            .iter()
            .map(|&b| (f32::from(b) - 128.0) / 128.0)
            .collect(),
        (1, 24) => data
            .as_chunks::<3>().0.iter()
            .map(|b| {
                let raw = i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
                raw as f32 / 8_388_608.0
            })
            .collect(),
        (3, 32) => data
            .as_chunks::<4>().0.iter()
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
        _ => return None,
    };

    Some(ParsedWav {
        samples,
        sample_rate,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn drum_hit_pcm(duration_ms: f64, leading_silence_ms: f64, channels: usize) -> Vec<f32> {
        let lead_frames = (leading_silence_ms * f64::from(RATE) / 1000.0).round() as usize;
        let total_frames = (duration_ms * f64::from(RATE) / 1000.0).round() as usize;
        let mut samples = vec![0.0_f32; total_frames * channels];
        for frame in lead_frames..total_frames {
            let t = (frame - lead_frames) as f32 / RATE as f32;
            let env = (-t * 45.0).exp();
            let value = (2.0 * std::f32::consts::PI * 180.0 * t).sin() * env * 0.85;
            for ch in 0..channels {
                samples[frame * channels + ch] = value;
            }
        }
        // Put an immediate crisp transient right at `lead_frames`.
        if lead_frames < total_frames {
            for ch in 0..channels {
                samples[lead_frames * channels + ch] = 0.9;
            }
        }
        samples
    }

    #[test]
    fn healthy_drum_hit_passes_with_zero_issues() {
        let pcm = drum_hit_pcm(80.0, 0.0, 2);
        let wav = encode_pcm16_wav(&pcm, RATE, 2).unwrap();
        assert!(wav.len() >= MIN_STABLE_BYTES);
        let report = inspect_wav_bytes("normal-hitnormal2.wav", &wav).unwrap();
        assert!(report.is_healthy(), "expected healthy, got {:?}", report.issues);
        assert!(report.duration_ms >= MIN_DURATION_MS);
        assert!(report.impact_ms.unwrap() <= MAX_IMPACT_DELAY_MS);
    }

    #[test]
    fn short_sample_triggers_too_short_and_auto_fix_pads_to_rc_and_stable_minimums() {
        // 10 ms mono @ 48 kHz = 480 frames = 960 data bytes + 44 header = 1004 bytes (< 25 ms AND < 1024 bytes).
        let pcm = drum_hit_pcm(10.0, 0.0, 1);
        let wav = encode_pcm16_wav(&pcm, RATE, 1).unwrap();
        assert!(wav.len() < MIN_STABLE_BYTES);

        let report = inspect_wav_bytes("short-clap.wav", &wav).unwrap();
        assert!(
            report
                .issues
                .iter()
                .any(|i| matches!(i, SampleHealthIssue::TooShort { .. })),
            "expected TooShort, got {:?}",
            report.issues
        );
        assert!(
            report
                .issues
                .iter()
                .any(|i| matches!(i, SampleHealthIssue::TooSmallForStable { .. })),
            "expected TooSmallForStable, got {:?}",
            report.issues
        );

        let fixed = auto_fix_wav_pcm(&pcm, RATE, 1).unwrap();
        assert!(!fixed.replaced_with_silent_wav);
        assert!(fixed.padded_ms > 0.0);
        assert!(fixed.wav.len() >= MIN_STABLE_BYTES);
        let after = inspect_wav_bytes("short-clap.wav", &fixed.wav).unwrap();
        assert!(after.is_healthy(), "fixed sample still has issues: {:?}", after.issues);
        assert!(after.duration_ms >= MIN_DURATION_MS);
    }

    #[test]
    fn leading_15ms_silence_triggers_late_impact_and_auto_fix_trims_without_clipping_transient() {
        let pcm = drum_hit_pcm(90.0, 15.0, 2);
        let wav = encode_pcm16_wav(&pcm, RATE, 2).unwrap();
        let report = inspect_wav_bytes("soft-hitclap2.wav", &wav).unwrap();
        assert!(
            report.issues.iter().any(
                |i| matches!(i, SampleHealthIssue::LateImpact { impact_ms } if *impact_ms >= 14.5)
            ),
            "expected LateImpact >= 14.5 ms, got {:?}",
            report.issues
        );
        let warnings = report.warnings();
        assert!(
            warnings[0].contains("soft-hitclap2.wav") && warnings[0].contains("leading silence"),
            "{warnings:?}"
        );

        let fixed = auto_fix_wav_pcm(&pcm, RATE, 2).unwrap();
        assert!(fixed.trimmed_ms >= 14.0 && fixed.trimmed_ms <= 15.0, "trimmed_ms={}", fixed.trimmed_ms);
        let after = inspect_wav_bytes("soft-hitclap2.wav", &fixed.wav).unwrap();
        assert!(after.is_healthy(), "fixed sample still has issues: {:?}", after.issues);
        assert!(after.impact_ms.unwrap() <= MAX_IMPACT_DELAY_MS);
        // Transient peak (~0.9) must not be clipped away.
        assert!(after.peak >= 0.85, "transient peak was clipped: {}", after.peak);
    }

    #[test]
    fn silent_bloated_wav_triggers_non_standard_silence_and_auto_fix_produces_44_byte_wav() {
        let silent = vec![0.0_f32; 48_000 * 2];
        let wav = encode_pcm16_wav(&silent, RATE, 2).unwrap();
        assert!(wav.len() > STANDARD_SILENT_WAV_BYTES);

        let report = inspect_wav_bytes("normal-hitnormal5.wav", &wav).unwrap();
        assert_eq!(
            report.issues,
            vec![SampleHealthIssue::NonStandardSilence { bytes: wav.len() }]
        );

        let fixed = auto_fix_wav_pcm(&silent, RATE, 2).unwrap();
        assert!(fixed.replaced_with_silent_wav);
        assert_eq!(fixed.wav.len(), STANDARD_SILENT_WAV_BYTES);
        let after = inspect_wav_bytes("normal-hitnormal5.wav", &fixed.wav).unwrap();
        assert!(after.is_healthy(), "44-byte blank WAV must be healthy, got {:?}", after.issues);
    }

    #[test]
    fn tiny_non_blank_wav_under_1024_bytes_triggers_too_small_for_stable() {
        // 8 kHz mono, 40 ms -> 320 frames -> 640 data bytes + 44 header = 684 bytes (>= 25 ms, < 1024 bytes).
        let rate = 8_000u32;
        let frames = 320usize;
        let mut pcm = vec![0.0_f32; frames];
        pcm[0] = 0.8;
        pcm[1] = -0.6;
        let wav = encode_pcm16_wav(&pcm, rate, 1).unwrap();
        assert!(wav.len() < MIN_STABLE_BYTES);

        let report = inspect_wav_bytes("tiny-hit.wav", &wav).unwrap();
        assert_eq!(
            report.issues,
            vec![SampleHealthIssue::TooSmallForStable { bytes: wav.len() }]
        );

        let fixed = auto_fix_wav_pcm(&pcm, rate, 1).unwrap();
        assert!(fixed.wav.len() >= MIN_STABLE_BYTES);
        let after = inspect_wav_bytes("tiny-hit.wav", &fixed.wav).unwrap();
        assert!(after.is_healthy(), "{:?}", after.issues);
    }

    #[test]
    fn inspect_file_handles_44_byte_silent_wav_on_disk() {
        let scratch = dawsu_test_support::Scratch::new("rc-silent-44");
        let path = scratch.path("normal-hitnormal2.wav");
        fs::write(&path, silent_wav_44(RATE, 2)).unwrap();
        let report = inspect_file(&path).unwrap();
        assert!(report.is_healthy(), "{:?}", report.issues);
        assert_eq!(report.bytes, STANDARD_SILENT_WAV_BYTES);
    }

    #[test]
    fn audit_mapset_samples_flags_unhealthy_samples_without_mutating_files_and_skips_song() {
        let scratch = dawsu_test_support::Scratch::new("rc-audit-mapset");
        let map = scratch.path("Artist - Song (Mapper) [Normal].osu");
        fs::write(
            &map,
            b"osu file format v14\n[General]\nAudioFilename: song.wav\nMode:0\n[Metadata]\nVersion:Normal\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n",
        )
        .unwrap();

        // Song audio with leading silence must be skipped by sample audit.
        let song_wav = encode_pcm16_wav(&drum_hit_pcm(100.0, 20.0, 2), RATE, 2).unwrap();
        fs::write(scratch.path("song.wav"), &song_wav).unwrap();

        // Healthy hitsound.
        let healthy_wav = encode_pcm16_wav(&drum_hit_pcm(80.0, 0.0, 2), RATE, 2).unwrap();
        fs::write(scratch.path("normal-hitnormal2.wav"), &healthy_wav).unwrap();

        // Unhealthy hitsound (leading 15 ms silence).
        let late_wav = encode_pcm16_wav(&drum_hit_pcm(80.0, 15.0, 2), RATE, 2).unwrap();
        let late_path = scratch.path("soft-hitclap2.wav");
        fs::write(&late_path, &late_wav).unwrap();

        let mut app = crate::Application::headless();
        app.document = Some(crate::Document::read(&map).unwrap());
        let reports = app.audit_mapset_samples().unwrap();
        assert_eq!(reports.len(), 1, "{reports:?}");
        assert_eq!(reports[0].name, "soft-hitclap2.wav");
        assert!(
            reports[0]
                .issues
                .iter()
                .any(|i| matches!(i, SampleHealthIssue::LateImpact { .. }))
        );
        // Existing files on disk are never mutated by audit.
        assert_eq!(fs::read(&late_path).unwrap(), late_wav);
    }

    #[test]
    fn generate_surfaces_rc_warnings_and_opt_in_auto_fix_repairs_staged_mixes() {
        let scratch = dawsu_test_support::Scratch::new("rc-generate-staged-mix");
        let map = scratch.path("Artist - Song (Mapper) [Hard].osu");
        fs::write(
            &map,
            b"osu file format v14\n[General]\nMode:0\n[Metadata]\nVersion:Hard\n[TimingPoints]\n0,500,4,1,0,100,1,0\n[HitObjects]\n256,192,1000,1,0,0:0:0:50:late1.wav\n300,192,1000,1,0,0:0:0:50:late2.wav\n",
        )
        .unwrap();
        let late = encode_pcm16_wav(&drum_hit_pcm(60.0, 15.0, 2), RATE, 2).unwrap();
        fs::write(scratch.path("late1.wav"), &late).unwrap();
        fs::write(scratch.path("late2.wav"), &late).unwrap();

        let doc = crate::Document::read(&map).unwrap();
        let source = crate::output::Source::from_document(&doc);

        // Default (auto_fix = false): surfaces non-blocking RC warning on staged mix.
        let plan_default = crate::output::generate_report_with_options(
            &source,
            Path::new(""),
            None,
            None,
            false,
            false,
            &|| false,
            &mut Vec::new(),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(plan_default.mixes.len(), 1);
        assert!(
            !plan_default.sample_warnings.is_empty(),
            "expected RC warning for late-impact staged mix"
        );
        let warning_lines = plan_default.sample_warning_lines();
        assert!(
            warning_lines
                .iter()
                .any(|line| line.contains("leading silence")),
            "{warning_lines:?}"
        );

        // Opt-in (auto_fix = true): trims leading silence so staged mix is healthy with 0 warnings.
        let plan_fixed = crate::output::generate_report_with_options(
            &source,
            Path::new(""),
            None,
            None,
            false,
            true,
            &|| false,
            &mut Vec::new(),
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(plan_fixed.mixes.len(), 1);
        assert!(
            plan_fixed.sample_warnings.is_empty(),
            "auto-fixed staged mix should be healthy, got {:?}",
            plan_fixed.sample_warnings
        );
    }
}
