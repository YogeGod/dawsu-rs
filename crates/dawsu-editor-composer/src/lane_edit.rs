//! Lane visibility and hit removal share the Hitsounds history.

use super::{Composer, normal};
use dawsu_foundation::{SampleIdentity, Sound};
use dawsu_hitsounding::MomentId;
use std::convert::Infallible;

impl Composer {
    pub fn delete_lane(&mut self, lane: &SampleIdentity) -> Result<(), String> {
        let hits = self.hitsounds();
        if hits.lane(lane).is_none() || hits.is_lane_hidden(lane) {
            return Err("This lane is no longer visible".into());
        }
        let slot = hits.lane_slot(lane).unwrap_or(lane);
        if matches!(
            slot,
            SampleIdentity::Standard {
                sound: Sound::Slider(_),
                ..
            }
        ) {
            return Err("Select the slider to edit its body sounds".into());
        }
        let moments: Vec<_> = hits
            .moments()
            .iter()
            .enumerate()
            .filter_map(|(index, moment)| moment.has_hit(lane).then_some(MomentId(index)))
            .collect();
        if self.protect_normals && normal(slot) && !moments.is_empty() {
            return Err("Hitnormals are protected".into());
        }
        let _ = self
            .history
            .apply("Delete lane", !moments.is_empty(), |hits| {
                for moment in moments {
                    hits.remove_hit(moment, lane);
                }
                hits.set_lane_hidden(lane, true);
                Ok::<_, Infallible>(())
            });
        self.selected.retain(|key| &key.lane != lane);
        Ok(())
    }

    pub fn show_lane(&mut self, lane: &SampleIdentity) -> bool {
        let hits = self.hitsounds();
        if hits.lane(lane).is_none() || !hits.is_lane_hidden(lane) {
            return false;
        }
        let _ = self.history.apply("Add lane", false, |hits| {
            hits.set_lane_hidden(lane, false);
            Ok::<_, Infallible>(())
        });
        true
    }

    /// Unhides every currently hidden lane in `lanes` as one undo step without marking dirty.
    pub fn show_lanes(&mut self, label: &str, lanes: &[SampleIdentity]) -> usize {
        let hits = self.hitsounds();
        let to_show: Vec<_> = lanes
            .iter()
            .filter(|lane| hits.lane(lane).is_some() && hits.is_lane_hidden(lane))
            .cloned()
            .collect();
        if to_show.is_empty() {
            return 0;
        }
        let count = to_show.len();
        let _ = self.history.apply(label, false, |hits| {
            for lane in &to_show {
                hits.set_lane_hidden(lane, false);
            }
            Ok::<_, Infallible>(())
        });
        count
    }

    /// Moves every second selected hit on `from_lane` (in chronological order) onto `to_lane`.
    pub fn alternate_lane_hits(
        &mut self,
        from_lane: &SampleIdentity,
        to_lane: &SampleIdentity,
    ) -> usize {
        if from_lane == to_lane {
            return 0;
        }
        let hits = self.hitsounds();
        if hits.lane(from_lane).is_none() || hits.lane(to_lane).is_none() {
            return 0;
        }
        let mut selected_on_lane: Vec<_> = self
            .selected_keys_in_order()
            .into_iter()
            .filter(|key| {
                &key.lane == from_lane
                    && hits
                        .moments()
                        .get(key.moment)
                        .is_some_and(|m| m.has_hit(from_lane))
            })
            .collect();
        selected_on_lane.sort_by(|a, b| {
            let ta = hits.moments()[a.moment].time().ms();
            let tb = hits.moments()[b.moment].time().ms();
            ta.total_cmp(&tb).then_with(|| a.moment.cmp(&b.moment))
        });
        let to_move: Vec<_> = selected_on_lane
            .into_iter()
            .enumerate()
            .filter_map(|(idx, key)| (idx % 2 == 1).then_some(key))
            .collect();
        if to_move.is_empty() {
            return 0;
        }
        let moved_count = to_move.len();
        self.apply("Alternate variants", |hits| {
            for key in &to_move {
                let vol = hits.moment(MomentId(key.moment)).hit_volume(from_lane);
                hits.remove_hit(MomentId(key.moment), from_lane);
                hits.add_hit_with_volume(MomentId(key.moment), to_lane.clone(), vol);
            }
        });
        if !self.lanes.contains(to_lane) {
            self.lanes.push(to_lane.clone());
        }
        let mut updated_selection: Vec<_> = self.selected_keys_in_order();
        for key in &to_move {
            if let Some(entry) = updated_selection
                .iter_mut()
                .find(|k| k.moment == key.moment && &k.lane == from_lane)
            {
                entry.lane = to_lane.clone();
            }
        }
        self.selected.replace(updated_selection);
        moved_count
    }
}
