//! Headless decoded audio and map timing provider abstractions for `dawsu-graph-audio`.

use dawsu_foundation::digest::sha256_hex;
use dawsu_graph::TimingGrid;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Planar decoded PCM audio supplied to source nodes by `dawsu-application` or tests.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    pub channels: Vec<Vec<f32>>,
    pub sample_rate: u32,
    pub offset_ms: f64,
    pub content_hash: String,
    pub decoder_id: String,
}

impl DecodedAudio {
    /// Constructs a mono `DecodedAudio` and computes its deterministic SHA-256 fingerprint.
    pub fn from_mono(samples: Vec<f32>, sample_rate: u32, offset_ms: f64) -> Self {
        let mut bytes = Vec::with_capacity(samples.len() * 4 + 4);
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        for &s in &samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let content_hash = sha256_hex(&bytes);
        Self {
            channels: vec![samples],
            sample_rate,
            offset_ms,
            content_hash,
            decoder_id: "pcm-f32-v1".to_owned(),
        }
    }

    /// Constructs a multi-channel `DecodedAudio` and computes its deterministic SHA-256 fingerprint.
    pub fn from_planar(channels: Vec<Vec<f32>>, sample_rate: u32, offset_ms: f64) -> Self {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(channels.len() as u32).to_le_bytes());
        for ch in &channels {
            bytes.extend_from_slice(&(ch.len() as u64).to_le_bytes());
            for &s in ch {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
        }
        let content_hash = sha256_hex(&bytes);
        Self {
            channels,
            sample_rate,
            offset_ms,
            content_hash,
            decoder_id: "pcm-f32-v1".to_owned(),
        }
    }

    pub fn frame_count(&self) -> usize {
        self.channels.iter().map(Vec::len).min().unwrap_or(0)
    }

    /// Peak absolute amplitude across the full source (averaged across channels) for `normaliseOver = "song"`.
    pub fn full_average_peak(&self) -> f32 {
        let frames = self.frame_count();
        if frames == 0 || self.channels.is_empty() {
            return 0.0;
        }
        match self.channels.len() {
            1 => self.channels[0][..frames]
                .iter()
                .fold(0f32, |p, &v| p.max(v.abs())),
            2 => self.channels[0][..frames]
                .iter()
                .zip(&self.channels[1][..frames])
                .fold(0f32, |p, (&l, &r)| p.max(((l + r) * 0.5).abs())),
            n => (0..frames).fold(0f32, |p, i| {
                let avg = self.channels.iter().map(|ch| ch[i]).sum::<f32>() / n as f32;
                p.max(avg.abs())
            }),
        }
    }

    pub fn duration_ms(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frame_count() as f64 * 1000.0 / f64::from(self.sample_rate)
        }
    }
}

/// Supplies decoded song and stem audio without coupling `dawsu-graph-audio` to `dawsu-audio`.
pub trait AudioProvider: Send + Sync {
    fn song_audio(&self) -> Result<Arc<DecodedAudio>, String>;
    fn file_audio(&self, path: &str) -> Result<Arc<DecodedAudio>, String>;
}

/// Supplies the active map's red timing lines without coupling to `dawsu-hitsounding`.
pub trait TimingProvider: Send + Sync {
    fn timing_grid(&self) -> Arc<TimingGrid>;
}

/// Simple in-memory provider bundle for headless evaluation and tests.
#[derive(Clone)]
pub struct StaticProviders {
    pub song: Result<Arc<DecodedAudio>, String>,
    pub files: BTreeMap<String, Result<Arc<DecodedAudio>, String>>,
    pub timing: Arc<TimingGrid>,
}

impl Default for StaticProviders {
    fn default() -> Self {
        Self {
            song: Err("no song audio configured".to_owned()),
            files: BTreeMap::new(),
            timing: Arc::new(TimingGrid::default()),
        }
    }
}

impl StaticProviders {
    pub fn with_song(song: Arc<DecodedAudio>, timing: Arc<TimingGrid>) -> Self {
        Self {
            song: Ok(song),
            files: BTreeMap::new(),
            timing,
        }
    }

    pub fn missing_song(message: impl Into<String>, timing: Arc<TimingGrid>) -> Self {
        Self {
            song: Err(message.into()),
            files: BTreeMap::new(),
            timing,
        }
    }
}

impl AudioProvider for StaticProviders {
    fn song_audio(&self) -> Result<Arc<DecodedAudio>, String> {
        self.song.clone()
    }

    fn file_audio(&self, path: &str) -> Result<Arc<DecodedAudio>, String> {
        self.files
            .get(path)
            .cloned()
            .unwrap_or_else(|| Err(format!("audio file '{path}' was not found")))
    }
}

impl TimingProvider for StaticProviders {
    fn timing_grid(&self) -> Arc<TimingGrid> {
        self.timing.clone()
    }
}
