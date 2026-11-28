//! GitHub Releases update checker, SHA-256 verified downloader, and install-on-close staging.
//!
//! Network access happens only when the user explicitly clicks Check / Download in
//! Preferences › Updates — never on startup and never during automated tests.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LATEST_RELEASE_API_URL: &str =
    "https://api.github.com/repos/YogeGod/dawsu-rs/releases/latest";
/// Release list queried by pre-release builds (`/releases/latest` never returns pre-releases).
pub const RELEASE_LIST_API_URL: &str =
    "https://api.github.com/repos/YogeGod/dawsu-rs/releases?per_page=10";
/// Maximum allowed release list response size (2 MiB).
pub const MAX_RELEASE_LIST_BYTES: u64 = 2 * 1024 * 1024;
pub const RELEASE_ASSET_URL_PREFIX: &str =
    "https://github.com/YogeGod/dawsu-rs/releases/download/";
/// Maximum allowed installer download size (256 MiB).
pub const MAX_INSTALLER_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum allowed `SHA256SUMS.txt` size (1 MiB).
pub const MAX_SHA256SUMS_BYTES: u64 = 1024 * 1024;

static PART_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePackageKind {
    Installer,
    PortableZip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedDownload {
    pub version: String,
    pub path: PathBuf,
    pub expected_sha256: String,
    pub kind: UpdatePackageKind,
}

/// Returns `true` when the running executable appears to be a portable ZIP install rather than an
/// Inno Setup installation (e.g. `portable.marker` exists next to the exe or no `unins000.exe`
/// uninstaller is present in the exe directory and it is outside Program Files).
pub fn is_portable_install(exe_path: &Path) -> bool {
    let Some(dir) = exe_path.parent() else {
        return false;
    };
    if dir.join("portable.marker").is_file() {
        return true;
    }
    if is_installed_copy_at(exe_path) {
        return false;
    }
    let dir_lower = dir.to_string_lossy().to_ascii_lowercase();
    !dir_lower.contains("\\program files") && !dir_lower.contains("/applications")
}

/// Validates a portable ZIP archive's Central Directory in pure Rust (zero extra crates):
/// verifies valid ZIP signatures, enforces the flat-archive safety rule (no `..`, no drive/root
/// prefixes, no backslashes escaping the install root, no Unix symlinks), and rejects ZIP64.
pub fn validate_flat_portable_zip(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.len() < 22 || &bytes[0..4] != b"PK\x03\x04" {
        return Err("Portable update archive is not a valid ZIP file.".into());
    }
    // Locate End of Central Directory (EOCD) signature 0x06054b50 scanning backward.
    let search_start = bytes.len().saturating_sub(65_557);
    let eocd_pos = (search_start..=bytes.len() - 22)
        .rev()
        .find(|&i| &bytes[i..i + 4] == b"PK\x05\x06")
        .ok_or("Portable ZIP is missing End of Central Directory record.")?;
    let raw_entries = u16::from_le_bytes([bytes[eocd_pos + 10], bytes[eocd_pos + 11]]);
    let raw_cd_size = u32::from_le_bytes([
        bytes[eocd_pos + 12],
        bytes[eocd_pos + 13],
        bytes[eocd_pos + 14],
        bytes[eocd_pos + 15],
    ]);
    let raw_cd_offset = u32::from_le_bytes([
        bytes[eocd_pos + 16],
        bytes[eocd_pos + 17],
        bytes[eocd_pos + 18],
        bytes[eocd_pos + 19],
    ]);
    if raw_entries == u16::MAX || raw_cd_size == u32::MAX || raw_cd_offset == u32::MAX {
        return Err("ZIP64 portable update archives are not supported.".into());
    }
    let entry_count = raw_entries as usize;
    let cd_size = raw_cd_size as usize;
    let cd_offset = raw_cd_offset as usize;
    if cd_offset.saturating_add(cd_size) > eocd_pos || entry_count == 0 {
        return Err("Portable ZIP has an invalid or empty Central Directory.".into());
    }

    let mut pos = cd_offset;
    let end = cd_offset + cd_size;
    let mut names = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        if pos + 46 > end || &bytes[pos..pos + 4] != b"PK\x01\x02" {
            return Err("Corrupt entry in portable ZIP Central Directory.".into());
        }
        let version_made_by_os = bytes[pos + 5];
        let ext_attr = u32::from_le_bytes([
            bytes[pos + 38],
            bytes[pos + 39],
            bytes[pos + 40],
            bytes[pos + 41],
        ]);
        if version_made_by_os == 3 && ((ext_attr >> 16) & 0o170000) == 0o120000 {
            return Err("Unsafe symlink entry inside portable ZIP.".into());
        }
        let name_len = u16::from_le_bytes([bytes[pos + 28], bytes[pos + 29]]) as usize;
        let extra_len = u16::from_le_bytes([bytes[pos + 30], bytes[pos + 31]]) as usize;
        let comment_len = u16::from_le_bytes([bytes[pos + 32], bytes[pos + 33]]) as usize;
        let name_start = pos + 46;
        let name_end = name_start + name_len;
        if name_end > end {
            return Err("Truncated filename in portable ZIP.".into());
        }
        let name = std::str::from_utf8(&bytes[name_start..name_end])
            .map_err(|_| "Non-UTF-8 filename in portable ZIP.")?;
        if name.is_empty()
            || name.starts_with('/')
            || name.starts_with('\\')
            || name.contains(':')
            || name.contains("..")
            || name.contains('\\')
            || name.contains('\0')
        {
            return Err(format!("Unsafe path inside portable ZIP: {name}"));
        }
        // Flat-archive rule: entries must not be nested more than 1 subfolder deep.
        if name.matches('/').count() > 1 {
            return Err(format!("Unexpected deep nesting in portable ZIP: {name}"));
        }
        names.push(name.to_owned());
        pos = name_end + extra_len + comment_len;
    }
    if pos != end {
        return Err("Portable ZIP Central Directory size mismatch.".into());
    }
    Ok(names)
}

#[derive(Default)]
struct UpdaterInner {
    last_release: Option<ParsedRelease>,
    verified_download: Option<VerifiedDownload>,
    armed_for_exit: bool,
}

/// Holds the last Rust-fetched release, the SHA-256-verified download, and whether it is armed
/// to launch when the main window finishes closing. Neither URLs nor file paths are accepted
/// from the webview.
#[derive(Default)]
pub struct StagedUpdate(Mutex<UpdaterInner>);

impl StagedUpdate {
    pub fn record_release(&self, release: ParsedRelease) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.last_release = Some(release);
    }

    pub fn last_release(&self) -> Option<ParsedRelease> {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.last_release.clone()
    }

    pub fn record_download(&self, download: VerifiedDownload) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.verified_download = Some(download);
        guard.armed_for_exit = false;
    }

    pub fn arm_verified_download(&self, data_root: &Path) -> Result<VerifiedDownload, String> {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(download) = guard.verified_download.clone() else {
            return Err("No verified update has been downloaded yet.".into());
        };
        validate_staged_installer_path(data_root, &download.path)?;
        guard.armed_for_exit = true;
        Ok(download)
    }

    pub fn clear(&self) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.armed_for_exit = false;
    }

    pub fn take_armed(&self) -> Option<VerifiedDownload> {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if !guard.armed_for_exit {
            return None;
        }
        guard.armed_for_exit = false;
        guard.verified_download.take()
    }

    /// Re-verifies the armed installer's path and SHA-256 on disk right before launching it.
    pub fn launch_if_staged(&self, data_root: &Path) {
        let Some(download) = self.take_armed() else {
            return;
        };
        let verified_path = match validate_staged_installer_path(data_root, &download.path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[dawsu updater] refusing to launch staged update: {e}");
                return;
            }
        };
        if let Err(e) = verify_file_sha256(&verified_path, &download.expected_sha256) {
            eprintln!("[dawsu updater] pre-launch SHA-256 check failed: {e}");
            let _ = fs::remove_file(&verified_path);
            return;
        }
        let launch_res = match download.kind {
            UpdatePackageKind::Installer => spawn_installer(&verified_path),
            UpdatePackageKind::PortableZip => spawn_portable_swap(&verified_path),
        };
        if let Err(e) = launch_res {
            eprintln!("[dawsu updater] failed to launch update: {e}");
        }
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

/// Picks the GitHub Releases endpoint for the running build: a build whose own version has a
/// pre-release part asks for the release list, a stable build keeps `/releases/latest`.
pub fn release_api_url(current: &str) -> &'static str {
    if parse_semver(current).is_some_and(|v| !v.pre.is_empty()) {
        RELEASE_LIST_API_URL
    } else {
        LATEST_RELEASE_API_URL
    }
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
    draft: bool,
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
    release_from_payload(payload)
}

/// Parses a GitHub `/releases` list and returns the highest-semver release that is not a draft
/// and has a valid tag.
pub fn pick_newest_release(json_str: &str) -> Result<ParsedRelease, String> {
    let payloads: Vec<GitHubReleasePayload> =
        serde_json::from_str(json_str).map_err(|e| format!("Invalid release list JSON: {e}"))?;
    let newest = payloads
        .into_iter()
        .filter(|p| !p.draft)
        .filter_map(|p| parse_semver(&p.tag_name).map(|v| (v, p)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, p)| p)
        .ok_or("No published release with a valid version tag was found.")?;
    release_from_payload(newest)
}

fn release_from_payload(payload: GitHubReleasePayload) -> Result<ParsedRelease, String> {
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

    let (installer_url, portable_url) = select_platform_assets(
        &assets,
        platform_token(std::env::consts::OS, std::env::consts::ARCH),
    );

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

/// Release asset name token for a platform (`tools/package-release.ps1` names assets
/// `dawsu-v<version>-windows-x64-{setup.exe,portable.zip}`). Platforms without published
/// packages return `None`.
pub fn platform_token(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("windows-x64"),
        _ => None,
    }
}

/// Returns `(installer_url, portable_url)` for the assets that carry `token`. With no token,
/// nothing matches, so the UI only offers the release page.
pub fn select_platform_assets(
    assets: &[ReleaseAsset],
    token: Option<&str>,
) -> (Option<String>, Option<String>) {
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return (None, None);
    };
    let token = token.to_ascii_lowercase();
    let find = |is_match: &dyn Fn(&str) -> bool| {
        assets
            .iter()
            .find(|a| {
                let lower = a.name.to_ascii_lowercase();
                lower.contains(&token) && is_match(&lower)
            })
            .map(|a| a.download_url.clone())
    };
    let installer =
        find(&|n| n.ends_with(".exe") && (n.contains("setup") || n.contains("installer")));
    let portable = find(&|n| n.ends_with(".zip") && n.contains("portable"));
    (installer, portable)
}

/// Returns true when the given executable sits inside an installed copy (e.g. Inno Setup
/// `unins*.exe` next to the binary on Windows, or an installed `.app` outside `/Volumes/`
/// on macOS) rather than a portable zip or `target/debug` build.
pub fn is_installed_copy_at(exe_path: &Path) -> bool {
    let Some(dir) = exe_path.parent() else {
        return false;
    };
    if cfg!(target_os = "windows") {
        fs::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                name.starts_with("unins") && name.ends_with(".exe") && entry.path().is_file()
            })
    } else if cfg!(target_os = "macos") {
        let s = exe_path.to_string_lossy();
        !s.starts_with("/Volumes/")
            && !s.contains("/target/")
            && exe_path
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

fn require_release_asset_url(url: &str, field: &str) -> Result<String, String> {
    let trimmed = require_https(url, field)?;
    if !trimmed.starts_with(RELEASE_ASSET_URL_PREFIX) {
        return Err(format!(
            "Refusing {field} URL outside {RELEASE_ASSET_URL_PREFIX}."
        ));
    }
    Ok(trimmed)
}

fn validate_safe_filename(name: &str) -> Result<String, String> {
    let clean = name.trim();
    if clean.is_empty()
        || clean.contains(['/', '\\', ':', '\0'])
        || clean.contains("..")
        || clean.bytes().any(|b| b.is_ascii_control())
    {
        return Err("Invalid installer filename.".into());
    }
    let mut comps = Path::new(clean).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => Ok(clean.to_string()),
        _ => Err("Invalid installer filename.".into()),
    }
}

fn sanitize_semver_dir(version: &str) -> Result<String, String> {
    let parsed = parse_semver(version)
        .ok_or_else(|| format!("Invalid update version '{version}'."))?;
    let mut out = format!("{}.{}.{}", parsed.major, parsed.minor, parsed.patch);
    if !parsed.pre.is_empty() {
        out.push('-');
        for (i, id) in parsed.pre.iter().enumerate() {
            if i > 0 {
                out.push('.');
            }
            match id {
                PreId::Numeric(n) => out.push_str(&n.to_string()),
                PreId::Alpha(s) => {
                    if !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                        return Err(format!("Invalid pre-release identifier in version '{version}'."));
                    }
                    out.push_str(s);
                }
            }
        }
    }
    Ok(out)
}

fn asset_filename_from_url(url: &str) -> Option<String> {
    let without_query = url.split(['?', '#']).next()?;
    let name = without_query.rsplit('/').next()?.trim();
    validate_safe_filename(name).ok()
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

    for chunk in padded.as_chunks::<64>().0 {
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
#[cfg(test)]
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

fn format_curl_error(context: &str, is_release_check: bool, stderr: &str, status: std::process::ExitStatus) -> String {
    if is_release_check && stderr.contains("404") {
        return "No public release was found (if the repository is still private, automatic update checks will work once it is public).".into();
    }
    if stderr.is_empty() {
        format!("{context} failed (status {status}).")
    } else {
        format!("{context} failed: {stderr}")
    }
}

fn build_curl_command(timeout_secs: u64, max_filesize: u64, accept: Option<&str>) -> Command {
    let user_agent = format!("dawsu-updater/{CURRENT_VERSION}");
    let mut cmd = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    let timeout = timeout_secs.to_string();
    let max_size = max_filesize.to_string();
    cmd.args([
        "--silent",
        "--show-error",
        "--fail",
        "--location",
        "--max-redirs",
        "5",
        "--max-time",
        &timeout,
        "--max-filesize",
        &max_size,
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "-H",
        &format!("User-Agent: {user_agent}"),
    ]);
    if let Some(accept) = accept {
        cmd.args(["-H", &format!("Accept: {accept}")]);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Maximum bytes of curl stderr retained for diagnostics (remainder drained to avoid pipe deadlock).
const MAX_CURL_STDERR_BYTES: u64 = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
enum BoundedRead {
    WithinLimit(Vec<u8>),
    Exceeded,
}

/// Reads at most `max_bytes` from `reader` without unbounded allocation.
fn read_bounded<R: Read>(reader: R, max_bytes: u64) -> std::io::Result<BoundedRead> {
    let mut buf = Vec::new();
    reader
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut buf)?;
    if buf.len() as u64 > max_bytes {
        return Ok(BoundedRead::Exceeded);
    }
    Ok(BoundedRead::WithinLimit(buf))
}

/// Collects stdout/stderr from a piped child while strictly bounding memory to `max_bytes`
/// (+ `MAX_CURL_STDERR_BYTES` for stderr). Kills and reaps the child immediately if stdout
/// exceeds `max_bytes` (even on chunked responses with no `Content-Length`).
fn collect_bounded_output(
    mut child: Child,
    context: &str,
    is_release_check: bool,
    max_bytes: u64,
) -> Result<Vec<u8>, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("Could not capture stdout for {context}."))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("Could not capture stderr for {context}."))?;

    let (stdout_res, stderr_bytes) = std::thread::scope(|scope| {
        let stderr_handle = scope.spawn(move || {
            let mut err_buf = Vec::new();
            let _ = (&mut stderr)
                .take(MAX_CURL_STDERR_BYTES)
                .read_to_end(&mut err_buf);
            let _ = std::io::copy(&mut stderr, &mut std::io::sink());
            err_buf
        });

        let out_res = read_bounded(stdout, max_bytes);
        // stdout was dropped above; if the cap was exceeded or reading failed, kill and reap the
        // child before joining stderr_handle so it cannot block on stdout or hold stderr open.
        if !matches!(out_res, Ok(BoundedRead::WithinLimit(_))) {
            let _ = child.kill();
            let _ = child.wait();
        }
        let err_bytes = stderr_handle.join().unwrap_or_default();
        (out_res, err_bytes)
    });

    let buf = match stdout_res {
        Ok(BoundedRead::WithinLimit(buf)) => buf,
        Ok(BoundedRead::Exceeded) => {
            return Err(format!("{context} response is larger than allowed."));
        }
        Err(e) => {
            return Err(format!("Could not read {context} response ({e})."));
        }
    };

    let status = child
        .wait()
        .map_err(|e| format!("Could not wait for {context} ({e})."))?;
    if !status.success() {
        let stderr_text = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
        return Err(format_curl_error(
            context,
            is_release_check,
            &stderr_text,
            status,
        ));
    }
    Ok(buf)
}

fn curl_https_bytes(
    url: &str,
    context: &str,
    is_release_check: bool,
    timeout_secs: u64,
    max_filesize: u64,
    accept: Option<&str>,
) -> Result<Vec<u8>, String> {
    let url = require_https(url, context)?;
    let mut cmd = build_curl_command(timeout_secs, max_filesize, accept);
    cmd.arg("--")
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = cmd
        .spawn()
        .map_err(|e| format!("Could not run HTTPS request ({e})."))?;
    collect_bounded_output(child, context, is_release_check, max_filesize)
}

fn curl_https_to_file(
    url: &str,
    dest: &Path,
    context: &str,
    timeout_secs: u64,
    max_filesize: u64,
) -> Result<(), String> {
    let url = require_https(url, context)?;
    let mut cmd = build_curl_command(timeout_secs, max_filesize, None);
    cmd.arg("--output").arg(dest).arg("--").arg(&url);

    let output = cmd
        .output()
        .map_err(|e| format!("Could not run HTTPS download ({e})."))?;

    if !output.status.success() {
        let _ = fs::remove_file(dest);
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format_curl_error(context, false, &stderr, output.status));
    }
    Ok(())
}

/// Computes the SHA-256 digest of a file on disk without buffering the whole file in memory.
pub fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("Could not read file for SHA-256: {e}"))?;
    let mut buf = Vec::new();
    let meta = file.metadata().map_err(|e| format!("Could not stat file: {e}"))?;
    if meta.len() > MAX_INSTALLER_BYTES {
        return Err("Downloaded file exceeds maximum allowed installer size.".into());
    }
    buf.reserve(meta.len() as usize);
    let mut chunk = [0u8; 65_536];
    loop {
        let n = file.read(&mut chunk).map_err(|e| format!("Could not read file: {e}"))?;
        if n == 0 {
            break;
        }
        if (buf.len() as u64) + (n as u64) > MAX_INSTALLER_BYTES {
            return Err("Downloaded file exceeds maximum allowed installer size.".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(sha256_hex(&buf))
}

pub fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<(), String> {
    let expected = expected_hex.trim().to_ascii_lowercase();
    if !is_valid_sha256_hex(&expected) {
        return Err("Expected SHA-256 digest must be 64 hexadecimal characters.".into());
    }
    let actual = sha256_file_hex(path)?;
    let mut diff = 0u8;
    for (a, b) in actual.bytes().zip(expected.bytes()) {
        diff |= a ^ b;
    }
    if diff == 0 {
        Ok(())
    } else {
        Err(format!("SHA-256 mismatch: expected {expected}, got {actual}."))
    }
}

/// Fetches the latest release JSON from GitHub Releases over HTTPS with a short timeout.
/// Returns both the UI result and the raw `ParsedRelease` for Rust-side download trust.
pub fn check_latest_release_parsed() -> Result<(UpdateCheckResult, ParsedRelease), String> {
    let url = release_api_url(CURRENT_VERSION);
    let is_list = url == RELEASE_LIST_API_URL;
    let max_bytes = if is_list {
        MAX_RELEASE_LIST_BYTES
    } else {
        MAX_SHA256SUMS_BYTES
    };
    let stdout = curl_https_bytes(
        url,
        "GitHub Releases check",
        true,
        8,
        max_bytes,
        Some("application/vnd.github+json"),
    )?;
    let body = String::from_utf8(stdout)
        .map_err(|e| format!("GitHub Releases returned non-UTF-8 response: {e}"))?;
    let parsed = if is_list {
        pick_newest_release(&body)?
    } else {
        parse_github_release(&body)?
    };
    let result = build_update_result(CURRENT_VERSION, parsed.clone());
    Ok((result, parsed))
}

/// Folder under app data where verified update installers are staged.
pub fn updates_dir(data_root: &Path) -> PathBuf {
    data_root.join("updates")
}

/// Verifies `installer_bytes` against `sha256sums_text` for `filename` and atomically writes the
/// verified installer into `<data_root>/updates/<version>/<filename>`. If verification fails,
/// nothing is written (and any stray temp file is removed).
#[cfg(test)]
pub fn verify_and_write_installer(
    data_root: &Path,
    version: &str,
    filename: &str,
    installer_bytes: &[u8],
    sha256sums_text: &str,
) -> Result<DownloadedUpdate, String> {
    let safe_version = sanitize_semver_dir(version)?;
    let clean_name = validate_safe_filename(filename)?;
    let expected_hex = parse_sha256sums(sha256sums_text, &clean_name).ok_or_else(|| {
        format!("SHA256SUMS.txt does not list a checksum for {clean_name}. Refusing to install.")
    })?;
    verify_sha256(installer_bytes, &expected_hex)?;

    let target_dir = updates_dir(data_root).join(&safe_version);
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Could not create updates folder: {e}"))?;
    let dest = target_dir.join(&clean_name);
    let seq = PART_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let tmp = target_dir.join(format!("{clean_name}.{}.{seq}.part", std::process::id()));
    if let Err(e) = fs::write(&tmp, installer_bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not write downloaded installer: {e}"));
    }
    if dest.exists() {
        fs::remove_file(&dest)
            .map_err(|e| {
                let _ = fs::remove_file(&tmp);
                format!("Could not replace previous installer file: {e}")
            })?;
    }
    if let Err(e) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not finalize downloaded installer: {e}"));
    }

    Ok(DownloadedUpdate {
        version: safe_version,
        installer_path: dest.to_string_lossy().into_owned(),
        sha256: expected_hex,
        size_bytes: installer_bytes.len() as u64,
    })
}

/// Downloads `SHA256SUMS.txt` and the release installer from a Rust-fetched `ParsedRelease`
/// over HTTPS, verifies the SHA-256 digest on disk before moving into place, and deletes the
/// temp file on any mismatch.
pub fn download_and_verify_release(
    data_root: &Path,
    release: &ParsedRelease,
) -> Result<(DownloadedUpdate, VerifiedDownload), String> {
    let prefer_portable = std::env::current_exe()
        .ok()
        .is_some_and(|exe| is_portable_install(&exe));
    let (package_url, kind) = if prefer_portable && release.portable_url.is_some() {
        (
            release.portable_url.as_deref().unwrap(),
            UpdatePackageKind::PortableZip,
        )
    } else if let Some(url) = release.installer_url.as_deref() {
        (url, UpdatePackageKind::Installer)
    } else if let Some(url) = release.portable_url.as_deref() {
        (url, UpdatePackageKind::PortableZip)
    } else {
        return Err("This release does not include an installer or portable ZIP.".to_string());
    };
    let sha256sums_url = release
        .sha256sums_url
        .as_deref()
        .ok_or_else(|| "This release does not include SHA256SUMS.txt.".to_string())?;
    let installer_url = require_release_asset_url(package_url, "update package")?;
    let sha256sums_url = require_release_asset_url(sha256sums_url, "SHA256SUMS.txt")?;

    let safe_version = sanitize_semver_dir(&release.version)?;
    let filename = asset_filename_from_url(&installer_url)
        .ok_or_else(|| "Update URL does not end with a valid filename.".to_string())?;
    let sums_bytes = curl_https_bytes(
        &sha256sums_url,
        "SHA256SUMS.txt download",
        false,
        20,
        MAX_SHA256SUMS_BYTES,
        None,
    )?;
    let sums_text = String::from_utf8(sums_bytes)
        .map_err(|e| format!("SHA256SUMS.txt is not valid UTF-8: {e}"))?;
    let expected_hex = parse_sha256sums(&sums_text, &filename).ok_or_else(|| {
        format!("SHA256SUMS.txt does not list a checksum for {filename}. Refusing to install.")
    })?;

    let target_dir = updates_dir(data_root).join(&safe_version);
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Could not create updates folder: {e}"))?;
    let seq = PART_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let tmp = target_dir.join(format!("{filename}.{}.{seq}.part", std::process::id()));
    curl_https_to_file(
        &installer_url,
        &tmp,
        "Update package download",
        180,
        MAX_INSTALLER_BYTES,
    )?;

    if let Err(e) = verify_file_sha256(&tmp, &expected_hex) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if kind == UpdatePackageKind::PortableZip {
        let zip_bytes = fs::read(&tmp).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("Could not read downloaded portable ZIP: {e}")
        })?;
        if let Err(e) = validate_flat_portable_zip(&zip_bytes) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    let size_bytes = fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    let dest = target_dir.join(&filename);
    if dest.exists() {
        fs::remove_file(&dest).map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("Could not replace previous update file: {e}")
        })?;
    }
    if let Err(e) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not finalize downloaded update: {e}"));
    }

    let ui_result = DownloadedUpdate {
        version: safe_version.clone(),
        installer_path: dest.to_string_lossy().into_owned(),
        sha256: expected_hex.clone(),
        size_bytes,
    };
    let verified = VerifiedDownload {
        version: safe_version,
        path: dest,
        expected_sha256: expected_hex,
        kind,
    };
    Ok((ui_result, verified))
}

/// Validates that a staged installer path is a real file inside `<data_root>/updates/`.
pub fn validate_staged_installer_path(data_root: &Path, candidate: &Path) -> Result<PathBuf, String> {
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

#[cfg(target_os = "windows")]
fn to_wide_null(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Formats a staged installer path for Win32 `ShellExecuteW`, stripping any extended-length
/// `\\?\` / `\\?\UNC\` prefix produced by `fs::canonicalize`.
#[cfg(any(target_os = "windows", test))]
fn installer_shell_execute_path(installer_path: &Path) -> String {
    strip_unc_prefix(installer_path)
}

fn spawn_installer(installer_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        // Use Win32 ShellExecuteW so Windows honors the Inno Setup UAC elevation manifest
        // instead of failing with ERROR_ELEVATION_REQUIRED (740) under CreateProcessW.
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn ShellExecuteW(
                hwnd: *mut core::ffi::c_void,
                lp_operation: *const u16,
                lp_file: *const u16,
                lp_parameters: *const u16,
                lp_directory: *const u16,
                n_show_cmd: i32,
            ) -> isize;
        }
        const SW_SHOWNORMAL: i32 = 1;
        let verb = to_wide_null("open");
        let file = to_wide_null(&installer_shell_execute_path(installer_path));
        let params = to_wide_null(&installer_launch_args().join(" "));
        let result = unsafe {
            ShellExecuteW(
                core::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                params.as_ptr(),
                core::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result <= 32 {
            return Err(format!("ShellExecuteW failed with code {result}."));
        }
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

/// Strips Windows extended-length UNC prefixes (`\\?\` / `\\?\UNC\`) before passing paths to
/// Win32 `ShellExecuteW` or external shell helpers like PowerShell `Expand-Archive`.
fn strip_unc_prefix(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw.into_owned()
    }
}

/// Relaunch step of the portable helper. Packages may ship the exe as `dawsu.exe` while the
/// running copy has an older name: start `dawsu.exe` and remove the old exe then. `-ine` is
/// case-insensitive, so a running `DAWSU.EXE` (the same NTFS file) never deletes itself.
#[cfg(any(target_os = "windows", test))]
const PORTABLE_RELAUNCH_PS: &str = "$newExe = Join-Path $env:DAWSU_DIR 'dawsu.exe'; \
    if ((Test-Path -LiteralPath $newExe) -and ([IO.Path]::GetFileName($env:DAWSU_EXE) -ine 'dawsu.exe')) \
    { Remove-Item -LiteralPath $env:DAWSU_EXE -Force -ErrorAction SilentlyContinue; Start-Process -FilePath $newExe } \
    else { Start-Process -FilePath $env:DAWSU_EXE }";

/// PowerShell text for the portable update helper: wait for `DAWSU_PID` to exit, extract
/// `DAWSU_ZIP` over `DAWSU_DIR`, then relaunch.
#[cfg(any(target_os = "windows", test))]
fn portable_swap_script() -> String {
    format!(
        "$pidToWait = [int]$env:DAWSU_PID; \
        while (Get-Process -Id $pidToWait -ErrorAction SilentlyContinue) {{ Start-Sleep -Milliseconds 200 }}; \
        Start-Sleep -Milliseconds 300; \
        Expand-Archive -LiteralPath $env:DAWSU_ZIP -DestinationPath $env:DAWSU_DIR -Force -ErrorAction Stop; \
        {PORTABLE_RELAUNCH_PS}"
    )
}

/// Spawns a detached helper that waits for this portable process to exit, extracts the verified
/// flat portable ZIP over the portable install directory, and relaunches `dawsu!`.
fn spawn_portable_swap(zip_path: &Path) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not resolve running exe path: {e}"))?;
    let install_dir = exe
        .parent()
        .ok_or("Could not resolve portable install directory.")?;
    let pid = std::process::id();
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = portable_swap_script();
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .env("DAWSU_PID", pid.to_string())
            .env("DAWSU_ZIP", strip_unc_prefix(zip_path))
            .env("DAWSU_DIR", strip_unc_prefix(install_dir))
            .env("DAWSU_EXE", strip_unc_prefix(&exe))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Could not spawn portable update helper: {e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let script = format!(
            "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; unzip -o \"$1\" -d \"$2\" && \"$3\" &"
        );
        Command::new("sh")
            .arg("-c")
            .arg(script)
            .arg("--")
            .arg(zip_path)
            .arg(install_dir)
            .arg(&exe)
            .spawn()
            .map_err(|e| format!("Could not spawn portable update helper: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_is_valid_and_not_the_stale_placeholder() {
        assert!(parse_semver(CURRENT_VERSION).is_some());
        assert_ne!(CURRENT_VERSION, "0.1.0");
        assert!(!is_newer_version(CURRENT_VERSION, CURRENT_VERSION));
    }

    #[test]
    fn release_api_url_lists_releases_for_prerelease_builds_only() {
        assert_eq!(
            release_api_url("1.3.0-alpha.1"),
            "https://api.github.com/repos/YogeGod/dawsu-rs/releases?per_page=10"
        );
        assert_eq!(
            release_api_url("1.3.0"),
            "https://api.github.com/repos/YogeGod/dawsu-rs/releases/latest"
        );
        assert_eq!(MAX_RELEASE_LIST_BYTES, 2 * 1024 * 1024);
    }

    fn release_entry(tag: &str, draft: bool) -> String {
        format!(
            r#"{{"tag_name": "{tag}", "draft": {draft},
                "html_url": "https://github.com/YogeGod/dawsu-rs/releases/tag/{tag}",
                "body": "notes {tag}", "assets": []}}"#
        )
    }

    #[test]
    fn pick_newest_release_skips_drafts_and_bad_tags() {
        // The draft and the bad tag would win if they were not filtered out.
        let list = format!(
            "[{}, {}, {}, {}, {}]",
            release_entry("v9.9.9", true),
            release_entry("v99-not-semver", false),
            release_entry("v1.3.0-alpha.1", false),
            release_entry("v1.3.0-alpha.10", false),
            release_entry("v1.2.0", false),
        );
        let picked = pick_newest_release(&list).expect("a valid release");
        assert_eq!(picked.tag_name, "v1.3.0-alpha.10");
        assert_eq!(picked.version, "1.3.0-alpha.10");
        assert_eq!(picked.notes, "notes v1.3.0-alpha.10");

        // `draft` is optional in the payload.
        let no_draft_field = r#"[{"tag_name": "v1.2.0",
            "html_url": "https://github.com/YogeGod/dawsu-rs/releases/tag/v1.2.0"}]"#;
        assert_eq!(pick_newest_release(no_draft_field).unwrap().version, "1.2.0");

        // Stable release outranks pre-release of the same core version.
        let with_stable = format!(
            "[{}, {}]",
            release_entry("v1.3.0-alpha.10", false),
            release_entry("v1.3.0", false),
        );
        assert_eq!(pick_newest_release(&with_stable).unwrap().version, "1.3.0");
    }

    #[test]
    fn pick_newest_release_errors_clearly_when_nothing_is_usable() {
        let empty = pick_newest_release("[]").expect_err("empty list must fail");
        assert!(empty.contains("No published release"), "{empty}");
        let none_valid = format!(
            "[{}, {}]",
            release_entry("v2.0.0", true),
            release_entry("nightly", false)
        );
        let err = pick_newest_release(&none_valid).expect_err("no usable release must fail");
        assert!(err.contains("No published release"), "{err}");
        assert!(pick_newest_release("{}").is_err());
    }

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
        let (installer_url, portable_url) =
            select_platform_assets(&parsed.assets, platform_token("windows", "x86_64"));
        assert_eq!(
            installer_url.as_deref(),
            Some("https://github.com/YogeGod/dawsu-rs/releases/download/v0.3.0/dawsu-v0.3.0-windows-x64-setup.exe")
        );
        assert_eq!(
            portable_url.as_deref(),
            Some("https://github.com/YogeGod/dawsu-rs/releases/download/v0.3.0/dawsu-v0.3.0-windows-x64-portable.zip")
        );
        // The host-bound parse uses the same filter for the running platform.
        assert_eq!(
            (parsed.installer_url.clone(), parsed.portable_url.clone()),
            select_platform_assets(
                &parsed.assets,
                platform_token(std::env::consts::OS, std::env::consts::ARCH)
            )
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

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            download_url: format!(
                "https://github.com/YogeGod/dawsu-rs/releases/download/v1.3.0/{name}"
            ),
            size: 1,
        }
    }

    #[test]
    fn platform_token_names_the_windows_release_assets() {
        assert_eq!(platform_token("windows", "x86_64"), Some("windows-x64"));
        assert_eq!(platform_token("windows", "aarch64"), None);
        assert_eq!(platform_token("macos", "aarch64"), None);
        assert_eq!(platform_token("linux", "x86_64"), None);
    }

    #[test]
    fn asset_filter_requires_the_platform_token_and_never_picks_stray_files() {
        // Stray files come first so a fallback or first-match bug would pick them.
        let assets = vec![
            asset("foo.exe"),
            asset("foo.zip"),
            asset("tools-setup.exe"),
            asset("extras-portable.zip"),
            asset("dawsu-v1.3.0-windows-x64-setup.exe"),
            asset("dawsu-v1.3.0-windows-x64-portable.zip"),
            asset("SHA256SUMS.txt"),
        ];
        let url = |n: &str| Some(asset(n).download_url);
        assert_eq!(
            select_platform_assets(&assets, platform_token("windows", "x86_64")),
            (
                url("dawsu-v1.3.0-windows-x64-setup.exe"),
                url("dawsu-v1.3.0-windows-x64-portable.zip")
            )
        );
        for (os, arch) in [("macos", "aarch64"), ("linux", "x86_64")] {
            assert_eq!(
                select_platform_assets(&assets, platform_token(os, arch)),
                (None, None),
                "{os}/{arch} must get no package from Windows-only assets"
            );
        }
        let stray_only = vec![asset("foo.exe"), asset("foo.zip")];
        assert_eq!(
            select_platform_assets(&stray_only, platform_token("windows", "x86_64")),
            (None, None)
        );
        assert_eq!(select_platform_assets(&assets, Some("")), (None, None));
    }

    #[test]
    fn bounded_read_enforces_cap_without_unbounded_allocation() {
        assert_eq!(
            read_bounded(std::io::Cursor::new(vec![b'a'; 16]), 16).unwrap(),
            BoundedRead::WithinLimit(vec![b'a'; 16])
        );
        assert_eq!(
            read_bounded(std::io::Cursor::new(vec![b'a'; 17]), 16).unwrap(),
            BoundedRead::Exceeded
        );
        assert_eq!(
            read_bounded(std::io::repeat(b'x'), 1024).unwrap(),
            BoundedRead::Exceeded
        );
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            let child = Command::new("cmd")
                .args(["/C", "for /L %i in (1,1,5000) do @echo 0123456789"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .creation_flags(0x0800_0000)
                .spawn()
                .unwrap();
            let err = collect_bounded_output(child, "GitHub Releases check", true, 32)
                .expect_err("oversized stream must fail");
            assert!(err.contains("larger than allowed"), "{err}");
        }
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

    #[test]
    fn portable_swap_script_relaunches_renamed_exe_case_insensitively() {
        let script = portable_swap_script();
        let expand = script.find("Expand-Archive").expect("extracts the ZIP");
        let relaunch = script.find(PORTABLE_RELAUNCH_PS).expect("ends with the relaunch step");
        assert!(expand < relaunch, "relaunch must run after extraction");
        assert!(script.ends_with(PORTABLE_RELAUNCH_PS));
        assert!(!script.contains('"'), "-Command mangles double quotes");
        for needle in [
            "$newExe = Join-Path $env:DAWSU_DIR 'dawsu.exe'",
            "if ((Test-Path -LiteralPath $newExe) -and ([IO.Path]::GetFileName($env:DAWSU_EXE) -ine 'dawsu.exe'))",
            "Remove-Item -LiteralPath $env:DAWSU_EXE -Force -ErrorAction SilentlyContinue; Start-Process -FilePath $newExe",
            "else { Start-Process -FilePath $env:DAWSU_EXE }",
        ] {
            assert!(PORTABLE_RELAUNCH_PS.contains(needle), "missing: {needle}");
        }
    }

    /// Runs the shipped relaunch step in Windows PowerShell with `Start-Process` swapped for an
    /// echo, against real files: returns (launched path, whether the running exe still exists).
    #[cfg(target_os = "windows")]
    fn run_relaunch_step(tag: &str, running: &str, extracted: &[&str]) -> (String, bool) {
        let dir = temp_test_dir(tag);
        let exe = dir.join(running);
        fs::write(&exe, b"old").unwrap();
        for name in extracted {
            fs::write(dir.join(name), b"new").unwrap();
        }
        let script = PORTABLE_RELAUNCH_PS.replace("Start-Process -FilePath", "Write-Output");
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .env("DAWSU_DIR", &dir)
            .env("DAWSU_EXE", &exe)
            .output()
            .expect("run powershell");
        let still_there = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy() == running);
        let _ = fs::remove_dir_all(&dir);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let launched = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (
            Path::new(&launched)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(launched),
            still_there,
        )
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn portable_relaunch_step_runs_correctly_in_powershell() {
        // Old name: the renamed dawsu.exe replaces it.
        assert_eq!(
            run_relaunch_step("relaunch-old-name", "dawsu-tauri.exe", &["dawsu.exe"]),
            ("dawsu.exe".to_string(), false)
        );
        // Already dawsu.exe: relaunch itself, delete nothing.
        assert_eq!(
            run_relaunch_step("relaunch-same-name", "dawsu.exe", &[]),
            ("dawsu.exe".to_string(), true)
        );
        // DAWSU.EXE is the same NTFS file as dawsu.exe: it must not delete itself.
        assert_eq!(
            run_relaunch_step("relaunch-upper-case", "DAWSU.EXE", &[]),
            ("DAWSU.EXE".to_string(), true)
        );
        // Old package without dawsu.exe: keep and relaunch the running exe.
        assert_eq!(
            run_relaunch_step("relaunch-no-new-exe", "dawsu-tauri.exe", &[]),
            ("dawsu-tauri.exe".to_string(), true)
        );
        // Paths containing spaces, brackets, and single quotes must not break relaunch.
        assert_eq!(
            run_relaunch_step("relaunch [special] 'dir'", "dawsu-tauri.exe", &["dawsu.exe"]),
            ("dawsu.exe".to_string(), false)
        );
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
        assert!(validate_staged_installer_path(&data_root, Path::new(&ok.installer_path)).is_ok());
        assert!(verify_file_sha256(Path::new(&ok.installer_path), &digest).is_ok());

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
        assert!(validate_staged_installer_path(&data_root, &outside).is_err());

        // Filename sanitization rejects drive letters / ADS colons and traversal.
        assert!(validate_safe_filename("C:evil.exe").is_err());
        assert!(validate_safe_filename("foo.exe:bar").is_err());
        assert!(validate_safe_filename("../evil.exe").is_err());
        assert!(require_release_asset_url("https://evil.example/setup.exe", "installer").is_err());
        assert!(
            require_release_asset_url(
                "https://github.com/YogeGod/dawsu-rs/releases/download/v1.0.0/setup.exe",
                "installer",
            )
            .is_ok()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn staged_update_state_and_installed_detection() {
        let root = temp_test_dir("updater-installed-check");
        let data_root = root.join("appdata");
        let ok = verify_and_write_installer(
            &data_root,
            "v1.1.0",
            "dawsu-v1.1.0-windows-x64-setup.exe",
            b"verified-installer-bytes",
            &format!(
                "{}  dawsu-v1.1.0-windows-x64-setup.exe\n",
                sha256_hex(b"verified-installer-bytes")
            ),
        )
        .unwrap();

        let staged = StagedUpdate::default();
        assert!(staged.take_armed().is_none());
        assert!(staged.arm_verified_download(&data_root).is_err());

        staged.record_download(VerifiedDownload {
            version: ok.version.clone(),
            path: PathBuf::from(&ok.installer_path),
            expected_sha256: ok.sha256.clone(),
            kind: UpdatePackageKind::Installer,
        });
        // Not armed yet -> take_armed is None.
        assert!(staged.take_armed().is_none());
        // Arm then clear -> take_armed is None.
        staged.arm_verified_download(&data_root).unwrap();
        staged.clear();
        assert!(staged.take_armed().is_none());
        // Arm then take -> returns once.
        staged.arm_verified_download(&data_root).unwrap();
        assert!(staged.take_armed().is_some());
        assert!(staged.take_armed().is_none());

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
            fs::write(installed_dir.join("unins001.exe"), b"unins").unwrap();
            assert!(is_installed_copy_at(&installed_exe));
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn flat_portable_zip_validator_accepts_flat_and_rejects_traversal() {
        fn build_minimal_zip(entries: &[&str]) -> Vec<u8> {
            let mut out = Vec::new();
            let mut cd = Vec::new();
            for &name in entries {
                let offset = out.len() as u32;
                let nb = name.as_bytes();
                // Local file header
                out.extend_from_slice(b"PK\x03\x04");
                out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                out.extend_from_slice(&[0; 12]);
                out.extend_from_slice(&(nb.len() as u16).to_le_bytes());
                out.extend_from_slice(&0u16.to_le_bytes());
                out.extend_from_slice(nb);
                // Central directory header
                cd.extend_from_slice(b"PK\x01\x02");
                cd.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                cd.extend_from_slice(&[0; 12]);
                cd.extend_from_slice(&(nb.len() as u16).to_le_bytes());
                cd.extend_from_slice(&0u16.to_le_bytes());
                cd.extend_from_slice(&0u16.to_le_bytes());
                cd.extend_from_slice(&[0; 8]);
                cd.extend_from_slice(&offset.to_le_bytes());
                cd.extend_from_slice(nb);
            }
            let cd_offset = out.len() as u32;
            let cd_size = cd.len() as u32;
            out.extend_from_slice(&cd);
            // EOCD
            out.extend_from_slice(b"PK\x05\x06");
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            out.extend_from_slice(&cd_size.to_le_bytes());
            out.extend_from_slice(&cd_offset.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out
        }

        let good = build_minimal_zip(&["dawsu-tauri.exe", "README.txt"]);
        assert_eq!(
            validate_flat_portable_zip(&good).unwrap(),
            vec!["dawsu-tauri.exe", "README.txt"]
        );

        let evil = build_minimal_zip(&["../evil.exe"]);
        assert!(validate_flat_portable_zip(&evil).is_err());

        let deep = build_minimal_zip(&["a/b/c.dll"]);
        assert!(validate_flat_portable_zip(&deep).is_err());

        // Unix symlink (version_made_by OS=3, external attrs S_IFLNK = 0o120000 << 16) must be refused.
        let mut symlink_zip = build_minimal_zip(&["link"]);
        let cd_pos = symlink_zip
            .windows(4)
            .position(|w| w == b"PK\x01\x02")
            .unwrap();
        symlink_zip[cd_pos + 5] = 3;
        let sym_attr = (0o120777u32) << 16;
        symlink_zip[cd_pos + 38..cd_pos + 42].copy_from_slice(&sym_attr.to_le_bytes());
        assert!(
            validate_flat_portable_zip(&symlink_zip)
                .unwrap_err()
                .contains("symlink")
        );

        assert_eq!(strip_unc_prefix(Path::new(r"\\?\C:\Tools\dawsu.exe")), r"C:\Tools\dawsu.exe");
        assert_eq!(strip_unc_prefix(Path::new(r"\\?\UNC\server\share\dawsu.exe")), r"\\server\share\dawsu.exe");
    }

    #[test]
    fn installer_shell_execute_path_strips_extended_unc_prefixes() {
        assert_eq!(
            installer_shell_execute_path(Path::new(
                r"\\?\C:\Users\Alice\AppData\Local\Temp\dawsu-setup.exe"
            )),
            r"C:\Users\Alice\AppData\Local\Temp\dawsu-setup.exe"
        );
        assert_eq!(
            installer_shell_execute_path(Path::new(r"\\?\UNC\fileserver\updates\dawsu setup.exe")),
            r"\\fileserver\updates\dawsu setup.exe"
        );
        assert_eq!(
            installer_shell_execute_path(Path::new(r"C:\Users\Alice\dawsu-setup.exe")),
            r"C:\Users\Alice\dawsu-setup.exe"
        );

        // End to end: the canonicalized staged path (`\\?\` on Windows) reaches ShellExecuteW plain.
        let root = temp_test_dir("updater-shell-execute-path");
        let data_root = root.join("appdata");
        let staged_dir = updates_dir(&data_root).join("1.1.0");
        fs::create_dir_all(&staged_dir).unwrap();
        let staged = staged_dir.join("dawsu-v1.1.0-windows-x64-setup.exe");
        fs::write(&staged, b"MZ").unwrap();
        let canon = validate_staged_installer_path(&data_root, &staged).unwrap();
        let shell_path = installer_shell_execute_path(&canon);
        assert!(!shell_path.starts_with(r"\\?\"), "{shell_path}");
        assert!(shell_path.ends_with("dawsu-v1.1.0-windows-x64-setup.exe"), "{shell_path}");
        assert!(Path::new(&shell_path).is_file(), "{shell_path}");
        let _ = fs::remove_dir_all(&root);
    }
}
