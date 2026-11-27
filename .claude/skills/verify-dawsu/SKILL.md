---
name: verify-dawsu
description: Verify the dawsu! Rust workspace before a commit, pull request, or release. Run Cargo checks and report actual test counts, corpus coverage, and any separately tested Tauri behavior.
---

# Verify dawsu!

Run from the Rust checkout root with the toolchain in `rust-toolchain.toml`. Use the checkout's build notes for local prerequisites. Keep logs and test artifacts in scratch or ignored output directories.

## Fixture coverage

- Use the checked-in save pack and lazer oracle results under `tests/fixtures/`.
- For real-map tests, use `DAWSU_CORPUS` only when it names a verified directory of copied maps. A real directory and a nonzero map count are required. Use a copy for the current checkout or worktree. Keep links and the live osu! Songs folder outside the test path.
- Check the test's fixture requirements before running a corpus comparison. Use `DAWSU_DRUM_CORPUS` for copied drum fixtures and prepare optional oracle data when those checks are in scope. Record the inputs actually exercised.
- Capture test output with `--show-output`. Some corpus tests report a skip and return successfully when their inputs are absent. Report these as skipped coverage alongside Cargo's counts.

## Checks

Run `node tools/verify-handoff.mjs --auto` (add `--with-everyday` on screen, UI, IPC, or workflow changes; `--nextest` uses `cargo-nextest`).

- **UI-only fast-path:** When committed + working-tree diff touches zero `*.rs`, `Cargo.*`, `rust-toolchain.toml`, `apps/tauri/capabilities/*`, `apps/tauri/tauri.conf.json`, or `apps/tauri/icons/*`, skip local Clippy and `cargo test --workspace` (`.github/workflows/windows-checks.yml` runs full Rust gate on PR). UI build, UI unit tests, frontend integration tests, `cargo build -p dawsu-tauri --locked`, and `everyday-check.mjs` stay required.
- **Single gate owner:** Implementer runs targeted tests during TDD plus the full gate once before handoff; `/ready-review` runs only touched tests unless editing code; orchestrator skips re-running an unchanged green gate.

Individual commands (what `verify-handoff.mjs` wraps):

1. **UI bundle & unit tests:** `npm --prefix apps/tauri/ui run build` and `npm --prefix apps/tauri/ui test`.
2. **Frontend integration & capability checks:** on POSIX, `node --test apps/tauri/tests/*.test.mjs`; on Windows `pwsh`, expand first with `$tests = @(Get-ChildItem apps/tauri/tests/*.test.mjs | Sort-Object Name | ForEach-Object { $_.FullName }); node --test @tests`. Chain native commands directly with `&&` — never wrap them inside `& { ... } && ...` (masks non-zero exit codes).
3. **Rust Clippy (CI gate):** `cargo clippy --workspace --all-targets --locked -- -D warnings`.
4. **Rust workspace tests:** `cargo test --workspace --locked` (add `-- --show-output` when auditing skipped corpus tests). For targeted `--release` unit benchmarks/tests (e.g. `songs_index_bench_5000`), pass `--lib` (`cargo test -p <crate> --lib --release --locked -- --ignored ...`) so Cargo does not recompile every integration test binary in release mode.
5. **Built-editor smoke check (UI/IPC/workflow changes):** build `cargo build -p dawsu-tauri --locked`, then run `node tools/everyday-check.mjs --exe <target-dir>/debug/dawsu-tauri.exe` (or run `node tools/verify-handoff.mjs --auto --with-everyday`, which auto-resolves `CARGO_TARGET_DIR` / `cargo metadata` `target_directory` and pre-starts `sccache --start-server` to prevent Windows `os error 10048` races). For 2-wide parallel worktree waves, set `CARGO_TARGET_DIR=<worktree>\target` (using `tools/setup-worktree.ps1`'s `E:` junction) instead of sharing one `CARGO_TARGET_DIR` across concurrent builds. Provenance freshness compares against the latest code commit (`crates`, `apps/tauri`, `Cargo.*`, `rust-toolchain.toml`) and `build-stamp.json`; delete temporary `target/everyday-check/` afterwards.
6. If `apps/tauri/Cargo.toml` defines a separate workspace, also run `cargo test --manifest-path apps/tauri/Cargo.toml --locked -- --show-output` and `cargo build --manifest-path apps/tauri/Cargo.toml --locked`.
7. Record any actual native window, picker, playback, audio listening, or osu! import checks separately.

Run long commands in the background when supported and collect their final exit codes. Use `--offline` when the required dependencies are already cached and the session calls for an offline run.

## Report

For each command, include the result and observed passed, failed, ignored, or skipped counts where applicable. Name the first failing test or build error. Report corpus and native coverage separately with the fixture or platform used. Verify a suspected pre-existing failure against the plan's base in an independent checkout before assigning that cause. End with whether the evidence meets the plan's checks.
