//! Optional, on-demand local CPU drum stem separation using HT-Demucs ONNX.
//!
//! Zero files are ever written to the osu! mapset folder:
//! - Downloaded model & ONNX Runtime library live under `<app_data>/models/`.
//! - Separated drum stems live under `<app_data>/stems/<song-sha256>-drums.wav`.
//! - Network requests happen ONLY when the user explicitly clicks "Separate drums".

use crate::Application;
use dawsu_audio::AudioClip;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

pub const TARGET_SAMPLE_RATE: u32 = 44_100;
/// HT-Demucs fixed input window length: 7.8 s at 44,100 Hz.
pub const CHUNK_SAMPLES: usize = 343_980;
/// 25% overlap between consecutive chunks (`CHUNK_SAMPLES * 3 / 4`).
pub const HOP_SAMPLES: usize = 257_985;
/// Default HT-Demucs fp16-weights ONNX model (~166 MB).
pub const DEFAULT_MODEL_FILENAME: &str = "htdemucs_fp16weights.onnx";
/// Pinned to one revision of the repository, so the address always serves the same file.
pub const DEFAULT_MODEL_URL: &str = "https://huggingface.co/StemSplitio/htdemucs-onnx/resolve/d54ed9eb60e258ea82131c6ee14578628816456a/htdemucs_fp16weights.onnx";
/// SHA-256 and size of the pinned model (the repository's LFS object id). A download or a cached
/// copy that differs is refused, so a changed or damaged file is never run.
pub const DEFAULT_MODEL_SHA256: &str = "d05c269d0178d2a72ad484b10b11dd370193fc923201c3b27a99f848745db70a";
pub const DEFAULT_MODEL_SIZE: u64 = 165_612_636;
/// Official Microsoft ONNX Runtime release version for on-demand dynamic loading.
pub const ORT_RELEASE_VERSION: &str = "1.22.0";
/// Microsoft's official NuGet package of that release. One package holds the native library for
/// every supported platform, and nuget.org publishes its hash, so it is pinned here and checked
/// before anything is unpacked (its SHA-512 matched nuget.org's catalog entry on 2026-11-24).
pub const ORT_PACKAGE_URL: &str =
    "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/1.22.0/microsoft.ml.onnxruntime.1.22.0.nupkg";
pub const ORT_PACKAGE_SHA256: &str = "d571e63a2329baacb713f441e65ad75284de354db6e1ac435fe4bebbb417986a";
pub const ORT_PACKAGE_SIZE: u64 = 121_402_697;

static ATOMIC_TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);
static ORT_ENV: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum StemSeparationJobState {
    #[default]
    Idle,
    DownloadingRuntime,
    #[serde(rename_all = "camelCase")]
    DownloadingModel {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Separating {
        chunk: usize,
        total_chunks: usize,
    },
    #[serde(rename_all = "camelCase")]
    Done {
        stem_path: String,
    },
    Cancelled,
    #[serde(rename_all = "camelCase")]
    Failed {
        message: String,
    },
}

impl StemSeparationJobState {
    pub fn is_running(&self) -> bool {
        matches!(
            self,
            Self::DownloadingRuntime
                | Self::DownloadingModel { .. }
                | Self::Separating { .. }
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StemSeparationStartOptions {
    #[serde(default)]
    pub force_rerun: bool,
    #[serde(default)]
    pub custom_model_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StemSeparationStatus {
    pub job: StemSeparationJobState,
    pub model_ready: bool,
    pub runtime_ready: bool,
    pub model_size_bytes: Option<u64>,
    pub cached_stem_path: Option<String>,
    pub song_loaded: bool,
    pub song_name: Option<String>,
    pub active_stem_loaded: bool,
    pub active_stem_monitoring: bool,
}

#[derive(Debug)]
struct SharedJob {
    state: Mutex<StemSeparationJobState>,
    cancel: Arc<AtomicBool>,
    generation: u64,
    song_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub(crate) struct StemSeparationManager {
    active: Option<Arc<SharedJob>>,
    generation: u64,
    applied_generation: u64,
}

impl StemSeparationManager {
    fn current_state(&self) -> StemSeparationJobState {
        match &self.active {
            Some(job) => job
                .state
                .lock()
                .map(|g| g.clone())
                .unwrap_or(StemSeparationJobState::Idle),
            None => StemSeparationJobState::Idle,
        }
    }

    fn is_running(&self) -> bool {
        self.current_state().is_running()
    }

    pub fn cancel(&mut self) {
        if let Some(job) = &self.active {
            job.cancel.store(true, Ordering::SeqCst);
            if let Ok(mut guard) = job.state.lock()
                && guard.is_running()
            {
                *guard = StemSeparationJobState::Cancelled;
            }
        }
    }

    fn take_unapplied_done(&mut self, current_song: Option<&Path>) -> Option<PathBuf> {
        let job = self.active.as_ref()?;
        if job.generation == self.applied_generation {
            return None;
        }
        // Only auto-load if the same song is still open.
        if let (Some(job_song), Some(open_song)) = (job.song_path.as_deref(), current_song)
            && job_song != open_song
        {
            return None;
        }
        let state = job.state.lock().ok()?.clone();
        if let StemSeparationJobState::Done { stem_path } = state {
            self.applied_generation = job.generation;
            Some(PathBuf::from(stem_path))
        } else {
            None
        }
    }
}

/// Directory inside `<app_data>` where the optional ONNX model & runtime live.
pub fn models_dir(data_root: &Path) -> PathBuf {
    data_root.join("models")
}

/// Directory inside `<app_data>` where separated drum stem WAVs are cached.
pub fn stems_dir(data_root: &Path) -> PathBuf {
    data_root.join("stems")
}

/// Default path for the downloaded HT-Demucs ONNX model.
pub fn default_model_path(data_root: &Path) -> PathBuf {
    models_dir(data_root).join(DEFAULT_MODEL_FILENAME)
}

/// Deterministic cache path `<app_data>/stems/<song-sha256>-drums.wav`.
pub fn stem_cache_path(data_root: &Path, song_sha256: &str) -> PathBuf {
    stems_dir(data_root).join(format!("{song_sha256}-drums.wav"))
}

fn hex(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

/// Whether the file at `path` has exactly this SHA-256 (lowercase hex). A missing or unreadable
/// file does not match.
pub fn file_matches_sha256(path: &Path, expected_hex: &str) -> bool {
    let Ok(file) = File::open(path) else { return false };
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return false,
        }
    }
    hex(&hasher.finalize()) == expected_hex
}

/// Computes lowercase hex SHA-256 of a file by streaming in 64 KiB chunks.
pub fn file_sha256_hex(path: &Path) -> Result<String, String> {
    let file = File::open(path)
        .map_err(|e| format!("Could not read song file for stem cache key: {e}"))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("Could not hash song file: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    Ok(out)
}

/// Expected shared library filename for ONNX Runtime on the current OS.
pub fn onnxruntime_lib_filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// Where this platform's library sits inside the pinned NuGet package, with its SHA-256 and size.
/// The hashes are of the files in `ORT_PACKAGE_URL` (checked 2026-11-24).
fn onnxruntime_package_entry() -> Result<(&'static str, &'static str, u64), String> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return Ok(("runtimes/win-x64/native/onnxruntime.dll", "579b636403983254346a5c1d80bd28f1519cd1e284cd204f8d4ff41f8d711559", 12_418_080));
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    return Ok(("runtimes/win-arm64/native/onnxruntime.dll", "79281671a386ed1baab9dbdbb09fe55f99577011472e9526cf9d0b468bb6bcc7", 12_679_728));
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok(("runtimes/osx-arm64/native/libonnxruntime.dylib", "6c2f8af5def6e533f17383a4f07dc4eabbd4807d242a478b69f62fcfbe3c1dd3", 33_514_304));
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return Ok(("runtimes/osx-x64/native/libonnxruntime.dylib", "c3b6c4eb78c7f4b52fb51bfff605176a01cee050f829326bb073ddd22fb42849", 37_737_280));
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return Ok(("runtimes/linux-x64/native/libonnxruntime.so", "3da6146e14e7b8aaec625dde11d6114c7457c87a5f93d744897da8781e35c673", 21_042_416));
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return Ok(("runtimes/linux-arm64/native/libonnxruntime.so", "0afd69a0ae38c5099fd0e8604dda398ac43dee67cd9c6394b5142b19e82528de", 17_643_256));
    #[allow(unreachable_code)]
    Err("Automatic ONNX Runtime download is not available for this CPU/OS. Set ORT_DYLIB_PATH to your local onnxruntime library.".into())
}

/// The ONNX Runtime library dawsu! may load: the file named by `ORT_DYLIB_PATH` (the user's own
/// explicit choice), else the pinned library next to the executable or in `<app_data>/models/`.
/// A library elsewhere on the system (`PATH`, System32, `/usr/lib`) is never loaded: Windows ships
/// its own older `onnxruntime.dll`, and an unknown version or a planted file must not run.
/// `verify` checks the pinned library's SHA-256; without it only its size is compared, which is
/// cheap enough for the status the dialog polls.
pub fn find_onnxruntime_library(data_root: &Path, verify: bool) -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("ORT_DYLIB_PATH") {
        let p = PathBuf::from(env_path);
        if p.is_file() {
            return Some(p);
        }
    }
    let (_, expected, size) = onnxruntime_package_entry().ok()?;
    let filename = onnxruntime_lib_filename();
    let next_to_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(filename)));
    next_to_exe
        .into_iter()
        .chain([models_dir(data_root).join(filename)])
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|m| m.is_file() && m.len() == size)
                && (!verify || file_matches_sha256(candidate, expected))
        })
}

fn unique_temp_path(target: &Path) -> PathBuf {
    let seq = ATOMIC_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "tmp".into());
    target.with_file_name(format!(".{name}.tmp.{pid}.{seq}"))
}

fn atomic_rename(tmp: &Path, dest: &Path) -> io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        if dest.exists() {
            let _ = fs::remove_file(dest);
        }
    }
    fs::rename(tmp, dest)
}

/// Downloads `url` to `dest` through a temporary file. The bytes are hashed as they arrive, and
/// the file only takes the name `dest` when its size and SHA-256 equal `expected`; otherwise the
/// temporary file is removed and nothing is installed.
fn download_file_atomic(
    url: &str,
    dest: &Path,
    expected: (&str, u64),
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<(), String> {
    let (expected_sha256, expected_size) = expected;
    if cancel.load(Ordering::SeqCst) {
        return Err("Cancelled".into());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create cache folder: {e}"))?;
    }

    let client = reqwest::blocking::Client::builder()
        .user_agent("dawsu-stem-separation/1.0")
        .build()
        .map_err(|e| format!("Could not start download client: {e}"))?;

    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("Download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Download failed with HTTP status {}.",
            response.status()
        ));
    }

    let total_bytes = response.content_length().or(Some(expected_size));
    let tmp = unique_temp_path(dest);
    let write_result = (|| -> Result<(), String> {
        let mut out = BufWriter::new(
            File::create(&tmp).map_err(|e| format!("Could not create temp file: {e}"))?,
        );
        let mut hasher = Sha256::new();
        let mut downloaded = 0u64;
        let mut buf = [0u8; 64 * 1024];
        on_progress(0, total_bytes);
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err("Cancelled".into());
            }
            let n = response
                .read(&mut buf)
                .map_err(|e| format!("Download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            downloaded += n as u64;
            if downloaded > expected_size {
                return Err(checksum_refusal(url));
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n])
                .map_err(|e| format!("Could not write downloaded file: {e}"))?;
            on_progress(downloaded, total_bytes);
        }
        out.flush()
            .map_err(|e| format!("Could not finish writing file: {e}"))?;
        drop(out);
        if downloaded != expected_size || hex(&hasher.finalize()) != expected_sha256 {
            return Err(checksum_refusal(url));
        }
        atomic_rename(&tmp, dest).map_err(|e| format!("Could not save downloaded file: {e}"))?;
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_result
}

fn checksum_refusal(url: &str) -> String {
    format!(
        "The downloaded file is not the one dawsu! expects, so it was not used ({url}). \
         Try again later; if it keeps happening, the file on the server has changed."
    )
}

/// Copies the zip entry named exactly `entry_name` to `dest_lib` through a temporary file, and
/// installs it only when its SHA-256 equals `expected_sha256`.
fn extract_runtime_from_package(
    package_path: &Path,
    entry_name: &str,
    expected_sha256: &str,
    dest_lib: &Path,
) -> Result<(), String> {
    let file = File::open(package_path)
        .map_err(|e| format!("Could not open the ONNX Runtime package: {e}"))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file))
        .map_err(|e| format!("Could not read the ONNX Runtime package: {e}"))?;
    let mut entry = zip
        .by_name(entry_name)
        .map_err(|_| "ONNX Runtime library was not found inside the downloaded package.".to_string())?;
    let tmp = unique_temp_path(dest_lib);
    let res = (|| -> Result<(), String> {
        let mut out = BufWriter::new(
            File::create(&tmp).map_err(|e| format!("Could not create runtime file: {e}"))?,
        );
        io::copy(&mut entry, &mut out)
            .map_err(|e| format!("Could not extract ONNX Runtime: {e}"))?;
        out.flush()
            .map_err(|e| format!("Could not flush runtime file: {e}"))?;
        drop(out);
        if !file_matches_sha256(&tmp, expected_sha256) {
            return Err("The ONNX Runtime library in the package is not the one dawsu! expects, so it was not installed.".into());
        }
        atomic_rename(&tmp, dest_lib)
            .map_err(|e| format!("Could not install ONNX Runtime: {e}"))
    })();
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

fn ensure_onnxruntime_library(data_root: &Path, cancel: &AtomicBool) -> Result<PathBuf, String> {
    if let Some(existing) = find_onnxruntime_library(data_root, true) {
        return Ok(existing);
    }
    let (entry_name, lib_sha256, _) = onnxruntime_package_entry()?;
    let models = models_dir(data_root);
    fs::create_dir_all(&models)
        .map_err(|e| format!("Could not create models folder: {e}"))?;
    let package_path = models.join(format!("onnxruntime-{ORT_RELEASE_VERSION}.nupkg"));
    let dest_lib = models.join(onnxruntime_lib_filename());

    download_file_atomic(
        ORT_PACKAGE_URL,
        &package_path,
        (ORT_PACKAGE_SHA256, ORT_PACKAGE_SIZE),
        cancel,
        |_, _| {},
    )?;
    let extract_res = extract_runtime_from_package(&package_path, entry_name, lib_sha256, &dest_lib);
    let _ = fs::remove_file(&package_path);
    extract_res?;
    Ok(dest_lib)
}

fn ort_environment(runtime_lib: &Path) -> Result<&'static ort::environment::Environment, String> {
    let result = ORT_ENV.get_or_init(|| {
        ort::init_from(runtime_lib)
            .map_err(|e| format!("Could not load ONNX Runtime ({}): {e}", runtime_lib.display()))?
            .with_name("dawsu-stem-separation")
            .build()
            .map_err(|e| format!("Could not initialize ONNX Runtime: {e}"))
    });
    result.as_ref().map_err(Clone::clone)
}

/// Extracts planar stereo `(left, right)` from an interleaved `AudioClip` and resamples to 44,100 Hz.
pub fn audio_clip_to_stereo_44100(clip: &AudioClip) -> (Vec<f32>, Vec<f32>) {
    let channels = clip.channels.max(1);
    let frames = clip.samples.len() / channels;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    if channels == 1 {
        for &s in &clip.samples[..frames] {
            left.push(s);
            right.push(s);
        }
    } else {
        for frame in clip.samples.chunks_exact(channels) {
            left.push(frame[0]);
            right.push(frame[1]);
        }
    }
    if clip.sample_rate == TARGET_SAMPLE_RATE || frames == 0 {
        return (left, right);
    }
    resample_stereo_linear(&left, &right, clip.sample_rate, TARGET_SAMPLE_RATE)
}

/// Linearly resamples planar stereo audio from `src_rate` to `dst_rate`.
pub fn resample_stereo_linear(
    left: &[f32],
    right: &[f32],
    src_rate: u32,
    dst_rate: u32,
) -> (Vec<f32>, Vec<f32>) {
    let src_len = left.len().min(right.len());
    if src_len == 0 || src_rate == 0 || dst_rate == 0 {
        return (Vec::new(), Vec::new());
    }
    if src_rate == dst_rate {
        return (left[..src_len].to_vec(), right[..src_len].to_vec());
    }
    let dst_len = ((src_len as f64) * (dst_rate as f64) / (src_rate as f64)).round() as usize;
    let dst_len = dst_len.max(1);
    let ratio = (src_rate as f64) / (dst_rate as f64);
    let mut out_l = Vec::with_capacity(dst_len);
    let mut out_r = Vec::with_capacity(dst_len);
    for i in 0..dst_len {
        let pos = (i as f64) * ratio;
        let idx = (pos.floor() as usize).min(src_len - 1);
        let next = (idx + 1).min(src_len - 1);
        let frac = (pos - idx as f64) as f32;
        out_l.push(left[idx] + (left[next] - left[idx]) * frac);
        out_r.push(right[idx] + (right[next] - right[idx]) * frac);
    }
    (out_l, out_r)
}

/// Computes the chunk start offsets for a stereo buffer of length `total_frames`.
pub fn chunk_starts(total_frames: usize, chunk_len: usize, hop: usize) -> Vec<usize> {
    if total_frames == 0 || chunk_len == 0 || hop == 0 {
        return Vec::new();
    }
    if total_frames <= chunk_len {
        return vec![0];
    }
    let mut starts = Vec::new();
    let mut start = 0usize;
    while start + chunk_len < total_frames {
        starts.push(start);
        start += hop;
    }
    // Always include a final chunk covering the tail of the song.
    let tail_start = total_frames.saturating_sub(chunk_len);
    if starts.last().copied() != Some(tail_start) {
        starts.push(tail_start);
    }
    starts
}

/// Processes planar stereo audio in overlapping `chunk_len`-sample windows with smooth
/// linear/triangular crossfade overlap-add so chunk boundaries are seamless.
pub fn separate_stereo_chunks<F>(
    left: &[f32],
    right: &[f32],
    chunk_len: usize,
    hop: usize,
    cancel: &AtomicBool,
    mut process_chunk: F,
) -> Result<(Vec<f32>, Vec<f32>), String>
where
    F: FnMut(&[f32], &[f32], usize, usize) -> Result<(Vec<f32>, Vec<f32>), String>,
{
    let total_frames = left.len().min(right.len());
    if total_frames == 0 {
        return Ok((Vec::new(), Vec::new()));
    }
    let starts = chunk_starts(total_frames, chunk_len, hop);
    let total_chunks = starts.len();
    let overlap = chunk_len.saturating_sub(hop);

    let mut acc_l = vec![0.0f64; total_frames];
    let mut acc_r = vec![0.0f64; total_frames];
    let mut weight_sum = vec![0.0f64; total_frames];

    let mut in_l = vec![0.0f32; chunk_len];
    let mut in_r = vec![0.0f32; chunk_len];

    for (chunk_idx, &start) in starts.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Err("Cancelled".into());
        }

        let copy_len = (total_frames - start).min(chunk_len);
        in_l[..copy_len].copy_from_slice(&left[start..start + copy_len]);
        in_r[..copy_len].copy_from_slice(&right[start..start + copy_len]);
        if copy_len < chunk_len {
            in_l[copy_len..].fill(0.0);
            in_r[copy_len..].fill(0.0);
        }

        let (out_l, out_r) = process_chunk(&in_l, &in_r, chunk_idx + 1, total_chunks)?;
        if out_l.len() < copy_len || out_r.len() < copy_len {
            return Err("Stem model returned fewer samples than expected.".into());
        }

        let fade_in = if chunk_idx > 0 && overlap > 0 {
            overlap.min(copy_len)
        } else {
            0
        };
        let fade_out = if chunk_idx + 1 < total_chunks && overlap > 0 {
            overlap.min(copy_len)
        } else {
            0
        };

        for i in 0..copy_len {
            let mut w = 1.0f64;
            if fade_in > 0 && i < fade_in {
                w = w.min((i as f64 + 0.5) / (fade_in as f64));
            }
            if fade_out > 0 && i + fade_out >= copy_len {
                let rem = copy_len - i;
                w = w.min((rem as f64 - 0.5).max(0.0) / (fade_out as f64));
            }
            let pos = start + i;
            acc_l[pos] += (out_l[i] as f64) * w;
            acc_r[pos] += (out_r[i] as f64) * w;
            weight_sum[pos] += w;
        }
    }

    let mut result_l = Vec::with_capacity(total_frames);
    let mut result_r = Vec::with_capacity(total_frames);
    for i in 0..total_frames {
        let w = weight_sum[i];
        if w > 0.0 {
            result_l.push((acc_l[i] / w) as f32);
            result_r.push((acc_r[i] / w) as f32);
        } else {
            result_l.push(0.0);
            result_r.push(0.0);
        }
    }
    Ok((result_l, result_r))
}

/// Runs HT-Demucs ONNX on CPU over planar 44.1 kHz stereo audio and returns the drums stem (index 0).
fn run_htdemucs_onnx(
    runtime_lib: &Path,
    model_path: &Path,
    left: &[f32],
    right: &[f32],
    cancel: &AtomicBool,
    mut on_chunk: impl FnMut(usize, usize),
) -> Result<(Vec<f32>, Vec<f32>), String> {
    let env = ort_environment(runtime_lib)?;
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 8))
        .unwrap_or(4);

    let mut session = ort::session::Session::builder(env)
        .map_err(|e| format!("Could not create ONNX session builder: {e}"))?
        .with_intra_threads(num_threads)
        .map_err(|e| format!("Could not configure ONNX CPU threads: {e}"))?
        .commit_from_file(model_path)
        .map_err(|e| format!("Could not load HT-Demucs ONNX model: {e}"))?;

    let input_name = session
        .inputs()
        .first()
        .map(|i| i.name().to_string())
        .ok_or_else(|| "ONNX model has no inputs.".to_string())?;
    let output_name = session
        .outputs()
        .first()
        .map(|o| o.name().to_string())
        .ok_or_else(|| "ONNX model has no outputs.".to_string())?;

    separate_stereo_chunks(
        left,
        right,
        CHUNK_SAMPLES,
        HOP_SAMPLES,
        cancel,
        |chunk_l, chunk_r, chunk_num, total_chunks| {
            on_chunk(chunk_num, total_chunks);
            let mut input_buf = Vec::with_capacity(2 * CHUNK_SAMPLES);
            input_buf.extend_from_slice(chunk_l);
            input_buf.extend_from_slice(chunk_r);

            let input_tensor = ort::value::Tensor::<f32>::from_array((
                [1usize, 2usize, CHUNK_SAMPLES],
                input_buf,
            ))
            .map_err(|e| format!("Could not build ONNX input tensor: {e}"))?;

            let outputs = session
                .run(vec![(input_name.as_str(), input_tensor)])
                .map_err(|e| format!("ONNX inference failed on chunk {chunk_num}: {e}"))?;

            let out_value = outputs
                .get(output_name.as_str())
                .ok_or_else(|| "ONNX model returned no output tensor.".to_string())?;

            let (shape, data) = out_value
                .try_extract_tensor::<f32>()
                .map_err(|e| format!("Could not read ONNX output tensor: {e}"))?;

            // Expected HT-Demucs output: [1, 4, 2, 343980] (or [4, 2, 343980]), stem 0 = drums.
            let needed = 2 * CHUNK_SAMPLES;
            if data.len() < needed {
                return Err(format!(
                    "Unexpected ONNX output tensor shape {shape:?} (len {} < {needed}).",
                    data.len()
                ));
            }
            let drums_l = data[0..CHUNK_SAMPLES].to_vec();
            let drums_r = data[CHUNK_SAMPLES..2 * CHUNK_SAMPLES].to_vec();
            Ok((drums_l, drums_r))
        },
    )
}

/// Atomically writes a 16-bit PCM stereo WAV file at `sample_rate` Hz.
pub fn write_stereo_wav_atomic(
    path: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<(), String> {
    let frames = left.len().min(right.len());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create stems cache folder: {e}"))?;
    }

    let tmp = unique_temp_path(path);
    let write_res = (|| -> Result<(), String> {
        let mut out = BufWriter::new(
            File::create(&tmp).map_err(|e| format!("Could not create temp stem WAV: {e}"))?,
        );
        let channels: u16 = 2;
        let bits_per_sample: u16 = 16;
        let block_align = channels * (bits_per_sample / 8);
        let byte_rate = sample_rate * (block_align as u32);
        let data_bytes = (frames as u64)
            .checked_mul(block_align as u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| "Separated stem is too large for WAV format.".to_string())?;
        let riff_size = 36u32
            .checked_add(data_bytes)
            .ok_or_else(|| "Separated stem exceeds WAV size limit.".to_string())?;

        out.write_all(b"RIFF")
            .and_then(|_| out.write_all(&riff_size.to_le_bytes()))
            .and_then(|_| out.write_all(b"WAVE"))
            .and_then(|_| out.write_all(b"fmt "))
            .and_then(|_| out.write_all(&16u32.to_le_bytes()))
            .and_then(|_| out.write_all(&1u16.to_le_bytes())) // PCM
            .and_then(|_| out.write_all(&channels.to_le_bytes()))
            .and_then(|_| out.write_all(&sample_rate.to_le_bytes()))
            .and_then(|_| out.write_all(&byte_rate.to_le_bytes()))
            .and_then(|_| out.write_all(&block_align.to_le_bytes()))
            .and_then(|_| out.write_all(&bits_per_sample.to_le_bytes()))
            .and_then(|_| out.write_all(b"data"))
            .and_then(|_| out.write_all(&data_bytes.to_le_bytes()))
            .map_err(|e| format!("Could not write WAV header: {e}"))?;

        for i in 0..frames {
            let l = (left[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            let r = (right[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            out.write_all(&l.to_le_bytes())
                .and_then(|_| out.write_all(&r.to_le_bytes()))
                .map_err(|e| format!("Could not write WAV samples: {e}"))?;
        }
        out.flush()
            .map_err(|e| format!("Could not flush WAV file: {e}"))?;
        drop(out);
        atomic_rename(&tmp, path).map_err(|e| format!("Could not finalize stem WAV: {e}"))?;
        Ok(())
    })();

    if write_res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_res
}

fn set_job_state(job: &SharedJob, state: StemSeparationJobState) {
    if let Ok(mut guard) = job.state.lock() {
        *guard = state;
    }
}

impl Application {
    fn open_song_path(&self) -> Option<PathBuf> {
        let document = self.document.as_ref()?;
        let folder = document.path.parent()?;
        let name = document.file.general("AudioFilename")?;
        crate::safe_child(folder, &name).ok()
    }

    fn cached_stem_for_open_song(&self) -> Option<PathBuf> {
        let song_path = self.open_song_path()?;
        let hash = file_sha256_hex(&song_path).ok()?;
        let cached = stem_cache_path(&self.data_root, &hash);
        if cached.is_file() { Some(cached) } else { None }
    }

    fn build_stem_separation_status(&self) -> StemSeparationStatus {
        let model_path = default_model_path(&self.data_root);
        let model_meta = fs::metadata(&model_path).ok();
        // Size only: the full SHA-256 check runs before the model is used, not on every poll.
        let model_ready = model_meta.as_ref().is_some_and(|m| m.is_file() && m.len() == DEFAULT_MODEL_SIZE);
        let model_size_bytes = model_meta.filter(|m| m.is_file()).map(|m| m.len());
        let runtime_ready = find_onnxruntime_library(&self.data_root, false).is_some();
        let song_path = self.open_song_path();
        let song_name = song_path
            .as_deref()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned());
        let cached_stem_path = self
            .cached_stem_for_open_song()
            .map(|p| p.to_string_lossy().into_owned());

        StemSeparationStatus {
            job: self.stem_separation.current_state(),
            model_ready,
            runtime_ready,
            model_size_bytes,
            cached_stem_path,
            song_loaded: self.song.is_some(),
            song_name,
            active_stem_loaded: self.has_drum_stem(),
            active_stem_monitoring: self.is_monitoring_stem(),
        }
    }

    /// Returns the current stem separation status, and auto-loads the generated stem
    /// into the Song ↔ Stem monitor (`T`) the first time a completed job is observed.
    pub fn stem_separation_status(&mut self) -> StemSeparationStatus {
        let current_song = self.open_song_path();
        if let Some(stem_path) = self
            .stem_separation
            .take_unapplied_done(current_song.as_deref())
        {
            self.load_drum_stem(stem_path);
        }
        self.build_stem_separation_status()
    }

    /// Starts background on-demand drum stem separation for the currently open song.
    pub fn stem_separation_start(
        &mut self,
        options: StemSeparationStartOptions,
    ) -> Result<StemSeparationStatus, String> {
        if self.stem_separation.is_running() {
            return Ok(self.build_stem_separation_status());
        }
        let song_path = self
            .open_song_path()
            .ok_or_else(|| "Open a map with a song first.".to_string())?;
        let song_clip = self
            .song
            .clone()
            .ok_or_else(|| "Wait for the song audio to finish loading first.".to_string())?;

        let song_hash = file_sha256_hex(&song_path)?;
        let out_stem_path = stem_cache_path(&self.data_root, &song_hash);

        if !options.force_rerun && out_stem_path.is_file() {
            self.stem_separation.generation += 1;
            let generation = self.stem_separation.generation;
            self.stem_separation.applied_generation = generation;
            self.stem_separation.active = Some(Arc::new(SharedJob {
                state: Mutex::new(StemSeparationJobState::Done {
                    stem_path: out_stem_path.to_string_lossy().into_owned(),
                }),
                cancel: Arc::new(AtomicBool::new(false)),
                generation,
                song_path: Some(song_path),
            }));
            self.load_drum_stem(out_stem_path);
            return Ok(self.build_stem_separation_status());
        }

        let custom_model = options
            .custom_model_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        if let Some(ref custom) = custom_model
            && !custom.is_file()
        {
            return Err(format!("Model file not found: {}", custom.display()));
        }

        self.stem_separation.generation += 1;
        let generation = self.stem_separation.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Arc::new(SharedJob {
            state: Mutex::new(StemSeparationJobState::DownloadingRuntime),
            cancel: cancel.clone(),
            generation,
            song_path: Some(song_path),
        });
        self.stem_separation.active = Some(job.clone());

        let data_root = self.data_root.clone();
        std::thread::Builder::new()
            .name("dawsu-stem-separation".into())
            .spawn(move || {
                let run_result = (|| -> Result<PathBuf, String> {
                    // 1. Ensure ONNX Runtime dynamic library is present.
                    set_job_state(&job, StemSeparationJobState::DownloadingRuntime);
                    let runtime_lib = ensure_onnxruntime_library(&data_root, &cancel)?;
                    if cancel.load(Ordering::SeqCst) {
                        return Err("Cancelled".into());
                    }

                    // 2. Ensure HT-Demucs ONNX model is present.
                    let model_path = match custom_model {
                        Some(p) => p,
                        None => {
                            let default_path = default_model_path(&data_root);
                            // A cached model is used only when it is the pinned file; an older
                            // unchecked or damaged download is fetched again.
                            if !file_matches_sha256(&default_path, DEFAULT_MODEL_SHA256) {
                                set_job_state(
                                    &job,
                                    StemSeparationJobState::DownloadingModel {
                                        downloaded_bytes: 0,
                                        total_bytes: None,
                                    },
                                );
                                download_file_atomic(
                                    DEFAULT_MODEL_URL,
                                    &default_path,
                                    (DEFAULT_MODEL_SHA256, DEFAULT_MODEL_SIZE),
                                    &cancel,
                                    |downloaded_bytes, total_bytes| {
                                        set_job_state(
                                            &job,
                                            StemSeparationJobState::DownloadingModel {
                                                downloaded_bytes,
                                                total_bytes,
                                            },
                                        );
                                    },
                                )?;
                            }
                            default_path
                        }
                    };
                    if cancel.load(Ordering::SeqCst) {
                        return Err("Cancelled".into());
                    }

                    // 3. Resample to 44,100 Hz stereo and run chunked ONNX inference.
                    let (left, right) = audio_clip_to_stereo_44100(&song_clip);
                    let total_chunks = chunk_starts(left.len(), CHUNK_SAMPLES, HOP_SAMPLES).len();
                    set_job_state(
                        &job,
                        StemSeparationJobState::Separating {
                            chunk: 0,
                            total_chunks,
                        },
                    );

                    let (drums_l, drums_r) = run_htdemucs_onnx(
                        &runtime_lib,
                        &model_path,
                        &left,
                        &right,
                        &cancel,
                        |chunk, total| {
                            set_job_state(
                                &job,
                                StemSeparationJobState::Separating {
                                    chunk,
                                    total_chunks: total,
                                },
                            );
                        },
                    )?;
                    if cancel.load(Ordering::SeqCst) {
                        return Err("Cancelled".into());
                    }

                    // 4. Atomically write the separated drums WAV to app-data cache.
                    write_stereo_wav_atomic(&out_stem_path, &drums_l, &drums_r, TARGET_SAMPLE_RATE)?;
                    Ok(out_stem_path)
                })();

                match run_result {
                    Ok(path) => set_job_state(
                        &job,
                        StemSeparationJobState::Done {
                            stem_path: path.to_string_lossy().into_owned(),
                        },
                    ),
                    Err(_) if cancel.load(Ordering::SeqCst) => {
                        set_job_state(&job, StemSeparationJobState::Cancelled);
                    }
                    Err(message) => {
                        set_job_state(&job, StemSeparationJobState::Failed { message });
                    }
                }
            })
            .map_err(|e| format!("Could not start stem separation thread: {e}"))?;

        Ok(self.build_stem_separation_status())
    }

    /// Cancels any active stem separation or model download job.
    pub fn stem_separation_cancel(&mut self) -> StemSeparationStatus {
        self.stem_separation.cancel();
        self.build_stem_separation_status()
    }

    /// Deletes the downloaded ~166 MB HT-Demucs model file from `<app_data>/models/` to free disk space.
    pub fn stem_separation_delete_cache(&mut self) -> Result<StemSeparationStatus, String> {
        if self.stem_separation.is_running() {
            return Err("Cancel the running separation before removing the downloaded model.".into());
        }
        let model_path = default_model_path(&self.data_root);
        if model_path.is_file() {
            fs::remove_file(&model_path)
                .map_err(|e| format!("Could not remove downloaded model: {e}"))?;
        }
        Ok(self.build_stem_separation_status())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;

    #[test]
    fn overlap_add_identity_reconstructs_short_exact_and_multi_chunk_stereo() {
        let cancel = AtomicBool::new(false);

        // Empty input
        let (el, er) = separate_stereo_chunks(&[], &[], 1024, 768, &cancel, |l, r, _, _| {
            Ok((l.to_vec(), r.to_vec()))
        })
        .unwrap();
        assert!(el.is_empty() && er.is_empty());

        // Test short (< chunk), exact (= chunk), and multi-chunk (> 2.5 chunks)
        let chunk_len = 1000usize;
        let hop = 750usize;
        for total_len in [237usize, 1000, 1500, 2631] {
            let left: Vec<f32> = (0..total_len)
                .map(|i| ((i as f32) * 0.031).sin() * 0.8)
                .collect();
            let right: Vec<f32> = (0..total_len)
                .map(|i| ((i as f32) * 0.053).cos() * 0.7)
                .collect();

            let mut seen_chunks = Vec::new();
            let (out_l, out_r) =
                separate_stereo_chunks(&left, &right, chunk_len, hop, &cancel, |l, r, idx, tot| {
                    assert_eq!(l.len(), chunk_len);
                    assert_eq!(r.len(), chunk_len);
                    seen_chunks.push((idx, tot));
                    Ok((l.to_vec(), r.to_vec()))
                })
                .unwrap();

            assert_eq!(out_l.len(), total_len);
            assert_eq!(out_r.len(), total_len);
            assert!(!seen_chunks.is_empty());
            for i in 0..total_len {
                assert!(
                    (out_l[i] - left[i]).abs() < 1e-5,
                    "len={total_len} left mismatch at {i}: {} vs {}",
                    out_l[i],
                    left[i]
                );
                assert!(
                    (out_r[i] - right[i]).abs() < 1e-5,
                    "len={total_len} right mismatch at {i}: {} vs {}",
                    out_r[i],
                    right[i]
                );
            }
        }
    }

    #[test]
    fn resample_48khz_to_44100hz_length_and_bounds() {
        // 1 second of 48 kHz stereo sine wave
        let frames_48k = 48_000usize;
        let mut interleaved = Vec::with_capacity(frames_48k * 2);
        for i in 0..frames_48k {
            let t = i as f32 / 48_000.0;
            interleaved.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5);
            interleaved.push((2.0 * std::f32::consts::PI * 880.0 * t).cos() * 0.25);
        }
        let clip = AudioClip {
            samples: interleaved,
            channels: 2,
            sample_rate: 48_000,
        };
        let (l, r) = audio_clip_to_stereo_44100(&clip);
        assert_eq!(l.len(), 44_100);
        assert_eq!(r.len(), 44_100);
        assert!(l.iter().all(|s| s.is_finite() && s.abs() <= 0.501));
        assert!(r.iter().all(|s| s.is_finite() && s.abs() <= 0.251));

        // Fast path: 44.1 kHz stays exact length and values
        let clip_44 = AudioClip {
            samples: vec![0.25, -0.25, 0.5, -0.5],
            channels: 2,
            sample_rate: 44_100,
        };
        let (l44, r44) = audio_clip_to_stereo_44100(&clip_44);
        assert_eq!(l44, vec![0.25, 0.5]);
        assert_eq!(r44, vec![-0.25, -0.5]);
    }

    #[test]
    fn stem_cache_path_determinism_and_atomic_wav_roundtrip_without_touching_mapset() {
        let scratch = Scratch::new("stem-cache-roundtrip");
        let app_data = scratch.path("app_data");
        let mapset_dir = scratch.path("mapset");
        fs::create_dir_all(&mapset_dir).unwrap();
        let fake_song = mapset_dir.join("audio.mp3");
        fs::write(&fake_song, b"deterministic-song-bytes-for-sha256").unwrap();

        let h1 = file_sha256_hex(&fake_song).unwrap();
        let h2 = file_sha256_hex(&fake_song).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);

        let cache_wav = stem_cache_path(&app_data, &h1);
        assert!(cache_wav.starts_with(&app_data));
        assert!(!cache_wav.starts_with(&mapset_dir));

        let left: Vec<f32> = (0..4410).map(|i| (i as f32 / 50.0).sin() * 0.6).collect();
        let right: Vec<f32> = (0..4410).map(|i| (i as f32 / 70.0).cos() * 0.4).collect();
        write_stereo_wav_atomic(&cache_wav, &left, &right, TARGET_SAMPLE_RATE).unwrap();

        // Confirm WAV decodes back via dawsu_audio and mapset folder still only has audio.mp3.
        let decoded = dawsu_audio::decode_with_cancel(&cache_wav, &|| false).unwrap();
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.sample_rate, TARGET_SAMPLE_RATE);
        assert_eq!(decoded.samples.len(), 4410 * 2);
        assert!((decoded.samples[20] - left[10]).abs() < 1e-3);
        assert!((decoded.samples[21] - right[10]).abs() < 1e-3);

        let mapset_entries: Vec<_> = fs::read_dir(&mapset_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(mapset_entries, vec!["audio.mp3"]);
    }

    #[test]
    fn status_and_cancel_state_transitions() {
        let mut mgr = StemSeparationManager::default();
        assert_eq!(mgr.current_state(), StemSeparationJobState::Idle);
        assert!(!mgr.is_running());

        let cancel = Arc::new(AtomicBool::new(false));
        let job = Arc::new(SharedJob {
            state: Mutex::new(StemSeparationJobState::Separating {
                chunk: 2,
                total_chunks: 10,
            }),
            cancel: cancel.clone(),
            generation: 1,
            song_path: None,
        });
        mgr.generation = 1;
        mgr.active = Some(job);
        assert!(mgr.is_running());

        mgr.cancel();
        assert!(cancel.load(Ordering::SeqCst));
        assert_eq!(mgr.current_state(), StemSeparationJobState::Cancelled);
        assert!(!mgr.is_running());

        // Mid-chunk cancellation stops separate_stereo_chunks cleanly
        let err = separate_stereo_chunks(
            &vec![0.0; 3000],
            &vec![0.0; 3000],
            1000,
            750,
            &cancel,
            |l, r, _, _| Ok((l.to_vec(), r.to_vec())),
        )
        .unwrap_err();
        assert_eq!(err, "Cancelled");
    }

    fn write_package(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    fn sha256_of(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    #[test]
    fn file_matches_sha256_only_for_the_exact_bytes() {
        let scratch = Scratch::new("stem-sha256-match");
        let file = scratch.path("model.onnx");
        fs::write(&file, b"pinned bytes").unwrap();
        assert!(file_matches_sha256(&file, &sha256_of(b"pinned bytes")));
        assert!(!file_matches_sha256(&file, &sha256_of(b"other bytes")));
        assert!(!file_matches_sha256(&scratch.path("missing.onnx"), &sha256_of(b"pinned bytes")));
    }

    #[test]
    fn package_extraction_installs_only_the_named_entry_with_the_pinned_hash() {
        let scratch = Scratch::new("stem-runtime-extract");
        let models = scratch.path("models");
        fs::create_dir_all(&models).unwrap();
        let package = models.join("onnxruntime.nupkg");
        let wanted: &[u8] = b"the x64 library";
        write_package(
            &package,
            &[
                ("runtimes/win-arm64/native/onnxruntime.dll", b"the arm64 library"),
                ("runtimes/win-x64/native/onnxruntime.dll", wanted),
            ],
        );
        let dest = models.join("onnxruntime.dll");

        // The exact entry is taken, not the first file with a matching name.
        extract_runtime_from_package(&package, "runtimes/win-x64/native/onnxruntime.dll", &sha256_of(wanted), &dest)
            .unwrap();
        assert_eq!(fs::read(&dest).unwrap(), wanted);

        // A library whose hash differs is refused and nothing new is installed.
        fs::remove_file(&dest).unwrap();
        let err = extract_runtime_from_package(
            &package,
            "runtimes/win-arm64/native/onnxruntime.dll",
            &sha256_of(wanted),
            &dest,
        )
        .unwrap_err();
        assert!(err.contains("not the one dawsu! expects"), "{err}");
        assert!(!dest.exists());
        let leftovers: Vec<_> = fs::read_dir(&models)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");

        // A missing entry is an error, not a silent success.
        assert!(extract_runtime_from_package(&package, "runtimes/osx-arm64/native/libonnxruntime.dylib", &sha256_of(wanted), &dest).is_err());
    }

    #[test]
    fn a_runtime_library_with_the_wrong_bytes_in_app_data_is_not_loaded() {
        if std::env::var_os("ORT_DYLIB_PATH").is_some() || onnxruntime_package_entry().is_err() {
            return; // An explicit user library, or no pinned build for this platform.
        }
        let scratch = Scratch::new("stem-runtime-find");
        let data_root = scratch.path("app_data");
        fs::create_dir_all(models_dir(&data_root)).unwrap();
        let (_, _, size) = onnxruntime_package_entry().unwrap();
        // Same size as the pinned library, different bytes: the size check passes, the hash fails.
        fs::write(models_dir(&data_root).join(onnxruntime_lib_filename()), vec![0u8; size as usize]).unwrap();
        assert!(find_onnxruntime_library(&data_root, true).is_none());
    }

    #[test]
    fn pinned_download_constants_are_well_formed() {
        for hash in [DEFAULT_MODEL_SHA256, ORT_PACKAGE_SHA256] {
            assert_eq!(hash.len(), 64);
            assert!(hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        }
        assert!(DEFAULT_MODEL_URL.starts_with("https://") && !DEFAULT_MODEL_URL.contains("/resolve/main/"));
        assert!(ORT_PACKAGE_URL.starts_with("https://api.nuget.org/") && ORT_PACKAGE_URL.contains(ORT_RELEASE_VERSION));
    }
}
