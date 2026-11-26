//! Cross-platform VST3 folder discovery, verified `moduleinfo.json` fast pre-scan,
//! concurrent `--scan` pipe draining, canonical cross-OS TUID helpers, and
//! inner-binary crash blacklist persistence.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

const MAX_PIPE_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
const MAX_MODULEINFO_BYTES: u64 = 4 * 1024 * 1024;

/// Metadata for a single VST3 plug-in class reported by `dawsu_vst3_host --scan`
/// or verified `moduleinfo.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vst3PluginInfo {
    pub path: PathBuf,
    pub uid: String,
    pub name: String,
    pub vendor: String,
    pub is_instrument: bool,
    pub is_effect: bool,
}

/// Persistent blacklist entry keyed on the inner platform binary's canonical path, size,
/// and live `fs::metadata(&binary_path).modified()` timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlacklistEntry {
    pub plugin_path: PathBuf,
    pub binary_path: PathBuf,
    pub binary_size: u64,
    pub modified_nanos: u128,
    pub reason: String,
}

/// Persistent VST3 crash/timeout blacklist (`vst3-blacklist.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vst3Blacklist {
    pub version: u32,
    pub entries: Vec<BlacklistEntry>,
}

impl Default for Vst3Blacklist {
    fn default() -> Self {
        Self {
            version: 1,
            entries: Vec::new(),
        }
    }
}

impl Vst3Blacklist {
    /// Loads a persisted blacklist from `path`, returning an empty list if missing or invalid.
    pub fn load(path: &Path) -> Self {
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        serde_json::from_slice::<Self>(&bytes)
            .ok()
            .filter(|b| b.version == 1)
            .unwrap_or_default()
    }

    /// Saves the blacklist atomically to `path`.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        dawsu_foundation::safe_write::write_new(path, &bytes).map_err(|e| e.to_string())
    }

    /// Returns the matching blacklist entry when `plugin_path`'s current inner binary matches
    /// the recorded canonical path, byte size, and live modification timestamp.
    pub fn is_blocked(&self, plugin_path: &Path) -> Option<&BlacklistEntry> {
        let (binary_path, size, mtime) = binary_fingerprint(plugin_path)?;
        self.entries.iter().find(|entry| {
            entry.binary_path == binary_path
                && entry.binary_size == size
                && entry.modified_nanos == mtime
        })
    }

    /// Records (or updates) a failure entry keyed on `plugin_path`'s inner binary.
    pub fn record_failure(&mut self, plugin_path: &Path, reason: String) {
        let Some((binary_path, binary_size, modified_nanos)) = binary_fingerprint(plugin_path)
        else {
            return;
        };
        self.entries.retain(|entry| entry.binary_path != binary_path);
        self.entries.push(BlacklistEntry {
            plugin_path: plugin_path.to_path_buf(),
            binary_path,
            binary_size,
            modified_nanos,
            reason,
        });
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Normalizes a 32-hex-digit UID string (stripping braces, hyphens, and whitespace)
/// into uppercase 32-character canonical hex.
pub fn canonicalize_vst3_uid(raw: &str) -> Option<String> {
    let hex: String = raw
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-' && *c != '{' && *c != '}')
        .collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_ascii_uppercase())
}

/// Formats 16 raw TUID bytes as an uppercase 32-character canonical hex string.
pub fn tuid_bytes_to_canonical_hex(bytes: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(32);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0F) as usize] as char);
    }
    out
}

fn hex_to_16_bytes(hex: &str) -> Option<[u8; 16]> {
    let clean = canonicalize_vst3_uid(hex)?;
    let mut bytes = [0u8; 16];
    for (i, byte) in bytes.iter_mut().enumerate() {
        let pair = &clean[i * 2..i * 2 + 2];
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(bytes)
}

/// Converts a legacy Windows COM-formatted `VST3::UID::toString(true)` hex string into
/// canonical raw-byte `VST3::UID::toString(false)` hex by byte-swapping the LE `GUID` header
/// fields (`Data1`, `Data2`, `Data3`).
pub fn com_guid_hex_to_canonical_hex(hex: &str) -> Option<String> {
    let mut bytes = hex_to_16_bytes(hex)?;
    bytes[0..4].reverse();
    bytes[4..6].reverse();
    bytes[6..8].reverse();
    Some(tuid_bytes_to_canonical_hex(&bytes))
}

/// Converts canonical cross-OS TUID hex to Windows COM `GUID` hex representation.
pub fn canonical_hex_to_com_guid_hex(hex: &str) -> Option<String> {
    com_guid_hex_to_canonical_hex(hex)
}

/// Compares two VST3 UIDs across canonical raw-byte and legacy Windows COM GUID orderings.
pub fn uids_equivalent(a: &str, b: &str) -> bool {
    let Some(ca) = canonicalize_vst3_uid(a) else {
        return a.trim().eq_ignore_ascii_case(b.trim());
    };
    let Some(cb) = canonicalize_vst3_uid(b) else {
        return false;
    };
    ca == cb || com_guid_hex_to_canonical_hex(&ca).as_deref() == Some(cb.as_str())
}

/// Failure while scanning a VST3 plug-in or parsing `--scan` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    SpawnFailed(String),
    HostExited { code: Option<i32>, stderr: String },
    HostReported(String),
    MalformedLine(String),
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpawnFailed(msg) => write!(f, "failed to launch VST3 host for scan: {msg}"),
            Self::HostExited { code, stderr } => {
                write!(f, "VST3 host exited with {:?}: {}", code, stderr.trim())
            }
            Self::HostReported(msg) => write!(f, "VST3 scan error: {msg}"),
            Self::MalformedLine(line) => write!(f, "malformed --scan output line: {line}"),
        }
    }
}

impl std::error::Error for ScanError {}

/// Standard system and user VST3 search directories for the current OS.
pub fn default_vst3_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let common = std::env::var_os("CommonProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Common Files"));
        let mut dirs = vec![common.join("VST3")];
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            let per_user = PathBuf::from(local_app_data)
                .join("Programs")
                .join("Common")
                .join("VST3");
            if !dirs.iter().any(|d| {
                d.to_string_lossy()
                    .eq_ignore_ascii_case(&per_user.to_string_lossy())
            }) {
                dirs.push(per_user);
            }
        }
        dirs
    }

    #[cfg(target_os = "macos")]
    {
        let mut dirs = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3"));
        }
        dirs
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut dirs = vec![
            PathBuf::from("/usr/lib/vst3"),
            PathBuf::from("/usr/local/lib/vst3"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".vst3"));
        }
        dirs
    }
}

/// Resolves the inner native binary file inside a `.vst3` bundle for the active OS/architecture
/// (or returns `plugin_path` directly when it is a flat `.vst3` binary file).
pub fn resolve_vst3_binary(plugin_path: &Path) -> Option<PathBuf> {
    if plugin_path.is_file() {
        return Some(plugin_path.to_path_buf());
    }
    if !plugin_path.is_dir() {
        return None;
    }

    let contents = plugin_path.join("Contents");
    let stem = plugin_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();

    #[cfg(target_os = "windows")]
    let arch_dirs: &[&str] = {
        #[cfg(target_arch = "x86_64")]
        {
            &["x86_64-win"]
        }
        #[cfg(target_arch = "aarch64")]
        {
            &["arm64-win", "arm64ec-win"]
        }
        #[cfg(target_arch = "x86")]
        {
            &["x86-win"]
        }
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "x86")))]
        {
            &[]
        }
    };

    #[cfg(target_os = "macos")]
    {
        let mac_dir = contents.join("MacOS");
        if mac_dir.is_dir() {
            if !stem.is_empty() {
                let exact = mac_dir.join(stem);
                if exact.is_file() {
                    return Some(exact);
                }
            }
            if let Ok(entries) = fs::read_dir(&mac_dir) {
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|ft| ft.is_file()) {
                        return Some(entry.path());
                    }
                }
            }
        }
        None
    }

    #[cfg(target_os = "windows")]
    {
        for sub in arch_dirs {
            let dir = contents.join(sub);
            if !dir.is_dir() {
                continue;
            }
            if !stem.is_empty() {
                let exact = dir.join(format!("{stem}.vst3"));
                if exact.is_file() {
                    return Some(exact);
                }
            }
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if entry.file_type().is_ok_and(|ft| ft.is_file())
                        && path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("vst3"))
                    {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let arch = match std::env::consts::ARCH {
            "x86_64" => "x86_64-linux",
            "aarch64" => "aarch64-linux",
            "x86" => "i386-linux",
            other => other,
        };
        let dir = contents.join(arch);
        if dir.is_dir() {
            if !stem.is_empty() {
                let exact = dir.join(format!("{stem}.so"));
                if exact.is_file() {
                    return Some(exact);
                }
            }
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if entry.file_type().is_ok_and(|ft| ft.is_file())
                        && path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("so"))
                    {
                        return Some(path);
                    }
                }
            }
        }
        None
    }
}

/// Computes `(canonical_binary_path, file_size_bytes, live_modified_nanos)` for the inner
/// native binary inside `plugin_path`. Uses `fs::metadata(&path)` (never stale `DirEntry`).
pub fn binary_fingerprint(plugin_path: &Path) -> Option<(PathBuf, u64, u128)> {
    let binary_path = resolve_vst3_binary(plugin_path)?;
    let meta = fs::metadata(&binary_path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let canonical = fs::canonicalize(&binary_path).unwrap_or(binary_path);
    let modified_nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    Some((canonical, meta.len(), modified_nanos))
}

/// Fast, zero-process metadata pre-scan using Steinberg's `moduleinfo.json`.
///
/// Always verifies that the current platform/architecture binary exists inside `plugin_path`
/// before trusting `moduleinfo.json`, inherits `Factory Info.Vendor` for classes without an
/// explicit `Vendor`, and filters to `Audio Module Class` entries with `Instrument` or `Fx`.
pub fn try_scan_moduleinfo(plugin_path: &Path) -> Option<Vec<Vst3PluginInfo>> {
    resolve_vst3_binary(plugin_path)?;
    let candidates = [
        plugin_path
            .join("Contents")
            .join("Resources")
            .join("moduleinfo.json"),
        plugin_path.join("Contents").join("moduleinfo.json"),
    ];
    let info_path = candidates.into_iter().find(|p| p.is_file())?;
    let meta = fs::metadata(&info_path).ok()?;
    if meta.len() > MAX_MODULEINFO_BYTES {
        return None;
    }
    let raw = fs::read_to_string(&info_path).ok()?;
    let root: serde_json::Value = serde_json::from_str(&raw).ok()?;

    let factory_vendor = root
        .get("Factory Info")
        .and_then(|f| f.get("Vendor"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let classes = root.get("Classes")?.as_array()?;

    let mut plugins = Vec::new();
    for class in classes {
        let obj = class.as_object()?;
        let category = obj.get("Category").and_then(|v| v.as_str()).unwrap_or("");
        if category != "Audio Module Class" {
            continue;
        }
        let raw_cid = obj.get("CID").and_then(|v| v.as_str())?;
        let uid = canonicalize_vst3_uid(raw_cid)?;
        let name = obj.get("Name").and_then(|v| v.as_str())?.trim();
        if name.is_empty() {
            return None;
        }
        let vendor = obj
            .get("Vendor")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(factory_vendor);

        let (is_instrument, is_effect) = match obj.get("Sub Categories") {
            Some(serde_json::Value::Array(items)) => {
                let mut inst = false;
                let mut fx = false;
                for item in items {
                    if let Some(tok) = item.as_str() {
                        if tok.trim().eq_ignore_ascii_case("Instrument") {
                            inst = true;
                        }
                        if tok.trim().eq_ignore_ascii_case("Fx") {
                            fx = true;
                        }
                    }
                }
                (inst, fx)
            }
            Some(serde_json::Value::String(text)) => (
                has_subcategory(text, "Instrument"),
                has_subcategory(text, "Fx"),
            ),
            _ => (false, false),
        };
        if !is_instrument && !is_effect {
            continue;
        }
        plugins.push(Vst3PluginInfo {
            path: plugin_path.to_path_buf(),
            uid,
            name: name.to_string(),
            vendor: vendor.to_string(),
            is_instrument,
            is_effect,
        });
    }

    Some(plugins)
}

/// Lightweight entry for an installed `.vst3` bundle/file discovered directly from disk
/// without spawning the sidecar host (mirrors v3.1.4's `Vst3Picker.Installed()`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledVst3 {
    pub label: String,
    pub path: PathBuf,
}

/// Lists installed `.vst3` plug-ins under the platform's default VST3 folders without launching
/// any plug-in binaries. Disambiguates duplicate filenames with their relative path.
pub fn list_installed_vst3() -> Vec<InstalledVst3> {
    list_installed_vst3_in(&default_vst3_dirs())
}

/// Lists installed `.vst3` plug-ins under `roots` without launching any plug-in binaries.
pub fn list_installed_vst3_in(roots: &[PathBuf]) -> Vec<InstalledVst3> {
    let paths = discover_vst3_paths_in(roots);
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let entries: Vec<(String, PathBuf)> = paths
        .into_iter()
        .map(|path| {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("VST3")
                .to_string();
            *counts.entry(stem.to_lowercase()).or_insert(0) += 1;
            (stem, path)
        })
        .collect();

    let mut result: Vec<InstalledVst3> = entries
        .into_iter()
        .map(|(stem, path)| {
            let label = if counts.get(&stem.to_lowercase()).copied().unwrap_or(0) > 1 {
                let rel = roots
                    .iter()
                    .find_map(|root| path.strip_prefix(root).ok())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                format!("{stem} — {rel}")
            } else {
                stem
            };
            InstalledVst3 { label, path }
        })
        .collect();

    result.sort_by(|a, b| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    result
}

/// Discovers `.vst3` plug-in paths across the platform's default VST3 folders.
pub fn discover_vst3_paths() -> Vec<PathBuf> {
    discover_vst3_paths_in(&default_vst3_dirs())
}

/// Discovers `.vst3` bundles and flat `.vst3` files under `roots`.
///
/// Once a directory entry has a `.vst3` extension (case-insensitive), it is recorded
/// as a plug-in and never recursed into, so nested binaries such as
/// `Plugin.vst3/Contents/x86_64-win/Plugin.vst3` are skipped.
pub fn discover_vst3_paths_in(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = BTreeSet::new();
    for root in roots {
        if root.is_dir() {
            walk_dir(root, &mut found);
        }
    }
    found.into_iter().collect()
}

fn walk_dir(dir: &Path, found: &mut BTreeSet<PathBuf>) {
    walk_dir_depth(dir, found, 16);
}

fn walk_dir_depth(dir: &Path, found: &mut BTreeSet<PathBuf>, depth: usize) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_vst3_path(&path) {
            found.insert(path);
            // Never recurse inside a .vst3 bundle directory.
            continue;
        }
        if entry.file_type().is_ok_and(|ft| ft.is_dir() && !ft.is_symlink()) {
            walk_dir_depth(&path, found, depth - 1);
        }
    }
}

fn is_vst3_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("vst3"))
}

/// Parses a single stdout line from `dawsu_vst3_host --scan`.
///
/// Expected class format: `CLASS\t<uid>\t<name>\t<vendor>\t<subcategories>`
pub fn parse_scan_line(plugin_path: &Path, raw_line: &str) -> Result<Option<Vst3PluginInfo>, ScanError> {
    let line = raw_line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Ok(None);
    }
    if let Some(msg) = line.strip_prefix("ERROR\t") {
        return Err(ScanError::HostReported(msg.to_string()));
    }
    let Some(rest) = line.strip_prefix("CLASS\t") else {
        return Ok(None);
    };
    let parts: Vec<&str> = rest.split('\t').collect();
    if parts.len() != 4 {
        return Err(ScanError::MalformedLine(line.to_string()));
    }
    let uid = parts[0].trim();
    let name = parts[1].trim();
    let vendor = parts[2].trim();
    let subcategories = parts[3].trim();
    if uid.is_empty() || name.is_empty() {
        return Err(ScanError::MalformedLine(line.to_string()));
    }

    let is_instrument = has_subcategory(subcategories, "Instrument");
    let is_effect = has_subcategory(subcategories, "Fx");

    Ok(Some(Vst3PluginInfo {
        path: plugin_path.to_path_buf(),
        uid: uid.to_string(),
        name: name.to_string(),
        vendor: vendor.to_string(),
        is_instrument,
        is_effect,
    }))
}

fn has_subcategory(subcategories: &str, needle: &str) -> bool {
    subcategories
        .split('|')
        .any(|token| token.trim().eq_ignore_ascii_case(needle))
}

/// Parses the full stdout of `dawsu_vst3_host --scan <plugin.vst3>`.
pub fn parse_scan_output(plugin_path: &Path, stdout: &str) -> Result<Vec<Vst3PluginInfo>, ScanError> {
    let mut plugins = Vec::new();
    for line in stdout.lines() {
        if let Some(info) = parse_scan_line(plugin_path, line)? {
            plugins.push(info);
        }
    }
    Ok(plugins)
}

fn read_capped_pipe<R: Read>(mut reader: R) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() < MAX_PIPE_CAPTURE_BYTES {
                    let take = n.min(MAX_PIPE_CAPTURE_BYTES - out.len());
                    out.extend_from_slice(&buf[..take]);
                }
            }
        }
    }
    out
}

/// Invokes `host_executable --scan <plugin_path>` and parses the reported classes,
/// draining stdout/stderr concurrently on reader threads so verbose plug-ins never deadlock
/// OS pipe buffers, and enforcing a 10-second timeout.
pub fn scan_plugin(host_executable: &Path, plugin_path: &Path) -> Result<Vec<Vst3PluginInfo>, ScanError> {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let mut child = Command::new(host_executable)
        .arg("--scan")
        .arg(plugin_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| ScanError::SpawnFailed(err.to_string()))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_thread = std::thread::spawn(move || stdout_pipe.map(read_capped_pipe).unwrap_or_default());
    let stderr_thread = std::thread::spawn(move || stderr_pipe.map(read_capped_pipe).unwrap_or_default());

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_thread.join();
                    let _ = stderr_thread.join();
                    return Err(ScanError::HostReported(
                        "Plugin scan timed out after 10 seconds.".to_string(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(ScanError::SpawnFailed(err.to_string()));
            }
        }
    };

    let stdout_bytes = stdout_thread.join().unwrap_or_default();
    let stderr_bytes = stderr_thread.join().unwrap_or_default();

    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let parsed = parse_scan_output(plugin_path, &stdout)?;
    if !status.success() {
        return Err(ScanError::HostExited {
            code: status.code(),
            stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        });
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("dawsu-vst3-scan-{label}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn skips_nested_binaries_inside_vst3_bundles() {
        let root = make_temp_dir("nested");

        // Bundle with nested .vst3 binary inside Contents/x86_64-win/
        let bundle = root.join("Vital.vst3");
        let nested_dir = bundle.join("Contents").join("x86_64-win");
        fs::create_dir_all(&nested_dir).unwrap();
        fs::write(nested_dir.join("Vital.vst3"), b"binary").unwrap();

        // Vendor subfolder with an effect bundle and a flat .VST3 file
        let vendor_dir = root.join("FabFilter");
        let fx_bundle = vendor_dir.join("Pro-Q 3.vst3");
        fs::create_dir_all(fx_bundle.join("Contents").join("x86_64-win")).unwrap();
        fs::write(
            fx_bundle.join("Contents").join("x86_64-win").join("Pro-Q 3.vst3"),
            b"binary",
        )
        .unwrap();
        let flat_plugin = vendor_dir.join("LegacySynth.VST3");
        fs::write(&flat_plugin, b"flat-dll").unwrap();

        // Unrelated file
        fs::write(vendor_dir.join("readme.txt"), b"ignore").unwrap();

        let found = discover_vst3_paths_in(std::slice::from_ref(&root));
        let _ = fs::remove_dir_all(&root);

        assert_eq!(found.len(), 3, "expected 3 top-level plugins, got {found:?}");
        assert!(found.contains(&bundle));
        assert!(found.contains(&fx_bundle));
        assert!(found.contains(&flat_plugin));
        assert!(
            !found.iter().any(|p| p.to_string_lossy().contains("Contents")),
            "nested bundle binary should be skipped: {found:?}"
        );
    }

    #[test]
    fn parses_instrument_and_fx_scan_lines() {
        let path = Path::new("/plugins/Multi.vst3");
        let stdout = "\
CLASS\t0123456789ABCDEF0123456789ABCDEF\tVital\tVital Audio\tInstrument|Synth\r\n\
CLASS\tFEDCBA9876543210FEDCBA9876543210\tValhallaRoom\tValhalla DSP\tFx|Reverb\n\
CLASS\tAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1\tHybridBox\tAcme\tInstrument|Fx|Sampler\n";

        let plugins = parse_scan_output(path, stdout).unwrap();
        assert_eq!(plugins.len(), 3);

        assert_eq!(plugins[0].name, "Vital");
        assert_eq!(plugins[0].vendor, "Vital Audio");
        assert!(plugins[0].is_instrument);
        assert!(!plugins[0].is_effect);

        assert_eq!(plugins[1].name, "ValhallaRoom");
        assert!(!plugins[1].is_instrument);
        assert!(plugins[1].is_effect);

        assert_eq!(plugins[2].name, "HybridBox");
        assert!(plugins[2].is_instrument);
        assert!(plugins[2].is_effect);

        // Verify JSON serialization round-trip
        let json = serde_json::to_string(&plugins).unwrap();
        let decoded: Vec<Vst3PluginInfo> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, plugins);
    }

    #[test]
    fn reports_error_and_malformed_scan_lines() {
        let path = Path::new("/plugins/Broken.vst3");
        let err = parse_scan_output(path, "ERROR\tFailed to load module\n").unwrap_err();
        assert_eq!(
            err,
            ScanError::HostReported("Failed to load module".to_string())
        );

        let err = parse_scan_line(path, "CLASS\tonly-two-fields").unwrap_err();
        assert!(matches!(err, ScanError::MalformedLine(_)));
    }

    #[test]
    fn lists_installed_vst3s_sorted_and_disambiguates_duplicates() {
        let root = std::env::temp_dir().join(format!(
            "dawsu-vst3-installed-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("GGD Matt Garstka.vst3/Contents/x86_64-win")).unwrap();
        fs::write(
            root.join("GGD Matt Garstka.vst3/Contents/x86_64-win/GGD Matt Garstka.vst3"),
            b"bin",
        )
        .unwrap();
        fs::create_dir_all(root.join("VendorA")).unwrap();
        fs::create_dir_all(root.join("VendorB")).unwrap();
        fs::write(root.join("VendorA/Drums.vst3"), b"a").unwrap();
        fs::write(root.join("VendorB/Drums.vst3"), b"b").unwrap();

        let items = list_installed_vst3_in(std::slice::from_ref(&root));
        let _ = fs::remove_dir_all(&root);

        assert_eq!(items.len(), 3);
        assert!(items[0].label.starts_with("Drums — VendorA"));
        assert!(items[1].label.starts_with("Drums — VendorB"));
        assert_eq!(items[2].label, "GGD Matt Garstka");
    }

    #[test]
    fn moduleinfo_fast_scan_inherits_vendor_and_requires_inner_binary() {
        let root = make_temp_dir("moduleinfo");
        let bundle = root.join("Vital.vst3");
        let resources = bundle.join("Contents").join("Resources");
        fs::create_dir_all(&resources).unwrap();
        let moduleinfo = serde_json::json!({
            "Name": "Vital",
            "Version": "1.5.5",
            "Factory Info": {
                "Vendor": "Vital Audio",
                "URL": "https://vital.audio",
                "E-Mail": "support@vital.audio",
                "Flags": { "Unicode": true, "Classes Discardable": false, "Component Non Discardable": false }
            },
            "Classes": [
                {
                    "CID": "0123456789abcdef0123456789abcdef",
                    "Category": "Audio Module Class",
                    "Name": "Vital",
                    "Sub Categories": ["Instrument", "Synth"]
                },
                {
                    "CID": "FEDCBA9876543210FEDCBA9876543210",
                    "Category": "Component Controller Class",
                    "Name": "Vital Controller"
                },
                {
                    "CID": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1",
                    "Category": "Audio Module Class",
                    "Name": "VitalFX",
                    "Vendor": "OverrideVendor",
                    "Sub Categories": "Fx|Filter"
                }
            ]
        });
        fs::write(
            resources.join("moduleinfo.json"),
            serde_json::to_vec_pretty(&moduleinfo).unwrap(),
        )
        .unwrap();

        // Missing inner platform binary: must NEVER register from moduleinfo.json alone.
        assert!(try_scan_moduleinfo(&bundle).is_none());

        // Create matching inner binary for the active OS/architecture.
        #[cfg(target_os = "windows")]
        let inner = bundle
            .join("Contents")
            .join("x86_64-win")
            .join("Vital.vst3");
        #[cfg(target_os = "macos")]
        let inner = bundle.join("Contents").join("MacOS").join("Vital");
        #[cfg(all(unix, not(target_os = "macos")))]
        let inner = bundle
            .join("Contents")
            .join("x86_64-linux")
            .join("Vital.so");

        fs::create_dir_all(inner.parent().unwrap()).unwrap();
        fs::write(&inner, b"pe-binary").unwrap();

        let parsed = try_scan_moduleinfo(&bundle).unwrap();
        let _ = fs::remove_dir_all(&root);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "Vital");
        assert_eq!(parsed[0].vendor, "Vital Audio");
        assert_eq!(parsed[0].uid, "0123456789ABCDEF0123456789ABCDEF");
        assert!(parsed[0].is_instrument && !parsed[0].is_effect);
        assert_eq!(parsed[1].name, "VitalFX");
        assert_eq!(parsed[1].vendor, "OverrideVendor");
        assert!(parsed[1].is_effect && !parsed[1].is_instrument);
    }

    #[test]
    fn blacklist_keys_on_inner_binary_and_invalidates_on_update() {
        let root = make_temp_dir("blacklist");
        let bundle = root.join("CrashingSynth.vst3");
        #[cfg(target_os = "windows")]
        let inner = bundle
            .join("Contents")
            .join("x86_64-win")
            .join("CrashingSynth.vst3");
        #[cfg(target_os = "macos")]
        let inner = bundle.join("Contents").join("MacOS").join("CrashingSynth");
        #[cfg(all(unix, not(target_os = "macos")))]
        let inner = bundle
            .join("Contents")
            .join("x86_64-linux")
            .join("CrashingSynth.so");
        fs::create_dir_all(inner.parent().unwrap()).unwrap();
        fs::write(&inner, b"v1-binary").unwrap();

        let blacklist_path = root.join("state").join("vst3-blacklist.json");
        let mut bl = Vst3Blacklist::default();
        bl.record_failure(&bundle, "Scan timed out".to_string());
        bl.save(&blacklist_path).unwrap();

        let loaded = Vst3Blacklist::load(&blacklist_path);
        assert_eq!(loaded.entries.len(), 1);
        assert!(loaded.is_blocked(&bundle).is_some());

        // Sleep past Windows NTFS 15.6 ms mtime resolution and update inner binary.
        std::thread::sleep(std::time::Duration::from_millis(40));
        fs::write(&inner, b"v2-fixed-binary-larger").unwrap();

        assert!(
            loaded.is_blocked(&bundle).is_none(),
            "updating inner binary must invalidate stale blacklist entry"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tuid_com_and_canonical_round_trip_and_equivalence() {
        let raw = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
            0xFF, 0x00,
        ];
        let canonical = tuid_bytes_to_canonical_hex(&raw);
        assert_eq!(canonical, "112233445566778899AABBCCDDEEFF00");
        let com_hex = canonical_hex_to_com_guid_hex(&canonical).unwrap();
        assert_eq!(com_hex, "443322116655887799AABBCCDDEEFF00");
        assert_eq!(
            com_guid_hex_to_canonical_hex(&com_hex).unwrap(),
            canonical
        );
        assert!(uids_equivalent(&canonical, &com_hex));
        assert!(uids_equivalent("{44332211-6655-8877-99AA-BBCCDDEEFF00}", &canonical));
    }

    #[test]
    fn scan_plugin_drains_large_noisy_pipes_without_deadlock() {
        // Generate > 128 KiB across stdout + stderr (larger than OS 4-64 KiB pipe buffers)
        // plus one valid CLASS line on stdout.
        #[cfg(windows)]
        {
            let root = make_temp_dir("pipe-drain");
            let script = root.join("noisy_scan.cmd");
            let body = "@echo off\r\n\
                powershell -NoProfile -Command \"for ($i=0; $i -lt 300; $i++) { [Console]::Out.WriteLine(('NOISE ' * 40)); [Console]::Error.WriteLine(('ERR ' * 40)) }; [Console]::Out.WriteLine('CLASS\t0123456789ABCDEF0123456789ABCDEF\tNoisySynth\tAcme\tInstrument|Synth')\"\r\n";
            fs::write(&script, body).unwrap();
            let plugins = scan_plugin(&script, Path::new("Noisy.vst3")).unwrap();
            let _ = fs::remove_dir_all(&root);
            assert_eq!(plugins.len(), 1);
            assert_eq!(plugins[0].name, "NoisySynth");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let root = make_temp_dir("pipe-drain");
            let script = root.join("noisy_scan.sh");
            let body = "#!/bin/sh\n\
                i=0; while [ $i -lt 2000 ]; do echo 'NOISE_LINE_0123456789ABCDEF0123456789ABCDEF'; echo 'ERR_LINE_0123456789ABCDEF' >&2; i=$((i+1)); done\n\
                printf 'CLASS\\t0123456789ABCDEF0123456789ABCDEF\\tNoisySynth\\tAcme\\tInstrument|Synth\\n'\n";
            fs::write(&script, body).unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
            let plugins = scan_plugin(&script, Path::new("Noisy.vst3")).unwrap();
            let _ = fs::remove_dir_all(&root);
            assert_eq!(plugins.len(), 1);
            assert_eq!(plugins[0].name, "NoisySynth");
        }
    }
}
