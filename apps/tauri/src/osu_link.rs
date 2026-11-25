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
