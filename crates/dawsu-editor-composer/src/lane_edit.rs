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
}
