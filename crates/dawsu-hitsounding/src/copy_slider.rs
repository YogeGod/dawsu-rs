//! Places unmatched source hits on the slider timeline as storyboard audio.

use super::{CopyMode, CopyOptions, SliderCopyPlacement};
use crate::{
    Moment, SliderBody,
    rules::{MINIMUM_PLAYED_VOLUME, played_volume},
};
use dawsu_foundation::{SliderSound, SongTime};
use std::collections::{HashSet, VecDeque};

pub(super) fn plan(
    dropped: Vec<usize>,
    source: &[&Moment],
    bodies: &[SliderBody],
    move_to_target_time: f64,
    options: &CopyOptions,
    cancel: &dyn Fn() -> bool,
) -> Option<(Vec<usize>, Vec<SliderCopyPlacement>)> {
    if dropped.is_empty()
        || bodies.is_empty()
        || options.mode != CopyMode::OverwriteOnlyDefined
        || !options.copy_hitsounds
        || !options.copy_sample_sets
        || !(options.copy_to_slider_ticks || options.copy_to_slider_slides)
    {
        return Some((dropped, Vec::new()));
    }

    let mut placements = Vec::new();
    let mut pending = Vec::new();
    let mut still_dropped = Vec::new();
    let mut used_ticks = HashSet::new();
    let mut ticks = Vec::new();
    if options.copy_to_slider_ticks {
        for body in bodies {
            ticks.extend(body.ticks().iter().map(|time| (time.ms(), body)));
        }
        // Stable ordering keeps the first slider for coincident ticks.
        ticks.sort_by(|a, b| a.0.total_cmp(&b.0));
    }

    for group in dropped.chunk_by(|&a, &b| source[a].time() == source[b].time()) {
        if cancel() {
            return None;
        }
        let hits: Vec<_> = group
            .iter()
            .map(|&i| source[i])
            .filter(|moment| moment.hits().next().is_some())
            .collect();
        let time = source[group[0]].time().ms() + move_to_target_time;
        if hits.is_empty() {
            still_dropped.extend_from_slice(group);
            continue;
        }

        let first = ticks.partition_point(|(at, _)| *at < time - options.leniency);
        let tick = ticks[first..]
            .iter()
            .take_while(|(at, _)| *at <= time + options.leniency)
            .find(|(at, _)| !used_ticks.contains(&at.to_bits()));
        if let Some((at, body)) = tick {
            used_ticks.insert(at.to_bits());
            placements.push(placement(&hits, *at, body, SliderSound::Tick, options));
        } else if options.copy_to_slider_slides {
            pending.push((group, hits, time));
        } else {
            still_dropped.extend_from_slice(group);
        }
    }

    // Let every tick claim its time before considering slide fallbacks. Sweep ordered slider
    // bodies to find the first one containing each remaining group without scanning the map per hit.
    let mut bodies: Vec<_> = bodies.iter().collect();
    bodies.sort_by(|a, b| a.start().ms().total_cmp(&b.start().ms()));
    let mut bodies = bodies.into_iter().peekable();
    let mut active = VecDeque::new();
    for (group, hits, time) in pending {
        if cancel() {
            return None;
        }
        while bodies.peek().is_some_and(|body| body.start().ms() < time) {
            active.push_back(bodies.next().expect("peeked body"));
        }
        while active.front().is_some_and(|body| body.end().ms() <= time) {
            active.pop_front();
        }
        if let Some(body) = active
            .front()
            .filter(|_| !used_ticks.contains(&time.to_bits()))
        {
            placements.push(placement(&hits, time, body, SliderSound::Slide, options));
        } else {
            still_dropped.extend_from_slice(group);
        }
    }
    still_dropped.sort_unstable();
    placements.sort_by(|a, b| a.time.ms().total_cmp(&b.time.ms()));
    Some((still_dropped, placements))
}

fn placement(
    hits: &[&Moment],
    time: f64,
    body: &SliderBody,
    role: SliderSound,
    options: &CopyOptions,
) -> SliderCopyPlacement {
    let first = hits[0].hits().next().map_or(100, |lane| played_volume(hits[0].hit_volume(lane)));
    let combined = options.copy_volumes && (hits.len() > 1
        || hits[0].hits().any(|lane| played_volume(hits[0].hit_volume(lane)) != first));
    let volume = if !options.copy_volumes
        || options.always_preserve_5_volume && body.volume() == MINIMUM_PLAYED_VOLUME
    {
        played_volume(body.volume())
    } else if combined {
        100
    } else {
        first
    };
    let layers = hits
        .iter()
        .flat_map(|moment| {
            moment.hits().map(move |lane| {
                let gain = if combined { played_volume(moment.hit_volume(lane)) } else { 100 };
                (lane.clone(), gain)
            })
        })
        .collect();
    SliderCopyPlacement {
        time: SongTime::from_ms(time),
        role,
        volume,
        layers,
    }
}
