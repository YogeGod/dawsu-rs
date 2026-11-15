//! Test-only helpers (ADR 0039): where the shared test files are, and scratch folders.
//! Only tests use this crate; it is not a module of the module map and never ships.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// The root of the dawsu-rs workspace.
pub fn workspace_root() -> PathBuf {
    // This crate sits in <root>/crates/dawsu-test-support.
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
    fn the_save_pack_has_its_17_files() {
        assert_eq!(save_pack_files().len(), 17);
    }
}
