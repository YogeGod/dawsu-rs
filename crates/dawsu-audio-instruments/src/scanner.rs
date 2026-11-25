//! Cross-platform VST3 folder discovery and `--scan` output parsing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Metadata for a single VST3 plug-in class reported by `dawsu_vst3_host --scan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vst3PluginInfo {
    pub path: PathBuf,
    pub uid: String,
    pub name: String,
    pub vendor: String,
    pub is_instrument: bool,
    pub is_effect: bool,
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
        vec![common.join("VST3")]
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

/// Invokes `host_executable --scan <plugin_path>` and parses the reported classes,
/// enforcing a 10-second timeout so a hanging plug-in cannot wedge the editor.
pub fn scan_plugin(host_executable: &Path, plugin_path: &Path) -> Result<Vec<Vst3PluginInfo>, ScanError> {
    use std::io::Read;
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

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ScanError::HostReported(
                        "Plugin scan timed out after 10 seconds.".to_string(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ScanError::SpawnFailed(err.to_string()));
            }
        }
    };

    let mut stdout_bytes = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut stdout_bytes);
    }
    let mut stderr_bytes = Vec::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_end(&mut stderr_bytes);
    }

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
}
