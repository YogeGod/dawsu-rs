use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const LEGACY_PREFIX: &str = "osu.Game.Resources.Skins.Legacy.";
const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub const WANTED_CLASSIC_RESOURCES: &[(&str, &str)] = &[
    ("hitcircle@2x.png", "hitcircle"),
    ("hitcircleoverlay@2x.png", "overlay"),
    ("approachcircle@2x.png", "approachCircle"),
    ("reversearrow@2x.png", "reverseArrow"),
    ("default-0@2x.png", "0"),
    ("default-1@2x.png", "1"),
    ("default-2@2x.png", "2"),
    ("default-3@2x.png", "3"),
    ("default-4@2x.png", "4"),
    ("default-5@2x.png", "5"),
    ("default-6@2x.png", "6"),
    ("default-7@2x.png", "7"),
    ("default-8@2x.png", "8"),
    ("default-9@2x.png", "9"),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkinEntry {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawSkinTextures {
    pub hitcircle: Option<Vec<u8>>,
    pub overlay: Option<Vec<u8>>,
    pub slider_end_circle: Option<Vec<u8>>,
    pub slider_end_overlay: Option<Vec<u8>>,
    pub approach_circle: Option<Vec<u8>>,
    pub reverse_arrow: Option<Vec<u8>>,
    pub digits: Vec<Option<Vec<u8>>>,
    pub digit_hd: bool,
    pub overlap: i32,
    pub overlay_above_number: bool,
    pub combo_colours: Vec<String>,
    pub slider_border: Option<String>,
    pub slider_track: Option<String>,
    pub circle_fill: f64,
}

pub fn load_lazer_classic_raw() -> Option<RawSkinTextures> {
    let path = find_lazer_resources()?;
    let bytes = std::fs::read(&path).ok()?;
    let mut raw = extract_legacy_pngs(&bytes)?;
    let hitcircle = raw.remove("hitcircle")?;
    let overlay = raw.remove("overlay")?;
    let digits = (0..10).map(|i| raw.remove(&i.to_string())).collect();
    let reverse_arrow = raw.remove("reverseArrow");
    let approach_circle = raw.remove("approachCircle");

    Some(RawSkinTextures {
        hitcircle: Some(hitcircle),
        overlay: Some(overlay),
        slider_end_circle: None,
        slider_end_overlay: None,
        approach_circle,
        reverse_arrow,
        digits,
        digit_hd: true,
        overlap: -2,
        overlay_above_number: true,
        combo_colours: Vec::new(),
        slider_border: None,
        slider_track: None,
        circle_fill: 0.92,
    })
}

/// Lists built-in skins (Argon, osu!lazer Classic) and any osu! skin folders found in the
/// osu! Skins directory beside `songs_folder` or the default osu! stable install.
pub fn list_available_skin_entries(songs_folder: Option<&Path>) -> Vec<SkinEntry> {
    let mut entries = vec![SkinEntry {
        id: "argon".into(),
        name: "Argon (osu!lazer)".into(),
    }];
    if find_lazer_resources().is_some() {
        entries.push(SkinEntry {
            id: "classic".into(),
            name: "Classic (osu!lazer)".into(),
        });
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(songs) = songs_folder
        && let Some(parent) = songs.parent()
    {
        let skins = parent.join("Skins");
        if skins.is_dir() {
            roots.push(skins);
        }
    }
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let skins = Path::new(&local).join("osu!").join("Skins");
        if skins.is_dir() && !roots.iter().any(|r| r == &skins) {
            roots.push(skins);
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        let skins = Path::new(&home).join("Library/Application Support/osu!/Skins");
        if skins.is_dir() && !roots.iter().any(|r| r == &skins) {
            roots.push(skins);
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(home) = std::env::var_os("HOME") {
        let skins = Path::new(&home).join(".local/share/osu/Skins");
        if skins.is_dir() && !roots.iter().any(|r| r == &skins) {
            roots.push(skins);
        }
    }

    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for root in roots {
        if let Ok(read) = std::fs::read_dir(&root) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let has_skin = path.join("skin.ini").is_file()
                        || path.join("hitcircle@2x.png").is_file()
                        || path.join("hitcircle.png").is_file();
                    if has_skin {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        found.push((name, path));
                    }
                }
            }
        }
    }
    found.sort_by_key(|a| a.0.to_lowercase());
    for (name, path) in found {
        entries.push(SkinEntry {
            id: format!("dir:{}", path.display()),
            name,
        });
    }
    entries
}

/// Loads hitcircle/overlay/approach/arrow/digit PNG bytes and `skin.ini` hints from a custom osu! skin folder.
pub fn load_skin_from_dir_raw(dir: &Path) -> Result<RawSkinTextures, String> {
    if !dir.is_dir() {
        return Err(format!("Skin folder not found: {}", dir.display()));
    }
    let mut prefix = String::from("default");
    let mut overlap: i32 = -2;
    let mut overlay_above_number = true;
    let mut combo_colours: Vec<String> = Vec::new();
    let mut slider_border: Option<String> = None;
    let mut slider_track: Option<String> = None;

    if let Ok(ini) = std::fs::read_to_string(dir.join("skin.ini")) {
        for raw_line in ini.lines() {
            let line = raw_line.split("//").next().unwrap_or("").trim();
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let key = k.trim();
            let val = v.trim();
            if key.eq_ignore_ascii_case("HitCirclePrefix") && !val.is_empty() {
                prefix = val.replace('\\', "/");
            } else if key.eq_ignore_ascii_case("HitCircleOverlap") {
                if let Ok(n) = val.parse::<i32>() {
                    overlap = n;
                }
            } else if key.eq_ignore_ascii_case("HitCircleOverlayAboveNumber")
                || key.eq_ignore_ascii_case("HitCircleOverlayAboveNumer")
            {
                overlay_above_number = val != "0";
            } else if key.len() == 6 && key[..5].eq_ignore_ascii_case("Combo") {
                if let Some(hex) = parse_rgb_triplet(val) {
                    combo_colours.push(hex);
                }
            } else if key.eq_ignore_ascii_case("SliderBorder") {
                slider_border = parse_rgb_triplet(val);
            } else if key.eq_ignore_ascii_case("SliderTrackOverride") {
                slider_track = parse_rgb_triplet(val);
            }
        }
    }

    let read_sprite = |stem: &str| -> (Option<Vec<u8>>, bool) {
        let p2 = dir.join(format!("{stem}@2x.png"));
        if let Ok(bytes) = std::fs::read(&p2) {
            return (Some(bytes), true);
        }
        let p1 = dir.join(format!("{stem}.png"));
        if let Ok(bytes) = std::fs::read(&p1) {
            return (Some(bytes), false);
        }
        (None, false)
    };

    let (hitcircle, _) = read_sprite("hitcircle");
    let (overlay, _) = read_sprite("hitcircleoverlay");
    let (slider_end_circle, _) = read_sprite("sliderendcircle");
    let (slider_end_overlay, _) = read_sprite("sliderendcircleoverlay");
    let (approach_circle, _) = read_sprite("approachcircle");
    let (reverse_arrow, _) = read_sprite("reversearrow");
    let mut digits = Vec::with_capacity(10);
    let mut digit_hd = false;
    for i in 0..10 {
        let stem = format!("{prefix}-{i}");
        let (bytes, hd) = read_sprite(&stem);
        let (bytes, hd) = if bytes.is_some() {
            (bytes, hd)
        } else {
            read_sprite(&format!("default-{i}"))
        };
        if hd {
            digit_hd = true;
        }
        digits.push(bytes);
    }

    if hitcircle.is_none() && overlay.is_none() {
        return Err("That folder does not contain hitcircle.png or hitcircle@2x.png.".into());
    }

    Ok(RawSkinTextures {
        hitcircle,
        overlay,
        slider_end_circle,
        slider_end_overlay,
        approach_circle,
        reverse_arrow,
        digits,
        digit_hd,
        overlap,
        overlay_above_number,
        combo_colours,
        slider_border,
        slider_track,
        circle_fill: 1.0,
    })
}

pub fn load_skin_by_id_raw(skin_id: &str) -> Result<Option<RawSkinTextures>, String> {
    if skin_id == "argon" || skin_id.is_empty() {
        return Ok(None);
    }
    if skin_id == "classic" {
        return Ok(load_lazer_classic_raw());
    }
    if let Some(dir) = skin_id.strip_prefix("dir:") {
        return load_skin_from_dir_raw(Path::new(dir)).map(Some);
    }
    Err(format!("Unknown preview skin: {skin_id}"))
}

fn parse_rgb_triplet(val: &str) -> Option<String> {
    let parts: Vec<u8> = val
        .split(',')
        .take(3)
        .filter_map(|p| p.trim().parse::<u8>().ok())
        .collect();
    if parts.len() == 3 {
        Some(format!("#{:02x}{:02x}{:02x}", parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

pub fn find_lazer_resources() -> Option<PathBuf> {
    if let Some(env_path) = std::env::var_os("DAWSU_LAZER_RESOURCES") {
        let p = PathBuf::from(env_path);
        if p.is_file() {
            return Some(p);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let p = Path::new(&local)
                .join("osulazer")
                .join("current")
                .join("osu.Game.Resources.dll");
            if p.is_file() {
                return Some(p);
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let system = PathBuf::from("/Applications/osu!.app/Contents/MacOS/osu.Game.Resources.dll");
        if system.is_file() {
            return Some(system);
        }
        if let Some(home) = std::env::var_os("HOME") {
            let user = Path::new(&home)
                .join("Applications/osu!.app/Contents/MacOS/osu.Game.Resources.dll");
            if user.is_file() {
                return Some(user);
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let p = Path::new(&home).join(".local/share/osu/osu.Game.Resources.dll");
            if p.is_file() {
                return Some(p);
            }
        }
    }

    None
}

fn read_u16(data: &[u8], off: usize) -> Option<u16> {
    let s = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn read_u32(data: &[u8], off: usize) -> Option<u32> {
    let s = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_u64(data: &[u8], off: usize) -> Option<u64> {
    let s = data.get(off..off + 8)?;
    Some(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

fn read_index(data: &[u8], off: usize, size: usize) -> Option<usize> {
    match size {
        2 => read_u16(data, off).map(|v| v as usize),
        4 => read_u32(data, off).map(|v| v as usize),
        _ => None,
    }
}

struct Section {
    virtual_addr: u32,
    virtual_size: u32,
    raw_ptr: u32,
    raw_size: u32,
}

fn rva_to_offset(sections: &[Section], rva: u32) -> Option<usize> {
    for sec in sections {
        let span = sec.virtual_size.max(sec.raw_size);
        if rva >= sec.virtual_addr && rva < sec.virtual_addr.saturating_add(span) {
            let delta = rva - sec.virtual_addr;
            if delta < sec.raw_size {
                return Some(sec.raw_ptr.checked_add(delta)? as usize);
            }
        }
    }
    None
}

/// Parses a .NET PE assembly (`osu.Game.Resources.dll`) as data only and extracts the classic
/// skin PNG resources under `osu.Game.Resources.Skins.Legacy.`.
pub fn extract_legacy_pngs(bytes: &[u8]) -> Option<HashMap<String, Vec<u8>>> {
    if bytes.get(0..2) != Some(b"MZ") {
        return None;
    }
    let pe_off = read_u32(bytes, 0x3C)? as usize;
    if bytes.get(pe_off..pe_off + 4) != Some(b"PE\0\0") {
        return None;
    }

    let coff = pe_off + 4;
    let num_sections = read_u16(bytes, coff + 2)? as usize;
    let opt_size = read_u16(bytes, coff + 16)? as usize;
    let opt = coff + 20;
    let magic = read_u16(bytes, opt)?;
    let data_dirs = match magic {
        0x10b => opt + 96,
        0x20b => opt + 112,
        _ => return None,
    };

    let cli_rva = read_u32(bytes, data_dirs + 14 * 8)?;
    let cli_size = read_u32(bytes, data_dirs + 14 * 8 + 4)?;
    if cli_rva == 0 || cli_size < 72 {
        return None;
    }

    let sec_table = opt + opt_size;
    let mut sections = Vec::with_capacity(num_sections);
    for i in 0..num_sections {
        let s = sec_table + i * 40;
        sections.push(Section {
            virtual_size: read_u32(bytes, s + 8)?,
            virtual_addr: read_u32(bytes, s + 12)?,
            raw_size: read_u32(bytes, s + 16)?,
            raw_ptr: read_u32(bytes, s + 20)?,
        });
    }

    let cli_off = rva_to_offset(&sections, cli_rva)?;
    let metadata_rva = read_u32(bytes, cli_off + 8)?;
    let resources_rva = read_u32(bytes, cli_off + 24)?;
    let resources_size = read_u32(bytes, cli_off + 28)? as usize;
    if metadata_rva == 0 || resources_rva == 0 || resources_size == 0 {
        return None;
    }

    let md_off = rva_to_offset(&sections, metadata_rva)?;
    let res_off = rva_to_offset(&sections, resources_rva)?;
    if bytes.get(md_off..md_off + 4) != Some(b"BSJB") {
        return None;
    }

    let version_len = read_u32(bytes, md_off + 12)? as usize;
    let version_padded = (version_len + 3) & !3;
    let num_streams = read_u16(bytes, md_off + 16 + version_padded + 2)? as usize;
    let mut cursor = md_off + 16 + version_padded + 4;

    let mut tables_stream: Option<(usize, usize)> = None;
    let mut strings_stream: Option<(usize, usize)> = None;

    for _ in 0..num_streams {
        let s_off = read_u32(bytes, cursor)? as usize;
        let s_size = read_u32(bytes, cursor + 4)? as usize;
        cursor += 8;
        let name_start = cursor;
        while *bytes.get(cursor)? != 0 {
            cursor += 1;
        }
        let name = std::str::from_utf8(bytes.get(name_start..cursor)?).ok()?;
        let name_len = cursor - name_start + 1;
        cursor = name_start + ((name_len + 3) & !3);

        match name {
            "#~" | "#-" => tables_stream = Some((md_off + s_off, s_size)),
            "#Strings" => strings_stream = Some((md_off + s_off, s_size)),
            _ => {}
        }
    }

    let (t_off, _t_size) = tables_stream?;
    let (str_off, str_size) = strings_stream?;

    let heap_sizes = *bytes.get(t_off + 6)?;
    let s_idx = if (heap_sizes & 0x01) != 0 { 4 } else { 2 };
    let g_idx = if (heap_sizes & 0x02) != 0 { 4 } else { 2 };
    let b_idx = if (heap_sizes & 0x04) != 0 { 4 } else { 2 };

    let valid_mask = read_u64(bytes, t_off + 8)?;
    let mut row_counts = [0u32; 64];
    let mut pos = t_off + 24;
    for (t, count) in row_counts.iter_mut().enumerate() {
        if (valid_mask >> t) & 1 != 0 {
            *count = read_u32(bytes, pos)?;
            pos += 4;
        }
    }

    if ((valid_mask >> 0x28) & 1) == 0 || row_counts[0x28] == 0 {
        return None;
    }

    let idx = |t: usize| -> usize { if row_counts[t] < 65536 { 2 } else { 4 } };
    let coded = |tag_bits: u32, tables: &[usize]| -> usize {
        let max_rows = tables.iter().map(|&t| row_counts[t]).max().unwrap_or(0);
        if max_rows < (1u32 << (16 - tag_bits)) {
            2
        } else {
            4
        }
    };

    let type_def_or_ref = coded(2, &[0x02, 0x01, 0x1B]);
    let has_constant = coded(2, &[0x04, 0x08, 0x17]);
    let has_custom_attribute = coded(
        5,
        &[
            0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B,
            0x20, 0x23, 0x26, 0x27, 0x28, 0x2A, 0x2C, 0x2B,
        ],
    );
    let has_field_marshal = coded(1, &[0x04, 0x08]);
    let has_decl_security = coded(2, &[0x02, 0x06, 0x20]);
    let member_ref_parent = coded(3, &[0x02, 0x01, 0x1A, 0x06, 0x1B]);
    let has_semantics = coded(1, &[0x14, 0x17]);
    let method_def_or_ref = coded(1, &[0x06, 0x0A]);
    let member_forwarded = coded(1, &[0x04, 0x06]);
    let implementation = coded(2, &[0x26, 0x23, 0x27]);
    let custom_attribute_type = coded(3, &[0x06, 0x0A]);
    let resolution_scope = coded(2, &[0x00, 0x1A, 0x23, 0x01]);

    for (t, &count) in row_counts.iter().enumerate().take(0x28) {
        if count == 0 {
            continue;
        }
        let row_size = match t {
            0x00 => 2 + s_idx + g_idx * 3,
            0x01 => resolution_scope + s_idx * 2,
            0x02 => 4 + s_idx * 2 + type_def_or_ref + idx(0x04) + idx(0x06),
            0x03 => idx(0x04),
            0x04 => 2 + s_idx + b_idx,
            0x05 => idx(0x06),
            0x06 => 4 + 2 + 2 + s_idx + b_idx + idx(0x08),
            0x07 => idx(0x08),
            0x08 => 2 + 2 + s_idx,
            0x09 => idx(0x02) + type_def_or_ref,
            0x0A => member_ref_parent + s_idx + b_idx,
            0x0B => 2 + has_constant + b_idx,
            0x0C => has_custom_attribute + custom_attribute_type + b_idx,
            0x0D => has_field_marshal + b_idx,
            0x0E => 2 + has_decl_security + b_idx,
            0x0F => 2 + 4 + idx(0x02),
            0x10 => 4 + idx(0x04),
            0x11 => b_idx,
            0x12 => idx(0x02) + idx(0x14),
            0x13 => idx(0x14),
            0x14 => 2 + s_idx + type_def_or_ref,
            0x15 => idx(0x02) + idx(0x17),
            0x16 => idx(0x17),
            0x17 => 2 + s_idx + b_idx,
            0x18 => 2 + idx(0x06) + has_semantics,
            0x19 => idx(0x02) + method_def_or_ref * 2,
            0x1A => s_idx,
            0x1B => b_idx,
            0x1C => 2 + member_forwarded + s_idx + idx(0x1A),
            0x1D => 4 + idx(0x04),
            0x1E => 4 + 4,
            0x1F => 4,
            0x20 => 4 + 2 * 4 + 4 + b_idx + s_idx * 2,
            0x21 => 4,
            0x22 => 4 * 3,
            0x23 => 2 * 4 + 4 + b_idx + s_idx * 2 + b_idx,
            0x24 => 4 + idx(0x23),
            0x25 => 4 * 3 + idx(0x23),
            0x26 => 4 + s_idx + b_idx,
            0x27 => 4 + 4 + s_idx * 2 + implementation,
            _ => return None,
        };
        pos = pos.checked_add((count as usize).checked_mul(row_size)?)?;
    }

    let wanted_map: HashMap<&str, &str> = WANTED_CLASSIC_RESOURCES.iter().copied().collect();
    let mut out = HashMap::new();
    let mr_row_size = 4 + 4 + s_idx + implementation;

    for i in 0..row_counts[0x28] as usize {
        let row = pos + i * mr_row_size;
        let offset = read_u32(bytes, row)? as usize;
        let name_idx = read_index(bytes, row + 8, s_idx)?;
        let impl_val = read_index(bytes, row + 8 + s_idx, implementation)?;
        if impl_val != 0 || name_idx >= str_size {
            continue;
        }
        let s_start = str_off + name_idx;
        let s_end = bytes
            .get(s_start..str_off + str_size)?
            .iter()
            .position(|&b| b == 0)
            .map(|p| s_start + p)?;
        let Ok(full_name) = std::str::from_utf8(bytes.get(s_start..s_end)?) else {
            continue;
        };
        let Some(short_name) = full_name.strip_prefix(LEGACY_PREFIX) else {
            continue;
        };
        let Some(&key) = wanted_map.get(short_name) else {
            continue;
        };
        if offset + 4 > resources_size {
            continue;
        }
        let len = read_u32(bytes, res_off + offset)? as usize;
        if offset + 4 + len > resources_size {
            continue;
        }
        let payload = bytes.get(res_off + offset + 4..res_off + offset + 4 + len)?;
        if payload.len() >= 8 && &payload[..8] == PNG_MAGIC {
            out.insert(key.to_string(), payload.to_vec());
        }
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_all_13_legacy_skin_pngs_from_local_lazer() {
        let Some(path) = find_lazer_resources() else {
            return;
        };
        let bytes = std::fs::read(&path).expect("read osu.Game.Resources.dll");
        let extracted = extract_legacy_pngs(&bytes).expect("parse .NET metadata and extract PNGs");
        assert!(
            extracted.len() >= 13,
            "expected at least 13 classic skin PNGs, got {}",
            extracted.len()
        );
        for &(_, key) in WANTED_CLASSIC_RESOURCES {
            let png = extracted
                .get(key)
                .unwrap_or_else(|| panic!("missing {key}"));
            assert!(
                png.starts_with(PNG_MAGIC),
                "{key} does not start with PNG magic bytes"
            );
        }
    }
}
