//! Fast Songs folder index: reads osu!'s binary `osu!.db` when present next to
//! the configured Songs folder, and falls back to a header-only `.osu` scan.

use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// One `.osu` difficulty inside a mapset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongDiffEntry {
    pub version: String,
    pub path: String,
    pub mode: u8,
}

/// One mapset folder grouping all of its difficulties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongMapsetEntry {
    pub set_id: i32,
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub tags: String,
    pub folder: String,
    pub diffs: Vec<SongDiffEntry>,
}

/// Search response returned to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongsSearchResult {
    pub total_mapsets: usize,
    pub total_diffs: usize,
    pub source: &'static str,
    pub matches: Vec<SongMapsetEntry>,
}

/// Build the Songs folder index, using `<songs_dir>/../osu!.db` when available
/// and falling back to a header-only `.osu` scan otherwise.
pub fn build_songs_index(songs_dir: &Path) -> Vec<SongMapsetEntry> {
    build_songs_index_with_source(songs_dir).0
}

/// Same as [`build_songs_index`], also reporting which source populated it
/// (`"osu!.db"` or `"folder-scan"`).
pub fn build_songs_index_with_source(songs_dir: &Path) -> (Vec<SongMapsetEntry>, &'static str) {
    if let Some(db_path) = songs_dir.parent().map(|parent| parent.join("osu!.db"))
        && let Ok(bytes) = fs::read(&db_path)
            && let Ok(entries) = parse_osu_db(&bytes, songs_dir)
                && !entries.is_empty() {
                    return (entries, "osu!.db");
                }
    (scan_songs_folder_fallback(songs_dir), "folder-scan")
}

/// Parse osu!'s binary `osu!.db` beatmap table (v20140609 through v2025+) and
/// resolve difficulty paths against `songs_dir`, skipping missing folders.
pub fn parse_osu_db(bytes: &[u8], songs_dir: &Path) -> Result<Vec<SongMapsetEntry>, String> {
    let mut reader = DbReader::new(bytes);
    let version = reader.read_i32()?;
    let _folder_count = reader.read_i32()?;
    let _account_unlocked = reader.read_u8()?;
    let _unlock_date = reader.read_i64()?;
    let _player_name = reader.read_osu_string()?;
    let beatmap_count = reader.read_i32()?;
    if beatmap_count < 0 {
        return Err("Negative beatmap count in osu!.db".into());
    }

    // Build a case-insensitive map of existing subfolders in `songs_dir` once so
    // we avoid per-beatmap filesystem syscalls and handle case-sensitive OSes.
    let existing_folders = existing_subfolders(songs_dir);
    let mut groups: HashMap<String, SongMapsetEntry> = HashMap::new();

    for _ in 0..beatmap_count {
        if version < 20191106 {
            let _entry_size = reader.read_i32()?;
        }

        let artist = reader.read_osu_string()?;
        let _artist_unicode = reader.read_osu_string()?;
        let title = reader.read_osu_string()?;
        let _title_unicode = reader.read_osu_string()?;
        let creator = reader.read_osu_string()?;
        let diff_version = reader.read_osu_string()?;
        let _audio_filename = reader.read_osu_string()?;
        let _md5_hash = reader.read_osu_string()?;
        let osu_filename = reader.read_osu_string()?;

        // ranked_status(1) + hitcircle(2) + slider(2) + spinner(2) + last_modified(8)
        reader.skip(15)?;
        // AR, CS, HP, OD
        if version < 20140609 {
            reader.skip(4)?;
        } else {
            reader.skip(16)?;
        }
        // slider_velocity (f64)
        reader.skip(8)?;

        if version >= 20140609 {
            // 4 star-rating Int-Double / Int-Float dictionaries (osu, taiko, catch, mania).
            for _ in 0..4 {
                reader.skip_star_rating_dict()?;
            }
        }

        // drain_time(4) + total_time(4) + preview_time(4)
        reader.skip(12)?;
        // timing points: count * (f64 + f64 + u8 = 17 bytes)
        let timing_count = reader.read_i32()?;
        if timing_count < 0 {
            return Err("Negative timing point count in osu!.db".into());
        }
        let timing_bytes = (timing_count as usize)
            .checked_mul(17)
            .ok_or_else(|| "Timing point count overflow in osu!.db".to_string())?;
        reader.skip(timing_bytes)?;

        let _beatmap_id = reader.read_i32()?;
        let set_id = reader.read_i32()?;
        let _thread_id = reader.read_i32()?;
        // 4 grades(4) + local_offset(2) + stack_leniency(4)
        reader.skip(10)?;
        let mode = reader.read_u8()?;

        let _song_source = reader.read_osu_string()?;
        let tags = reader.read_osu_string()?;
        // online_offset(2)
        reader.skip(2)?;
        let _title_font = reader.read_osu_string()?;
        // unplayed(1) + last_played(8) + is_osz2(1)
        reader.skip(10)?;
        let folder_name = reader.read_osu_string()?;
        // last_checked(8) + ignore_sound(1) + ignore_skin(1) + disable_storyboard(1)
        // + disable_video(1) + visual_override(1)
        reader.skip(13)?;
        if version < 20140609 {
            reader.skip(2)?;
        }
        // last_modified_int(4) + mania_scroll_speed(1)
        reader.skip(5)?;

        let folder_trimmed = folder_name.trim();
        let file_trimmed = osu_filename.trim();
        if folder_trimmed.is_empty() || file_trimmed.is_empty() {
            continue;
        }
        let folder_key = folder_trimmed.to_lowercase();
        let Some(actual_folder) = existing_folders.get(&folder_key) else {
            continue;
        };
        let diff_path = actual_folder.join(file_trimmed);
        let path_str = diff_path.to_string_lossy().into_owned();

        let entry = groups.entry(folder_key).or_insert_with(|| SongMapsetEntry {
            set_id,
            artist: artist.clone(),
            title: title.clone(),
            creator: creator.clone(),
            tags: tags.clone(),
            folder: folder_trimmed.to_string(),
            diffs: Vec::new(),
        });
        if entry.set_id <= 0 && set_id > 0 {
            entry.set_id = set_id;
        }
        if entry.artist.is_empty() && !artist.is_empty() {
            entry.artist = artist;
        }
        if entry.title.is_empty() && !title.is_empty() {
            entry.title = title;
        }
        if entry.creator.is_empty() && !creator.is_empty() {
            entry.creator = creator;
        }
        if entry.tags.is_empty() && !tags.is_empty() {
            entry.tags = tags;
        }
        entry.diffs.push(SongDiffEntry {
            version: if diff_version.is_empty() {
                file_trimmed.to_string()
            } else {
                diff_version
            },
            path: path_str,
            mode,
        });
    }

    Ok(finalize_mapsets(groups.into_values().collect()))
}

/// Fast header-only directory scan of `songs_dir`, reading only `[General]` and
/// `[Metadata]` of each `.osu` file and stopping before `[HitObjects]`.
pub fn scan_songs_folder_fallback(songs_dir: &Path) -> Vec<SongMapsetEntry> {
    let Ok(entries) = fs::read_dir(songs_dir) else {
        return Vec::new();
    };
    let mut mapsets = Vec::new();

    for entry in entries.flatten() {
        let folder_path = entry.path();
        if !folder_path.is_dir() {
            continue;
        }
        let folder_name = entry.file_name().to_string_lossy().into_owned();
        let Ok(files) = fs::read_dir(&folder_path) else {
            continue;
        };

        let mut set_id = -1i32;
        let mut artist = String::new();
        let mut title = String::new();
        let mut creator = String::new();
        let mut tags = String::new();
        let mut diffs = Vec::new();

        for file_entry in files.flatten() {
            let file_path = file_entry.path();
            if !file_path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("osu"))
            {
                continue;
            }
            let Some(header) = read_osu_header(&file_path) else {
                continue;
            };
            if set_id <= 0 && header.set_id > 0 {
                set_id = header.set_id;
            }
            if artist.is_empty() && !header.artist.is_empty() {
                artist = header.artist;
            }
            if title.is_empty() && !header.title.is_empty() {
                title = header.title;
            }
            if creator.is_empty() && !header.creator.is_empty() {
                creator = header.creator;
            }
            if tags.is_empty() && !header.tags.is_empty() {
                tags = header.tags;
            }
            let version = if header.version.is_empty() {
                file_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default()
            } else {
                header.version
            };
            diffs.push(SongDiffEntry {
                version,
                path: file_path.to_string_lossy().into_owned(),
                mode: header.mode,
            });
        }

        if !diffs.is_empty() {
            mapsets.push(SongMapsetEntry {
                set_id,
                artist,
                title,
                creator,
                tags,
                folder: folder_name,
                diffs,
            });
        }
    }

    finalize_mapsets(mapsets)
}

/// Multi-word case-insensitive search across artist, title, creator, difficulty
/// version, tags, folder name, and BeatmapSetID.
pub fn search_songs_index(
    index: &[SongMapsetEntry],
    query: &str,
    limit: usize,
) -> Vec<SongMapsetEntry> {
    if limit == 0 {
        return Vec::new();
    }
    let tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();

    if tokens.is_empty() {
        return index.iter().take(limit).cloned().collect();
    }

    let mut matches = Vec::new();
    for mapset in index {
        if mapset_matches(mapset, &tokens) {
            matches.push(mapset.clone());
            if matches.len() >= limit {
                break;
            }
        }
    }
    matches
}

fn mapset_matches(mapset: &SongMapsetEntry, tokens: &[String]) -> bool {
    let artist = mapset.artist.to_lowercase();
    let title = mapset.title.to_lowercase();
    let creator = mapset.creator.to_lowercase();
    let tags = mapset.tags.to_lowercase();
    let folder = mapset.folder.to_lowercase();
    let set_id = if mapset.set_id > 0 {
        mapset.set_id.to_string()
    } else {
        String::new()
    };

    tokens.iter().all(|tok| {
        artist.contains(tok)
            || title.contains(tok)
            || creator.contains(tok)
            || tags.contains(tok)
            || folder.contains(tok)
            || (!set_id.is_empty() && set_id.contains(tok))
            || mapset
                .diffs
                .iter()
                .any(|d| d.version.to_lowercase().contains(tok))
    })
}

fn existing_subfolders(songs_dir: &Path) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    let Ok(entries) = fs::read_dir(songs_dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            map.insert(name, path);
        }
    }
    map
}

fn finalize_mapsets(mut mapsets: Vec<SongMapsetEntry>) -> Vec<SongMapsetEntry> {
    for mapset in &mut mapsets {
        mapset
            .diffs
            .sort_by_key(|a| a.version.to_lowercase());
    }
    mapsets.sort_by(|a, b| {
        a.artist
            .to_lowercase()
            .cmp(&b.artist.to_lowercase())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.folder.to_lowercase().cmp(&b.folder.to_lowercase()))
    });
    mapsets
}

struct OsuHeader {
    artist: String,
    title: String,
    creator: String,
    version: String,
    tags: String,
    set_id: i32,
    mode: u8,
}

fn read_osu_header(path: &Path) -> Option<OsuHeader> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut section = "";
    let mut saw_metadata = false;
    let mut header = OsuHeader {
        artist: String::new(),
        title: String::new(),
        creator: String::new(),
        version: String::new(),
        tags: String::new(),
        set_id: -1,
        mode: 0,
    };

    for _ in 0..512 {
        line.clear();
        let n = reader.read_line(&mut line).ok()?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if saw_metadata {
                // We already finished [Metadata]; stop before [Difficulty]/[HitObjects].
                break;
            }
            section = match trimmed {
                "[General]" => "General",
                "[Metadata]" => {
                    saw_metadata = true;
                    "Metadata"
                }
                "[HitObjects]" | "[TimingPoints]" | "[Events]" | "[Difficulty]" => break,
                _ => "",
            };
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match section {
            "General" if key == "Mode" => {
                if let Ok(m) = value.parse::<u8>() {
                    header.mode = m;
                }
            }
            "Metadata" => match key {
                "Artist" => header.artist = value.to_string(),
                "Title" => header.title = value.to_string(),
                "Creator" => header.creator = value.to_string(),
                "Version" => header.version = value.to_string(),
                "Tags" => header.tags = value.to_string(),
                "BeatmapSetID" => {
                    if let Ok(id) = value.parse::<i32>() {
                        header.set_id = id;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    Some(header)
}

struct DbReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> DbReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| "Unexpected end of osu!.db".to_string())?;
        if end > self.bytes.len() {
            return Err("Unexpected end of osu!.db".into());
        }
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn skip(&mut self, len: usize) -> Result<(), String> {
        self.read_slice(len).map(|_| ())
    }

    fn read_u8(&mut self) -> Result<u8, String> {
        Ok(self.read_slice(1)?[0])
    }

    fn read_i32(&mut self) -> Result<i32, String> {
        let b = self.read_slice(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i64(&mut self) -> Result<i64, String> {
        let b = self.read_slice(8)?;
        Ok(i64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_uleb128(&mut self) -> Result<usize, String> {
        let mut result: usize = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.read_u8()?;
            let low = (byte & 0x7f) as usize;
            result |= low
                .checked_shl(shift)
                .ok_or_else(|| "ULEB128 overflow in osu!.db".to_string())?;
            if (byte & 0x80) == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift >= usize::BITS {
                return Err("ULEB128 too large in osu!.db".into());
            }
        }
    }

    fn read_osu_string(&mut self) -> Result<String, String> {
        match self.read_u8()? {
            0x00 => Ok(String::new()),
            0x0b => {
                let len = self.read_uleb128()?;
                let slice = self.read_slice(len)?;
                Ok(String::from_utf8_lossy(slice).into_owned())
            }
            other => Err(format!("Invalid osu!.db string tag: 0x{other:02x}")),
        }
    }

    fn skip_star_rating_dict(&mut self) -> Result<(), String> {
        let count = self.read_i32()?;
        if count < 0 {
            return Err("Negative star-rating pair count in osu!.db".into());
        }
        for _ in 0..count {
            let key_tag = self.read_u8()?;
            if key_tag != 0x08 {
                return Err(format!("Unexpected star-rating key tag: 0x{key_tag:02x}"));
            }
            self.skip(4)?; // mods i32
            match self.read_u8()? {
                0x0c => self.skip(4)?, // f32 (v20250107+)
                0x0d => self.skip(8)?, // f64 (v20140609..v20250106)
                other => {
                    return Err(format!("Unexpected star-rating value tag: 0x{other:02x}"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;

    fn write_osu_string(out: &mut Vec<u8>, s: &str) {
        if s.is_empty() {
            out.push(0x00);
            return;
        }
        out.push(0x0b);
        let mut len = s.len();
        loop {
            let mut byte = (len & 0x7f) as u8;
            len >>= 7;
            if len != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if len == 0 {
                break;
            }
        }
        out.extend_from_slice(s.as_bytes());
    }

    #[allow(clippy::too_many_arguments)]
    fn push_synthetic_beatmap(
        out: &mut Vec<u8>,
        version: i32,
        artist: &str,
        title: &str,
        creator: &str,
        diff: &str,
        osu_file: &str,
        set_id: i32,
        mode: u8,
        tags: &str,
        folder: &str,
        use_f32_stars: bool,
    ) {
        if version < 20191106 {
            out.extend_from_slice(&0i32.to_le_bytes());
        }
        write_osu_string(out, artist);
        write_osu_string(out, "");
        write_osu_string(out, title);
        write_osu_string(out, "");
        write_osu_string(out, creator);
        write_osu_string(out, diff);
        write_osu_string(out, "audio.mp3");
        write_osu_string(out, "deadbeef");
        write_osu_string(out, osu_file);
        out.extend_from_slice(&[0u8; 15]); // ranked + counts + last_modified
        if version < 20140609 {
            out.extend_from_slice(&[5u8; 4]);
        } else {
            out.extend_from_slice(&[0u8; 16]);
        }
        out.extend_from_slice(&1.4f64.to_le_bytes()); // slider velocity
        if version >= 20140609 {
            for i in 0..4 {
                if i == 0 {
                    out.extend_from_slice(&1i32.to_le_bytes());
                    out.push(0x08);
                    out.extend_from_slice(&0i32.to_le_bytes());
                    if use_f32_stars {
                        out.push(0x0c);
                        out.extend_from_slice(&5.25f32.to_le_bytes());
                    } else {
                        out.push(0x0d);
                        out.extend_from_slice(&5.25f64.to_le_bytes());
                    }
                } else {
                    out.extend_from_slice(&0i32.to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&[0u8; 12]); // drain/total/preview
        out.extend_from_slice(&1i32.to_le_bytes()); // 1 timing point
        out.extend_from_slice(&[0u8; 17]);
        out.extend_from_slice(&100i32.to_le_bytes()); // beatmap_id
        out.extend_from_slice(&set_id.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes()); // thread_id
        out.extend_from_slice(&[0u8; 10]); // grades + local_offset + stack_leniency
        out.push(mode);
        write_osu_string(out, "source");
        write_osu_string(out, tags);
        out.extend_from_slice(&0i16.to_le_bytes());
        write_osu_string(out, "");
        out.extend_from_slice(&[0u8; 10]); // unplayed + last_played + is_osz2
        write_osu_string(out, folder);
        out.extend_from_slice(&[0u8; 13]);
        if version < 20140609 {
            out.extend_from_slice(&0i16.to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 5]);
    }

    fn build_synthetic_db(version: i32, use_f32_stars: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&2i32.to_le_bytes()); // folder_count
        out.push(1); // account_unlocked
        out.extend_from_slice(&0i64.to_le_bytes()); // unlock_date
        write_osu_string(&mut out, "Yoge");
        out.extend_from_slice(&3i32.to_le_bytes()); // beatmap_count

        push_synthetic_beatmap(
            &mut out,
            version,
            "Camellia",
            "Ghost",
            "Monstrata",
            "Expert",
            "Camellia - Ghost (Monstrata) [Expert].osu",
            123456,
            0,
            "edm bass tech",
            "123456 Camellia - Ghost",
            use_f32_stars,
        );
        push_synthetic_beatmap(
            &mut out,
            version,
            "Camellia",
            "Ghost",
            "Monstrata",
            "Hard",
            "Camellia - Ghost (Monstrata) [Hard].osu",
            123456,
            0,
            "edm bass tech",
            "123456 Camellia - Ghost",
            use_f32_stars,
        );
        // Entry whose folder does not exist on disk should be skipped cleanly.
        push_synthetic_beatmap(
            &mut out,
            version,
            "Missing",
            "Deleted Map",
            "Nobody",
            "Normal",
            "missing.osu",
            999999,
            1,
            "gone",
            "999999 Missing - Deleted Map",
            use_f32_stars,
        );
        out
    }

    #[test]
    fn parses_synthetic_osu_db_v2021_and_v2025_and_skips_missing_folders() {
        let scratch = Scratch::new("songs-index-db");
        let songs_dir = scratch.path("Songs");
        let existing = songs_dir.join("123456 Camellia - Ghost");
        fs::create_dir_all(&existing).unwrap();

        for (version, f32_stars) in [(20211103, false), (20250107, true)] {
            let db_bytes = build_synthetic_db(version, f32_stars);
            let parsed = parse_osu_db(&db_bytes, &songs_dir).expect("parse synthetic osu!.db");
            assert_eq!(parsed.len(), 1);
            let set = &parsed[0];
            assert_eq!(set.set_id, 123456);
            assert_eq!(set.artist, "Camellia");
            assert_eq!(set.title, "Ghost");
            assert_eq!(set.creator, "Monstrata");
            assert_eq!(set.tags, "edm bass tech");
            assert_eq!(set.diffs.len(), 2);
            assert_eq!(set.diffs[0].version, "Expert");
            assert_eq!(set.diffs[1].version, "Hard");
            assert!(set.diffs[0].path.ends_with("Camellia - Ghost (Monstrata) [Expert].osu"));
        }
    }

    #[test]
    fn fallback_scan_reads_headers_only_and_handles_missing_or_corrupt_db() {
        let scratch = Scratch::new("songs-index-fallback");
        let songs_dir = scratch.path("Songs");
        let map_folder = songs_dir.join("42000 xi - Freedom Dive");
        fs::create_dir_all(&map_folder).unwrap();

        let osu_a = "\
osu file format v14

[General]
AudioFilename: audio.mp3
Mode: 0

[Metadata]
Title:Freedom Dive
Artist:xi
Creator:Nakagawa-Kanon
Version:Four Dimensions
Tags:bms stream classic
BeatmapSetID:42000

[HitObjects]
this should never be parsed as metadata:
Title:WrongTitle
";
        let osu_b = "\
osu file format v14

[General]
Mode: 1

[Metadata]
Title:Freedom Dive
Artist:xi
Creator:Nakagawa-Kanon
Version:Another
Tags:bms stream classic
BeatmapSetID:42000
";
        fs::write(map_folder.join("xi - Freedom Dive [Four Dimensions].osu"), osu_a).unwrap();
        fs::write(map_folder.join("xi - Freedom Dive [Another].osu"), osu_b).unwrap();

        // Write a corrupt osu!.db next to Songs to verify graceful fallback.
        fs::write(scratch.path("osu!.db"), b"not a valid osu db").unwrap();

        let (index, source) = build_songs_index_with_source(&songs_dir);
        assert_eq!(source, "folder-scan");
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].title, "Freedom Dive");
        assert_eq!(index[0].artist, "xi");
        assert_eq!(index[0].creator, "Nakagawa-Kanon");
        assert_eq!(index[0].set_id, 42000);
        assert_eq!(index[0].diffs.len(), 2);
        assert_eq!(index[0].diffs[0].version, "Another");
        assert_eq!(index[0].diffs[0].mode, 1);
        assert_eq!(index[0].diffs[1].version, "Four Dimensions");
        assert_eq!(index[0].diffs[1].mode, 0);
    }

    #[test]
    fn build_songs_index_prefers_osu_db_when_valid() {
        let scratch = Scratch::new("songs-index-prefers-db");
        let songs_dir = scratch.path("Songs");
        fs::create_dir_all(songs_dir.join("123456 Camellia - Ghost")).unwrap();
        fs::write(scratch.path("osu!.db"), build_synthetic_db(20250107, true)).unwrap();

        let (index, source) = build_songs_index_with_source(&songs_dir);
        assert_eq!(source, "osu!.db");
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].artist, "Camellia");
    }

    #[test]
    fn multi_word_search_matches_artist_title_mapper_diff_tags_and_set_id() {
        let index = vec![
            SongMapsetEntry {
                set_id: 123456,
                artist: "Camellia".into(),
                title: "Ghost".into(),
                creator: "Monstrata".into(),
                tags: "edm bass tech".into(),
                folder: "123456 Camellia - Ghost".into(),
                diffs: vec![SongDiffEntry {
                    version: "Expert".into(),
                    path: "Songs/123456 Camellia - Ghost/expert.osu".into(),
                    mode: 0,
                }],
            },
            SongMapsetEntry {
                set_id: 42000,
                artist: "xi".into(),
                title: "Freedom Dive".into(),
                creator: "Nakagawa-Kanon".into(),
                tags: "bms stream classic".into(),
                folder: "42000 xi - Freedom Dive".into(),
                diffs: vec![SongDiffEntry {
                    version: "Four Dimensions".into(),
                    path: "Songs/42000 xi - Freedom Dive/fd.osu".into(),
                    mode: 0,
                }],
            },
        ];

        assert_eq!(search_songs_index(&index, "", 10).len(), 2);
        assert_eq!(search_songs_index(&index, "camellia expert", 10).len(), 1);
        assert_eq!(search_songs_index(&index, "monstrata tech", 10).len(), 1);
        assert_eq!(search_songs_index(&index, "42000 dimensions", 10).len(), 1);
        assert_eq!(search_songs_index(&index, "xi kanon bms", 10).len(), 1);
        assert_eq!(search_songs_index(&index, "camellia freedom", 10).len(), 0);
        assert_eq!(search_songs_index(&index, "", 1).len(), 1);
    }
}
