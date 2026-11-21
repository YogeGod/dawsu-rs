//! Test-only helpers (ADR 0039): where the shared test files are, and scratch folders.
//! Only tests use this crate; it is not a module of the module map and never ships.

use std::fs;

/// serde_json for the tests that read the oracle's results (ADR 0040).
pub use serde_json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// The root of the dawsu-rs workspace.
pub fn workspace_root() -> PathBuf {
    // Prefer runtime CARGO_MANIFEST_DIR (set by Cargo for the active checkout) so a shared
    // CARGO_TARGET_DIR across throwaway worktrees never reads fixtures from a deleted checkout.
    let start = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    for dir in start.ancestors() {
        if dir.join("Cargo.toml").is_file() && dir.join("crates").join("dawsu-test-support").is_dir() {
            return dir.canonicalize().expect("workspace root");
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").canonicalize().expect("workspace root")
}

/// `tests/fixtures/save-pack/` (ADR 0033). Tests read it and never write to it.
pub fn save_pack_dir() -> PathBuf {
    workspace_root().join("tests").join("fixtures").join("save-pack")
}

/// Every `.osu` file of the save pack as (name relative to the pack, path), sorted by name.
pub fn save_pack_files() -> Vec<(String, PathBuf)> {
    let root = save_pack_dir();
    let mut out = Vec::new();
    for group in ["owner", "synthetic"] {
        for entry in fs::read_dir(root.join(group)).expect("save pack folder") {
            let path = entry.expect("save pack entry").path();
            if path.extension().is_some_and(|e| e == "osu") {
                let name = format!("{group}/{}", path.file_name().unwrap().to_string_lossy());
                out.push((name, path));
            }
        }
    }
    out.sort();
    out
}

/// One save-pack file's bytes, by its name relative to the pack (`owner/catalyst-impetus-std.osu`).
pub fn save_pack_bytes(name: &str) -> Vec<u8> {
    fs::read(save_pack_dir().join(name)).unwrap_or_else(|e| panic!("save pack file {name}: {e}"))
}

/// `tests/fixtures/oracle-cases/`: hand-made maps for the lazer oracle (ADR 0039).
pub fn oracle_cases_dir() -> PathBuf {
    workspace_root().join("tests").join("fixtures").join("oracle-cases")
}

/// Every `.osu` file of the hand-made oracle cases as (name, path), sorted by name.
pub fn oracle_case_files() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = fs::read_dir(oracle_cases_dir())
        .expect("oracle cases folder")
        .map(|e| e.expect("oracle case").path())
        .filter(|p| p.extension().is_some_and(|e| e == "osu"))
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    out.sort();
    out
}

/// `tests/fixtures/lazer-oracle/`: the lazer oracle's committed results (ADRs 0026, 0039).
pub fn oracle_results_dir() -> PathBuf {
    workspace_root().join("tests").join("fixtures").join("lazer-oracle")
}

/// The lazer oracle's result for one input map, checked against that map's bytes: the header
/// must name the same SHA-256, so a result never silently belongs to another version of the map.
/// `group` is the input folder's name (`save-pack` or `oracle-cases`).
pub fn oracle_result(results_dir: &Path, group: &str, name: &str, input: &[u8]) -> serde_json::Value {
    let path = results_dir.join(group).join(format!("{name}.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("oracle result {}: {e}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("oracle result {}: {e}", path.display()));
    let recorded = value["header"]["input_sha256"].as_str().unwrap_or_default();
    assert_eq!(recorded, sha256_hex(input), "the oracle result {} was made from different bytes; run the lazer oracle again", path.display());
    value
}

pub use dawsu_foundation::digest::sha256_hex;

static NEXT: AtomicU32 = AtomicU32::new(0);

/// A fresh folder for one test in the system's temporary folder, removed when dropped.
/// Tests run in parallel, so every scratch folder is unique.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Scratch {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("dawsu-test-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch folder");
        Scratch(dir)
    }

    pub fn dir(&self) -> &Path {
        &self.0
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_test_vectors() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn the_save_pack_has_its_17_files() {
        assert_eq!(save_pack_files().len(), 17);
    }
}
