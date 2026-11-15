//! Song time and note time as their own types (Responsibility 6), so the two cannot be mixed
//! without a conversion (TD-29).

/// A time in the song, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct SongTime(f64);

impl SongTime {
    pub const fn from_ms(ms: f64) -> SongTime {
        SongTime(ms)
    }

    pub const fn ms(self) -> f64 {
        self.0
    }
}

/// A note time in MIDI ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct NoteTicks(i64);

impl NoteTicks {
    pub const fn new(ticks: i64) -> NoteTicks {
        NoteTicks(ticks)
    }

    pub const fn ticks(self) -> i64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn song_time_keeps_its_milliseconds() {
        assert_eq!(SongTime::from_ms(1002.5).ms(), 1002.5);
        assert!(SongTime::from_ms(1.0) < SongTime::from_ms(2.0));
    }

    #[test]
    fn note_ticks_keep_their_ticks() {
        assert_eq!(NoteTicks::new(480).ticks(), 480);
    }
}
