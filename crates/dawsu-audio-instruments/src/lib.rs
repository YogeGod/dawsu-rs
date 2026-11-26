//! VST3 plug-in discovery, isolated Protocol v4 sidecar host control, pure-Rust SFZ/SF2
//! sampler engines, and WAV bouncing.
//!
//! Zero osu!/hitsounding domain types: this crate only knows about instruments,
//! audio buffers, MIDI events, and WAV files.

pub mod bounce;
pub mod host;
pub mod midi_in;
pub mod midi_state;
pub mod scanner;
pub mod sf2;
pub mod sfz;
pub mod shm;

use std::path::Path;

pub use bounce::{
    CrossfadeCurve, DEFAULT_TAIL_FADE_MS, apply_head_fade, crossfade_stereo, encode_wav_f32,
    encode_wav_f32_with_fade, encode_wav_i16, encode_wav_i16_with_fade, trim_and_fade_tail,
    trim_silence_tail, write_wav_f32, write_wav_f32_with_fade, write_wav_i16,
    write_wav_i16_with_fade,
};
pub use host::{
    CTX_FLAG_BAR_POSITION_VALID, CTX_FLAG_CYCLE_ACTIVE, CTX_FLAG_CYCLE_VALID, CTX_FLAG_PLAYING,
    CTX_FLAG_PROJECT_TIME_MUSIC_VALID, CTX_FLAG_TEMPO_VALID, CTX_FLAG_TIME_SIG_VALID, HostCommand,
    HostError, HostReply, HostSession, HostTransport, MAX_EVENTS, MAX_FRAMES, MidiEvent,
    PROTOCOL_MAGIC, PROTOCOL_VERSION, ProcessTransport, RESTART_IO_CHANGED,
    RESTART_LATENCY_CHANGED, RESTART_MIDI_CC_CHANGED, RESTART_PARAM_TITLES_CHANGED,
    RESTART_PARAM_VALUES_CHANGED, RESTART_RELOAD_COMPONENT, RenderRequest, RenderResult,
    SHM_FLAG_BYPASSED, SHM_FLAG_HAS_AUDIO_IN, ShmBlock, ShmEvent, ShmProcessContext,
    Vst3HostConfig,
};
pub use midi_in::{
    MidiInputEvent, MidiInputRouter, TimestampedMidiEvent, decode_midi_bytes, list_midi_input_ports,
};
pub use midi_state::MidiStateTracker;
pub use scanner::{
    BlacklistEntry, ScanError, Vst3Blacklist, Vst3PluginInfo, binary_fingerprint,
    canonical_hex_to_com_guid_hex, canonicalize_vst3_uid, com_guid_hex_to_canonical_hex,
    default_vst3_dirs, discover_vst3_paths, discover_vst3_paths_in, parse_scan_line,
    parse_scan_output, resolve_vst3_binary, scan_plugin, try_scan_moduleinfo,
    tuid_bytes_to_canonical_hex, uids_equivalent,
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
