//! Offline sample fingerprinting, persistent cache (`.dawsufp`), tag classification, and
//! acoustic similarity ranking for the Sample Browser.
use crate::drum_classify::{AcousticFeatures, classify_sample_tag, extract_features};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Decode at most 1.5 seconds (@ 48 kHz) per sample so multi-minute background songs never OOM.
pub const MAX_FINGERPRINT_FRAMES: usize = 72_000;
pub const MAX_QUERY_RESULTS: usize = 500;
pub const FINGERPRINT_DIM: usize = 11;
const CACHE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleFingerprint {
    pub rel_path: String,
    pub size: u64,
    pub mtime_ms: i128,
    pub tag: String,
    pub duration_ms: f64,
    pub vector: [f32; FINGERPRINT_DIM],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PersistedCache {
    version: u32,
    entries: Vec<SampleFingerprint>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleQueryHit {
    pub rel_path: String,
    pub tag: String,
    pub duration_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleQueryResult {
    pub total_indexed: usize,
    pub tags: Vec<String>,
    pub entries: Vec<SampleQueryHit>,
}

#[derive(Clone, Debug, Default)]
pub struct SampleFingerprintIndex {
    entries: Vec<SampleFingerprint>,
    by_path: HashMap<String, usize>,
}

impl SampleFingerprintIndex {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entry(&self, rel_path: &str) -> Option<&SampleFingerprint> {
        self.by_path
            .get(rel_path)
            .and_then(|&idx| self.entries.get(idx))
    }

    pub fn from_entries(mut entries: Vec<SampleFingerprint>) -> Self {
        entries.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
        let mut by_path = HashMap::with_capacity(entries.len());
        for (idx, item) in entries.iter().enumerate() {
            by_path.insert(item.rel_path.clone(), idx);
        }
        Self { entries, by_path }
    }

    pub fn cache_file_path(root: &Path, cache_dir: &Path) -> PathBuf {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in root.to_string_lossy().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        cache_dir.join(format!("{hash:016x}.dawsufp"))
    }

    /// Walks `root`, reuses cached fingerprints whose `(rel_path, size, mtime_ms)` match live
    /// `std::fs::metadata(&path)`, analyzes only new/changed files up to `MAX_FINGERPRINT_FRAMES`,
    /// and persists updated cache under `cache_dir`.
    pub fn load_or_update(root: &Path, cache_dir: &Path, max_files: usize) -> Self {
        let mut existing: HashMap<String, SampleFingerprint> = HashMap::new();
        let cache_file = Self::cache_file_path(root, cache_dir);
        if let Ok(bytes) = std::fs::read(&cache_file)
            && let Ok(parsed) = serde_json::from_slice::<PersistedCache>(&bytes)
            && parsed.version == CACHE_VERSION
        {
            for entry in parsed.entries {
                existing.insert(entry.rel_path.clone(), entry);
            }
        }

        let mut next_entries = Vec::new();
        let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
        while let Some((dir, rel_dir)) = stack.pop() {
            let Ok(read_dir) = std::fs::read_dir(&dir) else {
                continue;
            };
            let mut subdirs = Vec::new();
            let mut files = Vec::new();
            for item in read_dir.flatten() {
                // Never use `DirEntry::metadata()` on Windows NTFS ($I30 cache can be stale).
                let Ok(ft) = item.file_type() else { continue };
                let name = item.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let rel = if rel_dir.is_empty() {
                    name.clone()
                } else {
                    format!("{rel_dir}/{name}")
                };
                if ft.is_dir() {
                    subdirs.push((item.path(), rel));
                } else if ft.is_file() && is_audio_file(&name) {
                    files.push((item.path(), rel));
                }
            }
            subdirs.sort_by(|a, b| b.1.cmp(&a.1));
            stack.extend(subdirs);
            files.sort_by(|a, b| a.1.cmp(&b.1));

            for (path, rel_path) in files {
                if next_entries.len() >= max_files {
                    break;
                }
                let Ok(meta) = std::fs::metadata(&path) else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let size = meta.len();
                let mtime_ms = meta
                    .modified()
                    .ok()
                    .map_or(0, system_time_to_ms);
                if let Some(cached) = existing.get(&rel_path)
                    && cached.size == size
                    && cached.mtime_ms == mtime_ms
                {
                    next_entries.push(cached.clone());
                    continue;
                }
                let fp = fingerprint_file(&path, rel_path, size, mtime_ms);
                next_entries.push(fp);
            }
            if next_entries.len() >= max_files {
                break;
            }
        }

        let index = Self::from_entries(next_entries);
        if let Some(parent) = cache_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let persisted = PersistedCache {
            version: CACHE_VERSION,
            entries: index.entries.clone(),
        };
        if let Ok(json) = serde_json::to_vec(&persisted) {
            let _ = std::fs::write(&cache_file, json);
        }
        index
    }

    pub fn query(
        &self,
        tag: Option<&str>,
        similar_to: Option<&str>,
        filter: Option<&str>,
        limit: usize,
    ) -> SampleQueryResult {
        let cap = limit.clamp(1, MAX_QUERY_RESULTS);
        let filter_lower = filter
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);
        let tag_filter = tag.map(str::trim).filter(|s| !s.is_empty());
        let anchor = similar_to
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|rel| self.entry(rel));

        let mut tags_set = Vec::new();
        for entry in &self.entries {
            if !tags_set.contains(&entry.tag) {
                tags_set.push(entry.tag.clone());
            }
        }
        tags_set.sort();

        let mut scored: Vec<(f32, &SampleFingerprint)> = self
            .entries
            .iter()
            .filter(|entry| {
                tag_filter.is_none_or(|wanted| entry.tag.eq_ignore_ascii_case(wanted))
                    && filter_lower.as_deref().is_none_or(|needle| {
                        entry.rel_path.to_ascii_lowercase().contains(needle)
                    })
            })
            .map(|entry| {
                let sim = anchor.map_or(1.0, |ref_fp| similarity_score(&ref_fp.vector, &entry.vector));
                (sim, entry)
            })
            .collect();

        if anchor.is_some() {
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.rel_path.cmp(&b.1.rel_path)));
        }
        scored.truncate(cap);

        let entries = scored
            .into_iter()
            .map(|(sim, entry)| SampleQueryHit {
                rel_path: entry.rel_path.clone(),
                tag: entry.tag.clone(),
                duration_ms: entry.duration_ms,
                similarity: anchor.map(|_| sim),
            })
            .collect();

        SampleQueryResult {
            total_indexed: self.entries.len(),
            tags: tags_set,
            entries,
        }
    }
}

pub(crate) fn features_to_vector(f: Option<AcousticFeatures>) -> [f32; FINGERPRINT_DIM] {
    let Some(f) = f else {
        return [0.0; FINGERPRINT_DIM];
    };
    let clean = |v: f32| if v.is_finite() { v } else { 0.0 };
    [
        clean((f.decay_ms.max(1.0).ln() / 7.5).clamp(0.0, 1.5)),
        clean(f.sub_low.clamp(0.0, 1.0)),
        clean(f.low_mid.clamp(0.0, 1.0)),
        clean(f.mid.clamp(0.0, 1.0)),
        clean(f.high_mid.clamp(0.0, 1.0)),
        clean(f.high.clamp(0.0, 1.0)),
        clean(((f.centroid_hz / 20.0).max(1.0).ln() / 7.0).clamp(0.0, 1.5)),
        clean(((f.peak_hz / 20.0).max(1.0).ln() / 6.0).clamp(0.0, 1.5)),
        clean(f.flatness.clamp(0.0, 1.0)),
        clean((f.zcr * 2.0).clamp(0.0, 1.0)),
        clean((f.crest_factor.max(1.0).ln() / 3.4).clamp(0.0, 1.2)),
    ]
}

fn similarity_score(a: &[f32; FINGERPRINT_DIM], b: &[f32; FINGERPRINT_DIM]) -> f32 {
    let dist_sq: f32 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let d = x - y;
            d * d
        })
        .sum();
    1.0 / (1.0 + dist_sq.sqrt())
}

fn fingerprint_file(path: &Path, rel_path: String, size: u64, mtime_ms: i128) -> SampleFingerprint {
    let clip = dawsu_audio::decode_prefix(path, MAX_FINGERPRINT_FRAMES).ok();
    let features = clip.as_deref().and_then(extract_features);
    let tag = classify_sample_tag(&rel_path, features).to_owned();
    let duration_ms = clip.as_deref().map_or(0.0, dawsu_audio::AudioClip::duration_ms);
    let vector = features_to_vector(features);
    SampleFingerprint {
        rel_path,
        size,
        mtime_ms,
        tag,
        duration_ms,
        vector,
    }
}

fn is_audio_file(name: &str) -> bool {
    Path::new(name).extension().is_some_and(|ext| {
        ["wav", "ogg", "mp3"]
            .iter()
            .any(|candidate| ext.eq_ignore_ascii_case(candidate))
    })
}

fn system_time_to_ms(ts: SystemTime) -> i128 {
    match ts.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i128,
        Err(e) => -(e.duration().as_millis() as i128),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_test_wav(path: &Path, rate: u32, synth: impl FnMut(usize) -> f32, frames: usize) {
        let samples: Vec<f32> = (0..frames).map(synth).collect();
        let bytes = crate::sample_checks::encode_pcm16_wav(&samples, rate, 1).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn cache_reuses_unchanged_and_invalidates_after_windows_mtime_bump() {
        let scratch = dawsu_test_support::Scratch::new("fp-mtime");
        let lib = scratch.path("library");
        let cache_dir = scratch.path("cache");
        std::fs::create_dir_all(&lib).unwrap();
        let kick_path = lib.join("punch.wav");
        encode_test_wav(
            &kick_path,
            44_100,
            |i| {
                let t = i as f32 / 44_100.0;
                (2.0 * std::f32::consts::PI * 65.0 * t).sin() * (-t * 18.0).exp() * 0.9
            },
            4410,
        );
        let idx1 = SampleFingerprintIndex::load_or_update(&lib, &cache_dir, 200);
        assert_eq!(idx1.len(), 1);
        let fp1 = idx1.entry("punch.wav").unwrap().clone();
        assert_eq!(fp1.tag, "Kick");

        // Second scan reuses cached entry cleanly.
        let idx2 = SampleFingerprintIndex::load_or_update(&lib, &cache_dir, 200);
        assert_eq!(idx2.entry("punch.wav").unwrap(), &fp1);

        // Sleep > 30 ms for Windows ~15.6 ms NTFS clock granularity before rewriting with hi-hat audio.
        std::thread::sleep(std::time::Duration::from_millis(40));
        encode_test_wav(
            &kick_path,
            44_100,
            |i| {
                let t = i as f32 / 44_100.0;
                let noise = if i % 2 == 0 { 0.85 } else { -0.85 };
                noise * (-t * 40.0).exp()
            },
            4410,
        );
        let idx3 = SampleFingerprintIndex::load_or_update(&lib, &cache_dir, 200);
        let fp3 = idx3.entry("punch.wav").unwrap();
        assert_ne!(fp3.mtime_ms, fp1.mtime_ms, "expected live mtime change");
        assert_eq!(fp3.tag, "Hat", "updated file audio should re-classify tag");
    }

    #[test]
    fn similarity_search_ranks_kick_near_kick_and_filters_500_samples_under_50ms() {
        let mut entries = Vec::with_capacity(500);
        let kick_vec = [0.4, 0.75, 0.15, 0.05, 0.03, 0.02, 0.25, 0.20, 0.05, 0.04, 0.5];
        let hat_vec = [0.25, 0.01, 0.02, 0.10, 0.35, 0.52, 0.95, 0.80, 0.65, 0.70, 0.7];
        entries.push(SampleFingerprint {
            rel_path: "kicks/anchor-kick.wav".into(),
            size: 1000,
            mtime_ms: 1,
            tag: "Kick".into(),
            duration_ms: 150.0,
            vector: kick_vec,
        });
        entries.push(SampleFingerprint {
            rel_path: "kicks/close-kick.wav".into(),
            size: 1000,
            mtime_ms: 1,
            tag: "Kick".into(),
            duration_ms: 150.0,
            vector: [0.42, 0.72, 0.16, 0.06, 0.03, 0.03, 0.26, 0.21, 0.06, 0.04, 0.49],
        });
        for i in 2..500 {
            entries.push(SampleFingerprint {
                rel_path: format!("hats/hat-{i:03}.wav"),
                size: 1000,
                mtime_ms: 1,
                tag: "Hat".into(),
                duration_ms: 80.0,
                vector: hat_vec,
            });
        }
        let idx = SampleFingerprintIndex::from_entries(entries);
        let t0 = std::time::Instant::now();
        let res = idx.query(None, Some("kicks/anchor-kick.wav"), None, 500);
        let elapsed = t0.elapsed();
        assert_eq!(res.entries.len(), 500);
        assert_eq!(res.entries[0].rel_path, "kicks/anchor-kick.wav");
        assert_eq!(res.entries[1].rel_path, "kicks/close-kick.wav");
        assert!(elapsed.as_millis() < 50, "500-item similarity query took {:?}", elapsed);
    }
}
