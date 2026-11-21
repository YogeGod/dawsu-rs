//! osu!'s default hit and slider sounds, shipped inside dawsu!.
//!
//! The default sounds folder from the Preferences comes first, as a player's skin comes before
//! osu!'s own sounds. A sound that folder lacks plays from the copy dawsu! ships, so no default
//! hitnormal, whistle, finish, clap, slide, slider whistle or tick is "Sample unavailable".
//!
//! The 21 files are unchanged copies of osu!'s classic skin sounds, `Skins/Legacy/` in
//! ppy/osu-resources at tag 2026.1021.0 (commit 46d344264e5938e168cfbbd2c8fab5f572ebeda4): the
//! resources package osu!lazer 131fe72 uses (osu.Game/osu.Game.csproj:42), and the folder its
//! default classic skin reads (osu.Game/Skinning/DefaultLegacySkin.cs:42). They are licensed
//! CC BY-NC 4.0 by ppy Pty Ltd: see `assets/default-sounds/README.md` and
//! THIRD-PARTY-NOTICES.md.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! sound {
    ($name:literal) => {
        ($name, include_bytes!(concat!("../assets/default-sounds/", $name)) as &[u8])
    };
}

/// The shipped sounds by file name: the four hit sounds and the three slider sounds of each set.
pub(crate) const DEFAULT_SOUNDS: [(&str, &[u8]); 21] = [
    sound!("normal-hitnormal.wav"),
    sound!("normal-hitwhistle.wav"),
    sound!("normal-hitfinish.wav"),
    sound!("normal-hitclap.wav"),
    sound!("normal-sliderslide.wav"),
    sound!("normal-slidertick.wav"),
    sound!("normal-sliderwhistle.wav"),
    sound!("soft-hitnormal.wav"),
    sound!("soft-hitwhistle.wav"),
    sound!("soft-hitfinish.wav"),
    sound!("soft-hitclap.wav"),
    sound!("soft-sliderslide.wav"),
    sound!("soft-slidertick.wav"),
    sound!("soft-sliderwhistle.wav"),
    sound!("drum-hitnormal.wav"),
    sound!("drum-hitwhistle.wav"),
    sound!("drum-hitfinish.wav"),
    sound!("drum-hitclap.wav"),
    sound!("drum-sliderslide.wav"),
    sound!("drum-slidertick.wav"),
    sound!("drum-sliderwhistle.wav"),
];

/// The temporary folder the shipped sounds are written to, so the audio code can read them like
/// any other file. Its name carries a hash of the sounds, so another dawsu! version never reads
/// this version's files.
fn folder() -> &'static PathBuf {
    static FOLDER: OnceLock<PathBuf> = OnceLock::new();
    FOLDER.get_or_init(|| {
        let hash = DEFAULT_SOUNDS
            .iter()
            .flat_map(|(name, bytes)| name.as_bytes().iter().chain(bytes.iter()))
            .fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3));
        std::env::temp_dir().join(format!("dawsu-rs-default-sounds-{hash:016x}"))
    })
}

/// The shipped sound named `file_name` (any letter case, such as `soft-hitclap.wav`), on
/// disk. It is written again when the file is missing or changed. `None` when dawsu! ships no
/// sound by that name, or the temporary folder can't be written.
pub(crate) fn path(file_name: &str) -> Option<PathBuf> {
    path_in(folder(), file_name)
}

fn path_in(folder: &Path, file_name: &str) -> Option<PathBuf> {
    let (name, bytes) = DEFAULT_SOUNDS.iter().find(|(name, _)| name.eq_ignore_ascii_case(file_name))?;
    let target = folder.join(name);
    let intact = || std::fs::read(&target).is_ok_and(|on_disk| on_disk == *bytes);
    if intact() {
        return Some(target);
    }
    std::fs::create_dir_all(folder).ok()?;
    // Each writer has its own part file, so two tests or two dawsu! windows never mix bytes.
    static PART: AtomicU64 = AtomicU64::new(0);
    let part = folder.join(format!("{name}.{}-{}.part", std::process::id(), PART.fetch_add(1, Ordering::Relaxed)));
    if std::fs::write(&part, bytes).is_err() {
        let _ = std::fs::remove_file(&part);
        return None;
    }
    let moved = std::fs::rename(&part, &target).is_ok();
    if !moved {
        let _ = std::fs::remove_file(&part);
    }
    (moved || intact()).then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETS: [&str; 3] = ["normal", "soft", "drum"];
    const SOUNDS: [&str; 7] = ["hitnormal", "hitwhistle", "hitfinish", "hitclap", "sliderslide", "slidertick", "sliderwhistle"];

    #[test]
    fn every_set_has_its_hit_and_slider_sounds() {
        for set in SETS {
            for sound in SOUNDS {
                let name = format!("{set}-{sound}.wav");
                let path = path(&name).unwrap_or_else(|| panic!("{name} is shipped"));
                assert_eq!(path.file_name().unwrap(), name.as_str());
                let (_, bytes) = DEFAULT_SOUNDS.iter().find(|(n, _)| *n == name).unwrap();
                assert_eq!(std::fs::read(&path).unwrap(), *bytes, "{name} on disk");
                let clip = dawsu_audio::decode(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert!(!clip.samples.is_empty() && clip.samples.iter().any(|s| s.abs() > 0.01), "{name} is not silent");
            }
        }
    }

    #[test]
    fn the_files_are_the_unchanged_osu_resources_files() {
        // Sizes from `git ls-tree -l 2026.1021.0 osu.Game.Resources/Skins/Legacy/` in ppy/osu-resources.
        let upstream = [
            ("normal-hitnormal.wav", 201_380),
            ("normal-hitwhistle.wav", 213_964),
            ("normal-hitfinish.wav", 300_336),
            ("normal-hitclap.wav", 18_368),
            ("normal-sliderslide.wav", 223_744),
            ("normal-slidertick.wav", 94_284),
            ("normal-sliderwhistle.wav", 67_256),
            ("soft-hitnormal.wav", 145_508),
            ("soft-hitwhistle.wav", 145_548),
            ("soft-hitfinish.wav", 402_392),
            ("soft-hitclap.wav", 19_476),
            ("soft-sliderslide.wav", 103_760),
            ("soft-slidertick.wav", 117_804),
            ("soft-sliderwhistle.wav", 47_580),
            ("drum-hitnormal.wav", 145_508),
            ("drum-hitwhistle.wav", 70_492),
            ("drum-hitfinish.wav", 115_100),
            ("drum-hitclap.wav", 115_528),
            ("drum-sliderslide.wav", 103_760),
            ("drum-slidertick.wav", 68_364),
            ("drum-sliderwhistle.wav", 67_256),
        ];        for ((name, bytes), (expected, size)) in DEFAULT_SOUNDS.iter().zip(upstream) {
            assert_eq!((*name, bytes.len()), (expected, size));
            assert_eq!(&bytes[..4], b"RIFF", "{name}");
            assert_eq!(&bytes[8..12], b"WAVE", "{name}");
        }
    }

    #[test]
    fn names_match_in_any_letter_case_and_other_names_are_not_shipped() {
        assert_eq!(path("Soft-SliderSlide.WAV").unwrap().file_name().unwrap(), "soft-sliderslide.wav");
        for name in ["soft-hitclap2.wav", "soft-sliderslide2.wav", "taiko-soft-hitnormal.wav", "combobreak.wav", "soft-sliderslide.ogg", "sliderslide.wav", ""] {
            assert_eq!(path(name), None, "{name}");
        }
    }

    #[test]
    fn a_missing_or_changed_file_is_written_again() {
        const DRUM_TICK: &[u8] = DEFAULT_SOUNDS[19].1;
        let scratch = dawsu_test_support::Scratch::new("default-sounds");
        let folder = scratch.path("sounds");
        let path = path_in(&folder, "drum-slidertick.wav").unwrap();
        assert_eq!(path, folder.join("drum-slidertick.wav"));
        assert_eq!(std::fs::read(&path).unwrap(), DRUM_TICK);
        std::fs::write(&path, b"changed").unwrap();
        assert_eq!(path_in(&folder, "drum-slidertick.wav").unwrap(), path);
        assert_eq!(std::fs::read(&path).unwrap(), DRUM_TICK, "a changed file is written again");
        std::fs::remove_file(&path).unwrap();
        path_in(&folder, "drum-slidertick.wav").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), DRUM_TICK, "a missing file is written again");
        let left: Vec<_> = std::fs::read_dir(&folder).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, ["drum-slidertick.wav"], "no part file is left");
    }
}
