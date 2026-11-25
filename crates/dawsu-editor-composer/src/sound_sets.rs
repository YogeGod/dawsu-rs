//! Placing and unlinking sound sets: each is one undo step on the document.

use super::{Composer, HitKey, destination_at, nearest_moment};
use dawsu_foundation::{Hitsound, SampleIdentity, Sound};
use dawsu_hitsounding::MomentId;

/// The member id of the moment's own hitnormal that an additions-only set moved to its index (D6b).
pub const MOMENT_HITNORMAL: &str = "moment-hitnormal";

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

/// One member hit of a sound set: `(member_id, lane, volume)`.
pub type SoundSetHit = (String, SampleIdentity, i32);

/// One placement update: `(placement_id, set_digest, gain, hits)`.
pub type PlacementUpdate = (u64, String, i32, Vec<SoundSetHit>);

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
    #[allow(clippy::too_many_arguments)]
    pub fn place_sound_set(&mut self, time: f64, set_id: &str, set_digest: &str, gain: i32, hits: &[SoundSetHit], hitnormal: SetHitnormal, tolerance: f64) -> Option<u64> {
        if self.is_set_placed_at(time, set_id, tolerance) {
            return None;
        }
        let volume = self.insert_volume;
        let mut placed = None;
        let mut hits = hits.to_vec();
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
                    // The moved hitnormal is one of the placement's members, so Update, Reset
                    // and Unlink handle it and another set leaves it alone.
                    if let [lane] = own.as_slice() {
                        let gain = h.moment(moment).hit_volume(lane);
                        h.remove_hit(moment, lane);
                        hits.push((MOMENT_HITNORMAL.to_string(), target.clone(), gain));
                    }
                }
            }
            placed = h.place_sound_set(moment, set_id, set_digest, gain, &hits).map(|id| (id, moment));
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

    /// Plays each placement `(id, set digest, gain, hits)` as its set's new version, all as one
    /// undo step named `label`, and gives each new lane a row. Returns the times (ms) of the
    /// moments where the mapper's own hitnormal kept the set's new one out (D3).
    pub fn update_placements(&mut self, label: &str, updates: &[PlacementUpdate]) -> Vec<f64> {
        let mut kept = Vec::new();
        if updates.is_empty() {
            return kept;
        }
        self.apply(label, |h| {
            for (id, digest, gain, hits) in updates {
                if h.update_placement(*id, digest, *gain, hits)
                    && let Some(p) = h.placements().iter().find(|p| p.id == *id)
                {
                    kept.push(h.moment(p.moment).time().ms());
                }
            }
        });
        self.add_rows(updates.iter().flat_map(|(_, _, _, hits)| hits.iter().map(|(_, lane, _)| lane)));
        kept
    }

    /// Sets a placement's volume as one undo step: `hits` are its set's members at `gain`.
    /// Members edited by hand keep their hits.
    pub fn set_placement_gain(&mut self, id: u64, set_digest: &str, gain: i32, hits: &[SoundSetHit]) {
        if self.hitsounds().placements().iter().any(|p| p.id == id) {
            self.apply("Set sound set volume", |h| {
                h.update_placement(id, set_digest, gain, hits);
            });
            self.add_rows(hits.iter().map(|(_, lane, _)| lane));
        }
    }

    /// Puts a placement's edited members back as one undo step. With `current` (the set's
    /// digest and its hits at the placement's gain), members then follow the set's values.
    pub fn reset_placement(&mut self, id: u64, current: Option<(&str, &[SoundSetHit])>) {
        let Some(gain) = self.hitsounds().placements().iter().find(|p| p.id == id).map(|p| p.gain) else { return };
        self.apply("Reset sound set edits", |h| {
            h.reset_placement(id);
            if let Some((digest, hits)) = current {
                h.update_placement(id, digest, gain, hits);
            }
        });
        if let Some((_, hits)) = current {
            self.add_rows(hits.iter().map(|(_, lane, _)| lane));
        }
    }

    /// How many of a placement's added sounds are edited, of how many.
    pub fn placement_edits(&self, id: u64) -> Option<(usize, usize)> {
        let h = self.hitsounds();
        let p = h.placements().iter().find(|p| p.id == id)?;
        let added: Vec<_> = p.applied.iter().filter(|a| a.added).collect();
        Some((added.iter().filter(|a| h.is_member_edited(p, a)).count(), added.len()))
    }

    /// The live placement one of whose added hits is selected, the most recent first.
    pub fn selected_placement(&self) -> Option<u64> {
        let h = self.hitsounds();
        let selected = self.selected.items();
        h.placements().iter().rev().filter(|p| h.is_placement_live(p)).find(|p| {
            p.applied.iter().any(|a| a.added && selected.contains(&HitKey { moment: p.moment.0, lane: a.lane.clone() }))
        }).map(|p| p.id)
    }

    fn add_rows<'a>(&mut self, lanes: impl Iterator<Item = &'a SampleIdentity>) {
        for lane in lanes {
            if !self.lanes.contains(lane) {
                self.lanes.push(lane.clone());
            }
        }
    }
}
