// Ported from osu!lazer (ppy/osu) at 131fe72a15581049401a588a13684938d4d0e0c7:
//   osu.Game/Rulesets/Objects/SliderEventGenerator.cs (Generate, generateTicks)
// Copyright (c) ppy Pty Ltd <contact@ppy.sh>. Licensed under the MIT Licence (ADR 0027).
//
// The legacy last tick is left out: the osu! slider makes no object from it
// (osu.Game.Rulesets.Osu/Objects/Slider.cs, CreateNestedHitObjects), so it plays no sound.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderEventType {
    Head,
    Tick,
    Repeat,
    Tail,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderEvent {
    pub kind: SliderEventType,
    pub time: f64,
    pub span_index: usize,
}

/// The head, ticks, repeats and tail of a slider, in the order lazer generates them.
pub fn generate(start_time: f64, span_duration: f64, velocity: f64, tick_distance: f64, total_distance: f64, span_count: usize) -> Vec<SliderEvent> {
    const MAX_LENGTH: f64 = 100_000.0;
    let length = MAX_LENGTH.min(total_distance);
    let tick_distance = tick_distance.clamp(0.0, length.max(0.0));
    let min_distance_from_end = velocity * 10.0;

    let mut events = vec![SliderEvent { kind: SliderEventType::Head, time: start_time, span_index: 0 }];

    for span in 0..span_count {
        let span_start_time = start_time + span as f64 * span_duration;
        let reversed = span % 2 == 1;
        if tick_distance != 0.0 {
            let mut ticks = Vec::new();
            let mut d = tick_distance;
            while d <= length {
                if d >= length - min_distance_from_end {
                    break;
                }
                let path_progress = d / length;
                let time_progress = if reversed { 1.0 - path_progress } else { path_progress };
                ticks.push(SliderEvent { kind: SliderEventType::Tick, time: span_start_time + time_progress * span_duration, span_index: span });
                d += tick_distance;
            }
            if reversed {
                ticks.reverse();
            }
            events.extend(ticks);
        }
        if span + 1 < span_count {
            events.push(SliderEvent { kind: SliderEventType::Repeat, time: span_start_time + span_duration, span_index: span });
        }
    }

    let total_duration = span_count as f64 * span_duration;
    events.push(SliderEvent { kind: SliderEventType::Tail, time: start_time + total_duration, span_index: span_count.saturating_sub(1) });
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_span_slider_has_a_head_ticks_a_repeat_and_a_tail() {
        let e = generate(1000.0, 300.0, 0.5, 50.0, 150.0, 2);
        let kinds: Vec<SliderEventType> = e.iter().map(|e| e.kind).collect();
        assert_eq!(kinds.first(), Some(&SliderEventType::Head));
        assert_eq!(kinds.last(), Some(&SliderEventType::Tail));
        assert_eq!(kinds.iter().filter(|k| **k == SliderEventType::Repeat).count(), 1);
        assert_eq!(kinds.iter().filter(|k| **k == SliderEventType::Tick).count(), 4);
        assert_eq!(e.last().unwrap().time, 1600.0);
    }

    #[test]
    fn no_ticks_with_an_infinite_tick_distance() {
        let e = generate(0.0, 100.0, 1.0, f64::INFINITY, 100.0, 1);
        assert_eq!(e.len(), 2, "an infinite tick distance is clamped to the length, which leaves no tick");
    }
}
