//! The shared sample vocabulary (Responsibility 7). What a sample means when osu! plays it,
//! and its file names, belong to the hitsound rules (ADR 0036).

/// A sample set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SampleSet {
    Normal,
    Soft,
    Drum,
}

/// A hitsound: the hitnormal or one of the three additions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hitsound {
    Normal,
    Whistle,
    Finish,
    Clap,
}

/// A slider sound: the slide, the slider whistle or the slider tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SliderSound {
    Slide,
    Whistle,
    Tick,
}

/// A hitsound or a slider sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sound {
    Hit(Hitsound),
    Slider(SliderSound),
}

/// A custom sample index; 0 means the skin's own sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct CustomIndex(u32);

impl CustomIndex {
    pub const fn new(index: u32) -> CustomIndex {
        CustomIndex(index)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Names one sample sound: a set, a sound and an index, or a sample file named by the map.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SampleIdentity {
    Standard { set: SampleSet, sound: Sound, index: CustomIndex },
    /// A file name a map gives directly, such as a hit object's own sample file.
    File(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_compare_by_value() {
        let a = SampleIdentity::Standard { set: SampleSet::Soft, sound: Sound::Hit(Hitsound::Clap), index: CustomIndex::new(2) };
        let b = SampleIdentity::Standard { set: SampleSet::Soft, sound: Sound::Hit(Hitsound::Clap), index: CustomIndex::new(2) };
        assert_eq!(a, b);
        assert_ne!(a, SampleIdentity::File("clap.wav".into()));
    }
}
