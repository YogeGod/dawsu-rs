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

1. `cargo check --workspace --all-targets --locked`
2. `cargo test --workspace --locked -- --show-output`
3. `cargo build --workspace --locked`
4. If `apps/tauri/Cargo.toml` defines a separate workspace, also run `cargo test --manifest-path apps/tauri/Cargo.toml --locked -- --show-output` and `cargo build --manifest-path apps/tauri/Cargo.toml --locked`. Check the workspace members with Cargo metadata when the layout is unclear.
5. For an editor UI or workflow change, use the plan's runnable checks and copied map inputs. Record any actual native window, picker, playback, audio listening, or osu! import checks separately.

Run long commands in the background when supported and collect their final exit codes. Use `--offline` when the required dependencies are already cached and the session calls for an offline run.

## Report

For each command, include the result and observed passed, failed, ignored, or skipped counts where applicable. Name the first failing test or build error. Report corpus and native coverage separately with the fixture or platform used. Verify a suspected pre-existing failure against the plan's base in an independent checkout before assigning that cause. End with whether the evidence meets the plan's checks.
