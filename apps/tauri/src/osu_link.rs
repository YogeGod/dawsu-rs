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
        && let Some(found) = match_window_title_in_songs(&title, songs_folder) {
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
        && s.is_dir() {
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
                && canon_file.starts_with(&canon_songs) {
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
            if title.starts_with("osu!  - ") || title.starts_with("osu! - ") {
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
    let rest = title
        .strip_prefix("osu!  - ")
        .or_else(|| title.strip_prefix("osu! - "))?;
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
    /// One of `"reloaded_stable"`, `"lazer_external_edit"`, `"no_matching_window"`,
    /// `"focus_blocked"`, or `"unsupported_platform"`.
    pub status: &'static str,
    pub message: String,
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

/// Extracts the difficulty name inside the last `[...]` of a `.osu` filename.
pub fn difficulty_name_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let open = stem.rfind('[')?;
    let close = stem.rfind(']')?;
    if close <= open + 1 {
        return None;
    }
    Some(stem[open + 1..close].trim().to_string())
}

/// True when an osu!stable window title (`osu!  - Artist - Title [Diff]`) matches the saved `.osu`.
pub fn window_title_matches_saved_map(title: &str, saved_path: &Path) -> bool {
    let Some(rest) = title
        .strip_prefix("osu!  - ")
        .or_else(|| title.strip_prefix("osu! - "))
    else {
        return false;
    };
    let Some(open) = rest.rfind('[') else {
        return false;
    };
    let Some(close) = rest.rfind(']') else {
        return false;
    };
    if close <= open + 1 {
        return false;
    }
    let window_diff = rest[open + 1..close].trim();
    let Some(saved_diff) = difficulty_name_from_path(saved_path) else {
        return false;
    };
    window_diff.eq_ignore_ascii_case(&saved_diff)
}

/// Asks a running osu! client to reload `saved_path`:
/// - For osu!lazer `File → Edit externally` mounts, reminds the mapper to click Finish in osu!lazer
///   (lazer's `ExternalEditOperation` has no external IPC to trigger Finish).
/// - On Windows, finds a visible osu!stable window whose title matches the saved difficulty and
///   sends the v3.1.4 `OsuKeys.cs` reload sequence (bring forward, Ctrl+L × 10, Enter).
/// - On macOS/Linux, returns a clear cross-platform message without failing.
pub fn reload_osu_editor(saved_path: &Path) -> OsuReloadResult {
    if is_lazer_external_edit_path(saved_path) {
        return OsuReloadResult {
            status: "lazer_external_edit",
            message: "Saved into osu!lazer's external edit folder. Click Finish in osu!lazer to apply."
                .into(),
        };
    }

    #[cfg(windows)]
    {
        let Some(hwnd) = find_matching_osu_window(saved_path) else {
            return OsuReloadResult {
                status: "no_matching_window",
                message: "No osu! editor window with this difficulty was found.".into(),
            };
        };
        if send_osu_stable_reload(hwnd) {
            OsuReloadResult {
                status: "reloaded_stable",
                message: "Sent Ctrl+L to osu! to reload the saved difficulty.".into(),
            }
        } else {
            OsuReloadResult {
                status: "focus_blocked",
                message: "Could not bring osu! forward. Switch to osu! and press Ctrl+L to reload."
                    .into(),
            }
        }
    }

    #[cfg(not(windows))]
    {
        OsuReloadResult {
            status: "unsupported_platform",
            message: "Automatic editor reload is only available for osu!stable on Windows. In osu!lazer, use File → Edit externally and click Finish."
                .into(),
        }
    }
}

#[cfg(windows)]
fn find_matching_osu_window(saved_path: &Path) -> Option<isize> {
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

    struct Search<'a> {
        saved_path: &'a Path,
        found: Option<isize>,
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
            let search = &mut *(lparam as *mut Search<'_>);
            if window_title_matches_saved_map(&title, search.saved_path) {
                search.found = Some(hwnd);
                return 0;
            }
            1
        }
    }

    let mut search = Search {
        saved_path,
        found: None,
    };
    unsafe {
        EnumWindows(callback, (&mut search as *mut Search<'_>) as isize);
    }
    search.found
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

/// Faithful port of v3.1.4 `OsuKeys.cs`: brings osu!stable forward, presses Ctrl+L repeatedly
/// (osu! reads hardware scan codes and may drop the first keystroke), then confirms with Enter.
#[cfg(windows)]
fn send_osu_stable_reload(hwnd: isize) -> bool {
    use std::mem::{ManuallyDrop, size_of};
    use std::thread;
    use std::time::Duration;

    const VK_CONTROL: u16 = 0x11;
    const VK_L: u16 = 0x4C;
    const VK_RETURN: u16 = 0x0D;
    const KEYEVENTF_KEYUP: u32 = 0x0002;
    const INPUT_KEYBOARD: u32 = 1;
    const SW_RESTORE: i32 = 9;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> isize;
        fn SetForegroundWindow(hwnd: isize) -> i32;
        fn IsIconic(hwnd: isize) -> i32;
        fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        fn MapVirtualKeyW(code: u32, map_type: u32) -> u32;
        fn SendInput(count: u32, inputs: *const WinInput, size: i32) -> u32;
    }

    if hwnd == 0 {
        return false;
    }
    unsafe {
        if GetForegroundWindow() != hwnd {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd) == 0 {
                return false;
            }
            thread::sleep(Duration::from_millis(300));
        }

        let send_key = |vk: u16, up: bool| {
            let scan = MapVirtualKeyW(u32::from(vk), 0) as u16;
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
            SendInput(1, &input, size_of::<WinInput>() as i32);
            thread::sleep(Duration::from_millis(1));
        };

        send_key(VK_CONTROL, false);
        for _ in 0..10 {
            send_key(VK_L, false);
            send_key(VK_L, true);
        }
        send_key(VK_CONTROL, true);
        thread::sleep(Duration::from_millis(100));
        send_key(VK_RETURN, false);
        send_key(VK_RETURN, true);
    }
    true
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

    #[test]
    fn matches_osu_stable_window_title_against_saved_difficulty() {
        let saved = Path::new("E:/osu!/Songs/12345 Song/Artist - Song (Mapper) [Insane].osu");
        assert_eq!(difficulty_name_from_path(saved).as_deref(), Some("Insane"));
        assert!(window_title_matches_saved_map(
            "osu!  - Artist - Song [Insane]",
            saved
        ));
        assert!(window_title_matches_saved_map(
            "osu! - Artist - Song [insane]",
            saved
        ));
        assert!(!window_title_matches_saved_map(
            "osu!  - Artist - Song [Hard]",
            saved
        ));
        assert!(!window_title_matches_saved_map("Notepad", saved));
    }

    #[test]
    fn reload_reports_lazer_external_edit_without_touching_windows() {
        let hex64 = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let path = std::env::temp_dir()
            .join(hex64)
            .join("Artist - Song (Mapper) [Hard].osu");
        let res = reload_osu_editor(&path);
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
