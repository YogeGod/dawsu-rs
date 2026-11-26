//! Audio decoding and playback; no editor or map types.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

pub mod denormal;
#[cfg(windows)]
mod directsound;
pub mod dll;
#[cfg(windows)]
mod mme;
pub mod ring;
mod rt_thread;
#[cfg(windows)]
mod wasapi_low_latency;

pub use dll::{AudioClockDll, ClockSnapshot, ClockSnapshotSlot, monotonic_rel_ns};
pub use ring::{CachePadded, SpscConsumer, SpscProducer, spsc_ring};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputBackend {
    #[default]
    System,
    Mme,
    DirectSound,
    Asio,
}

impl OutputBackend {
    pub fn available() -> &'static [Self] {
        #[cfg(all(windows, feature = "asio-local"))]
        return &[Self::System, Self::DirectSound, Self::Mme, Self::Asio];
        #[cfg(all(windows, not(feature = "asio-local")))]
        return &[Self::System, Self::DirectSound, Self::Mme];
        #[cfg(not(windows))]
        return &[Self::System];
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System if cfg!(windows) => "WASAPI",
            Self::System => "System",
            Self::Mme => "MME",
            Self::DirectSound => "DirectSound",
            Self::Asio => "ASIO",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
}

#[cfg(all(windows, feature = "asio-local"))]
fn registered_asio_devices() -> Vec<OutputDevice> {
    // Iterating CPAL's ASIO devices loads each driver to query capabilities. Settings only
    // needs registered names. CPAL uses the same driver name as its backend device ID.
    asio_sys::Asio::new()
        .driver_names()
        .into_iter()
        .map(|name| {
            let id = cpal::DeviceId::new(cpal::HostId::Asio, &name).to_string();
            OutputDevice { id, name }
        })
        .collect()
}

#[cfg(windows)]
const NATIVE_SAMPLE_RATE: u32 = 48_000;
/// Shown when the output device's sample format has no player here; cpal's names for formats
/// (such as `I24`) mean nothing to a mapper, so the message leaves the format out.
const UNSUPPORTED_SAMPLE_FORMAT: &str = "This audio device uses a sample format dawsu! cannot play. Choose another device.";

#[cfg(windows)]
struct NativeStream {
    name: String,
    buffer_ms: u32,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl Drop for NativeStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum OutputStream {
    System { _stream: cpal::Stream },
    #[cfg(windows)]
    Native { _stream: NativeStream },
}

#[derive(Debug, Clone)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
}
impl AudioClip {
    /// The loudest frame of each of `bars` equal slices, over every channel, scaled so the
    /// loudest bar is 1: the outline the Browser draws under a sample's name.
    pub fn peaks(&self, bars: usize) -> Vec<f32> {
        let channels = self.channels.max(1);
        let frames = self.samples.len() / channels;
        let mut peaks: Vec<f32> = (0..bars)
            .map(|bar| {
                let (from, to) = (bar * frames / bars, (bar + 1) * frames / bars);
                self.samples[from * channels..to * channels]
                    .iter()
                    .fold(0.0f32, |loudest, sample| loudest.max(sample.abs()))
            })
            .collect();
        let loudest = peaks.iter().copied().fold(0.0, f32::max);
        if loudest > 0.0 {
            peaks.iter_mut().for_each(|peak| *peak /= loudest);
        }
        peaks
    }
    pub fn duration_ms(&self) -> f64 {
        self.samples.len() as f64 * 1000.0
            / self.channels.max(1) as f64
            / self.sample_rate.max(1) as f64
    }
    fn sample(&self, frame: f64, channel: usize) -> f32 {
        if frame < 0.0 || !frame.is_finite() {
            return 0.0;
        }
        let at = frame.floor() as usize;
        let part = frame.fract() as f32;
        let channels = self.channels.max(1);
        let ch = channel.min(channels - 1);
        let a = self
            .samples
            .get(at.saturating_mul(channels).saturating_add(ch))
            .copied()
            .unwrap_or(0.0);
        let b = self
            .samples
            .get(
                at.saturating_add(1)
                    .saturating_mul(channels)
                    .saturating_add(ch),
            )
            .copied()
            .unwrap_or(a);
        a + (b - a) * part
    }
}
/// Maximum number of samples pre-reserved from container metadata so a
/// corrupted header frame count cannot trigger an OOM panic.
const MAX_DECODE_PRE_RESERVE_SAMPLES: usize = 120_000_000;

fn decode_pre_reserve_samples(num_frames: Option<u64>, channels: usize) -> usize {
    num_frames
        .map(|frames| {
            usize::try_from(frames)
                .unwrap_or(usize::MAX)
                .saturating_mul(channels)
                .min(MAX_DECODE_PRE_RESERVE_SAMPLES)
        })
        .unwrap_or(0)
}

pub fn decode(path: &Path) -> Result<Arc<AudioClip>, String> {
    decode_with_cancel(path, &|| false)
}
pub fn decode_bytes(bytes: Vec<u8>, extension: Option<&str>) -> Result<Arc<AudioClip>, String> {
    decode_source(Box::new(std::io::Cursor::new(bytes)), extension, "memory", &|| false, None)
}
pub fn decode_prefix(path: &Path, max_frames: usize) -> Result<Arc<AudioClip>, String> {
    let source = File::open(path).map_err(|e| format!("Opening {}: {e}", path.display()))?;
    decode_source(
        Box::new(source),
        path.extension().and_then(|s| s.to_str()),
        &path.display().to_string(),
        &|| false,
        Some(max_frames.max(1)),
    )
}
pub fn decode_with_cancel(
    path: &Path,
    cancel: &dyn Fn() -> bool,
) -> Result<Arc<AudioClip>, String> {
    let source = File::open(path).map_err(|e| format!("Opening {}: {e}", path.display()))?;
    decode_source(
        Box::new(source),
        path.extension().and_then(|s| s.to_str()),
        &path.display().to_string(),
        cancel,
        None,
    )
}
fn decode_source(
    source: Box<dyn symphonia::core::io::MediaSource>,
    extension: Option<&str>,
    label: &str,
    cancel: &dyn Fn() -> bool,
    max_frames: Option<usize>,
) -> Result<Arc<AudioClip>, String> {
    let mut hint = Hint::new();
    if let Some(ext) = extension {
        hint.with_extension(ext);
    }
    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            MediaSourceStream::new(source, Default::default()),
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| e.to_string())?;
    let track = reader
        .default_track(TrackType::Audio)
        .ok_or("No audio track")?
        .clone();
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("Missing audio format")?
        .clone();
    let sample_rate = params.sample_rate.ok_or("Missing sample rate")?;
    let channels = params
        .channels
        .as_ref()
        .map(|c| c.count())
        .ok_or("Missing channels")?;
    if channels == 0 || sample_rate == 0 {
        return Err("Invalid audio format".into());
    }
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default().gapless(true))
        .map_err(|e| e.to_string())?;
    let mut samples = Vec::new();
    let reserve = decode_pre_reserve_samples(track.num_frames, channels);
    let reserve = max_frames.map_or(reserve, |limit| reserve.min(limit.saturating_mul(channels)));
    if reserve > 0 {
        samples.reserve(reserve);
    }
    let max_samples = max_frames.map(|limit| limit.saturating_mul(channels));
    while let Some(packet) = reader.next_packet().map_err(|e| e.to_string())? {
        if cancel() {
            return Err("Audio decoding cancelled".into());
        }
        if packet.track_id != track.id {
            continue;
        }
        let buffer = decoder
            .decode(&packet)
            .map_err(|e| format!("Decoding {label}: {e}"))?;
        let old = samples.len();
        samples.resize(old + buffer.samples_interleaved(), 0.0);
        buffer.copy_to_slice_interleaved(&mut samples[old..]);
        if let Some(limit) = max_samples
            && samples.len() >= limit
        {
            samples.truncate(limit);
            break;
        }
    }
    if samples.is_empty() {
        return Err("Audio file contains no samples".into());
    }
    Ok(Arc::new(AudioClip {
        samples,
        channels,
        sample_rate,
    }))
}
pub fn stretch(audio: &AudioClip, speed: f64) -> Result<Arc<AudioClip>, String> {
    if !speed.is_finite() || !(0.25..=1.5).contains(&speed) {
        return Err("Playback speed must be between 25% and 150%".into());
    }
    let frames = audio.samples.len() / audio.channels.max(1);
    let frames = (frames as f64 / speed).round() as usize;
    let count = frames
        .checked_mul(audio.channels)
        .ok_or("Stretched audio is too large")?;
    if count > 300_000_000 {
        return Err("This song is too large for the current offline speed cache".into());
    }
    let mut output = vec![0.0; count];
    let mut processor =
        signalsmith_stretch::Stretch::preset_default(audio.channels as u32, audio.sample_rate);
    if !processor.exact(&audio.samples, &mut output) {
        return Err("The time stretcher could not process this audio".into());
    }
    Ok(Arc::new(AudioClip {
        samples: output,
        channels: audio.channels,
        sample_rate: audio.sample_rate,
    }))
}

#[derive(Clone)]
pub struct ScheduledSound {
    pub at_ms: f64,
    pub end_ms: Option<f64>,
    pub clip: Arc<AudioClip>,
    pub gain: f32,
}
struct Voice {
    clip: Arc<AudioClip>,
    frame: f64,
    gain: f32,
}

// The maximum end time in each subtree lets a seek skip whole blocks of
// expired cues, including gaps underneath a long-running loop.
#[derive(Default)]
struct CueIndex {
    max_end: Vec<f64>,
    leaves: usize,
}

impl CueIndex {
    const BLOCK: usize = 32;

    fn build(cues: &[ScheduledSound], speed: f64) -> Self {
        let leaves = cues.len().div_ceil(Self::BLOCK).next_power_of_two();
        let mut max_end = vec![f64::NEG_INFINITY; 2 * leaves];
        for (i, block) in cues.chunks(Self::BLOCK).enumerate() {
            max_end[leaves + i] = block
                .iter()
                .map(|c| cue_end(c, speed))
                .fold(f64::NEG_INFINITY, f64::max);
        }
        for i in (1..leaves).rev() {
            max_end[i] = max_end[i * 2].max(max_end[i * 2 + 1]);
        }
        Self { max_end, leaves }
    }

    fn overlaps(
        &self,
        cues: &[ScheduledSound],
        limit: usize,
        time: f64,
        speed: f64,
        out: &mut Vec<usize>,
    ) {
        if self.leaves == 0 {
            return;
        }
        let mut node = 1;
        // Walk left-to-right without recursion or callback allocations.
        loop {
            if self.max_end[node] > time {
                if node < self.leaves {
                    node *= 2;
                    continue;
                }
                let first = (node - self.leaves) * Self::BLOCK;
                if first >= limit {
                    break;
                }
                out.extend(
                    (first..limit.min(first + Self::BLOCK))
                        .filter(|&i| time < cue_end(&cues[i], speed)),
                );
            }
            while node > 1 && node % 2 == 1 {
                node /= 2;
            }
            if node == 1 {
                break;
            }
            node += 1;
        }
    }
}

/// Zero-latency, stereo-linked peak limiter for the master bus.
/// Transparent (unity gain) whenever the frame peak stays at or below `CEILING`,
/// with instant attack on overloads and a 50 ms exponential release back to 1.0.
struct MasterLimiter {
    gain: f32,
    release_coeff: f32,
}

impl MasterLimiter {
    const CEILING: f32 = 0.98;
    const RELEASE_SECS: f64 = 0.05;

    fn new(sample_rate: u32) -> Self {
        let rate = f64::from(sample_rate.max(1));
        let release_coeff = (1.0 - (-1.0 / (Self::RELEASE_SECS * rate)).exp()) as f32;
        Self {
            gain: 1.0,
            release_coeff,
        }
    }

    fn reset(&mut self) {
        self.gain = 1.0;
    }

    fn process_frame(&mut self, frame: &mut [f32]) {
        let mut peak = 0.0_f32;
        for sample in frame.iter() {
            let mag = sample.abs();
            if mag > peak {
                peak = mag;
            }
        }
        let target = if peak > Self::CEILING {
            Self::CEILING / peak
        } else {
            1.0
        };
        if target < self.gain {
            self.gain = target;
        } else if self.gain < 1.0 {
            self.gain += (target - self.gain) * self.release_coeff;
            if (1.0 - self.gain) < 1e-5 && target == 1.0 {
                self.gain = 1.0;
            }
        }
        if self.gain < 1.0 {
            for sample in frame.iter_mut() {
                *sample = (*sample * self.gain).clamp(-1.0, 1.0);
            }
        } else {
            for sample in frame.iter_mut() {
                if !sample.is_finite() {
                    *sample = 0.0;
                }
            }
        }
    }
}

const MIN_PLAYBACK_SPEED: f64 = 0.25;
const MAX_PLAYBACK_SPEED: f64 = 2.0;

#[inline]
fn clamp_speed(speed: f64) -> f64 {
    if speed.is_finite() && speed > 0.0 {
        speed.clamp(MIN_PLAYBACK_SPEED, MAX_PLAYBACK_SPEED)
    } else {
        1.0
    }
}

struct SongStretcher {
    sample_rate: u32,
    stretch: signalsmith_stretch::Stretch,
    in_buf: Vec<f32>,
    out_buf: Vec<f32>,
    discard_buf: Vec<f32>,
    out_pos: usize,
    out_len: usize,
    primed: bool,
    primed_song_ptr: usize,
    primed_speed: f64,
    read_frame: f64,
    in_frac: f64,
    expected_song_ms: f64,
}

impl SongStretcher {
    const CHUNK_OUT: usize = 256;

    fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.max(1);
        let mut stretch = signalsmith_stretch::Stretch::preset_default(2, rate);
        let in_lat = stretch.input_latency();
        let out_lat = stretch.output_latency();
        // Pre-allocate worst-case buffers off the audio thread (up to 2.0x speed).
        let max_in_frames = in_lat
            .max(out_lat.saturating_mul(2))
            .max(Self::CHUNK_OUT.saturating_mul(2) + 4)
            .max(128);
        let in_buf = vec![0.0; max_in_frames * 2];
        let mut out_buf = vec![0.0; Self::CHUNK_OUT * 2];
        let discard_buf = vec![0.0; out_lat.max(1) * 2];
        // Pre-warm Signalsmith internal processing off-thread so the first callback allocates nothing.
        let warm_frames = Self::CHUNK_OUT.min(128);
        stretch.process(&in_buf[..warm_frames * 2], &mut out_buf[..warm_frames * 2]);
        stretch.reset();
        out_buf.fill(0.0);
        Self {
            sample_rate: rate,
            stretch,
            in_buf,
            out_buf,
            discard_buf,
            out_pos: 0,
            out_len: 0,
            primed: false,
            primed_song_ptr: 0,
            primed_speed: 1.0,
            read_frame: 0.0,
            in_frac: 0.0,
            expected_song_ms: 0.0,
        }
    }

    fn reset(&mut self) {
        self.primed = false;
        self.out_pos = 0;
        self.out_len = 0;
        self.in_frac = 0.0;
    }

    fn fill_input(&mut self, song: &AudioClip, start_frame: f64, count: usize) -> usize {
        let cap_frames = self.in_buf.len() / 2;
        debug_assert!(
            count <= cap_frames,
            "SongStretcher::in_buf capacity exceeded ({count} > {cap_frames})"
        );
        let frames = count.min(cap_frames);
        let ratio = song.sample_rate as f64 / self.sample_rate as f64;
        for i in 0..frames {
            let src_frame = (start_frame + i as f64) * ratio;
            self.in_buf[i * 2] = song.sample(src_frame, 0);
            self.in_buf[i * 2 + 1] = song.sample(src_frame, 1);
        }
        frames
    }

    fn prime(&mut self, song: &AudioClip, song_time_ms: f64, speed: f64) {
        let speed = clamp_speed(speed);
        let sample_rate = self.sample_rate;
        self.stretch.reset();
        let in_lat = self.stretch.input_latency();
        let out_lat = self.stretch.output_latency();

        let base_frame = song_time_ms * sample_rate as f64 / 1000.0;
        // Discarding `out_lat` output frames below aligns output frame 0 directly to `base_frame`
        // (matching Signalsmith Stretch's `exact()`), so `start_in` must NOT subtract `out_lat * speed`.
        let start_in = base_frame;

        if in_lat > 0 {
            let in_frames = self.fill_input(song, start_in, in_lat);
            self.stretch.seek(&self.in_buf[..in_frames * 2], speed);
            self.read_frame = start_in + in_frames as f64;
        } else {
            self.read_frame = start_in;
        }

        if out_lat > 0 {
            let discard_frames = out_lat.min(self.discard_buf.len() / 2);
            debug_assert!(
                out_lat <= self.discard_buf.len() / 2,
                "SongStretcher::discard_buf capacity exceeded"
            );
            let req_discard_in = (discard_frames as f64 * speed).round() as usize;
            let discard_in = self.fill_input(song, self.read_frame, req_discard_in);
            self.stretch.process(
                &self.in_buf[..discard_in * 2],
                &mut self.discard_buf[..discard_frames * 2],
            );
            self.read_frame += discard_in as f64;
        }

        self.primed = true;
        self.primed_song_ptr = song as *const AudioClip as usize;
        self.primed_speed = speed;
        self.in_frac = 0.0;
        self.out_pos = 0;
        self.out_len = 0;
        self.expected_song_ms = song_time_ms;
    }

    fn next_frame(&mut self, song: &AudioClip, song_time_ms: f64, speed: f64) -> [f32; 2] {
        let speed = clamp_speed(speed);
        let song_ptr = song as *const AudioClip as usize;
        if !self.primed
            || self.primed_song_ptr != song_ptr
            || (self.primed_speed - speed).abs() > 1e-6
            || (song_time_ms - self.expected_song_ms).abs() > 2.0
        {
            self.prime(song, song_time_ms, speed);
        }

        if self.out_pos >= self.out_len {
            let in_exact = self.in_frac + Self::CHUNK_OUT as f64 * speed;
            let req_in = in_exact.floor() as usize;
            self.in_frac = in_exact - req_in as f64;
            let in_frames = self.fill_input(song, self.read_frame, req_in);
            self.read_frame += in_frames as f64;
            self.stretch.process(
                &self.in_buf[..in_frames * 2],
                &mut self.out_buf[..Self::CHUNK_OUT * 2],
            );
            self.out_pos = 0;
            self.out_len = Self::CHUNK_OUT;
        }

        let idx = self.out_pos * 2;
        self.out_pos += 1;
        self.expected_song_ms += speed * 1000.0 / self.sample_rate as f64;
        [self.out_buf[idx], self.out_buf[idx + 1]]
    }
}

/// Real-time stereo audio source mixed into playback ahead of the master limiter.
///
/// Implementations run on the audio output thread and must not block, lock, or allocate.
pub trait LiveAudioSource: Send {
    fn process_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        sample_rate: u32,
        is_playing: bool,
    );

    /// Timeline-aware stereo block processing; defaults to `process_block`.
    fn process_timeline_block(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        sample_rate: u32,
        is_playing: bool,
        _position_ms: f64,
        _speed: f64,
    ) {
        self.process_block(left, right, sample_rate, is_playing);
    }

    /// Called on pause, seek, or cycle wrap so live instruments cut ringing tails cleanly.
    fn reset(&mut self) {}
}

#[inline]
fn sanitize_live_sample(sample: f32) -> f32 {
    if sample.is_finite() { sample } else { 0.0 }
}

pub struct Mixer {
    pub song: Option<Arc<AudioClip>>,
    pub cues: Vec<ScheduledSound>,
    pub playing: bool,
    pub position_ms: f64,
    pub song_gain: f32,
    pub hitsound_gain: f32,
    pub master_gain: f32,
    pub song_peak: f32,
    pub hitsound_peak: f32,
    pub song_offset: f64,
    pub cycle: Option<(f64, f64)>,
    pub speed: f64,
    /// When true (default), slowed or sped-up song playback keeps its original pitch via the
    /// real-time stretcher. When false, the song reads directly at `speed` (classic pitch shift).
    pub keep_pitch: bool,
    pub limiter_enabled: bool,
    sample_rate: u32,
    voices: Vec<Voice>,
    /// The one Listen clip (drum hit detection's review): a new one replaces it.
    listen: Option<Voice>,
    active_cues: Vec<usize>,
    cue_index: CueIndex,
    next_cue: usize,
    indexed: bool,
    limiter: MasterLimiter,
    stretcher: SongStretcher,
    live_source: Option<Box<dyn LiveAudioSource>>,
    live_left: Vec<f32>,
    live_right: Vec<f32>,
    seek_fade_pos: usize,
    seek_fade_frames: usize,
    seek_fade_active: bool,
    cycle_wraps: u64,
}
impl Mixer {
    const LIVE_BLOCK_FRAMES: usize = 512;

    pub fn new(sample_rate: u32) -> Self {
        Self {
            song: None,
            cues: Vec::new(),
            playing: false,
            position_ms: 0.0,
            song_gain: 1.0,
            hitsound_gain: 1.0,
            master_gain: 1.0,
            song_peak: 0.0,
            hitsound_peak: 0.0,
            song_offset: 0.0,
            cycle: None,
            speed: 1.0,
            keep_pitch: true,
            limiter_enabled: true,
            sample_rate,
            voices: Vec::with_capacity(32),
            listen: None,
            active_cues: Vec::with_capacity(256),
            cue_index: CueIndex::default(),
            next_cue: 0,
            indexed: false,
            limiter: MasterLimiter::new(sample_rate),
            stretcher: SongStretcher::new(sample_rate),
            live_source: None,
            live_left: vec![0.0; Self::LIVE_BLOCK_FRAMES],
            live_right: vec![0.0; Self::LIVE_BLOCK_FRAMES],
            seek_fade_pos: 0,
            seek_fade_frames: (sample_rate as usize / 200).max(1),
            seek_fade_active: false,
            cycle_wraps: 0,
        }
    }
    /// Replaces the live audio source, returning the previous source so callers
    /// can drop it off the audio thread.
    pub fn set_live_source(
        &mut self,
        source: Option<Box<dyn LiveAudioSource>>,
    ) -> Option<Box<dyn LiveAudioSource>> {
        std::mem::replace(&mut self.live_source, source)
    }
    pub fn seek(&mut self, ms: f64) {
        self.seek_internal(ms, true);
    }

    fn seek_internal(&mut self, ms: f64, declick_song: bool) {
        self.position_ms = if ms.is_finite() { ms.max(0.0) } else { 0.0 };
        self.indexed = false;
        self.limiter.reset();
        self.stretcher.reset();
        if let Some(source) = self.live_source.as_mut() {
            source.reset();
        }
        if declick_song && self.playing {
            self.seek_fade_pos = 0;
            self.seek_fade_active = true;
        } else {
            self.seek_fade_active = false;
        }
    }
    pub fn audition(&mut self, clip: Arc<AudioClip>, gain: f32) {
        if self.voices.len() < 32 {
            self.voices.push(Voice {
                clip,
                frame: 0.0,
                gain,
            });
        }
    }
    /// Plays `clip` in place of the current Listen clip, or stops Listen with `None`. Lane
    /// auditions keep playing either way.
    pub fn listen(&mut self, clip: Option<Arc<AudioClip>>) {
        self.listen = clip.map(|clip| Voice { clip, frame: 0.0, gain: 1.0 });
    }
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        let out_channels = channels.max(1);
        let total_frames = out.len() / out_channels;
        let mut offset_frames = 0usize;
        while offset_frames < total_frames {
            let block_speed = clamp_speed(self.speed);
            let ms_per_frame = block_speed * 1000.0 / self.sample_rate.max(1) as f64;
            let mut sub_frames = (total_frames - offset_frames).min(Self::LIVE_BLOCK_FRAMES);
            if self.playing
                && let Some((left, right)) = self.cycle
                && right > left
            {
                if self.position_ms >= right || (right - self.position_ms) <= ms_per_frame * 1e-4 {
                    self.seek_internal(left, false);
                    self.cycle_wraps = self.cycle_wraps.wrapping_add(1);
                }
                let frames_to_right = ((right - self.position_ms) / ms_per_frame).ceil().max(1.0) as usize;
                sub_frames = sub_frames.min(frames_to_right);
            }
            let block = &mut out[offset_frames * out_channels..(offset_frames + sub_frames) * out_channels];
            let block_frames = sub_frames;
            let has_live = if block_frames > 0 {
                if let Some(source) = self.live_source.as_mut() {
                    let left = &mut self.live_left[..block_frames];
                    let right = &mut self.live_right[..block_frames];
                    left.fill(0.0);
                    right.fill(0.0);
                    source.process_timeline_block(
                        left,
                        right,
                        self.sample_rate,
                        self.playing,
                        self.position_ms,
                        block_speed,
                    );
                    true
                } else {
                    false
                }
            } else {
                false
            };
            for (frame_idx, frame) in block.chunks_mut(out_channels).enumerate() {
                if self.playing {
                    if !self.indexed {
                        self.active_cues.clear();
                        self.next_cue = self
                            .cues
                            .partition_point(|cue| cue.at_ms <= self.position_ms);
                        if self.cue_index.leaves == 0 {
                            // Direct mixer users may supply their own cue vector.
                            self.active_cues
                                .extend((0..self.next_cue).filter(|i| {
                                    self.position_ms < cue_end(&self.cues[*i], self.speed)
                                }));
                        } else {
                            self.cue_index.overlaps(
                                &self.cues,
                                self.next_cue,
                                self.position_ms,
                                self.speed,
                                &mut self.active_cues,
                            );
                        }
                        self.indexed = true;
                    }
                    while self.next_cue < self.cues.len()
                        && self.cues[self.next_cue].at_ms <= self.position_ms
                    {
                        self.active_cues.push(self.next_cue);
                        self.next_cue += 1;
                    }
                    let position = self.position_ms;
                    let cues = &self.cues;
                    let speed = clamp_speed(self.speed);
                    self.active_cues
                        .retain(|i| position < cue_end(&cues[*i], speed));
                }
                let speed = clamp_speed(self.speed);
                let stretched_song_frame = if self.playing && self.keep_pitch && (speed - 1.0).abs() > 1e-6 {
                    self.song.as_ref().map(|song| {
                        self.stretcher.next_frame(
                            song,
                            self.position_ms - self.song_offset,
                            speed,
                        )
                    })
                } else {
                    None
                };
                let song_fade = if self.seek_fade_active {
                    self.seek_fade_pos += 1;
                    let factor =
                        (self.seek_fade_pos as f32 / self.seek_fade_frames as f32).min(1.0);
                    if self.seek_fade_pos >= self.seek_fade_frames {
                        self.seek_fade_active = false;
                    }
                    factor
                } else {
                    1.0
                };
                for (channel, value) in frame.iter_mut().enumerate() {
                    if out_channels > 2 && channel >= 2 {
                        *value = 0.0;
                        continue;
                    }
                    let mut song_mix = 0.0;
                    let mut hs_mix = 0.0;
                    if self.playing {
                        if let Some([l, r]) = stretched_song_frame {
                            let sample = if out_channels == 1 {
                                (l + r) * 0.5
                            } else if channel == 0 {
                                l
                            } else {
                                r
                            };
                            song_mix += sample * self.song_gain * song_fade;
                        } else if let Some(song) = &self.song {
                            song_mix += song.sample(
                                (self.position_ms - self.song_offset)
                                    * song.sample_rate as f64
                                    / 1000.0,
                                channel,
                            ) * self.song_gain
                                * song_fade;
                        }
                        // Cues read by source position, so seek and cycle cannot double-trigger or skip a cue.
                        for index in &self.active_cues {
                            let cue = &self.cues[*index];
                            let elapsed = self.position_ms - cue.at_ms;
                            if elapsed < 0.0 {
                                continue;
                            }
                            let mut source =
                                elapsed / speed * cue.clip.sample_rate as f64 / 1000.0;
                            if let Some(end) = cue.end_ms {
                                if self.position_ms >= end {
                                    continue;
                                }
                                let len = cue.clip.samples.len() / cue.clip.channels;
                                if len > 0 {
                                    source %= len as f64;
                                }
                            }
                            hs_mix += cue.clip.sample(source, channel) * cue.gain * self.hitsound_gain;
                        }
                    }
                    for voice in self.voices.iter().chain(&self.listen) {
                        hs_mix += voice.clip.sample(voice.frame, channel) * voice.gain * self.hitsound_gain;
                    }
                    if has_live && frame_idx < block_frames {
                        let live_l = sanitize_live_sample(self.live_left[frame_idx]);
                        let live_r = sanitize_live_sample(self.live_right[frame_idx]);
                        let live_sample = if out_channels == 1 {
                            (live_l + live_r) * 0.5
                        } else if channel == 0 {
                            live_l
                        } else {
                            live_r
                        };
                        hs_mix += live_sample * self.hitsound_gain;
                    }
                    self.song_peak = self.song_peak.max(song_mix.abs());
                    self.hitsound_peak = self.hitsound_peak.max(hs_mix.abs());
                    let mix = song_mix + hs_mix;
                    *value = mix * self.master_gain;
                }
                if self.limiter_enabled {
                    self.limiter.process_frame(frame);
                } else {
                    for value in frame.iter_mut() {
                        *value = value.clamp(-1.0, 1.0);
                    }
                }
                for voice in self.voices.iter_mut().chain(&mut self.listen) {
                    voice.frame += voice.clip.sample_rate as f64 / self.sample_rate as f64;
                }
                self.voices
                    .retain(|v| v.frame < (v.clip.samples.len() / v.clip.channels) as f64);
                if self.listen.as_ref().is_some_and(|v| {
                    v.frame >= (v.clip.samples.len() / v.clip.channels.max(1)) as f64
                }) {
                    self.listen = None;
                }
                if self.playing {
                    self.position_ms += self.speed * 1000.0 / self.sample_rate as f64;
                }
            }
            offset_frames += sub_frames;
        }
    }
}

struct Schedule {
    song: Option<Arc<AudioClip>>,
    cues: Vec<ScheduledSound>,
    active: Vec<usize>,
    index: CueIndex,
    speed: f64,
}

impl Schedule {
    fn prepare(song: Option<Arc<AudioClip>>, mut cues: Vec<ScheduledSound>, speed: f64) -> Self {
        let speed = clamp_speed(speed);
        cues.sort_by(|a, b| a.at_ms.total_cmp(&b.at_ms));
        let active = Vec::with_capacity(cues.len());
        let index = CueIndex::build(&cues, speed);
        Self {
            song,
            cues,
            active,
            index,
            speed,
        }
    }

    fn swap_into(mut self, mixer: &mut Mixer) -> Self {
        let song_changed = match (&self.song, &mixer.song) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        let speed_changed = (self.speed - mixer.speed).abs() > 1e-6;
        std::mem::swap(&mut self.song, &mut mixer.song);
        std::mem::swap(&mut self.cues, &mut mixer.cues);
        std::mem::swap(&mut self.active, &mut mixer.active_cues);
        std::mem::swap(&mut self.index, &mut mixer.cue_index);
        std::mem::swap(&mut self.speed, &mut mixer.speed);
        mixer.indexed = false;
        if song_changed || speed_changed {
            mixer.stretcher.reset();
        }
        self
    }
}

type Retired = Arc<OnceLock<Schedule>>;

/// The caller's side of Listen. A Listen clip can be a whole song, so the audio thread never holds
/// its last handle: the slot keeps one until the audio thread lets go, then frees it on the caller's
/// thread in [`ListenSlot::collect`].
type ListenState = (Option<Arc<AudioClip>>, Vec<Arc<AudioClip>>);

#[derive(Default)]
struct ListenSlot(Mutex<ListenState>);

impl ListenSlot {
    /// Makes `clip` the current one and returns the handle to send to the audio thread.
    fn start(&self, clip: Option<Arc<AudioClip>>) -> Option<Arc<AudioClip>> {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.1.retain(|old| Arc::strong_count(old) > 1);
        if let Some(old) = std::mem::replace(&mut slot.0, clip.clone()) {
            slot.1.push(old);
        }
        clip
    }
    /// Whether the audio thread still holds (or is about to receive) the current clip.
    fn is_listening(&self) -> bool {
        let slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.0.as_ref().is_some_and(|clip| Arc::strong_count(clip) > 1)
    }
    /// Frees the clips the audio thread let go of; true while one is still held there.
    fn collect(&self) -> bool {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.1.retain(|clip| Arc::strong_count(clip) > 1);
        if slot.0.as_ref().is_some_and(|clip| Arc::strong_count(clip) == 1) {
            slot.0 = None;
        }
        slot.0.is_some() || !slot.1.is_empty()
    }
}
fn collect_retired(slots: &mut Vec<Retired>) -> bool {
    slots.retain(|slot| Arc::strong_count(slot) > 1);
    !slots.is_empty()
}

const CMD_RING_CAPACITY: usize = 256;
const RETIRE_LIVE_CAPACITY: usize = 64;
const MAX_COMMANDS_PER_BUFFER: usize = 64;

/// Inclusive `[min, max]` requested buffer bounds (in milliseconds) per backend.
pub fn buffer_ms_range(backend: OutputBackend) -> (u32, u32) {
    match backend {
        OutputBackend::System | OutputBackend::Asio => (2, 500),
        OutputBackend::Mme | OutputBackend::DirectSound => (10, 500),
    }
}

pub fn validate_buffer_ms(backend: OutputBackend, ms: u32) -> Result<(), String> {
    let (min, max) = buffer_ms_range(backend);
    if (min..=max).contains(&ms) {
        Ok(())
    } else {
        Err(format!("Enter an audio buffer between {min} and {max} ms."))
    }
}

fn buffer_request(
    backend: OutputBackend,
    ms: u32,
    rate: cpal::SampleRate,
    supported: &cpal::SupportedBufferSize,
) -> Result<cpal::BufferSize, String> {
    validate_buffer_ms(backend, ms)?;
    let frames = ((u64::from(ms) * u64::from(rate) + 500) / 1000) as u32;
    if let cpal::SupportedBufferSize::Range { min, max } = *supported
        && !(min..=max).contains(&frames)
    {
        return Err(format!(
            "This device supports {min}–{max} buffer frames at {rate} Hz. The {ms} ms request needs {frames} frames."
        ));
    }
    Ok(cpal::BufferSize::Fixed(frames))
}

#[doc(hidden)]
pub struct PreparedLoad {
    schedule: Schedule,
    slot: Retired,
}

enum Command {
    Load(Schedule, Retired),
    SetLiveSource(Option<Box<dyn LiveAudioSource>>),
    Play(bool, u64),
    Seek(f64, u64),
    Audition(Arc<AudioClip>, f32),
    Listen(Option<Arc<AudioClip>>),
    Gains(f32, f32),
    HitsoundGain(f32),
    Cycle(Option<(f64, f64)>),
    Offset(f64),
    KeepPitch(bool),
}

#[derive(Default)]
pub(crate) struct AudioTelemetry {
    pub(crate) position: CachePadded<AtomicU64>,
    pub(crate) playing: CachePadded<AtomicBool>,
    pub(crate) command_seq: CachePadded<AtomicU64>,
    pub(crate) peak: CachePadded<AtomicU32>,
    pub(crate) master_peak_lr: CachePadded<AtomicU64>,
    pub(crate) master_rms_lr: CachePadded<AtomicU64>,
    pub(crate) phase_corr: CachePadded<AtomicU32>,
    pub(crate) bus_peaks: CachePadded<AtomicU64>,
    pub(crate) underruns: CachePadded<AtomicU64>,
    pub(crate) observed_buffer_frames: CachePadded<AtomicU32>,
}

const SPECTRUM_TAP_CAP: usize = 16_384;

/// Lock-free mono sample tap fed by the real-time audio callback only when `enabled` is true.
pub struct SpectrumTap {
    enabled: AtomicBool,
    sample_rate: AtomicU32,
    push_count: AtomicU64,
    consumer: Mutex<SpscConsumer<f32, SPECTRUM_TAP_CAP>>,
}

impl SpectrumTap {
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn push_count(&self) -> u64 {
        self.push_count.load(Ordering::Relaxed)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.load(Ordering::Relaxed)
    }

    /// Drains pending mono samples into `dst` (up to `max_samples`), returning `(count, sample_rate)`.
    pub fn drain_into(&self, dst: &mut Vec<f32>, max_samples: usize) -> (usize, u32) {
        let mut rx = self.consumer.lock().unwrap_or_else(|e| e.into_inner());
        let mut drained = 0usize;
        while drained < max_samples {
            let Some(sample) = rx.try_pop() else { break };
            dst.push(sample);
            drained += 1;
        }
        (drained, self.sample_rate())
    }
}

struct EngineHandles {
    cmd_tx: SpscProducer<Command, CMD_RING_CAPACITY>,
    retire_live_rx: SpscConsumer<Box<dyn LiveAudioSource>, RETIRE_LIVE_CAPACITY>,
    telemetry: Arc<AudioTelemetry>,
    spectrum_tap: Arc<SpectrumTap>,
    clock_slot: Arc<ClockSnapshotSlot>,
}

fn build_render_parts(
    sample_rate: u32,
    channels: usize,
    rt_promotion: bool,
    telemetry: Option<Arc<AudioTelemetry>>,
) -> (StreamRender, EngineHandles) {
    let _ = monotonic_rel_ns(Instant::now());
    let (cmd_tx, cmd_rx) = spsc_ring::<Command, CMD_RING_CAPACITY>();
    let (retire_live_tx, retire_live_rx) =
        spsc_ring::<Box<dyn LiveAudioSource>, RETIRE_LIVE_CAPACITY>();
    let (tap_tx, tap_rx) = spsc_ring::<f32, SPECTRUM_TAP_CAP>();
    let telemetry = telemetry.unwrap_or_else(|| Arc::new(AudioTelemetry::default()));
    let spectrum_tap = Arc::new(SpectrumTap {
        enabled: AtomicBool::new(false),
        sample_rate: AtomicU32::new(sample_rate.max(1)),
        push_count: AtomicU64::new(0),
        consumer: Mutex::new(tap_rx),
    });
    let clock_slot = Arc::new(ClockSnapshotSlot::new());
    let render = StreamRender::new(
        sample_rate,
        channels,
        cmd_rx,
        retire_live_tx,
        Arc::clone(&telemetry),
        Arc::clone(&spectrum_tap),
        tap_tx,
        Arc::clone(&clock_slot),
    )
    .with_rt_promotion(rt_promotion);
    (
        render,
        EngineHandles {
            cmd_tx,
            retire_live_rx,
            telemetry,
            spectrum_tap,
            clock_slot,
        },
    )
}

pub struct AudioEngine {
    _stream: OutputStream,
    backend: OutputBackend,
    sample_rate: u32,
    effective_buffer_ms: Option<u32>,
    cmd_tx: Mutex<SpscProducer<Command, CMD_RING_CAPACITY>>,
    retirements: Mutex<Vec<Retired>>,
    retired_live_sources: Mutex<SpscConsumer<Box<dyn LiveAudioSource>, RETIRE_LIVE_CAPACITY>>,
    auditions: Mutex<Vec<Arc<AudioClip>>>,
    listen: ListenSlot,
    telemetry: Arc<AudioTelemetry>,
    spectrum_tap: Arc<SpectrumTap>,
    clock_slot: Arc<ClockSnapshotSlot>,
    pub errors: Arc<AtomicU64>,
    pub device_name: String,
    device_id: Option<String>,
}
impl AudioEngine {
    pub fn open(device_name: Option<&str>) -> Result<Self, String> {
        Self::open_with_buffer(device_name, None)
    }

    /// Requests a device buffer in milliseconds. The host chooses the actual callback size.
    pub fn open_with_buffer(device_name: Option<&str>, buffer_ms: Option<u32>) -> Result<Self, String> {
        Self::open_system(device_name, None, buffer_ms)
    }

    pub fn open_output(
        backend: OutputBackend, device_id: Option<&str>, buffer_ms: Option<u32>,
    ) -> Result<Self, String> {
        if backend == OutputBackend::System {
            return Self::open_system(None, device_id, buffer_ms);
        }
        if backend == OutputBackend::Asio {
            #[cfg(all(windows, feature = "asio-local"))]
            {
                let id = device_id.ok_or("Choose an ASIO driver")?;
                if !registered_asio_devices().iter().any(|device| device.id == id) {
                    return Err("Selected ASIO driver is unavailable".into());
                }
                let host = cpal::host_from_id(cpal::HostId::Asio)
                    .map_err(|error| error.to_string())?;
                return Self::open_cpal(backend, host, None, Some(id), buffer_ms);
            }
            #[cfg(not(all(windows, feature = "asio-local")))]
            return Err("ASIO is unavailable in this build".into());
        }
        #[cfg(windows)]
        {
            if let Some(ms) = buffer_ms {
                validate_buffer_ms(backend, ms)?;
            }
            let errors = Arc::new(AtomicU64::new(0));
            let (mut state, handles) = build_render_parts(NATIVE_SAMPLE_RATE, 2, false, None);
            let render = move |out: &mut [i16]| state.fill(out);
            let stream = match backend {
                OutputBackend::Mme => mme::open(device_id, buffer_ms, render, errors.clone()),
                OutputBackend::DirectSound => directsound::open(device_id, buffer_ms, render, errors.clone()),
                OutputBackend::System | OutputBackend::Asio => unreachable!("CPAL output opened above"),
            }?;
            Ok(Self {
                backend,
                sample_rate: NATIVE_SAMPLE_RATE,
                effective_buffer_ms: Some(stream.buffer_ms),
                device_name: stream.name.clone(),
                device_id: device_id.map(str::to_owned),
                _stream: OutputStream::Native { _stream: stream },
                cmd_tx: Mutex::new(handles.cmd_tx),
                retirements: Mutex::new(Vec::new()),
                retired_live_sources: Mutex::new(handles.retire_live_rx),
                auditions: Mutex::new(Vec::new()),
                listen: ListenSlot::default(),
                telemetry: handles.telemetry,
                spectrum_tap: handles.spectrum_tap,
                clock_slot: handles.clock_slot,
                errors,
            })
        }
        #[cfg(not(windows))]
        Err(format!("{} output is available only on Windows", backend.label()))
    }

    fn open_system(
        device_name: Option<&str>,
        device_id: Option<&str>,
        buffer_ms: Option<u32>,
    ) -> Result<Self, String> {
        if let Some(ms) = buffer_ms {
            validate_buffer_ms(OutputBackend::System, ms)?;
        }
        #[cfg(windows)]
        if let Ok(engine) = Self::open_wasapi_low_latency(device_name, device_id, buffer_ms) {
            return Ok(engine);
        }
        Self::open_cpal(
            OutputBackend::System,
            cpal::default_host(),
            device_name,
            device_id,
            buffer_ms,
        )
    }

    #[cfg(windows)]
    fn open_wasapi_low_latency(
        device_name: Option<&str>,
        device_id: Option<&str>,
        buffer_ms: Option<u32>,
    ) -> Result<Self, String> {
        let errors = Arc::new(AtomicU64::new(0));
        let telemetry = Arc::new(AudioTelemetry::default());
        let opened = wasapi_low_latency::open(
            device_name,
            device_id,
            buffer_ms,
            Arc::clone(&errors),
            Arc::clone(&telemetry),
            {
                let telemetry_for_render = Arc::clone(&telemetry);
                move |sample_rate, channels| {
                    build_render_parts(sample_rate, channels, true, Some(telemetry_for_render))
                }
            },
        )?;
        let effective_ms = opened.stream.buffer_ms;
        let device_name_out = opened.stream.name.clone();
        Ok(Self {
            _stream: OutputStream::Native {
                _stream: opened.stream,
            },
            backend: OutputBackend::System,
            sample_rate: opened.sample_rate,
            effective_buffer_ms: Some(effective_ms),
            cmd_tx: Mutex::new(opened.payload.cmd_tx),
            retirements: Mutex::new(Vec::new()),
            retired_live_sources: Mutex::new(opened.payload.retire_live_rx),
            auditions: Mutex::new(Vec::new()),
            listen: ListenSlot::default(),
            telemetry,
            spectrum_tap: opened.payload.spectrum_tap,
            clock_slot: opened.payload.clock_slot,
            errors,
            device_name: device_name_out,
            device_id: opened.device_id,
        })
    }

    pub fn output_devices(backend: OutputBackend) -> Result<Vec<OutputDevice>, String> {
        match backend {
            OutputBackend::System => cpal::default_host().output_devices()
                .map_err(|error| error.to_string())
                .map(|devices| devices.filter_map(|device| {
                    let id = device.id().ok()?.to_string();
                    let name = device.description().ok()?.name().to_owned();
                    Some(OutputDevice { id, name })
                }).collect()),
            OutputBackend::Asio => {
                #[cfg(all(windows, feature = "asio-local"))]
                return Ok(registered_asio_devices());
                #[cfg(not(all(windows, feature = "asio-local")))]
                return Err("ASIO is unavailable in this build".into());
            }
            #[cfg(windows)]
            OutputBackend::Mme => mme::devices(),
            #[cfg(windows)]
            OutputBackend::DirectSound => directsound::devices(),
            #[cfg(not(windows))]
            other => Err(format!("{} output is available only on Windows", other.label())),
        }
    }

    pub fn backend(&self) -> OutputBackend {
        self.backend
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn effective_buffer_ms(&self) -> Option<u32> {
        if let Some(ms) = self.effective_buffer_ms {
            return Some(ms);
        }
        let observed = self.telemetry.observed_buffer_frames.load(Ordering::Relaxed);
        (observed > 0 && self.sample_rate > 0).then(|| {
            ((u64::from(observed) * 1000 + u64::from(self.sample_rate) / 2)
                / u64::from(self.sample_rate))
            .max(1) as u32
        })
    }

    pub fn device_id(&self) -> Option<&str> {
        self.device_id.as_deref()
    }

    fn open_cpal(
        backend: OutputBackend,
        host: cpal::Host,
        device_name: Option<&str>,
        device_id: Option<&str>,
        buffer_ms: Option<u32>,
    ) -> Result<Self, String> {
        if let Some(ms) = buffer_ms {
            validate_buffer_ms(backend, ms)?;
        }
        let device = if let Some(id) = device_id {
            let id = id.parse::<cpal::DeviceId>().map_err(|error| error.to_string())?;
            host.device_by_id(&id).ok_or("Selected output device is unavailable")?
        } else if let Some(name) = device_name {
            host.output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.description().is_ok_and(|desc| desc.name() == name))
                .ok_or("Selected output device is unavailable")?
        } else {
            host.default_output_device()
                .ok_or("No audio output device is available")?
        };
        let name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "Audio output".into());
        let device_id = device.id().ok().map(|id| id.to_string());
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let mut config = supported.config();
        let sample_rate = config.sample_rate;
        let mut fixed_buffer_ms = None;
        if let Some(ms) = buffer_ms {
            let req = buffer_request(backend, ms, sample_rate, supported.buffer_size())?;
            if let cpal::BufferSize::Fixed(frames) = req {
                fixed_buffer_ms = Some(
                    ((u64::from(frames) * 1000 + u64::from(sample_rate.max(1)) / 2)
                        / u64::from(sample_rate.max(1)))
                    .max(1) as u32,
                );
            }
            config.buffer_size = req;
        }
        let errors = Arc::new(AtomicU64::new(0));
        let (render, handles) = build_render_parts(sample_rate, config.channels as usize, true, None);
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream::<f32>(&device, config, render, errors.clone()),
            cpal::SampleFormat::F64 => stream::<f64>(&device, config, render, errors.clone()),
            cpal::SampleFormat::I16 => stream::<i16>(&device, config, render, errors.clone()),
            cpal::SampleFormat::I24 => stream::<cpal::I24>(&device, config, render, errors.clone()),
            cpal::SampleFormat::I32 => stream::<i32>(&device, config, render, errors.clone()),
            cpal::SampleFormat::U16 => stream::<u16>(&device, config, render, errors.clone()),
            _ => return Err(UNSUPPORTED_SAMPLE_FORMAT.into()),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: OutputStream::System { _stream: stream },
            backend,
            sample_rate,
            effective_buffer_ms: fixed_buffer_ms,
            retirements: Mutex::new(Vec::new()),
            retired_live_sources: Mutex::new(handles.retire_live_rx),
            auditions: Mutex::new(Vec::new()),
            listen: ListenSlot::default(),
            cmd_tx: Mutex::new(handles.cmd_tx),
            telemetry: handles.telemetry,
            spectrum_tap: handles.spectrum_tap,
            clock_slot: handles.clock_slot,
            errors,
            device_name: name,
            device_id,
        })
    }

    fn drain_retired_live_sources(&self) {
        let mut live_rx = self
            .retired_live_sources
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while live_rx.try_pop().is_some() {}
    }

    fn send_command(&self, cmd: Command, critical: bool) {
        send_command_with_retry(&self.cmd_tx, &self.retired_live_sources, cmd, critical);
    }

    pub fn devices() -> Vec<String> {
        cpal::default_host()
            .output_devices()
            .map(|ds| {
                ds.filter_map(|d| d.description().ok().map(|d| d.name().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn load(&self, song: Option<Arc<AudioClip>>, cues: Vec<ScheduledSound>) {
        self.load_at_speed(song, cues, 1.0);
    }
    pub fn load_at_speed(
        &self,
        song: Option<Arc<AudioClip>>,
        cues: Vec<ScheduledSound>,
        speed: f64,
    ) {
        let schedule = Schedule::prepare(song, cues, speed);
        let slot = Arc::new(OnceLock::new());
        // Only producers touch this mutex. Keep ownership until the callback
        // has released its handle, so old clips and vectors are freed here.
        let mut retirements = self.retirements.lock().unwrap_or_else(|e| e.into_inner());
        collect_retired(&mut retirements);
        retirements.push(slot.clone());
        drop(retirements);
        self.send_command(Command::Load(schedule, slot), true);
    }
    /// Release completed schedules on the caller thread; request another poll
    /// if a callback has yet to hand one back.
    pub fn collect_retired(&self) -> bool {
        let mut slots = self.retirements.lock().unwrap_or_else(|e| e.into_inner());
        self.drain_retired_live_sources();
        let listening = self.listen.collect();
        let mut auditions = self.auditions.lock().unwrap_or_else(|e| e.into_inner());
        auditions.retain(|clip| Arc::strong_count(clip) > 1);
        let auditioning = !auditions.is_empty();
        collect_retired(&mut slots) || listening || auditioning
    }

    /// Installs or replaces the live audio source mixed ahead of the master limiter.
    ///
    /// The previous source is sent back over a bounded SPSC retirement ring so it is
    /// dropped on the caller's thread rather than the real-time audio thread.
    pub fn set_live_source(&self, source: Option<Box<dyn LiveAudioSource>>) {
        self.drain_retired_live_sources();
        self.send_command(Command::SetLiveSource(source), true);
    }

    /// Detaches any active live audio source.
    pub fn clear_live_source(&self) {
        self.set_live_source(None);
    }

    /// Plays `clip` in place of the current Listen clip; `None` stops Listen.
    pub fn listen(&self, clip: Option<Arc<AudioClip>>) {
        let clip = self.listen.start(clip);
        self.send_command(Command::Listen(clip), true);
    }
    /// Whether a Listen clip is still playing.
    pub fn is_listening(&self) -> bool {
        self.listen.is_listening()
    }

    pub fn play(&self, on: bool) {
        let seq = self.telemetry.command_seq.fetch_add(1, Ordering::AcqRel) + 1;
        self.telemetry.playing.store(on, Ordering::Release);
        self.send_command(Command::Play(on, seq), true);
    }
    pub fn seek(&self, ms: f64) {
        let clamped = if ms.is_finite() { ms.max(0.0) } else { 0.0 };
        let seq = self.telemetry.command_seq.fetch_add(1, Ordering::AcqRel) + 1;
        self.telemetry.position.store(clamped.to_bits(), Ordering::Release);
        self.send_command(Command::Seek(clamped, seq), true);
    }
    pub fn audition(&self, clip: Arc<AudioClip>, gain: f32) {
        let mut auditions = self.auditions.lock().unwrap_or_else(|e| e.into_inner());
        auditions.retain(|held| Arc::strong_count(held) > 1);
        auditions.push(clip.clone());
        drop(auditions);
        self.send_command(Command::Audition(clip, gain), false);
    }
    pub fn gains(&self, song: f32, master: f32) {
        self.send_command(
            Command::Gains(song.clamp(0.0, 1.0), master.clamp(0.2, 1.0)),
            false,
        );
    }
    pub fn hitsound_gain(&self, hitsound: f32) {
        self.send_command(Command::HitsoundGain(hitsound.clamp(0.0, 2.0)), false);
    }
    pub fn cycle(&self, range: Option<(f64, f64)>) {
        self.send_command(Command::Cycle(range), true);
    }
    pub fn offset(&self, ms: f64) {
        self.send_command(Command::Offset(ms), true);
    }
    pub fn keep_pitch(&self, enabled: bool) {
        self.send_command(Command::KeepPitch(enabled), true);
    }
    pub fn peaks(&self) -> (f32, f32, f32) {
        let master = f32::from_bits(self.telemetry.peak.load(Ordering::Relaxed));
        let packed = self.telemetry.bus_peaks.load(Ordering::Relaxed);
        let song = f32::from_bits((packed >> 32) as u32);
        let hitsounds = f32::from_bits(packed as u32);
        (master, song, hitsounds)
    }
    pub fn stereo_peaks(&self) -> (f32, f32) {
        unpack_f32_pair(self.telemetry.master_peak_lr.load(Ordering::Relaxed))
    }
    pub fn stereo_rms(&self) -> (f32, f32) {
        unpack_f32_pair(self.telemetry.master_rms_lr.load(Ordering::Relaxed))
    }
    pub fn phase_correlation(&self) -> f32 {
        f32::from_bits(self.telemetry.phase_corr.load(Ordering::Relaxed))
    }
    pub fn underruns(&self) -> u64 {
        self.telemetry.underruns.load(Ordering::Relaxed)
    }
    pub fn position_ms(&self) -> f64 {
        f64::from_bits(self.telemetry.position.load(Ordering::Relaxed))
    }
    pub fn is_playing(&self) -> bool {
        #[cfg(windows)]
        if let OutputStream::Native { _stream: stream } = &self._stream
            && stream.thread.as_ref().is_some_and(|thread| thread.is_finished())
        {
            return false;
        }
        self.telemetry.playing.load(Ordering::Relaxed)
    }

    pub fn telemetry_handle(&self) -> AudioTelemetryHandle {
        AudioTelemetryHandle(Arc::clone(&self.telemetry))
    }

    pub fn spectrum_tap(&self) -> Arc<SpectrumTap> {
        Arc::clone(&self.spectrum_tap)
    }

    pub fn set_spectrum_tap_enabled(&self, enabled: bool) {
        self.spectrum_tap.set_enabled(enabled);
    }

    pub fn clock_snapshot(&self) -> Option<ClockSnapshot> {
        self.clock_slot.load()
    }
}

/// Cloneable lock-free handle to an [`AudioEngine`]'s real-time telemetry atomics.
#[derive(Clone)]
pub struct AudioTelemetryHandle(Arc<AudioTelemetry>);

impl AudioTelemetryHandle {
    pub fn position_ms(&self) -> f64 {
        f64::from_bits(self.0.position.load(Ordering::Relaxed))
    }

    pub fn is_playing(&self) -> bool {
        self.0.playing.load(Ordering::Relaxed)
    }

    pub fn peaks(&self) -> (f32, f32, f32) {
        let master = f32::from_bits(self.0.peak.load(Ordering::Relaxed));
        let packed = self.0.bus_peaks.load(Ordering::Relaxed);
        let song = f32::from_bits((packed >> 32) as u32);
        let hitsounds = f32::from_bits(packed as u32);
        (master, song, hitsounds)
    }

    pub fn stereo_peaks(&self) -> (f32, f32) {
        unpack_f32_pair(self.0.master_peak_lr.load(Ordering::Relaxed))
    }

    pub fn stereo_rms(&self) -> (f32, f32) {
        unpack_f32_pair(self.0.master_rms_lr.load(Ordering::Relaxed))
    }

    pub fn phase_correlation(&self) -> f32 {
        f32::from_bits(self.0.phase_corr.load(Ordering::Relaxed))
    }

    pub fn underruns(&self) -> u64 {
        self.0.underruns.load(Ordering::Relaxed)
    }
}

#[inline]
fn pack_f32_pair(a: f32, b: f32) -> u64 {
    let a = if a.is_finite() { a } else { 0.0 };
    let b = if b.is_finite() { b } else { 0.0 };
    ((a.to_bits() as u64) << 32) | (b.to_bits() as u64)
}

#[inline]
fn unpack_f32_pair(packed: u64) -> (f32, f32) {
    (
        f32::from_bits((packed >> 32) as u32),
        f32::from_bits(packed as u32),
    )
}

fn send_command_with_retry(
    cmd_tx: &Mutex<SpscProducer<Command, CMD_RING_CAPACITY>>,
    retire_live_rx: &Mutex<SpscConsumer<Box<dyn LiveAudioSource>, RETIRE_LIVE_CAPACITY>>,
    mut cmd: Command,
    critical: bool,
) {
    if let Ok(mut live_rx) = retire_live_rx.lock() {
        while live_rx.try_pop().is_some() {}
    }
    for attempt in 0..32 {
        let pushed = {
            let mut tx = cmd_tx.lock().unwrap_or_else(|e| e.into_inner());
            tx.try_push(cmd)
        };
        match pushed {
            Ok(()) => return,
            Err(rejected) => {
                if !critical {
                    return;
                }
                cmd = rejected;
                if attempt < 8 {
                    std::thread::yield_now();
                } else {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

struct StreamRender {
    mixer: Mixer,
    receive: SpscConsumer<Command, CMD_RING_CAPACITY>,
    retire_live: SpscProducer<Box<dyn LiveAudioSource>, RETIRE_LIVE_CAPACITY>,
    telemetry: Arc<AudioTelemetry>,
    spectrum_tap: Arc<SpectrumTap>,
    tap_tx: SpscProducer<f32, SPECTRUM_TAP_CAP>,
    clock_slot: Arc<ClockSnapshotSlot>,
    clock_dll: AudioClockDll,
    clock_generation: u64,
    applied_seq: u64,
    channels: usize,
    scratch: Vec<f32>,
    rt_promotion: bool,
    rt_promoted: bool,
    last_callback: Option<Instant>,
}

impl StreamRender {
    #[allow(clippy::too_many_arguments)]
    fn new(
        rate: u32,
        channels: usize,
        receive: SpscConsumer<Command, CMD_RING_CAPACITY>,
        retire_live: SpscProducer<Box<dyn LiveAudioSource>, RETIRE_LIVE_CAPACITY>,
        telemetry: Arc<AudioTelemetry>,
        spectrum_tap: Arc<SpectrumTap>,
        tap_tx: SpscProducer<f32, SPECTRUM_TAP_CAP>,
        clock_slot: Arc<ClockSnapshotSlot>,
    ) -> Self {
        let channels = channels.max(1);
        // Reused, frame-aligned scratch space. Callbacks never allocate sample buffers.
        let samples = (262144 / channels).max(1) * channels;
        Self {
            mixer: Mixer::new(rate),
            receive,
            retire_live,
            telemetry,
            spectrum_tap,
            tap_tx,
            clock_slot,
            clock_dll: AudioClockDll::new(0.1),
            clock_generation: 0,
            applied_seq: 0,
            channels,
            scratch: vec![0.0; samples],
            rt_promotion: false,
            rt_promoted: false,
            last_callback: None,
        }
    }

    fn with_rt_promotion(mut self, enabled: bool) -> Self {
        self.rt_promotion = enabled;
        self
    }

    fn fill<T: cpal::SizedSample + cpal::FromSample<f32>>(&mut self, out: &mut [T]) {
        let _denormal = denormal::DenormalGuard::enter();
        if self.rt_promotion && !self.rt_promoted {
            self.rt_promoted = true;
            rt_thread::promote_callback_thread();
        }
        let cb_now = Instant::now();
        let total_frames = out.len() / self.channels;
        if total_frames > 0 {
            let _ = self.telemetry.observed_buffer_frames.compare_exchange(
                0,
                total_frames as u32,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
            if self.rt_promotion {
                let period_secs = total_frames as f64 / self.mixer.sample_rate.max(1) as f64;
                if period_secs >= 0.002
                    && let Some(prev) = self.last_callback
                    && cb_now.duration_since(prev).as_secs_f64() > period_secs * 1.8
                {
                    self.telemetry.underruns.fetch_add(1, Ordering::Relaxed);
                }
                self.last_callback = Some(cb_now);
            }
        }

        let mut reset_dll = false;
        for _ in 0..MAX_COMMANDS_PER_BUFFER {
            let Some(command) = self.receive.try_pop() else {
                break;
            };
            match command {
                Command::Load(schedule, slot) => {
                    let prev_speed = self.mixer.speed;
                    let old = schedule.swap_into(&mut self.mixer);
                    let _ = slot.set(old);
                    if (prev_speed - self.mixer.speed).abs() > 1e-6 {
                        reset_dll = true;
                    }
                }
                Command::SetLiveSource(source) => {
                    if let Some(old) = self.mixer.set_live_source(source) {
                        let _ = self.retire_live.try_push(old);
                    }
                }
                Command::Play(on, seq) => {
                    self.applied_seq = self.applied_seq.max(seq);
                    let was_playing = self.mixer.playing;
                    self.mixer.playing = on;
                    if on != was_playing {
                        reset_dll = true;
                    }
                    if on && !was_playing {
                        self.mixer.seek_fade_pos = 0;
                        self.mixer.seek_fade_active = true;
                    } else if !on && was_playing
                        && let Some(source) = self.mixer.live_source.as_mut() {
                            source.reset();
                        }
                }
                Command::Seek(ms, seq) => {
                    self.applied_seq = self.applied_seq.max(seq);
                    self.mixer.seek(ms);
                    reset_dll = true;
                }
                Command::Audition(clip, gain) => self.mixer.audition(clip, gain),
                Command::Listen(clip) => self.mixer.listen(clip),
                Command::Gains(song, master) => {
                    self.mixer.song_gain = song;
                    self.mixer.master_gain = master;
                }
                Command::HitsoundGain(hitsound) => {
                    self.mixer.hitsound_gain = hitsound;
                }
                Command::Cycle(range) => self.mixer.cycle = range,
                Command::Offset(ms) => {
                    self.mixer.song_offset = ms;
                    self.mixer.stretcher.reset();
                }
                Command::KeepPitch(enabled) => {
                    if self.mixer.keep_pitch != enabled {
                        self.mixer.keep_pitch = enabled;
                        self.mixer.stretcher.reset();
                    }
                }
            }
        }

        let wraps_before = self.mixer.cycle_wraps;
        let song_pos_before = self.mixer.position_ms;
        let speed_for_dll = clamp_speed(self.mixer.speed);

        let mut peak_l = 0.0_f32;
        let mut peak_r = 0.0_f32;
        let mut sum_l2 = 0.0_f64;
        let mut sum_r2 = 0.0_f64;
        let mut sum_lr = 0.0_f64;
        let mut rendered_frames = 0_usize;
        self.mixer.song_peak = 0.0;
        self.mixer.hitsound_peak = 0.0;
        for chunk in out.chunks_mut(self.scratch.len()) {
            let buffer = &mut self.scratch[..chunk.len()];
            self.mixer.render(buffer, self.channels);
            for (sample, value) in chunk.iter_mut().zip(buffer.iter()) {
                *sample = T::from_sample(*value);
            }
            let tap_on = self.spectrum_tap.enabled.load(Ordering::Relaxed);
            for frame in buffer.chunks_exact(self.channels) {
                let l = frame[0];
                let r = if self.channels >= 2 { frame[1] } else { l };
                peak_l = peak_l.max(l.abs());
                peak_r = peak_r.max(r.abs());
                let lf = f64::from(l);
                let rf = f64::from(r);
                sum_l2 += lf * lf;
                sum_r2 += rf * rf;
                sum_lr += lf * rf;
                rendered_frames += 1;
                if tap_on {
                    let _ = self.tap_tx.try_push(0.5 * (l + r));
                }
            }
            if tap_on {
                self.spectrum_tap
                    .push_count
                    .fetch_add((buffer.len() / self.channels) as u64, Ordering::Relaxed);
            }
        }

        if self.mixer.cycle_wraps != wraps_before {
            reset_dll = true;
        }
        if reset_dll {
            self.clock_generation = self.clock_generation.wrapping_add(1);
            self.clock_dll.reset(self.clock_generation);
        }
        if total_frames > 0 {
            let anchor_song_ms = if self.mixer.playing {
                if self.mixer.cycle_wraps == wraps_before {
                    song_pos_before
                } else {
                    self.mixer.cycle.map_or(self.mixer.position_ms, |(l, _)| l)
                }
            } else {
                self.mixer.position_ms
            };
            let snap = self.clock_dll.update(
                monotonic_rel_ns(cb_now),
                total_frames as u32,
                self.mixer.sample_rate,
                anchor_song_ms,
                speed_for_dll,
                self.mixer.playing,
            );
            self.clock_slot.store(snap);
        }
        let highest = peak_l.max(peak_r);
        self.telemetry.peak.store(highest.to_bits(), Ordering::Relaxed);
        self.telemetry
            .master_peak_lr
            .store(pack_f32_pair(peak_l, peak_r), Ordering::Relaxed);
        let frames_denom = rendered_frames.max(1) as f64;
        let rms_l = (sum_l2 / frames_denom).sqrt() as f32;
        let rms_r = (sum_r2 / frames_denom).sqrt() as f32;
        self.telemetry
            .master_rms_lr
            .store(pack_f32_pair(rms_l, rms_r), Ordering::Relaxed);
        let corr_denom = (sum_l2 * sum_r2).sqrt();
        let phase_corr = if corr_denom > 1e-12 {
            (sum_lr / corr_denom).clamp(-1.0, 1.0) as f32
        } else {
            0.0
        };
        let phase_corr = if phase_corr.is_finite() { phase_corr } else { 0.0 };
        self.telemetry
            .phase_corr
            .store(phase_corr.to_bits(), Ordering::Relaxed);
        let bus_packed = pack_f32_pair(self.mixer.song_peak, self.mixer.hitsound_peak);
        self.telemetry.bus_peaks.store(bus_packed, Ordering::Relaxed);

        let synced = self.applied_seq >= self.telemetry.command_seq.load(Ordering::Acquire);
        if synced {
            self.telemetry
                .position
                .store(self.mixer.position_ms.to_bits(), Ordering::Relaxed);
            self.telemetry
                .playing
                .store(self.mixer.playing, Ordering::Relaxed);
        }
    }
}

#[doc(hidden)]
pub struct RtBenchHarness {
    handles: EngineHandles,
    retirements: Vec<Retired>,
    render: StreamRender,
}

impl RtBenchHarness {
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let (render, handles) = build_render_parts(sample_rate, channels, false, None);
        Self {
            handles,
            retirements: Vec::with_capacity(16),
            render,
        }
    }

    pub fn prepare_load(
        &mut self,
        song: Option<Arc<AudioClip>>,
        cues: Vec<ScheduledSound>,
        speed: f64,
    ) -> PreparedLoad {
        let slot = Arc::new(OnceLock::new());
        self.retirements.push(slot.clone());
        PreparedLoad {
            schedule: Schedule::prepare(song, cues, speed),
            slot,
        }
    }

    pub fn push_prepared_load(&mut self, load: PreparedLoad) {
        let _ = self
            .handles
            .cmd_tx
            .try_push(Command::Load(load.schedule, load.slot));
    }

    pub fn play(&mut self, on: bool) {
        let seq = self.handles.telemetry.command_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.handles.cmd_tx.try_push(Command::Play(on, seq));
    }

    pub fn seek(&mut self, ms: f64) {
        let seq = self.handles.telemetry.command_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.handles.cmd_tx.try_push(Command::Seek(ms, seq));
    }

    pub fn cycle(&mut self, range: Option<(f64, f64)>) {
        let _ = self.handles.cmd_tx.try_push(Command::Cycle(range));
    }

    pub fn keep_pitch(&mut self, on: bool) {
        let _ = self.handles.cmd_tx.try_push(Command::KeepPitch(on));
    }

    pub fn audition(&mut self, clip: Arc<AudioClip>, gain: f32) {
        let _ = self.handles.cmd_tx.try_push(Command::Audition(clip, gain));
    }

    pub fn listen(&mut self, clip: Option<Arc<AudioClip>>) {
        let _ = self.handles.cmd_tx.try_push(Command::Listen(clip));
    }

    pub fn set_live_source(&mut self, source: Option<Box<dyn LiveAudioSource>>) {
        let _ = self.handles.cmd_tx.try_push(Command::SetLiveSource(source));
    }

    pub fn fill(&mut self, out: &mut [f32]) {
        self.render.fill(out);
    }

    pub fn drain_retired(&mut self) {
        while self.handles.retire_live_rx.try_pop().is_some() {}
        self.retirements.clear();
    }

    pub fn stereo_peaks(&self) -> (f32, f32) {
        unpack_f32_pair(self.handles.telemetry.master_peak_lr.load(Ordering::Relaxed))
    }

    pub fn stereo_rms(&self) -> (f32, f32) {
        unpack_f32_pair(self.handles.telemetry.master_rms_lr.load(Ordering::Relaxed))
    }

    pub fn phase_correlation(&self) -> f32 {
        f32::from_bits(self.handles.telemetry.phase_corr.load(Ordering::Relaxed))
    }

    pub fn spectrum_tap(&self) -> Arc<SpectrumTap> {
        Arc::clone(&self.handles.spectrum_tap)
    }

    pub fn set_spectrum_tap_enabled(&self, enabled: bool) {
        self.handles.spectrum_tap.set_enabled(enabled);
    }

    pub fn clock_snapshot(&self) -> Option<ClockSnapshot> {
        self.handles.clock_slot.load()
    }
}

fn stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut render: StreamRender,
    errors: Arc<AtomicU64>,
) -> Result<cpal::Stream, String> {
    device
        .build_output_stream::<T, _, _>(
            config,
            move |out, _| render.fill(out),
            move |_error| {
                errors.fetch_add(1, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| e.to_string())
}
fn cue_end(cue: &ScheduledSound, speed: f64) -> f64 {
    cue.end_ms
        .unwrap_or(cue.at_ms + cue.clip.duration_ms() * speed)
}

#[cfg(test)]
mod asio_tests {
    use super::*;
    use serde::Deserialize;

    #[test]
    fn saved_asio_backend_is_readable_even_without_local_sdk_support() {
        let saved = serde::de::value::StrDeserializer::<serde::de::value::Error>::new("asio");
        assert_eq!(OutputBackend::deserialize(saved).unwrap(), OutputBackend::Asio);
        assert_eq!(
            OutputBackend::available().contains(&OutputBackend::Asio),
            cfg!(all(windows, feature = "asio-local")),
        );
        assert!(AudioEngine::open_output(OutputBackend::Asio, None, None).is_err());
    }

    #[test]
    fn an_unplayable_sample_format_is_not_named_by_its_code_name() {
        use cpal::SampleFormat::*;
        for format in [I8, I24, I64, U8, U24, U32, U64, F64] {
            for name in [format!("{format:?}"), format.to_string()] {
                assert!(!UNSUPPORTED_SAMPLE_FORMAT.contains(&name), "{UNSUPPORTED_SAMPLE_FORMAT} names {name}");
            }
        }
    }

    #[cfg(all(windows, feature = "asio-local"))]
    #[test]
    fn asio_registration_listing_uses_cpal_ids_without_opening_drivers() {
        let devices = AudioEngine::output_devices(OutputBackend::Asio).unwrap();
        for device in &devices {
            let id = device.id.parse::<cpal::DeviceId>().unwrap();
            assert_eq!(id.host(), cpal::HostId::Asio);
            assert_eq!(id.id(), device.name);
        }
        assert!(AudioEngine::open_output(OutputBackend::Asio, Some("wasapi:invalid"), None).is_err());
        println!("{} ASIO registrations from the SDK name query", devices.len());
    }
}

#[cfg(test)]
mod callback_benchmark {
    use super::*;

    #[test]
    fn shared_renderer_produces_native_stereo_pcm_and_updates_telemetry() {
        let (mut render, mut handles) = build_render_parts(48_000, 2, false, None);
        let send = &mut handles.cmd_tx;
        let telemetry = &handles.telemetry;
        assert!(send.try_push(Command::Seek(400.0, 1)).is_ok());
        assert!(
            send.try_push(Command::Audition(
                Arc::new(AudioClip {
                    samples: vec![0.25; 2],
                    channels: 1,
                    sample_rate: 48_000,
                }),
                1.0,
            ))
            .is_ok()
        );
        let mut pcm = [0_i16; 4];
        render.fill(&mut pcm);
        assert_eq!(pcm, [8192; 4]);
        assert_eq!(
            f32::from_bits(telemetry.peak.load(Ordering::Relaxed)),
            0.25
        );
        assert_eq!(
            unpack_f32_pair(telemetry.master_peak_lr.load(Ordering::Relaxed)),
            (0.25, 0.25)
        );
        assert_eq!(
            unpack_f32_pair(telemetry.master_rms_lr.load(Ordering::Relaxed)),
            (0.25, 0.25)
        );
        assert!(
            (f32::from_bits(telemetry.phase_corr.load(Ordering::Relaxed)) - 1.0).abs() < 1e-6
        );
        assert_eq!(
            f64::from_bits(telemetry.position.load(Ordering::Relaxed)),
            400.0
        );
    }

    #[test]
    fn send_command_never_drops_critical_stop_or_unload_under_full_ring() {
        let (tx, mut rx) = spsc_ring::<Command, CMD_RING_CAPACITY>();
        let (_retire_tx, retire_rx) =
            spsc_ring::<Box<dyn LiveAudioSource>, RETIRE_LIVE_CAPACITY>();
        let tx = Arc::new(Mutex::new(tx));
        let retire_rx = Mutex::new(retire_rx);
        {
            let mut guard = tx.lock().unwrap();
            for i in 0..CMD_RING_CAPACITY {
                assert!(guard.try_push(Command::HitsoundGain(1.0)).is_ok(), "slot {i}");
            }
        }
        let seen_stop = Arc::new(AtomicBool::new(false));
        let seen_stop_reader = Arc::clone(&seen_stop);
        let reader = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            while let Some(cmd) = rx.try_pop() {
                if let Command::Play(false, 99) = cmd {
                    seen_stop_reader.store(true, Ordering::Release);
                    break;
                }
                if rx.is_empty() {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
        send_command_with_retry(&tx, &retire_rx, Command::Play(false, 99), true);
        reader.join().unwrap();
        assert!(seen_stop.load(Ordering::Acquire));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "opens Windows audio devices"]
    fn native_outputs_process_silent_transport_commands() {
        use std::time::{Duration, Instant};

        fn wait_for(condition: impl Fn() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !condition() {
                assert!(Instant::now() < deadline, "audio worker did not process its command");
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        for backend in [OutputBackend::Mme, OutputBackend::DirectSound] {
            let devices = AudioEngine::output_devices(backend).unwrap();
            assert!(!devices.is_empty(), "{} returned no devices", backend.label());
            let invalid = AudioEngine::open_output(backend, Some("nonexistent-device-id"), Some(80));
            assert!(invalid.is_err(), "{} accepted an invalid device ID", backend.label());
            for device in [None, devices.first()] {
                let audio = AudioEngine::open_output(backend, device.map(|item| item.id.as_str()), Some(80))
                    .unwrap_or_else(|error| panic!("{} could not open: {error}", backend.label()));
                assert_eq!(audio.backend(), backend);
                if let Some(device) = device {
                    assert_eq!(audio.device_name, device.name);
                }
                assert!(audio.effective_buffer_ms().unwrap() >= 60);
                // An empty schedule exercises the actual callback without producing audible samples.
                audio.seek(1000.0);
                wait_for(|| audio.position_ms() == 1000.0);
                audio.play(true);
                wait_for(|| audio.is_playing() && audio.position_ms() > 1000.0);
                assert_eq!(audio.peaks().0.to_bits(), 0);
                audio.play(false);
                wait_for(|| !audio.is_playing());
                assert_eq!(audio.errors.load(Ordering::Relaxed), 0, "{} stream errored", backend.label());
            }
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "opens Windows audio devices"]
    fn wasapi_low_latency_shared_mode_negotiates_low_latency_without_underruns() {
        use std::time::{Duration, Instant};

        fn wait_for(condition: impl Fn() -> bool) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !condition() {
                assert!(Instant::now() < deadline, "WASAPI worker did not process command");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        let audio = AudioEngine::open_output(OutputBackend::System, None, None)
            .expect("default WASAPI output opens");
        let eff = audio.effective_buffer_ms();
        eprintln!(
            "WASAPI default device '{}' ({} Hz) effective_buffer_ms={eff:?}",
            audio.device_name,
            audio.sample_rate()
        );
        assert!(eff.is_some_and(|ms| ms <= 10), "expected low-latency WASAPI buffer, got {eff:?}");
        audio.seek(500.0);
        wait_for(|| audio.position_ms() == 500.0);
        audio.play(true);
        wait_for(|| audio.is_playing() && audio.position_ms() > 500.0);
        audio.play(false);
        wait_for(|| !audio.is_playing());
        assert_eq!(audio.errors.load(Ordering::Relaxed), 0);
        assert_eq!(audio.underruns(), 0);
    }

    #[test]
    fn polling_releases_replaced_song_without_another_load() {
        let song = Arc::new(AudioClip {
            samples: vec![0.1],
            channels: 1,
            sample_rate: 48_000,
        });
        let weak = Arc::downgrade(&song);
        let mut mixer = Mixer::new(48_000);
        mixer.song = Some(song);
        let slot = Arc::new(OnceLock::new());
        let mut slots = vec![slot.clone()];
        let old = Schedule::prepare(None, vec![], 1.0).swap_into(&mut mixer);
        assert!(slot.set(old).is_ok());
        assert!(collect_retired(&mut slots));
        drop(slot);
        assert!(!collect_retired(&mut slots));
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn prepared_overlap_index_matches_scan_across_blocks_and_long_tails() {
        let clip = Arc::new(AudioClip {
            samples: vec![0.1; 4],
            channels: 1,
            sample_rate: 1000,
        });
        let cues: Vec<_> = (0..500)
            .map(|i| ScheduledSound {
                at_ms: i as f64,
                end_ms: (i % 69 == 0).then_some(i as f64 + 92.0),
                clip: clip.clone(),
                gain: 1.0,
            })
            .collect();
        let index = CueIndex::build(&cues, 0.75);
        for time in (0..550).map(|i| i as f64 + 0.5) {
            let limit = cues.partition_point(|cue| cue.at_ms <= time);
            let expected: Vec<_> = (0..limit)
                .filter(|&i| cue_end(&cues[i], 0.75) > time)
                .collect();
            let mut found = Vec::new();
            index.overlaps(&cues, limit, time, 0.75, &mut found);
            assert_eq!(found, expected, "seek to {time}");
        }
    }

    #[test]
    #[ignore = "explicit offline callback timing measurement"]
    fn dense_late_loop() {
        let clip = Arc::new(AudioClip {
            samples: vec![0.1; 48],
            channels: 1,
            sample_rate: 48_000,
        });
        let mut mixer = Mixer::new(48_000);
        mixer.cues = (0..100_000)
            .map(|i| ScheduledSound {
                at_ms: i as f64,
                end_ms: None,
                clip: clip.clone(),
                gain: 1.0,
            })
            .collect();
        let cues = std::mem::take(&mut mixer.cues);
        let mut replacement = Schedule::prepare(None, cues.clone(), 1.0);
        Schedule::prepare(None, cues, 1.0).swap_into(&mut mixer);
        mixer.cycle = Some((99_990.0, 99_991.0));
        mixer.playing = true;
        let mut buffer = [0.0; 128];
        let mut times = Vec::new();
        for _ in 0..200 {
            mixer.seek(99_990.0);
            let start = std::time::Instant::now();
            mixer.render(&mut buffer, 2);
            times.push(start.elapsed());
            assert!(buffer[0] > 0.0);
        }
        times.sort();
        eprintln!(
            "100k cues, 64 frames at 48 kHz, 200 callbacks: median {:?}, max {:?}",
            times[100], times[199]
        );
        times.clear();
        for _ in 0..200 {
            let slot = OnceLock::new();
            mixer.seek(99_990.0);
            let start = std::time::Instant::now();
            let _ = slot.set(replacement.swap_into(&mut mixer));
            mixer.render(&mut buffer, 2);
            times.push(start.elapsed());
            replacement = slot.into_inner().unwrap();
        }
        times.sort();
        eprintln!(
            "Same callback with a prepared schedule replacement: median {:?}, max {:?}",
            times[100], times[199]
        );
    }
}


#[cfg(test)]
mod buffer_tests {
    use super::*;

    #[test]
    fn buffer_requests_use_frames_and_respect_reported_device_limits() {
        use cpal::{BufferSize, SupportedBufferSize};
        let limited = SupportedBufferSize::Range { min: 128, max: 512 };
        assert_eq!(
            buffer_request(OutputBackend::System, 2, 48_000, &SupportedBufferSize::Unknown),
            Ok(BufferSize::Fixed(96))
        );
        assert!(
            buffer_request(OutputBackend::Mme, 5, 48_000, &SupportedBufferSize::Unknown)
                .unwrap_err()
                .contains("10 and 500")
        );
        assert_eq!(
            buffer_request(OutputBackend::System, 10, 48_000, &limited),
            Ok(BufferSize::Fixed(480))
        );
        assert_eq!(
            buffer_request(OutputBackend::System, 11, 44_100, &SupportedBufferSize::Unknown),
            Ok(BufferSize::Fixed(485))
        );
        assert!(
            buffer_request(OutputBackend::System, 30, 48_000, &limited)
                .unwrap_err()
                .contains("1440 frames")
        );
        assert!(
            buffer_request(OutputBackend::System, 1, 48_000, &SupportedBufferSize::Unknown)
                .is_err()
        );
        assert!(
            buffer_request(OutputBackend::System, 501, 48_000, &SupportedBufferSize::Unknown)
                .is_err()
        );
    }
}
#[cfg(test)]
mod listen_tests {
    use super::*;

    fn clip(value: f32, frames: usize) -> Arc<AudioClip> {
        Arc::new(AudioClip { samples: vec![value; frames * 2], channels: 2, sample_rate: 1000 })
    }

    fn render(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        mixer.render(&mut out, 2);
        out
    }

    #[test]
    fn listen_replaces_and_stops_its_one_clip_and_leaves_auditions_alone() {
        let mut mixer = Mixer::new(1000);
        mixer.listen(Some(clip(0.25, 10)));
        assert_eq!(render(&mut mixer, 2), [0.25; 4]);
        // A second Listen replaces the first instead of mixing with it.
        mixer.listen(Some(clip(0.5, 10)));
        assert_eq!(render(&mut mixer, 1), [0.5; 2]);
        mixer.audition(clip(0.125, 10), 1.0);
        assert_eq!(render(&mut mixer, 1), [0.625; 2]);
        mixer.listen(None);
        assert_eq!(render(&mut mixer, 1), [0.125; 2]);
        // A clip that runs out ends on its own.
        mixer.listen(Some(clip(0.25, 2)));
        render(&mut mixer, 3);
        assert!(mixer.listen.is_none());
    }

    #[test]
    fn the_caller_frees_listen_clips_never_the_audio_thread() {
        let slot = ListenSlot::default();
        let first = clip(0.1, 1000);
        let freed = Arc::downgrade(&first);
        // `audio` stands for the audio thread's handle.
        let audio = slot.start(Some(first));
        assert!(slot.is_listening());
        // Replaced: the old clip waits in the slot until the audio thread lets go of it.
        let second = slot.start(Some(clip(0.2, 1000)));
        assert!(slot.collect());
        assert!(freed.upgrade().is_some());
        drop(audio);
        assert!(slot.collect(), "the second clip still plays");
        assert!(freed.upgrade().is_none(), "freed by collect, on the caller's thread");
        // A clip the audio thread let go of is also freed by the next Listen.
        let third = slot.start(Some(clip(0.3, 1000)));
        let replaced = slot.start(Some(clip(0.4, 1000)));
        drop(third);
        let last = slot.start(None);
        assert!(last.is_none());
        assert_eq!(slot.0.lock().unwrap().1.len(), 2, "the third clip is freed; the two still held wait");
        drop(replaced);
        // The audio thread finished the clip: Listen is over and the slot releases it here.
        drop(second);
        assert!(!slot.is_listening());
        assert!(!slot.collect());
        assert!(slot.start(None).is_none() && !slot.is_listening());
    }
}

#[cfg(test)]
mod peak_tests {
    use super::*;

    #[test]
    fn peaks_take_the_loudest_frame_of_each_bar_over_every_channel_scaled_to_the_loudest_bar() {
        let mono = AudioClip { samples: vec![0.0, 0.25, -0.5, 0.125, 0.0, 0.0, 0.0, 0.05], channels: 1, sample_rate: 8 };
        assert_eq!(mono.peaks(4), [0.5, 1.0, 0.0, 0.1]);
        // Stereo: frames interleave left and right; the louder channel counts.
        let stereo = AudioClip { samples: vec![0.1, -0.4, 0.2, 0.0], channels: 2, sample_rate: 8 };
        assert_eq!(stereo.peaks(2), [1.0, 0.5]);
        // More bars than frames: a bar without a frame is silent.
        assert_eq!(stereo.peaks(4), [0.0, 1.0, 0.0, 0.5]);
        let silent = AudioClip { samples: vec![0.0; 6], channels: 2, sample_rate: 8 };
        assert_eq!(silent.peaks(3), [0.0, 0.0, 0.0]);
        assert!(mono.peaks(0).is_empty());
    }
}

#[cfg(test)]
mod limiter_tests {
    use super::*;

    #[test]
    fn master_limiter_is_transparent_below_ceiling_and_prevents_overload_clipping() {
        let mut mixer = Mixer::new(1000);
        mixer.playing = true;
        // Below ceiling (0.5): output is bit-exact.
        mixer.song = Some(Arc::new(AudioClip {
            samples: vec![0.5; 20],
            channels: 2,
            sample_rate: 1000,
        }));
        let mut quiet = vec![0.0; 8];
        mixer.render(&mut quiet, 2);
        assert_eq!(quiet, vec![0.5; 8]);

        // Overload: stereo song (L=0.86, R=0.43) + click (L=0.50, R=0.25) -> L=1.36, R=0.68.
        let mut mixer = Mixer::new(1000);
        mixer.playing = true;
        mixer.song = Some(Arc::new(AudioClip {
            samples: [0.86_f32, 0.43].repeat(300),
            channels: 2,
            sample_rate: 1000,
        }));
        mixer.cues = vec![ScheduledSound {
            at_ms: 0.0,
            end_ms: None,
            clip: Arc::new(AudioClip {
                samples: [0.50_f32, 0.25].repeat(2),
                channels: 2,
                sample_rate: 1000,
            }),
            gain: 1.0,
        }];
        let mut out = vec![0.0; 600];
        mixer.render(&mut out, 2);
        // Peak frame is limited to the 0.98 ceiling and keeps the exact 2:1 stereo ratio.
        assert!((out[0] - MasterLimiter::CEILING).abs() < 1e-5, "got {}", out[0]);
        assert!((out[1] - MasterLimiter::CEILING * 0.5).abs() < 1e-5, "got {}", out[1]);
        assert!(out.iter().all(|s| s.abs() <= MasterLimiter::CEILING + 1e-5));
        // After the 2 ms click ends and ~250 ms of release passes, gain returns to unity (0.86).
        let tail_l = out[out.len() - 2];
        assert!((tail_l - 0.86).abs() < 0.01, "expected release back to 0.86, got {tail_l}");

        // Seek resets gain reduction immediately.
        let mut seek_mixer = Mixer::new(1000);
        seek_mixer.playing = true;
        seek_mixer.song = Some(Arc::new(AudioClip {
            samples: vec![1.5; 40],
            channels: 2,
            sample_rate: 1000,
        }));
        let mut hot = vec![0.0; 4];
        seek_mixer.render(&mut hot, 2);
        assert!(seek_mixer.limiter.gain < 0.7);
        seek_mixer.seek(0.0);
        assert_eq!(seek_mixer.limiter.gain, 1.0);
    }
}

#[cfg(test)]
mod streaming_stretch_tests {
    use super::*;

    fn stereo_sine(sample_rate: u32, seconds: usize, freq_hz: f32) -> Arc<AudioClip> {
        let frames = sample_rate as usize * seconds;
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = (2.0 * std::f32::consts::PI * freq_hz * i as f32 / sample_rate as f32).sin() * 0.5;
            samples.push(v);
            samples.push(v);
        }
        Arc::new(AudioClip {
            samples,
            channels: 2,
            sample_rate,
        })
    }

    #[test]
    fn mixer_streams_pitch_preserving_stretch_at_half_and_one_point_five_speed() {
        let rate = 48_000;
        let song = stereo_sine(rate, 4, 220.0);

        for speed in [0.5_f64, 1.5_f64] {
            let mut mixer = Mixer::new(rate);
            mixer.song = Some(song.clone());
            mixer.speed = speed;
            mixer.playing = true;
            mixer.seek(500.0);

            // Render 0.5 seconds of stereo output across multiple callback-sized blocks.
            let total_frames = (rate / 2) as usize;
            let mut out = vec![0.0_f32; total_frames * 2];
            for chunk in out.chunks_mut(512 * 2) {
                mixer.render(chunk, 2);
            }

            assert!(
                out.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
                "speed {speed}: output must be finite and bounded"
            );
            let peak = out.iter().fold(0.0_f32, |a, &b| a.max(b.abs()));
            assert!(
                peak > 0.1,
                "speed {speed}: expected non-silent stretched output, peak={peak}"
            );
            // Position advances by rendered_ms * speed.
            let expected_pos = 500.0 + 500.0 * speed;
            assert!(
                (mixer.position_ms - expected_pos).abs() < 1.0,
                "speed {speed}: expected pos ~{expected_pos}, got {}",
                mixer.position_ms
            );
        }
    }
}

#[cfg(test)]
mod live_source_tests {
    use super::*;

    struct ConstantLiveSource {
        left_value: f32,
        right_value: f32,
        last_sample_rate: Arc<AtomicU32>,
        last_playing: Arc<AtomicBool>,
    }

    impl LiveAudioSource for ConstantLiveSource {
        fn process_block(
            &mut self,
            left: &mut [f32],
            right: &mut [f32],
            sample_rate: u32,
            is_playing: bool,
        ) {
            self.last_sample_rate.store(sample_rate, Ordering::Relaxed);
            self.last_playing.store(is_playing, Ordering::Relaxed);
            left.fill(self.left_value);
            right.fill(self.right_value);
        }
    }

    #[test]
    fn mock_live_source_mixes_cleanly_through_master_limiter() {
        let rate_seen = Arc::new(AtomicU32::new(0));
        let playing_seen = Arc::new(AtomicBool::new(false));

        // 1. Quiet live source passes through transparently when below ceiling.
        let mut mixer = Mixer::new(48_000);
        mixer.playing = false;
        mixer.set_live_source(Some(Box::new(ConstantLiveSource {
            left_value: 0.25,
            right_value: -0.125,
            last_sample_rate: rate_seen.clone(),
            last_playing: playing_seen.clone(),
        })));
        let mut quiet = vec![0.0_f32; 8];
        mixer.render(&mut quiet, 2);
        assert_eq!(rate_seen.load(Ordering::Relaxed), 48_000);
        assert!(!playing_seen.load(Ordering::Relaxed));
        for &frame in quiet.as_chunks::<2>().0 {
            assert_eq!(frame, [0.25, -0.125]);
        }

        // 2. Live source + song overload is caught by MasterLimiter, preserving 2:1 stereo ratio.
        let mut mixer = Mixer::new(1000);
        mixer.playing = true;
        mixer.song = Some(Arc::new(AudioClip {
            samples: [0.60_f32, 0.30].repeat(64),
            channels: 2,
            sample_rate: 1000,
        }));
        mixer.set_live_source(Some(Box::new(ConstantLiveSource {
            left_value: 0.90,
            right_value: 0.45,
            last_sample_rate: rate_seen.clone(),
            last_playing: playing_seen.clone(),
        })));
        let mut out = vec![0.0_f32; 64];
        mixer.render(&mut out, 2);
        assert!(playing_seen.load(Ordering::Relaxed));
        assert!(
            (out[0] - MasterLimiter::CEILING).abs() < 1e-5,
            "expected left limited to ceiling, got {}",
            out[0]
        );
        assert!(
            (out[1] - MasterLimiter::CEILING * 0.5).abs() < 1e-5,
            "expected right to preserve 2:1 ratio, got {}",
            out[1]
        );
        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= MasterLimiter::CEILING + 1e-5));

        // 3. Non-finite samples from a live source are sanitized and do not poison output.
        let mut mixer = Mixer::new(48_000);
        mixer.set_live_source(Some(Box::new(ConstantLiveSource {
            left_value: f32::NAN,
            right_value: f32::INFINITY,
            last_sample_rate: rate_seen,
            last_playing: playing_seen,
        })));
        let mut sanitized = vec![0.0_f32; 8];
        mixer.render(&mut sanitized, 2);
        assert_eq!(sanitized, vec![0.0_f32; 8]);
    }

    #[test]
    fn stream_render_swaps_and_retires_live_source_without_audio_thread_drop() {
        let (mut render, mut handles) = build_render_parts(48_000, 2, false, None);
        let send = &mut handles.cmd_tx;
        let retire_live_rx = &mut handles.retire_live_rx;
        let telemetry = &handles.telemetry;

        assert!(
            send.try_push(Command::SetLiveSource(Some(Box::new(ConstantLiveSource {
                left_value: 0.5,
                right_value: 0.5,
                last_sample_rate: Arc::new(AtomicU32::new(0)),
                last_playing: Arc::new(AtomicBool::new(false)),
            }))))
            .is_ok()
        );

        let mut pcm = [0_i16; 8];
        render.fill(&mut pcm);
        assert!(retire_live_rx.try_pop().is_none());
        assert_eq!(f32::from_bits(telemetry.peak.load(Ordering::Relaxed)), 0.5);

        // Replacing with None hands the previous source back over the retirement ring.
        assert!(send.try_push(Command::SetLiveSource(None)).is_ok());
        render.fill(&mut pcm);
        assert!(retire_live_rx.try_pop().is_some());
        assert!(retire_live_rx.try_pop().is_none());
    }

    #[test]
    fn mixer_cycle_wrap_splits_live_block_and_resets_once() {
        struct WrapRecorder {
            resets: usize,
            was_reset: bool,
            calls: Vec<(usize, f64, bool)>,
        }
        impl LiveAudioSource for WrapRecorder {
            fn process_block(
                &mut self,
                left: &mut [f32],
                right: &mut [f32],
                _sample_rate: u32,
                _is_playing: bool,
            ) {
                left.fill(0.0);
                right.fill(0.0);
            }
            fn process_timeline_block(
                &mut self,
                left: &mut [f32],
                right: &mut [f32],
                _sample_rate: u32,
                _is_playing: bool,
                position_ms: f64,
                _speed: f64,
            ) {
                let post_reset = self.was_reset;
                self.was_reset = false;
                self.calls.push((left.len(), position_ms, post_reset));
                if post_reset {
                    // Simulate re-trigger at cycle.left sounding immediately in sub-block 2.
                    left.fill(0.5);
                    right.fill(0.5);
                } else {
                    left.fill(0.1);
                    right.fill(0.1);
                }
            }
            fn reset(&mut self) {
                self.resets += 1;
                self.was_reset = true;
            }
        }

        let rate = 48_000u32;
        let ms_per_frame = 1000.0 / f64::from(rate);
        let cycle_left = 100.0;
        let cycle_right = 500.0;
        let mut mixer = Mixer::new(rate);
        mixer.playing = true;
        mixer.limiter_enabled = false;
        mixer.cycle = Some((cycle_left, cycle_right));
        mixer.seek(cycle_right - 64.0 * ms_per_frame);
        let calls_box = Box::new(WrapRecorder {
            resets: 0,
            was_reset: false,
            calls: Vec::new(),
        });
        mixer.set_live_source(Some(calls_box));

        let mut out = vec![0.0_f32; 256 * 2];
        mixer.render(&mut out, 2);

        // Pre-wrap (frames 0..64) has 0.1; post-wrap (frames 64..256) has re-triggered 0.5 at cycle.left.
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert!((out[63 * 2] - 0.1).abs() < 1e-6);
        assert!((out[64 * 2] - 0.5).abs() < 1e-6);
        assert!((out[255 * 2] - 0.5).abs() < 1e-6);
        assert_eq!(mixer.cycle_wraps, 1);

        // Retrieve source via set_live_source(None)? Box<dyn LiveAudioSource> can't downcast directly,
        // so also inspect cycle_wraps == 1 and position_ms == cycle_left + 192 * ms_per_frame.
        let expected_pos = cycle_left + 192.0 * ms_per_frame;
        assert!(
            (mixer.position_ms - expected_pos).abs() < 1e-6,
            "expected {}, got {}",
            expected_pos,
            mixer.position_ms
        );
    }
}

#[cfg(test)]
mod decode_tests {
    use super::*;

    fn write_test_pcm16_wav(path: &Path, sample_rate: u32, channels: u16, frames: usize) {
        let data_bytes = (frames * usize::from(channels) * 2) as u32;
        let riff_size = 36 + data_bytes;
        let byte_rate = sample_rate * u32::from(channels) * 2;
        let block_align = channels * 2;
        let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_size.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_bytes.to_le_bytes());
        for frame in 0..frames {
            for ch in 0..usize::from(channels) {
                let sample: i16 = if ch == 0 {
                    ((frame % 100) as i16) * 100
                } else {
                    -((frame % 100) as i16) * 100
                };
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn decode_pre_reserve_handles_missing_normal_and_corrupted_frame_counts() {
        assert_eq!(decode_pre_reserve_samples(None, 2), 0);
        assert_eq!(decode_pre_reserve_samples(Some(0), 2), 0);
        assert_eq!(decode_pre_reserve_samples(Some(48_000 * 180), 2), 48_000 * 180 * 2);
        assert_eq!(
            decode_pre_reserve_samples(Some(u64::MAX), 2),
            MAX_DECODE_PRE_RESERVE_SAMPLES
        );
        assert_eq!(
            decode_pre_reserve_samples(Some(100_000_000), 4),
            MAX_DECODE_PRE_RESERVE_SAMPLES
        );
    }

    #[test]
    fn decode_with_cancel_pre_reserves_and_decodes_pcm_wav() {
        let frames = 10_000;
        let channels = 2;
        let path = std::env::temp_dir().join(format!(
            "dawsu_audio_g9_decode_{}.wav",
            std::process::id()
        ));
        write_test_pcm16_wav(&path, 44_100, channels as u16, frames);
        let clip = decode_with_cancel(&path, &|| false);
        let _ = std::fs::remove_file(&path);
        let clip = clip.unwrap();
        assert_eq!(clip.channels, channels);
        assert_eq!(clip.sample_rate, 44_100);
        assert_eq!(clip.samples.len(), frames * channels);
        assert_eq!(clip.samples.capacity(), frames * channels);
    }

    #[test]
    fn keep_pitch_toggle_switches_between_stretched_and_direct_playback() {
        let rate = 48_000u32;
        let frames = 48_000usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin() * 0.5;
            samples.push(v);
            samples.push(v);
        }
        let song = Arc::new(AudioClip {
            samples,
            channels: 2,
            sample_rate: rate,
        });

        let mut direct = Mixer::new(rate);
        direct.song = Some(song.clone());
        direct.playing = true;
        direct.speed = 0.5;
        direct.keep_pitch = false;
        let mut buf_direct = vec![0.0_f32; 4096 * 2];
        direct.render(&mut buf_direct, 2);

        let mut pitched = Mixer::new(rate);
        pitched.song = Some(song);
        pitched.playing = true;
        pitched.speed = 0.5;
        pitched.keep_pitch = true;
        let mut buf_pitched = vec![0.0_f32; 4096 * 2];
        pitched.render(&mut buf_pitched, 2);

        // Zero-crossing count on direct 0.5x playback should be roughly half of pitch-preserved 0.5x.
        let crossings = |buf: &[f32]| -> usize {
            buf.as_chunks::<2>()
                .0
                .iter()
                .skip(512)
                .map(|f| f[0])
                .collect::<Vec<_>>()
                .windows(2)
                .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
                .count()
        };
        let zc_direct = crossings(&buf_direct);
        let zc_pitched = crossings(&buf_pitched);
        assert!(
            zc_pitched > zc_direct + 8,
            "keep_pitch=true ({zc_pitched} cycles) should preserve ~440Hz vs direct 0.5x ({zc_direct} cycles)"
        );
    }

    #[test]
    fn decode_prefix_stops_at_max_frames_on_sixty_second_wav() {
        let rate = 48_000u32;
        let channels = 2u16;
        let frames = rate as usize * 10; // 10 s WAV (~1.9 MB) capped cleanly at 72,000 frames (1.5 s)
        let path = std::env::temp_dir().join(format!(
            "dawsu_audio_prefix_{}.wav",
            std::process::id()
        ));
        write_test_pcm16_wav(&path, rate, channels, frames);
        let clip = decode_prefix(&path, 72_000).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(clip.sample_rate, rate);
        assert_eq!(clip.channels, usize::from(channels));
        assert_eq!(clip.samples.len() / clip.channels, 72_000);
    }

    #[test]
    fn spectrum_tap_zero_pushes_when_disabled_and_phase_corr_negative_one_on_inverted_tone() {
        struct InvertedTone;
        impl LiveAudioSource for InvertedTone {
            fn process_block(&mut self, left: &mut [f32], right: &mut [f32], _rate: u32, _playing: bool) {
                for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
                    let s = (i as f32 * 0.1).sin() * 0.5;
                    *l = s;
                    *r = -s;
                }
            }
        }
        let mut harness = RtBenchHarness::new(48_000, 2);
        harness.set_live_source(Some(Box::new(InvertedTone)));
        harness.play(true);
        let mut out = vec![0.0_f32; 512 * 2];
        for _ in 0..4 {
            harness.fill(&mut out);
        }
        assert_eq!(harness.spectrum_tap().push_count(), 0);
        assert!(harness.phase_correlation() <= -0.99, "phase={}", harness.phase_correlation());

        harness.set_spectrum_tap_enabled(true);
        harness.fill(&mut out);
        assert_eq!(harness.spectrum_tap().push_count(), 512);
        let mut drained = Vec::new();
        let (n, sr) = harness.spectrum_tap().drain_into(&mut drained, 1024);
        assert_eq!((n, sr), (512, 48_000));
        // Inverted L + (-L) sums cleanly to zero mono signal.
        assert!(drained.iter().all(|v| v.abs() < 1e-6));
    }
}
