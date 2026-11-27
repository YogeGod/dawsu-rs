# Product

<!-- impeccable:product-schema 1 -->

## Platform

Native desktop application built with Tauri v2, a Rust core, and a Svelte 5 + HTML5 `<canvas>` UI (ADR 0063). Windows (x64, WebView2) is actively built, packaged, and verified via automated real-app checks; cross-platform code paths exist for macOS and Linux, with macOS/Linux build and runtime verification still open.

## Users

osu! mappers editing hitsounds across difficulties, authoring custom drum kits, and composing melodic keysounds aligned to gameplay objects.

## Product Purpose

Edit, audition, share across difficulties, verify, and safely export osu! map audio in a familiar DAW-style arrangement while matching osu!lazer timing, slider, combo, and format semantics.

Shipped in `v1.3.0-alpha.1` (Alpha):

- Hitsound timeline editor with 3-path mapset intake, bundled synthetic practice mapset, fast `osu!.db`-cached Songs browser, seven editing tools, Rhythm Gallery presets, live tap recording (`Numpad 1–9` / `A–L`), Volume and Mixer panel, and pitch-preserving playback speed (`25%`–`150%`).
- Shared Base Layer across difficulties with live `Matched / Empty / Orphaned` counts, non-destructive per-difficulty lane rules, and green-line-only volume curves.
- Export Review with coverage strips, `▶ Listen` auditioning, line-by-line `.osu` diff preview, Ranking Criteria sample auto-fix, and 4-part pre-write read-back verification.
- Drum VST3 hosting with `.osz` budget-aware kit bouncing, Drum Hit Detection & Node Graph, optional on-demand local CPU HT-Demucs drum stem separation, inline Keysounding Piano Roll with pure-Rust `.sf2`/`.sfz` sampler + VST3 + live MIDI recording, osu! client link + reload-after-Save, crash recovery/logging, and SHA-256 verified auto-updates.

Still open: live VST3 effect mixer returns, licensed sound packs & online map search, macOS/Linux runtime verification, and formal human ear & feel listening checks.

## Stack

- **Core & audio:** 14-crate Rust workspace (`crates/`) — beatmap format & byte-preserving writer, hitsounding, keysounding (`.sf2`/`.sfz`), composition, audio engine (`cpal`, `symphonia`, `signalsmith-stretch`, `vorbis_rs`), MIDI (`midir`), node graph & drum detection, and optional on-demand CPU ONNX stems (`ort`).
- **Desktop shell & UI:** `apps/tauri` (Tauri v2, O(1) `ViewStateToken` scene state, typed domain action seams), Vite-bundled Svelte 5 (runes) chrome, HTML5 `<canvas>` dense timeline surface, and a native C++ VST3 host sidecar (`native/vst3-host`).
- **Oracle:** `tools/lazer-oracle` pinning osu!lazer (`ppy/osu` `131fe72`) as the behavioural reference.

## Operating Context

Project rules and architecture invariants live in [AGENTS.md](AGENTS.md), current sprint tracking in [NOW.md](NOW.md), capability verification in [FEATURE-COVERAGE.md](FEATURE-COVERAGE.md), and full specs/ADRs in the design vault (`E:/Obsidian/Workspace/dawsu-rewrite`). Validation uses copied maps or `tests/fixtures/` only and never modifies original mapsets, sample banks, or plug-in installations.

## Product Principles

- Real map data and working actions are the product.
- Preserve untouched map bytes and newer external edits.
- Keep the familiar arrangement and precise editing semantics.
- Report physical audio and manual proof separately from automated tests.

## Evidence on Hand

Automated Rust workspace tests (`cargo test --workspace --locked`), zero-warning Clippy CI gate (`cargo clippy --workspace --all-targets --locked -- -D warnings`), Svelte UI unit tests, frontend integration tests, pinned osu!lazer oracle fixtures, and real-window Windows WebView2 checks (`tools/everyday-check.mjs`). Human listening passes and macOS/Linux runtime verification remain tracked separately and are not claimed by automated tests.
