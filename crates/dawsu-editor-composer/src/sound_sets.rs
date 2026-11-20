//! Placing and unlinking sound sets: each is one undo step on the document.

use super::{Composer, HitKey, destination_at, nearest_moment};
use dawsu_foundation::{Hitsound, SampleIdentity, Sound};
use dawsu_hitsounding::MomentId;

/// What placing a set does to the hitnormal the moment already has (D6, D6b). An object plays one
/// custom index for its hitnormal and additions, so the moment's own hitnormal can't stay as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetHitnormal {
    /// Leave the moment's hits as they are.
    Keep,
    /// The set has a hitnormal: it replaces every other hitnormal on the moment (D6).
    ReplaceWithSet,
    /// The set has only additions: the moment's hitnormal moves to this lane, the same sound
    /// copied at the set's index, keeping its volume (D6b).
    Reindex(SampleIdentity),
}

fn is_hitnormal(lane: &SampleIdentity) -> bool {
    matches!(lane, SampleIdentity::Standard { sound: Sound::Hit(Hitsound::Normal), .. })
}

impl Composer {
    /// The moment a set placed at `time` lands on, if one is within `tolerance`.
    pub fn moment_at(&self, time: f64, tolerance: f64) -> Option<MomentId> {
        nearest_moment(self.hitsounds(), time, tolerance)
    }

    /// True when a live placement of `set_id` is on the moment a set placed at `time` lands on.
    pub fn is_set_placed_at(&self, time: f64, set_id: &str, tolerance: f64) -> bool {
        let h = self.hitsounds();
        nearest_moment(h, time, tolerance).is_some_and(|m| h.placements().iter().any(|p| p.moment == m && p.set_id == set_id && h.is_placement_live(p)))
    }

    /// Places a sound set's hits at `time` (on the nearest moment within `tolerance`, else a new
    /// moment) as one undo step, selects the hits it added, and returns the placement's id. None,
    /// with no undo step, when the set is already placed there.
    pub fn place_sound_set(&mut self, time: f64, set_id: &str, set_digest: &str, gain: i32, hits: &[(String, SampleIdentity, i32)], hitnormal: SetHitnormal, tolerance: f64) -> Option<u64> {
        if self.is_set_placed_at(time, set_id, tolerance) {
            return None;
        }
        let volume = self.insert_volume;
        let mut placed = None;
        self.apply("Place sound set", |h| {
            let moment = destination_at(h, time, volume, tolerance);
            // Hitnormals another set placed here belong to that set and stay.
            let kept = h.placed_lanes(moment);
            let own: Vec<_> = h.moment(moment).hits().filter(|lane| is_hitnormal(lane) && !kept.contains(*lane) && !hits.iter().any(|(_, set_lane, _)| set_lane == *lane)).cloned().collect();
            match &hitnormal {
                SetHitnormal::Keep => {}
                SetHitnormal::ReplaceWithSet => {
                    for lane in &own {
                        h.remove_hit(moment, lane);
                    }
                }
                SetHitnormal::Reindex(target) => {
                    if let [lane] = own.as_slice() {
                        let gain = h.moment(moment).hit_volume(lane);
                        h.remove_hit(moment, lane);
                        h.add_hit_with_volume(moment, target.clone(), gain);
                    }
                }
            }
            placed = h.place_sound_set(moment, set_id, set_digest, gain, hits).map(|id| (id, moment));
        });
        let (id, moment) = placed?;
        let reindexed = match hitnormal {
            SetHitnormal::Reindex(lane) => Some(lane),
            _ => None,
        };
        for lane in hits.iter().map(|(_, lane, _)| lane).chain(reindexed.as_ref()) {
            if !self.lanes.contains(lane) {
                self.lanes.push(lane.clone());
            }
        }
        let added: Vec<_> = self.hitsounds().placements().iter().find(|p| p.id == id).into_iter()
            .flat_map(|p| p.applied.iter().filter(|a| a.added).map(|a| HitKey { moment: moment.0, lane: a.lane.clone() }))
            .collect();
        self.selected.replace(added);
        Some(id)
    }

    /// Removes a placement's link as one undo step; its hits stay.
    pub fn unlink_placement(&mut self, id: u64) {
        if self.hitsounds().placements().iter().any(|p| p.id == id) {
            self.apply("Unlink sound set", |h| h.unlink_placement(id));
        }
    }
}
