//! VST3 plug-in discovery, isolated Protocol v3 sidecar host control, pure-Rust SFZ/SF2
//! sampler engines, and WAV bouncing.
//!
//! Zero osu!/hitsounding domain types: this crate only knows about instruments,
//! audio buffers, MIDI events, and WAV files.

pub mod bounce;
pub mod host;
pub mod midi_in;
pub mod scanner;
pub mod sf2;
pub mod sfz;
pub mod shm;

use std::path::Path;

pub use bounce::{encode_wav_f32, encode_wav_i16, trim_silence_tail, write_wav_f32, write_wav_i16};
pub use host::{
    HostCommand, HostError, HostReply, HostSession, HostTransport, MAX_EVENTS, MAX_FRAMES,
    MidiEvent, PROTOCOL_MAGIC, PROTOCOL_VERSION, ProcessTransport, RenderRequest, RenderResult,
    ShmBlock, ShmEvent, Vst3HostConfig,
};
pub use midi_in::{
    MidiInputEvent, MidiInputRouter, TimestampedMidiEvent, decode_midi_bytes, list_midi_input_ports,
};
pub use scanner::{
    ScanError, Vst3PluginInfo, default_vst3_dirs, discover_vst3_paths, discover_vst3_paths_in,
    parse_scan_line, parse_scan_output, scan_plugin,
};
pub use sf2::{Sf2Instrument, Sf2Preset, Sf2Zone};
pub use sfz::{
    SamplerAudio, SamplerClip, SamplerNote, SamplerSummary, SfzInstrument, SfzRegion,
    decode_wav_bytes, decode_wav_file, parse_sfz_note,
};
pub use shm::ShmChannel;

/// Unified pure-Rust sample-based instrument (`.sfz` or `.sf2`).
#[derive(Debug, Clone)]
pub enum SampleInstrument {
    Sfz(SfzInstrument),
    Sf2(Sf2Instrument),
}

impl SampleInstrument {
    /// Returns true if `path` has a `.sfz` or `.sf2` extension.
    pub fn is_soundfont_path(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("sfz") || e.eq_ignore_ascii_case("sf2"))
    }

    /// Loads an `.sfz` or `.sf2` file from disk.
    pub fn load(path: &Path) -> Result<Self, String> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        match ext.as_str() {
            "sfz" => Ok(Self::Sfz(SfzInstrument::from_file(path)?)),
            "sf2" => Ok(Self::Sf2(Sf2Instrument::from_file(path)?)),
            _ => Err(format!(
                "Unsupported instrument file '{}'. Choose a .sfz or .sf2 SoundFont.",
                path.display()
            )),
        }
    }

    pub fn summary(&self) -> SamplerSummary {
        match self {
            Self::Sfz(sfz) => sfz.summary(),
            Self::Sf2(sf2) => sf2.summary(),
        }
    }

    pub fn render_notes_with<F>(
        &self,
        preset_index: usize,
        notes: &[SamplerNote],
        sample_rate: u32,
        load_sample: F,
    ) -> Result<SamplerAudio, String>
    where
        F: FnMut(&Path) -> Result<SamplerClip, String>,
    {
        match self {
            Self::Sfz(sfz) => sfz.render_notes_with(notes, sample_rate, load_sample),
            Self::Sf2(sf2) => sf2.render_notes(preset_index, notes, sample_rate),
        }
    }
}
