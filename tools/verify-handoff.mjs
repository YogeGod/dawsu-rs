#!/usr/bin/env node
// One-command handoff verifier for dawsu-rs.
//
// Usage: node tools/verify-handoff.mjs [--auto] [--ui-only] [--with-everyday] [--nextest] [--help]
//
// Checks:
//   1. Git working tree cleanliness (warns if dirty)
//   2. npm run build in apps/tauri/ui
//   3. npm test in apps/tauri/ui
//   4. node --test apps/tauri/tests/*.test.mjs
//   5. cargo clippy --workspace --all-targets --locked -- -D warnings (skipped on UI-only diffs with --auto/--ui-only)
//   6. cargo test --workspace --locked --no-fail-fast (or cargo nextest with --nextest; skipped on UI-only diffs)
//   7. Optional (--with-everyday): cargo build -p dawsu-tauri --locked + node tools/everyday-check.mjs
//   8. git diff --stat origin/main...HEAD with risk flags for Cargo.toml,
//      Cargo.lock, crates/ count changes, capabilities/, and tauri.conf.json

import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const uiDir = join(repo, "apps", "tauri", "ui");
const testsDir = join(repo, "apps", "tauri", "tests");
const DEFAULT_WIN_LIBCLANG_PATH = "E:/Claude/drum-onnx/venv311/Lib/site-packages/clang/native";

export function resolveTargetDir({ envTargetDir = "", metadataTargetDir = "", repoDir = repo } = {}) {
  const envTrimmed = String(envTargetDir ?? "").trim();
  if (envTrimmed) return resolve(repoDir, envTrimmed);
  const metaTrimmed = String(metadataTargetDir ?? "").trim();
  if (metaTrimmed) return resolve(repoDir, metaTrimmed);
  return join(repoDir, "target");
}

export function isRustAffectingPath(filePath) {
  const normalized = String(filePath ?? "")
    .replaceAll("\\", "/")
    .replace(/^\.\//, "");
  if (!normalized) return false;
  if (normalized.endsWith(".rs")) return true;
  if (/(?:^|\/)(?:Cargo\.toml|Cargo\.lock|rust-toolchain\.toml)$/.test(normalized)) return true;
  if (normalized.startsWith("capabilities/") || normalized.includes("/capabilities/")) return true;
  if (normalized === "tauri.conf.json" || normalized.endsWith("/tauri.conf.json")) return true;
  if (normalized.startsWith("apps/tauri/icons/")) return true;
  return false;
}

function parsePorcelainPath(line) {
  const raw = line.slice(3).trim();
  const target = raw.includes(" -> ") ? raw.split(" -> ").at(-1) : raw;
  return target.replace(/^"(.*)"$/, "$1");
}

function parseCliArgs(argv) {
  const opts = {
    auto: false,
    uiOnly: false,
    withEveryday: false,
    nextest: false,
    help: false,
  };
  for (const arg of argv) {
    if (arg === "--auto") opts.auto = true;
    else if (arg === "--ui-only") opts.uiOnly = true;
    else if (arg === "--with-everyday") opts.withEveryday = true;
    else if (arg === "--nextest") opts.nextest = true;
    else if (arg === "--help" || arg === "-h") opts.help = true;
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return opts;
}

function printHelp() {
  console.log(`Usage: node tools/verify-handoff.mjs [options]

Options:
  --auto           Skip local Clippy & Rust workspace tests when no Rust/Cargo/Tauri-config files changed
  --ui-only        Unconditionally skip local Clippy & Rust workspace tests
  --with-everyday  Also run cargo build -p dawsu-tauri --locked + everyday-check.mjs
  --nextest        Use cargo nextest run --workspace --locked --no-fail-fast for Rust tests
  --help, -h       Show this help and exit`);
}

function git(args) {
  return spawnSync("git", args, { cwd: repo, encoding: "utf8" });
}

function section(title) {
  console.log(`\n=== ${title} ===`);
}

function buildRustEnv() {
  if (process.platform !== "win32" || process.env.LIBCLANG_PATH) {
    return process.env;
  }
  if (existsSync(DEFAULT_WIN_LIBCLANG_PATH)) {
    return { ...process.env, LIBCLANG_PATH: DEFAULT_WIN_LIBCLANG_PATH };
  }
  return process.env;
}

function detectCargoTargetDir(rustEnv) {
  const envTargetDir = rustEnv.CARGO_TARGET_DIR;
  if (envTargetDir && envTargetDir.trim()) {
    return resolveTargetDir({ envTargetDir, repoDir: repo });
  }
  const res = spawnSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
    cwd: repo,
    env: rustEnv,
    encoding: "utf8",
  });
  let metadataTargetDir = "";
  if (res.status === 0 && res.stdout) {
    try {
      metadataTargetDir = JSON.parse(res.stdout)?.target_directory ?? "";
    } catch {}
  }
  return resolveTargetDir({ envTargetDir: "", metadataTargetDir, repoDir: repo });
}

function ensureSccacheServer(rustEnv) {
  const wrapper = String(rustEnv.RUSTC_WRAPPER ?? "").trim();
  let active = wrapper.toLowerCase().includes("sccache");
  if (!active) {
    const cargoHome = rustEnv.CARGO_HOME?.trim() || join(homedir(), ".cargo");
    for (const name of ["config.toml", "config"]) {
      const cfgPath = join(cargoHome, name);
      if (existsSync(cfgPath)) {
        try {
          if (readFileSync(cfgPath, "utf8").toLowerCase().includes("sccache")) {
            active = true;
            break;
          }
        } catch {}
      }
    }
  }
  if (!active) return;

  const candidates = [];
  if (wrapper.toLowerCase().includes("sccache")) candidates.push(wrapper);
  if (process.platform === "win32" && process.env.LOCALAPPDATA) {
    candidates.push(join(process.env.LOCALAPPDATA, "Microsoft", "WinGet", "Links", "sccache.exe"));
  }
  candidates.push("sccache");
  const bin = candidates.find((c, i) => i === candidates.length - 1 || existsSync(c));
  try {
    spawnSync(bin, ["--start-server"], { cwd: repo, env: rustEnv, stdio: "ignore" });
  } catch {}
}

function runCommand(label, command, args, options = {}) {
  section(label);
  const t0 = Date.now();
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? repo,
    env: options.env ?? process.env,
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

function main() {
  let opts;
  try {
    opts = parseCliArgs(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    printHelp();
    process.exitCode = 1;
    return;
  }

  if (opts.help) {
    printHelp();
    return;
  }

  const started = Date.now();
  const results = [];
  const rustEnv = buildRustEnv();

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

  // Resolve baseRef before diff checks & --auto
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

  let committedFiles = [];
  if (baseRef) {
    const diffFilesRes = git(["diff", "--name-only", `${baseRef}...HEAD`]);
    committedFiles = diffFilesRes.status === 0
      ? diffFilesRes.stdout.split(/\r?\n/).map(line => line.trim()).filter(Boolean)
      : [];
  }

  const uallStatusRes = git(["status", "--porcelain", "-uall"]);
  const dirtyPaths = uallStatusRes.status === 0
    ? uallStatusRes.stdout.split(/\r?\n/).filter(line => line.trim().length > 0).map(parsePorcelainPath)
    : dirtyLines.map(parsePorcelainPath);

  let skipRustChecks = false;
  let skipReason = "";
  if (opts.uiOnly) {
    skipRustChecks = true;
    skipReason = "--ui-only passed";
  } else if (opts.auto && baseRef) {
    const changedPaths = [...new Set([...committedFiles, ...dirtyPaths])];
    if (!changedPaths.some(isRustAffectingPath)) {
      skipRustChecks = true;
      skipReason = "UI-only diff; GitHub Actions runs full Rust gate on PR";
    }
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

  // 4. Rust Clippy + Workspace Tests
  let sccacheStarted = false;
  const startSccacheOnce = () => {
    if (!sccacheStarted) {
      sccacheStarted = true;
      ensureSccacheServer(rustEnv);
    }
  };

  const clippyLabel = "Clippy lints (cargo clippy --workspace --all-targets --locked -- -D warnings)";
  const testLabel = opts.nextest
    ? "Rust tests (cargo nextest run --workspace --locked --no-fail-fast)"
    : "Rust tests (cargo test --workspace --locked --no-fail-fast)";

  if (skipRustChecks) {
    section("Rust Clippy & Workspace Tests");
    console.log(`SKIP: ${clippyLabel} (${skipReason})`);
    console.log(`SKIP: ${testLabel} (${skipReason})`);
    results.push({ label: clippyLabel, status: "SKIP", seconds: "0.0" });
    results.push({ label: testLabel, status: "SKIP", seconds: "0.0" });
  } else {
    startSccacheOnce();
    results.push(
      runCommand(
        clippyLabel,
        "cargo",
        ["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"],
        { env: rustEnv }
      )
    );
    const testArgs = opts.nextest
      ? ["nextest", "run", "--workspace", "--locked", "--no-fail-fast"]
      : ["test", "--workspace", "--locked", "--no-fail-fast"];
    results.push(runCommand(testLabel, "cargo", testArgs, { env: rustEnv }));
  }

  // 5. Optional built-editor smoke check (--with-everyday)
  if (opts.withEveryday) {
    startSccacheOnce();
    const buildRes = runCommand(
      "Editor build (cargo build -p dawsu-tauri --locked)",
      "cargo",
      ["build", "-p", "dawsu-tauri", "--locked"],
      { env: rustEnv }
    );
    results.push(buildRes);

    if (buildRes.status === "PASS") {
      const targetDir = detectCargoTargetDir(rustEnv);
      const exePath = join(
        targetDir,
        "debug",
        process.platform === "win32" ? "dawsu-tauri.exe" : "dawsu-tauri"
      );
      let everydayPassed = false;
      try {
        const checkRes = runCommand(
          "Everyday check (node tools/everyday-check.mjs)",
          process.execPath,
          [join("tools", "everyday-check.mjs"), "--exe", exePath],
          { cwd: repo }
        );
        results.push(checkRes);
        everydayPassed = checkRes.status === "PASS";
      } finally {
        if (everydayPassed) {
          const everydayDir = join(repo, "target", "everyday-check");
          if (existsSync(everydayDir)) {
            rmSync(everydayDir, { recursive: true, force: true, maxRetries: 10, retryDelay: 150 });
          }
        }
      }
    } else {
      section("Everyday check (node tools/everyday-check.mjs)");
      console.log("SKIP: skipped because dawsu-tauri build failed.");
      results.push({ label: "Everyday check (node tools/everyday-check.mjs)", status: "SKIP", seconds: "0.0" });
    }
  }

  // 6. Diff stat against origin/main...HEAD and sensitive-file flags
  section("Diff summary (origin/main...HEAD)");
  const flags = [];
  if (baseRef) {
    const diffStatRes = git(["diff", "--stat", `${baseRef}...HEAD`]);
    const statOut = diffStatRes.stdout?.trim() ?? "";
    console.log(statOut || "(no committed diff between " + baseRef + " and HEAD)");

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
  for (const filePath of dirtyPaths) {
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
}

const thisFile = fileURLToPath(import.meta.url);
const entryFile = process.argv[1] ? resolve(process.argv[1]) : "";
const isEntry = process.platform === "win32"
  ? entryFile.toLowerCase() === thisFile.toLowerCase()
  : entryFile === thisFile;
if (isEntry) {
  main();
}
