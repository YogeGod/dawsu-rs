//! VST3 plug-in discovery, isolated Protocol v3 sidecar host control, and WAV bouncing.
//!
//! Zero osu!/hitsounding domain types: this crate only knows about VST3 plug-ins,
//! audio buffers, MIDI events, and WAV files.

pub mod bounce;
pub mod host;
pub mod midi_in;
pub mod scanner;
pub mod shm;

pub use bounce::{encode_wav_f32, encode_wav_i16, trim_silence_tail, write_wav_f32, write_wav_i16};
pub use host::{
    HostCommand, HostError, HostReply, HostSession, HostTransport, MAX_EVENTS, MAX_FRAMES,
    MidiEvent, PROTOCOL_MAGIC, PROTOCOL_VERSION, ProcessTransport, RenderRequest, RenderResult,
    ShmBlock, ShmEvent, Vst3HostConfig,
};
pub use midi_in::{MidiInputEvent, MidiInputRouter, decode_midi_bytes, list_midi_input_ports};
pub use scanner::{
    ScanError, Vst3PluginInfo, default_vst3_dirs, discover_vst3_paths, discover_vst3_paths_in,
    parse_scan_line, parse_scan_output, scan_plugin,
};
pub use shm::ShmChannel;
