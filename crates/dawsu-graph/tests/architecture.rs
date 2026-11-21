//! Structural checks for `dawsu-graph` (Layer 4 · Engines): dependency boundary and no static mutable state.

use std::fs;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn sources() -> Vec<(String, String)> {
    let dir = manifest_dir().join("src");
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read_to_string(&path).unwrap(),
            ));
        }
    }
    out
}

#[test]
fn crate_references_allowed_dependencies_only() {
    let manifest = fs::read_to_string(manifest_dir().join("Cargo.toml")).unwrap();
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap();
    let dawsu_deps: Vec<&str> = deps
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('=').next().unwrap().trim())
        .filter(|name| name.starts_with("dawsu-"))
        .collect();
    assert_eq!(dawsu_deps, ["dawsu-foundation"]);
    assert!(
        !manifest.contains("[dev-dependencies]"),
        "dawsu-graph tests use no extra dependencies"
    );
}

#[test]
fn no_static_mutable_state_or_wall_clock() {
    for (name, text) in sources() {
        for banned in [
            "static mut",
            "thread_local!",
            "OnceLock",
            "LazyLock",
            "Mutex",
            "RefCell",
            "AtomicU",
            "SystemTime::now",
            "Instant::now",
        ] {
            assert!(!text.contains(banned), "{name} contains {banned}");
        }
    }
}

#[test]
fn no_audio_domain_editor_or_ui_references() {
    let banned = [
        "dawsu_audio",
        "dawsu_graph_audio",
        "dawsu_hitsounding",
        "dawsu_keysounding",
        "dawsu_osu_format",
        "dawsu_osu_client",
        "dawsu_editor",
        "dawsu_application",
        "dawsu_ui",
        "tauri",
        "cpal",
        "symphonia",
        "signalsmith",
    ];
    for (name, text) in sources() {
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
