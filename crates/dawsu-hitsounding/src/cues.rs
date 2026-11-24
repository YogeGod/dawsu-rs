//! The cue list (hitsound domain Responsibility 23): what the audio engine plays for a difficulty,
//! built from its lanes, hits and moments with unsaved edits included, at the rules' played volume.

use crate::model::Hitsounds;
use crate::rules::played_volume;
use dawsu_foundation::{Cue, SampleIdentity};

/// The cues of `hitsounds`, in time order. Lanes in `muted` are left out, and while `soloed` is
/// not empty every lane not in it is left out too.
pub fn cue_list(hitsounds: &Hitsounds, muted: &[SampleIdentity], soloed: &[SampleIdentity]) -> Vec<Cue> {
    cue_list_filtered(hitsounds, muted, soloed, |_, _| false)
}

/// Like [`cue_list`], except per-hit `(moment_idx, lane)` pairs for which `is_hit_muted` returns
/// `true` are left out before lanes resolve to their played sample.
pub fn cue_list_filtered(
    hitsounds: &Hitsounds,
    muted: &[SampleIdentity],
    soloed: &[SampleIdentity],
    is_hit_muted: impl Fn(usize, &SampleIdentity) -> bool,
) -> Vec<Cue> {
    let audible = |lane: &SampleIdentity| !muted.contains(lane) && (soloed.is_empty() || soloed.contains(lane));
    let plays = |lane: &SampleIdentity| hitsounds.lane(lane).map_or_else(|| lane.clone(), |l| l.plays_as());
    let gain = |volume: i32| f64::from(played_volume(volume)) / 100.0;

    let mut cues = Vec::new();
    for (moment_idx, m) in hitsounds.moments().iter().enumerate() {
        for lane in m.hits().filter(|l| audible(l) && !is_hit_muted(moment_idx, l)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decompile::{decompile, Decompiled};
    use crate::model::MomentId;
    use dawsu_foundation::{CustomIndex, Hitsound, SampleSet, Sound};
    use dawsu_osu_format::OsuFile;

    const MINIMAL_MAP: &str = "\
osu file format v14

[General]
Mode: 0

[TimingPoints]
0,500,4,2,0,50,1,0

[HitObjects]
256,192,1000,1,0,0:0:0:0:
256,192,1500,1,0,0:0:0:0:
";

    fn sample_hitsounds(folder: &[String]) -> Hitsounds {
        let file = OsuFile::read(MINIMAL_MAP.as_bytes().to_vec()).file;
        match decompile(&file, folder, &|| false, &mut |_, _| {}) {
            Decompiled::Done(h) => h,
            other => panic!("expected hitsounds, got {other:?}"),
        }
    }

    #[test]
    fn cue_list_filtered_matches_removing_hits_and_preserves_shared_sample_lanes() {
        let folder = vec!["soft-hitclap.wav".to_owned()];
        let mut hits = sample_hitsounds(&folder);
        let clap1 = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: CustomIndex::new(1),
        };
        let clap2 = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: CustomIndex::new(2),
        };
        let clap3 = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: CustomIndex::new(3),
        };
        // Both clap2 and clap3 have no numbered wav in the folder, so both resolve to the skin's
        // Soft Clap index 0. clap1 resolves to soft-hitclap.wav.
        hits.add_hit_with_volume(MomentId(0), clap2.clone(), 40);
        hits.add_hit_with_volume(MomentId(0), clap3.clone(), 80);
        hits.add_hit_with_volume(MomentId(1), clap1.clone(), 60);
        hits.add_hit_with_volume(MomentId(1), clap2.clone(), 30);

        assert_eq!(
            hits.lane(&clap2).unwrap().plays_as(),
            hits.lane(&clap3).unwrap().plays_as(),
            "test setup: both lanes must resolve to the same physical sample"
        );

        let listen_muted = [(0usize, clap2.clone()), (1usize, clap1.clone())];
        let mut cloned = hits.clone();
        for (m, lane) in &listen_muted {
            cloned.remove_hit(MomentId(*m), lane);
        }
        let expected = cue_list(&cloned, &[], &[]);
        let actual = cue_list_filtered(&hits, &[], &[], |m, lane| {
            listen_muted.iter().any(|(mm, l)| *mm == m && l == lane)
        });
        assert_eq!(actual, expected);

        // At moment 0 (1000 ms), clap2 (40%) was listen-muted, while clap3 (80%) — which resolves
        // to the exact same physical sample — must still play at 80%.
        let skin_clap = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Clap),
            index: CustomIndex::new(0),
        };
        let m0_claps: Vec<_> = actual
            .iter()
            .filter(|c| c.time().ms() == 1000.0 && c.sample() == &skin_clap)
            .collect();
        assert_eq!(m0_claps.len(), 1);
        assert_eq!(m0_claps[0].gain(), 0.8);

        // Lane mute/solo still applies alongside listen-mute.
        let normal = SampleIdentity::Standard {
            set: SampleSet::Soft,
            sound: Sound::Hit(Hitsound::Normal),
            index: CustomIndex::new(0),
        };
        let expected_muted = cue_list(&cloned, &[normal.clone()], &[]);
        let actual_muted = cue_list_filtered(&hits, &[normal], &[], |m, lane| {
            listen_muted.iter().any(|(mm, l)| *mm == m && l == lane)
        });
        assert_eq!(actual_muted, expected_muted);
    }
}
