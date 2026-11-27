//! Exact audio identity grouping shared by `drum_classify` and `drum_view`.

use dawsu_audio::AudioClip;
use dawsu_foundation::{SampleIdentity, Sound};
use dawsu_hitsounding::{Lane, LaneSource};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SoundGroup {
    pub primary_index: usize,
    pub member_indices: Vec<usize>,
}

pub(crate) fn is_slider_identity(identity: &SampleIdentity) -> bool {
    matches!(
        identity,
        SampleIdentity::Standard {
            sound: Sound::Slider(_),
            ..
        }
    )
}

pub(crate) fn is_clip_silent(clip: &AudioClip) -> bool {
    clip.samples.is_empty() || clip.samples.iter().all(|&s| s.abs() < 1.0e-4)
}

pub(crate) fn primary_sort_key(
    lane: &Lane,
    hit_count: usize,
    slice_order: usize,
) -> (u8, u32, std::cmp::Reverse<usize>, usize) {
    let source_rank = match &lane.source {
        LaneSource::File(_) => 0u8,
        LaneSource::Skin => 1u8,
    };
    let index_rank = match &lane.identity {
        SampleIdentity::Standard { index, .. } => index.get(),
        SampleIdentity::File(_) => u32::MAX,
    };
    (
        source_rank,
        index_rank,
        std::cmp::Reverse(hit_count),
        slice_order,
    )
}

fn clip_signature(clip: &AudioClip) -> (u32, usize, usize, u64) {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &s in &clip.samples {
        let bits = if s == 0.0 { 0u32 } else { s.to_bits() };
        for b in bits.to_le_bytes() {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    (clip.sample_rate, clip.channels, clip.samples.len(), hash)
}

fn clips_exact_eq(a: &Arc<AudioClip>, b: &Arc<AudioClip>) -> bool {
    Arc::ptr_eq(a, b)
        || (a.sample_rate == b.sample_rate
            && a.channels == b.channels
            && a.samples == b.samples)
}

type ClipSignature = (u32, usize, usize, u64);
type ClipBucketMap = HashMap<ClipSignature, Vec<(Arc<AudioClip>, usize)>>;

/// Groups non-silent, non-slider lanes by exact audio equality (identical decoded PCM audio, or
/// identical [`Lane::plays_as`] skin fallback when `clip_for` returns `None`).
///
/// Silent clips and slider lanes never merge with any other lane: when
/// `include_singleton_silent_or_slider` is `true`, each gets its own singleton group; when `false`,
/// they are omitted from the returned groups.
pub(crate) fn group_lanes_by_sound(
    lanes: &[&Lane],
    clip_for: &impl Fn(&SampleIdentity) -> Option<Arc<AudioClip>>,
    hit_counts: &HashMap<SampleIdentity, usize>,
    include_singleton_silent_or_slider: bool,
) -> (Vec<SoundGroup>, Vec<Option<usize>>) {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut group_of: Vec<Option<usize>> = vec![None; lanes.len()];
    let mut clip_buckets: ClipBucketMap = HashMap::new();
    let mut plays_as_buckets: HashMap<SampleIdentity, usize> = HashMap::new();

    for (idx, &lane) in lanes.iter().enumerate() {
        if is_slider_identity(&lane.identity) {
            if include_singleton_silent_or_slider {
                let g = groups.len();
                groups.push(vec![idx]);
                group_of[idx] = Some(g);
            }
            continue;
        }
        match clip_for(&lane.identity) {
            Some(clip) if is_clip_silent(&clip) => {
                if include_singleton_silent_or_slider {
                    let g = groups.len();
                    groups.push(vec![idx]);
                    group_of[idx] = Some(g);
                }
            }
            Some(clip) => {
                let sig = clip_signature(&clip);
                let bucket = clip_buckets.entry(sig).or_default();
                if let Some(&(_, group_idx)) = bucket
                    .iter()
                    .find(|(existing, _)| clips_exact_eq(existing, &clip))
                {
                    groups[group_idx].push(idx);
                    group_of[idx] = Some(group_idx);
                } else {
                    let group_idx = groups.len();
                    groups.push(vec![idx]);
                    group_of[idx] = Some(group_idx);
                    bucket.push((clip, group_idx));
                }
            }
            None => {
                let key = lane.plays_as();
                if let Some(&group_idx) = plays_as_buckets.get(&key) {
                    groups[group_idx].push(idx);
                    group_of[idx] = Some(group_idx);
                } else {
                    let group_idx = groups.len();
                    groups.push(vec![idx]);
                    group_of[idx] = Some(group_idx);
                    plays_as_buckets.insert(key, group_idx);
                }
            }
        }
    }

    let resolved_groups = groups
        .into_iter()
        .map(|mut members| {
            members.sort_by_key(|&idx| {
                let lane = lanes[idx];
                let hits = hit_counts.get(&lane.identity).copied().unwrap_or(0);
                primary_sort_key(lane, hits, idx)
            });
            let primary_index = members[0];
            SoundGroup {
                primary_index,
                member_indices: members,
            }
        })
        .collect();

    (resolved_groups, group_of)
}
