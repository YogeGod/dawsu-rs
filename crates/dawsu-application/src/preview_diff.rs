//! Cached sibling-difficulty projection for Live Preview Diff and the 28 px 4-Diff Mini-Ribbon.
//!
//! Caches parsed sibling `.osu` files by `(path, mtime, len)`, handles non-standard modes
//! (taiko/catch/mania) cleanly via `PreviewDiffStatus::UnsupportedMode`, and projects up to
//! 4 sibling difficulties into bucketed `[matched, unmatched, silent]` coverage strips.

use dawsu_osu_format::{OsuFile, ObjectKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

/// Maximum cached sibling `.osu` entries so repeated opens never leak memory unboundedly.
pub const MAX_PREVIEW_DIFF_CACHE_ENTRIES: usize = 64;
/// Maximum readable `.osu` size for ribbon projection (32 MiB).
const MAX_SIBLING_OSU_BYTES: u64 = 32 * 1024 * 1024;

/// Which difficulty playback audio focuses on while a Live Preview Diff overlay is active.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewAudioFocus {
    #[default]
    Source,
    PreviewDiff,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewDiffStatus {
    Ready,
    UnsupportedMode { mode: i32 },
    Unreadable { message: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RibbonBucket {
    pub matched: u16,
    pub unmatched: u16,
    pub silent: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRibbonRow {
    pub path: PathBuf,
    pub version: String,
    pub status: PreviewDiffStatus,
    pub buckets: Vec<RibbonBucket>,
}

#[derive(Clone, Debug)]
struct CachedDiff {
    mtime: Option<SystemTime>,
    len: u64,
    version: String,
    mode: i32,
    object_times_ms: Vec<f64>,
}

/// In-memory sibling-difficulty cache keyed by `(path, mtime, len)`.
#[derive(Default)]
pub struct PreviewDiffCache {
    entries: HashMap<PathBuf, CachedDiff>,
}

impl PreviewDiffCache {
    fn parse_sibling(path: &Path) -> Result<CachedDiff, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let mtime = meta.modified().ok();
        let len = meta.len();
        if len > MAX_SIBLING_OSU_BYTES {
            return Err(format!("Difficulty file is too large to preview ({len} bytes)"));
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let osu = OsuFile::read(bytes).file;
        let mode = osu
            .general("Mode")
            .and_then(|v| v.trim().parse::<i32>().ok())
            .unwrap_or(0);
        let version = osu
            .value("Metadata", "Version")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Difficulty")
                    .to_string()
            });
        let object_times_ms: Vec<f64> = if mode == 0 {
            osu.hit_objects()
                .iter()
                .filter_map(|obj| match obj.kind() {
                    ObjectKind::Circle | ObjectKind::Slider(_) | ObjectKind::Spinner { .. } => {
                        Some(obj.time())
                    }
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        Ok(CachedDiff {
            mtime,
            len,
            version,
            mode,
            object_times_ms,
        })
    }

    fn insert_bounded(&mut self, path: PathBuf, diff: CachedDiff) {
        if self.entries.len() >= MAX_PREVIEW_DIFF_CACHE_ENTRIES && !self.entries.contains_key(&path) {
            self.entries.clear();
        }
        self.entries.insert(path, diff);
    }

    pub fn load_sibling(&mut self, path: &Path) -> Result<(String, PreviewDiffStatus, Vec<f64>), String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        let mtime = meta.modified().ok();
        let len = meta.len();
        if let Some(hit) = self.entries.get(path)
            && hit.mtime == mtime
            && hit.len == len
        {
            let status = if hit.mode == 0 {
                PreviewDiffStatus::Ready
            } else {
                PreviewDiffStatus::UnsupportedMode { mode: hit.mode }
            };
            return Ok((hit.version.clone(), status, hit.object_times_ms.clone()));
        }
        let parsed = Self::parse_sibling(path)?;
        let status = if parsed.mode == 0 {
            PreviewDiffStatus::Ready
        } else {
            PreviewDiffStatus::UnsupportedMode { mode: parsed.mode }
        };
        let result = (parsed.version.clone(), status, parsed.object_times_ms.clone());
        self.insert_bounded(path.to_path_buf(), parsed);
        Ok(result)
    }

    /// Validates that `path` is a `.osu` file inside `allowed_root` (preventing path traversal).
    pub fn validate_sibling_path(allowed_root: &Path, path: &Path) -> Result<PathBuf, String> {
        let is_osu = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("osu"));
        if !is_osu {
            return Err("Only .osu difficulties can be previewed.".to_string());
        }
        let root_canon = std::fs::canonicalize(allowed_root).map_err(|e| e.to_string())?;
        let path_canon = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        if path_canon.parent() != Some(root_canon.as_path()) {
            return Err("Sibling difficulty must reside inside the open mapset folder.".to_string());
        }
        Ok(path_canon)
    }

    /// Projects up to 4 sibling difficulties without holding `cache` across disk reads or parsing.
    pub fn project_ribbon_with_lock(
        cache: &Mutex<Self>,
        allowed_root: &Path,
        siblings: &[PathBuf],
        source_hit_times_ms: &[f64],
        duration_ms: f64,
        tolerance_ms: f64,
        bin_count: usize,
    ) -> Vec<DiffRibbonRow> {
        let bins = bin_count.clamp(16, 256);
        let span = duration_ms.max(1.0);
        let tol = tolerance_ms.max(0.0);
        let mut sorted_hits = source_hit_times_ms.to_vec();
        sorted_hits.sort_by(f64::total_cmp);

        siblings
            .iter()
            .take(4)
            .map(|raw_path| {
                let canon = match Self::validate_sibling_path(allowed_root, raw_path) {
                    Ok(p) => p,
                    Err(message) => {
                        return DiffRibbonRow {
                            path: raw_path.clone(),
                            version: raw_path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Difficulty")
                                .to_string(),
                            status: PreviewDiffStatus::Unreadable { message },
                            buckets: Vec::new(),
                        };
                    }
                };
                let meta = match std::fs::metadata(&canon) {
                    Ok(m) => m,
                    Err(e) => {
                        return DiffRibbonRow {
                            path: raw_path.clone(),
                            version: raw_path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Difficulty")
                                .to_string(),
                            status: PreviewDiffStatus::Unreadable { message: e.to_string() },
                            buckets: Vec::new(),
                        };
                    }
                };
                let mtime = meta.modified().ok();
                let len = meta.len();
                let cached = cache.lock().ok().and_then(|guard| {
                    guard
                        .entries
                        .get(&canon)
                        .filter(|hit| hit.mtime == mtime && hit.len == len)
                        .cloned()
                });
                let loaded = match cached {
                    Some(hit) => Ok(hit),
                    None => match Self::parse_sibling(&canon) {
                        Ok(parsed) => {
                            if let Ok(mut guard) = cache.lock() {
                                guard.insert_bounded(canon.clone(), parsed.clone());
                            }
                            Ok(parsed)
                        }
                        Err(e) => Err(e),
                    },
                };
                match loaded {
                    Ok(diff) if diff.mode == 0 => {
                        let mut buckets = vec![RibbonBucket::default(); bins];
                        for t in diff.object_times_ms {
                            let idx = ((t / span) * (bins as f64))
                                .floor()
                                .clamp(0.0, (bins - 1) as f64) as usize;
                            if has_hit_near(&sorted_hits, t, tol) {
                                buckets[idx].matched = buckets[idx].matched.saturating_add(1);
                            } else {
                                buckets[idx].unmatched = buckets[idx].unmatched.saturating_add(1);
                            }
                        }
                        DiffRibbonRow {
                            path: raw_path.clone(),
                            version: diff.version,
                            status: PreviewDiffStatus::Ready,
                            buckets,
                        }
                    }
                    Ok(diff) => DiffRibbonRow {
                        path: raw_path.clone(),
                        version: diff.version,
                        status: PreviewDiffStatus::UnsupportedMode { mode: diff.mode },
                        buckets: Vec::new(),
                    },
                    Err(message) => DiffRibbonRow {
                        path: raw_path.clone(),
                        version: raw_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Difficulty")
                            .to_string(),
                        status: PreviewDiffStatus::Unreadable { message },
                        buckets: Vec::new(),
                    },
                }
            })
            .collect()
    }

    /// Projects up to 4 sibling difficulties into `bin_count` coverage buckets against `source_hit_times_ms`.
    pub fn project_ribbon(
        &mut self,
        siblings: &[PathBuf],
        source_hit_times_ms: &[f64],
        duration_ms: f64,
        tolerance_ms: f64,
        bin_count: usize,
    ) -> Vec<DiffRibbonRow> {
        let bins = bin_count.clamp(16, 256);
        let span = duration_ms.max(1.0);
        let tol = tolerance_ms.max(0.0);
        let mut sorted_hits = source_hit_times_ms.to_vec();
        sorted_hits.sort_by(f64::total_cmp);

        siblings
            .iter()
            .take(4)
            .map(|path| match self.load_sibling(path) {
                Ok((version, PreviewDiffStatus::Ready, objects)) => {
                    let mut buckets = vec![RibbonBucket::default(); bins];
                    for t in objects {
                        let idx = ((t / span) * (bins as f64)).floor().clamp(0.0, (bins - 1) as f64) as usize;
                        let matched = has_hit_near(&sorted_hits, t, tol);
                        if matched {
                            buckets[idx].matched = buckets[idx].matched.saturating_add(1);
                        } else {
                            buckets[idx].unmatched = buckets[idx].unmatched.saturating_add(1);
                        }
                    }
                    DiffRibbonRow {
                        path: path.clone(),
                        version,
                        status: PreviewDiffStatus::Ready,
                        buckets,
                    }
                }
                Ok((version, status, _)) => DiffRibbonRow {
                    path: path.clone(),
                    version,
                    status,
                    buckets: Vec::new(),
                },
                Err(message) => DiffRibbonRow {
                    path: path.clone(),
                    version: path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Difficulty")
                        .to_string(),
                    status: PreviewDiffStatus::Unreadable { message },
                    buckets: Vec::new(),
                },
            })
            .collect()
    }
}

fn has_hit_near(sorted: &[f64], target: f64, tol: f64) -> bool {
    let idx = sorted.partition_point(|&t| t < target - tol);
    sorted.get(idx).is_some_and(|&t| (t - target).abs() <= tol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caches_by_mtime_and_handles_unsupported_mode() {
        let dir = std::env::temp_dir().join(format!("dawsu-preview-diff-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let std_path = dir.join("std.osu");
        let mania_path = dir.join("mania.osu");
        std::fs::write(
            &std_path,
            "osu file format v14\n[General]\nMode: 0\n[Metadata]\nVersion:Hard\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n",
        )
        .unwrap();
        std::fs::write(
            &mania_path,
            "osu file format v14\n[General]\nMode: 3\n[Metadata]\nVersion:4K\n[HitObjects]\n64,192,1000,1,0,0:0:0:0:\n",
        )
        .unwrap();

        let mut cache = PreviewDiffCache::default();
        let rows = cache.project_ribbon(&[std_path.clone(), mania_path.clone()], &[1002.0], 4000.0, 5.0, 32);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].status, PreviewDiffStatus::Ready);
        assert_eq!(rows[0].buckets.iter().map(|b| b.matched as u32).sum::<u32>(), 1);
        assert_eq!(rows[0].buckets.iter().map(|b| b.unmatched as u32).sum::<u32>(), 1);
        assert_eq!(rows[1].status, PreviewDiffStatus::UnsupportedMode { mode: 3 });

        // Second call hits cache cleanly.
        let rows2 = cache.project_ribbon(std::slice::from_ref(&std_path), &[1002.0], 4000.0, 5.0, 32);
        assert_eq!(rows2[0].version, "Hard");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
