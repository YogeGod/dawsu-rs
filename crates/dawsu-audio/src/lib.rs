//! Audio decoding and playback; no editor or map types.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

#[derive(Debug, Clone)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub sample_rate: u32,
}
impl AudioClip {
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
    active_cues: Vec<usize>,
    next_cue: usize,
    indexed: bool,
}
impl Mixer {
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
            active_cues: Vec::with_capacity(256),
            next_cue: 0,
            indexed: false,
        }
    }
    pub fn seek(&mut self, ms: f64) {
        self.position_ms = if ms.is_finite() { ms.max(0.0) } else { 0.0 };
        self.indexed = false;
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
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        for frame in out.chunks_mut(channels.max(1)) {
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
                    self.active_cues.extend(
                        (0..self.next_cue)
                            .filter(|i| self.position_ms < cue_end(&self.cues[*i], self.speed)),
                    );
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
            for (channel, value) in frame.iter_mut().enumerate() {
                let mut mix = 0.0;
                if self.playing {
                    if let Some(song) = &self.song {
                        mix += song.sample(
                            (self.position_ms - self.song_offset) / self.speed
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
                for voice in &self.voices {
                    mix += voice.clip.sample(voice.frame, channel) * voice.gain;
                }
                *value = (mix * self.master_gain).clamp(-1.0, 1.0);
            }
            for voice in &mut self.voices {
                voice.frame += voice.clip.sample_rate as f64 / self.sample_rate as f64;
            }
            self.voices
                .retain(|v| v.frame < (v.clip.samples.len() / v.clip.channels) as f64);
            if self.playing {
                self.position_ms += self.speed * 1000.0 / self.sample_rate as f64;
            }
        }
    }
}

enum Command {
    Load(Option<Arc<AudioClip>>, Vec<ScheduledSound>, Vec<usize>, f64),
    Play(bool),
    Seek(f64),
    Audition(Arc<AudioClip>, f32),
    Gains(f32, f32),
    Cycle(Option<(f64, f64)>),
    Offset(f64),
}
pub struct AudioEngine {
    _stream: cpal::Stream,
    send: mpsc::Sender<Command>,
    position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    pub errors: Arc<AtomicU64>,
    pub peak: Arc<AtomicU32>,
    pub device_name: String,
}
impl AudioEngine {
    pub fn open(device_name: Option<&str>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if let Some(name) = device_name {
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
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let (send, receive) = mpsc::channel();
        let position = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let errors = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU32::new(0));
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream::<f32>(
                &device,
                config,
                receive,
                position.clone(),
                playing.clone(),
                errors.clone(),
                peak.clone(),
            ),
            cpal::SampleFormat::I16 => stream::<i16>(
                &device,
                config,
                receive,
                position.clone(),
                playing.clone(),
                errors.clone(),
                peak.clone(),
            ),
            cpal::SampleFormat::U16 => stream::<u16>(
                &device,
                config,
                receive,
                position.clone(),
                playing.clone(),
                errors.clone(),
                peak.clone(),
            ),
            other => return Err(format!("Unsupported output sample format: {other:?}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            send,
            position,
            playing,
            errors,
            peak,
            device_name: name,
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
        mut cues: Vec<ScheduledSound>,
        speed: f64,
    ) {
        cues.sort_by(|a, b| a.at_ms.total_cmp(&b.at_ms));
        let active = Vec::with_capacity(cues.len());
        let _ = self.send.send(Command::Load(song, cues, active, speed));
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
        self.playing.load(Ordering::Relaxed)
    }
}
fn stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    receive: mpsc::Receiver<Command>,
    position: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    errors: Arc<AtomicU64>,
    peak: Arc<AtomicU32>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let mut mixer = Mixer::new(config.sample_rate);
    // Reused scratch space: output callbacks never allocate sample buffers.
    let mut scratch = vec![0.0f32; 262144];
    device
        .build_output_stream::<T, _, _>(
            config,
            move |out, _| {
                while let Ok(command) = receive.try_recv() {
                    match command {
                        Command::Load(song, cues, active, speed) => {
                            mixer.song = song;
                            mixer.cues = cues;
                            mixer.active_cues = active;
                            mixer.speed = speed;
                            mixer.indexed = false;
                        }
                        Command::Play(on) => mixer.playing = on,
                        Command::Seek(ms) => mixer.seek(ms),
                        Command::Audition(clip, gain) => mixer.audition(clip, gain),
                        Command::Gains(song, master) => {
                            mixer.song_gain = song;
                            mixer.master_gain = master;
                        }
                        Command::Cycle(range) => mixer.cycle = range,
                        Command::Offset(ms) => mixer.song_offset = ms,
                    }
                }
                for chunk in out.chunks_mut(scratch.len()) {
                    let buffer = &mut scratch[..chunk.len()];
                    mixer.render(buffer, channels);
                    let mut highest = 0.0f32;
                    for (sample, v) in chunk.iter_mut().zip(buffer.iter()) {
                        *sample = T::from_sample(*v);
                        highest = highest.max(v.abs());
                    }
                    peak.store(highest.to_bits(), Ordering::Relaxed);
                }
                position.store(mixer.position_ms.to_bits(), Ordering::Relaxed);
                playing.store(mixer.playing, Ordering::Relaxed);
            },
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
