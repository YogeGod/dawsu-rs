//! Pure-Rust SFZ 1.0/2.0 sampler parser and polyphonic audio renderer.
//!
//! Supports `<control>`, `<global>`, `<group>`, `<region>`, `default_path`, `sample`,
//! `key`/`lokey`/`hikey`/`pitch_keycenter`, `lovel`/`hivel`, `volume`, `pan`, `tune`,
//! `transpose`, `ampeg_attack`, `ampeg_release`, `group`/`off_by` choke groups, and
//! `loop_mode`/`loop_start`/`loop_end`. Zero external dependencies.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One musical note to render through an SFZ or SF2 sampler instrument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SamplerNote {
    pub start_ms: f64,
    pub duration_ms: f64,
    pub pitch: u8,
    pub velocity: u8,
}

/// Decoded audio buffer fed into the sampler voice renderer (mono or stereo f32).
#[derive(Debug, Clone)]
pub struct SamplerClip {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

/// Interleaved stereo `f32` render output from an SFZ or SF2 instrument.
#[derive(Debug, Clone)]
pub struct SamplerAudio {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
}

/// Metadata summary exposed to the UI when inspecting or loading an SFZ or SF2 file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SamplerSummary {
    pub format: String,
    pub name: String,
    pub path: String,
    pub region_count: usize,
    pub key_low: Option<u8>,
    pub key_high: Option<u8>,
    pub presets: Vec<String>,
}

/// One parsed SFZ region with inherited `<global>` and `<group>` opcodes resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct SfzRegion {
    pub sample_path: PathBuf,
    pub lokey: u8,
    pub hikey: u8,
    pub pitch_keycenter: u8,
    pub lovel: u8,
    pub hivel: u8,
    pub volume_db: f32,
    pub pan: f32,
    pub tune_cents: f32,
    pub transpose: i8,
    pub ampeg_attack: f32,
    pub ampeg_release: f32,
    pub group: Option<u32>,
    pub off_by: Option<u32>,
    pub loop_continuous: bool,
    pub loop_start: Option<usize>,
    pub loop_end: Option<usize>,
}

impl SfzRegion {
    pub fn matches(&self, pitch: u8, velocity: u8) -> bool {
        pitch >= self.lokey
            && pitch <= self.hikey
            && velocity >= self.lovel
            && velocity <= self.hivel
    }
}

/// Parsed SFZ instrument definition ready for offline or live-preview rendering.
#[derive(Debug, Clone)]
pub struct SfzInstrument {
    pub name: String,
    pub source_path: PathBuf,
    pub regions: Vec<SfzRegion>,
}

#[derive(Debug, Clone)]
struct OpcodeScope {
    lokey: u8,
    hikey: u8,
    pitch_keycenter: Option<u8>,
    lovel: u8,
    hivel: u8,
    volume_db: f32,
    pan: f32,
    tune_cents: f32,
    transpose: i8,
    ampeg_attack: f32,
    ampeg_release: f32,
    group: Option<u32>,
    off_by: Option<u32>,
    loop_continuous: bool,
    loop_start: Option<usize>,
    loop_end: Option<usize>,
    sample: Option<String>,
}

impl Default for OpcodeScope {
    fn default() -> Self {
        Self {
            lokey: 0,
            hikey: 127,
            pitch_keycenter: None,
            lovel: 0,
            hivel: 127,
            volume_db: 0.0,
            pan: 0.0,
            tune_cents: 0.0,
            transpose: 0,
            ampeg_attack: 0.002,
            ampeg_release: 0.08,
            group: None,
            off_by: None,
            loop_continuous: false,
            loop_start: None,
            loop_end: None,
            sample: None,
        }
    }
}

impl OpcodeScope {
    fn apply(&mut self, key: &str, value: &str) {
        let val = value.trim();
        match key {
            "sample" => {
                if !val.is_empty() {
                    self.sample = Some(val.to_string());
                }
            }
            "key" => {
                if let Some(n) = parse_sfz_note(val) {
                    self.lokey = n;
                    self.hikey = n;
                    self.pitch_keycenter = Some(n);
                }
            }
            "lokey" => {
                if let Some(n) = parse_sfz_note(val) {
                    self.lokey = n;
                }
            }
            "hikey" => {
                if let Some(n) = parse_sfz_note(val) {
                    self.hikey = n;
                }
            }
            "pitch_keycenter" => {
                if let Some(n) = parse_sfz_note(val) {
                    self.pitch_keycenter = Some(n);
                }
            }
            "lovel" => {
                if let Ok(v) = val.parse::<u8>() {
                    self.lovel = v.min(127);
                }
            }
            "hivel" => {
                if let Ok(v) = val.parse::<u8>() {
                    self.hivel = v.min(127);
                }
            }
            "volume" => {
                if let Ok(v) = val.parse::<f32>()
                    && v.is_finite()
                {
                    self.volume_db = v.clamp(-96.0, 24.0);
                }
            }
            "pan" => {
                if let Ok(v) = val.parse::<f32>()
                    && v.is_finite()
                {
                    self.pan = v.clamp(-100.0, 100.0);
                }
            }
            "tune" => {
                if let Ok(v) = val.parse::<f32>()
                    && v.is_finite()
                {
                    self.tune_cents = v.clamp(-1200.0, 1200.0);
                }
            }
            "transpose" => {
                if let Ok(v) = val.parse::<i8>() {
                    self.transpose = v.clamp(-60, 60);
                }
            }
            "ampeg_attack" => {
                if let Ok(v) = val.parse::<f32>()
                    && v.is_finite()
                {
                    self.ampeg_attack = v.clamp(0.0, 10.0);
                }
            }
            "ampeg_release" => {
                if let Ok(v) = val.parse::<f32>()
                    && v.is_finite()
                {
                    self.ampeg_release = v.clamp(0.001, 10.0);
                }
            }
            "group" => {
                if let Ok(v) = val.parse::<u32>() {
                    self.group = Some(v);
                }
            }
            "off_by" => {
                if let Ok(v) = val.parse::<u32>() {
                    self.off_by = Some(v);
                }
            }
            "loop_mode" => {
                self.loop_continuous = matches!(
                    val.to_ascii_lowercase().as_str(),
                    "loop_continuous" | "loop_sustain"
                );
            }
            "loop_start" => {
                if let Ok(v) = val.parse::<usize>() {
                    self.loop_start = Some(v);
                }
            }
            "loop_end" => {
                if let Ok(v) = val.parse::<usize>() {
                    self.loop_end = Some(v);
                }
            }
            _ => {}
        }
    }

    fn finish_region(&self, base_dir: &Path, default_path: &str) -> Option<SfzRegion> {
        let raw_sample = self.sample.as_deref()?;
        let normalized_default = default_path.replace('\\', "/");
        let normalized_sample = raw_sample.replace('\\', "/");
        let combined = if normalized_default.is_empty() {
            normalized_sample
        } else if normalized_default.ends_with('/') {
            format!("{normalized_default}{normalized_sample}")
        } else {
            format!("{normalized_default}/{normalized_sample}")
        };
        let sample_path = resolve_sfz_sample_path(base_dir, &combined);
        let pitch_keycenter = self.pitch_keycenter.unwrap_or(self.lokey);
        Some(SfzRegion {
            sample_path,
            lokey: self.lokey.min(self.hikey),
            hikey: self.hikey.max(self.lokey),
            pitch_keycenter,
            lovel: self.lovel.min(self.hivel),
            hivel: self.hivel.max(self.lovel),
            volume_db: self.volume_db,
            pan: self.pan,
            tune_cents: self.tune_cents,
            transpose: self.transpose,
            ampeg_attack: self.ampeg_attack,
            ampeg_release: self.ampeg_release,
            group: self.group,
            off_by: self.off_by,
            loop_continuous: self.loop_continuous,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
        })
    }
}

fn resolve_sfz_sample_path(base_dir: &Path, relative: &str) -> PathBuf {
    let candidate = base_dir.join(relative);
    if candidate.is_file() {
        return candidate;
    }
    // Case-insensitive lookup for cross-platform SFZ banks authored on Windows.
    let mut current = base_dir.to_path_buf();
    for segment in relative.split('/').filter(|s| !s.is_empty() && *s != ".") {
        if segment == ".." {
            current.pop();
            continue;
        }
        let direct = current.join(segment);
        if direct.exists() {
            current = direct;
            continue;
        }
        if let Ok(entries) = fs::read_dir(&current)
            && let Some(found) = entries
                .flatten()
                .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(segment))
        {
            current = found.path();
        } else {
            current = direct;
        }
    }
    current
}

/// Parses either a MIDI number (`0..=127`) or a note name (`c4`, `F#3`, `Bb-1`).
pub fn parse_sfz_note(raw: &str) -> Option<u8> {
    let trimmed = raw.trim();
    if let Ok(n) = trimmed.parse::<u8>() {
        return (n <= 127).then_some(n);
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let base: i32 = match bytes[0].to_ascii_lowercase() {
        b'c' => 0,
        b'd' => 2,
        b'e' => 4,
        b'f' => 5,
        b'g' => 7,
        b'a' => 9,
        b'b' => 11,
        _ => return None,
    };
    let (accidental, oct_str) = match bytes[1] {
        b'#' => (1, &trimmed[2..]),
        b'b' | b'B' => (-1, &trimmed[2..]),
        _ => (0, &trimmed[1..]),
    };
    let octave: i32 = oct_str.parse().ok()?;
    let midi = (octave + 1) * 12 + base + accidental;
    (0..=127).contains(&midi).then_some(midi as u8)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Control,
    Global,
    Group,
    Region,
}

fn parse_opcodes_in_line(line: &str) -> Vec<(String, String)> {
    // Strip // comments.
    let line = match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    };
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    // Find every `<opcode>=` token start so filenames containing spaces (like `sample=Piano C4.wav`)
    // are captured completely up to the next opcode or end-of-line.
    let bytes = trimmed.as_bytes();
    let mut starts: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            let mut key_start = i;
            while key_start > 0
                && (bytes[key_start - 1].is_ascii_alphanumeric() || bytes[key_start - 1] == b'_')
            {
                key_start -= 1;
            }
            if key_start < i && (key_start == 0 || bytes[key_start - 1].is_ascii_whitespace()) {
                starts.push((key_start, i));
            }
        }
        i += 1;
    }
    let mut out = Vec::with_capacity(starts.len());
    for idx in 0..starts.len() {
        let (ks, eq) = starts[idx];
        let val_end = if idx + 1 < starts.len() {
            starts[idx + 1].0
        } else {
            trimmed.len()
        };
        let key = trimmed[ks..eq].to_ascii_lowercase();
        let val = trimmed[eq + 1..val_end].trim().to_string();
        out.push((key, val));
    }
    out
}

impl SfzInstrument {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("Could not read SFZ file {}: {e}", path.display()))?;
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("SFZ Instrument")
            .to_string();
        let mut inst = Self::parse(&text, base_dir, name)?;
        inst.source_path = path.to_path_buf();
        Ok(inst)
    }

    pub fn parse(source: &str, base_dir: &Path, name: impl Into<String>) -> Result<Self, String> {
        let mut section = Section::None;
        let mut default_path = String::new();
        let mut global = OpcodeScope::default();
        let mut group = OpcodeScope::default();
        let mut region: Option<OpcodeScope> = None;
        let mut regions = Vec::new();

        for raw_line in source.lines() {
            // Strip inline comments first so headers inside comments are ignored.
            let line = match raw_line.find("//") {
                Some(idx) => &raw_line[..idx],
                None => raw_line,
            };
            let mut rest = line.trim();
            while !rest.is_empty() {
                if let Some(open) = rest.find('<') {
                    let before = rest[..open].trim();
                    if !before.is_empty() {
                        apply_opcodes(
                            section,
                            before,
                            &mut default_path,
                            &mut global,
                            &mut group,
                            &mut region,
                        );
                    }
                    let after_open = &rest[open + 1..];
                    let Some(close) = after_open.find('>') else {
                        break;
                    };
                    let tag = after_open[..close].trim().to_ascii_lowercase();
                    if let Some(finished) = region.take()
                        && let Some(r) = finished.finish_region(base_dir, &default_path)
                    {
                        regions.push(r);
                    }
                    match tag.as_str() {
                        "control" => section = Section::Control,
                        "global" => {
                            section = Section::Global;
                            global = OpcodeScope::default();
                            group = global.clone();
                        }
                        "group" => {
                            section = Section::Group;
                            group = global.clone();
                        }
                        "region" => {
                            section = Section::Region;
                            region = Some(group.clone());
                        }
                        _ => section = Section::None,
                    }
                    rest = after_open[close + 1..].trim();
                } else {
                    apply_opcodes(
                        section,
                        rest,
                        &mut default_path,
                        &mut global,
                        &mut group,
                        &mut region,
                    );
                    break;
                }
            }
        }
        if let Some(finished) = region.take()
            && let Some(r) = finished.finish_region(base_dir, &default_path)
        {
            regions.push(r);
        }
        if regions.is_empty() {
            return Err("No playable <region> entries found in SFZ file.".into());
        }
        Ok(Self {
            name: name.into(),
            source_path: base_dir.to_path_buf(),
            regions,
        })
    }

    pub fn summary(&self) -> SamplerSummary {
        let key_low = self.regions.iter().map(|r| r.lokey).min();
        let key_high = self.regions.iter().map(|r| r.hikey).max();
        SamplerSummary {
            format: "sfz".into(),
            name: self.name.clone(),
            path: self.source_path.to_string_lossy().into_owned(),
            region_count: self.regions.len(),
            key_low,
            key_high,
            presets: vec![self.name.clone()],
        }
    }

    /// Renders `notes` into stereo interleaved `f32` audio using the built-in pure-Rust WAV decoder.
    pub fn render_notes(&self, notes: &[SamplerNote], sample_rate: u32) -> Result<SamplerAudio, String> {
        self.render_notes_with(notes, sample_rate, decode_wav_file)
    }

    /// Renders `notes` using a custom sample decoder (e.g. `dawsu_audio::decode` for `.wav`/`.ogg`/`.flac`/`.mp3`).
    pub fn render_notes_with<F>(
        &self,
        notes: &[SamplerNote],
        sample_rate: u32,
        mut load_sample: F,
    ) -> Result<SamplerAudio, String>
    where
        F: FnMut(&Path) -> Result<SamplerClip, String>,
    {
        let sr = sample_rate.max(8_000);
        if notes.is_empty() {
            return Ok(SamplerAudio {
                samples: Vec::new(),
                channels: 2,
                sample_rate: sr,
            });
        }
        let mut sorted: Vec<SamplerNote> = notes.to_vec();
        sorted.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));

        let mut clip_cache: HashMap<PathBuf, SamplerClip> = HashMap::new();
        let mut voices: Vec<ScheduledVoice> = Vec::new();

        for note in &sorted {
            let vel = note.velocity.clamp(1, 127);
            let start_frame = ((note.start_ms.max(0.0) / 1000.0) * f64::from(sr)).round() as usize;
            let hold_frames = ((note.duration_ms.max(10.0) / 1000.0) * f64::from(sr)).round() as usize;
            for region in self.regions.iter().filter(|r| r.matches(note.pitch, vel)) {
                // Apply choke groups: any earlier voice with `off_by == region.group` releases right here.
                if let Some(grp) = region.group {
                    for prev in &mut voices {
                        if prev.off_by == Some(grp) && start_frame > prev.start_frame {
                            prev.end_frame = prev.end_frame.min(start_frame);
                            prev.release_frames = prev.release_frames.min(((sr / 200).max(64)) as usize);
                        }
                    }
                }
                if !clip_cache.contains_key(&region.sample_path)
                    && let Ok(clip) = load_sample(&region.sample_path)
                {
                    clip_cache.insert(region.sample_path.clone(), clip);
                }
                if !clip_cache.contains_key(&region.sample_path) {
                    continue;
                }
                let semitones = f64::from(note.pitch) - f64::from(region.pitch_keycenter)
                    + f64::from(region.transpose)
                    + f64::from(region.tune_cents) / 100.0;
                let pitch_ratio = 2.0_f64.powf(semitones / 12.0);
                let vel_gain = (f32::from(vel) / 127.0).powf(1.5);
                let db_gain = 10.0_f32.powf(region.volume_db / 20.0);
                let gain = (vel_gain * db_gain).clamp(0.0, 4.0);
                let attack_frames = (region.ampeg_attack * sr as f32).round() as usize;
                let release_frames = (region.ampeg_release * sr as f32).round().max(32.0) as usize;
                voices.push(ScheduledVoice {
                    sample_key: region.sample_path.clone(),
                    start_frame,
                    end_frame: start_frame + hold_frames,
                    pitch_ratio,
                    gain,
                    pan: (region.pan / 100.0).clamp(-1.0, 1.0),
                    attack_frames,
                    release_frames,
                    off_by: region.off_by,
                    loop_continuous: region.loop_continuous,
                    loop_start: region.loop_start,
                    loop_end: region.loop_end,
                });
            }
        }

        if voices.is_empty() {
            return Ok(SamplerAudio {
                samples: Vec::new(),
                channels: 2,
                sample_rate: sr,
            });
        }

        let total_frames = voices
            .iter()
            .map(|v| v.end_frame.saturating_add(v.release_frames))
            .max()
            .unwrap_or(0)
            .min(sr as usize * 30); // safety cap 30 s per bounce slice
        let mut out = vec![0.0_f32; total_frames * 2];

        for voice in &voices {
            let Some(clip) = clip_cache.get(&voice.sample_key) else {
                continue;
            };
            mix_voice_into_stereo(
                &mut out,
                sr,
                clip,
                VoiceParams {
                    start_frame: voice.start_frame,
                    end_frame: voice.end_frame,
                    pitch_ratio: voice.pitch_ratio,
                    gain: voice.gain,
                    pan: voice.pan,
                    attack_frames: voice.attack_frames,
                    release_frames: voice.release_frames,
                    loop_range: voice
                        .loop_continuous
                        .then(|| voice.loop_start.zip(voice.loop_end))
                        .flatten()
                        .filter(|(s, e)| e > s),
                },
            );
        }

        // Gentle soft clip so multi-voice chords never exceed [-1.0, 1.0] harshly.
        for s in &mut out {
            *s = soft_clip_sample(*s);
        }

        Ok(SamplerAudio {
            samples: out,
            channels: 2,
            sample_rate: sr,
        })
    }
}

fn soft_clip_sample(x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    const KNEE: f32 = 0.95;
    const HEADROOM: f32 = 1.0 - KNEE;
    let ax = x.abs();
    if ax <= KNEE {
        x
    } else {
        (KNEE + HEADROOM * ((ax - KNEE) / HEADROOM).tanh()).copysign(x)
    }
}

fn apply_opcodes(
    section: Section,
    text: &str,
    default_path: &mut String,
    global: &mut OpcodeScope,
    group: &mut OpcodeScope,
    region: &mut Option<OpcodeScope>,
) {
    for (key, value) in parse_opcodes_in_line(text) {
        match section {
            Section::Control => {
                if key == "default_path" {
                    *default_path = value;
                }
            }
            Section::Global => {
                global.apply(&key, &value);
                group.apply(&key, &value);
            }
            Section::Group => group.apply(&key, &value),
            Section::Region => {
                if let Some(r) = region.as_mut() {
                    r.apply(&key, &value);
                }
            }
            Section::None => {}
        }
    }
}

struct ScheduledVoice {
    sample_key: PathBuf,
    start_frame: usize,
    end_frame: usize,
    pitch_ratio: f64,
    gain: f32,
    pan: f32,
    attack_frames: usize,
    release_frames: usize,
    off_by: Option<u32>,
    loop_continuous: bool,
    loop_start: Option<usize>,
    loop_end: Option<usize>,
}

pub(crate) struct VoiceParams {
    pub start_frame: usize,
    pub end_frame: usize,
    pub pitch_ratio: f64,
    pub gain: f32,
    pub pan: f32,
    pub attack_frames: usize,
    pub release_frames: usize,
    pub loop_range: Option<(usize, usize)>,
}

/// Shared click-free polyphonic voice mixer used by both SFZ and SF2 engines.
pub(crate) fn mix_voice_into_stereo(
    out_stereo: &mut [f32],
    out_rate: u32,
    clip: &SamplerClip,
    params: VoiceParams,
) {
    let src_channels = usize::from(clip.channels.max(1));
    let src_frames = clip.samples.len() / src_channels;
    if src_frames == 0 || clip.sample_rate == 0 {
        return;
    }
    let out_frames = out_stereo.len() / 2;
    let total_end = params
        .end_frame
        .saturating_add(params.release_frames)
        .min(out_frames);
    if params.start_frame >= total_end {
        return;
    }

    let step = params.pitch_ratio * (f64::from(clip.sample_rate) / f64::from(out_rate.max(1)));
    let pan_norm = (params.pan.clamp(-1.0, 1.0) + 1.0) * 0.5;
    let angle = pan_norm * std::f32::consts::FRAC_PI_2;
    let gain_l = params.gain * angle.cos() * std::f32::consts::SQRT_2;
    let gain_r = params.gain * angle.sin() * std::f32::consts::SQRT_2;

    let loop_bounds = params.loop_range.and_then(|(s, e)| {
        let e = e.min(src_frames);
        (s < e).then_some((s as f64, e as f64))
    });

    let mut pos = 0.0_f64;
    for frame in params.start_frame..total_end {
        if let Some((ls, le)) = loop_bounds {
            if pos >= le {
                let len = (le - ls).max(1.0);
                pos = ls + (pos - ls) % len;
            }
        } else if pos >= src_frames as f64 {
            break;
        }

        let i0 = (pos.floor() as usize).min(src_frames - 1);
        let i1 = (i0 + 1).min(src_frames - 1);
        let frac = (pos - i0 as f64) as f32;

        let (l, r) = if src_channels == 1 {
            let s = clip.samples[i0] + (clip.samples[i1] - clip.samples[i0]) * frac;
            (s, s)
        } else {
            let l0 = clip.samples[i0 * src_channels];
            let r0 = clip.samples[i0 * src_channels + 1];
            let l1 = clip.samples[i1 * src_channels];
            let r1 = clip.samples[i1 * src_channels + 1];
            (l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac)
        };

        let rel_frame = frame - params.start_frame;
        let attack_env = if params.attack_frames > 0 && rel_frame < params.attack_frames {
            rel_frame as f32 / params.attack_frames as f32
        } else {
            1.0
        };
        let release_env = if frame >= params.end_frame && params.release_frames > 0 {
            let into_rel = frame - params.end_frame;
            (1.0 - into_rel as f32 / params.release_frames as f32).max(0.0)
        } else {
            1.0
        };
        let env = attack_env * release_env;

        out_stereo[frame * 2] += l * gain_l * env;
        out_stereo[frame * 2 + 1] += r * gain_r * env;
        pos += step;
    }
}

/// Pure-Rust WAV decoder supporting 8/16/24-bit PCM and 32-bit IEEE float (mono & stereo).
pub fn decode_wav_file(path: &Path) -> Result<SamplerClip, String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("Could not read sample {}: {e}", path.display()))?;
    decode_wav_bytes(&bytes)
}

pub fn decode_wav_bytes(bytes: &[u8]) -> Result<SamplerClip, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("Not a RIFF WAVE file.".into());
    }
    let mut cursor = 12usize;
    let mut format_tag = 0u16;
    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut bits_per_sample = 0u16;
    let mut data_slice: Option<&[u8]> = None;

    while cursor + 8 <= bytes.len() {
        let id = &bytes[cursor..cursor + 4];
        let size = u32::from_le_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;
        cursor += 8;
        let end = cursor.saturating_add(size).min(bytes.len());
        let chunk = &bytes[cursor..end];
        if id == b"fmt " && chunk.len() >= 16 {
            format_tag = u16::from_le_bytes([chunk[0], chunk[1]]);
            channels = u16::from_le_bytes([chunk[2], chunk[3]]);
            sample_rate = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            bits_per_sample = u16::from_le_bytes([chunk[14], chunk[15]]);
        } else if id == b"data" {
            data_slice = Some(chunk);
        }
        cursor = end + (size & 1);
    }

    let data = data_slice.ok_or("WAV file missing data chunk.")?;
    if channels == 0 || sample_rate == 0 {
        return Err("WAV file missing valid fmt chunk.".into());
    }
    let samples = match (format_tag, bits_per_sample) {
        (1, 8) => data
            .iter()
            .map(|&b| (f32::from(b) - 128.0) / 128.0)
            .collect::<Vec<_>>(),
        (1, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| f32::from(i16::from_le_bytes(c)) / 32768.0)
            .collect(),
        (1, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&c| {
                let sign = if c[2] & 0x80 != 0 { 0xff } else { 0x00 };
                let val = i32::from_le_bytes([c[0], c[1], c[2], sign]);
                val as f32 / 8_388_608.0
            })
            .collect(),
        (3, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&c| {
                let v = f32::from_le_bytes(c);
                if v.is_finite() { v } else { 0.0 }
            })
            .collect(),
        _ => {
            return Err(format!(
                "Unsupported WAV format tag {format_tag} ({bits_per_sample}-bit)."
            ));
        }
    };
    Ok(SamplerClip {
        samples,
        channels,
        sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bounce::write_wav_i16;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dawsu-sfz-{label}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sine_stereo(freq_hz: f32, sr: u32, frames: usize, amp: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = (2.0 * std::f32::consts::PI * freq_hz * i as f32 / sr as f32).sin() * amp;
            out.push(v);
            out.push(v);
        }
        out
    }

    #[test]
    fn parses_sfz_note_names_and_numbers() {
        assert_eq!(parse_sfz_note("60"), Some(60));
        assert_eq!(parse_sfz_note("c4"), Some(60));
        assert_eq!(parse_sfz_note("C#4"), Some(61));
        assert_eq!(parse_sfz_note("bb3"), Some(58));
        assert_eq!(parse_sfz_note("A4"), Some(69));
        assert_eq!(parse_sfz_note("128"), None);
    }

    #[test]
    fn parses_and_renders_sfz_velocity_layers_pitch_shift_and_choke_groups() {
        let dir = make_temp_dir("render");
        let samples_dir = dir.join("samples");
        fs::create_dir_all(&samples_dir).unwrap();
        let soft_wav = samples_dir.join("piano soft.wav");
        let loud_wav = samples_dir.join("piano loud.wav");
        let open_hat_wav = samples_dir.join("hat_open.wav");
        let closed_hat_wav = samples_dir.join("hat_closed.wav");

        write_wav_i16(&soft_wav, &sine_stereo(440.0, 48_000, 12_000, 0.25), 48_000, None).unwrap();
        write_wav_i16(&loud_wav, &sine_stereo(440.0, 48_000, 12_000, 0.8), 48_000, None).unwrap();
        write_wav_i16(&open_hat_wav, &sine_stereo(2000.0, 48_000, 24_000, 0.7), 48_000, None).unwrap();
        write_wav_i16(&closed_hat_wav, &sine_stereo(4000.0, 48_000, 2_400, 0.6), 48_000, None).unwrap();

        let sfz_text = r#"
// Minimal multi-layer SFZ fixture
<control>
default_path=samples/

<global>
ampeg_attack=0.001
ampeg_release=0.02

<group>
lokey=c4 hikey=c5 pitch_keycenter=a4
<region> lovel=1 hivel=64 sample=piano soft.wav
<region> lovel=65 hivel=127 sample=piano loud.wav

<group>
<region> key=46 group=1 off_by=2 sample=hat_open.wav
<region> key=42 group=2 sample=hat_closed.wav
"#;
        let sfz_path = dir.join("kit.sfz");
        fs::write(&sfz_path, sfz_text).unwrap();

        let inst = SfzInstrument::from_file(&sfz_path).unwrap();
        assert_eq!(inst.regions.len(), 4);
        let summary = inst.summary();
        assert_eq!(summary.format, "sfz");
        assert_eq!(summary.region_count, 4);
        assert_eq!(summary.key_low, Some(42));
        assert_eq!(summary.key_high, Some(72));

        // Velocity layer selection: loud note is substantially louder than soft note.
        let soft_audio = inst
            .render_notes(&[SamplerNote { start_ms: 0.0, duration_ms: 100.0, pitch: 69, velocity: 50 }], 48_000)
            .unwrap();
        let loud_audio = inst
            .render_notes(&[SamplerNote { start_ms: 0.0, duration_ms: 100.0, pitch: 69, velocity: 120 }], 48_000)
            .unwrap();
        let peak = |buf: &[f32]| buf.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        assert!(peak(&soft_audio.samples) > 0.01);
        assert!(peak(&loud_audio.samples) > peak(&soft_audio.samples) * 3.0);

        // Choke group: open hat at 0 ms (400 ms duration) is choked by closed hat at 80 ms.
        let unchoked = inst
            .render_notes(
                &[SamplerNote { start_ms: 0.0, duration_ms: 400.0, pitch: 46, velocity: 100 }],
                48_000,
            )
            .unwrap();
        let choked = inst
            .render_notes(
                &[
                    SamplerNote { start_ms: 0.0, duration_ms: 400.0, pitch: 46, velocity: 100 },
                    SamplerNote { start_ms: 80.0, duration_ms: 40.0, pitch: 42, velocity: 100 },
                ],
                48_000,
            )
            .unwrap();
        assert!(peak(&choked.samples[..3_000 * 2]) > 0.1, "open hat must sound before choke");
        assert!(
            choked.samples.len() * 2 < unchoked.samples.len(),
            "choked take ({}) must end well before unchoked open hat ({})",
            choked.samples.len(),
            unchoked.samples.len()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn soft_clip_passes_below_knee_and_saturates_smoothly_above_095() {
        assert_eq!(soft_clip_sample(0.5), 0.5);
        assert_eq!(soft_clip_sample(-0.95), -0.95);
        let over = soft_clip_sample(1.2);
        let extreme = soft_clip_sample(3.0);
        assert!(over > 0.95 && over < 1.0, "over={over}");
        assert!(extreme > over && extreme <= 1.0, "extreme={extreme}");
        assert_eq!(soft_clip_sample(-1.2), -over);
        assert_eq!(soft_clip_sample(f32::NAN), 0.0);
    }
}
