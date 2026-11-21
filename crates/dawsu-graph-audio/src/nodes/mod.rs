//! Built-in v1 audio analysis node catalogue registration.

pub mod audio_prep;
pub mod curve;
pub mod events;
pub mod onsets;
pub mod output;
pub mod source;
pub mod spectral;

use crate::providers::{AudioProvider, TimingProvider};
use dawsu_graph::NodeRegistry;
use std::sync::Arc;

/// Registers all 18 v1 audio-graph node types into `registry` using the supplied headless providers.
pub fn register_audio_nodes(
    registry: &mut NodeRegistry,
    audio: Arc<dyn AudioProvider>,
    timing: Arc<dyn TimingProvider>,
) -> Result<(), String> {
    registry.register(Arc::new(source::SongAudioNode {
        provider: audio.clone(),
    }))?;
    registry.register(Arc::new(source::AudioFileNode { provider: audio }))?;
    registry.register(Arc::new(source::MapTimingNode {
        provider: timing,
    }))?;
    registry.register(Arc::new(source::SelectedRangeNode))?;
    registry.register(Arc::new(audio_prep::MonoMixNode))?;
    registry.register(Arc::new(audio_prep::AudioFilterNode))?;
    registry.register(Arc::new(spectral::SpectrogramNode))?;
    registry.register(Arc::new(spectral::PercussiveNode))?;
    registry.register(Arc::new(onsets::OnsetCurveNode))?;
    registry.register(Arc::new(curve::CurveMathNode))?;
    registry.register(Arc::new(events::PeakPickNode))?;
    registry.register(Arc::new(events::TimeSamplerNode))?;
    registry.register(Arc::new(events::MeasureAtEventsNode))?;
    registry.register(Arc::new(events::ConditionFilterNode))?;
    registry.register(Arc::new(events::MergeEventsNode))?;
    registry.register(Arc::new(events::BeatSnapNode))?;
    registry.register(Arc::new(output::LaneOutputNode))?;
    registry.register(Arc::new(output::ViewerOutputNode))?;
    Ok(())
}
