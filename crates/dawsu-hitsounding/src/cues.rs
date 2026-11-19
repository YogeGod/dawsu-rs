//! The cue list (hitsound domain Responsibility 23): what the audio engine plays for a difficulty,
//! built from its lanes, hits and moments with unsaved edits included, at the rules' played volume.

use crate::model::Hitsounds;
use crate::rules::played_volume;
use dawsu_foundation::{Cue, SampleIdentity};

/// The cues of `hitsounds`, in time order. Lanes in `muted` are left out, and while `soloed` is
/// not empty every lane not in it is left out too.
pub fn cue_list(hitsounds: &Hitsounds, muted: &[SampleIdentity], soloed: &[SampleIdentity]) -> Vec<Cue> {
    let audible = |lane: &SampleIdentity| !muted.contains(lane) && (soloed.is_empty() || soloed.contains(lane));
    let plays = |lane: &SampleIdentity| hitsounds.lane(lane).map_or_else(|| lane.clone(), |l| l.plays_as());
    let gain = |volume: i32| f64::from(played_volume(volume)) / 100.0;

    let mut cues = Vec::new();
    for m in hitsounds.moments() {
        for lane in m.hits().filter(|l| audible(l)) {
            cues.push(Cue::new(m.time(), plays(lane), gain(m.hit_volume(lane)), None));
        }
    }
    for b in hitsounds.bodies() {
        if audible(b.slide()) {
            cues.push(Cue::new(b.start(), plays(b.slide()), gain(b.volume()), Some(b.end())));
        }
        let whistle = b.whistle_lane();
        if b.whistle() && audible(&whistle) {
            cues.push(Cue::new(b.start(), plays(&whistle), gain(b.volume()), Some(b.end())));
        }
        if audible(b.tick()) {
            for t in b.ticks() {
                cues.push(Cue::new(*t, plays(b.tick()), gain(b.volume()), None));
            }
        }
    }
    cues.sort_by(|a, b| a.time().ms().total_cmp(&b.time().ms()));
    cues
}
