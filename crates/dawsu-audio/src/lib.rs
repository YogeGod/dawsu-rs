//! Audio decoding and playback; no editor or map types.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

#[cfg(windows)]
mod directsound;
#[cfg(windows)]
mod mme;

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
pub fn decode(path: &Path) -> Result<Arc<AudioClip>, String> {
    decode_with_cancel(path, &|| false)
}
pub fn decode_with_cancel(
    path: &Path,
    cancel: &dyn Fn() -> bool,
) -> Result<Arc<AudioClip>, String> {
    let source = File::open(path).map_err(|e| format!("Opening {}: {e}", path.display()))?;
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }
    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            MediaSourceStream::new(Box::new(source), Default::default()),
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
    while let Some(packet) = reader.next_packet().map_err(|e| e.to_string())? {
        if cancel() {
            return Err("Audio decoding cancelled".into());
        }
        if packet.track_id != track.id {
            continue;
        }
        let buffer = decoder
            .decode(&packet)
            .map_err(|e| format!("Decoding {}: {e}", path.display()))?;
        let old = samples.len();
        samples.resize(old + buffer.samples_interleaved(), 0.0);
        buffer.copy_to_slice_interleaved(&mut samples[old..]);
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

struct SongStretcher {
    sample_rate: u32,
    stretch: Option<signalsmith_stretch::Stretch>,
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
        Self {
            sample_rate: sample_rate.max(1),
            stretch: None,
            in_buf: Vec::new(),
            out_buf: vec![0.0; Self::CHUNK_OUT * 2],
            discard_buf: Vec::new(),
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

    fn fill_input(&mut self, song: &AudioClip, start_frame: f64, count: usize) {
        let needed = count * 2;
        if self.in_buf.len() < needed {
            self.in_buf.resize(needed, 0.0);
        }
        let ratio = song.sample_rate as f64 / self.sample_rate as f64;
        for i in 0..count {
            let src_frame = (start_frame + i as f64) * ratio;
            self.in_buf[i * 2] = song.sample(src_frame, 0);
            self.in_buf[i * 2 + 1] = song.sample(src_frame, 1);
        }
    }

    fn prime(&mut self, song: &AudioClip, song_time_ms: f64, speed: f64) {
        let sample_rate = self.sample_rate;
        let stretch = self
            .stretch
            .get_or_insert_with(|| signalsmith_stretch::Stretch::preset_default(2, sample_rate));
        stretch.reset();
        let in_lat = stretch.input_latency();
        let out_lat = stretch.output_latency();

        let base_frame = song_time_ms * sample_rate as f64 / 1000.0;
        let start_in = base_frame - out_lat as f64 * speed;

        if in_lat > 0 {
            self.fill_input(song, start_in, in_lat);
            if let Some(stretch) = self.stretch.as_mut() {
                stretch.seek(&self.in_buf[..in_lat * 2], speed);
            }
        }
        self.read_frame = start_in + in_lat as f64;

        if out_lat > 0 {
            let discard_in = (out_lat as f64 * speed).round() as usize;
            self.fill_input(song, self.read_frame, discard_in);
            let discard_samples = out_lat * 2;
            if self.discard_buf.len() < discard_samples {
                self.discard_buf.resize(discard_samples, 0.0);
            }
            if let Some(stretch) = self.stretch.as_mut() {
                stretch.process(
                    &self.in_buf[..discard_in * 2],
                    &mut self.discard_buf[..discard_samples],
                );
            }
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
            let in_frames = in_exact.floor() as usize;
            self.in_frac = in_exact - in_frames as f64;
            self.fill_input(song, self.read_frame, in_frames);
            self.read_frame += in_frames as f64;
            if let Some(stretch) = self.stretch.as_mut() {
                stretch.process(
                    &self.in_buf[..in_frames * 2],
                    &mut self.out_buf[..Self::CHUNK_OUT * 2],
                );
            }
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
    pub master_gain: f32,
    pub song_offset: f64,
    pub cycle: Option<(f64, f64)>,
    pub speed: f64,
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
            master_gain: 1.0,
            song_offset: 0.0,
            cycle: None,
            speed: 1.0,
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
        self.position_ms = if ms.is_finite() { ms.max(0.0) } else { 0.0 };
        self.indexed = false;
        self.limiter.reset();
        self.stretcher.reset();
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
        let block_samples = Self::LIVE_BLOCK_FRAMES * out_channels;
        for block in out.chunks_mut(block_samples) {
            let block_frames = block.len() / out_channels;
            let has_live = if block_frames > 0 {
                if let Some(source) = self.live_source.as_mut() {
                    let left = &mut self.live_left[..block_frames];
                    let right = &mut self.live_right[..block_frames];
                    left.fill(0.0);
                    right.fill(0.0);
                    source.process_block(left, right, self.sample_rate, self.playing);
                    true
                } else {
                    false
                }
            } else {
                false
            };
            for (frame_idx, frame) in block.chunks_mut(out_channels).enumerate() {
                if let Some((left, right)) = self.cycle {
                    if self.playing && right > left && self.position_ms >= right {
                        self.seek(left);
                    }
                }
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
                    let speed = self.speed;
                    self.active_cues
                        .retain(|i| position < cue_end(&cues[*i], speed));
                }
                let speed = if self.speed.is_finite() && self.speed > 0.0 {
                    self.speed
                } else {
                    1.0
                };
                let stretched_song_frame = if self.playing && (speed - 1.0).abs() > 1e-6 {
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
                for (channel, value) in frame.iter_mut().enumerate() {
                    let mut mix = 0.0;
                    if self.playing {
                        if let Some([l, r]) = stretched_song_frame {
                            let sample = if out_channels == 1 {
                                (l + r) * 0.5
                            } else if channel == 0 {
                                l
                            } else {
                                r
                            };
                            mix += sample * self.song_gain;
                        } else if let Some(song) = &self.song {
                            mix += song.sample(
                                (self.position_ms - self.song_offset)
                                    * song.sample_rate as f64
                                    / 1000.0,
                                channel,
                            ) * self.song_gain;
                        }
                        // Cues read by source position, so seek and cycle cannot double-trigger or skip a cue.
                        for index in &self.active_cues {
                            let cue = &self.cues[*index];
                            let elapsed = self.position_ms - cue.at_ms;
                            if elapsed < 0.0 {
                                continue;
                            }
                            let mut source =
                                elapsed / self.speed * cue.clip.sample_rate as f64 / 1000.0;
                            if let Some(end) = cue.end_ms {
                                if self.position_ms >= end {
                                    continue;
                                }
                                let len = cue.clip.samples.len() / cue.clip.channels;
                                if len > 0 {
                                    source %= len as f64;
                                }
                            }
                            mix += cue.clip.sample(source, channel) * cue.gain;
                        }
                    }
                    for voice in self.voices.iter().chain(&self.listen) {
                        mix += voice.clip.sample(voice.frame, channel) * voice.gain;
                    }
                    if has_live && frame_idx < block_frames {
                        let live_l = sanitize_live_sample(self.live_left[frame_idx]);
                        let live_r = sanitize_live_sample(self.live_right[frame_idx]);
                        mix += if out_channels == 1 {
                            (live_l + live_r) * 0.5
                        } else if channel == 0 {
                            live_l
                        } else if channel == 1 {
                            live_r
                        } else {
                            0.0
                        };
                    }
                    *value = mix * self.master_gain;
                }
                self.limiter.process_frame(frame);
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
#[derive(Default)]
struct ListenSlot(Mutex<(Option<Arc<AudioClip>>, Vec<Arc<AudioClip>>)>);

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

const MAX_COMMANDS_PER_BUFFER: usize = 16;

fn validate_buffer_ms(ms: u32) -> Result<(), String> {
    if (10..=500).contains(&ms) {
        Ok(())
    } else {
        Err("Enter an audio buffer between 10 and 500 ms.".into())
    }
}

fn buffer_request(
    ms: u32,
    rate: cpal::SampleRate,
    supported: &cpal::SupportedBufferSize,
) -> Result<cpal::BufferSize, String> {
    validate_buffer_ms(ms)?;
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

enum Command {
    Load(Schedule, Retired),
    SetLiveSource(Option<Box<dyn LiveAudioSource>>),
    Play(bool),
    Seek(f64),
    Audition(Arc<AudioClip>, f32),
    Listen(Option<Arc<AudioClip>>),
    Gains(f32, f32),
    Cycle(Option<(f64, f64)>),
    Offset(f64),
}
pub struct AudioEngine {
    _stream: OutputStream,
    backend: OutputBackend,
    effective_buffer_ms: Option<u32>,
    send: mpsc::Sender<Command>,
    retirements: Mutex<Vec<Retired>>,
    retired_live_sources: Mutex<mpsc::Receiver<Box<dyn LiveAudioSource>>>,
    listen: ListenSlot,
    position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    pub errors: Arc<AtomicU64>,
    pub peak: Arc<AtomicU32>,
    pub device_name: String,
    device_id: Option<String>,
}
impl AudioEngine {
    pub fn open(device_name: Option<&str>) -> Result<Self, String> {
        Self::open_with_buffer(device_name, None)
    }

    /// Requests a device buffer in milliseconds. The host chooses the actual callback size.
    pub fn open_with_buffer(device_name: Option<&str>, buffer_ms: Option<u32>) -> Result<Self, String> {
        Self::open_cpal(
            OutputBackend::System,
            cpal::default_host(),
            device_name,
            None,
            buffer_ms,
        )
    }

    pub fn open_output(
        backend: OutputBackend, device_id: Option<&str>, buffer_ms: Option<u32>,
    ) -> Result<Self, String> {
        if backend == OutputBackend::System {
            return Self::open_cpal(backend, cpal::default_host(), None, device_id, buffer_ms);
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
                validate_buffer_ms(ms)?;
            }
            let (send, receive) = mpsc::channel();
            let (retire_live_tx, retire_live_rx) = mpsc::sync_channel(16);
            let position = Arc::new(AtomicU64::new(0));
            let playing = Arc::new(AtomicBool::new(false));
            let errors = Arc::new(AtomicU64::new(0));
            let peak = Arc::new(AtomicU32::new(0));
            let mut state = StreamRender::new(NATIVE_SAMPLE_RATE, 2, receive, retire_live_tx,
                position.clone(), playing.clone(), peak.clone());
            let render = move |out: &mut [i16]| state.fill(out);
            let stream = match backend {
                OutputBackend::Mme => mme::open(device_id, buffer_ms, render, errors.clone()),
                OutputBackend::DirectSound => directsound::open(device_id, buffer_ms, render, errors.clone()),
                OutputBackend::System | OutputBackend::Asio => unreachable!("CPAL output opened above"),
            }?;
            return Ok(Self {
                backend,
                effective_buffer_ms: Some(stream.buffer_ms),
                device_name: stream.name.clone(),
                device_id: device_id.map(str::to_owned),
                _stream: OutputStream::Native { _stream: stream },
                send, position, playing, errors, peak,
                retirements: Mutex::new(Vec::new()),
                retired_live_sources: Mutex::new(retire_live_rx),
                listen: ListenSlot::default(),
            });
        }
        #[cfg(not(windows))]
        Err(format!("{} output is available only on Windows", backend.label()))
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

    pub fn effective_buffer_ms(&self) -> Option<u32> {
        self.effective_buffer_ms
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
            validate_buffer_ms(ms)?;
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
        if let Some(ms) = buffer_ms {
            config.buffer_size = buffer_request(ms, config.sample_rate, supported.buffer_size())?;
        }
        let (send, receive) = mpsc::channel();
        let (retire_live_tx, retire_live_rx) = mpsc::sync_channel(16);
        let position = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU32::new(0));
        let render = StreamRender::new(
            config.sample_rate, config.channels as usize, receive, retire_live_tx,
            position.clone(), playing.clone(), peak.clone(),
        );
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
            effective_buffer_ms: None,
            retirements: Mutex::new(Vec::new()),
            retired_live_sources: Mutex::new(retire_live_rx),
            listen: ListenSlot::default(),
            send,
            position,
            playing,
            errors,
            peak,
            device_name: name,
            device_id,
        })
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
        let _ = self.send.send(Command::Load(schedule, slot));
    }
    /// Release completed schedules on the caller thread; request another poll
    /// if a callback has yet to hand one back.
    pub fn collect_retired(&self) -> bool {
        let mut slots = self.retirements.lock().unwrap_or_else(|e| e.into_inner());
        let live_rx = self
            .retired_live_sources
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while live_rx.try_recv().is_ok() {}
        let listening = self.listen.collect();
        collect_retired(&mut slots) || listening
    }

    /// Installs or replaces the live audio source mixed ahead of the master limiter.
    ///
    /// The previous source is sent back over a bounded non-blocking channel so it is
    /// dropped on the caller's thread rather than the real-time audio thread.
    pub fn set_live_source(&self, source: Option<Box<dyn LiveAudioSource>>) {
        let live_rx = self
            .retired_live_sources
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while live_rx.try_recv().is_ok() {}
        drop(live_rx);
        let _ = self.send.send(Command::SetLiveSource(source));
    }

    /// Detaches any active live audio source.
    pub fn clear_live_source(&self) {
        self.set_live_source(None);
    }

    /// Plays `clip` in place of the current Listen clip; `None` stops Listen.
    pub fn listen(&self, clip: Option<Arc<AudioClip>>) {
        let clip = self.listen.start(clip);
        let _ = self.send.send(Command::Listen(clip));
    }
    /// Whether a Listen clip is still playing.
    pub fn is_listening(&self) -> bool {
        self.listen.is_listening()
    }

    pub fn play(&self, on: bool) {
        let _ = self.send.send(Command::Play(on));
    }
    pub fn seek(&self, ms: f64) {
        let _ = self.send.send(Command::Seek(ms));
    }
    pub fn audition(&self, clip: Arc<AudioClip>, gain: f32) {
        let _ = self.send.send(Command::Audition(clip, gain));
    }
    pub fn gains(&self, song: f32, master: f32) {
        let _ = self
            .send
            .send(Command::Gains(song.clamp(0.0, 1.0), master.clamp(0.2, 1.0)));
    }
    pub fn cycle(&self, range: Option<(f64, f64)>) {
        let _ = self.send.send(Command::Cycle(range));
    }
    pub fn offset(&self, ms: f64) {
        let _ = self.send.send(Command::Offset(ms));
    }
    pub fn position_ms(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Relaxed))
    }
    pub fn is_playing(&self) -> bool {
        #[cfg(windows)]
        if let OutputStream::Native { _stream: stream } = &self._stream
            && stream.thread.as_ref().is_some_and(|thread| thread.is_finished())
        {
            return false;
        }
        self.playing.load(Ordering::Relaxed)
    }
}
struct StreamRender {
    mixer: Mixer,
    receive: mpsc::Receiver<Command>,
    retire_live: mpsc::SyncSender<Box<dyn LiveAudioSource>>,
    position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    peak: Arc<AtomicU32>,
    channels: usize,
    scratch: Vec<f32>,
}

impl StreamRender {
    fn new(
        rate: u32,
        channels: usize,
        receive: mpsc::Receiver<Command>,
        retire_live: mpsc::SyncSender<Box<dyn LiveAudioSource>>,
        position: Arc<AtomicU64>,
        playing: Arc<AtomicBool>,
        peak: Arc<AtomicU32>,
    ) -> Self {
        // Reused, frame-aligned scratch space. Callbacks never allocate sample buffers.
        let samples = (262144 / channels).max(1) * channels;
        Self {
            mixer: Mixer::new(rate),
            receive,
            retire_live,
            position,
            playing,
            peak,
            channels,
            scratch: vec![0.0; samples],
        }
    }

    fn fill<T: cpal::SizedSample + cpal::FromSample<f32>>(&mut self, out: &mut [T]) {
        for command in self.receive.try_iter().take(MAX_COMMANDS_PER_BUFFER) {
            match command {
                Command::Load(schedule, slot) => {
                    let old = schedule.swap_into(&mut self.mixer);
                    let _ = slot.set(old);
                }
                Command::SetLiveSource(source) => {
                    if let Some(old) = self.mixer.set_live_source(source) {
                        let _ = self.retire_live.try_send(old);
                    }
                }
                Command::Play(on) => self.mixer.playing = on,
                Command::Seek(ms) => self.mixer.seek(ms),
                Command::Audition(clip, gain) => self.mixer.audition(clip, gain),
                Command::Listen(clip) => self.mixer.listen(clip),
                Command::Gains(song, master) => {
                    self.mixer.song_gain = song;
                    self.mixer.master_gain = master;
                }
                Command::Cycle(range) => self.mixer.cycle = range,
                Command::Offset(ms) => {
                    self.mixer.song_offset = ms;
                    self.mixer.stretcher.reset();
                }
            }
        }
        let mut highest = 0.0_f32;
        for chunk in out.chunks_mut(self.scratch.len()) {
            let buffer = &mut self.scratch[..chunk.len()];
            self.mixer.render(buffer, self.channels);
            for (sample, value) in chunk.iter_mut().zip(buffer.iter()) {
                *sample = T::from_sample(*value);
                highest = highest.max(value.abs());
            }
        }
        self.peak.store(highest.to_bits(), Ordering::Relaxed);
        self.position.store(self.mixer.position_ms.to_bits(), Ordering::Relaxed);
        self.playing.store(self.mixer.playing, Ordering::Relaxed);
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
        let (send, receive) = mpsc::channel();
        let (retire_live_tx, _retire_live_rx) = mpsc::sync_channel(16);
        let position = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU32::new(0));
        let mut render = StreamRender::new(48_000, 2, receive, retire_live_tx, position.clone(), playing, peak.clone());
        send.send(Command::Seek(400.0)).unwrap();
        send.send(Command::Audition(Arc::new(AudioClip {
            samples: vec![0.25; 2], channels: 1, sample_rate: 48_000,
        }), 1.0)).unwrap();
        let mut pcm = [0_i16; 4];
        render.fill(&mut pcm);
        assert_eq!(pcm, [8192; 4]);
        assert_eq!(f32::from_bits(peak.load(Ordering::Relaxed)), 0.25);
        assert_eq!(f64::from_bits(position.load(Ordering::Relaxed)), 400.0);
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
                assert_eq!(audio.peak.load(Ordering::Relaxed), 0);
                audio.play(false);
                wait_for(|| !audio.is_playing());
                assert_eq!(audio.errors.load(Ordering::Relaxed), 0, "{} stream errored", backend.label());
            }
        }
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
        assert_eq!(buffer_request(10, 48_000, &limited), Ok(BufferSize::Fixed(480)));
        assert_eq!(buffer_request(11, 44_100, &SupportedBufferSize::Unknown), Ok(BufferSize::Fixed(485)));
        assert!(buffer_request(30, 48_000, &limited).unwrap_err().contains("1440 frames"));
        assert!(buffer_request(0, 48_000, &SupportedBufferSize::Unknown).is_err());
        assert!(buffer_request(501, 48_000, &SupportedBufferSize::Unknown).is_err());
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
        for frame in quiet.chunks_exact(2) {
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
        let (send, receive) = mpsc::channel();
        let (retire_live_tx, retire_live_rx) = mpsc::sync_channel(16);
        let position = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU32::new(0));
        let mut render = StreamRender::new(
            48_000,
            2,
            receive,
            retire_live_tx,
            position,
            playing,
            peak.clone(),
        );

        send.send(Command::SetLiveSource(Some(Box::new(ConstantLiveSource {
            left_value: 0.5,
            right_value: 0.5,
            last_sample_rate: Arc::new(AtomicU32::new(0)),
            last_playing: Arc::new(AtomicBool::new(false)),
        }))))
        .unwrap();

        let mut pcm = [0_i16; 8];
        render.fill(&mut pcm);
        assert!(retire_live_rx.try_recv().is_err());
        assert_eq!(f32::from_bits(peak.load(Ordering::Relaxed)), 0.5);

        // Replacing with None hands the previous source back over the retirement channel.
        send.send(Command::SetLiveSource(None)).unwrap();
        render.fill(&mut pcm);
        assert!(retire_live_rx.try_recv().is_ok());
        assert!(retire_live_rx.try_recv().is_err());
    }
}
