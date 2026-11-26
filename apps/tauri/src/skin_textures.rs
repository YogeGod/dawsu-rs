use serde_json::{Value, json};
use std::path::Path;

pub fn load_skin_textures_json() -> Value {
    let Some(raw) = dawsu_application::skin_textures::load_lazer_classic_raw() else {
        return Value::Null;
    };
    let (Some(hitcircle), Some(overlay)) = (raw.hitcircle.as_deref(), raw.overlay.as_deref()) else {
        return Value::Null;
    };
    let digits: Vec<Value> = raw
        .digits
        .iter()
        .map(|d| {
            d.as_deref()
                .map_or(Value::Null, |png| Value::String(png_data_url(png)))
        })
        .collect();
    json!({
        "hitcircle": png_data_url(hitcircle),
        "overlay": png_data_url(overlay),
        "approachCircle": raw.approach_circle.as_deref().map_or(Value::Null, |png| Value::String(png_data_url(png))),
        "reverseArrow": raw.reverse_arrow.as_deref().map_or(Value::Null, |png| Value::String(png_data_url(png))),
        "digits": digits,
        "circleFill": raw.circle_fill,
        "overlap": raw.overlap,
        "overlayAboveNumber": raw.overlay_above_number,
    })
}

/// Lists built-in skins (Argon, osu!lazer Classic) and any osu! skin folders found in the
/// osu! Skins directory beside `songs_folder` or the default osu! stable install.
pub fn list_available_skins(songs_folder: Option<&Path>) -> Value {
    json!(dawsu_application::skin_textures::list_available_skin_entries(songs_folder))
}

/// Loads hitcircle/overlay/approach/arrow/digit PNGs and `skin.ini` hints from a custom osu! skin folder.
pub fn load_skin_from_dir(dir: &Path) -> Result<Value, String> {
    let raw = dawsu_application::skin_textures::load_skin_from_dir_raw(dir)?;
    let opt_url = |b: Option<&[u8]>| b.map(png_data_url);
    let digits: Vec<Value> = raw
        .digits
        .iter()
        .map(|d| {
            d.as_deref()
                .map_or(Value::Null, |png| Value::String(png_data_url(png)))
        })
        .collect();
    Ok(json!({
        "hitcircle": opt_url(raw.hitcircle.as_deref()),
        "overlay": opt_url(raw.overlay.as_deref()),
        "sliderEndCircle": opt_url(raw.slider_end_circle.as_deref()),
        "sliderEndOverlay": opt_url(raw.slider_end_overlay.as_deref()),
        "approachCircle": opt_url(raw.approach_circle.as_deref()),
        "reverseArrow": opt_url(raw.reverse_arrow.as_deref()),
        "digits": digits,
        "digitHd": raw.digit_hd,
        "overlap": raw.overlap,
        "overlayAboveNumber": raw.overlay_above_number,
        "comboColours": raw.combo_colours,
        "sliderBorder": raw.slider_border,
        "sliderTrack": raw.slider_track,
        "circleFill": raw.circle_fill,
    }))
}

fn png_data_url(png: &[u8]) -> String {
    let mut out = String::with_capacity(22 + png.len().div_ceil(3) * 4);
    out.push_str("data:image/png;base64,");
    encode_base64(png, &mut out);
    out
}

/// Safely encodes a small local image file (<= 2 MiB) as a `data:image/...;base64,...` URL
/// compatible with the app's strict `img-src 'self' data:` CSP.
pub(crate) fn image_data_url(path: &Path) -> Option<String> {
    const MAX_THUMBNAIL_BYTES: u64 = 2 * 1024 * 1024;
    let mime = match path.extension().and_then(|e| e.to_str())?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    };
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_THUMBNAIL_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let prefix = format!("data:{mime};base64,");
    let mut out = String::with_capacity(prefix.len() + bytes.len().div_ceil(3) * 4);
    out.push_str(&prefix);
    encode_base64(&bytes, &mut out);
    Some(out)
}

fn encode_base64(input: &[u8], out: &mut String) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_application::skin_textures::{
        WANTED_CLASSIC_RESOURCES, extract_legacy_pngs, find_lazer_resources,
    };

    const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

    #[test]
    fn extracts_all_13_legacy_skin_pngs_from_local_lazer() {
        let Some(path) = find_lazer_resources() else {
            // osu!lazer is not installed on this machine (e.g. CI); skip gracefully.
            return;
        };
        let bytes = std::fs::read(&path).expect("read osu.Game.Resources.dll");
        let extracted = extract_legacy_pngs(&bytes).expect("parse .NET metadata and extract PNGs");
        assert!(extracted.len() >= 13, "expected at least 13 classic skin PNGs, got {}", extracted.len());
        for &(_, key) in WANTED_CLASSIC_RESOURCES {
            let png = extracted.get(key).unwrap_or_else(|| panic!("missing {key}"));
            assert!(png.starts_with(PNG_MAGIC), "{key} does not start with PNG magic bytes");
        }
        let payload = load_skin_textures_json();
        assert!(payload["hitcircle"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert!(payload["overlay"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert!(payload["reverseArrow"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(payload["digits"].as_array().unwrap().len(), 10);
    }
}
