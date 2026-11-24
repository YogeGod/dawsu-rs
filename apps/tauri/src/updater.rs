//! GitHub Releases update checker, SHA-256 verified downloader, and install-on-close staging.
//!
//! Network access happens only when the user explicitly clicks Check / Download in
//! Preferences › Updates — never on startup and never during automated tests.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LATEST_RELEASE_API_URL: &str =
    "https://api.github.com/repos/YogeGod/dawsu-rs/releases/latest";

/// Holds a SHA-256-verified installer staged to launch once the main window finishes closing.
#[derive(Default)]
pub struct StagedUpdate(Mutex<Option<PathBuf>>);

impl StagedUpdate {
    pub fn stage(&self, path: PathBuf) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = Some(path);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = None;
        }
    }

    pub fn take(&self) -> Option<PathBuf> {
        self.0.lock().ok().and_then(|mut guard| guard.take())
    }

    pub fn launch_if_staged(&self) {
        let Some(installer) = self.take() else {
            return;
        };
        if !installer.is_file() {
            return;
        }
        let _ = spawn_installer(&installer);
    }
}

/// Parsed semantic version (`MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<PreId>,
}

/// A dot-separated pre-release identifier per SemVer 2.0.0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreId {
    Numeric(u64),
    Alpha(String),
}

impl Ord for PreId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            // SemVer 2.0.0 §11.4.3: numeric identifiers always have lower precedence than non-numeric.
            (Self::Numeric(_), Self::Alpha(_)) => Ordering::Less,
            (Self::Alpha(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Alpha(a), Self::Alpha(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PreId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // Stable release has higher precedence than a pre-release of the same core version.
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parses a semantic version string with optional leading `v`/`V`, pre-release, and build metadata.
pub fn parse_semver(raw: &str) -> Option<SemVer> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);

    // Strip `+build` metadata (ignored for version precedence).
    let without_build = stripped.split('+').next()?;
    let (core, pre_str) = match without_build.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (without_build, None),
    };

    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let mut pre = Vec::new();
    if let Some(pre_str) = pre_str {
        if pre_str.is_empty() {
            return None;
        }
        for id in pre_str.split('.') {
            if id.is_empty() {
                return None;
            }
            if id.bytes().all(|b| b.is_ascii_digit()) {
                pre.push(PreId::Numeric(id.parse::<u64>().ok()?));
            } else {
                pre.push(PreId::Alpha(id.to_string()));
            }
        }
    }

    Some(SemVer {
        major,
        minor,
        patch,
        pre,
    })
}

/// Returns `true` when `candidate` is strictly newer than `current`.
pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    match (parse_semver(current), parse_semver(candidate)) {
        (Some(cur), Some(cand)) => cand > cur,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedRelease {
    pub tag_name: String,
    pub version: String,
    pub release_url: String,
    pub notes: String,
    pub installer_url: Option<String>,
    pub portable_url: Option<String>,
    pub sha256sums_url: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub is_installed: bool,
    pub release_url: String,
    pub notes: String,
    pub installer_url: Option<String>,
    pub installer_name: Option<String>,
    pub portable_url: Option<String>,
    pub sha256sums_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedUpdate {
    pub version: String,
    pub installer_path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Deserialize)]
struct GitHubReleasePayload {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubAssetPayload>,
}

#[derive(Deserialize)]
struct GitHubAssetPayload {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

fn require_https(url: &str, field: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if trimmed.starts_with("https://") {
        Ok(trimmed.to_string())
    } else {
        Err(format!("Refusing non-HTTPS URL for {field}."))
    }
}

/// Parses a GitHub Releases API JSON payload and matches installer, portable ZIP, and `SHA256SUMS.txt`.
pub fn parse_github_release(json_str: &str) -> Result<ParsedRelease, String> {
    let payload: GitHubReleasePayload =
        serde_json::from_str(json_str).map_err(|e| format!("Invalid release JSON: {e}"))?;

    let tag_name = payload.tag_name.trim().to_string();
    if parse_semver(&tag_name).is_none() {
        return Err(format!("Release tag '{tag_name}' is not a valid semver."));
    }
    let version = tag_name
        .strip_prefix('v')
        .or_else(|| tag_name.strip_prefix('V'))
        .unwrap_or(&tag_name)
        .to_string();
    let release_url = require_https(&payload.html_url, "html_url")?;
    let notes = payload.body.unwrap_or_default().trim().to_string();

    let mut assets = Vec::with_capacity(payload.assets.len());
    for raw in payload.assets {
        let Ok(download_url) = require_https(&raw.browser_download_url, &raw.name) else {
            continue;
        };
        assets.push(ReleaseAsset {
            name: raw.name,
            download_url,
            size: raw.size,
        });
    }

    let sha256sums_url = assets
        .iter()
        .find(|a| {
            let lower = a.name.to_ascii_lowercase();
            lower == "sha256sums.txt" || lower.ends_with("sha256sums.txt")
        })
        .map(|a| a.download_url.clone());

    let installer_url = assets
        .iter()
        .find(|a| {
            let lower = a.name.to_ascii_lowercase();
            lower.ends_with(".exe") && (lower.contains("setup") || lower.contains("installer"))
        })
        .or_else(|| {
            assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().ends_with(".exe"))
        })
        .map(|a| a.download_url.clone());

    let portable_url = assets
        .iter()
        .find(|a| {
            let lower = a.name.to_ascii_lowercase();
            lower.ends_with(".zip") && lower.contains("portable")
        })
        .or_else(|| {
            assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().ends_with(".zip"))
        })
        .map(|a| a.download_url.clone());

    Ok(ParsedRelease {
        tag_name,
        version,
        release_url,
        notes,
        installer_url,
        portable_url,
        sha256sums_url,
        assets,
    })
}

/// Returns true when the given executable sits inside an installed copy (e.g. Inno Setup
/// `unins000.exe` next to the binary on Windows, or a `.app` bundle on macOS) rather than
/// a portable zip or `target/debug` build.
pub fn is_installed_copy_at(exe_path: &Path) -> bool {
    let Some(dir) = exe_path.parent() else {
        return false;
    };
    if cfg!(target_os = "windows") {
        dir.join("unins000.exe").is_file()
    } else if cfg!(target_os = "macos") {
        exe_path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().ends_with(".app"))
    } else {
        false
    }
}

pub fn is_installed_copy() -> bool {
    std::env::current_exe()
        .ok()
        .is_some_and(|p| is_installed_copy_at(&p))
}

fn asset_filename_from_url(url: &str) -> Option<String> {
    let without_query = url.split(['?', '#']).next()?;
    let name = without_query.rsplit('/').next()?.trim();
    if name.is_empty() || name.contains("..") || name.contains('\\') {
        None
    } else {
        Some(name.to_string())
    }
}

/// Builds the UI-facing update check result from the current version and parsed release.
pub fn build_update_result(current_version: &str, release: ParsedRelease) -> UpdateCheckResult {
    let update_available = is_newer_version(current_version, &release.version);
    let installer_name = release
        .installer_url
        .as_deref()
        .and_then(asset_filename_from_url);
    UpdateCheckResult {
        current_version: current_version.to_string(),
        latest_version: release.version,
        update_available,
        is_installed: is_installed_copy(),
        release_url: release.release_url,
        notes: release.notes,
        installer_url: release.installer_url,
        installer_name,
        portable_url: release.portable_url,
        sha256sums_url: release.sha256sums_url,
    }
}

/// Computes the lowercase hex SHA-256 digest of `bytes` (FIPS 180-4, pure Rust).
pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(bytes.len() + 72);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = String::with_capacity(64);
    for word in h {
        use std::fmt::Write;
        let _ = write!(&mut out, "{word:08x}");
    }
    out
}

fn is_valid_sha256_hex(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Verifies `bytes` against an expected 64-character hex SHA-256 digest.
pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> Result<(), String> {
    let expected = expected_hex.trim().to_ascii_lowercase();
    if !is_valid_sha256_hex(&expected) {
        return Err("Expected SHA-256 digest must be 64 hexadecimal characters.".into());
    }
    let actual = sha256_hex(bytes);
    let mut diff = 0u8;
    for (a, b) in actual.bytes().zip(expected.bytes()) {
        diff |= a ^ b;
    }
    if diff == 0 {
        Ok(())
    } else {
        Err(format!(
            "SHA-256 mismatch: expected {expected}, got {actual}."
        ))
    }
}

/// Looks up `filename` in a `SHA256SUMS.txt` manifest (`<hash>  <filename>` or `<hash> *<filename>`).
pub fn parse_sha256sums(text: &str, filename: &str) -> Option<String> {
    let target = filename.trim();
    if target.is_empty() {
        return None;
    }
    let target_base = target.rsplit(['/', '\\']).next().unwrap_or(target);
    let mut case_insensitive_match = None;

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let hash = parts.next()?.trim();
        let rest = parts.next()?.trim();
        let entry_name = rest.strip_prefix('*').unwrap_or(rest).trim();
        if !is_valid_sha256_hex(hash) || entry_name.is_empty() {
            continue;
        }
        let entry_base = entry_name
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(entry_name);
        if entry_name == target || entry_base == target_base {
            return Some(hash.to_ascii_lowercase());
        }
        if case_insensitive_match.is_none() && entry_base.eq_ignore_ascii_case(target_base) {
            case_insensitive_match = Some(hash.to_ascii_lowercase());
        }
    }

    case_insensitive_match
}

fn format_check_error(stderr: &str, status: std::process::ExitStatus) -> String {
    if stderr.contains("404") {
        return "No public release was found (if the repository is still private, automatic update checks will work once it is public).".into();
    }
    if stderr.is_empty() {
        format!("GitHub Releases check failed (status {status}).")
    } else {
        format!("GitHub Releases check failed: {stderr}")
    }
}

fn curl_https_bytes(url: &str, timeout_secs: u64, accept: Option<&str>) -> Result<Vec<u8>, String> {
    let url = require_https(url, "update download")?;
    let user_agent = format!("dawsu-updater/{CURRENT_VERSION}");
    let mut cmd = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    let timeout = timeout_secs.to_string();
    cmd.args([
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--max-time",
        &timeout,
        "--proto",
        "=https",
        "-H",
        &format!("User-Agent: {user_agent}"),
    ]);
    if let Some(accept) = accept {
        cmd.args(["-H", &format!("Accept: {accept}")]);
    }
    cmd.arg(&url);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Could not run HTTPS request ({e})."))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format_check_error(&stderr, output.status));
    }
    Ok(output.stdout)
}

/// Fetches the latest release JSON from GitHub Releases over HTTPS with a short timeout.
///
/// Only called when the user explicitly clicks "Check for updates".
pub fn check_latest_release() -> Result<UpdateCheckResult, String> {
    let stdout = curl_https_bytes(
        LATEST_RELEASE_API_URL,
        8,
        Some("application/vnd.github+json"),
    )?;
    let body = String::from_utf8(stdout)
        .map_err(|e| format!("GitHub Releases returned non-UTF-8 response: {e}"))?;
    let parsed = parse_github_release(&body)?;
    Ok(build_update_result(CURRENT_VERSION, parsed))
}

/// Folder under app data where verified update installers are staged.
pub fn updates_dir(data_root: &Path) -> PathBuf {
    data_root.join("updates")
}

/// Verifies `installer_bytes` against `sha256sums_text` for `filename` and atomically writes the
/// verified installer into `<data_root>/updates/<version>/<filename>`. If verification fails,
/// nothing is written (and any stray temp file is removed).
pub fn verify_and_write_installer(
    data_root: &Path,
    version: &str,
    filename: &str,
    installer_bytes: &[u8],
    sha256sums_text: &str,
) -> Result<DownloadedUpdate, String> {
    if parse_semver(version).is_none() {
        return Err(format!("Invalid update version '{version}'."));
    }
    let clean_name = filename.trim();
    if clean_name.is_empty()
        || clean_name.contains(['/', '\\'])
        || clean_name.contains("..")
    {
        return Err("Invalid installer filename.".into());
    }
    let expected_hex = parse_sha256sums(sha256sums_text, clean_name).ok_or_else(|| {
        format!("SHA256SUMS.txt does not list a checksum for {clean_name}. Refusing to install.")
    })?;
    verify_sha256(installer_bytes, &expected_hex)?;

    let safe_version = version
        .trim()
        .strip_prefix('v')
        .or_else(|| version.trim().strip_prefix('V'))
        .unwrap_or(version.trim());
    let target_dir = updates_dir(data_root).join(safe_version);
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Could not create updates folder: {e}"))?;
    let dest = target_dir.join(clean_name);
    let tmp = target_dir.join(format!("{clean_name}.part"));
    if let Err(e) = fs::write(&tmp, installer_bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not write downloaded installer: {e}"));
    }
    if dest.exists() {
        let _ = fs::remove_file(&dest);
    }
    if let Err(e) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not finalize downloaded installer: {e}"));
    }

    Ok(DownloadedUpdate {
        version: safe_version.to_string(),
        installer_path: dest.to_string_lossy().into_owned(),
        sha256: expected_hex,
        size_bytes: installer_bytes.len() as u64,
    })
}

/// Downloads `SHA256SUMS.txt` and the release installer over HTTPS, verifies the SHA-256 digest,
/// and saves the verified installer inside `<data_root>/updates/<version>/`.
pub fn download_and_verify_update(
    data_root: &Path,
    version: &str,
    installer_url: &str,
    sha256sums_url: &str,
) -> Result<DownloadedUpdate, String> {
    let filename = asset_filename_from_url(installer_url)
        .ok_or_else(|| "Installer URL does not end with a valid filename.".to_string())?;
    let sums_bytes = curl_https_bytes(sha256sums_url, 20, None)?;
    let sums_text = String::from_utf8(sums_bytes)
        .map_err(|e| format!("SHA256SUMS.txt is not valid UTF-8: {e}"))?;
    let installer_bytes = curl_https_bytes(installer_url, 180, None)?;
    verify_and_write_installer(data_root, version, &filename, &installer_bytes, &sums_text)
}

/// Validates that a staged installer path is a real file inside `<data_root>/updates/`.
pub fn validate_staged_installer_path(data_root: &Path, raw: &str) -> Result<PathBuf, String> {
    let candidate = PathBuf::from(raw.trim());
    if !candidate.is_file() {
        return Err("Downloaded installer file not found.".into());
    }
    let canon_root = updates_dir(data_root)
        .canonicalize()
        .map_err(|_| "Updates folder does not exist.".to_string())?;
    let canon_file = candidate
        .canonicalize()
        .map_err(|e| format!("Could not resolve installer path: {e}"))?;
    if !canon_file.starts_with(&canon_root) {
        return Err("Refusing to run an installer outside dawsu!'s updates folder.".into());
    }
    Ok(canon_file)
}

pub fn installer_launch_args() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        // Inno Setup silent upgrade: waits for dawsu! to exit, updates in place, and relaunches.
        &["/SILENT", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS"]
    } else {
        &[]
    }
}

fn spawn_installer(installer_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        Command::new(installer_path)
            .args(installer_launch_args())
            .spawn()
            .map_err(|e| format!("Could not launch installer: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(installer_path)
            .spawn()
            .map_err(|e| format!("Could not open update package: {e}"))?;
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(installer_path)
            .spawn()
            .map_err(|e| format!("Could not open update package: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison_handles_stable_prerelease_and_equal() {
        assert!(is_newer_version("0.1.9", "0.2.0"));
        assert!(is_newer_version("v0.1.9", "v0.2.0"));
        assert!(is_newer_version("1.0.0-beta.1", "v1.0.0"));
        assert!(is_newer_version("1.0.0-alpha.1", "1.0.0-beta.1"));
        assert!(is_newer_version("1.0.0-beta.2", "1.0.0-beta.10"));

        assert!(!is_newer_version("0.2.0", "0.2.0"));
        assert!(!is_newer_version("v0.2.0", "0.2.0"));
        assert!(!is_newer_version("0.2.0", "0.1.9"));
        assert!(!is_newer_version("1.0.0", "1.0.0-rc.1"));
        assert!(!is_newer_version("invalid", "0.2.0"));
        assert!(!is_newer_version("0.2.0", "not-a-version"));
    }

    #[test]
    fn parses_synthetic_github_release_and_matches_assets() {
        let payload = r#"{
            "tag_name": "v0.3.0",
            "html_url": "https://github.com/YogeGod/dawsu-rs/releases/tag/v0.3.0",
            "body": "Installer, portable ZIP, and update checker.",
            "assets": [
                {
                    "name": "dawsu-v0.3.0-windows-x64-setup.exe",
                    "browser_download_url": "https://github.com/YogeGod/dawsu-rs/releases/download/v0.3.0/dawsu-v0.3.0-windows-x64-setup.exe",
                    "size": 12345678
                },
                {
                    "name": "dawsu-v0.3.0-windows-x64-portable.zip",
                    "browser_download_url": "https://github.com/YogeGod/dawsu-rs/releases/download/v0.3.0/dawsu-v0.3.0-windows-x64-portable.zip",
                    "size": 11223344
                },
                {
                    "name": "SHA256SUMS.txt",
                    "browser_download_url": "https://github.com/YogeGod/dawsu-rs/releases/download/v0.3.0/SHA256SUMS.txt",
                    "size": 256
                }
            ]
        }"#;

        let parsed = parse_github_release(payload).expect("valid release JSON");
        assert_eq!(parsed.tag_name, "v0.3.0");
        assert_eq!(parsed.version, "0.3.0");
        assert_eq!(
            parsed.installer_url.as_deref(),
            Some("https://github.com/YogeGod/dawsu-rs/releases/download/v0.3.0/dawsu-v0.3.0-windows-x64-setup.exe")
        );
        assert_eq!(
            parsed.portable_url.as_deref(),
            Some("https://github.com/YogeGod/dawsu-rs/releases/download/v0.3.0/dawsu-v0.3.0-windows-x64-portable.zip")
        );
        assert_eq!(
            parsed.sha256sums_url.as_deref(),
            Some("https://github.com/YogeGod/dawsu-rs/releases/download/v0.3.0/SHA256SUMS.txt")
        );

        let result = build_update_result("0.1.0", parsed);
        assert!(result.update_available);
        assert_eq!(result.current_version, "0.1.0");
        assert_eq!(result.latest_version, "0.3.0");
        assert_eq!(result.notes, "Installer, portable ZIP, and update checker.");
    }

    #[test]
    fn parse_sha256sums_and_verify_sha256_catch_tampering() {
        let data = b"dawsu release artifact bytes";
        let expected = sha256_hex(data);
        // Known NIST vector sanity check for "abc".
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let manifest = format!(
            "# dawsu! release checksums\n\
             {}  dawsu-v0.3.0-windows-x64-portable.zip\n\
             0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef *dawsu-v0.3.0-windows-x64-setup.exe\n",
            expected.to_ascii_uppercase()
        );

        let found = parse_sha256sums(&manifest, "dawsu-v0.3.0-windows-x64-portable.zip")
            .expect("should find portable zip digest");
        assert_eq!(found, expected);
        assert!(verify_sha256(data, &found).is_ok());

        let mut tampered = data.to_vec();
        tampered[0] ^= 0x01;
        let err = verify_sha256(&tampered, &found).expect_err("tampered bytes must fail");
        assert!(err.contains("SHA-256 mismatch"), "unexpected error: {err}");

        assert!(parse_sha256sums(&manifest, "missing-file.zip").is_none());
        assert!(verify_sha256(data, "not-hex").is_err());
    }

    fn temp_test_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dawsu-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn verify_and_write_installer_accepts_valid_hash_and_rejects_tampered_bytes() {
        let root = temp_test_dir("updater-verify-stage");
        let data_root = root.join("appdata");
        let installer_bytes = b"MZ-fake-inno-setup-binary-bytes";
        let digest = sha256_hex(installer_bytes);
        let sums = format!("{digest}  dawsu-v1.1.0-windows-x64-setup.exe\n");

        let ok = verify_and_write_installer(
            &data_root,
            "v1.1.0",
            "dawsu-v1.1.0-windows-x64-setup.exe",
            installer_bytes,
            &sums,
        )
        .expect("valid installer must verify and stage");
        assert_eq!(ok.version, "1.1.0");
        assert_eq!(ok.sha256, digest);
        assert_eq!(ok.size_bytes, installer_bytes.len() as u64);
        assert!(Path::new(&ok.installer_path).is_file());
        assert!(validate_staged_installer_path(&data_root, &ok.installer_path).is_ok());

        // Tampered bytes must be refused and leave no file in the new version folder.
        let mut tampered = installer_bytes.to_vec();
        tampered[0] ^= 0xff;
        let err = verify_and_write_installer(
            &data_root,
            "v1.2.0",
            "dawsu-v1.2.0-windows-x64-setup.exe",
            &tampered,
            &format!("{digest}  dawsu-v1.2.0-windows-x64-setup.exe\n"),
        )
        .expect_err("tampered installer must be rejected");
        assert!(err.contains("SHA-256 mismatch"), "{err}");
        assert!(!updates_dir(&data_root).join("1.2.0").exists());

        // Path outside updates folder must be refused.
        let outside = root.join("outside.exe");
        fs::write(&outside, b"bad").unwrap();
        assert!(validate_staged_installer_path(&data_root, &outside.to_string_lossy()).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn staged_update_state_and_installed_detection() {
        let staged = StagedUpdate::default();
        assert!(staged.take().is_none());
        staged.stage(PathBuf::from("C:/tmp/setup.exe"));
        staged.clear();
        assert!(staged.take().is_none());

        staged.stage(PathBuf::from("C:/tmp/setup.exe"));
        assert_eq!(staged.take(), Some(PathBuf::from("C:/tmp/setup.exe")));
        assert!(staged.take().is_none());

        let root = temp_test_dir("updater-installed-check");
        let portable_dir = root.join("portable");
        fs::create_dir_all(&portable_dir).unwrap();
        let portable_exe = portable_dir.join("dawsu.exe");
        fs::write(&portable_exe, b"exe").unwrap();
        assert!(!is_installed_copy_at(&portable_exe));

        if cfg!(target_os = "windows") {
            let installed_dir = root.join("installed");
            fs::create_dir_all(&installed_dir).unwrap();
            let installed_exe = installed_dir.join("dawsu.exe");
            fs::write(&installed_exe, b"exe").unwrap();
            fs::write(installed_dir.join("unins000.exe"), b"unins").unwrap();
            assert!(is_installed_copy_at(&installed_exe));
        }
        let _ = fs::remove_dir_all(&root);
    }
}
