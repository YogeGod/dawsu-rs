//! Mapset-wide 12-slot custom sample index packing, greedy longest-run index assignment,
//! `AdditionOnlyNormal` policy, and standard 44-byte silent WAV helper.
//!
//! Sharing one 12-slot custom index pool across every difficulty in a mapset merges compatible
//! sample combinations into fewer numbered sample files and assigns longest-run indices to cut
//! redundant green lines, matching (and extending) Mapping Tools' Hitsound Studio packing.

use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Standard osu! Ranking Criteria 44-byte blank 16-bit mono PCM WAV (0 data bytes).
pub const STANDARD_SILENT_WAV: [u8; 44] = [
    b'R', b'I', b'F', b'F', 36, 0, 0, 0, b'W', b'A', b'V', b'E', b'f', b'm', b't', b' ', 16, 0,
    0, 0, 1, 0, 1, 0, 0x44, 0xAC, 0, 0, 0x88, 0x58, 0x01, 0, 2, 0, 16, 0, b'd', b'a', b't', b'a',
    0, 0, 0, 0,
];

/// What hitnormal sound plays when a moment contains only additions (whistle, finish, or clap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdditionOnlyNormal {
    /// Keep the hitnormal of the difficulty's active timing-point sample set (default).
    #[default]
    Difficulty,
    /// Use the skin hitnormal of the top addition's sample set (Mapping Tools default).
    AdditionsSet,
    /// Always use the skin's `normal-hitnormal`.
    SkinNormal,
    /// Always use the skin's `soft-hitnormal`.
    SkinSoft,
    /// Always use the skin's `drum-hitnormal`.
    SkinDrum,
    /// Use a standard 44-byte silent WAV so only the additions are heard.
    Silent,
}

impl AdditionOnlyNormal {
    /// Resolves which hitnormal sample (or `None` for the standard 44-byte silent WAV) should
    /// accompany an addition-only moment given the difficulty's active normal set and the top
    /// addition's sample set.
    pub fn resolve_normal_set(
        self,
        difficulty_set: SampleSet,
        addition_set: SampleSet,
    ) -> Option<SampleSet> {
        match self {
            Self::Difficulty => Some(difficulty_set),
            Self::AdditionsSet => Some(addition_set),
            Self::SkinNormal => Some(SampleSet::Normal),
            Self::SkinSoft => Some(SampleSet::Soft),
            Self::SkinDrum => Some(SampleSet::Drum),
            Self::Silent => None,
        }
    }
}

/// Canonical 12-slot key for a custom index: `(SampleSet, Hitsound)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SlotKey {
    pub set: SampleSet,
    pub sound: Hitsound,
}

impl SlotKey {
    pub fn stem(self) -> &'static str {
        match (self.set, self.sound) {
            (SampleSet::Normal, Hitsound::Normal) => "normal-hitnormal",
            (SampleSet::Normal, Hitsound::Whistle) => "normal-hitwhistle",
            (SampleSet::Normal, Hitsound::Finish) => "normal-hitfinish",
            (SampleSet::Normal, Hitsound::Clap) => "normal-hitclap",
            (SampleSet::Soft, Hitsound::Normal) => "soft-hitnormal",
            (SampleSet::Soft, Hitsound::Whistle) => "soft-hitwhistle",
            (SampleSet::Soft, Hitsound::Finish) => "soft-hitfinish",
            (SampleSet::Soft, Hitsound::Clap) => "soft-hitclap",
            (SampleSet::Drum, Hitsound::Normal) => "drum-hitnormal",
            (SampleSet::Drum, Hitsound::Whistle) => "drum-hitwhistle",
            (SampleSet::Drum, Hitsound::Finish) => "drum-hitfinish",
            (SampleSet::Drum, Hitsound::Clap) => "drum-hitclap",
        }
    }

    pub fn from_stem(s: &str) -> Option<Self> {
        let (set, sound) = match s {
            "normal-hitnormal" => (SampleSet::Normal, Hitsound::Normal),
            "normal-hitwhistle" => (SampleSet::Normal, Hitsound::Whistle),
            "normal-hitfinish" => (SampleSet::Normal, Hitsound::Finish),
            "normal-hitclap" => (SampleSet::Normal, Hitsound::Clap),
            "soft-hitnormal" => (SampleSet::Soft, Hitsound::Normal),
            "soft-hitwhistle" => (SampleSet::Soft, Hitsound::Whistle),
            "soft-hitfinish" => (SampleSet::Soft, Hitsound::Finish),
            "soft-hitclap" => (SampleSet::Soft, Hitsound::Clap),
            "drum-hitnormal" => (SampleSet::Drum, Hitsound::Normal),
            "drum-hitwhistle" => (SampleSet::Drum, Hitsound::Whistle),
            "drum-hitfinish" => (SampleSet::Drum, Hitsound::Finish),
            "drum-hitclap" => (SampleSet::Drum, Hitsound::Clap),
            _ => return None,
        };
        Some(Self { set, sound })
    }

    pub fn filename(self, index: u32, ext: &str) -> String {
        let stem = self.stem();
        if index <= 1 {
            format!("{stem}.{ext}")
        } else {
            format!("{stem}{index}.{ext}")
        }
    }
}

impl Serialize for SlotKey {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.stem())
    }
}

impl<'de> Deserialize<'de> for SlotKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_stem(&raw)
            .ok_or_else(|| serde::de::Error::custom(format!("Unknown slot key: {raw}")))
    }
}

/// Content identity placed into one slot of a custom sample index: a canonical set of
/// `(sample_id, quantized_relative_gain_pct)` pairs or `Silent`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SlotContent {
    Silent,
    Mix {
        /// Sorted list of `(sample_key, gain_pct)` rounded to 5% steps.
        layers: Vec<(String, u8)>,
    },
}

impl SlotContent {
    /// Quantizes per-layer gains to 5% steps relative to the loudest layer (`100%`).
    pub fn from_layers(layers: &[(SampleIdentity, i32)]) -> Self {
        if layers.is_empty() {
            return Self::Silent;
        }
        let max_vol = layers.iter().map(|(_, v)| (*v).max(1)).max().unwrap_or(100);
        let mut entries: BTreeSet<(String, u8)> = BTreeSet::new();
        for (id, vol) in layers {
            let rel = ((*vol).max(0) as f64 / max_vol as f64 * 100.0).round() as u32;
            let quantized = ((rel + 2) / 5 * 5).clamp(5, 100) as u8;
            entries.insert((sample_identity_key(id), quantized));
        }
        Self::Mix {
            layers: entries.into_iter().collect(),
        }
    }
}

fn sample_identity_key(id: &SampleIdentity) -> String {
    match id {
        SampleIdentity::Standard { set, sound, index } => {
            format!("std:{set:?}:{sound:?}:{index:?}")
        }
        SampleIdentity::File(name) => format!("file:{}", name.to_ascii_lowercase()),
    }
}

/// One 12-slot requirement or allocated custom index bucket.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomIndexBucket {
    pub index: u32,
    pub slots: BTreeMap<SlotKey, SlotContent>,
}

impl CustomIndexBucket {
    /// Returns `true` if every non-empty slot in `req` matches this bucket's slot exactly.
    pub fn fits(&self, req: &BTreeMap<SlotKey, SlotContent>) -> bool {
        req.iter().all(|(k, v)| self.slots.get(k) == Some(v))
    }

    /// Returns `true` if `req` has no conflicting slot contents with this bucket.
    pub fn can_merge(&self, req: &BTreeMap<SlotKey, SlotContent>) -> bool {
        req.iter()
            .all(|(k, v)| self.slots.get(k).is_none_or(|mine| mine == v))
    }

    /// Merges all slot requirements from `req` into this bucket.
    pub fn merge_in(&mut self, req: &BTreeMap<SlotKey, SlotContent>) {
        for (k, v) in req {
            self.slots.entry(*k).or_insert_with(|| v.clone());
        }
    }
}

/// Mapset-wide custom sample index pool with optional schema persistence and `allow_growth` guard.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomIndexPool {
    pub first_index: u32,
    pub buckets: Vec<CustomIndexBucket>,
}

impl CustomIndexPool {
    pub fn new(first_index: u32) -> Self {
        Self {
            first_index: first_index.max(1),
            buckets: Vec::new(),
        }
    }

    /// Builds or extends a mapset-wide 12-slot pool from `requirements` (one slot map per moment
    /// across all enabled difficulties).
    ///
    /// - Reuses existing `saved` buckets first so index numbers stay stable across repeat exports.
    /// - Greedily merges new requirements into the first compatible bucket (most-constrained first).
    /// - When `allow_growth == false`, returns an error if any requirement does not fit the saved
    ///   schema without allocating a new custom index.
    pub fn build_schema(
        requirements: &[BTreeMap<SlotKey, SlotContent>],
        saved: Option<&CustomIndexPool>,
        first_index: u32,
        allow_growth: bool,
    ) -> Result<Self, String> {
        let first_index = first_index.max(1);
        let mut pool = saved
            .cloned()
            .unwrap_or_else(|| CustomIndexPool::new(first_index));
        pool.first_index = pool.first_index.max(first_index);

        // Sort distinct requirements largest-first (more constrained slots packed first).
        let mut unique: Vec<&BTreeMap<SlotKey, SlotContent>> = Vec::new();
        for req in requirements {
            if req.is_empty() {
                continue;
            }
            if !unique.iter().any(|u| *u == req) {
                unique.push(req);
            }
        }
        unique.sort_by(|a, b| b.len().cmp(&a.len()));

        let mut next_idx = pool
            .buckets
            .iter()
            .map(|b| b.index)
            .max()
            .map_or(pool.first_index, |m| m.saturating_add(1).max(pool.first_index));

        for req in unique {
            if pool.buckets.iter().any(|b| b.fits(req)) {
                continue;
            }
            if !allow_growth {
                return Err(
                    "Export requires a new custom sample index, but schema growth is locked (allow_growth = false)."
                        .into(),
                );
            }
            if let Some(target) = pool.buckets.iter_mut().find(|b| b.can_merge(req)) {
                target.merge_in(req);
            } else {
                let mut bucket = CustomIndexBucket {
                    index: next_idx,
                    slots: BTreeMap::new(),
                };
                bucket.merge_in(req);
                pool.buckets.push(bucket);
                next_idx = next_idx.saturating_add(1);
            }
        }

        Ok(pool)
    }

    /// Assigns one custom sample index to each chronological moment in `moments` using greedy
    /// longest-run selection so consecutive moments stay on the same index as long as possible,
    /// minimizing `[TimingPoints]` green-line switches.
    pub fn assign_longest_runs(
        &self,
        moments: &[BTreeMap<SlotKey, SlotContent>],
    ) -> Result<Vec<u32>, String> {
        let mut out = Vec::with_capacity(moments.len());
        let mut i = 0usize;
        while i < moments.len() {
            if moments[i].is_empty() {
                let carry = out.last().copied().unwrap_or(0);
                out.push(carry);
                i += 1;
                continue;
            }
            // Pick the bucket that fits the longest contiguous run starting at `i`.
            let mut best: Option<(u32, usize)> = None;
            for bucket in &self.buckets {
                if !bucket.fits(&moments[i]) {
                    continue;
                }
                let mut run = 1usize;
                while i + run < moments.len()
                    && (moments[i + run].is_empty() || bucket.fits(&moments[i + run]))
                {
                    run += 1;
                }
                if best.is_none_or(|(_, best_run)| run > best_run) {
                    best = Some((bucket.index, run));
                }
            }
            let Some((chosen_idx, run_len)) = best else {
                return Err(format!(
                    "Moment #{i} did not fit any custom index bucket in the mapset schema."
                ));
            };
            for _ in 0..run_len {
                out.push(chosen_idx);
            }
            i += run_len;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(set: SampleSet, sound: Hitsound) -> SlotKey {
        SlotKey { set, sound }
    }

    fn mix(name: &str) -> SlotContent {
        SlotContent::Mix {
            layers: vec![(format!("file:{name}"), 100)],
        }
    }

    #[test]
    fn compatible_slots_merge_into_one_index_and_clashes_split() {
        let r1 = BTreeMap::from([(slot(SampleSet::Drum, Hitsound::Normal), mix("kick.wav"))]);
        let r2 = BTreeMap::from([(slot(SampleSet::Drum, Hitsound::Clap), mix("snare.wav"))]);
        let r3 = BTreeMap::from([(slot(SampleSet::Drum, Hitsound::Normal), mix("kick2.wav"))]);

        let pool = CustomIndexPool::build_schema(&[r1.clone(), r2.clone(), r3.clone()], None, 1, true)
            .unwrap();
        assert_eq!(pool.buckets.len(), 2, "kick + snare share index 1; kick2 needs index 2");
        assert_eq!(pool.buckets[0].index, 1);
        assert_eq!(pool.buckets[1].index, 2);
        assert!(pool.buckets[0].fits(&r1));
        assert!(pool.buckets[0].fits(&r2));
        assert!(pool.buckets[1].fits(&r3));
    }

    #[test]
    fn longest_run_assignment_minimizes_index_switches() {
        let kick = BTreeMap::from([(slot(SampleSet::Drum, Hitsound::Normal), mix("kick.wav"))]);
        let clap_a = BTreeMap::from([(slot(SampleSet::Drum, Hitsound::Clap), mix("clap_a.wav"))]);
        let clap_b = BTreeMap::from([(slot(SampleSet::Drum, Hitsound::Clap), mix("clap_b.wav"))]);

        let mut b1 = CustomIndexBucket { index: 1, slots: BTreeMap::new() };
        b1.merge_in(&kick);
        b1.merge_in(&clap_a);
        let mut b2 = CustomIndexBucket { index: 2, slots: BTreeMap::new() };
        b2.merge_in(&kick);
        b2.merge_in(&clap_b);
        let pool = CustomIndexPool { first_index: 1, buckets: vec![b1, b2] };

        let seq = vec![kick.clone(), clap_b.clone(), kick.clone(), clap_b.clone()];
        let assigned = pool.assign_longest_runs(&seq).unwrap();
        assert_eq!(assigned, vec![2, 2, 2, 2], "picks bucket 2 once for entire 4-hit run");
    }

    #[test]
    fn allow_growth_false_refuses_new_indices() {
        let r1 = BTreeMap::from([(slot(SampleSet::Normal, Hitsound::Normal), mix("a.wav"))]);
        let r2 = BTreeMap::from([(slot(SampleSet::Normal, Hitsound::Normal), mix("b.wav"))]);
        let saved = CustomIndexPool::build_schema(&[r1.clone()], None, 1, true).unwrap();
        let locked_ok = CustomIndexPool::build_schema(&[r1], Some(&saved), 1, false);
        assert!(locked_ok.is_ok());
        let locked_err = CustomIndexPool::build_schema(&[r2], Some(&saved), 1, false);
        assert!(locked_err.is_err());
    }

    #[test]
    fn addition_only_normal_resolves_all_six_policies() {
        assert_eq!(
            AdditionOnlyNormal::Difficulty.resolve_normal_set(SampleSet::Soft, SampleSet::Drum),
            Some(SampleSet::Soft)
        );
        assert_eq!(
            AdditionOnlyNormal::AdditionsSet.resolve_normal_set(SampleSet::Soft, SampleSet::Drum),
            Some(SampleSet::Drum)
        );
        assert_eq!(
            AdditionOnlyNormal::SkinNormal.resolve_normal_set(SampleSet::Soft, SampleSet::Drum),
            Some(SampleSet::Normal)
        );
        assert_eq!(
            AdditionOnlyNormal::SkinSoft.resolve_normal_set(SampleSet::Normal, SampleSet::Drum),
            Some(SampleSet::Soft)
        );
        assert_eq!(
            AdditionOnlyNormal::SkinDrum.resolve_normal_set(SampleSet::Normal, SampleSet::Soft),
            Some(SampleSet::Drum)
        );
        assert_eq!(
            AdditionOnlyNormal::Silent.resolve_normal_set(SampleSet::Normal, SampleSet::Drum),
            None
        );
        assert_eq!(STANDARD_SILENT_WAV.len(), 44);
    }
}
