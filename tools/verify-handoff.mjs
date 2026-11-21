#!/usr/bin/env node
// One-command handoff verifier for dawsu-rs.
//
// Usage: node tools/verify-handoff.mjs
//
// Checks:
//   1. Git working tree cleanliness (warns if dirty)
//   2. npm run build in apps/tauri/ui
//   3. npm test in apps/tauri/ui
//   4. node --test apps/tauri/tests/*.test.mjs
//   5. cargo test --workspace --locked
//   6. git diff --stat origin/main...HEAD with risk flags for Cargo.toml,
//      Cargo.lock, crates/ count changes, capabilities/, and tauri.conf.json

import { spawnSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const uiDir = join(repo, "apps", "tauri", "ui");
const testsDir = join(repo, "apps", "tauri", "tests");
const started = Date.now();

function git(args) {
  return spawnSync("git", args, { cwd: repo, encoding: "utf8" });
}

function section(title) {
  console.log(`\n=== ${title} ===`);
}

function runCommand(label, command, args, options = {}) {
  section(label);
  const t0 = Date.now();
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? repo,
    stdio: "inherit",
    shell: options.shell ?? false,
  });
  const seconds = ((Date.now() - t0) / 1000).toFixed(1);
  if (result.error) {
    console.error(`FAIL: ${label} could not start (${result.error.message})`);
    return { label, status: "FAIL", seconds };
  }
  const ok = result.status === 0;
  console.log(`${ok ? "PASS" : "FAIL"}: ${label} (${seconds}s, exit ${result.status ?? "signal"})`);
  return { label, status: ok ? "PASS" : "FAIL", seconds };
}

function countCratesAtRef(ref) {
  const res = git(["ls-tree", "-d", "--name-only", ref, "crates/"]);
  if (res.status !== 0) return null;
  return res.stdout
    .split(/\r?\n/)
    .map(line => line.trim())
    .filter(Boolean).length;
}

function countCratesOnDisk() {
  const cratesPath = join(repo, "crates");
  if (!existsSync(cratesPath)) return 0;
  return readdirSync(cratesPath, { withFileTypes: true }).filter(entry => entry.isDirectory()).length;
}

function classifySensitivePath(filePath) {
  const normalized = filePath.replaceAll("\\", "/");
  const reasons = [];
  if (normalized === "Cargo.toml" || normalized.endsWith("/Cargo.toml")) {
    reasons.push("Cargo.toml");
  }
  if (normalized === "Cargo.lock" || normalized.endsWith("/Cargo.lock")) {
    reasons.push("Cargo.lock");
  }
  if (normalized.startsWith("capabilities/") || normalized.includes("/capabilities/")) {
    reasons.push("capabilities/");
  }
  if (normalized === "tauri.conf.json" || normalized.endsWith("/tauri.conf.json")) {
    reasons.push("tauri.conf.json");
  }
  return reasons;
}

const results = [];

// 1. Provenance and git status
section("Git status");
const branchRes = git(["rev-parse", "--abbrev-ref", "HEAD"]);
const headRes = git(["rev-parse", "--short", "HEAD"]);
const branch = branchRes.status === 0 ? branchRes.stdout.trim() : "unknown";
const head = headRes.status === 0 ? headRes.stdout.trim() : "unknown";
console.log(`repo:   ${repo}`);
console.log(`branch: ${branch} (${head})`);

const statusRes = git(["status", "--porcelain"]);
const dirtyLines = statusRes.status === 0
  ? statusRes.stdout.split(/\r?\n/).filter(line => line.trim().length > 0)
  : [];

if (statusRes.status !== 0) {
  console.warn("WARN: could not read git status.");
  results.push({ label: "git status", status: "WARN", seconds: "0.0" });
} else if (dirtyLines.length === 0) {
  console.log("PASS: working tree is clean.");
  results.push({ label: "git status", status: "PASS", seconds: "0.0" });
} else {
  console.warn(`WARN: working tree has ${dirtyLines.length} uncommitted change(s):`);
  for (const line of dirtyLines) console.warn(`  ${line}`);
  results.push({ label: "git status", status: "WARN", seconds: "0.0" });
}

// 2. UI build + UI tests
const npmShell = process.platform === "win32";
results.push(runCommand("UI build (npm run build)", "npm", ["run", "build"], { cwd: uiDir, shell: npmShell }));
results.push(runCommand("UI tests (npm test)", "npm", ["test"], { cwd: uiDir, shell: npmShell }));

// 3. Tauri frontend/contract tests (expand *.test.mjs in Node for cross-platform shells)
const testFiles = existsSync(testsDir)
  ? readdirSync(testsDir)
      .filter(name => name.endsWith(".test.mjs"))
      .sort()
      .map(name => join("apps", "tauri", "tests", name))
  : [];

if (testFiles.length === 0) {
  section("Tauri JS tests (node --test apps/tauri/tests/*.test.mjs)");
  console.error(`FAIL: no *.test.mjs files found in ${testsDir}`);
  results.push({ label: "Tauri JS tests", status: "FAIL", seconds: "0.0" });
} else {
  results.push(
    runCommand(
      `Tauri JS tests (${testFiles.length} files)`,
      process.execPath,
      ["--test", ...testFiles],
      { cwd: repo }
    )
  );
}

// 4. Rust workspace tests
results.push(runCommand("Rust tests (cargo test --workspace --locked)", "cargo", ["test", "--workspace", "--locked"]));

// 5. Diff stat against origin/main...HEAD and sensitive-file flags
section("Diff summary (origin/main...HEAD)");
let baseRef = "origin/main";
if (git(["rev-parse", "--verify", baseRef]).status !== 0) {
  if (git(["rev-parse", "--verify", "main"]).status === 0) {
    console.warn("WARN: origin/main not found; falling back to local main.");
    baseRef = "main";
  } else {
    baseRef = null;
    console.warn("WARN: neither origin/main nor main ref was found; skipping diff summary.");
  }
}

const flags = [];
if (baseRef) {
  const diffStatRes = git(["diff", "--stat", `${baseRef}...HEAD`]);
  const statOut = diffStatRes.stdout?.trim() ?? "";
  console.log(statOut || "(no committed diff between " + baseRef + " and HEAD)");

  const diffFilesRes = git(["diff", "--name-only", `${baseRef}...HEAD`]);
  const committedFiles = diffFilesRes.status === 0
    ? diffFilesRes.stdout.split(/\r?\n/).map(line => line.trim()).filter(Boolean)
    : [];

  for (const file of committedFiles) {
    const reasons = classifySensitivePath(file);
    if (reasons.length > 0) {
      flags.push(`[committed ${reasons.join(", ")}] ${file}`);
    }
  }

  const baseCrates = countCratesAtRef(baseRef);
  const headCrates = countCratesAtRef("HEAD");
  const diskCrates = countCratesOnDisk();
  if (baseCrates !== null && headCrates !== null && baseCrates !== headCrates) {
    flags.push(`[crates/ count changed] ${baseRef}: ${baseCrates} -> HEAD: ${headCrates}`);
  }
  if (headCrates !== null && diskCrates !== headCrates) {
    flags.push(`[crates/ count changed in working tree] HEAD: ${headCrates} -> disk: ${diskCrates}`);
  }
}

// Also flag uncommitted sensitive files so they are caught before commit.
for (const line of dirtyLines) {
  const rawPath = line.slice(3).trim();
  const filePath = rawPath.includes(" -> ") ? rawPath.split(" -> ").at(-1) : rawPath;
  const reasons = classifySensitivePath(filePath);
  if (reasons.length > 0) {
    flags.push(`[uncommitted ${reasons.join(", ")}] ${filePath}`);
  }
}

section("Handoff risk flags");
if (flags.length === 0) {
  console.log("None — no edits to Cargo.toml, Cargo.lock, crates/ count, capabilities/, or tauri.conf.json.");
} else {
  for (const flag of flags) console.warn(`FLAG: ${flag}`);
}

// Summary
section("Handoff verification summary");
for (const item of results) {
  console.log(`  [${item.status.padEnd(4)}] ${item.label} (${item.seconds}s)`);
}
const totalSeconds = ((Date.now() - started) / 1000).toFixed(1);
const failed = results.filter(item => item.status === "FAIL");
console.log(`Total time: ${totalSeconds}s — ${failed.length === 0 ? "ALL CHECKS PASSED" : `${failed.length} CHECK(S) FAILED`}`);

if (failed.length > 0) {
  process.exitCode = 1;
}
