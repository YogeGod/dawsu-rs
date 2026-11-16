//! Timeline maths over a tempo map, and snapping (Responsibilities 4 and 4a).
//!
//! Each editor builds a `TempoMap` from its own timing (the Hitsound map's red lines, the Keysound
//! tempo map); the kernel never reads them itself.
//!
//! `TempoMap::section_at`, `section_after` and the core of `snap` follow osu!lazer's
//! `osu.Game/Beatmaps/ControlPoints/ControlPointInfo.cs` (`TimingPointAt`, `TimingPointAfter`,
//! `GetClosestSnappedTime`, `getClosestPositiveSnappedTime`) at commit
//! 131fe72a15581049401a588a13684938d4d0e0c7, rewritten in Rust and widened to the long snaps
//! (2/1 to 4/1) lazer does not have.
//! Copyright (c) 2025 ppy Pty Ltd <contact@ppy.sh>. Licensed under the MIT Licence; the full notice
//! is in THIRD-PARTY-NOTICES.md (ADR 0027).

use dawsu_foundation::SongTime;

/// A snap step: whole beats, or 1/N of one beat. Never both more than one beat and a fraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapSpacing {
    beats: u32,
    divisions: u32,
}

/// Why a snap spacing cannot be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapSpacingError {
    Zero,
    LongAndFraction,
}

impl SnapSpacing {
    /// The values the snap menu offers (Responsibility 4): 4/1, 3/1, 2/1, then 1/1 to 1/9.
    pub const OPTIONS: [SnapSpacing; 12] = [
        SnapSpacing {
            beats: 4,
            divisions: 1,
        },
        SnapSpacing {
            beats: 3,
            divisions: 1,
        },
        SnapSpacing {
            beats: 2,
            divisions: 1,
        },
        SnapSpacing {
            beats: 1,
            divisions: 1,
        },
        SnapSpacing {
            beats: 1,
            divisions: 2,
        },
        SnapSpacing {
            beats: 1,
            divisions: 3,
        },
        SnapSpacing {
            beats: 1,
            divisions: 4,
        },
        SnapSpacing {
            beats: 1,
            divisions: 5,
        },
        SnapSpacing {
            beats: 1,
            divisions: 6,
        },
        SnapSpacing {
            beats: 1,
            divisions: 7,
        },
        SnapSpacing {
            beats: 1,
            divisions: 8,
        },
        SnapSpacing {
            beats: 1,
            divisions: 9,
        },
    ];

    pub fn new(beats: u32, divisions: u32) -> Result<SnapSpacing, SnapSpacingError> {
        if beats == 0 || divisions == 0 {
            return Err(SnapSpacingError::Zero);
        }
        if beats > 1 && divisions > 1 {
            return Err(SnapSpacingError::LongAndFraction);
        }
        Ok(SnapSpacing { beats, divisions })
    }

    /// 1/N of a beat.
    pub fn fraction(divisions: u32) -> Result<SnapSpacing, SnapSpacingError> {
        SnapSpacing::new(1, divisions)
    }

    pub fn beats(self) -> u32 {
        self.beats
    }

    pub fn divisions(self) -> u32 {
        self.divisions
    }

    pub fn is_long(self) -> bool {
        self.beats > 1
    }

    pub fn step_ms(self, beat_length_ms: f64) -> f64 {
        beat_length_ms * self.beats as f64 / self.divisions as f64
    }

    /// The next finer (`direction` > 0) or coarser option in `OPTIONS` (Alt+wheel), clamped.
    pub fn neighbour(self, direction: i32) -> SnapSpacing {
        let i = SnapSpacing::OPTIONS
            .iter()
            .position(|o| *o == self)
            .unwrap_or(3) as i32;
        SnapSpacing::OPTIONS
            [(i + direction.signum()).clamp(0, SnapSpacing::OPTIONS.len() as i32 - 1) as usize]
    }
}

/// One red line: from `start` on, a beat lasts `beat_length_ms` and a bar has `beats_per_bar` beats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingSection {
    pub start: SongTime,
    pub beat_length_ms: f64,
    pub beats_per_bar: u32,
}

impl TimingSection {
    /// lazer's `TimingControlPoint.DEFAULT`: 60 BPM, 4/4, from time 0.
    pub const DEFAULT: TimingSection = TimingSection {
        start: SongTime::from_ms(0.0),
        beat_length_ms: 1000.0,
        beats_per_bar: 4,
    };
}

/// A bar and beat position, both counted from 1; `beat_fraction` is how far into the beat (0 to <1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarBeat {
    pub bar: i64,
    pub beat: u32,
    pub beat_fraction: f64,
}

/// An editor's tempo map: red lines sorted by time.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TempoMap {
    sections: Vec<TimingSection>,
}

impl TempoMap {
    /// Sorts the sections by start time; a later section at the same time wins, as in lazer.
    pub fn new(mut sections: Vec<TimingSection>) -> TempoMap {
        sections.sort_by(|a, b| a.start.ms().total_cmp(&b.start.ms()));
        sections.dedup_by(|later, earlier| {
            let same = later.start == earlier.start;
            if same {
                *earlier = *later;
            }
            same
        });
        TempoMap { sections }
    }

    pub fn sections(&self) -> &[TimingSection] {
        &self.sections
    }

    /// The section active at `time`: the last one starting at or before it, else the first one,
    /// else the default (lazer's `TimingPointAt`).
    pub fn section_at(&self, time: SongTime) -> TimingSection {
        match self.sections.partition_point(|s| s.start.ms() <= time.ms()) {
            0 => self
                .sections
                .first()
                .copied()
                .unwrap_or(TimingSection::DEFAULT),
            n => self.sections[n - 1],
        }
    }

    /// The first section starting strictly after `time` (lazer's `TimingPointAfter`).
    pub fn section_after(&self, time: SongTime) -> Option<TimingSection> {
        self.sections
            .get(self.sections.partition_point(|s| s.start.ms() <= time.ms()))
            .copied()
    }

    pub fn beat_length_at(&self, time: SongTime) -> f64 {
        self.section_at(time).beat_length_ms
    }

    /// The closest grid line to `time` (lazer's `GetClosestSnappedTime` with no reference time):
    /// snapped within its own section, unless the next red line is closer.
    pub fn snap(&self, time: SongTime, spacing: SnapSpacing) -> SongTime {
        let section = self.section_at(time);
        let snapped = closest_positive_snapped_time(section, time.ms(), spacing);
        match self.section_after(time) {
            Some(after) if (time.ms() - after.start.ms()).abs() <= (time.ms() - snapped).abs() => {
                after.start
            }
            _ => SongTime::from_ms(snapped),
        }
    }

    /// The previous (`direction` < 0) or next grid line from `time`; never before 0.
    pub fn grid_step(&self, time: SongTime, direction: i32, spacing: SnapSpacing) -> SongTime {
        let dir = direction.signum() as f64;
        let step = spacing.step_ms(self.beat_length_at(time));
        let target = self
            .snap(SongTime::from_ms(time.ms() + dir * step * 0.51), spacing)
            .ms();
        let target = if (target - time.ms()).abs() < 1e-6 {
            time.ms() + dir * step
        } else {
            target
        };
        SongTime::from_ms(target.max(0.0))
    }

    /// The bar and beat at `time`. Each red line starts a new bar, as in osu!.
    pub fn bar_beat_at(&self, time: SongTime) -> BarBeat {
        let current = self.section_at(time);
        let mut bar: i64 = 1;
        for pair in self.sections.windows(2) {
            if pair[1].start.ms() > current.start.ms() {
                break;
            }
            let bar_ms = pair[0].beat_length_ms * pair[0].beats_per_bar as f64;
            bar += ((pair[1].start.ms() - pair[0].start.ms()) / bar_ms - 1e-9)
                .ceil()
                .max(0.0) as i64;
        }
        let beats = (time.ms() - current.start.ms()) / current.beat_length_ms;
        let whole = (beats + 1e-9).floor();
        let per_bar = current.beats_per_bar.max(1) as i64;
        let whole_i = whole as i64;
        bar += whole_i.div_euclid(per_bar);
        BarBeat {
            bar,
            beat: whole_i.rem_euclid(per_bar) as u32 + 1,
            beat_fraction: (beats - whole).max(0.0),
        }
    }
}

/// lazer's `getClosestPositiveSnappedTime`, with the step widened to `spacing`.
fn closest_positive_snapped_time(section: TimingSection, time: f64, spacing: SnapSpacing) -> f64 {
    let step = spacing.step_ms(section.beat_length_ms);
    let beats = (time.max(0.0) - section.start.ms()) / step;
    // C#'s Math.Round(x, MidpointRounding.AwayFromZero) is Rust's f64::round.
    let snapped = section.start.ms() + beats.round() * step;
    if snapped >= 0.0 {
        snapped
    } else {
        snapped + step
    }
}

/// The snap modes (Responsibility 4a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnapMode {
    /// To the closest grid line.
    #[default]
    Grid,
    /// Moves by whole grid steps, so an item keeps its offset from its own grid line.
    GridRelative,
    /// To the closest object time the editor supplies.
    Objects,
}

/// An editor's snap settings: on or off, the mode and the spacing. Per editor, never static.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snap {
    pub on: bool,
    pub mode: SnapMode,
    pub spacing: SnapSpacing,
}

impl Default for Snap {
    fn default() -> Self {
        Snap {
            on: true,
            mode: SnapMode::Grid,
            spacing: SnapSpacing {
                beats: 1,
                divisions: 4,
            },
        }
    }
}

impl Snap {
    /// `time` snapped. `objects` are the editor's object times, sorted; used only in Objects mode.
    pub fn time(&self, tempo: &TempoMap, objects: &[SongTime], time: SongTime) -> SongTime {
        if !self.on {
            return time;
        }
        match self.mode {
            SnapMode::Grid | SnapMode::GridRelative => tempo.snap(time, self.spacing),
            SnapMode::Objects => closest_object(objects, time).unwrap_or(time),
        }
    }

    /// The snapped offset for dragging an item at `from` by `raw_ms`: with Grid relative by whole
    /// grid steps (the item keeps its offset from its line), otherwise to where `from + raw` snaps.
    pub fn offset(
        &self,
        tempo: &TempoMap,
        objects: &[SongTime],
        from: SongTime,
        raw_ms: f64,
    ) -> f64 {
        let moved = SongTime::from_ms(from.ms() + raw_ms);
        if self.on && self.mode == SnapMode::GridRelative {
            return tempo.snap(moved, self.spacing).ms() - tempo.snap(from, self.spacing).ms();
        }
        self.time(tempo, objects, moved).ms() - from.ms()
    }
}

fn closest_object(objects: &[SongTime], time: SongTime) -> Option<SongTime> {
    let next = objects.partition_point(|o| o.ms() < time.ms());
    let before = next.checked_sub(1).map(|i| objects[i]);
    let after = objects.get(next).copied();
    match (before, after) {
        (Some(b), Some(a)) => Some(if time.ms() - b.ms() <= a.ms() - time.ms() {
            b
        } else {
            a
        }),
        (b, a) => b.or(a),
    }
}
