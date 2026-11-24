use crate::project::{lane_identity, lane_key, path_digest};
use crate::{Application, Document};
use dawsu_foundation::SampleIdentity;
use dawsu_hitsounding::rules::read_sample_file_name;
use dawsu_hitsounding::{Hitsounds, MomentId, Place};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const SHARED_BASE_VERSION: u32 = 1;
pub const DEFAULT_LENIENCY_MS: f64 = 5.0;
pub const MAX_LENIENCY_MS: f64 = 50.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedBaseLayer {
    pub version: u32,
    pub enabled: bool,
    pub source_difficulty: Option<String>,
    pub moments: Vec<BaseMoment>,
    #[serde(default)]
    pub overrides_by_diff: BTreeMap<String, BTreeSet<i64>>,
}

impl Default for SharedBaseLayer {
    fn default() -> Self {
        Self {
            version: SHARED_BASE_VERSION,
            enabled: false,
            source_difficulty: None,
            moments: Vec::new(),
            overrides_by_diff: BTreeMap::new(),
        }
    }
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
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| document.version())
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

fn load_base(path: &Path) -> Result<Option<SharedBaseLayer>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("Could not read Shared Base file: {err}")),
    };
    let layer: SharedBaseLayer = serde_json::from_slice(&bytes)
        .map_err(|err| format!("Could not parse Shared Base file: {err}"))?;
    if layer.version != SHARED_BASE_VERSION {
        return Err(format!(
            "Unsupported Shared Base file version {}.",
            layer.version
        ));
    }
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

        let mut planned: Vec<(MomentId, i64, i64, Vec<(SampleIdentity, i32)>)> = Vec::new();
        let mut skipped_overrides = 0usize;
        let mut unmatched_base_moments = 0usize;
        let mut touched_lanes: BTreeSet<SampleIdentity> = BTreeSet::new();

        {
            let hitsounds = document.composer.hitsounds();
            for base_moment in &layer.moments {
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
                    skipped_overrides += 1;
                    continue;
                }

                let mut resolved_hits = Vec::with_capacity(base_moment.hits.len());
                for hit in &base_moment.hits {
                    if let Some(identity) = resolve_sample(&hit.sample) {
                        touched_lanes.insert(identity.clone());
                        resolved_hits.push((identity, i32::from(hit.volume.min(100))));
                    }
                }
                planned.push((moment_id, matched_ms, base_moment.time_ms, resolved_hits));
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
        Ok(SharedBaseApplyResult {
            applied_moments,
            skipped_overrides,
            unmatched_base_moments,
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
        Ok(SharedBaseStatus::default())
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
}
