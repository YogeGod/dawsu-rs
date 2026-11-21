//! Audio analysis nodes and classic drum detection (`DrumOnsets` port).
//!
//! References `dawsu-foundation`, `dawsu-graph`, `serde`, `serde_json`, and the standard library only.

pub mod classic;
pub mod nodes;
pub mod presets;
pub mod providers;

pub use nodes::register_audio_nodes;
pub use presets::{
    CLASSIC_KICK_SNARE_JSON, classic_kick_snare_graph, set_classic_sensitivity,
    set_classic_snap_muted,
};
pub use providers::{AudioProvider, DecodedAudio, StaticProviders, TimingProvider};
