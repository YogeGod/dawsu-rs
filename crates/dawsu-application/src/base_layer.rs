use crate::project::{lane_identity, lane_key, path_digest};
use crate::{Application, Document, songs_index};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_hitsounding::volume_envelope::{VolumeEnvelope, apply_envelope_to_osu};
use dawsu_hitsounding::{Hitsounds, MomentId, Place};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

type ResolvedBaseHit = (SampleIdentity, i32);
type BestBaseMomentMatch = (MomentId, f64, i64, i64, Vec<ResolvedBaseHit>);
type PlannedBaseMoment = (MomentId, i64, i64, Vec<ResolvedBaseHit>);
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const SHARED_BASE_VERSION: u32 = 1;
pub const DEFAULT_LENIENCY_MS: f64 = 5.0;
pub const MAX_LENIENCY_MS: f64 = 50.0;

/// Inclusive time range in milliseconds for a per-difficulty Shared Base lane rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRangeMs {
    pub start_ms: i64,
    pub end_ms: i64,
}

impl TimeRangeMs {
    pub fn contains(self, time_ms: i64) -> bool {
        let (lo, hi) = if self.start_ms <= self.end_ms {
            (self.start_ms, self.end_ms)
        } else {
            (self.end_ms, self.start_ms)
        };
        (lo..=hi).contains(&time_ms)
    }
}

/// Non-destructive per-difficulty rule applied on top of the Shared Base score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BaseRule {
    /// Hide a lane in this difficulty, optionally only inside `range`.
    MuteLane {
        lane: String,
        #[serde(default)]
        range: Option<TimeRangeMs>,
    },
    /// Replace `from` with `to` in this difficulty, optionally only inside `range`.
    ReplaceLane {
        from: String,
        to: String,
        #[serde(default)]
        range: Option<TimeRangeMs>,
    },
    /// Add a difficulty-only hit on `lane` at `time_ms`.
    AddHit {
        lane: String,
        #[serde(rename = "timeMs", alias = "time_ms")]
        time_ms: i64,
        volume: u8,
    },
    /// Remove a Shared Base hit on `lane` at `time_ms` in this difficulty only.
    RemoveHit {
        lane: String,
        #[serde(rename = "timeMs", alias = "time_ms")]
        time_ms: i64,
    },
    /// Override a slider body's sample set and whistle in this difficulty.
    SliderBody {
        #[serde(rename = "sliderTimeMs", alias = "slider_time_ms")]
        slider_time_ms: i64,
        #[serde(rename = "sampleSet", alias = "sample_set")]
        sample_set: String,
        whistle: bool,
    },
}

/// Live projection summary for one difficulty tab inside the open mapset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffProjectionTab {
    pub file: String,
    pub path: String,
    pub version: String,
    pub matched: usize,
    pub empty: usize,
    pub orphaned: usize,
    pub rule_count: usize,
    pub is_active: bool,
    pub has_volume_envelope: bool,
}

/// Outcome of relinking a moved mapset folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelinkMapsetResult {
    pub old_path: String,
    pub new_path: String,
    pub migrated_base: bool,
    pub migrated_project: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedBaseLayer {
    pub version: u32,
    pub enabled: bool,
    pub source_difficulty: Option<String>,
    pub moments: Vec<BaseMoment>,
    #[serde(default)]
    pub overrides_by_diff: BTreeMap<String, BTreeSet<i64>>,
    /// Per-difficulty named lane/range rules applied non-destructively over `moments`.
    #[serde(default)]
    pub rules_by_diff: BTreeMap<String, Vec<BaseRule>>,
    /// Shared green-line-only master volume envelope for the mapset.
    #[serde(default)]
    pub volume_envelope: VolumeEnvelope,
    /// Optional per-difficulty green-line-only volume envelopes overriding `volume_envelope`.
    #[serde(default)]
    pub volume_envelopes_by_diff: BTreeMap<String, VolumeEnvelope>,
}

impl Default for SharedBaseLayer {
    fn default() -> Self {
        Self {
            version: SHARED_BASE_VERSION,
            enabled: false,
            source_difficulty: None,
            moments: Vec::new(),
            overrides_by_diff: BTreeMap::new(),
            rules_by_diff: BTreeMap::new(),
            volume_envelope: VolumeEnvelope::default(),
            volume_envelopes_by_diff: BTreeMap::new(),
        }
    }
}

/// Applies non-destructive per-difficulty [`BaseRule`]s to a copy of the Shared Base moments.
pub fn apply_base_rules(base_moments: &[BaseMoment], rules: &[BaseRule]) -> Vec<BaseMoment> {
    if rules.is_empty() {
        return base_moments.to_vec();
    }
    let mut by_time: BTreeMap<i64, Vec<BaseHit>> = BTreeMap::new();
    for m in base_moments {
        by_time.insert(m.time_ms, m.hits.clone());
    }
    let in_range = |range: Option<TimeRangeMs>, t: i64| range.is_none_or(|r| r.contains(t));

    for rule in rules {
        match rule {
            BaseRule::MuteLane { lane, range } => {
                for (&t, hits) in &mut by_time {
                    if in_range(*range, t) {
                        hits.retain(|h| !h.sample.eq_ignore_ascii_case(lane));
                    }
                }
            }
            BaseRule::ReplaceLane { from, to, range } => {
                for (&t, hits) in &mut by_time {
                    if in_range(*range, t) {
                        for h in hits.iter_mut() {
                            if h.sample.eq_ignore_ascii_case(from) {
                                h.sample = to.clone();
                            }
                        }
                        // Deduplicate if `to` was already present at this timestamp.
                        let mut seen = BTreeSet::new();
                        hits.retain(|h| seen.insert(h.sample.to_ascii_lowercase()));
                    }
                }
            }
            BaseRule::AddHit { lane, time_ms, volume } => {
                let hits = by_time.entry(*time_ms).or_default();
                let vol = (*volume).clamp(5, 100);
                if let Some(existing) = hits.iter_mut().find(|h| h.sample.eq_ignore_ascii_case(lane)) {
                    existing.volume = vol;
                } else {
                    hits.push(BaseHit {
                        sample: lane.clone(),
                        volume: vol,
                    });
                }
            }
            BaseRule::RemoveHit { lane, time_ms } => {
                if let Some(hits) = by_time.get_mut(time_ms) {
                    hits.retain(|h| !h.sample.eq_ignore_ascii_case(lane));
                }
            }
            BaseRule::SliderBody { .. } => {}
        }
    }

    by_time
        .into_iter()
        .filter(|(_, hits)| !hits.is_empty())
        .map(|(time_ms, hits)| BaseMoment { time_ms, hits })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseMoment {
    pub time_ms: i64,
    pub hits: Vec<BaseHit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseHit {
    pub sample: String,
    pub volume: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedBaseStatus {
    pub enabled: bool,
    pub has_base: bool,
    pub source_difficulty: Option<String>,
    pub base_moment_count: usize,
    pub current_diff_overrides: usize,
    pub current_diff_inherited: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedBaseApplyResult {
    pub applied_moments: usize,
    pub skipped_overrides: usize,
    pub unmatched_base_moments: usize,
    pub status: SharedBaseStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedBaseApplyAllResult {
    pub updated_diffs: usize,
    pub skipped_diffs: usize,
    pub total_applied_moments: usize,
    pub total_skipped_overrides: usize,
    pub failed_diffs: Vec<String>,
    pub status: SharedBaseStatus,
}

pub(crate) type MomentSnapshot = BTreeMap<i64, Vec<(String, i32)>>;

pub(crate) struct BaseLayerTrack {
    pub path: PathBuf,
    pub version: u64,
    pub snapshot: MomentSnapshot,
}

fn clamp_leniency(leniency_ms: f64) -> f64 {
    if leniency_ms.is_finite() {
        leniency_ms.clamp(0.0, MAX_LENIENCY_MS)
    } else {
        DEFAULT_LENIENCY_MS
    }
}

fn canonical_mapset_folder(map_path: &Path) -> Result<PathBuf, String> {
    let parent = map_path
        .parent()
        .ok_or_else(|| "Missing mapset folder.".to_string())?;
    Ok(parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf()))
}

pub(crate) fn storage_path(projects_root: &Path, map_path: &Path) -> Result<PathBuf, String> {
    let folder = canonical_mapset_folder(map_path)?;
    let hash = path_digest(&folder);
    Ok(projects_root.join(format!("base-{hash}.json")))
}

fn diff_key(document: &Document) -> String {
    document
        .path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| document.version().to_ascii_lowercase())
}

fn diff_label(document: &Document) -> String {
    let version = document.version();
    if !version.trim().is_empty() {
        return version;
    }
    document
        .path
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "Current difficulty".to_string())
}

fn normalize_layer_keys(layer: &mut SharedBaseLayer) {
    let mut normalized_overrides: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    for (k, times) in std::mem::take(&mut layer.overrides_by_diff) {
        normalized_overrides
            .entry(k.to_ascii_lowercase())
            .or_default()
            .extend(times);
    }
    layer.overrides_by_diff = normalized_overrides;

    let mut normalized_rules: BTreeMap<String, Vec<BaseRule>> = BTreeMap::new();
    for (k, rules) in std::mem::take(&mut layer.rules_by_diff) {
        let entry = normalized_rules.entry(k.to_ascii_lowercase()).or_default();
        for rule in rules {
            if !entry.contains(&rule) {
                entry.push(rule);
            }
        }
    }
    layer.rules_by_diff = normalized_rules;

    let mut normalized_envelopes: BTreeMap<String, VolumeEnvelope> = BTreeMap::new();
    for (k, env) in std::mem::take(&mut layer.volume_envelopes_by_diff) {
        if env.is_empty() {
            continue;
        }
        normalized_envelopes
            .entry(k.to_ascii_lowercase())
            .or_insert(env);
    }
    layer.volume_envelopes_by_diff = normalized_envelopes;
}

fn load_base(path: &Path) -> Result<Option<SharedBaseLayer>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("Could not read Shared Base file: {err}")),
    };
    let mut layer: SharedBaseLayer = serde_json::from_slice(&bytes)
        .map_err(|err| format!("Could not parse Shared Base file: {err}"))?;
    if layer.version != SHARED_BASE_VERSION {
        return Err(format!(
            "Unsupported Shared Base file version {}.",
            layer.version
        ));
    }
    normalize_layer_keys(&mut layer);
    Ok(Some(layer))
}

fn save_base(path: &Path, layer: &SharedBaseLayer) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err("Invalid Shared Base storage path.".into());
    };
    fs::create_dir_all(parent)
        .map_err(|err| format!("Could not create Shared Base folder: {err}"))?;
    let bytes = serde_json::to_vec_pretty(layer)
        .map_err(|err| format!("Could not encode Shared Base data: {err}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes).map_err(|err| format!("Could not write Shared Base file: {err}"))?;
    if let Err(rename_err) = fs::rename(&tmp, path) {
        // On Windows, fs::rename may fail if the destination already exists.
        let _ = fs::remove_file(path);
        if let Err(retry_err) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "Could not save Shared Base file: {rename_err} ({retry_err})"
            ));
        }
    }
    Ok(())
}

fn delete_base(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("Could not remove Shared Base file: {err}")),
    }
}

fn resolve_sample(key: &str) -> Option<SampleIdentity> {
    if let Some(identity) = lane_identity(key) {
        return Some(identity);
    }
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(read_sample_file_name(trimmed).unwrap_or_else(|| SampleIdentity::File(trimmed.to_owned())))
}

fn capture_moments(hitsounds: &Hitsounds) -> Vec<BaseMoment> {
    let mut by_time: BTreeMap<i64, BTreeMap<String, u8>> = BTreeMap::new();
    for moment in hitsounds.moments() {
        let mut has_any = false;
        let time_ms = moment.time().ms().round() as i64;
        for lane in moment.hits() {
            has_any = true;
            let sample = lane_key(lane);
            let volume = moment.hit_volume(lane).clamp(0, 100) as u8;
            by_time.entry(time_ms).or_default().insert(sample, volume);
        }
        if !has_any {
            continue;
        }
    }
    by_time
        .into_iter()
        .map(|(time_ms, hits)| BaseMoment {
            time_ms,
            hits: hits
                .into_iter()
                .map(|(sample, volume)| BaseHit { sample, volume })
                .collect(),
        })
        .collect()
}

pub(crate) fn snapshot_moments(hitsounds: &Hitsounds) -> MomentSnapshot {
    let mut by_time: BTreeMap<i64, BTreeMap<String, i32>> = BTreeMap::new();
    for moment in hitsounds.moments() {
        let time_ms = moment.time().ms().round() as i64;
        let entry = by_time.entry(time_ms).or_default();
        for lane in moment.hits() {
            entry.insert(lane_key(lane), moment.hit_volume(lane));
        }
    }
    by_time
        .into_iter()
        .filter(|(_, hits)| !hits.is_empty())
        .map(|(time_ms, hits)| (time_ms, hits.into_iter().collect()))
        .collect()
}

fn diff_changed_times(before: &MomentSnapshot, after: &MomentSnapshot) -> Vec<i64> {
    let mut changed = Vec::new();
    for (time_ms, prev_hits) in before {
        if after.get(time_ms) != Some(prev_hits) {
            changed.push(*time_ms);
        }
    }
    for time_ms in after.keys() {
        if !before.contains_key(time_ms) {
            changed.push(*time_ms);
        }
    }
    changed.sort_unstable();
    changed.dedup();
    changed
}

fn project_counts_for_hitsounds(
    hitsounds: &Hitsounds,
    base_moments: &[BaseMoment],
    leniency_ms: f64,
) -> (usize, usize, usize) {
    let mut matched_objects: BTreeSet<usize> = BTreeSet::new();
    let mut orphaned = 0usize;
    for bm in base_moments {
        if let Some((mid, _)) = find_matching_object_moment(hitsounds, bm.time_ms, leniency_ms) {
            matched_objects.insert(mid.0);
        } else {
            orphaned += 1;
        }
    }
    let total_objects = hitsounds
        .moments()
        .iter()
        .filter(|m| !matches!(m.place(), Place::NoObject))
        .count();
    let matched = matched_objects.len();
    let empty = total_objects.saturating_sub(matched);
    (matched, empty, orphaned)
}

fn find_matching_object_moment(
    hitsounds: &Hitsounds,
    time_ms: i64,
    leniency_ms: f64,
) -> Option<(MomentId, i64)> {
    let target = time_ms as f64;
    hitsounds
        .moments()
        .iter()
        .enumerate()
        .filter(|(_, m)| !matches!(m.place(), Place::NoObject))
        .filter(|(_, m)| (m.time().ms() - target).abs() <= leniency_ms)
        .min_by(|(_, a), (_, b)| {
            let da = (a.time().ms() - target).abs();
            let db = (b.time().ms() - target).abs();
            da.total_cmp(&db)
        })
        .map(|(idx, m)| (MomentId(idx), m.time().ms().round() as i64))
}

fn build_status(document: &Document, layer: Option<&SharedBaseLayer>) -> SharedBaseStatus {
    let Some(layer) = layer else {
        return SharedBaseStatus::default();
    };
    let key = diff_key(document);
    let overrides = layer.overrides_by_diff.get(&key);
    let current_diff_overrides = overrides.map_or(0, BTreeSet::len);
    let hitsounds = document.composer.hitsounds();
    let mut current_diff_inherited = 0usize;
    for moment in &layer.moments {
        if let Some((_, matched_ms)) =
            find_matching_object_moment(hitsounds, moment.time_ms, DEFAULT_LENIENCY_MS)
        {
            let is_overridden = overrides.is_some_and(|set| {
                set.contains(&matched_ms) || set.contains(&moment.time_ms)
            });
            if !is_overridden {
                current_diff_inherited += 1;
            }
        }
    }
    SharedBaseStatus {
        enabled: layer.enabled,
        has_base: !layer.moments.is_empty(),
        source_difficulty: layer.source_difficulty.clone(),
        base_moment_count: layer.moments.len(),
        current_diff_overrides,
        current_diff_inherited,
    }
}

impl Application {
    fn shared_base_projects_root(&self) -> PathBuf {
        self.project_root
            .clone()
            .unwrap_or_else(|| self.data_root.join("projects"))
    }

    fn shared_base_file_path(&self) -> Result<PathBuf, String> {
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        storage_path(&self.shared_base_projects_root(), &document.path)
    }

    pub fn shared_base_enabled(&self) -> bool {
        self.base_layer_track.is_some()
    }

    pub fn shared_base_status(&self) -> SharedBaseStatus {
        let Some(document) = self.document.as_ref() else {
            return SharedBaseStatus::default();
        };
        let Ok(path) = storage_path(&self.shared_base_projects_root(), &document.path) else {
            return SharedBaseStatus::default();
        };
        let Ok(layer) = load_base(&path) else {
            return SharedBaseStatus::default();
        };
        build_status(document, layer.as_ref())
    }

    pub fn shared_base_set_enabled(&mut self, enabled: bool) -> Result<SharedBaseStatus, String> {
        let path = self.shared_base_file_path()?;
        let mut layer = load_base(&path)?.unwrap_or_default();
        layer.enabled = enabled;
        save_base(&path, &layer)?;
        self.sync_shared_base_track();
        self.bump_view_revision();
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        Ok(build_status(document, Some(&layer)))
    }

    pub fn shared_base_capture_current(&mut self) -> Result<SharedBaseStatus, String> {
        if self.busy {
            return Err("Wait for the current job to finish first.".into());
        }
        let path = self.shared_base_file_path()?;
        let (moments, label, key) = {
            let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
            (
                capture_moments(document.composer.hitsounds()),
                diff_label(document),
                diff_key(document),
            )
        };
        let mut layer = load_base(&path)?.unwrap_or_default();
        layer.version = SHARED_BASE_VERSION;
        layer.enabled = true;
        layer.source_difficulty = Some(label.clone());
        layer.moments = moments;
        layer.overrides_by_diff.remove(&key);
        save_base(&path, &layer)?;
        self.sync_shared_base_track();
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        let status = build_status(document, Some(&layer));
        self.status = format!(
            "Captured {} Shared Base {} from {}.",
            status.base_moment_count,
            if status.base_moment_count == 1 {
                "moment"
            } else {
                "moments"
            },
            label,
        );
        self.bump_view_revision();
        Ok(status)
    }

    pub fn shared_base_apply_to_current(
        &mut self,
        leniency_ms: f64,
        overwrite_overrides: bool,
    ) -> Result<SharedBaseApplyResult, String> {
        if self.busy {
            return Err("Wait for the current job to finish first.".into());
        }
        let path = self.shared_base_file_path()?;
        let mut layer = load_base(&path)?
            .ok_or_else(|| "No Shared Base captured for this mapset yet.".to_string())?;
        if layer.moments.is_empty() {
            return Err("No Shared Base captured for this mapset yet.".into());
        }
        let leniency = clamp_leniency(leniency_ms);

        let document = self.document.as_mut().ok_or("Open a difficulty first.")?;
        let key = diff_key(document);
        let diff_overrides = layer.overrides_by_diff.get(&key);
        let diff_rules = layer
            .rules_by_diff
            .get(&key)
            .or_else(|| layer.rules_by_diff.get(&key.to_ascii_lowercase()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let effective_moments = apply_base_rules(&layer.moments, diff_rules);

        let mut best_by_moment: BTreeMap<usize, BestBaseMomentMatch> = BTreeMap::new();
        let mut skipped_moment_ids: BTreeSet<usize> = BTreeSet::new();
        let mut unmatched_base_moments = 0usize;

        {
            let hitsounds = document.composer.hitsounds();
            for base_moment in &effective_moments {
                let Some((moment_id, matched_ms)) =
                    find_matching_object_moment(hitsounds, base_moment.time_ms, leniency)
                else {
                    unmatched_base_moments += 1;
                    continue;
                };

                let is_overridden = diff_overrides.is_some_and(|set| {
                    set.contains(&matched_ms) || set.contains(&base_moment.time_ms)
                });
                if is_overridden && !overwrite_overrides {
                    skipped_moment_ids.insert(moment_id.0);
                    continue;
                }

                let mut resolved_hits = Vec::with_capacity(base_moment.hits.len());
                for hit in &base_moment.hits {
                    if let Some(identity) = resolve_sample(&hit.sample) {
                        resolved_hits.push((identity, i32::from(hit.volume.min(100))));
                    }
                }
                if !base_moment.hits.is_empty() && resolved_hits.is_empty() {
                    continue;
                }
                let dist =
                    (hitsounds.moment(moment_id).time().ms() - base_moment.time_ms as f64).abs();
                let replace = best_by_moment
                    .get(&moment_id.0)
                    .is_none_or(|(_, prev_dist, _, _, _)| dist < *prev_dist);
                if replace {
                    best_by_moment.insert(
                        moment_id.0,
                        (moment_id, dist, matched_ms, base_moment.time_ms, resolved_hits),
                    );
                }
            }
        }

        let skipped_overrides = skipped_moment_ids.len();
        let planned: Vec<PlannedBaseMoment> = best_by_moment
            .into_values()
            .map(|(id, _, matched_ms, base_ms, hits)| (id, matched_ms, base_ms, hits))
            .collect();
        let mut touched_lanes: BTreeSet<SampleIdentity> = BTreeSet::new();
        for (_, _, _, hits) in &planned {
            for (lane, _) in hits {
                touched_lanes.insert(lane.clone());
            }
        }

        let applied_moments = planned.len();
        if !planned.is_empty() {
            document.composer.apply("Apply Shared Base", |h| {
                for (moment_id, _, _, hits) in &planned {
                    let existing: Vec<SampleIdentity> =
                        h.moment(*moment_id).hits().cloned().collect();
                    for lane in existing {
                        h.remove_hit(*moment_id, &lane);
                    }
                    for (lane, volume) in hits {
                        h.add_hit_with_volume(*moment_id, lane.clone(), *volume);
                    }
                }
            });
            for lane in &touched_lanes {
                if !document.composer.lanes.contains(lane) {
                    document.composer.lanes.push(lane.clone());
                }
            }
        }

        if overwrite_overrides && !planned.is_empty() {
            if let Some(set) = layer.overrides_by_diff.get_mut(&key) {
                for (_, matched_ms, base_ms, _) in &planned {
                    set.remove(matched_ms);
                    set.remove(base_ms);
                }
                if set.is_empty() {
                    layer.overrides_by_diff.remove(&key);
                }
            }
            let _ = save_base(&path, &layer);
        }

        self.sync_shared_base_track();
        self.sync_audio();
        self.refresh_recovery_edit();

        let status = self
            .document
            .as_ref()
            .map(|doc| build_status(doc, Some(&layer)))
            .unwrap_or_default();
        self.status = format!(
            "Applied Shared Base to {} {} ({} skipped, {} unmatched).",
            applied_moments,
            if applied_moments == 1 {
                "moment"
            } else {
                "moments"
            },
            skipped_overrides,
            unmatched_base_moments,
        );
        self.bump_view_revision();
        Ok(SharedBaseApplyResult {
            applied_moments,
            skipped_overrides,
            unmatched_base_moments,
            status,
        })
    }

    /// Applies the mapset's Shared Base to the currently open difficulty (in memory, 1 Undo step)
    /// and to every other sibling `.osu` difficulty in the mapset folder (with read-back
    /// verification and backups). Respects per-diff overrides unless `overwrite_overrides` is true.
    #[allow(clippy::disallowed_methods)]
    pub fn shared_base_apply_to_all_diffs(
        &mut self,
        leniency_ms: f64,
        overwrite_overrides: bool,
    ) -> Result<SharedBaseApplyAllResult, String> {
        if self.busy {
            return Err("Wait for the current job to finish first.".into());
        }
        let current_result = self.shared_base_apply_to_current(leniency_ms, overwrite_overrides)?;
        let path = self.shared_base_file_path()?;
        let mut layer = load_base(&path)?
            .ok_or_else(|| "No Shared Base captured for this mapset yet.".to_string())?;
        let leniency = clamp_leniency(leniency_ms);

        let (current_path, siblings) = {
            let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
            (document.path.clone(), document.difficulties.clone())
        };
        let current_canon = current_path
            .canonicalize()
            .unwrap_or_else(|_| current_path.clone());
        let folder_files = current_path
            .parent()
            .and_then(|p| dawsu_osu_format::list_folder(p).ok())
            .unwrap_or_default();

        let mut updated_diffs = usize::from(current_result.applied_moments > 0);
        let mut skipped_diffs = usize::from(current_result.applied_moments == 0);
        let mut total_applied_moments = current_result.applied_moments;
        let mut total_skipped_overrides = current_result.skipped_overrides;
        let mut failed_diffs = Vec::new();
        let mut layer_dirty = false;
        let policy = dawsu_foundation::safe_write::BackupPolicy::new(self.backups_folder(), 20);

        for sibling_path in siblings {
            let sibling_canon = sibling_path
                .canonicalize()
                .unwrap_or_else(|_| sibling_path.clone());
            let is_same_file = if cfg!(windows) {
                sibling_canon
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&current_canon.to_string_lossy())
            } else {
                sibling_canon == current_canon
            };
            if is_same_file {
                continue;
            }
            let diff_name = sibling_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| sibling_path.display().to_string());
            let sibling_key = diff_name.to_ascii_lowercase();

            let original_bytes = match fs::read(&sibling_path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    failed_diffs.push(format!("{diff_name}: could not read ({e})"));
                    continue;
                }
            };
            let parsed = dawsu_osu_format::OsuFile::read(original_bytes.clone());
            // Skip dedicated [Hitsounds] template diffs (no real gameplay objects or named [Hitsounds]).
            if sibling_key.contains("[hitsounds]") || parsed.file.hit_objects().is_empty() {
                skipped_diffs += 1;
                continue;
            }
            let mut hits = match dawsu_hitsounding::decompile(
                &parsed.file,
                &folder_files,
                &|| false,
                &mut |_, _| {},
            ) {
                dawsu_hitsounding::Decompiled::Done(hits) => hits,
                dawsu_hitsounding::Decompiled::UnsupportedMode { .. } => {
                    skipped_diffs += 1;
                    continue;
                }
                dawsu_hitsounding::Decompiled::Cancelled => {
                    skipped_diffs += 1;
                    continue;
                }
            };

            let diff_overrides = layer.overrides_by_diff.get(&sibling_key);
            let sibling_rules = layer
                .rules_by_diff
                .get(&sibling_key)
                .or_else(|| layer.rules_by_diff.get(&diff_name))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let effective_sibling_moments = apply_base_rules(&layer.moments, sibling_rules);
            // Deduplicate per MomentId: keep only the closest base moment within leniency.
            let mut best_by_moment: BTreeMap<usize, BestBaseMomentMatch> = BTreeMap::new();
            let mut skipped_moment_ids: BTreeSet<usize> = BTreeSet::new();
            let mut unresolved_moments = 0usize;

            for base_moment in &effective_sibling_moments {
                let Some((moment_id, matched_ms)) =
                    find_matching_object_moment(&hits, base_moment.time_ms, leniency)
                else {
                    continue;
                };
                let is_overridden = diff_overrides.is_some_and(|set| {
                    set.contains(&matched_ms) || set.contains(&base_moment.time_ms)
                });
                if is_overridden && !overwrite_overrides {
                    skipped_moment_ids.insert(moment_id.0);
                    continue;
                }
                let mut resolved_hits = Vec::with_capacity(base_moment.hits.len());
                for hit in &base_moment.hits {
                    if let Some(identity) = resolve_sample(&hit.sample) {
                        resolved_hits.push((identity, i32::from(hit.volume.min(100))));
                    }
                }
                // Never wipe an existing sibling moment if Base had hits that failed to resolve.
                if !base_moment.hits.is_empty() && resolved_hits.is_empty() {
                    unresolved_moments += 1;
                    continue;
                }
                let dist =
                    (hits.moment(moment_id).time().ms() - base_moment.time_ms as f64).abs();
                let replace = best_by_moment
                    .get(&moment_id.0)
                    .is_none_or(|(_, prev_dist, _, _, _)| dist < *prev_dist);
                if replace {
                    best_by_moment.insert(
                        moment_id.0,
                        (moment_id, dist, matched_ms, base_moment.time_ms, resolved_hits),
                    );
                }
            }

            if unresolved_moments > 0 {
                failed_diffs.push(format!(
                    "{diff_name}: skipped {unresolved_moments} moment(s) with unresolved sample names"
                ));
            }
            total_skipped_overrides += skipped_moment_ids.len();
            if best_by_moment.is_empty() {
                skipped_diffs += 1;
                continue;
            }

            let planned: Vec<PlannedBaseMoment> = best_by_moment
                .into_values()
                .map(|(id, _, matched_ms, base_ms, new_hits)| (id, matched_ms, base_ms, new_hits))
                .collect();

            for (moment_id, _, _, new_hits) in &planned {
                let existing: Vec<SampleIdentity> =
                    hits.moment(*moment_id).hits().cloned().collect();
                for lane in existing {
                    hits.remove_hit(*moment_id, &lane);
                }
                for (lane, volume) in new_hits {
                    hits.add_hit_with_volume(*moment_id, lane.clone(), *volume);
                }
            }

            let (out_file, problems) = match dawsu_hitsounding::compile(
                &parsed.file,
                &hits,
                dawsu_hitsounding::Placement::GreenLines,
                &|| false,
                &mut |_, _| {},
            ) {
                dawsu_hitsounding::Compiled::Done { file, problems } => (file, problems),
                dawsu_hitsounding::Compiled::Cancelled => {
                    skipped_diffs += 1;
                    continue;
                }
            };
            if !problems.is_empty() {
                failed_diffs.push(format!(
                    "{diff_name}: {} compile problem(s) (open this diff to review)",
                    problems.len()
                ));
                continue;
            }
            let compiled_bytes = out_file.to_bytes();
            if let Err(mismatch) =
                dawsu_hitsounding::verify_compiled_osu(&compiled_bytes, &hits, &folder_files)
            {
                failed_diffs.push(format!("{diff_name}: verification failed ({mismatch})"));
                continue;
            }
            let effective_env = layer
                .volume_envelopes_by_diff
                .get(&sibling_key)
                .or_else(|| layer.volume_envelopes_by_diff.get(&diff_name))
                .filter(|e| !e.is_empty())
                .unwrap_or(&layer.volume_envelope);
            let (out_file, _) = apply_envelope_to_osu(&out_file, effective_env);
            let out_bytes = out_file.to_bytes();
            if dawsu_osu_format::OsuFile::read(out_bytes.clone())
                .file
                .format_version()
                .is_none()
            {
                failed_diffs.push(format!(
                    "{diff_name}: volume envelope verification failed (missing osu file format header)"
                ));
                continue;
            }
            let clear_sibling_overrides = |layer: &mut SharedBaseLayer| -> bool {
                if !overwrite_overrides {
                    return false;
                }
                if let Some(set) = layer.overrides_by_diff.get_mut(&sibling_key) {
                    let before = set.len();
                    for (_, matched_ms, base_ms, _) in &planned {
                        set.remove(matched_ms);
                        set.remove(base_ms);
                    }
                    let changed = set.len() != before;
                    if set.is_empty() {
                        layer.overrides_by_diff.remove(&sibling_key);
                    }
                    changed
                } else {
                    false
                }
            };
            if out_bytes == original_bytes {
                if clear_sibling_overrides(&mut layer) {
                    layer_dirty = true;
                }
                skipped_diffs += 1;
                continue;
            }
            match out_file
                .save_over_expected(&sibling_path, &original_bytes, &policy)
            {
                Ok(dawsu_osu_format::SaveOutcome::Saved { .. }) => {
                    updated_diffs += 1;
                    total_applied_moments += planned.len();
                    if clear_sibling_overrides(&mut layer) {
                        layer_dirty = true;
                    }
                }
                Ok(dawsu_osu_format::SaveOutcome::NothingToSave) => {
                    if clear_sibling_overrides(&mut layer) {
                        layer_dirty = true;
                    }
                    skipped_diffs += 1;
                }
                Ok(other) => {
                    failed_diffs.push(format!(
                        "{diff_name}: {}",
                        crate::refusals::write(&other)
                    ));
                }
                Err(e) => {
                    failed_diffs.push(format!("{diff_name}: {e}"));
                }
            }
        }

        if layer_dirty
            && let Err(e) = save_base(&path, &layer)
        {
            failed_diffs.push(format!("Shared Base metadata could not be saved: {e}"));
        }

        let status = self
            .document
            .as_ref()
            .map(|doc| build_status(doc, Some(&layer)))
            .unwrap_or_default();
        self.status = if failed_diffs.is_empty() {
            format!(
                "Applied Shared Base across {updated_diffs} diff(s) ({total_applied_moments} moments, {total_skipped_overrides} overrides kept)."
            )
        } else {
            format!(
                "Applied Shared Base across {updated_diffs} diff(s) ({total_applied_moments} moments, {} issue(s)).",
                failed_diffs.len()
            )
        };
        self.bump_view_revision();
        Ok(SharedBaseApplyAllResult {
            updated_diffs,
            skipped_diffs,
            total_applied_moments,
            total_skipped_overrides,
            failed_diffs,
            status,
        })
    }

    pub fn shared_base_clear_current_overrides(&mut self) -> Result<SharedBaseStatus, String> {
        let path = self.shared_base_file_path()?;
        let key = {
            let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
            diff_key(document)
        };
        let Some(mut layer) = load_base(&path)? else {
            return Ok(SharedBaseStatus::default());
        };
        if layer.overrides_by_diff.remove(&key).is_some() {
            save_base(&path, &layer)?;
            self.bump_view_revision();
        }
        self.sync_shared_base_track();
        let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
        Ok(build_status(document, Some(&layer)))
    }

    pub fn shared_base_clear(&mut self) -> Result<SharedBaseStatus, String> {
        let path = self.shared_base_file_path()?;
        delete_base(&path)?;
        self.base_layer_track = None;
        self.status = "Cleared Shared Base for this mapset.".into();
        self.bump_view_revision();
        Ok(SharedBaseStatus::default())
    }

    /// Returns the non-destructive per-difficulty Shared Base rules for the open difficulty (or
    /// `diff_file` if specified).
    pub fn shared_base_rules(&self, diff_file: Option<&str>) -> Vec<BaseRule> {
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        let Ok(path) = storage_path(&self.shared_base_projects_root(), &document.path) else {
            return Vec::new();
        };
        let Ok(Some(layer)) = load_base(&path) else {
            return Vec::new();
        };
        let key = diff_file
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| diff_key(document).to_ascii_lowercase());
        layer
            .rules_by_diff
            .get(&key)
            .or_else(|| diff_file.and_then(|d| layer.rules_by_diff.get(d)))
            .cloned()
            .unwrap_or_default()
    }

    /// Appends a non-destructive per-difficulty rule (`MuteLane`, `ReplaceLane`, `AddHit`,
    /// `RemoveHit`, or `SliderBody`) to the open difficulty (or named `diff_file`).
    pub fn shared_base_add_rule(
        &mut self,
        diff_file: Option<&str>,
        rule: BaseRule,
    ) -> Result<Vec<BaseRule>, String> {
        let path = self.shared_base_file_path()?;
        let key = {
            let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
            diff_file
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| diff_key(document).to_ascii_lowercase())
        };
        let mut layer = load_base(&path)?.unwrap_or_default();
        layer.version = SHARED_BASE_VERSION;
        let rules = layer.rules_by_diff.entry(key.clone()).or_default();
        if !rules.contains(&rule) {
            rules.push(rule);
        }
        let result = rules.clone();
        save_base(&path, &layer)?;
        self.status = format!("Added per-difficulty lane rule ({} total).", result.len());
        self.bump_view_revision();
        Ok(result)
    }

    /// Removes the per-difficulty rule at `index` for the open difficulty (or named `diff_file`).
    pub fn shared_base_remove_rule(
        &mut self,
        diff_file: Option<&str>,
        index: usize,
    ) -> Result<Vec<BaseRule>, String> {
        let path = self.shared_base_file_path()?;
        let key = {
            let document = self.document.as_ref().ok_or("Open a difficulty first.")?;
            diff_file
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| diff_key(document).to_ascii_lowercase())
        };
        let Some(mut layer) = load_base(&path)? else {
            return Ok(Vec::new());
        };
        let remaining = if let Some(rules) = layer.rules_by_diff.get_mut(&key) {
            if index < rules.len() {
                rules.remove(index);
            }
            let out = rules.clone();
            if rules.is_empty() {
                layer.rules_by_diff.remove(&key);
            }
            out
        } else {
            Vec::new()
        };
        save_base(&path, &layer)?;
        self.bump_view_revision();
        Ok(remaining)
    }

    /// Reads the shared or per-difficulty green-line-only master volume envelope.
    pub fn shared_base_volume_envelope(&self, diff_file: Option<&str>) -> VolumeEnvelope {
        let Some(document) = self.document.as_ref() else {
            return VolumeEnvelope::default();
        };
        let Ok(path) = storage_path(&self.shared_base_projects_root(), &document.path) else {
            return VolumeEnvelope::default();
        };
        let Ok(Some(layer)) = load_base(&path) else {
            return VolumeEnvelope::default();
        };
        if let Some(diff) = diff_file {
            let key = diff.to_ascii_lowercase();
            if let Some(env) = layer
                .volume_envelopes_by_diff
                .get(&key)
                .or_else(|| layer.volume_envelopes_by_diff.get(diff))
            {
                return env.clone();
            }
        }
        layer.volume_envelope
    }

    /// Saves a green-line-only master volume envelope (shared when `diff_file` is `None`, or
    /// per-difficulty when `diff_file` is `Some`).
    pub fn shared_base_set_volume_envelope(
        &mut self,
        diff_file: Option<&str>,
        envelope: VolumeEnvelope,
    ) -> Result<VolumeEnvelope, String> {
        let path = self.shared_base_file_path()?;
        let mut layer = load_base(&path)?.unwrap_or_default();
        layer.version = SHARED_BASE_VERSION;
        let normalized = VolumeEnvelope::new(envelope.points);
        if let Some(diff) = diff_file {
            let key = diff.to_ascii_lowercase();
            if normalized.is_empty() {
                layer.volume_envelopes_by_diff.remove(&key);
            } else {
                layer.volume_envelopes_by_diff.insert(key, normalized.clone());
            }
        } else {
            layer.volume_envelope = normalized.clone();
        }
        save_base(&path, &layer)?;
        self.status = if normalized.is_empty() {
            "Cleared master volume curve.".into()
        } else {
            format!("Saved master volume curve ({} point(s)).", normalized.points.len())
        };
        self.bump_view_revision();
        Ok(normalized)
    }

    /// Projects the Shared Base (or open difficulty's hits when no base exists yet) live across
    /// every `.osu` difficulty in the mapset so the editor can show live difficulty tabs with
    /// matched / empty / orphaned counts and rule badges.
    #[allow(clippy::disallowed_methods)]
    pub fn shared_base_diff_projections(&self, leniency_ms: f64) -> Vec<DiffProjectionTab> {
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        let leniency = clamp_leniency(leniency_ms);
        let layer = storage_path(&self.shared_base_projects_root(), &document.path)
            .ok()
            .and_then(|p| load_base(&p).ok().flatten())
            .unwrap_or_default();
        let active_moments: Vec<BaseMoment> = if !layer.moments.is_empty() {
            layer.moments.clone()
        } else {
            capture_moments(document.composer.hitsounds())
        };
        let active_canon = document
            .path
            .canonicalize()
            .unwrap_or_else(|_| document.path.clone());
        let folder_files = document
            .path
            .parent()
            .and_then(|p| dawsu_osu_format::list_folder(p).ok())
            .unwrap_or_default();

        let mut tabs = Vec::with_capacity(document.difficulties.len());
        for diff_path in &document.difficulties {
            let canon = diff_path
                .canonicalize()
                .unwrap_or_else(|_| diff_path.clone());
            let is_active = if cfg!(windows) {
                canon
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&active_canon.to_string_lossy())
            } else {
                canon == active_canon
            };
            let file_name = diff_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| diff_path.display().to_string());
            let lower_key = file_name.to_ascii_lowercase();
            let rules = layer
                .rules_by_diff
                .get(&lower_key)
                .or_else(|| layer.rules_by_diff.get(&file_name))
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let effective = apply_base_rules(&active_moments, rules);
            let has_volume_envelope = layer
                .volume_envelopes_by_diff
                .get(&lower_key)
                .or_else(|| layer.volume_envelopes_by_diff.get(&file_name))
                .is_some_and(|e| !e.is_empty());

            let (version, matched, empty, orphaned) = if is_active {
                let (m, e, o) =
                    project_counts_for_hitsounds(document.composer.hitsounds(), &effective, leniency);
                (diff_label(document), m, e, o)
            } else {
                match fs::read(diff_path) {
                    Ok(bytes) => {
                        let parsed = dawsu_osu_format::OsuFile::read(bytes).file;
                        let ver = parsed
                            .value("Metadata", "Version")
                            .filter(|v| !v.trim().is_empty())
                            .unwrap_or_else(|| file_name.clone());
                        match dawsu_hitsounding::decompile(
                            &parsed,
                            &folder_files,
                            &|| false,
                            &mut |_, _| {},
                        ) {
                            dawsu_hitsounding::Decompiled::Done(h) => {
                                let (m, e, o) =
                                    project_counts_for_hitsounds(&h, &effective, leniency);
                                (ver, m, e, o)
                            }
                            _ => (ver, 0, 0, 0),
                        }
                    }
                    Err(_) => (file_name.clone(), 0, 0, 0),
                }
            };

            tabs.push(DiffProjectionTab {
                file: file_name,
                path: diff_path.to_string_lossy().into_owned(),
                version,
                matched,
                empty,
                orphaned,
                rule_count: rules.len(),
                is_active,
                has_volume_envelope,
            });
        }
        tabs
    }

    /// Locates a moved mapset folder using `songs_folder` (matching exact folder name or
    /// difficulty filename) or relinks directly to `explicit_new_folder`, migrating any stored
    /// Shared Base and recent-file entry cleanly so work is never lost when a folder moves.
    pub fn relink_moved_mapset(
        &mut self,
        missing_osu_path: &Path,
        explicit_new_folder: Option<&Path>,
    ) -> Result<RelinkMapsetResult, String> {
        let missing_file = missing_osu_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Missing difficulty filename.")?;
        let old_folder = missing_osu_path
            .parent()
            .ok_or("Missing old mapset folder.")?;

        let new_osu_path = if let Some(folder) = explicit_new_folder {
            let direct = folder.join(missing_file);
            if direct.is_file() {
                direct
            } else {
                // Fall back to first `.osu` file in the chosen folder if the diff was renamed.
                fs::read_dir(folder)
                    .map_err(|e| format!("Could not read folder: {e}"))?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| {
                        p.is_file()
                            && p.extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("osu"))
                    })
                    .ok_or("No .osu difficulty found in the chosen folder.")?
            }
        } else {
            let songs_dir = &self.preferences.songs_folder;
            if !songs_dir.is_dir() {
                return Err("Set your osu! Songs folder in Preferences or pick the moved folder directly.".into());
            }
            let index = songs_index::load_or_build_songs_index(songs_dir, &self.data_root, false);
            index
                .find_relink_candidate(missing_osu_path)
                .ok_or_else(|| format!("Could not locate {missing_file} in the Songs folder."))?
        };

        Ok(self.finish_relink_moved_mapset(missing_osu_path, old_folder, new_osu_path))
    }

    /// Relinks a moved mapset directly to an already-resolved `.osu` difficulty path
    /// (used by the Tauri command after searching `SongsIndexCache` off the editor thread).
    pub fn relink_moved_mapset_to_osu_path(
        &mut self,
        missing_osu_path: &Path,
        new_osu_path: &Path,
    ) -> Result<RelinkMapsetResult, String> {
        let missing_file = missing_osu_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Missing difficulty filename.")?;
        let old_folder = missing_osu_path
            .parent()
            .ok_or("Missing old mapset folder.")?;
        if !new_osu_path.is_file() {
            return Err(format!(
                "Could not locate {missing_file} in the Songs folder."
            ));
        }
        Ok(self.finish_relink_moved_mapset(
            missing_osu_path,
            old_folder,
            new_osu_path.to_path_buf(),
        ))
    }

    fn finish_relink_moved_mapset(
        &mut self,
        missing_osu_path: &Path,
        old_folder: &Path,
        new_osu_path: PathBuf,
    ) -> RelinkMapsetResult {
        let projects_root = self.shared_base_projects_root();
        let mut migrated_base = false;
        let old_base_path = projects_root.join(format!("base-{}.json", path_digest(old_folder)));
        if let Ok(new_base_path) = storage_path(&projects_root, &new_osu_path)
            && old_base_path.is_file()
            && old_base_path != new_base_path
            && let Ok(Some(layer)) = load_base(&old_base_path)
            && save_base(&new_base_path, &layer).is_ok()
        {
            migrated_base = true;
        }

        let mut migrated_project = false;
        let old_proj_path = projects_root.join(format!("{}.json", path_digest(missing_osu_path)));
        let new_proj_path = projects_root.join(format!("{}.json", path_digest(&new_osu_path)));
        if old_proj_path.is_file()
            && old_proj_path != new_proj_path
            && !new_proj_path.exists()
            && fs::copy(&old_proj_path, &new_proj_path).is_ok()
        {
            migrated_project = true;
        }

        // Replace missing path in recent maps list if present.
        for recent in &mut self.preferences.recent {
            if *recent == missing_osu_path {
                *recent = new_osu_path.clone();
            }
        }
        self.save_preferences();

        RelinkMapsetResult {
            old_path: missing_osu_path.to_string_lossy().into_owned(),
            new_path: new_osu_path.to_string_lossy().into_owned(),
            migrated_base,
            migrated_project,
        }
    }

    pub(crate) fn sync_shared_base_track(&mut self) {
        let Some(document) = self.document.as_ref() else {
            self.base_layer_track = None;
            return;
        };
        let Ok(path) = storage_path(&self.shared_base_projects_root(), &document.path) else {
            self.base_layer_track = None;
            return;
        };
        let enabled = matches!(load_base(&path), Ok(Some(layer)) if layer.enabled);
        if !enabled {
            self.base_layer_track = None;
            return;
        }
        self.base_layer_track = Some(BaseLayerTrack {
            path: document.path.clone(),
            version: document.composer.history.version(),
            snapshot: snapshot_moments(document.composer.hitsounds()),
        });
    }

    pub(crate) fn record_shared_base_overrides_if_needed(&mut self) {
        let Some(document) = self.document.as_ref() else {
            self.base_layer_track = None;
            return;
        };
        let version = document.composer.history.version();
        if let Some(track) = self.base_layer_track.as_ref()
            && track.path == document.path
            && track.version == version
        {
            return;
        }
        let Ok(path) = storage_path(&self.shared_base_projects_root(), &document.path) else {
            self.base_layer_track = None;
            return;
        };
        let Ok(Some(mut layer)) = load_base(&path) else {
            self.base_layer_track = None;
            return;
        };
        if !layer.enabled {
            self.base_layer_track = None;
            return;
        }
        let current_snapshot = snapshot_moments(document.composer.hitsounds());
        if let Some(track) = self.base_layer_track.as_ref()
            && track.path == document.path
        {
            let changed = diff_changed_times(&track.snapshot, &current_snapshot);
            if !changed.is_empty() {
                let key = diff_key(document);
                let overrides = layer.overrides_by_diff.entry(key).or_default();
                let before_len = overrides.len();
                for time_ms in changed {
                    overrides.insert(time_ms);
                }
                if overrides.len() != before_len {
                    let _ = save_base(&path, &layer);
                }
            }
        }
        self.base_layer_track = Some(BaseLayerTrack {
            path: document.path.clone(),
            version,
            snapshot: current_snapshot,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;
    use std::time::{Duration, Instant};

    fn sample_osu(version: &str, objects: &[&str]) -> String {
        let mut out = format!(
            "osu file format v14\n\n[General]\nAudioFilename: audio.mp3\nMode: 0\n\n[Metadata]\nTitle: Test\nArtist: Artist\nCreator: Mapper\nVersion: {version}\n\n[Difficulty]\nHPDrainRate:5\nCircleSize:4\nOverallDifficulty:5\nApproachRate:5\nSliderMultiplier:1.4\nSliderTickRate:1\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n"
        );
        for obj in objects {
            out.push_str(obj);
            out.push('\n');
        }
        out
    }

    fn folder_entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    fn app_with_temp_data(scratch: &Scratch) -> Application {
        let mut app = Application::headless();
        let data_root = scratch.path("appdata");
        app.project_root = Some(data_root.join("projects"));
        app.data_root = data_root;
        app
    }

    fn open_map(app: &mut Application, path: &Path) {
        app.request_open(path.to_path_buf());
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    // The Shared Base UI (ui/lib/sharedBase.js, BaseDiffStrip, SharedBaseModal) reads these names.
    #[test]
    fn shared_base_json_field_names_stay_camel_case_for_the_ui() {
        let status = SharedBaseStatus {
            base_moment_count: 7,
            ..SharedBaseStatus::default()
        };
        let status_json = serde_json::to_value(&status).unwrap();
        assert_eq!(status_json["baseMomentCount"], 7);
        for key in [
            "enabled",
            "hasBase",
            "sourceDifficulty",
            "currentDiffOverrides",
            "currentDiffInherited",
        ] {
            assert!(
                status_json.get(key).is_some(),
                "status lacks {key}: {status_json}"
            );
        }

        let apply = SharedBaseApplyResult {
            applied_moments: 3,
            skipped_overrides: 1,
            unmatched_base_moments: 2,
            status: status.clone(),
        };
        let apply_json = serde_json::to_value(&apply).unwrap();
        assert_eq!(apply_json["appliedMoments"], 3);
        assert_eq!(apply_json["skippedOverrides"], 1);
        assert_eq!(apply_json["unmatchedBaseMoments"], 2);
        assert_eq!(apply_json["status"]["baseMomentCount"], 7);

        let apply_all = SharedBaseApplyAllResult {
            updated_diffs: 2,
            skipped_diffs: 1,
            total_applied_moments: 9,
            total_skipped_overrides: 4,
            failed_diffs: vec!["Hard: could not read".into()],
            status,
        };
        let all_json = serde_json::to_value(&apply_all).unwrap();
        assert_eq!(all_json["updatedDiffs"], 2);
        assert_eq!(all_json["skippedDiffs"], 1);
        assert_eq!(all_json["totalAppliedMoments"], 9);
        assert_eq!(all_json["totalSkippedOverrides"], 4);
        assert_eq!(all_json["failedDiffs"][0], "Hard: could not read");
        assert_eq!(all_json["status"]["baseMomentCount"], 7);
    }

    #[test]
    fn shared_base_persists_in_appdata_and_never_touches_mapset_folder() {
        let scratch = Scratch::new("shared-base-persist");
        let mapset = scratch.path("mapset");
        fs::create_dir_all(&mapset).unwrap();
        let diff_a = mapset.join("Test [Insane].osu");
        // Circle at 1000ms with whistle (2) and clap (8), circle at 2000ms with finish (4)
        fs::write(
            &diff_a,
            sample_osu(
                "Insane",
                &[
                    "256,192,1000,1,10,0:0:0:80:",
                    "256,192,2000,1,4,0:0:0:65:",
                ],
            ),
        )
        .unwrap();
        let before_files = folder_entries(&mapset);

        let mut app = app_with_temp_data(&scratch);
        open_map(&mut app, &diff_a);
        assert!(!app.shared_base_status().enabled);
        assert!(!app.shared_base_status().has_base);

        let status = app.shared_base_capture_current().unwrap();
        assert!(status.enabled);
        assert!(status.has_base);
        assert_eq!(status.source_difficulty.as_deref(), Some("Insane"));
        assert_eq!(status.base_moment_count, 2);

        // Mapset folder must be completely untouched.
        assert_eq!(folder_entries(&mapset), before_files);

        // App-data projects directory must hold the base-<hash>.json file.
        let projects_dir = scratch.path("appdata").join("projects");
        let base_path = storage_path(&projects_dir, &diff_a).unwrap();
        assert!(base_path.exists());
        assert!(
            base_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("base-")
        );

        // Reload in a fresh Application instance pointing at the same appdata.
        let mut reloaded = app_with_temp_data(&scratch);
        open_map(&mut reloaded, &diff_a);
        let reloaded_status = reloaded.shared_base_status();
        assert!(reloaded_status.enabled);
        assert!(reloaded_status.has_base);
        assert_eq!(reloaded_status.base_moment_count, 2);
    }

    #[test]
    fn capture_apply_with_leniency_and_single_undo_step() {
        let scratch = Scratch::new("shared-base-apply-undo");
        let mapset = scratch.path("mapset");
        fs::create_dir_all(&mapset).unwrap();
        let insane = mapset.join("Song [Insane].osu");
        let hard = mapset.join("Song [Hard].osu");

        // Insane has hits at 1000ms (whistle+clap @ 80%) and 2000ms (finish @ 60%) and 4000ms (clap @ 70%)
        fs::write(
            &insane,
            sample_osu(
                "Insane",
                &[
                    "256,192,1000,1,10,0:0:0:80:",
                    "256,192,2000,1,4,0:0:0:60:",
                    "256,192,4000,1,8,0:0:0:70:",
                ],
            ),
        )
        .unwrap();
        // Hard has an object at 1003ms (within 5ms leniency), 2000ms (exact), and no object near 4000ms.
        fs::write(
            &hard,
            sample_osu(
                "Hard",
                &["256,192,1003,1,0,0:0:0:100:", "256,192,2000,1,0,0:0:0:100:"],
            ),
        )
        .unwrap();

        let mut app = app_with_temp_data(&scratch);
        open_map(&mut app, &insane);
        app.shared_base_capture_current().unwrap();

        open_map(&mut app, &hard);
        let before_snapshot = snapshot_moments(app.document().unwrap().composer.hitsounds());

        let result = app.shared_base_apply_to_current(5.0, false).unwrap();
        assert_eq!(result.applied_moments, 2);
        assert_eq!(result.skipped_overrides, 0);
        assert_eq!(result.unmatched_base_moments, 1);

        let after_snapshot = snapshot_moments(app.document().unwrap().composer.hitsounds());
        assert_ne!(after_snapshot, before_snapshot);
        let clap = resolve_sample("Skin|normal-hitclap").unwrap();
        let m0 = app
            .document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .find(|m| m.time().ms().round() as i64 == 1003)
            .unwrap();
        assert!(m0.has_hit(&clap));
        assert_eq!(m0.hit_volume(&clap), 80);

        // One single Undo step reverts the entire apply cleanly.
        app.command("undo");
        let undone_snapshot = snapshot_moments(app.document().unwrap().composer.hitsounds());
        assert_eq!(undone_snapshot, before_snapshot);
    }

    #[test]
    fn overrides_are_preserved_unless_overwrite_is_true_and_clear_removes_file() {
        let scratch = Scratch::new("shared-base-overrides");
        let mapset = scratch.path("mapset");
        fs::create_dir_all(&mapset).unwrap();
        let insane = mapset.join("Song [Insane].osu");
        let hard = mapset.join("Song [Hard].osu");

        fs::write(
            &insane,
            sample_osu(
                "Insane",
                &[
                    "256,192,1000,1,8,0:0:0:85:",
                    "256,192,2000,1,4,0:0:0:75:",
                ],
            ),
        )
        .unwrap();
        fs::write(
            &hard,
            sample_osu(
                "Hard",
                &["256,192,1000,1,0,0:0:0:100:", "256,192,2000,1,0,0:0:0:100:"],
            ),
        )
        .unwrap();

        let mut app = app_with_temp_data(&scratch);
        open_map(&mut app, &insane);
        app.shared_base_capture_current().unwrap();

        open_map(&mut app, &hard);
        // Simulate a mapper editing the 1000ms moment on Hard while Shared Base is enabled.
        let whistle = resolve_sample("Skin|normal-hitwhistle").unwrap();
        app.document
            .as_mut()
            .unwrap()
            .composer
            .add(1000.0, whistle.clone(), 90, 5.0);
        app.refresh_recovery_edit();

        let status = app.shared_base_status();
        assert_eq!(status.current_diff_overrides, 1);
        assert_eq!(status.current_diff_inherited, 1);

        // Apply with overwrite_overrides = false skips 1000ms and applies 2000ms.
        let res_keep = app.shared_base_apply_to_current(5.0, false).unwrap();
        assert_eq!(res_keep.applied_moments, 1);
        assert_eq!(res_keep.skipped_overrides, 1);
        let m1000 = app
            .document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .find(|m| m.time().ms().round() as i64 == 1000)
            .unwrap();
        assert!(m1000.has_hit(&whistle));

        // Apply with overwrite_overrides = true overwrites 1000ms and clears the override.
        let res_overwrite = app.shared_base_apply_to_current(5.0, true).unwrap();
        assert_eq!(res_overwrite.applied_moments, 2);
        assert_eq!(res_overwrite.skipped_overrides, 0);
        assert_eq!(res_overwrite.status.current_diff_overrides, 0);
        let m1000_after = app
            .document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .find(|m| m.time().ms().round() as i64 == 1000)
            .unwrap();
        assert!(!m1000_after.has_hit(&whistle));

        // Clear removes the base file and disables Shared Base.
        let projects_dir = scratch.path("appdata").join("projects");
        let base_path = storage_path(&projects_dir, &hard).unwrap();
        assert!(base_path.exists());
        let cleared = app.shared_base_clear().unwrap();
        assert!(!cleared.enabled);
        assert!(!cleared.has_base);
        assert!(!base_path.exists());
    }

    #[test]
    fn apply_base_to_all_diffs_updates_siblings_safely_and_keeps_overrides() {
        let scratch = dawsu_test_support::Scratch::new("base-layer-apply-all");
        let mapset = scratch.path("mapset");
        fs::create_dir_all(&mapset).unwrap();

        let insane = mapset.join("Map - Test (Author) [Insane].osu");
        let hard = mapset.join("Map - Test (Author) [Hard].osu");
        let normal = mapset.join("Map - Test (Author) [Normal].osu");

        fs::write(
            &insane,
            sample_osu(
                "Insane",
                &[
                    "256,192,1000,1,8,0:0:0:85:",
                    "256,192,2000,1,4,0:0:0:75:",
                ],
            ),
        )
        .unwrap();
        fs::write(
            &hard,
            sample_osu(
                "Hard",
                &["256,192,1000,1,0,0:0:0:100:", "256,192,2000,1,0,0:0:0:100:"],
            ),
        )
        .unwrap();
        fs::write(
            &normal,
            sample_osu("Normal", &["256,192,1000,1,0,0:0:0:100:"]),
        )
        .unwrap();

        let mut app = app_with_temp_data(&scratch);
        open_map(&mut app, &insane);
        app.shared_base_capture_current().unwrap();

        // Mark 1000ms on Hard as a per-diff override so Apply All keeps it untouched.
        let projects_dir = scratch.path("appdata").join("projects");
        let base_path = storage_path(&projects_dir, &insane).unwrap();
        let mut layer = load_base(&base_path).unwrap().unwrap();
        layer
            .overrides_by_diff
            .entry("map - test (author) [hard].osu".into())
            .or_default()
            .insert(1000);
        save_base(&base_path, &layer).unwrap();

        let res = app.shared_base_apply_to_all_diffs(5.0, false).unwrap();
        assert!(res.failed_diffs.is_empty(), "unexpected failures: {:?}", res.failed_diffs);
        assert_eq!(res.updated_diffs, 3);
        assert_eq!(res.total_skipped_overrides, 1);

        let hard_text = fs::read_to_string(&hard).unwrap();
        assert!(
            hard_text.contains("256,192,1000,1,0,"),
            "overridden 1000ms on Hard should stay untouched: {hard_text}"
        );
        assert!(
            hard_text.contains("256,192,2000,1,4,"),
            "2000ms on Hard should receive finish from Base: {hard_text}"
        );

        let normal_text = fs::read_to_string(&normal).unwrap();
        assert!(
            normal_text.contains("256,192,1000,1,8,"),
            "1000ms on Normal should receive clap from Base: {normal_text}"
        );
    }

    #[test]
    fn coverage_summary_respects_fallback_sound() {
        let scratch = dawsu_test_support::Scratch::new("coverage-fallback-sound");
        let mapset = scratch.path("mapset");
        fs::create_dir_all(&mapset).unwrap();
        let diff = mapset.join("Map - Test (Author) [Normal].osu");
        fs::write(
            &diff,
            sample_osu("Normal", &["256,192,1000,1,0,0:0:0:100:"]),
        )
        .unwrap();

        let mut app = app_with_temp_data(&scratch);
        open_map(&mut app, &diff);
        // Remove the default hitnormal so the 1000ms object has 0 explicit hits.
        let doc = app.document.as_mut().unwrap();
        let moment_id = MomentId(0);
        let existing: Vec<_> = doc
            .composer
            .hitsounds()
            .moment(moment_id)
            .hits()
            .cloned()
            .collect();
        doc.composer.apply("Clear hits", |h| {
            for lane in existing {
                h.remove_hit(moment_id, &lane);
            }
        });

        let without_fallback = doc.coverage_summary();
        assert!(!without_fallback.fallback_active);
        assert_eq!(without_fallback.empty, 1);
        assert_eq!(without_fallback.matched, 0);

        doc.export_choices = Some(crate::ExportChoices {
            fallback_sound: Some(dawsu_hitsounding::copy::FallbackSound {
                set: dawsu_foundation::SampleSet::Soft,
                sound: dawsu_foundation::Hitsound::Normal,
                volume: 60,
            }),
            ..crate::ExportChoices::default()
        });

        let with_fallback = doc.coverage_summary();
        assert!(with_fallback.fallback_active);
        assert_eq!(with_fallback.empty, 0);
        assert_eq!(with_fallback.fallback_covered, 1);
        assert_eq!(with_fallback.matched, 1);
    }

    #[test]
    fn per_diff_rules_mute_and_replace_lanes_within_time_ranges() {
        let base = vec![
            BaseMoment {
                time_ms: 1000,
                hits: vec![
                    BaseHit { sample: "drum-hitnormal".into(), volume: 90 },
                    BaseHit { sample: "soft-hitclap".into(), volume: 80 },
                ],
            },
            BaseMoment {
                time_ms: 3000,
                hits: vec![
                    BaseHit { sample: "drum-hitnormal".into(), volume: 90 },
                    BaseHit { sample: "soft-hitclap".into(), volume: 80 },
                ],
            },
        ];
        let rules = vec![
            BaseRule::MuteLane {
                lane: "soft-hitclap".into(),
                range: Some(TimeRangeMs { start_ms: 0, end_ms: 2000 }),
            },
            BaseRule::ReplaceLane {
                from: "drum-hitnormal".into(),
                to: "soft-hitwhistle".into(),
                range: Some(TimeRangeMs { start_ms: 2500, end_ms: 4000 }),
            },
            BaseRule::AddHit {
                lane: "normal-hitfinish".into(),
                time_ms: 1500,
                volume: 75,
            },
        ];
        let out = apply_base_rules(&base, &rules);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].time_ms, 1000);
        assert_eq!(out[0].hits.len(), 1);
        assert_eq!(out[0].hits[0].sample, "drum-hitnormal");
        assert_eq!(out[1].time_ms, 1500);
        assert_eq!(out[1].hits[0].sample, "normal-hitfinish");
        assert_eq!(out[2].time_ms, 3000);
        assert!(out[2].hits.iter().any(|h| h.sample == "soft-hitwhistle"));
        assert!(out[2].hits.iter().any(|h| h.sample == "soft-hitclap"));
    }

    #[test]
    fn live_diff_projections_and_volume_envelope_and_relink_moved_mapset() {
        let scratch = dawsu_test_support::Scratch::new("shared-base-v2-relink");
        let songs = scratch.path("Songs");
        let old_dir = songs.join("12345 Artist - Title");
        let moved_dir = songs.join("12345 Artist - Title (Moved)");
        fs::create_dir_all(&old_dir).unwrap();
        let hard = old_dir.join("Map [Hard].osu");
        let normal = old_dir.join("Map [Normal].osu");
        fs::write(&hard, sample_osu("Hard", &[
            "256,192,1000,1,8,0:0:0:100:",
            "256,192,2000,1,2,0:0:0:100:",
        ])).unwrap();
        fs::write(&normal, sample_osu("Normal", &[
            "256,192,1000,1,0,0:0:0:100:",
            "256,192,3000,1,0,0:0:0:100:",
        ])).unwrap();

        let mut app = app_with_temp_data(&scratch);
        app.preferences.songs_folder = songs.clone();
        open_map(&mut app, &hard);
        app.shared_base_capture_current().unwrap();
        let tabs = app.shared_base_diff_projections(5.0);
        assert_eq!(tabs.len(), 2);
        let normal_tab = tabs.iter().find(|t| t.version == "Normal").unwrap();
        assert_eq!(normal_tab.matched, 1);
        assert_eq!(normal_tab.empty, 1);
        assert_eq!(normal_tab.orphaned, 1);

        let saved_env = app
            .shared_base_set_volume_envelope(
                None,
                VolumeEnvelope::new(vec![
                    dawsu_hitsounding::volume_envelope::VolumePoint {
                        time_ms: 1000,
                        volume: 100,
                        curve: dawsu_hitsounding::volume_envelope::CurveKind::Linear,
                    },
                    dawsu_hitsounding::volume_envelope::VolumePoint {
                        time_ms: 2000,
                        volume: 60,
                        curve: dawsu_hitsounding::volume_envelope::CurveKind::Step,
                    },
                ]),
            )
            .unwrap();
        assert_eq!(saved_env.points.len(), 2);
        assert_eq!(app.shared_base_volume_envelope(None).points.len(), 2);

        // Simulate moving the mapset folder on disk and relinking automatically via Songs folder.
        fs::rename(&old_dir, &moved_dir).unwrap();
        let relinked = app.relink_moved_mapset(&hard, None).unwrap();
        assert!(relinked.new_path.contains("Moved"));
        assert!(relinked.migrated_base);

        // Verify candidate scoring (folder+file = 5, file = 3, folder = 2, none = None)
        // and `relink_moved_mapset_to_osu_path` choosing exact diff on folder-only match.
        let renamed_dir = songs.join("99999 Renamed - Set");
        fs::create_dir_all(&renamed_dir).unwrap();
        let renamed_diff_a = renamed_dir.join("Alpha [Easy].osu");
        let renamed_diff_b = renamed_dir.join("Map [Hard].osu");
        fs::write(&renamed_diff_a, sample_osu("Easy", &["256,192,1000,1,0,0:0:0:100:"])).unwrap();
        fs::write(&renamed_diff_b, sample_osu("Hard", &["256,192,1000,1,0,0:0:0:100:"])).unwrap();

        let entries = vec![
            songs_index::SongMapsetEntry {
                set_id: 1,
                artist: "A".into(),
                title: "T".into(),
                creator: "C".into(),
                tags: String::new(),
                folder: "12345 Artist - Title (Moved)".into(),
                diffs: vec![songs_index::SongDiffEntry {
                    version: "Normal".into(),
                    path: moved_dir.join("Map [Normal].osu").to_string_lossy().into_owned(),
                    mode: 0,
                }],
            },
            songs_index::SongMapsetEntry {
                set_id: 2,
                artist: "A".into(),
                title: "T".into(),
                creator: "C".into(),
                tags: String::new(),
                folder: "99999 Renamed - Set".into(),
                diffs: vec![songs_index::SongDiffEntry {
                    version: "Hard".into(),
                    path: renamed_diff_b.to_string_lossy().into_owned(),
                    mode: 0,
                }],
            },
        ];
        // `file_matches` (score 3) beats `folder_matches` (score 2) when exact folder moved.
        assert_eq!(
            songs_index::find_relink_candidate(&entries, &hard),
            Some(renamed_diff_b.clone())
        );
        // Folder-only match (score 2) when folder name matches but diff filename differs.
        let missing_same_folder_other_diff = moved_dir.join("DifferentName [Insane].osu");
        let folder_only =
            songs_index::find_relink_candidate(&entries[..1], &missing_same_folder_other_diff);
        assert_eq!(folder_only, Some(moved_dir.join("Map [Normal].osu")));
        // Direct relink to exact `.osu` path succeeds cleanly.
        let exact = app
            .relink_moved_mapset_to_osu_path(&missing_same_folder_other_diff, &renamed_diff_b)
            .unwrap();
        assert_eq!(exact.new_path, renamed_diff_b.to_string_lossy());
        assert!(songs_index::find_relink_candidate(&[], &hard).is_none());
    }

    #[test]
    fn apply_base_to_all_diffs_succeeds_with_volume_envelope() {
        let scratch = dawsu_test_support::Scratch::new("shared-base-all-diffs-envelope");
        let mapset = scratch.path("mapset");
        fs::create_dir_all(&mapset).unwrap();

        let insane = mapset.join("Map - Test (Author) [Insane].osu");
        let hard = mapset.join("Map - Test (Author) [Hard].osu");
        fs::write(&insane, sample_osu("Insane", &["256,192,1000,1,8,0:0:0:85:"])).unwrap();
        fs::write(&hard, sample_osu("Hard", &["256,192,1000,1,0,0:0:0:100:"])).unwrap();

        let mut app = app_with_temp_data(&scratch);
        open_map(&mut app, &insane);
        app.shared_base_capture_current().unwrap();
        app.shared_base_set_volume_envelope(
            None,
            VolumeEnvelope::new(vec![dawsu_hitsounding::volume_envelope::VolumePoint {
                time_ms: 0,
                volume: 60,
                curve: dawsu_hitsounding::volume_envelope::CurveKind::Step,
            }]),
        )
        .unwrap();

        let res = app.shared_base_apply_to_all_diffs(5.0, false).unwrap();
        assert!(
            res.failed_diffs.is_empty(),
            "volume envelope must verify pre-envelope hits and succeed: {:?}",
            res.failed_diffs
        );
        let parsed_hard = dawsu_osu_format::OsuFile::read(fs::read(&hard).unwrap()).file;
        assert!(
            parsed_hard
                .timing_points()
                .iter()
                .any(|tp| tp.volume() == Some(60)),
            "expected Hard green/timing point volume scaled to 60%"
        );
    }

    fn save_map(app: &mut Application) {
        app.save();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.busy {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !app.save_review_pending(),
            "unexpected save review during test"
        );
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn apply_base_to_all_keeps_overrides_recorded_by_editing_mixed_case_diff() {
        let scratch = dawsu_test_support::Scratch::new("base-layer-case-overrides");
        let mapset = scratch.path("mapset");
        fs::create_dir_all(&mapset).unwrap();

        let insane = mapset.join("Map - Test (Author) [Insane].osu");
        let hard = mapset.join("Map - Test (Author) [Hard].osu");
        let normal = mapset.join("Map - Test (Author) [Normal].osu");

        fs::write(
            &insane,
            sample_osu(
                "Insane",
                &[
                    "256,192,1000,1,8,0:0:0:85:",
                    "256,192,2000,1,4,0:0:0:75:",
                ],
            ),
        )
        .unwrap();
        fs::write(
            &hard,
            sample_osu(
                "Hard",
                &[
                    "256,192,1000,1,0,0:0:0:100:",
                    "256,192,2000,1,0,0:0:0:100:",
                ],
            ),
        )
        .unwrap();
        fs::write(
            &normal,
            sample_osu(
                "Normal",
                &[
                    "256,192,1000,1,2,0:0:0:90:",
                    "256,192,2000,1,0,0:0:0:100:",
                ],
            ),
        )
        .unwrap();

        let mut app = app_with_temp_data(&scratch);
        open_map(&mut app, &insane);
        app.shared_base_capture_current().unwrap();

        // Edit Hard via the real composer and save so Hard has its overridden 1000ms whistle.
        open_map(&mut app, &hard);
        let whistle = resolve_sample("Skin|normal-hitwhistle").unwrap();
        app.document
            .as_mut()
            .unwrap()
            .composer
            .add(1000.0, whistle, 90, 5.0);
        app.refresh_recovery_edit();
        save_map(&mut app);

        let projects_dir = scratch.path("appdata").join("projects");
        let base_path = storage_path(&projects_dir, &insane).unwrap();
        let raw_layer: SharedBaseLayer =
            serde_json::from_slice(&fs::read(&base_path).unwrap()).unwrap();
        assert!(
            raw_layer
                .overrides_by_diff
                .contains_key("map - test (author) [hard].osu"),
            "overrides must be recorded under lowercase key: {:?}",
            raw_layer.overrides_by_diff
        );

        // Also seed a legacy mixed-case override key on Normal to prove load_base migrates it.
        let mut seeded = raw_layer;
        seeded
            .overrides_by_diff
            .entry("Map - Test (Author) [Normal].osu".into())
            .or_default()
            .insert(1000);
        save_base(&base_path, &seeded).unwrap();

        // Switch back to Insane and apply to all difficulties with overwrite disabled.
        open_map(&mut app, &insane);
        let res = app.shared_base_apply_to_all_diffs(5.0, false).unwrap();
        assert!(res.failed_diffs.is_empty(), "failures: {:?}", res.failed_diffs);
        assert_eq!(res.total_skipped_overrides, 2);

        let hard_text = fs::read_to_string(&hard).unwrap();
        assert!(
            hard_text.contains("256,192,1000,1,2,"),
            "Hard's overridden 1000ms whistle should stay untouched: {hard_text}"
        );
        assert!(
            hard_text.contains("256,192,2000,1,4,"),
            "Hard's 2000ms moment should receive finish from Base: {hard_text}"
        );

        let normal_text = fs::read_to_string(&normal).unwrap();
        assert!(
            normal_text.contains("256,192,1000,1,2,"),
            "Normal's legacy mixed-case 1000ms override should stay untouched: {normal_text}"
        );
        assert!(
            normal_text.contains("256,192,2000,1,4,"),
            "Normal's 2000ms moment should receive finish from Base: {normal_text}"
        );
    }

    #[test]
    fn apply_base_to_current_keeps_closest_match_when_multiple_base_moments_hit_same_object() {
        let scratch = dawsu_test_support::Scratch::new("base-layer-closest-current");
        let mapset = scratch.path("mapset");
        fs::create_dir_all(&mapset).unwrap();

        let insane = mapset.join("Map - Test (Author) [Insane].osu");
        let hard = mapset.join("Map - Test (Author) [Hard].osu");
        fs::write(
            &insane,
            sample_osu(
                "Insane",
                &[
                    "256,192,1000,1,8,0:0:0:85:",
                    "256,192,1004,1,2,0:0:0:70:",
                ],
            ),
        )
        .unwrap();
        fs::write(
            &hard,
            sample_osu("Hard", &["256,192,1000,1,0,0:0:0:100:"]),
        )
        .unwrap();

        let mut app = app_with_temp_data(&scratch);
        open_map(&mut app, &insane);
        app.shared_base_capture_current().unwrap();

        open_map(&mut app, &hard);
        let result = app.shared_base_apply_to_current(5.0, false).unwrap();
        assert_eq!(result.applied_moments, 1);

        let clap = resolve_sample("Skin|normal-hitclap").unwrap();
        let whistle = resolve_sample("Skin|normal-hitwhistle").unwrap();
        let m1000 = app
            .document()
            .unwrap()
            .composer
            .hitsounds()
            .moments()
            .iter()
            .find(|m| m.time().ms().round() as i64 == 1000)
            .unwrap();
        assert!(m1000.has_hit(&clap), "exact 1000ms clap should win");
        assert!(
            !m1000.has_hit(&whistle),
            "farther 1004ms whistle must not overwrite exact 1000ms match"
        );
    }
}
