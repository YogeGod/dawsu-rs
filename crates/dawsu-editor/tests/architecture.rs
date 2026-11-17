//! Structural checks: the crate's references (module map, ADR 0024), no static mutable state
//! (CS-12), and no dispatch on id strings (invariant 9).

use std::fs;
use std::path::Path;

fn sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
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
fn the_crate_references_dawsu_foundation_only() {
    let manifest =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap();
    let names: Vec<&str> = deps
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('=').next().unwrap().trim())
        .collect();
    assert_eq!(names, ["dawsu-foundation"]);
    assert!(
        !manifest.contains("[dev-dependencies]"),
        "tests use no other crate either"
    );
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

#[test]
fn i9_no_command_dispatch_is_a_match_over_id_strings() {
    for (name, text) in sources() {
        for (n, line) in text.lines().enumerate() {
            let t = line.trim_start();
            let is_string_arm = t.starts_with('"') && t.contains("\" =>");
            assert!(
                !is_string_arm,
                "{name}:{}: a match arm on a string: {line}",
                n + 1
            );
            assert!(!t.contains("match id"), "{name}:{}: {line}", n + 1);
        }
    }
}
