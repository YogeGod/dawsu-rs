//! Pure-Rust acoustic & filename drum sample classifier for Drum Map auto-detection.
use dawsu_audio::AudioClip;
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::{
    Lane,
    drums::{DrumMap, DrumRow, lane_key},
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DrumRole {
    Kick,
    Snare,
    Clap,
    ClosedHat,
    OpenHat,
    HighTom,
    MidTom,
    FloorTom,
    Crash,
    Ride,
    Perc,
}

impl DrumRole {
    fn label(self) -> &'static str {
        match self {
            Self::Kick => "Kick",
            Self::Snare => "Snare",
            Self::Clap => "Clap",
            Self::ClosedHat => "Closed Hat",
            Self::OpenHat => "Open Hat",
            Self::HighTom => "High Tom",
            Self::MidTom => "Mid Tom",
            Self::FloorTom => "Floor Tom",
            Self::Crash => "Crash",
            Self::Ride => "Ride",
            Self::Perc => "Perc",
        }
    }

    fn midi_note(self) -> u8 {
        match self {
            Self::Kick => 36,
            Self::Snare => 38,
            Self::Clap => 39,
            Self::ClosedHat => 42,
            Self::OpenHat => 46,
            Self::HighTom => 50,
            Self::MidTom => 47,
            Self::FloorTom => 41,
            Self::Crash => 49,
            Self::Ride => 51,
            Self::Perc => 56,
        }
    }
}

/// Core kit slots included in an auto-detected Drum Map in Cubase kit order.
const CORE_SLOTS: [DrumRole; 10] = [
    DrumRole::Kick,
    DrumRole::Snare,
    DrumRole::Clap,
    DrumRole::ClosedHat,
    DrumRole::OpenHat,
    DrumRole::HighTom,
    DrumRole::MidTom,
    DrumRole::FloorTom,
    DrumRole::Crash,
    DrumRole::Ride,
];

#[derive(Debug, Clone, Copy)]
enum ClassifiedRole {
    Fixed(DrumRole),
    GenericTom { pitch_hz: f32 },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AcousticFeatures {
    pub(crate) decay_ms: f32,
    pub(crate) sub_low: f32,
    pub(crate) low_mid: f32,
    pub(crate) mid: f32,
    pub(crate) high_mid: f32,
    pub(crate) high: f32,
    pub(crate) centroid_hz: f32,
    pub(crate) peak_hz: f32,
    pub(crate) flatness: f32,
    pub(crate) zcr: f32,
    pub(crate) clap_peaks: usize,
    pub(crate) crest_factor: f32,
}

/// Automatically builds a kit `DrumMap` from `lanes` by inspecting custom filenames and
/// analyzing each lane's decoded PCM audio clip. Silent samples are left unassigned.
pub fn auto_detect_drum_map(
    lanes: &[Lane],
    clip_for: impl Fn(&SampleIdentity) -> Option<Arc<AudioClip>>,
) -> DrumMap {
    let mut fixed_buckets: Vec<(DrumRole, &Lane)> = Vec::new();
    let mut toms: Vec<(f32, &Lane)> = Vec::new();

    for lane in lanes {
        let clip = clip_for(&lane.identity);
        let features = clip.as_deref().and_then(extract_features);

        // If we have audio and it is effectively silent, leave it unassigned.
        if clip.is_some() && features.is_none() {
            continue;
        }

        if let Some(hint) = filename_hint(&lane.identity) {
            match hint {
                ClassifiedRole::Fixed(role) => fixed_buckets.push((role, lane)),
                ClassifiedRole::GenericTom { pitch_hz } => {
                    let resolved_pitch = features.map_or(pitch_hz, |f| f.peak_hz);
                    toms.push((resolved_pitch, lane));
                }
            }
            continue;
        }

        if let Some(feat) = features {
            match classify_audio(feat) {
                ClassifiedRole::Fixed(role) => fixed_buckets.push((role, lane)),
                ClassifiedRole::GenericTom { pitch_hz } => toms.push((pitch_hz, lane)),
            }
            continue;
        }

        if let Some(fallback) = standard_fallback_role(&lane.identity) {
            fixed_buckets.push((fallback, lane));
        }
    }

    // Sort toms from highest fundamental pitch to lowest so High/Mid/Floor Tom order matches.
    toms.sort_by(|a, b| b.0.total_cmp(&a.0));
    match toms.len() {
        0 => {}
        1 => fixed_buckets.push((DrumRole::MidTom, toms[0].1)),
        2 => {
            fixed_buckets.push((DrumRole::HighTom, toms[0].1));
            fixed_buckets.push((DrumRole::FloorTom, toms[1].1));
        }
        _ => {
            for (idx, &(_, lane)) in toms.iter().enumerate() {
                let role = if idx == 0 {
                    DrumRole::HighTom
                } else if idx + 1 == toms.len() {
                    DrumRole::FloorTom
                } else {
                    DrumRole::MidTom
                };
                fixed_buckets.push((role, lane));
            }
        }
    }

    let mut rows = Vec::new();
    let mut row_num = 1u64;

    let roles_in_order = CORE_SLOTS
        .into_iter()
        .chain(std::iter::once(DrumRole::Perc));

    for role in roles_in_order {
        let matched: Vec<&Lane> = fixed_buckets
            .iter()
            .filter_map(|&(r, lane)| (r == role).then_some(lane))
            .collect();
        if matched.is_empty() {
            if role != DrumRole::Perc {
                rows.push(DrumRow {
                    id: row_guid(row_num),
                    name: role.label().into(),
                    lane: None,
                    slot: None,
                    midi_note: Some(role.midi_note()),
                    choke_group: None,
                });
                row_num += 1;
            }
            continue;
        }

        for (i, lane) in matched.into_iter().enumerate() {
            let name = if i == 0 {
                role.label().to_owned()
            } else {
                format!("{} {}", role.label(), i + 1)
            };
            rows.push(DrumRow {
                id: row_guid(row_num),
                name,
                lane: Some(lane_key(lane)),
                slot: None,
                midi_note: Some(role.midi_note()),
                choke_group: None,
            });
            row_num += 1;
        }
    }

    DrumMap {
        id: "d0000000-0000-4000-8000-000000000001".into(),
        name: "Auto-detected kit".into(),
        rows,
        audio: Vec::new(),
    }
}

fn row_guid(n: u64) -> String {
    format!("d0000001-0000-4000-8000-{n:012x}")
}

fn filename_hint(identity: &SampleIdentity) -> Option<ClassifiedRole> {
    let raw = match identity {
        SampleIdentity::File(name) => name.as_str(),
        SampleIdentity::Standard { .. } => return None,
    };
    filename_hint_for_name(raw, true)
}

fn filename_hint_for_name(raw: &str, ignore_generic_osu: bool) -> Option<ClassifiedRole> {
    let stem = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .split('.')
        .next()
        .unwrap_or(raw)
        .to_ascii_lowercase();

    if stem.starts_with("dawsu-mix") || (ignore_generic_osu && is_generic_osu_sample(&stem)) {
        return None;
    }

    let tokens: Vec<&str> = stem
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let has_token = |word: &str| tokens.contains(&word);

    if stem.contains("kick") || stem.contains("bassdrum") || has_token("bd") {
        return Some(ClassifiedRole::Fixed(DrumRole::Kick));
    }
    if stem.contains("clap") {
        return Some(ClassifiedRole::Fixed(DrumRole::Clap));
    }
    if stem.contains("snare") || stem.contains("sidestick") || has_token("snr") || has_token("sd") || has_token("rim") {
        return Some(ClassifiedRole::Fixed(DrumRole::Snare));
    }
    if (stem.contains("open") && stem.contains("hat"))
        || stem.contains("openhat")
        || has_token("oh")
    {
        return Some(ClassifiedRole::Fixed(DrumRole::OpenHat));
    }
    if stem.contains("hihat")
        || stem.contains("hi-hat")
        || stem.contains("hi_hat")
        || stem.contains("closedhat")
        || has_token("hat")
        || has_token("hh")
        || has_token("chh")
        || has_token("pedal")
    {
        return Some(ClassifiedRole::Fixed(DrumRole::ClosedHat));
    }
    if stem.contains("floortom") || (stem.contains("floor") && stem.contains("tom")) || has_token("ftom") {
        return Some(ClassifiedRole::Fixed(DrumRole::FloorTom));
    }
    if stem.contains("midtom") || (stem.contains("mid") && stem.contains("tom")) || has_token("mtom") {
        return Some(ClassifiedRole::Fixed(DrumRole::MidTom));
    }
    if stem.contains("hightom") || stem.contains("hitom") || (stem.contains("high") && stem.contains("tom")) || stem.contains("racktom") {
        return Some(ClassifiedRole::Fixed(DrumRole::HighTom));
    }
    if has_token("tom") || stem.contains("tom") {
        return Some(ClassifiedRole::GenericTom { pitch_hz: 160.0 });
    }
    if stem.contains("crash") || stem.contains("china") || stem.contains("splash") || stem.contains("cym") {
        return Some(ClassifiedRole::Fixed(DrumRole::Crash));
    }
    if stem.contains("ride") || stem.contains("bell") {
        return Some(ClassifiedRole::Fixed(DrumRole::Ride));
    }
    if stem.contains("perc")
        || stem.contains("shaker")
        || stem.contains("tamb")
        || stem.contains("cowbell")
        || stem.contains("woodblock")
        || stem.contains("clave")
        || stem.contains("bongo")
        || stem.contains("conga")
    {
        return Some(ClassifiedRole::Fixed(DrumRole::Perc));
    }
    None
}

fn is_generic_osu_sample(stem: &str) -> bool {
    let rest = stem
        .strip_prefix("normal-")
        .or_else(|| stem.strip_prefix("soft-"))
        .or_else(|| stem.strip_prefix("drum-"));
    let Some(rest) = rest else {
        return false;
    };
    let base = rest.trim_end_matches(|c: char| c.is_ascii_digit());
    matches!(
        base,
        "hitnormal" | "hitclap" | "hitwhistle" | "hitfinish" | "sliderslide" | "sliderwhistle" | "slidertick"
    )
}

fn standard_fallback_role(identity: &SampleIdentity) -> Option<DrumRole> {
    use dawsu_foundation::{Hitsound, SampleSet, Sound};
    match identity {
        SampleIdentity::Standard {
            set,
            sound: Sound::Hit(sound),
            ..
        } => Some(match (set, sound) {
            (SampleSet::Drum | SampleSet::Normal, Hitsound::Normal) => DrumRole::Kick,
            (SampleSet::Soft, Hitsound::Normal) => DrumRole::ClosedHat,
            (SampleSet::Soft | SampleSet::Normal, Hitsound::Clap) => DrumRole::Snare,
            (SampleSet::Drum, Hitsound::Clap) => DrumRole::MidTom,
            (SampleSet::Drum, Hitsound::Finish) => DrumRole::FloorTom,
            (SampleSet::Soft | SampleSet::Normal, Hitsound::Finish) => DrumRole::Crash,
            (SampleSet::Drum, Hitsound::Whistle) => DrumRole::Ride,
            (SampleSet::Soft | SampleSet::Normal, Hitsound::Whistle) => DrumRole::Perc,
        }),
        SampleIdentity::File(_) => Some(DrumRole::Perc),
        _ => None,
    }
}

pub(crate) fn extract_features(clip: &AudioClip) -> Option<AcousticFeatures> {
    let channels = clip.channels.max(1);
    let sample_rate = clip.sample_rate.max(8000) as f32;
    let total_frames = clip.samples.len() / channels;
    if total_frames == 0 {
        return None;
    }

    let max_analyze_frames = total_frames.min((sample_rate * 2.0) as usize);
    let mut mono = Vec::with_capacity(max_analyze_frames);
    let mut peak = 0.0_f32;
    for f in 0..max_analyze_frames {
        let mut sum = 0.0_f32;
        for c in 0..channels {
            sum += clip.samples[f * channels + c];
        }
        let s = sum / channels as f32;
        peak = peak.max(s.abs());
        mono.push(s);
    }
    if peak < 1.0e-4 {
        return None;
    }

    let onset = mono
        .iter()
        .position(|s| s.abs() >= peak * 0.1)
        .unwrap_or(0);

    let crest_end = mono.len().min(onset + (sample_rate * 0.250) as usize);
    let crest_slice = &mono[onset..crest_end.max(onset + 1).min(mono.len())];
    let crest_mean_sq =
        crest_slice.iter().map(|s| s * s).sum::<f32>() / crest_slice.len().max(1) as f32;
    let crest_factor = if crest_mean_sq > 1.0e-12 {
        (peak / crest_mean_sq.sqrt()).clamp(1.0, 30.0)
    } else {
        1.0
    };

    // Measure effective -20 dB decay time via 10 ms hop RMS envelope.
    let win_10ms = ((sample_rate * 0.010) as usize).max(16);
    let hop_10ms = (win_10ms / 2).max(8);
    let mut env = Vec::new();
    let mut pos = onset;
    while pos < mono.len() {
        let end = (pos + win_10ms).min(mono.len());
        let mean_sq = mono[pos..end].iter().map(|s| s * s).sum::<f32>() / (end - pos) as f32;
        env.push(mean_sq.sqrt());
        pos += hop_10ms;
    }
    let (peak_env_idx, peak_env) = env
        .iter()
        .copied()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((0, peak));
    let tail_thresh = peak_env * 0.1;
    let decay_steps = env
        .iter()
        .enumerate()
        .skip(peak_env_idx)
        .find(|&(_, &v)| v < tail_thresh)
        .map(|(i, _)| i.saturating_sub(peak_env_idx))
        .unwrap_or_else(|| env.len().saturating_sub(peak_env_idx));
    let decay_ms = (decay_steps * hop_10ms) as f32 * 1000.0 / sample_rate;

    // Count distinct transient bursts in first 45 ms (distinguishes Claps from single Snares).
    let win_3ms = ((sample_rate * 0.003) as usize).max(8);
    let max_burst_frames = mono.len().min(onset + (sample_rate * 0.045) as usize);
    let mut burst_env = Vec::new();
    let mut bpos = onset;
    while bpos < max_burst_frames {
        let end = (bpos + win_3ms).min(max_burst_frames);
        let rms = (mono[bpos..end].iter().map(|s| s * s).sum::<f32>() / (end - bpos) as f32).sqrt();
        burst_env.push(rms);
        bpos += (win_3ms / 2).max(4);
    }
    let burst_max = burst_env.iter().copied().fold(0.0_f32, f32::max);
    let mut clap_peaks = 0usize;
    if burst_max > 0.0 {
        let min_gap = ((sample_rate * 0.005) as usize / ((win_3ms / 2).max(4))).max(1);
        let mut last_peak_idx: Option<usize> = None;
        let mut min_since_peak = f32::MAX;
        for i in 1..burst_env.len().saturating_sub(1) {
            let v = burst_env[i];
            min_since_peak = min_since_peak.min(v);
            if v >= burst_max * 0.30 && v >= burst_env[i - 1] && v > burst_env[i + 1] {
                let separated = match last_peak_idx {
                    None => true,
                    Some(prev) => i - prev >= min_gap && min_since_peak <= v * 0.72,
                };
                if separated {
                    clap_peaks += 1;
                    last_peak_idx = Some(i);
                    min_since_peak = v;
                }
            }
        }
    }

    // Zero-crossing rate over the first 50 ms from onset.
    let zcr_end = mono.len().min(onset + (sample_rate * 0.050) as usize);
    let zcr = if zcr_end > onset + 1 {
        let crossings = mono[onset..zcr_end]
            .windows(2)
            .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
            .count();
        crossings as f32 / (zcr_end - onset) as f32
    } else {
        0.0
    };

    // Average power spectrum across up to 4 overlapping 2048-point Hann windows.
    const N: usize = 2048;
    const HOP: usize = 512;
    let bins = N / 2;
    let mut power = vec![0.0_f32; bins];
    let mut fft_buf = vec![(0.0_f32, 0.0_f32); N];
    let mut windows = 0usize;

    for w in 0..4 {
        let start = onset + w * HOP;
        if w > 0 && start >= mono.len() {
            break;
        }
        windows += 1;
        for (i, slot) in fft_buf.iter_mut().enumerate().take(N) {
            let s = mono.get(start + i).copied().unwrap_or(0.0);
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (N - 1) as f32).cos());
            *slot = (s * hann, 0.0);
        }
        crate::waveform::fft_radix2_inplace(&mut fft_buf);
        for (bin, &(re, im)) in fft_buf[..bins].iter().enumerate() {
            power[bin] += re * re + im * im;
        }
    }
    if windows > 1 {
        let inv = 1.0 / windows as f32;
        for p in &mut power {
            *p *= inv;
        }
    }

    let hz_per_bin = sample_rate / N as f32;
    let (mut sub_e, mut low_mid_e, mut mid_e, mut high_mid_e, mut high_e) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
    let (mut total_e, mut weighted_freq) = (0.0_f32, 0.0_f32);
    let (mut peak_bin_e, mut peak_hz) = (0.0_f32, 120.0_f32);
    let (mut flat_sum_log, mut flat_sum, mut flat_count) = (0.0_f64, 0.0_f64, 0usize);

    for (bin, &p) in power.iter().enumerate() {
        let hz = bin as f32 * hz_per_bin;
        if !(20.0..=20000.0).contains(&hz) {
            continue;
        }
        total_e += p;
        weighted_freq += hz * p;
        if (20.0..92.0).contains(&hz) {
            sub_e += p;
        } else if (92.0..400.0).contains(&hz) {
            low_mid_e += p;
        } else if (400.0..2000.0).contains(&hz) {
            mid_e += p;
        } else if (2000.0..6000.0).contains(&hz) {
            high_mid_e += p;
        } else {
            high_e += p;
        }
        if (40.0..=1000.0).contains(&hz) && p > peak_bin_e {
            peak_bin_e = p;
            peak_hz = hz;
        }
        if (500.0..=12000.0).contains(&hz) {
            let p64 = f64::from(p) + 1.0e-12;
            flat_sum_log += p64.ln();
            flat_sum += p64;
            flat_count += 1;
        }
    }

    if total_e <= 1.0e-12 {
        return None;
    }

    let flatness = if flat_count > 0 && flat_sum > 0.0 {
        let geo = (flat_sum_log / flat_count as f64).exp();
        let arith = flat_sum / flat_count as f64;
        (geo / arith) as f32
    } else {
        0.0
    };

    Some(AcousticFeatures {
        decay_ms,
        sub_low: sub_e / total_e,
        low_mid: low_mid_e / total_e,
        mid: mid_e / total_e,
        high_mid: high_mid_e / total_e,
        high: high_e / total_e,
        centroid_hz: weighted_freq / total_e,
        peak_hz,
        flatness,
        zcr,
        clap_peaks,
        crest_factor,
    })
}

fn classify_audio(f: AcousticFeatures) -> ClassifiedRole {
    if f.sub_low >= 0.40 || (f.sub_low + f.low_mid >= 0.65 && f.centroid_hz < 500.0 && f.peak_hz < 95.0) {
        return ClassifiedRole::Fixed(DrumRole::Kick);
    }

    if (f.low_mid >= 0.30 || (f.sub_low + f.low_mid >= 0.50 && (75.0..=420.0).contains(&f.peak_hz)))
        && f.flatness < 0.25
        && (f.high_mid + f.high) < 0.28
    {
        return ClassifiedRole::GenericTom { pitch_hz: f.peak_hz };
    }

    // Metal / cymbals have almost no low-mid drum shell punch.
    if (f.sub_low + f.low_mid) < 0.08 && (f.high_mid + f.high >= 0.45 || f.centroid_hz >= 3800.0) {
        if f.clap_peaks >= 2 && f.centroid_hz < 7500.0 && f.decay_ms < 320.0 {
            return ClassifiedRole::Fixed(DrumRole::Clap);
        }
        if f.decay_ms < 190.0 {
            return ClassifiedRole::Fixed(DrumRole::ClosedHat);
        }
        if f.flatness < 0.14 && f.decay_ms >= 250.0 {
            return ClassifiedRole::Fixed(DrumRole::Ride);
        }
        if f.decay_ms < 520.0 && f.high >= 0.22 {
            return ClassifiedRole::Fixed(DrumRole::OpenHat);
        }
        return ClassifiedRole::Fixed(DrumRole::Crash);
    }

    if (f.mid + f.high_mid >= 0.35 || (f.low_mid >= 0.08 && (f.high_mid + f.high) >= 0.25)) && f.sub_low < 0.25 {
        if (f.sub_low + f.low_mid) < 0.12 && f.clap_peaks >= 2 && f.centroid_hz > 1100.0 && f.decay_ms < 320.0 {
            return ClassifiedRole::Fixed(DrumRole::Clap);
        }
        if f.flatness >= 0.12 || f.zcr >= 0.08 || (f.low_mid >= 0.08 && (f.high_mid >= 0.10 || f.high >= 0.15)) {
            return ClassifiedRole::Fixed(DrumRole::Snare);
        }
        return ClassifiedRole::Fixed(DrumRole::Perc);
    }

    if f.peak_hz < 140.0 && f.centroid_hz < 600.0 {
        ClassifiedRole::Fixed(DrumRole::Kick)
    } else if f.centroid_hz >= 3500.0 {
        ClassifiedRole::Fixed(DrumRole::ClosedHat)
    } else {
        ClassifiedRole::Fixed(DrumRole::Perc)
    }
}

pub(crate) fn classify_sample_tag(filename: &str, features: Option<AcousticFeatures>) -> &'static str {
    let role = filename_hint_for_name(filename, false).or_else(|| features.map(classify_audio));
    match role {
        Some(ClassifiedRole::Fixed(DrumRole::Kick)) => "Kick",
        Some(ClassifiedRole::Fixed(DrumRole::Snare)) => "Snare",
        Some(ClassifiedRole::Fixed(DrumRole::Clap)) => "Clap",
        Some(ClassifiedRole::Fixed(DrumRole::ClosedHat | DrumRole::OpenHat)) => "Hat",
        Some(ClassifiedRole::Fixed(DrumRole::HighTom | DrumRole::MidTom | DrumRole::FloorTom)
            | ClassifiedRole::GenericTom { .. }) => "Tom",
        Some(ClassifiedRole::Fixed(DrumRole::Crash | DrumRole::Ride)) => "Cymbal",
        Some(ClassifiedRole::Fixed(DrumRole::Perc)) => "Perc",
        None => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_foundation::{CustomIndex, Hitsound, SampleSet, Sound};
    use dawsu_hitsounding::LaneSource;
    use std::collections::HashMap;

    const SR: u32 = 44_100;

    struct Rng(u64);
    impl Rng {
        fn next_f32(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 32) as i32) as f32 / i32::MAX as f32
        }
    }

    fn make_clip(len_ms: f32, mut synth: impl FnMut(f32) -> f32) -> Arc<AudioClip> {
        let frames = (SR as f32 * len_ms / 1000.0) as usize;
        let samples = (0..frames)
            .map(|i| synth(i as f32 / SR as f32))
            .collect();
        Arc::new(AudioClip { samples, channels: 1, sample_rate: SR })
    }

    fn kick_clip() -> Arc<AudioClip> {
        let mut phase = 0.0_f32;
        make_clip(250.0, |t| {
            let f = 55.0 + 110.0 * (-t / 0.030).exp();
            phase += 2.0 * std::f32::consts::PI * f / SR as f32;
            phase.sin() * (-t / 0.065).exp()
        })
    }

    fn tom_clip(freq_hz: f32) -> Arc<AudioClip> {
        make_clip(300.0, |t| {
            (2.0 * std::f32::consts::PI * freq_hz * t).sin() * (-t / 0.090).exp()
        })
    }

    fn snare_clip() -> Arc<AudioClip> {
        let mut rng = Rng(1);
        let mut prev = 0.0_f32;
        make_clip(300.0, |t| {
            let n = rng.next_f32();
            let hp = n - 0.6 * prev;
            prev = n;
            let body = (2.0 * std::f32::consts::PI * 200.0 * t).sin() * (-t / 0.050).exp() * 0.5;
            let wires = hp * (-t / 0.080).exp() * 0.55;
            body + wires
        })
    }

    fn clap_clip() -> Arc<AudioClip> {
        let mut rng = Rng(2);
        let mut lp1 = 0.0_f32;
        let mut prev_lp = 0.0_f32;
        let mut hp = 0.0_f32;
        make_clip(220.0, |t| {
            let n = rng.next_f32();
            lp1 += 0.25 * (n - lp1);
            hp = 0.90 * (hp + lp1 - prev_lp);
            prev_lp = lp1;
            let burst = |center: f32| (-(t - center).abs() / 0.0018).exp();
            let env = (burst(0.003) + burst(0.015) + burst(0.027)).min(1.0) * (-t / 0.045).exp();
            hp * env
        })
    }

    fn hat_clip(tau_sec: f32, len_ms: f32) -> Arc<AudioClip> {
        let mut rng = Rng(3);
        let mut prev = 0.0_f32;
        let mut prev2 = 0.0_f32;
        make_clip(len_ms, |t| {
            let n = rng.next_f32();
            // Strong 2-stage differentiator pushes energy > 6 kHz.
            let hp1 = n - 0.9 * prev;
            let hp2 = hp1 - 0.9 * prev2;
            prev = n;
            prev2 = hp1;
            hp2 * 0.35 * (-t / tau_sec).exp()
        })
    }

    fn ride_clip() -> Arc<AudioClip> {
        let mut rng = Rng(4);
        make_clip(800.0, |t| {
            let bell = (2.0 * std::f32::consts::PI * 3200.0 * t).sin() * 0.65
                + (2.0 * std::f32::consts::PI * 4850.0 * t).sin() * 0.45;
            let wash = rng.next_f32() * 0.06;
            (bell + wash) * (-t / 0.300).exp()
        })
    }

    fn silent_clip() -> Arc<AudioClip> {
        make_clip(100.0, |_| 0.0)
    }

    fn file_lane(name: &str) -> Lane {
        Lane {
            identity: SampleIdentity::File(name.into()),
            source: LaneSource::File(name.into()),
        }
    }

    fn std_lane(set: SampleSet, sound: Hitsound) -> Lane {
        Lane {
            identity: SampleIdentity::Standard {
                set,
                sound: Sound::Hit(sound),
                index: CustomIndex::new(0),
            },
            source: LaneSource::Skin,
        }
    }

    #[test]
    fn recognizes_misleading_osu_sample_names_by_audio_and_sorts_toms_and_duplicates() {
        let lanes = vec![
            // Misleading osu! names: whistle contains a kick, normal contains a snare!
            file_lane("soft-hitwhistle.wav"),
            file_lane("drum-hitnormal.wav"),
            file_lane("soft-hitnormal.wav"),
            file_lane("drum-hitclap.wav"),
            file_lane("drum-hitfinish.wav"),
            file_lane("normal-hitclap.wav"),
            file_lane("normal-hitwhistle.wav"),
            file_lane("normal-hitfinish.wav"),
            file_lane("soft-hitfinish.wav"),
            file_lane("soft-hitclap.wav"),
            // Custom filename overrides + silent file that must stay unassigned.
            file_lane("custom_subkick_01.wav"),
            file_lane("unused-silence.wav"),
        ];

        let mut clips: HashMap<SampleIdentity, Arc<AudioClip>> = HashMap::new();
        clips.insert(SampleIdentity::File("soft-hitwhistle.wav".into()), kick_clip());
        clips.insert(SampleIdentity::File("drum-hitnormal.wav".into()), snare_clip());
        clips.insert(SampleIdentity::File("soft-hitnormal.wav".into()), clap_clip());
        clips.insert(SampleIdentity::File("drum-hitclap.wav".into()), tom_clip(100.0));
        clips.insert(SampleIdentity::File("drum-hitfinish.wav".into()), tom_clip(260.0));
        clips.insert(SampleIdentity::File("normal-hitclap.wav".into()), tom_clip(175.0));
        clips.insert(SampleIdentity::File("normal-hitwhistle.wav".into()), hat_clip(0.025, 140.0));
        clips.insert(SampleIdentity::File("normal-hitfinish.wav".into()), hat_clip(0.160, 420.0));
        clips.insert(SampleIdentity::File("soft-hitfinish.wav".into()), hat_clip(0.450, 1100.0));
        clips.insert(SampleIdentity::File("soft-hitclap.wav".into()), ride_clip());
        clips.insert(SampleIdentity::File("custom_subkick_01.wav".into()), tom_clip(190.0));
        clips.insert(SampleIdentity::File("unused-silence.wav".into()), silent_clip());

        let kit = auto_detect_drum_map(&lanes, |id| clips.get(id).cloned());
        kit.validate().expect("auto-detected kit must pass DrumMap::validate");

        let find = |lane_file: &str| {
            kit.rows
                .iter()
                .find(|r| r.lane.as_deref() == Some(&format!("BeatmapFile|{lane_file}")))
                .map(|r| r.name.as_str())
        };

        assert_eq!(find("soft-hitwhistle.wav"), Some("Kick"));
        assert_eq!(find("custom_subkick_01.wav"), Some("Kick 2"));
        assert_eq!(find("drum-hitnormal.wav"), Some("Snare"));
        assert_eq!(find("soft-hitnormal.wav"), Some("Clap"));
        assert_eq!(find("normal-hitwhistle.wav"), Some("Closed Hat"));
        assert_eq!(find("normal-hitfinish.wav"), Some("Open Hat"));
        assert_eq!(find("soft-hitfinish.wav"), Some("Crash"));
        assert_eq!(find("soft-hitclap.wav"), Some("Ride"));
        assert_eq!(find("drum-hitfinish.wav"), Some("High Tom"));
        assert_eq!(find("normal-hitclap.wav"), Some("Mid Tom"));
        assert_eq!(find("drum-hitclap.wav"), Some("Floor Tom"));
        assert_eq!(find("unused-silence.wav"), None, "silent samples must not be assigned");
    }

    #[test]
    fn falls_back_cleanly_when_clips_are_not_decoded_yet() {
        let lanes = vec![
            std_lane(SampleSet::Drum, Hitsound::Normal),
            std_lane(SampleSet::Soft, Hitsound::Clap),
        ];
        let kit = auto_detect_drum_map(&lanes, |_| None);
        kit.validate().unwrap();
        assert_eq!(
            kit.rows.iter().find(|r| r.name == "Kick").and_then(|r| r.lane.as_deref()),
            Some("Skin|drum-hitnormal")
        );
        assert_eq!(
            kit.rows.iter().find(|r| r.name == "Snare").and_then(|r| r.lane.as_deref()),
            Some("Skin|soft-hitclap")
        );
        // Unused standard slots stay present as unassigned rows for Drum Map Setup.
        assert!(kit.rows.iter().any(|r| r.name == "Crash" && r.lane.is_none()));
    }
}
