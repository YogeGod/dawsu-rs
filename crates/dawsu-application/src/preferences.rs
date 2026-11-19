use crate::{Preferences, shortcuts};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(super) fn load(native: &Path, legacy: Option<&Path>) -> (Preferences, bool) {
    match std::fs::read(native) {
        Ok(bytes) => (
            serde_json::from_slice(without_bom(&bytes)).unwrap_or_default(),
            false,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => legacy
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<LegacyPreferences>(without_bom(&bytes)).ok())
            .map_or_else(
                || (Preferences::default(), true),
                LegacyPreferences::convert,
            ),
        Err(_) => (Preferences::default(), false),
    }
}

pub(super) fn without_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
struct LegacyPreferences {
    songs_folder: Option<PathBuf>,
    hitsound_folder: Option<PathBuf>,
    recent_maps: Option<Vec<PathBuf>>,
    key_preset: Option<u32>,
    key_style_asked: bool,
    key_overrides: Option<shortcuts::LegacyOverrides>,
    classic_browser: bool,
    browser_open: Option<bool>,
    inspector_open: Option<bool>,
    audition_on_placement: Option<bool>,
    protect_hitnormals: bool,
    lane_names_collapsed: bool,
    global_volume: Option<f32>,
    buffer_ms: Option<u32>,
}

impl LegacyPreferences {
    fn convert(self) -> (Preferences, bool) {
        let mut preferences = Preferences::default();
        preferences.songs_folder = self.songs_folder.unwrap_or_default();
        preferences.hitsound_library = self
            .hitsound_folder
            .filter(|path| !path.as_os_str().is_empty());
        let mut seen = HashSet::new();
        preferences.recent = self
            .recent_maps
            .unwrap_or_default()
            .into_iter()
            .filter(|path| {
                path.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("osu") || ext.eq_ignore_ascii_case("osz")
                })
            })
            .filter(|path| seen.insert(path.to_string_lossy().to_lowercase()))
            .take(20)
            .collect();
        if self.key_preset == Some(1) {
            preferences.key_preset = "osu!".into();
        }
        preferences.key_overrides = shortcuts::migrate(self.key_overrides.unwrap_or_default());
        preferences.classic_browser = self.classic_browser;
        preferences.audio_buffer_ms = self.buffer_ms.filter(|ms| (10..=500).contains(ms));
        preferences.show_browser = self.browser_open.unwrap_or(preferences.show_browser);
        preferences.show_inspector = self.inspector_open.unwrap_or(preferences.show_inspector);
        preferences.audition = self.audition_on_placement.unwrap_or(preferences.audition);
        preferences.protect_hitnormals = self.protect_hitnormals;
        preferences.lane_names = !self.lane_names_collapsed;
        if let Some(volume) = self.global_volume {
            preferences.master_gain = (volume / 100.0).clamp(0.2, 1.0);
        }
        let first = !self.key_style_asked && !matches!(self.key_preset, Some(0 | 1));
        (preferences, first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dawsu_test_support::Scratch;
    use std::path::PathBuf;

    #[test]
    fn first_launch_reads_a_copied_csharp_file_without_changing_it() {
        let fixture = Scratch::new("legacy-preferences");
        let native = fixture.path("rust.json");
        let legacy = fixture.path("csharp.json");
        let bytes = br#"{
            "SongsFolder":"test Songs",
            "HitsoundFolder":"test Library",
            "RecentMaps":["first.osu","second.osz","first.osu"],
            "KeyPreset":1,
            "KeyStyleAsked":true,
            "ClassicBrowser":true,
            "InspectorOpen":false,
            "BrowserOpen":false,
            "GlobalVolume":47,
            "BufferMs":45,
            "AuditionOnPlacement":false,
            "ProtectHitnormals":true,
            "LaneNamesCollapsed":true,
            "KeyOverrides":{
                "Cubase":{"undo":["None+F6"]},
                "Osu":{"undo":["None+F5"],"play":["None+F5","None+Space"],"moveLeft":["Alt+OemComma"]}
            }
        }"#;
        std::fs::write(&legacy, bytes).unwrap();
        let (pref, first) = load(&native, Some(&legacy));
        assert!(!first);
        assert_eq!(pref.songs_folder, PathBuf::from("test Songs"));
        assert_eq!(pref.hitsound_library, Some(PathBuf::from("test Library")));
        assert_eq!(pref.recent, ["first.osu", "second.osz"].map(PathBuf::from));
        assert_eq!(pref.key_preset, "osu!");
        assert_eq!(pref.audio_buffer_ms, Some(45));
        assert_eq!(pref.key_overrides["DAW"]["undo"], ["None+F6"]);
        assert!(pref.key_overrides["osu!"]["undo"].is_empty());
        assert_eq!(pref.key_overrides["osu!"]["nudgeLeft"], ["Alt+OemComma"]);
        let app = crate::Application::create(pref.clone(), false, None, None, false, crate::data_folder());
        assert_eq!(
            app.keymap
                .command_for(dawsu_editor::keys::KeyCombination::plain(
                    dawsu_editor::keys::Key::F5
                )),
            Some("play")
        );
        assert!(pref.classic_browser && pref.protect_hitnormals);
        assert!(!pref.show_browser && !pref.show_inspector && !pref.audition && !pref.lane_names);
        assert!((pref.master_gain - 0.47).abs() < 0.0001);
        assert!(pref.default_samples.as_os_str().is_empty());
        assert!(!native.exists());
        assert_eq!(std::fs::read(&legacy).unwrap(), bytes);

        std::fs::write(&native, br#"{"classic_browser":false,"key_preset":"DAW"}"#).unwrap();
        let (pref, first) = load(&native, Some(&legacy));
        assert!(!first && !pref.classic_browser);
        assert_eq!(pref.key_preset, "DAW");
        assert!(pref.hitsound_library.is_none());
        assert!(pref.audio_buffer_ms.is_none());

        std::fs::write(&native, b"invalid").unwrap();
        let (pref, first) = load(&native, Some(&legacy));
        assert!(!first && !pref.classic_browser);
        assert_eq!(std::fs::read(&native).unwrap(), b"invalid");
        assert_eq!(std::fs::read(&legacy).unwrap(), bytes);
    }
}
