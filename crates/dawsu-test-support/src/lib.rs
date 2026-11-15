//! Test-only helpers (ADR 0039): where the shared test files are, and scratch folders.
//! Only tests use this crate; it is not a module of the module map and never ships.

use std::fs;

/// serde_json for the tests that read the oracle's results (ADR 0040).
pub use serde_json;
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

/// SHA-256 of `bytes` as lower-case hex (FIPS 180-4), for the oracle's provenance check.
pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
        0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = bytes.to_vec();
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
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
