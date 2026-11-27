# Development Guide

Developer setup, architecture overview, build/test commands, and local tooling for **dawsu!**.

If you are an osu! mapper looking to download and use dawsu!, see [README.md](../README.md) or [Setting Up](wiki/Setting-Up.md).

## Architecture Overview

- **Rust core & domain logic** (`crates/`): all beatmap state, `.osu`/`.osb` parsing and byte-preserving writes, timing, snapping, undo/redo, compile/decompile, audio playback, stem separation, drum hit detection, node graph, keysound synthesizer/SF2/SFZ sampler, and export live in the 14 workspace crates (see [Cargo.toml](../Cargo.toml)).
- **Desktop shell** (`apps/tauri/`): Tauri v2 desktop host (ADR 0063) with O(1) `ViewStateToken` scene diffing (`apps/tauri/src/scene.rs`), typed domain action seams, and IPC commands synced with `apps/tauri/capabilities/default.json`.
- **UI & timeline** (`apps/tauri/ui/`): Svelte 5 (runes only) chrome/modals bundled by Vite into `apps/tauri/dist/ui/`, paired with a `<canvas>` dense timeline surface (ruler, waveform, grid, hit diamonds, selection/volume overlays, overview strip).
- **Native VST3 sidecar** (`native/vst3-host/`): out-of-process C++ VST3 host executable (`dawsu_vst3_host.exe`) built via CMake against the Steinberg VST 3 SDK (`v3.8.1_build_84`).
- **Behavioural oracle** (`tools/lazer-oracle/`): C# oracle referencing osu!lazer (`ppy/osu` pinned at `131fe72`) to verify timing, slider, combo, and audio decode behaviour.

Related project docs:

- [AGENTS.md](../AGENTS.md) — coding invariants, verification rules, and workflow hygiene
- [PRODUCT.md](../PRODUCT.md) — product scope, stack, and principles
- [NOW.md](../NOW.md) — active sprint state and recent milestones
- [FEATURE-COVERAGE.md](../FEATURE-COVERAGE.md) — capability ledger and test evidence
- [THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md) — third-party licences and attribution

## Prerequisites

- **Rust toolchain:** pinned by [rust-toolchain.toml](../rust-toolchain.toml) (`1.100.0`, Edition 2024) and [Cargo.lock](../Cargo.lock). Always pass `--locked` to Cargo commands.
- **Node.js & npm:** Node.js 22+ for the Svelte 5 / Vite frontend (`apps/tauri/ui/`) and JS test suites.
- **C++ toolchain & `libclang`:** MSVC x64 build tools (on Windows) and LLVM `libclang` (`LIBCLANG_PATH`), required to build `signalsmith-stretch`.
- **Optional:**
  - **CMake + Steinberg VST 3 SDK (`v3.8.1_build_84`):** to build `dawsu_vst3_host.exe` (`native/vst3-host`).
  - **Inno Setup 6 (`ISCC.exe`):** to build the Windows per-user Setup installer (`installer/dawsu.iss`).
  - **.NET SDK:** to run `tools/lazer-oracle`.
  - **Playwright Core:** for `tools/everyday-check.mjs` and `tools/ui-screenshot.mjs` (discovered via `DAWSU_PLAYWRIGHT_CORE` or the npm `_npx` cache; nothing is auto-installed).

## Build and Test

All line endings in the repo must be LF (`\n`). Run these verification gates before committing or opening a PR:

### Frontend (Svelte UI & Integration Tests)

```sh
npm --prefix apps/tauri/ui run build
npm --prefix apps/tauri/ui test
```

Frontend integration tests:

```sh
node --test apps/tauri/tests/*.test.mjs
```

On Windows PowerShell (`pwsh`), expand the glob first (do not wrap native commands inside `& { ... } && ...`):

```pwsh
$tests = @(Get-ChildItem apps/tauri/tests/*.test.mjs | Sort-Object Name | ForEach-Object { $_.FullName })
node --test @tests
```

### Rust Workspace & Tauri App

```sh
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build -p dawsu-tauri --locked
```

Build release binary or run directly against a **copied** difficulty:

```sh
cargo build --release -p dawsu-tauri --locked
cargo run -p dawsu-tauri --locked -- "path/to/a/copied-difficulty.osu"
```

The Windows binary is `dawsu-tauri.exe`. macOS and Linux build and runtime checks remain open.

## Local Windows Notes

- Set `LIBCLANG_PATH` to your LLVM `libclang` folder before building.
- Keeping Cargo's build output and temporary files on a drive with plenty of free space helps; `tools/setup-worktree.ps1` can set this up.
- The osu!lazer source used as the behavioural reference is `ppy/osu` at `131fe72`.

### Local ASIO Build on Windows

ASIO is available in the optional Tauri development build with `--features asio-local`:

1. Set `CPAL_ASIO_DIR` to the extracted Steinberg ASIO SDK.
2. Set `LIBCLANG_PATH` and load the MSVC x64 developer environment before running Cargo.
3. Use a separate target directory for the ASIO build.

The standard Windows release build uses WASAPI, DirectSound, and MME. The ASIO driver is loaded when selected; while active, switch back to WASAPI before changing the ASIO driver or buffer size. Publication of an ASIO build requires the agreement and attribution recorded in the vault's ASIO decision.

## Developer Tools & Harnesses

- **Real-app smoke check (`tools/everyday-check.mjs`):**
  ```sh
  node tools/everyday-check.mjs --exe target/debug/dawsu-tauri.exe
  ```
  Opens the real built editor on a copy of a fixture map and exercises menus, tools, snap, placing/moving/deleting hits, undo/redo, panels, dialogs, windows, and Save, then verifies files on disk and checks exe mtime against `HEAD` and `apps/tauri/dist/ui/build-stamp.json`. Options: `--map`, `--shots`, `--json`, `--keep-profile`, `--allow-stale`. See [AGENTS.md](../AGENTS.md#verification).
- **UI screenshots (`tools/ui-screenshot.mjs`):**
  ```sh
  node tools/ui-screenshot.mjs --exe target/debug/dawsu-tauri.exe --map "path/to/copied.osu" --out shot.png
  ```
  Screenshots the built editor on a temporary copy of a mapset. Options: `--width`, `--height`, `--eval`, `--wait`, `--preset`, `--csp`, `--window`, `--eval-window`, `--profile`, `--keep-profile`.
- **Synthetic demo mapset generator (`tools/make-demo-mapset.mjs`):**
  ```sh
  node tools/make-demo-mapset.mjs [output-dir]
  ```
  Generates a licence-free 2-difficulty synthetic mapset (`demo-song.wav` + 3 WAV samples + `[Hard].osu` + `[Normal].osu`) for local QA and screenshots.
- **Performance benchmark (`tools/perf-measure.mjs`):** measures map load and playhead timings.
- **osu!lazer oracle (`tools/lazer-oracle`):** verifies osu! format, timing, slider, combo, and audio decode behaviour against osu!lazer ([its README](../tools/lazer-oracle/README.md)):
  ```sh
  dotnet run -c Release --project tools/lazer-oracle -- tests/fixtures/lazer-oracle tests/fixtures/save-pack tests/fixtures/oracle-cases
  ```
- **Windows release packaging (`tools/package-release.ps1`):** stages `dawsu-tauri.exe`, `dawsu_vst3_host.exe` (when built), docs, and assets; produces `-portable.zip`, `-setup.exe` (via Inno Setup), and `SHA256SUMS.txt` under `dist/release/`.

These tools require Windows (WebView2), except `tools/lazer-oracle`.

## CI Workflows

- [windows-checks.yml](../.github/workflows/windows-checks.yml) — currently disabled on GitHub, so run its checks locally (see [AGENTS.md](../AGENTS.md#verification)). When enabled, it runs Svelte UI build/tests, frontend integration tests, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked` on Windows for PRs touching non-Markdown files and on pushes to `main`.
- [release.yml](../.github/workflows/release.yml) — builds the VST3 sidecar, UI bundle, release binary, Inno Setup installer, portable ZIP, and `SHA256SUMS.txt` on `v*` tags or manual dispatch.
