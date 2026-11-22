use dawsu_hitsounding::drums::{ggd_kit, read_dawsudrum, read_drm};
use std::fs;
use std::path::Path;

const DRUM_CORPUS_DIR: &str = r"E:\GitHub\dawsu-drum-corpus";

#[test]
fn drum_corpus_read_only_parse_and_validate() {
    let root = Path::new(DRUM_CORPUS_DIR);
    if !root.is_dir() {
        eprintln!("Skipping drum_corpus_read_only_parse_and_validate: {DRUM_CORPUS_DIR} not present");
        return;
    }

    let mut dawsudrum_ok = 0usize;
    let mut drm_ok = 0usize;
    let mut nka_ok = 0usize;
    let mut failures = Vec::new();
    let mut seq = 0usize;
    let mut next_id = || {
        seq += 1;
        format!("00000000-0000-4000-8000-{seq:012}")
    };

    for entry in fs::read_dir(root).unwrap().flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        match ext {
            "dawsudrum" => match read_dawsudrum(&text) {
                Ok(map) => {
                    if let Err(e) = map.validate() {
                        failures.push(format!("{}: validate failed: {e:?}", path.display()));
                    } else {
                        dawsudrum_ok += 1;
                    }
                }
                Err(e) => failures.push(format!("{}: read_dawsudrum failed: {e:?}", path.display())),
            },
            "drm" => match read_drm(&text, "Corpus DRM", &mut next_id) {
                Ok(map) => {
                    if let Err(e) = map.validate() {
                        failures.push(format!("{}: validate failed: {e:?}", path.display()));
                    } else {
                        drm_ok += 1;
                    }
                }
                Err(e) => failures.push(format!("{}: read_drm failed: {e:?}", path.display())),
            },
            "nka" => match ggd_kit(&text) {
                Ok(kit) if !kit.is_empty() => nka_ok += 1,
                Ok(_) => failures.push(format!("{}: ggd_kit returned empty", path.display())),
                Err(dawsu_hitsounding::drums::DrumFileRefusal::NotThisFormat(_)) => {}
                Err(e) => failures.push(format!("{}: ggd_kit failed: {e:?}", path.display())),
            },
            _ => {}
        }
    }

    eprintln!(
        "drum_corpus_read_only_parse_and_validate: dawsudrum={dawsudrum_ok}, drm={drm_ok}, nka={nka_ok}, failures={}",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "Drum corpus failures:\n{}",
        failures.join("\n")
    );
}
