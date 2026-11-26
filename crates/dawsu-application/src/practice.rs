//! Pure-Rust bundled practice mapset generator for onboarding and first-run exploration.

use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const SAMPLE_RATE: u32 = 44_100;
const PRACTICE_DIR_NAME: &str = "dawsu-practice";
const PROJECTS_DIR_NAME: &str = "projects";
const BACKUPS_DIR_NAME: &str = "backups";
const SESSION_PREFIX: &str = "session-";

/// Maximum age before inactive practice session folders in temp storage are pruned.
pub const PRACTICE_SESSION_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Filename of the primary `[Hard]` practice difficulty.
pub const HARD_FILENAME: &str = "dawsu! Studio - dawsu! Demo Track (Demo) [Hard].osu";
/// Filename of the companion `[Normal]` practice difficulty.
pub const NORMAL_FILENAME: &str = "dawsu! Studio - dawsu! Demo Track (Demo) [Normal].osu";

const HARD_OBJECTS: &[&str] = &[
    "128,192,1000,1,0,3:0:1:90:",
    "224,192,1500,1,8,3:3:1:85:",
    "320,192,2000,1,0,3:0:1:90:",
    "384,192,2250,1,8,3:3:1:80:",
    "320,256,2500,1,2,2:2:1:85:",
    "192,256,3000,2,0,L|352:256,1,140,0|2,3:0|2:2,0:0:0:0:",
    "128,192,4000,1,0,3:0:1:95:",
    "256,192,4500,1,8,3:3:1:90:",
    "384,192,5000,1,2,2:2:1:85:",
];

// Simpler rhythm: omits 2250 ms and adds an unhitsounded circle at 5500 ms.
const NORMAL_OBJECTS: &[&str] = &[
    "128,192,1000,1,0,0:0:0:0:",
    "256,192,1500,1,0,0:0:0:0:",
    "384,192,2000,1,0,0:0:0:0:",
    "256,256,2500,1,0,0:0:0:0:",
    "192,256,3000,2,0,L|352:256,1,140,0|0,0:0|0:0,0:0:0:0:",
    "128,192,4000,1,0,0:0:0:0:",
    "256,192,4500,1,0,0:0:0:0:",
    "384,192,5000,1,0,0:0:0:0:",
    "256,128,5500,1,0,0:0:0:0:",
];

static NEXT_SESSION_SEQ: AtomicU64 = AtomicU64::new(0);

/// Returns the shared namespace folder under OS temp storage for bundled practice mapsets.
pub fn practice_temp_root() -> PathBuf {
    std::env::temp_dir().join(PRACTICE_DIR_NAME)
}

fn canonical_practice_temp_root() -> PathBuf {
    std::env::temp_dir()
        .canonicalize()
        .unwrap_or_else(|_| std::env::temp_dir())
        .join(PRACTICE_DIR_NAME)
}

fn strip_windows_verbatim(path: &Path) -> PathBuf {
    let Some(raw) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(rest) = raw.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

fn normalize_component_key(component: Component<'_>) -> Option<OsString> {
    match component {
        Component::CurDir => None,
        other => {
            let raw = other.as_os_str();
            if cfg!(windows) {
                Some(raw.to_string_lossy().to_ascii_lowercase().into())
            } else {
                Some(raw.to_os_string())
            }
        }
    }
}

fn normalized_components(path: &Path) -> Vec<OsString> {
    let clean = strip_windows_verbatim(path);
    clean
        .components()
        .filter_map(normalize_component_key)
        .collect()
}

pub(crate) fn is_path_under_root(path: &Path, root: &Path) -> bool {
    let path_parts = normalized_components(path);
    let root_parts = normalized_components(root);
    !root_parts.is_empty()
        && path_parts.len() >= root_parts.len()
        && path_parts[..root_parts.len()] == root_parts[..]
}

pub(crate) fn same_normalized_path(a: &Path, b: &Path) -> bool {
    if normalized_components(a) == normalized_components(b) {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => normalized_components(&ca) == normalized_components(&cb),
        _ => false,
    }
}

/// Returns `true` when `path` resides inside the bundled temp practice namespace.
pub fn is_practice_mapset_path(path: &Path) -> bool {
    let raw_root = practice_temp_root();
    let canon_root = canonical_practice_temp_root();
    let candidates = [Some(path.to_path_buf()), path.canonicalize().ok()];
    for candidate in candidates.into_iter().flatten() {
        if is_path_under_root(&candidate, &raw_root) || is_path_under_root(&candidate, &canon_root) {
            return true;
        }
    }
    false
}

fn practice_session_root_and_name(path: &Path) -> Option<(PathBuf, OsString)> {
    for ancestor in path.ancestors() {
        let Some(name) = ancestor.file_name() else {
            continue;
        };
        if name.to_string_lossy().starts_with(SESSION_PREFIX)
            && let Some(root) = ancestor.parent()
        {
            return Some((strip_windows_verbatim(root), name.to_os_string()));
        }
    }
    None
}

/// Returns an isolated per-session project directory under the practice temp root so session files
/// never pollute `%LOCALAPPDATA%\dawsu-rs\projects\`.
pub(crate) fn practice_project_root_for(path: &Path) -> Option<PathBuf> {
    let (root, session_name) = practice_session_root_and_name(path)?;
    Some(root.join(PROJECTS_DIR_NAME).join(session_name))
}

/// Returns an isolated per-session save-backup directory under the practice temp root.
pub(crate) fn practice_backups_dir_for(path: &Path) -> Option<PathBuf> {
    let (root, session_name) = practice_session_root_and_name(path)?;
    Some(root.join(BACKUPS_DIR_NAME).join(session_name))
}

fn synth_kick(duration_sec: f64) -> Vec<f32> {
    let count = (f64::from(SAMPLE_RATE) * duration_sec).floor() as usize;
    let mut out = Vec::with_capacity(count);
    let mut phase = 0.0_f64;
    let rate = f64::from(SAMPLE_RATE);
    for i in 0..count {
        let t = i as f64 / rate;
        let freq = 55.0 + 120.0 * (-t * 36.0).exp();
        phase += (2.0 * std::f64::consts::PI * freq) / rate;
        let env = (-t * 22.0).exp();
        out.push((phase.sin() * env * 0.85) as f32);
    }
    out
}

fn synth_clap(duration_sec: f64) -> Vec<f32> {
    let count = (f64::from(SAMPLE_RATE) * duration_sec).floor() as usize;
    let mut out = Vec::with_capacity(count);
    let mut seed: u32 = 1337;
    let rate = f64::from(SAMPLE_RATE);
    for i in 0..count {
        // Explicit wrapping matches JS float-masked 31-bit LCG without debug-mode overflow panics.
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) & 0x7fff_ffff;
        let rand = (f64::from(seed) / f64::from(0x7fff_ffff_u32)) * 2.0 - 1.0;
        let t = i as f64 / rate;
        let burst = if t < 0.018 {
            0.9
        } else {
            (-(t - 0.018) * 32.0).exp()
        };
        out.push((rand * burst * 0.65) as f32);
    }
    out
}

fn synth_whistle(duration_sec: f64) -> Vec<f32> {
    let count = (f64::from(SAMPLE_RATE) * duration_sec).floor() as usize;
    let mut out = Vec::with_capacity(count);
    let rate = f64::from(SAMPLE_RATE);
    for i in 0..count {
        let t = i as f64 / rate;
        let env = (-t * 18.0).exp();
        let tone = (2.0 * std::f64::consts::PI * 1320.0 * t).sin() * 0.45
            + (2.0 * std::f64::consts::PI * 1980.0 * t).sin() * 0.2;
        out.push((tone * env) as f32);
    }
    out
}

fn synth_song(duration_sec: f64) -> Vec<f32> {
    let count = (f64::from(SAMPLE_RATE) * duration_sec).floor() as usize;
    let mut out = Vec::with_capacity(count);
    let chords = [220.0_f64, 261.63, 329.63, 392.0];
    let rate = f64::from(SAMPLE_RATE);
    for i in 0..count {
        let t = i as f64 / rate;
        let beat = (t / 0.5).floor() as u32;
        let beat_frac = (t % 0.5) / 0.5;
        let pulse = (-beat_frac * 6.0).exp() * 0.18;
        let bass_freq = if beat % 4 < 2 { 110.0 } else { 130.81 };
        let bass = (2.0 * std::f64::consts::PI * bass_freq * t).sin() * pulse;
        let pad: f64 = chords
            .iter()
            .map(|&f| (2.0 * std::f64::consts::PI * f * t).sin() * 0.045)
            .sum();
        out.push((bass + pad) as f32);
    }
    out
}

fn build_osu(version: &str, objects: &[&str]) -> String {
    let header = [
        "osu file format v14",
        "",
        "[General]",
        "AudioFilename: demo-song.wav",
        "AudioLeadIn: 0",
        "PreviewTime: 1000",
        "Countdown: 0",
        "SampleSet: Soft",
        "StackLeniency: 0.7",
        "Mode: 0",
        "",
        "[Metadata]",
        "Title: dawsu! Demo Track",
        "Artist: dawsu! Studio",
        "Creator: Demo",
    ];
    let middle = [
        "",
        "[Difficulty]",
        "HPDrainRate: 5",
        "CircleSize: 4",
        "OverallDifficulty: 6",
        "ApproachRate: 8",
        "SliderMultiplier: 1.4",
        "SliderTickRate: 1",
        "",
        "[TimingPoints]",
        "500,500,4,2,1,85,1,0",
        "",
        "[HitObjects]",
    ];
    let mut lines: Vec<String> = header.iter().map(|s| (*s).to_owned()).collect();
    lines.push(format!("Version: {version}"));
    lines.extend(middle.iter().map(|s| (*s).to_owned()));
    lines.extend(objects.iter().map(|s| (*s).to_owned()));
    lines.push(String::new());
    lines.join("\r\n")
}

fn prune_stale_sessions(root: &Path, active_folder: Option<&Path>, max_age: Duration) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(SESSION_PREFIX) {
            continue;
        }
        let path = entry.path();
        if active_folder.is_some_and(|active| same_normalized_path(&path, active)) {
            continue;
        }
        // Always read live metadata on Windows rather than DirEntry::metadata() stale $I30 cache.
        let expired = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if expired {
            let _ = fs::remove_dir_all(&path);
            let _ = fs::remove_dir_all(root.join(PROJECTS_DIR_NAME).join(&name));
            let _ = fs::remove_dir_all(root.join(BACKUPS_DIR_NAME).join(&name));
        }
    }
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let bytes = crate::sample_checks::encode_pcm16_wav(samples, SAMPLE_RATE, 1)?;
    fs::write(path, bytes).map_err(|e| format!("Writing {}: {e}", path.display()))
}

/// Materializes the bundled synthetic practice mapset inside a fresh `session-*` directory in `root`,
/// best-effort prunes stale sibling sessions older than `max_age`, and returns the `[Hard].osu` path.
pub fn materialize_practice_mapset_in(
    root: &Path,
    active_folder: Option<&Path>,
    max_age: Duration,
) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|e| format!("Creating practice folder {}: {e}", root.display()))?;
    prune_stale_sessions(root, active_folder, max_age);

    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let seq = NEXT_SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    let session_dir = root.join(format!("{SESSION_PREFIX}{unix_ms}-{}-{seq}", std::process::id()));
    fs::create_dir(&session_dir)
        .map_err(|e| format!("Creating practice session {}: {e}", session_dir.display()))?;

    let write_all = || -> Result<PathBuf, String> {
        write_wav(&session_dir.join("demo-song.wav"), &synth_song(8.0))?;
        write_wav(&session_dir.join("drum-hitnormal.wav"), &synth_kick(0.14))?;
        write_wav(&session_dir.join("drum-hitclap.wav"), &synth_clap(0.12))?;
        write_wav(&session_dir.join("soft-hitwhistle.wav"), &synth_whistle(0.16))?;

        let hard_path = session_dir.join(HARD_FILENAME);
        let normal_path = session_dir.join(NORMAL_FILENAME);
        fs::write(&hard_path, build_osu("Hard", HARD_OBJECTS).as_bytes())
            .map_err(|e| format!("Writing {}: {e}", hard_path.display()))?;
        fs::write(&normal_path, build_osu("Normal", NORMAL_OBJECTS).as_bytes())
            .map_err(|e| format!("Writing {}: {e}", normal_path.display()))?;
        Ok(hard_path.canonicalize().unwrap_or(hard_path))
    };

    match write_all() {
        Ok(path) => Ok(path),
        Err(error) => {
            let _ = fs::remove_dir_all(&session_dir);
            Err(error)
        }
    }
}

/// Materializes the bundled synthetic practice mapset inside [`practice_temp_root()`].
pub fn materialize_practice_mapset(active_folder: Option<&Path>) -> Result<PathBuf, String> {
    materialize_practice_mapset_in(&practice_temp_root(), active_folder, PRACTICE_SESSION_MAX_AGE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Application, sample_checks};
    use dawsu_foundation::{Hitsound, SampleIdentity, SampleSet, Sound};
    use dawsu_hitsounding::MomentId;
    use dawsu_test_support::Scratch;
    use std::time::Instant;

    struct SessionCleanup(Option<PathBuf>);

    impl Drop for SessionCleanup {
        fn drop(&mut self) {
            if let Some(map) = self.0.take() {
                if let Some(dir) = map.parent() {
                    let _ = fs::remove_dir_all(dir);
                }
                if let Some(project) = practice_project_root_for(&map) {
                    let _ = fs::remove_dir_all(project);
                }
                if let Some(backups) = practice_backups_dir_for(&map) {
                    let _ = fs::remove_dir_all(backups);
                }
            }
        }
    }

    fn wait_idle(app: &mut Application) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.is_busy() {
            app.poll();
            assert!(Instant::now() < deadline, "application stayed busy");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn folder_file_count(path: &Path) -> usize {
        let Ok(entries) = fs::read_dir(path) else {
            return 0;
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| {
                let p = entry.path();
                if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                    folder_file_count(&p)
                } else {
                    1
                }
            })
            .sum()
    }

    #[test]
    fn materialize_writes_six_healthy_files_passing_sample_checks() {
        let scratch = Scratch::new("practice-materialize");
        let hard = materialize_practice_mapset_in(scratch.dir(), None, PRACTICE_SESSION_MAX_AGE)
            .expect("practice mapset writes cleanly");
        let dir = hard.parent().expect("session dir").to_path_buf();
        let expected = [
            "demo-song.wav",
            "drum-hitnormal.wav",
            "drum-hitclap.wav",
            "soft-hitwhistle.wav",
            HARD_FILENAME,
            NORMAL_FILENAME,
        ];
        for name in expected {
            assert!(dir.join(name).is_file(), "missing {name}");
        }
        for hs in ["drum-hitnormal.wav", "drum-hitclap.wav", "soft-hitwhistle.wav"] {
            let report = sample_checks::inspect_file(&dir.join(hs)).expect("valid wav");
            assert!(report.is_healthy(), "{hs} had issues: {:?}", report.issues);
        }
        let files: Vec<String> = expected.iter().map(|s| (*s).to_owned()).collect();
        let issues = sample_checks::audit_folder(&dir, &files, Some("demo-song.wav"));
        assert!(issues.is_empty(), "audit_folder reported issues: {issues:?}");
    }

    #[test]
    fn path_matching_handles_verbatim_case_symlinks_missing_subpaths_and_prefixes() {
        let raw_root = practice_temp_root();
        let missing_child = raw_root.join("session-missing").join(HARD_FILENAME);
        assert!(is_practice_mapset_path(&missing_child));

        let canon_child = canonical_practice_temp_root()
            .join("session-missing")
            .join(HARD_FILENAME);
        assert!(is_practice_mapset_path(&canon_child));

        let sibling_prefix = std::env::temp_dir()
            .join("dawsu-practice-other")
            .join("session-1")
            .join(HARD_FILENAME);
        assert!(!is_practice_mapset_path(&sibling_prefix));

        let scratch = Scratch::new("practice-canon-real");
        let hard = materialize_practice_mapset_in(scratch.dir(), None, PRACTICE_SESSION_MAX_AGE)
            .unwrap();
        assert!(is_path_under_root(&hard, scratch.dir()));
        assert!(!is_path_under_root(&hard, &scratch.dir().with_file_name("dawsu-other")));

        if cfg!(windows) {
            assert!(is_path_under_root(
                Path::new(r"\\?\C:\Temp\dawsu-practice\session-1\Map.osu"),
                Path::new(r"c:\temp\DAWSU-PRACTICE")
            ));
            assert!(!is_path_under_root(
                Path::new(r"\\?\C:\Temp\dawsu-practice-other\session-1\Map.osu"),
                Path::new(r"C:\Temp\dawsu-practice")
            ));
        }

        // Canonical path under real practice_temp_root matches even when temp_dir() uses a short or
        // symlinked prefix (e.g. Windows 8.3 short name or macOS /var -> /private/var).
        let session_in_temp = materialize_practice_mapset(None).unwrap();
        let _cleanup = SessionCleanup(Some(session_in_temp.clone()));
        assert!(is_practice_mapset_path(&session_in_temp));
    }

    #[test]
    fn stale_sessions_prune_while_canonical_active_session_and_other_dirs_stay() {
        let scratch = Scratch::new("practice-prune");
        let stale = materialize_practice_mapset_in(scratch.dir(), None, PRACTICE_SESSION_MAX_AGE)
            .unwrap();
        let active = materialize_practice_mapset_in(scratch.dir(), None, PRACTICE_SESSION_MAX_AGE)
            .unwrap();
        let stale_dir = stale.parent().unwrap().to_path_buf();
        let active_dir_canon = active.parent().unwrap().canonicalize().unwrap();
        let stale_project = practice_project_root_for(&stale).unwrap();
        let stale_backup = practice_backups_dir_for(&stale).unwrap();
        fs::create_dir_all(&stale_project).unwrap();
        fs::create_dir_all(&stale_backup).unwrap();
        fs::write(stale_project.join("session.json"), b"{}").unwrap();
        let non_session = scratch.path("custom-notes");
        fs::create_dir_all(&non_session).unwrap();

        std::thread::sleep(Duration::from_millis(45));

        let fresh = materialize_practice_mapset_in(
            scratch.dir(),
            Some(&active_dir_canon),
            Duration::from_millis(25),
        )
        .unwrap();
        assert!(!stale_dir.exists(), "stale session dir should be pruned");
        assert!(!stale_project.exists(), "stale project dir should be pruned");
        assert!(!stale_backup.exists(), "stale backup dir should be pruned");
        assert!(active.parent().unwrap().exists(), "active session dir must stay");
        assert!(fresh.parent().unwrap().exists(), "fresh session dir must exist");
        assert!(non_session.exists(), "non-session dir must stay untouched");
    }

    #[test]
    fn headless_practice_open_loads_cleanly_guards_dirty_and_writes_zero_data_root_files() {
        let data_scratch = Scratch::new("practice-data-root");
        let data_root = data_scratch.dir().to_path_buf();
        let mut app = Application::headless_with_projects(data_root.clone());

        app.request_open_practice_mapset();
        wait_idle(&mut app);
        assert!(app.error().is_none(), "{:?}", app.error());

        let loaded_path = app.document().expect("practice doc open").path.clone();
        let _cleanup = SessionCleanup(Some(loaded_path.clone()));
        assert!(is_practice_mapset_path(&loaded_path));
        assert!(app.warnings.is_empty(), "warnings: {:?}", app.warnings);
        assert!(app.preferences().recent.is_empty());
        assert!(!app.has_original_snapshot());

        let doc = app.document().unwrap();
        assert_eq!(doc.difficulties.len(), 2);
        assert!(!doc.composer.lanes.is_empty());
        assert!(!doc.composer.hitsounds().moments().is_empty());

        let deadline = Instant::now() + Duration::from_secs(10);
        while app.is_song_loading() {
            app.poll();
            assert!(Instant::now() < deadline, "song decode did not settle");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!app.waveform.is_empty(), "expected decoded song waveform");

        // Dirty-edit guard refuses re-opening while dirty and preserves edits.
        app.document.as_mut().unwrap().composer.apply("Test edit", |hits| {
            hits.add_hit(
                MomentId(0),
                SampleIdentity::Standard {
                    set: SampleSet::Normal,
                    sound: Sound::Hit(Hitsound::Finish),
                    index: dawsu_foundation::CustomIndex::new(0),
                },
            );
        });
        assert!(app.is_dirty());
        app.request_open_practice_mapset();
        assert!(app.error().is_some());
        assert_eq!(app.document().unwrap().path, loaded_path);
        app.set_error(None);

        // Save, switch to [Normal].osu, and close: data_root stays free of practice artifacts.
        app.save();
        wait_idle(&mut app);
        assert!(app.error().is_none(), "{:?}", app.error());

        let normal_path = loaded_path.parent().unwrap().join(NORMAL_FILENAME);
        app.request_open(normal_path);
        wait_idle(&mut app);
        assert!(app.error().is_none(), "{:?}", app.error());
        assert!(app.preferences().recent.is_empty());

        app.close_document();
        // Give the original_snapshot background check (never spawned for practice) no chance to race.
        std::thread::sleep(Duration::from_millis(40));

        assert_eq!(folder_file_count(&data_root.join("original-snapshots")), 0);
        assert_eq!(folder_file_count(&data_root.join("projects")), 0);
        assert_eq!(folder_file_count(&data_root.join("recovery")), 0);
        assert_eq!(folder_file_count(&data_root.join("backups")), 0);
        let session_dirs = fs::read_dir(data_root.join("sessions"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        assert_eq!(session_dirs, 0, "Presence::drop must clean up empty sessions/<hash> dirs");
    }
}
