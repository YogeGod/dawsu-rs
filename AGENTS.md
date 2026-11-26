# AGENTS.md — dawsu! (`dawsu-rs`)

Working rules for coding agents in this repository. Keep edits cross-platform (Windows, macOS, Linux) and use LF line endings.

## Architecture invariants

- **Rust is the source of truth.** All beatmap state, `.osu`/`.osb` parsing and byte-preserving writes, timing, snapping, undo/redo, compile/decompile, audio playback, and export live in `crates/` and `apps/tauri/src/`. The web UI sends typed actions through the bridge and renders the scene Rust returns; keep game and editing logic in Rust.
- **osu!lazer is the behavioural oracle.** Verify osu! format, timing, slider, combo, and audio decode behaviour against `E:/dawsu-source-cache/ppy-osu-131fe72` and `tools/lazer-oracle`. When the lazer pin moves, re-run the oracle and commit updated fixtures together.
- **Canvas stays Canvas.** Keep the dense timeline surface (ruler, waveform, neutral grid, `Song · objects` row, hit diamonds, selection/volume overlays, overview strip) on `<canvas>` for Chromium and WebKit frame rate. Build surrounding chrome, panels, toolbars, and dialogs in Svelte 5.
- **Svelte 5 runes only.** Write all components under `apps/tauri/ui/` with Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`, `$bindable`). Keep legacy Svelte 4 syntax (`export let`, `$:`, `createEventDispatcher`, writable stores) out of the codebase. Put pure, unit-testable UI helpers in plain `.js` modules under `apps/tauri/ui/src/lib/`.
- **Encapsulated Application view state (`ViewStateToken`).** Keep UI-observed `Application` fields `pub(crate)` in `crates/dawsu-application`; bump `view_revision` on every UI-visible mutation so `apps/tauri/src/scene.rs` detects changes in O(1) via `ViewStateToken`. Do not re-expose raw `pub` view fields or re-introduce per-tick scene scanning.
- **Typed domain action seams & literal `invoke()`.** Route related main-window Tauri commands through typed domain action seams (`sound_set_action`, `mapset_revert_action`, `shared_base_action`, `stem_separation_action`, `midi_action`, `keysounds_action`) instead of adding single-purpose commands. Every `invoke("...")` in `apps/tauri/ui/src/` must pass a string literal (enforced by `apps/tauri/tests/capabilities.test.mjs`) and stay synced with `apps/tauri/build.rs` and `apps/tauri/capabilities/default.json`.
- **Single `Bridge` + `registry` chrome seam.** Svelte parts under `apps/tauri/ui/src/parts/` communicate with the editor exclusively via `bridge` (`apps/tauri/ui/src/lib/bridge.svelte.js`) and `registry.js`. `main.js` exports only `{ commands, bridge }` and boots with `exposeGlobals: false`; never re-add production `globalThis.*` app wiring or pass-through wrappers on `window.dawsuUi` (read `window.dawsuUi.bridge.*` in `tools/everyday-check.mjs`).
- **Crate boundary.** Keep the workspace crate layout stable; add a new crate only when a heavy native dependency (such as ONNX or VST3) forces isolation.
- **Layout menu freeze.** Leave `View > Layout...` untouched; only keep it from breaking.
- **Map & worktree safety.** Test against copied maps or `tests/fixtures/` only. Treat real osu! installs and `E:/GitHub/dawsu-drum-corpus` as read-only. Never modify sibling worktrees (`E:/GitHub/dawsu-rs-*`) or delete branches checked out in another worktree.

## Verification

Run the checks relevant to your change while iterating, and all four before handing off a slice:

- **Rust Clippy (CI gate):** `cargo clippy --workspace --all-targets --locked -- -D warnings`
- **Rust workspace:** `cargo test --workspace --locked`
- **Svelte UI build & unit tests** (in `apps/tauri/ui`): `npm run build` and `npm test`
- **Frontend integration tests:** `node --test apps/tauri/tests/*.test.mjs` (in Windows `pwsh`, expand first: `$tests = @(Get-ChildItem apps/tauri/tests/*.test.mjs | Sort-Object Name | ForEach-Object { $_.FullName }); node --test @tests`, and never wrap native commands inside `& { ... } && ...` which masks non-zero exit codes)
- **osu!lazer oracle** (when changing format/timing/hitsound rules or fixtures):
  `dotnet run -c Release --project tools/lazer-oracle -- tests/fixtures/lazer-oracle tests/fixtures/save-pack tests/fixtures/oracle-cases`

GitHub runs the first three on Windows for every pull request that changes more than `.md` files, and on `main` after each merge (`.github/workflows/windows-checks.yml`); run the oracle yourself when it applies.

**Everyday check (Windows, against the built editor):** after a change to the screen, build the editor (`cargo build -p dawsu-tauri --locked`) and run `node tools/everyday-check.mjs --exe target/debug/dawsu-tauri.exe`. It opens the real `dawsu-tauri.exe` on a copy of a fixture map and uses it like a mapper for about a minute: menus, tools, snap, placing, moving and deleting a hit, undo and redo, panels, dialogs and windows, Save, then checks the files on disk. It verifies exe mtime against `HEAD` and `build-stamp.json` (run against a fresh build before amending test-only edits, or rebuild after Rust/UI edits), writes temporary artifacts to `target/everyday-check/` (remove when done), prints one `PASS`, `FAIL` or `SKIP` line per step, and exits 1 when a step failed. The JS tests above run against a fake page; this one catches a screen that no longer starts or responds. A failing step that shows the app is wrong stays failing: fix the app, not the step.

**Merge gate (Yoge, 2026-11-21):** nothing is merged into `main`, by a person or an agent, unless the everyday check passes on a build of that exact commit. Paste its last line (`everyday check: N of M steps passed`) and the commit id in the pull request. Never merge in the same step that opens a pull request.

**Windows build note:** `signalsmith-stretch` requires `LIBCLANG_PATH` (on this machine: `E:/Claude/drum-onnx/venv311/Lib/site-packages/clang/native`). Keep build and temporary artifacts on `E:`.

## Context pointers

Read only the reference matching your task:

- **Current repo sprint & `app.js` retirement state:** [NOW.md](NOW.md)
- **Feature parity matrix & testing notes:** [FEATURE-COVERAGE.md](FEATURE-COVERAGE.md), [README.md](README.md)
- **Oracle tool & `--audio` mode:** [tools/lazer-oracle/README.md](tools/lazer-oracle/README.md)
- **Design vault (ADRs, active sprint, UI spec, module map):** `E:/Obsidian/Workspace/dawsu-rewrite/` (`DECISIONS.md`, `now.md`, `design/ui-spec.md`, `architecture/module-map.md`) — read when changing architecture, UI layout, or domain rules.
