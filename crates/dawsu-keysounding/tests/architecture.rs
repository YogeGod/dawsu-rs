//! Structural checks for `dawsu-keysounding`: enforces TDD Invariant 1 (allowed dawsu dependencies only).

use std::fs;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

#[test]
fn crate_references_allowed_dawsu_dependencies_only() {
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
        ["dawsu-foundation", "dawsu-hitsounding", "dawsu-osu-format"]
    );
}
