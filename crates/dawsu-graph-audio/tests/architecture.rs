//! Structural checks for `dawsu-graph-audio` (Layer 4 · Engines): dependency boundaries and
//! forbidden audio/domain/UI imports.

use std::fs;
use std::path::{Path, PathBuf};

fn manifest_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn collect_rs(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read_to_string(&path).unwrap(),
            ));
        }
    }
}

#[test]
fn crate_references_foundation_and_graph_only() {
    let manifest = fs::read_to_string(manifest_dir().join("Cargo.toml")).unwrap();
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap();
    let mut dawsu_deps: Vec<&str> = deps
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('=').next().unwrap().trim())
        .filter(|name| name.starts_with("dawsu-"))
        .collect();
    dawsu_deps.sort();
    assert_eq!(dawsu_deps, ["dawsu-foundation", "dawsu-graph"]);
}

#[test]
fn no_forbidden_audio_domain_editor_or_ui_references() {
    let banned = [
        "dawsu_audio",
        "dawsu_hitsounding",
        "dawsu_keysounding",
        "dawsu_osu_format",
        "dawsu_osu_client",
        "dawsu_editor",
        "dawsu_application",
        "dawsu_ui",
        "cpal",
        "symphonia",
        "signalsmith",
        "tauri",
    ];
    let mut files = Vec::new();
    collect_rs(&manifest_dir().join("src"), &mut files);
    for (name, text) in files {
        for (idx, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for word in banned {
                assert!(
                    !line.contains(word),
                    "{name}:{}: forbidden reference {word}: {line}",
                    idx + 1
                );
            }
        }
    }
}

#[test]
fn no_static_mutable_state_or_hardcoded_drum_dispatch_in_nodes() {
    let mut files = Vec::new();
    collect_rs(&manifest_dir().join("src").join("nodes"), &mut files);
    for (name, text) in files {
        for banned in [
            "static mut",
            "thread_local!",
            "OnceLock",
            "LazyLock",
            "Mutex",
            "RefCell",
            "AtomicU",
            "\"kick\"",
            "\"snare\"",
        ] {
            assert!(!text.contains(banned), "{name} contains {banned}");
        }
    }
}
