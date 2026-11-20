//! Placing and unlinking sound sets: each is one undo step on the document.

use super::{Composer, HitKey, destination_at, nearest_moment};
use dawsu_foundation::SampleIdentity;

impl Composer {
    /// Places a sound set's hits at `time` (on the nearest moment within `tolerance`, else a new
    /// moment) as one undo step, selects the hits it added, and returns the placement's id. None,
    /// with no undo step, when the set is already placed there.
    pub fn place_sound_set(&mut self, time: f64, set_id: &str, set_digest: &str, gain: i32, hits: &[(String, SampleIdentity, i32)], tolerance: f64) -> Option<u64> {
        let h = self.hitsounds();
        if nearest_moment(h, time, tolerance).is_some_and(|m| h.placements().iter().any(|p| p.moment == m && p.set_id == set_id)) {
            return None;
        }
        let volume = self.insert_volume;
        let mut placed = None;
        self.apply("Place sound set", |h| {
            let moment = destination_at(h, time, volume, tolerance);
            placed = h.place_sound_set(moment, set_id, set_digest, gain, hits).map(|id| (id, moment));
        });
        let (id, moment) = placed?;
        for (_, lane, _) in hits {
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
