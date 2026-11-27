import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { isRustAffectingPath, resolveTargetDir } from "../../../tools/verify-handoff.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

test("resolveTargetDir gives precedence to envTargetDir, then metadataTargetDir, then repo/target", () => {
  assert.equal(
    resolveTargetDir({
      envTargetDir: "E:\\dev-caches\\cargo-target",
      metadataTargetDir: "E:\\other\\target",
      repoDir: repoRoot,
    }),
    resolve(repoRoot, "E:\\dev-caches\\cargo-target")
  );
  assert.equal(
    resolveTargetDir({
      envTargetDir: "custom-target",
      metadataTargetDir: "E:\\other\\target",
      repoDir: repoRoot,
    }),
    resolve(repoRoot, "custom-target")
  );
  assert.equal(
    resolveTargetDir({
      envTargetDir: "   ",
      metadataTargetDir: "E:\\dev-caches\\cargo-target",
      repoDir: repoRoot,
    }),
    resolve(repoRoot, "E:\\dev-caches\\cargo-target")
  );
  assert.equal(
    resolveTargetDir({
      envTargetDir: "",
      metadataTargetDir: "",
      repoDir: repoRoot,
    }),
    join(repoRoot, "target")
  );
});

test("isRustAffectingPath returns false for UI-only and docs/scripts paths", () => {
  const uiOnlyPaths = [
    "apps/tauri/ui/src/parts/ToastStack.svelte",
    "apps/tauri/tests/confirm.test.mjs",
    "AGENTS.md",
    "NOW.md",
    "tools/everyday-check.mjs",
  ];
  for (const p of uiOnlyPaths) {
    assert.equal(isRustAffectingPath(p), false, `expected false for ${p}`);
  }
});

test("isRustAffectingPath returns true for Rust, Cargo, toolchain, capabilities, tauri.conf.json, icons, and Windows paths", () => {
  const rustPaths = [
    "crates/dawsu-application/src/lib.rs",
    "apps/tauri/build.rs",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "apps/tauri/capabilities/default.json",
    "apps/tauri/tauri.conf.json",
    "apps/tauri/icons/icon.ico",
    "crates\\dawsu-application\\src\\lib.rs",
    "apps\\tauri\\capabilities\\default.json",
    "apps\\tauri\\icons\\icon.png",
  ];
  for (const p of rustPaths) {
    assert.equal(isRustAffectingPath(p), true, `expected true for ${p}`);
  }
});

test("tools/verify-handoff.mjs --help exits 0 and lists supported flags", () => {
  const res = spawnSync(process.execPath, [join(repoRoot, "tools", "verify-handoff.mjs"), "--help"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  assert.equal(res.status, 0, `expected exit 0, got ${res.status}: ${res.stderr}`);
  for (const flag of ["--auto", "--ui-only", "--with-everyday", "--nextest"]) {
    assert.ok(res.stdout.includes(flag), `expected help output to mention ${flag}`);
  }
});
