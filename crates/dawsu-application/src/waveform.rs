//! Display data belongs to Application, outside the audio callback.
use dawsu_audio::AudioClip;
#[derive(Debug, Clone, Copy, Default)]
pub struct WaveBin {
    pub peak: f32,
    pub low: f32,
    pub mid: f32,
    pub high: f32,
}
pub fn analyse(audio: &AudioClip) -> Vec<WaveBin> {
    let channels = audio.channels.max(1);
    let frames = audio.samples.len() / channels;
    let step = frames.div_ceil(4000).max(1);
    let mut bins = vec![WaveBin::default(); frames.div_ceil(step)];
    let mut low = 0.0f32;
    let mut wide = 0.0f32;
    let low_alpha = 1.0 - (-2.0 * std::f32::consts::PI * 200.0 / audio.sample_rate as f32).exp();
    let wide_alpha = 1.0 - (-2.0 * std::f32::consts::PI * 2000.0 / audio.sample_rate as f32).exp();
    for (i, frame) in audio.samples.chunks_exact(channels).enumerate() {
        let sample = frame.iter().sum::<f32>() / channels as f32;
        low += low_alpha * (sample - low);
        wide += wide_alpha * (sample - wide);
        let bin = &mut bins[i / step];
        bin.peak = bin
            .peak
            .max(frame.iter().fold(0.0f32, |a, b| a.max(b.abs())));
        bin.low = bin.low.max(low.abs());
        bin.mid = bin.mid.max((wide - low).abs());
        bin.high = bin.high.max((sample - wide).abs());
    }
    bins
}
#[cfg(test)]
mod tests {
    use super::*;
    fn wave(f: f32) -> AudioClip {
        AudioClip {
            sample_rate: 48000,
            channels: 1,
            samples: (0..48000)
                .map(|n| (2.0 * std::f32::consts::PI * f * n as f32 / 48000.0).sin())
                .collect(),
        }
    }
    #[test]
    fn frequency_colours_come_from_the_audio() {
        let bass = analyse(&wave(70.0));
        let treble = analyse(&wave(8000.0));
        let energy = |bins: &[WaveBin]| {
            bins.iter()
                .fold((0.0, 0.0), |(l, h), b| (l + b.low, h + b.high))
        };
        let (l, h) = energy(&bass);
        assert!(l > h * 3.0);
        let (l, h) = energy(&treble);
        assert!(h > l * 3.0);
        assert!(bass.len() <= 4000);
    }
}
