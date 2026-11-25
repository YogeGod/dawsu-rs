//! Export ownership manifest, automatic stale-file clean-up, opt-in `take_over_samples`,
//! `SingleSampleFormat` pass-through, and isolated `HitsoundOnly` folder export.
//!
//! All manifest state is stored under `<data_root>/export-manifests/` (never inside the osu!
//! mapset folder, keeping the mapset 100% clean of non-gameplay files per Ranking Criteria).

use crate::project::path_digest;
use dawsu_foundation::digest::sha256_hex;
use dawsu_hitsounding::index_packing::CustomIndexPool;
use dawsu_hitsounding::tick_render::SliderEditHistory;
use dawsu_osu_format::{OsuFile, check_map_path};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const MANIFEST_VERSION: u32 = 1;

/// Where Export writes output files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportWriteMode {
    /// Write straight into the mapset's `.osu` difficulties with automatic backups (default).
    #[default]
    Direct,
    /// Write an isolated folder containing the song, generated samples, and hitsound-only `.osu`
    /// difficulties without touching the real mapset folder at all.
    HitsoundOnly,
}

/// How unchanged single custom samples are written compared to newly mixed samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SingleSampleFormat {
    /// Keep unchanged single `.wav`/`.ogg` source files byte-identical (pass-through).
    #[default]
    Source,
    /// Re-encode single samples using the chosen `MixFormat` (16-bit WAV, 32-bit float, or Ogg).
    MatchMixFormat,
}

/// Persistent ownership record for one mapset's direct exports.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOwnershipManifest {
    pub version: u32,
    /// Map of lowercase sample filename -> SHA-256 hex digest of the file dawsu! wrote.
    #[serde(default)]
    pub owned_samples: BTreeMap<String, String>,
    /// Previously injected slider tick / body edits so repeat exports can restore them cleanly.
    #[serde(default)]
    pub slider_history: SliderEditHistory,
    /// Mapset-wide 12-slot custom sample index schema persisted across exports.
    #[serde(default)]
    pub index_pool: CustomIndexPool,
}

/// Summary of stale files moved out of the mapset during automatic clean-up.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaleCleanupReport {
    pub moved_to_removed: Vec<String>,
    pub skipped_modified: Vec<String>,
    pub skipped_retained: Vec<String>,
}

/// Result of an isolated `HitsoundOnly` folder export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HitsoundOnlyExportResult {
    pub output_dir: PathBuf,
    pub osu_files: Vec<String>,
    pub sample_files: Vec<String>,
    pub copied_song: Option<String>,
}

pub fn manifest_path(data_root: &Path, mapset_root: &Path) -> PathBuf {
    let canonical = mapset_root
        .canonicalize()
        .unwrap_or_else(|_| mapset_root.to_path_buf());
    data_root
        .join("export-manifests")
        .join(format!("manifest-{}.json", path_digest(&canonical)))
}

pub fn load_manifest(data_root: &Path, mapset_root: &Path) -> ExportOwnershipManifest {
    let path = manifest_path(data_root, mapset_root);
    let Ok(bytes) = fs::read(&path) else {
        return ExportOwnershipManifest {
            version: MANIFEST_VERSION,
            ..Default::default()
        };
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|_| ExportOwnershipManifest {
        version: MANIFEST_VERSION,
        ..Default::default()
    })
}

pub fn save_manifest(
    data_root: &Path,
    mapset_root: &Path,
    manifest: &ExportOwnershipManifest,
) -> Result<(), String> {
    let path = manifest_path(data_root, mapset_root);
    let Some(parent) = path.parent() else {
        return Err("Invalid export manifest path.".into());
    };
    fs::create_dir_all(parent).map_err(|e| format!("Could not create export manifest folder: {e}"))?;
    let mut stamped = manifest.clone();
    stamped.version = MANIFEST_VERSION;
    let bytes = serde_json::to_vec_pretty(&stamped)
        .map_err(|e| format!("Could not encode export manifest: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes).map_err(|e| format!("Could not write export manifest: {e}"))?;
    if fs::rename(&tmp, &path).is_err() {
        let _ = fs::remove_file(&path);
        fs::rename(&tmp, &path).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("Could not replace export manifest: {e}")
        })?;
    }
    Ok(())
}

/// Automatically moves stale dawsu!-owned custom sample files out of `mapset_root` into
/// `<data_root>/removed-samples/<mapset-hash>/` so repeat exports leave zero unused files.
///
/// Safety guarantees:
/// - Never touches files not listed in `manifest.owned_samples` unless `take_over_samples` is
///   explicitly opted in AND the file is in `takeover_candidates` (in which case a backup is
///   saved under `backups_root` first).
/// - Never touches a previously owned file whose bytes were modified externally by the mapper
///   (unless `take_over_samples` is true).
/// - Never removes any filename in `retained_names` (active export targets, undo history, or
///   crash recovery references).
pub fn cleanup_stale_samples(
    mapset_root: &Path,
    data_root: &Path,
    backups_root: &Path,
    manifest: &mut ExportOwnershipManifest,
    active_samples: &BTreeSet<String>,
    retained_names: &BTreeSet<String>,
    take_over_samples: bool,
    takeover_candidates: &BTreeSet<String>,
) -> Result<StaleCleanupReport, String> {
    let mut report = StaleCleanupReport::default();
    let canonical = mapset_root
        .canonicalize()
        .unwrap_or_else(|_| mapset_root.to_path_buf());
    let removed_dir = data_root
        .join("removed-samples")
        .join(path_digest(&canonical));

    let active_lower: BTreeSet<String> = active_samples
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let retained_lower: BTreeSet<String> = retained_names
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();

    let previously_owned: Vec<(String, String)> = manifest
        .owned_samples
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    for (name_lower, recorded_digest) in previously_owned {
        if active_lower.contains(&name_lower) {
            continue;
        }
        if retained_lower.contains(&name_lower) {
            report.skipped_retained.push(name_lower);
            continue;
        }
        let Some(disk_path) = find_case_insensitive_file(mapset_root, &name_lower) else {
            manifest.owned_samples.remove(&name_lower);
            continue;
        };
        let Ok(disk_bytes) = fs::read(&disk_path) else {
            continue;
        };
        let disk_digest = sha256_hex(&disk_bytes);
        if disk_digest != recorded_digest && !take_over_samples {
            report.skipped_modified.push(name_lower);
            continue;
        }
        if disk_digest != recorded_digest && take_over_samples {
            backup_sample_file(&disk_path, backups_root);
        }
        move_to_removed(&disk_path, &removed_dir)?;
        manifest.owned_samples.remove(&name_lower);
        report.moved_to_removed.push(name_lower);
    }

    if take_over_samples {
        for candidate in takeover_candidates {
            let lower = candidate.to_ascii_lowercase();
            if active_lower.contains(&lower)
                || retained_lower.contains(&lower)
                || manifest.owned_samples.contains_key(&lower)
            {
                continue;
            }
            if let Some(disk_path) = find_case_insensitive_file(mapset_root, &lower) {
                backup_sample_file(&disk_path, backups_root);
                move_to_removed(&disk_path, &removed_dir)?;
                report.moved_to_removed.push(lower);
            }
        }
    }

    Ok(report)
}

/// Copies an unchanged single sample file straight through when `SingleSampleFormat::Source` is
/// selected and the file is already a valid `.wav` or `.ogg` file.
pub fn passthrough_single_sample(path: &Path, single_format: SingleSampleFormat) -> Option<(String, Vec<u8>)> {
    if single_format != SingleSampleFormat::Source {
        return None;
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext != "wav" && ext != "ogg" {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    Some((ext, bytes))
}

/// Writes an isolated Hitsound-only folder outside the mapset containing:
/// - one hitsound-only `.osu` per planned difficulty (gameplay sliders/spinners simplified to
///   passive circles at their hitsounded moments if requested, or planned `.osu` copies),
/// - every staged sample file,
/// - a copy of the mapset's song audio when present.
///
/// The original mapset folder is NEVER written to.
pub fn write_hitsound_only_directory(
    mapset_root: &Path,
    output_dir: &Path,
    osu_files: &[(String, Vec<u8>)],
    samples: &[(String, Vec<u8>)],
) -> Result<HitsoundOnlyExportResult, String> {
    if output_dir == mapset_root
        || output_dir
            .canonicalize()
            .ok()
            .zip(mapset_root.canonicalize().ok())
            .is_some_and(|(a, b)| a == b)
    {
        return Err("Hitsound-only export directory must be separate from the mapset folder.".into());
    }
    fs::create_dir_all(output_dir)
        .map_err(|e| format!("Could not create hitsound-only export directory: {e}"))?;

    let mut written_osu = Vec::with_capacity(osu_files.len());
    let mut audio_filename: Option<String> = None;

    for (name, bytes) in osu_files {
        check_map_path(name).map_err(|e| format!("Unsafe output filename {name}: {e:?}"))?;
        if audio_filename.is_none() {
            let parsed = OsuFile::read(bytes.clone()).file;
            if let Some(af) = parsed.general("AudioFilename") {
                let trimmed = af.trim().trim_matches('"');
                if !trimmed.is_empty() && check_map_path(trimmed).is_ok() {
                    audio_filename = Some(trimmed.to_owned());
                }
            }
        }
        let dest = output_dir.join(name);
        fs::write(&dest, bytes)
            .map_err(|e| format!("Could not write hitsound-only file {name}: {e}"))?;
        written_osu.push(name.clone());
    }

    let mut written_samples = Vec::with_capacity(samples.len());
    for (name, bytes) in samples {
        check_map_path(name).map_err(|e| format!("Unsafe sample filename {name}: {e:?}"))?;
        let dest = output_dir.join(name);
        fs::write(&dest, bytes)
            .map_err(|e| format!("Could not write sample file {name}: {e}"))?;
        written_samples.push(name.clone());
    }

    let mut copied_song = None;
    if let Some(ref song_name) = audio_filename {
        let src_song = mapset_root.join(song_name);
        if src_song.is_file() {
            let dst_song = output_dir.join(song_name);
            if let Some(parent) = dst_song.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::copy(&src_song, &dst_song)
                .map_err(|e| format!("Could not copy song audio {song_name}: {e}"))?;
            copied_song = Some(song_name.clone());
        }
    }

    Ok(HitsoundOnlyExportResult {
        output_dir: output_dir.to_path_buf(),
        osu_files: written_osu,
        sample_files: written_samples,
        copied_song,
    })
}

fn find_case_insensitive_file(dir: &Path, target_lower: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().to_ascii_lowercase() == target_lower {
            let path = entry.path();
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn backup_sample_file(src: &Path, backups_root: &Path) {
    let dest_dir = backups_root.join("takeover-samples");
    if fs::create_dir_all(&dest_dir).is_ok()
        && let Some(name) = src.file_name()
    {
        let _ = fs::copy(src, dest_dir.join(name));
    }
}

fn move_to_removed(src: &Path, removed_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(removed_dir)
        .map_err(|e| format!("Could not create removed-samples folder: {e}"))?;
    let file_name = src
        .file_name()
        .ok_or("Missing filename for stale sample removal.")?;
    let dest = removed_dir.join(file_name);
    if dest.exists() {
        let _ = fs::remove_file(&dest);
    }
    match fs::rename(src, &dest) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices || cfg!(windows) => {
            fs::copy(src, &dest).map_err(|e| format!("Could not archive stale sample: {e}"))?;
            fs::remove_file(src).map_err(|e| format!("Could not remove stale sample: {e}"))?;
            Ok(())
        }
        Err(err) => Err(format!("Could not move stale sample to removed-samples: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;

    #[test]
    fn cleanup_moves_stale_owned_samples_and_spares_modified_or_retained_files() {
        let scratch = Scratch::new("export-manifest-cleanup");
        let mapset = scratch.path("mapset");
        let data = scratch.path("data");
        let backups = scratch.path("backups");
        fs::create_dir_all(&mapset).unwrap();

        let stale_bytes = b"RIFFstale".to_vec();
        let kept_bytes = b"RIFFkept".to_vec();
        let modified_bytes = b"RIFFmapper-edited".to_vec();
        let retained_bytes = b"RIFFundo".to_vec();
        fs::write(mapset.join("dawsu-mix-stale.wav"), &stale_bytes).unwrap();
        fs::write(mapset.join("dawsu-mix-kept.wav"), &kept_bytes).unwrap();
        fs::write(mapset.join("dawsu-mix-edited.wav"), &modified_bytes).unwrap();
        fs::write(mapset.join("dawsu-mix-undo.wav"), &retained_bytes).unwrap();

        let mut manifest = ExportOwnershipManifest {
            version: MANIFEST_VERSION,
            owned_samples: BTreeMap::from([
                ("dawsu-mix-stale.wav".into(), sha256_hex(&stale_bytes)),
                ("dawsu-mix-kept.wav".into(), sha256_hex(&kept_bytes)),
                ("dawsu-mix-edited.wav".into(), sha256_hex(b"original-bytes")),
                ("dawsu-mix-undo.wav".into(), sha256_hex(&retained_bytes)),
            ]),
            ..Default::default()
        };

        let active = BTreeSet::from(["dawsu-mix-kept.wav".to_string()]);
        let retained = BTreeSet::from(["dawsu-mix-undo.wav".to_string()]);
        let report = cleanup_stale_samples(
            &mapset,
            &data,
            &backups,
            &mut manifest,
            &active,
            &retained,
            false,
            &BTreeSet::new(),
        )
        .unwrap();

        assert_eq!(report.moved_to_removed, vec!["dawsu-mix-stale.wav"]);
        assert_eq!(report.skipped_modified, vec!["dawsu-mix-edited.wav"]);
        assert_eq!(report.skipped_retained, vec!["dawsu-mix-undo.wav"]);
        assert!(!mapset.join("dawsu-mix-stale.wav").exists());
        assert!(mapset.join("dawsu-mix-kept.wav").exists());
        assert!(mapset.join("dawsu-mix-edited.wav").exists());
        assert!(mapset.join("dawsu-mix-undo.wav").exists());
    }

    #[test]
    fn hitsound_only_export_leaves_mapset_untouched_and_copies_song() {
        let scratch = Scratch::new("hitsound-only-dir");
        let mapset = scratch.path("mapset");
        let out_dir = scratch.path("hitsound-only");
        fs::create_dir_all(&mapset).unwrap();
        fs::write(mapset.join("song.mp3"), b"fake-mp3").unwrap();
        let osu = b"osu file format v14\r\n\r\n[General]\r\nAudioFilename: song.mp3\r\nMode: 0\r\n".to_vec();

        let res = write_hitsound_only_directory(
            &mapset,
            &out_dir,
            &[("Map [Hard].osu".into(), osu)],
            &[("drum-hitclap2.wav".into(), b"RIFF".to_vec())],
        )
        .unwrap();

        assert_eq!(res.copied_song.as_deref(), Some("song.mp3"));
        assert!(out_dir.join("song.mp3").is_file());
        assert!(out_dir.join("Map [Hard].osu").is_file());
        assert!(out_dir.join("drum-hitclap2.wav").is_file());
        assert!(!mapset.join("drum-hitclap2.wav").exists());
    }

    #[test]
    fn passthrough_preserves_wav_and_ogg_sources_only_in_source_mode() {
        let scratch = Scratch::new("passthrough");
        let ogg = scratch.path("clap.ogg");
        fs::write(&ogg, b"OggS1234").unwrap();
        assert_eq!(
            passthrough_single_sample(&ogg, SingleSampleFormat::Source),
            Some(("ogg".into(), b"OggS1234".to_vec()))
        );
        assert_eq!(
            passthrough_single_sample(&ogg, SingleSampleFormat::MatchMixFormat),
            None
        );
    }
}
