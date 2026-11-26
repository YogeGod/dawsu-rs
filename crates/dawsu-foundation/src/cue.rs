//! The cue (Responsibility 9): a plain playback instruction. It holds no lane, hit, note or
//! map (invariant 9), so the audio engine can play it knowing only this crate.

use crate::{SampleIdentity, SongTime};

/// Play `sample` at `time` with the final `gain`, looping until `loop_end` if set,
/// routed through `strip_id` (`0` = dry / default path).
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    time: SongTime,
    sample: SampleIdentity,
    gain: f64,
    loop_end: Option<SongTime>,
    strip_id: u32,
}

impl Cue {
    pub fn new(time: SongTime, sample: SampleIdentity, gain: f64, loop_end: Option<SongTime>) -> Cue {
        Cue {
            time,
            sample,
            gain,
            loop_end,
            strip_id: 0,
        }
    }

    pub fn with_strip_id(mut self, strip_id: u32) -> Self {
        self.strip_id = strip_id;
        self
    }

    pub fn time(&self) -> SongTime {
        self.time
    }

    pub fn sample(&self) -> &SampleIdentity {
        &self.sample
    }

    pub fn gain(&self) -> f64 {
        self.gain
    }

    pub fn loop_end(&self) -> Option<SongTime> {
        self.loop_end
    }

    pub fn strip_id(&self) -> u32 {
        self.strip_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CustomIndex, Hitsound, SampleSet, Sound};

    #[test]
    fn a_cue_keeps_what_it_was_given() {
        let sample = SampleIdentity::Standard { set: SampleSet::Drum, sound: Sound::Hit(Hitsound::Finish), index: CustomIndex::new(1) };
        let cue = Cue::new(SongTime::from_ms(4480.0), sample.clone(), 0.4, None);
        assert_eq!(cue.time(), SongTime::from_ms(4480.0));
        assert_eq!(cue.sample(), &sample);
        assert_eq!(cue.gain(), 0.4);
        assert_eq!(cue.loop_end(), None);
        assert_eq!(cue.strip_id(), 0);
        let routed = cue.with_strip_id(16);
        assert_eq!(routed.strip_id(), 16);
    }
}
