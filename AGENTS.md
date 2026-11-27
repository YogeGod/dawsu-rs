# AGENTS.md — dawsu! (`dawsu-rs`)

Rules for coding agents in this repo. Keep edits cross-platform (Windows, macOS, Linux), use LF line endings.

## Architecture invariants

- **Rust is the source of truth.** All beatmap state, `.osu`/`.osb` parsing and byte-preserving writes, timing, snapping, undo/redo, compile/decompile, audio playback, export live in `crates/` and `apps/tauri/src/`. Web UI sends typed actions through bridge, renders scene Rust returns; keep game and editing logic in Rust.
- **osu!lazer is the behavioural oracle.** Verify osu! format, timing, slider, combo, audio decode behaviour against `E:/dawsu-source-cache/ppy-osu-131fe72` and `tools/lazer-oracle`. When lazer pin moves, re-run oracle and commit updated fixtures together.
- **Canvas stays Canvas.** Keep dense timeline surface (ruler, waveform, neutral grid, `Song · objects` row, hit diamonds, selection/volume overlays, overview strip) on `<canvas>` for Chromium and WebKit frame rate. Build surrounding chrome, panels, toolbars, dialogs in Svelte 5.
- **Svelte 5 runes only.** Write all components under `apps/tauri/ui/` with Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`, `$bindable`). Keep legacy Svelte 4 syntax (`export let`, `$:`, `createEventDispatcher`, writable stores) out. Put pure, unit-testable UI helpers in plain `.js` modules under `apps/tauri/ui/src/lib/`.
- **Encapsulated Application view state (`ViewStateToken`).** Keep UI-observed `Application` fields `pub(crate)` in `crates/dawsu-application`; bump `view_revision` on every UI-visible mutation so `apps/tauri/src/scene.rs` detects changes in O(1) via `ViewStateToken`. Do not re-expose raw `pub` view fields or re-introduce per-tick scene scanning.
- **Typed domain action seams & literal `invoke()`.** Route related main-window Tauri commands through typed domain action seams (`sound_set_action`, `mapset_revert_action`, `shared_base_action`, `stem_separation_action`, `midi_action`, `keysounds_action`), not new single-purpose commands. Every `invoke("...")` in `apps/tauri/ui/src/` must pass string literal (enforced by `apps/tauri/tests/capabilities.test.mjs`) and stay synced with `apps/tauri/build.rs` and `apps/tauri/capabilities/default.json`.
- **Single `Bridge` + `registry` chrome seam.** Svelte parts under `apps/tauri/ui/src/parts/` talk to editor only via `bridge` (`apps/tauri/ui/src/lib/bridge.svelte.js`) and `registry.js`. `main.js` exports only `{ commands, bridge }`, boots with `exposeGlobals: false`; never re-add production `globalThis.*` app wiring or pass-through wrappers on `window.dawsuUi` (read `window.dawsuUi.bridge.*` in `tools/everyday-check.mjs`).
- **Crate boundary.** Keep workspace crate layout stable; add new crate only when heavy native dependency (ONNX, VST3) forces isolation.
- **Layout menu freeze.** Leave `View > Layout...` untouched; only keep it from breaking.
- **Map & worktree safety.** Test against copied maps or `tests/fixtures/` only. Treat real osu! installs and `E:/GitHub/dawsu-drum-corpus` as read-only. Never modify sibling worktrees (`E:/GitHub/dawsu-rs-*`) or delete branches checked out in another worktree.

## Verification

Run relevant checks while iterating, and handoff gate before PR/merge:

- **One-command verifier:** `node tools/verify-handoff.mjs --auto` (add `--with-everyday` on screen/UI/IPC/workflow changes; `--nextest` uses `cargo-nextest`).
- **UI-only fast-path:** when committed + working-tree diff touches zero `*.rs`, `Cargo.*`, `rust-toolchain.toml`, `apps/tauri/capabilities/*`, `apps/tauri/tauri.conf.json`, or `apps/tauri/icons/*`, skip local Clippy + `cargo test --workspace` (`.github/workflows/windows-checks.yml` runs full Rust gate on PR); UI build, UI unit tests, frontend integration tests, `cargo build -p dawsu-tauri --locked`, and `everyday-check.mjs` stay required.
- **Single gate owner:** implementer runs targeted tests during TDD + full gate once before handoff; `/ready-review` runs only touched tests unless editing code; orchestrator skips re-running an unchanged green gate.
- **Rust Clippy (CI gate):** `cargo clippy --workspace --all-targets --locked -- -D warnings`
- **Rust workspace:** `cargo test --workspace --locked`
- **Svelte UI build & unit tests** (in `apps/tauri/ui`): `npm run build` and `npm test`
- **Frontend integration tests:** `node --test apps/tauri/tests/*.test.mjs` (in Windows `pwsh`, expand first: `$tests = @(Get-ChildItem apps/tauri/tests/*.test.mjs | Sort-Object Name | ForEach-Object { $_.FullName }); node --test @tests`; never wrap native commands inside `& { ... } && ...` — masks non-zero exit codes)
- **osu!lazer oracle** (when changing format/timing/hitsound rules or fixtures):
  `dotnet run -c Release --project tools/lazer-oracle -- tests/fixtures/lazer-oracle tests/fixtures/save-pack tests/fixtures/oracle-cases`

GitHub runs first three on Windows for every PR changing more than `.md` files, and on `main` after each merge (`.github/workflows/windows-checks.yml`); run oracle yourself when it applies.

**Everyday check (Windows, against the built editor):** after screen change, build editor (`cargo build -p dawsu-tauri --locked`) and run `node tools/everyday-check.mjs --exe target/debug/dawsu-tauri.exe`. Opens real `dawsu-tauri.exe` on copy of fixture map, uses it like mapper ~1 min: menus, tools, snap, placing, moving and deleting hit, undo/redo, panels, dialogs and windows, Save, then checks files on disk. Verifies exe mtime against latest code commit (`crates`, `apps/tauri`, `Cargo.*`, `rust-toolchain.toml`) and `build-stamp.json`, writes temp artifacts to `target/everyday-check/` (remove when done), prints one `PASS`, `FAIL` or `SKIP` line per step, exits 1 on failed step. JS tests above run against fake page; this one catches screen that no longer starts or responds. Failing step showing app is wrong stays failing: fix app, not step.

**Merge gate (Yoge, 2026-11-21):** nothing merges into `main`, by person or agent, unless everyday check passes on build of that exact commit. Paste its last line (`everyday check: N of M steps passed`) and commit id in PR. Never merge in same step that opens PR.

**Git & docs hygiene:** Run `git fetch origin` before committing or pushing to `main`. Commit `NOW.md` refresh on feature branch after code commit, `--ff-only` merge in `dawsu-rs-main`, push, then `checkout --detach origin/main` in feature worktree before `git branch -d`. Also refresh `E:/Obsidian/Workspace/dawsu-rewrite/now.md` and keep `AGENTS.md` + `NOW.md` in `caveman-compress` terse format.

**Windows notes:** For fresh worktrees run `pwsh -NoProfile -ExecutionPolicy Bypass -File tools/setup-worktree.ps1` (junctions `target/` and `apps/tauri/ui/node_modules` to `E:/dawsu-build/`, keeps `C:` free, pre-starts `sccache`). Single-worktree runs reuse shared `CARGO_TARGET_DIR` (`E:/dev-caches/cargo-target`, auto-resolved by `verify-handoff.mjs`); 2-wide parallel worktree waves should set `CARGO_TARGET_DIR=<worktree>\target` so concurrent Cargo builds do not collide on `.cargo-lock` or `dawsu-tauri.exe`. Both `setup-worktree.ps1` and `verify-handoff.mjs` run `sccache --start-server` before Cargo to prevent Windows port-bind races (`os error 10048`). Machine `~/.cargo/config.toml` configures `sccache` (`SCCACHE_DIR=E:/dawsu-build/sccache`), `rust-lld`, and `LIBCLANG_PATH` (`E:/Claude/drum-onnx/venv311/Lib/site-packages/clang/native`); keep `TEMP`/`TMP` and build/temp artifacts on `E:/dawsu-build/tmp`. NTFS dir mtime: never trust `DirEntry::metadata()` (`FindNextFileW` reads stale parent `$I30` cache) — filter with `entry.file_type()` and call `fs::metadata(&path)` for live mtime; sleep `>=30 ms` in mtime-mutation tests (~15.6 ms Windows clock tick).

## Context pointers

Read only reference matching task:

- **Current repo sprint:** [NOW.md](NOW.md)
- **Feature parity matrix & testing notes:** [FEATURE-COVERAGE.md](FEATURE-COVERAGE.md), [README.md](README.md)
- **Oracle tool & `--audio` mode:** [tools/lazer-oracle/README.md](tools/lazer-oracle/README.md)
- **Design vault (ADRs, active sprint, UI spec, module map):** `E:/Obsidian/Workspace/dawsu-rewrite/` (`DECISIONS.md`, `now.md`, `design/ui-spec.md`, `architecture/module-map.md`) — read when changing architecture, UI layout, or domain rules.
