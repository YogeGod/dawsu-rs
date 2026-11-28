use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Finds the beatmap currently open in an osu! client:
/// 1. un!stable / Mapping Tools IPC (`\\.\pipe\mtipc`, Windows)
/// 2. osu!lazer `File -> Edit externally` mounted temp folder (cross-platform)
/// 3. osu!stable window title matched against the Songs folder (Windows)
pub fn find_open_osu_map(songs_folder: Option<&Path>) -> Result<PathBuf, String> {
    #[cfg(windows)]
    if let Some((folder, osu_file)) = try_read_mtipc() {
        return resolve_in_songs(&folder, &osu_file, songs_folder, "osu! (MTIPC)");
    }

    if let Some(lazer_map) = find_lazer_external_edit(&std::env::temp_dir()) {
        return Ok(lazer_map);
    }

    #[cfg(windows)]
    if let Some(title) = find_osu_window_title()
        && let Some(found) = match_window_title_in_songs(&title, songs_folder)
    {
        return Ok(found);
    }

    Err(
        "No open osu! map found. Open a map in un!stable or osu!stable's editor, or use File → Edit externally in osu!lazer."
            .into(),
    )
}

fn candidate_songs_folders(songs_folder: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(s) = songs_folder
        && s.is_dir()
    {
        out.push(s.to_path_buf());
    }
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = Path::new(&local).join("osu!").join("Songs");
        if p.is_dir() && !out.iter().any(|x| x == &p) {
            out.push(p);
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        let p = Path::new(&home).join("Library/Application Support/osu!/Songs");
        if p.is_dir() && !out.iter().any(|x| x == &p) {
            out.push(p);
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(home) = std::env::var_os("HOME") {
        let p = Path::new(&home).join(".local/share/osu/Songs");
        if p.is_dir() && !out.iter().any(|x| x == &p) {
            out.push(p);
        }
    }
    out
}

#[cfg(windows)]
fn resolve_in_songs(
    folder: &str,
    osu_file: &str,
    songs_folder: Option<&Path>,
    client: &str,
) -> Result<PathBuf, String> {
    let candidates = candidate_songs_folders(songs_folder);
    if candidates.is_empty() {
        return Err(format!(
            "Set the osu! Songs folder in Preferences so dawsu! can locate {client}'s map."
        ));
    }
    for songs in &candidates {
        let candidate = songs.join(folder).join(osu_file);
        if candidate.is_file()
            && candidate
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("osu"))
            && let (Ok(canon_songs), Ok(canon_file)) =
                (fs::canonicalize(songs), fs::canonicalize(&candidate))
            && canon_file.starts_with(&canon_songs)
        {
            return Ok(candidate);
        }
    }
    Err(format!(
        "{client} reported '{folder}/{osu_file}', but it wasn't found in your Songs folder. Check Preferences → Folders."
    ))
}

/// Port of v3.1.4 `OsuLazerExternalEdit.cs`: checks temp dir for 64-hex-char folders containing `.osu`.
pub fn find_lazer_external_edit(temp_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(temp_dir).ok()?;
    let mut mounts: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified().or_else(|_| m.created()))
                .unwrap_or(std::time::UNIX_EPOCH);
            mounts.push((mtime, path));
        }
    }
    mounts.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, mount) in mounts {
        let Ok(files) = fs::read_dir(&mount) else {
            continue;
        };
        let mut osu_files: Vec<(std::time::SystemTime, PathBuf)> = files
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("osu"))
            })
            .map(|p| {
                let t = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (t, p)
            })
            .collect();
        if !osu_files.is_empty() {
            osu_files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            return Some(osu_files[0].1.clone());
        }
    }
    None
}

/// Reads a .NET BinaryReader 7-bit-length-prefixed UTF-8 string from `buf`.
pub fn read_dotnet_string(buf: &[u8], cursor: &mut usize) -> Option<String> {
    let mut len: usize = 0;
    let mut shift = 0u32;
    for _ in 0..5 {
        let b = *buf.get(*cursor)?;
        *cursor += 1;
        len |= ((b & 0x7f) as usize) << shift;
        if (b & 0x80) == 0 {
            break;
        }
        shift += 7;
    }
    if len > 4096 {
        return None;
    }
    let slice = buf.get(*cursor..*cursor + len)?;
    *cursor += len;
    String::from_utf8(slice.to_vec()).ok()
}

/// Parses an MTIPC `ReadBeatmap` (message 3) reply: 2 f64 + 4 f32 + folder string + osu_file string.
pub fn parse_mtipc_beatmap(reply: &[u8]) -> Option<(String, String)> {
    // 2 * 8 + 4 * 4 = 32 bytes header
    let mut cursor = 32;
    if reply.len() < cursor {
        return None;
    }
    let folder = read_dotnet_string(reply, &mut cursor)?;
    let osu_file = read_dotnet_string(reply, &mut cursor)?;
    if folder.is_empty() || osu_file.is_empty() {
        return None;
    }
    Some((folder, osu_file))
}

#[cfg(windows)]
fn try_read_mtipc() -> Option<(String, String)> {
    const PIPE_PATH: &str = r"\\.\pipe\mtipc";
    if !Path::new(PIPE_PATH).exists() {
        return None;
    }
    let mut pipe = OpenOptions::new().read(true).write(true).open(PIPE_PATH).ok()?;
    // Handshake: message 0 -> expects i32 1337
    pipe.write_all(&0i32.to_le_bytes()).ok()?;
    pipe.flush().ok()?;
    let mut hello = [0u8; 64];
    let n = pipe.read(&mut hello).ok()?;
    if n < 4 || i32::from_le_bytes([hello[0], hello[1], hello[2], hello[3]]) != 1337 {
        return None;
    }
    // ReadBeatmap: message 3
    pipe.write_all(&3i32.to_le_bytes()).ok()?;
    pipe.flush().ok()?;
    let mut buf = vec![0u8; 8192];
    let n = pipe.read(&mut buf).ok()?;
    parse_mtipc_beatmap(&buf[..n])
}

#[cfg(windows)]
fn find_osu_window_title() -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(
            cb: unsafe extern "system" fn(isize, isize) -> i32,
            lparam: isize,
        ) -> i32;
        fn GetWindowTextW(hwnd: isize, buf: *mut u16, max: i32) -> i32;
        fn IsWindowVisible(hwnd: isize) -> i32;
    }

    unsafe extern "system" fn callback(hwnd: isize, lparam: isize) -> i32 {
        unsafe {
            if IsWindowVisible(hwnd) == 0 {
                return 1;
            }
            let mut buf = [0u16; 512];
            let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if len <= 0 {
                return 1;
            }
            let title = OsString::from_wide(&buf[..len as usize])
                .to_string_lossy()
                .into_owned();
            if strip_osu_window_prefix(&title).is_some() {
                let slot = &mut *(lparam as *mut Option<String>);
                *slot = Some(title);
                return 0;
            }
            1
        }
    }

    let mut found: Option<String> = None;
    unsafe {
        EnumWindows(callback, (&mut found as *mut Option<String>) as isize);
    }
    found
}

#[cfg(windows)]
fn match_window_title_in_songs(title: &str, songs_folder: Option<&Path>) -> Option<PathBuf> {
    // osu! window titles look like: "osu!  - Artist - Title [Difficulty]"
    let rest = strip_osu_window_prefix(title)?;
    let bracket_start = rest.rfind('[')?;
    let bracket_end = rest.rfind(']')?;
    if bracket_end <= bracket_start + 1 {
        return None;
    }
    let diff = rest[bracket_start + 1..bracket_end].trim().to_lowercase();
    let song_part = rest[..bracket_start].trim().to_lowercase();
    let title_token = song_part
        .split(" - ")
        .last()
        .unwrap_or(&song_part)
        .trim()
        .to_string();

    for songs in candidate_songs_folders(songs_folder) {
        let Ok(dirs) = fs::read_dir(&songs) else {
            continue;
        };
        for dir in dirs.flatten() {
            let folder_name = dir.file_name().to_string_lossy().to_lowercase();
            if !title_token.is_empty() && !folder_name.contains(&title_token) {
                continue;
            }
            let Ok(files) = fs::read_dir(dir.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if !path.is_file()
                    || !path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("osu"))
                {
                    continue;
                }
                let fname = file.file_name().to_string_lossy().to_lowercase();
                if fname.contains(&format!("[{diff}]")) {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Outcome of asking osu! to reload a saved beatmap (F011).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsuReloadResult {
    /// One of `"reloaded_stable"`, `"lazer_external_edit"`, `"no_matching_window"`, `"ambiguous"`,
    /// `"title_mismatch"`, `"focus_blocked"`, or `"unsupported_platform"`.
    pub status: &'static str,
    pub message: String,
}

fn reload_result(status: &'static str, message: impl Into<String>) -> OsuReloadResult {
    OsuReloadResult {
        status,
        message: message.into(),
    }
}

/// The saved difficulty's `[Metadata]` fields an osu!stable window title is matched against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedMapIdentity {
    pub artist: String,
    pub title: String,
    pub artist_unicode: String,
    pub title_unicode: String,
    pub version: String,
    pub creator: String,
}

impl SavedMapIdentity {
    pub fn from_document(document: &crate::Document) -> Self {
        let value = |key| document.file.value("Metadata", key).unwrap_or_default();
        Self {
            artist: value("Artist"),
            title: value("Title"),
            artist_unicode: value("ArtistUnicode"),
            title_unicode: value("TitleUnicode"),
            version: value("Version"),
            creator: value("Creator"),
        }
    }
}

/// True when `path` sits inside an osu!lazer `File → Edit externally` mounted temp folder
/// (`ExternalEditOperation.cs`: 64-hex-char directory inside the OS temp directory).
pub fn is_lazer_external_edit_path(path: &Path) -> bool {
    is_lazer_external_edit_in_temp(path, &std::env::temp_dir())
}

pub fn is_lazer_external_edit_in_temp(path: &Path, temp_dir: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    let Some(folder_name) = parent.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if folder_name.len() != 64 || !folder_name.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let Some(grandparent) = parent.parent() else {
        return false;
    };
    if grandparent == temp_dir {
        return true;
    }
    if let (Ok(a), Ok(b)) = (fs::canonicalize(grandparent), fs::canonicalize(temp_dir)) {
        return a == b;
    }
    false
}

/// Lower case, without the characters Windows forbids in file names, runs of spaces collapsed:
/// an osu!stable window title and the `.osu` file name it came from then compare equal.
fn normalize_title_part(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'))
        .collect();
    kept.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The text after an osu!stable (`osu!  - `) or un!stable (`un!stable <build> - `) title prefix.
fn strip_osu_window_prefix(title: &str) -> Option<&str> {
    title
        .strip_prefix("osu!  - ")
        .or_else(|| title.strip_prefix("osu! - "))
        .or_else(|| {
            let (build, rest) = title.strip_prefix("un!stable ")?.split_once(" - ")?;
            (!build.is_empty() && !build.contains(char::is_whitespace)).then_some(rest)
        })
}

/// True when an osu!stable window title (`osu!  - Artist - Title [Diff]`, or
/// `Artist - Title (Creator) [Diff].osu` after the prefix) names the saved difficulty: the final
/// `[...]` equals `Version`, and the text before it is `Artist - Title` in its romanised or Unicode
/// form, optionally followed by one `(creator)` group.
pub fn window_title_matches_identity(window_title: &str, id: &SavedMapIdentity) -> bool {
    let version = normalize_title_part(&id.version);
    let artist = normalize_title_part(&id.artist);
    let title = normalize_title_part(&id.title);
    if version.is_empty() || (artist.is_empty() && title.is_empty()) {
        return false;
    }
    let Some(rest) = strip_osu_window_prefix(window_title) else {
        return false;
    };
    let rest = rest.trim_end();
    let rest = match rest.len().checked_sub(4) {
        Some(cut) if rest.is_char_boundary(cut) && rest[cut..].eq_ignore_ascii_case(".osu") => {
            &rest[..cut]
        }
        _ => rest,
    };
    let (Some(open), Some(close)) = (rest.rfind('['), rest.rfind(']')) else {
        return false;
    };
    if close < open || !rest[close + 1..].trim().is_empty() {
        return false;
    }
    if normalize_title_part(&rest[open + 1..close]) != version {
        return false;
    }
    let song = normalize_title_part(&rest[..open]);
    // Each missing Unicode field falls back to its romanised one on its own.
    let or_romanised = |unicode: &str, romanised: &str| {
        let unicode = normalize_title_part(unicode);
        if unicode.is_empty() {
            romanised.to_string()
        } else {
            unicode
        }
    };
    let pairs = [
        (artist.clone(), title.clone()),
        (
            or_romanised(&id.artist_unicode, &artist),
            or_romanised(&id.title_unicode, &title),
        ),
    ];
    let saved_creator = normalize_title_part(&id.creator);
    pairs.iter().any(|(artist, title)| {
        let expected = normalize_title_part(&format!("{artist} - {title}"));
        song.strip_prefix(expected.as_str()).is_some_and(|tail| {
            let tail = tail.trim();
            tail.is_empty()
                || tail
                    .strip_prefix('(')
                    .and_then(|t| t.strip_suffix(')'))
                    .is_some_and(|creator| {
                        // A shown creator must be the saved one: same song and difficulty name
                        // from another mapper's mapset gets no keys.
                        !creator.contains(['(', ')'])
                            && (saved_creator.is_empty() || creator.trim() == saved_creator)
                    })
        })
    })
}

/// Asks a running osu! client to reload `saved_path`:
/// - For osu!lazer `File → Edit externally` mounts, reminds the mapper to click Finish in osu!lazer
///   (lazer's `ExternalEditOperation` has no external IPC to trigger Finish).
/// - On Windows, finds the one visible osu!stable window whose title names `identity` and sends
///   the v3.1.4 `OsuKeys.cs` reload sequence (bring forward, Ctrl+L × 10, Enter), checking the
///   foreground window and its title again before any key and before Enter.
/// - On macOS/Linux, returns a clear cross-platform message without failing.
pub fn reload_osu_editor(saved_path: &Path, identity: &SavedMapIdentity) -> OsuReloadResult {
    if is_lazer_external_edit_path(saved_path) {
        return reload_result(
            "lazer_external_edit",
            "Saved into osu!lazer's external edit folder. Click Finish in osu!lazer to apply.",
        );
    }

    #[cfg(windows)]
    {
        reload_stable_with(&mut Win32Windows, identity)
    }

    #[cfg(not(windows))]
    {
        let _ = identity;
        reload_result(
            "unsupported_platform",
            "Automatic editor reload is only available for osu!stable on Windows. In osu!lazer, use File → Edit externally and click Finish.",
        )
    }
}

const VK_CONTROL: u16 = 0x11;
const VK_L: u16 = 0x4C;
const VK_RETURN: u16 = 0x0D;

/// The window reads and key sends the osu!stable reload needs, so tests can fake them.
#[cfg_attr(not(windows), allow(dead_code))]
trait OsuWindowSystem {
    /// Visible top-level windows with their titles.
    fn windows(&mut self) -> Vec<(isize, String)>;
    fn foreground(&mut self) -> isize;
    fn title(&mut self, hwnd: isize) -> Option<String>;
    /// Restores a minimised window and asks to bring it forward; false when refused.
    fn bring_forward(&mut self, hwnd: isize) -> bool;
    fn sleep_ms(&mut self, ms: u64);
    fn key(&mut self, vk: u16, up: bool);
}

/// Holds Ctrl down while alive and releases it on drop, so no exit path (focus loss, panic)
/// leaves Ctrl stuck.
#[cfg_attr(not(windows), allow(dead_code))]
struct CtrlGuard<'a, S: OsuWindowSystem>(&'a mut S);

#[cfg_attr(not(windows), allow(dead_code))]
impl<'a, S: OsuWindowSystem> CtrlGuard<'a, S> {
    fn press(sys: &'a mut S) -> Self {
        let guard = Self(sys);
        guard.0.key(VK_CONTROL, false);
        guard
    }
}

impl<S: OsuWindowSystem> Drop for CtrlGuard<'_, S> {
    fn drop(&mut self) {
        self.0.key(VK_CONTROL, true);
    }
}

/// `Some` when the foreground window is no longer `hwnd` or its title no longer names the map.
#[cfg_attr(not(windows), allow(dead_code))]
fn recheck_osu_window<S: OsuWindowSystem>(
    sys: &mut S,
    hwnd: isize,
    id: &SavedMapIdentity,
    unsent: &str,
) -> Option<OsuReloadResult> {
    if sys.foreground() != hwnd {
        return Some(reload_result(
            "focus_blocked",
            format!("Could not bring osu! forward. Switch to osu! and press Ctrl+L to reload. {unsent}"),
        ));
    }
    let title = sys.title(hwnd).unwrap_or_default();
    if window_title_matches_identity(&title, id) {
        return None;
    }
    let shown = strip_osu_window_prefix(&title).unwrap_or(&title).trim();
    Some(reload_result(
        "title_mismatch",
        format!("osu! has another map open: {shown}. {unsent}"),
    ))
}

/// Faithful port of v3.1.4 `OsuKeys.cs`: brings osu!stable forward, presses Ctrl+L repeatedly
/// (osu! reads hardware scan codes and may drop the first keystroke), then confirms with Enter.
#[cfg_attr(not(windows), allow(dead_code))]
fn reload_stable_with<S: OsuWindowSystem>(sys: &mut S, id: &SavedMapIdentity) -> OsuReloadResult {
    let matching: Vec<isize> = sys
        .windows()
        .into_iter()
        .filter(|(hwnd, title)| *hwnd != 0 && window_title_matches_identity(title, id))
        .map(|(hwnd, _)| hwnd)
        .collect();
    let hwnd = match matching.as_slice() {
        [] => {
            return reload_result(
                "no_matching_window",
                "No osu! editor window has this map open. Nothing was sent.",
            );
        }
        [one] => *one,
        _ => {
            return reload_result(
                "ambiguous",
                "More than one osu! window shows this difficulty. Switch to the right one and press Ctrl+L. Nothing was sent.",
            );
        }
    };
    if sys.foreground() != hwnd && !sys.bring_forward(hwnd) {
        return reload_result(
            "focus_blocked",
            "Could not bring osu! forward. Switch to osu! and press Ctrl+L to reload. Nothing was sent.",
        );
    }
    sys.sleep_ms(300);
    if let Some(refused) = recheck_osu_window(sys, hwnd, id, "Nothing was sent.") {
        return refused;
    }
    {
        let ctrl = CtrlGuard::press(sys);
        for _ in 0..10 {
            if ctrl.0.foreground() != hwnd {
                return reload_result(
                    "focus_blocked",
                    "osu! lost focus during the reload. Switch to osu! and press Ctrl+L to reload.",
                );
            }
            ctrl.0.key(VK_L, false);
            ctrl.0.key(VK_L, true);
        }
    }
    sys.sleep_ms(100);
    if let Some(refused) = recheck_osu_window(sys, hwnd, id, "Ctrl+L was sent, Enter was not.") {
        return refused;
    }
    sys.key(VK_RETURN, false);
    sys.key(VK_RETURN, true);
    reload_result(
        "reloaded_stable",
        "Sent Ctrl+L to osu! to reload the saved difficulty.",
    )
}

#[cfg(windows)]
#[repr(C)]
struct WinKeybdInput {
    w_vk: u16,
    w_scan: u16,
    dw_flags: u32,
    time: u32,
    dw_extra_info: usize,
}

#[cfg(windows)]
#[repr(C)]
struct WinMouseInputPadding {
    dx: i32,
    dy: i32,
    mouse_data: u32,
    dw_flags: u32,
    time: u32,
    dw_extra_info: usize,
}

#[cfg(windows)]
#[repr(C)]
union WinInputUnion {
    ki: std::mem::ManuallyDrop<WinKeybdInput>,
    _mi: std::mem::ManuallyDrop<WinMouseInputPadding>,
}

#[cfg(windows)]
#[repr(C)]
struct WinInput {
    r#type: u32,
    u: WinInputUnion,
}

#[cfg(windows)]
mod win32 {
    use super::WinInput;

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn EnumWindows(cb: unsafe extern "system" fn(isize, isize) -> i32, lparam: isize) -> i32;
        pub fn GetWindowTextW(hwnd: isize, buf: *mut u16, max: i32) -> i32;
        pub fn IsWindowVisible(hwnd: isize) -> i32;
        pub fn GetForegroundWindow() -> isize;
        pub fn SetForegroundWindow(hwnd: isize) -> i32;
        pub fn IsIconic(hwnd: isize) -> i32;
        pub fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        pub fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
        pub fn SendInput(count: u32, inputs: *const WinInput, size: i32) -> u32;
    }

    pub fn window_text(hwnd: isize) -> Option<String> {
        let mut buf = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        (len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

#[cfg(windows)]
struct Win32Windows;

#[cfg(windows)]
impl OsuWindowSystem for Win32Windows {
    fn windows(&mut self) -> Vec<(isize, String)> {
        // Only collects; matching runs in safe Rust after EnumWindows returns.
        unsafe extern "system" fn collect(hwnd: isize, lparam: isize) -> i32 {
            unsafe {
                if win32::IsWindowVisible(hwnd) != 0
                    && let Some(title) = win32::window_text(hwnd)
                {
                    (*(lparam as *mut Vec<(isize, String)>)).push((hwnd, title));
                }
            }
            1
        }
        let mut found: Vec<(isize, String)> = Vec::new();
        unsafe {
            win32::EnumWindows(collect, (&mut found as *mut Vec<(isize, String)>) as isize);
        }
        found
    }

    fn foreground(&mut self) -> isize {
        unsafe { win32::GetForegroundWindow() }
    }

    fn title(&mut self, hwnd: isize) -> Option<String> {
        win32::window_text(hwnd)
    }

    fn bring_forward(&mut self, hwnd: isize) -> bool {
        const SW_RESTORE: i32 = 9;
        unsafe {
            if win32::IsIconic(hwnd) != 0 {
                win32::ShowWindow(hwnd, SW_RESTORE);
            }
            win32::SetForegroundWindow(hwnd) != 0
        }
    }

    fn sleep_ms(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    fn key(&mut self, vk: u16, up: bool) {
        use std::mem::{ManuallyDrop, size_of};
        const KEYEVENTF_KEYUP: u32 = 0x0002;
        const INPUT_KEYBOARD: u32 = 1;
        unsafe {
            let scan = win32::MapVirtualKeyW(u32::from(vk), 0) as u16;
            let input = WinInput {
                r#type: INPUT_KEYBOARD,
                u: WinInputUnion {
                    ki: ManuallyDrop::new(WinKeybdInput {
                        w_vk: vk,
                        w_scan: scan,
                        dw_flags: if up { KEYEVENTF_KEYUP } else { 0 },
                        time: 0,
                        dw_extra_info: 0,
                    }),
                },
            };
            win32::SendInput(1, &input, size_of::<WinInput>() as i32);
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_osu_lazer_external_edit_paths_only_for_64_hex_folders_in_temp() {
        let temp = Path::new("C:/Temp/dawsu-test");
        let hex64 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let lazer_map = temp.join(hex64).join("Artist - Song (Mapper) [Hard].osu");
        assert!(is_lazer_external_edit_in_temp(&lazer_map, temp));

        let short_hex = temp.join("0123456789abcdef").join("Hard.osu");
        assert!(!is_lazer_external_edit_in_temp(&short_hex, temp));

        let regular = Path::new("E:/osu!/Songs/12345 Song/Artist - Song [Hard].osu");
        assert!(!is_lazer_external_edit_in_temp(regular, temp));
    }

    fn identity(artist: &str, title: &str, version: &str) -> SavedMapIdentity {
        SavedMapIdentity {
            artist: artist.into(),
            title: title.into(),
            version: version.into(),
            ..SavedMapIdentity::default()
        }
    }

    #[test]
    fn osu_link_matches_window_title_on_artist_title_and_difficulty() {
        let id = identity("Artist", "Song", "Insane");
        let matches = |t: &str| window_title_matches_identity(t, &id);
        assert!(matches("osu!  - Artist - Song [Insane]"));
        assert!(matches("osu! - Artist - Song [insane]"));
        assert!(matches("osu!  - Artist - Song (Mapper) [Insane].osu"));
        assert!(matches("osu!  - artist  -  song   [ Insane ]"));
        assert!(!matches("osu!  - Artist - Song [Hard]"));
        assert!(!matches("osu!  - Other - Song [Insane]"));
        assert!(!matches("osu!  - Artist - Other [Insane]"));
        assert!(!matches("osu!  - Artist - Song extra [Insane]"));
        assert!(!matches("osu!  - Artist - Song [Insane] - Notepad"));
        // Only one (creator) group may follow: a second group names another song.
        assert!(!matches("osu!  - Artist - Song (TV Size) (Mapper) [Insane].osu"));
        assert!(!matches("osu!  - Artist - Song (a) b (c) [Insane]"));
        assert!(!matches("Notepad"));
    }

    #[test]
    fn osu_link_matches_un_stable_window_titles() {
        let id = identity("Jakub Zytecki", "Satya's Diary (feat. Piotr Gibner)", "a");
        let matches = |t: &str| window_title_matches_identity(t, &id);
        // Live title of an un!stable editor (2026-11-28).
        assert!(matches(
            "un!stable master-6625bea - Jakub Zytecki - Satya's Diary (feat. Piotr Gibner) (nhlx) [a].osu"
        ));
        assert!(!matches(
            "un!stable master-6625bea - Jakub Zytecki - Satya's Diary (feat. Piotr Gibner) (nhlx) [b].osu"
        ));
        // Same song and difficulty name, another mapper's mapset: no keys.
        let with_creator = SavedMapIdentity { creator: "nhlx".into(), ..id.clone() };
        assert!(window_title_matches_identity(
            "un!stable master-6625bea - Jakub Zytecki - Satya's Diary (feat. Piotr Gibner) (NHLX) [a].osu",
            &with_creator
        ));
        assert!(!window_title_matches_identity(
            "un!stable master-6625bea - Jakub Zytecki - Satya's Diary (feat. Piotr Gibner) (someone) [a].osu",
            &with_creator
        ));
        assert!(!window_title_matches_identity(
            "osu!  - Jakub Zytecki - Satya's Diary (feat. Piotr Gibner) (someone) [a].osu",
            &with_creator
        ));
        assert!(!matches(
            "un!stable - Jakub Zytecki - Satya's Diary (feat. Piotr Gibner) (nhlx) [a].osu"
        ));
        assert!(!matches(
            "un!stable two words - Jakub Zytecki - Satya's Diary (feat. Piotr Gibner) (nhlx) [a].osu"
        ));
    }

    #[test]
    fn osu_link_matches_song_titles_holding_brackets_on_the_final_difficulty_group() {
        let id = identity("Artist", "Song [TV Size]", "Insane");
        assert!(window_title_matches_identity(
            "osu!  - Artist - Song [TV Size] [Insane]",
            &id
        ));
        assert!(window_title_matches_identity(
            "osu!  - Artist - Song [TV Size] (Mapper) [Insane].osu",
            &id
        ));
        assert!(!window_title_matches_identity(
            "osu!  - Artist - Song [TV Size]",
            &id
        ));
    }

    #[test]
    fn osu_link_matches_unicode_and_mixed_romanised_unicode_pairs() {
        let full = SavedMapIdentity {
            artist_unicode: "アーティスト".into(),
            title_unicode: "歌".into(),
            ..identity("Artist", "Song", "Insane")
        };
        assert!(window_title_matches_identity("osu!  - アーティスト - 歌 [Insane]", &full));
        assert!(window_title_matches_identity("osu!  - Artist - Song [Insane]", &full));
        // Both Unicode fields are set, so a half-Unicode title names neither pair.
        assert!(!window_title_matches_identity("osu!  - アーティスト - Song [Insane]", &full));

        let title_only = SavedMapIdentity {
            title_unicode: "歌".into(),
            ..identity("Artist", "Song", "Insane")
        };
        assert!(window_title_matches_identity("osu!  - Artist - 歌 [Insane]", &title_only));
        let artist_only = SavedMapIdentity {
            artist_unicode: "アーティスト".into(),
            ..identity("Artist", "Song", "Insane")
        };
        assert!(window_title_matches_identity("osu!  - アーティスト - Song [Insane]", &artist_only));
        assert!(!window_title_matches_identity("osu!  - アーティスト - 歌 [Insane]", &artist_only));
    }

    #[test]
    fn osu_link_ignores_characters_windows_forbids_in_file_names() {
        let id = identity("Artist", "Re:Zero? *Start*", "Insane");
        assert!(window_title_matches_identity(
            "osu!  - Artist - Re:Zero? *Start* [Insane]",
            &id
        ));
        assert!(window_title_matches_identity(
            "osu!  - Artist - ReZero Start (Mapper) [Insane].osu",
            &id
        ));
    }

    #[test]
    fn osu_link_refuses_to_match_empty_metadata() {
        let any = "osu!  - Artist - Song [Insane]";
        assert!(!window_title_matches_identity(any, &identity("Artist", "Song", "")));
        assert!(!window_title_matches_identity(any, &identity("Artist", "Song", "???")));
        assert!(!window_title_matches_identity(
            "osu!  -  -  [Insane]",
            &identity("", "", "Insane")
        ));
        assert!(!window_title_matches_identity(any, &identity("", "", "Insane")));
    }

    const OSU: isize = 7;
    const OTHER: isize = 9;

    /// A scripted window system: records every key, and can change the foreground window or
    /// osu!'s title at the first sleep (the 300 ms wait after focus) or after a number of keys.
    #[derive(Default)]
    struct FakeWindows {
        windows: Vec<(isize, String)>,
        foreground: isize,
        refuse_focus: bool,
        at_first_sleep: Option<(isize, Option<String>)>,
        lose_focus_after_keys: Option<usize>,
        panic_on_l: bool,
        keys: Vec<(u16, bool)>,
    }

    impl FakeWindows {
        fn with_osu(title: &str) -> Self {
            Self {
                windows: vec![(OTHER, "Notepad".into()), (OSU, title.into())],
                foreground: OTHER,
                ..Self::default()
            }
        }
        fn count(&self, vk: u16, up: bool) -> usize {
            self.keys.iter().filter(|k| **k == (vk, up)).count()
        }
    }

    impl OsuWindowSystem for FakeWindows {
        fn windows(&mut self) -> Vec<(isize, String)> {
            self.windows.clone()
        }
        fn foreground(&mut self) -> isize {
            self.foreground
        }
        fn title(&mut self, hwnd: isize) -> Option<String> {
            self.windows.iter().find(|w| w.0 == hwnd).map(|w| w.1.clone())
        }
        fn bring_forward(&mut self, hwnd: isize) -> bool {
            if self.refuse_focus {
                return false;
            }
            self.foreground = hwnd;
            true
        }
        fn sleep_ms(&mut self, _ms: u64) {
            if let Some((foreground, title)) = self.at_first_sleep.take() {
                self.foreground = foreground;
                if let Some(title) = title {
                    self.windows.retain(|w| w.0 != OSU);
                    self.windows.push((OSU, title));
                }
            }
        }
        fn key(&mut self, vk: u16, up: bool) {
            if self.panic_on_l && vk == VK_L {
                panic!("SendInput failed");
            }
            self.keys.push((vk, up));
            if self.lose_focus_after_keys.is_some_and(|n| self.keys.len() >= n) {
                self.foreground = OTHER;
            }
        }
    }

    #[test]
    fn osu_link_reload_sends_ctrl_l_ten_times_then_enter_to_the_matching_window() {
        let mut sys = FakeWindows::with_osu("osu!  - Artist - Song [Insane]");
        let res = reload_stable_with(&mut sys, &identity("Artist", "Song", "Insane"));
        assert_eq!(res.status, "reloaded_stable");
        let mut expected = vec![(VK_CONTROL, false)];
        for _ in 0..10 {
            expected.extend([(VK_L, false), (VK_L, true)]);
        }
        expected.extend([(VK_CONTROL, true), (VK_RETURN, false), (VK_RETURN, true)]);
        assert_eq!(sys.keys, expected);
    }

    #[test]
    fn osu_link_reload_sends_nothing_when_the_foreground_changes_after_focus() {
        let mut sys = FakeWindows::with_osu("osu!  - Artist - Song [Insane]");
        sys.at_first_sleep = Some((OTHER, None));
        let res = reload_stable_with(&mut sys, &identity("Artist", "Song", "Insane"));
        assert_eq!(res.status, "focus_blocked");
        assert!(sys.keys.is_empty(), "keys: {:?}", sys.keys);
    }

    #[test]
    fn osu_link_reload_sends_nothing_when_osu_shows_another_map_after_focus() {
        let mut sys = FakeWindows::with_osu("osu!  - Artist - Song [Insane]");
        sys.at_first_sleep = Some((OSU, Some("osu!  - Someone - Else [Insane]".into())));
        let res = reload_stable_with(&mut sys, &identity("Artist", "Song", "Insane"));
        assert_eq!(res.status, "title_mismatch");
        assert_eq!(
            res.message,
            "osu! has another map open: Someone - Else [Insane]. Nothing was sent."
        );
        assert!(sys.keys.is_empty(), "keys: {:?}", sys.keys);
    }

    #[test]
    fn osu_link_reload_refuses_two_matching_windows_and_reports_no_match() {
        let mut sys = FakeWindows::with_osu("osu!  - Artist - Song [Insane]");
        sys.windows.push((11, "osu! - Artist - Song (Mapper) [Insane].osu".into()));
        let res = reload_stable_with(&mut sys, &identity("Artist", "Song", "Insane"));
        assert_eq!(res.status, "ambiguous");
        assert!(sys.keys.is_empty());

        let mut sys = FakeWindows::with_osu("osu!  - Artist - Song [Hard]");
        let res = reload_stable_with(&mut sys, &identity("Artist", "Song", "Insane"));
        assert_eq!(res.status, "no_matching_window");
        assert!(sys.keys.is_empty());
    }

    #[test]
    fn osu_link_reload_sends_nothing_when_focus_is_refused() {
        let mut sys = FakeWindows::with_osu("osu!  - Artist - Song [Insane]");
        sys.refuse_focus = true;
        let res = reload_stable_with(&mut sys, &identity("Artist", "Song", "Insane"));
        assert_eq!(res.status, "focus_blocked");
        assert!(sys.keys.is_empty());
    }

    #[test]
    fn osu_link_reload_releases_ctrl_and_skips_enter_when_focus_is_lost_mid_sequence() {
        let mut sys = FakeWindows::with_osu("osu!  - Artist - Song [Insane]");
        sys.lose_focus_after_keys = Some(4);
        let res = reload_stable_with(&mut sys, &identity("Artist", "Song", "Insane"));
        assert_eq!(res.status, "focus_blocked");
        assert_eq!(sys.count(VK_CONTROL, false), 1);
        assert_eq!(sys.count(VK_CONTROL, true), 1);
        assert_eq!(sys.keys.last(), Some(&(VK_CONTROL, true)));
        assert_eq!(sys.count(VK_RETURN, false), 0);
    }

    #[test]
    fn osu_link_reload_releases_ctrl_when_a_key_send_panics() {
        let mut sys = FakeWindows::with_osu("osu!  - Artist - Song [Insane]");
        sys.panic_on_l = true;
        let id = identity("Artist", "Song", "Insane");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reload_stable_with(&mut sys, &id)
        }));
        assert!(outcome.is_err());
        assert_eq!(sys.keys, vec![(VK_CONTROL, false), (VK_CONTROL, true)]);
    }

    #[test]
    fn reload_reports_lazer_external_edit_without_touching_windows() {
        let hex64 = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let path = std::env::temp_dir()
            .join(hex64)
            .join("Artist - Song (Mapper) [Hard].osu");
        let res = reload_osu_editor(&path, &identity("Artist", "Song", "Hard"));
        assert_eq!(res.status, "lazer_external_edit");
        assert!(res.message.contains("Finish"));
    }

    #[cfg(windows)]
    #[test]
    fn win_input_struct_matches_win32_abi_size() {
        let expected = if cfg!(target_pointer_width = "64") {
            40
        } else {
            28
        };
        assert_eq!(std::mem::size_of::<WinInput>(), expected);
    }
}
