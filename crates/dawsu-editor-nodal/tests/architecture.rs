//! Structural checks for `dawsu-editor-nodal` (Layer 5 · Editing), enforcing Rule R9
//! (no reference to `dawsu-graph-audio` or `dawsu-graph-stems`) and no static mutable state.

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
fn crate_respects_module_map_and_rule_r9() {
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
    assert_eq!(
        dawsu_deps,
        [
            "dawsu-editor",
            "dawsu-editor-composer",
            "dawsu-foundation",
            "dawsu-graph",
            "dawsu-hitsounding",
        ]
    );
}

#[test]
fn no_forbidden_node_library_audio_format_or_ui_references() {
    let banned = [
        "dawsu_graph_audio",
        "dawsu_graph_stems",
        "dawsu_audio",
        "dawsu_osu_format",
        "dawsu_application",
        "dawsu_ui",
        "tauri",
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

#[test]
fn no_static_mutable_state() {
    for (name, text) in sources() {
        for banned in [
            "static mut",
            "thread_local!",
            "OnceLock",
            "LazyLock",
            "Mutex",
            "RefCell",
            "AtomicU",
        ] {
            assert!(!text.contains(banned), "{name} contains {banned}");
        }
    }
}
