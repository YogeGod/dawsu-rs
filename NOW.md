# NOW.md — Current Sprint (`dawsu-rs`)

`origin/main` is `f80592b` (`PR #93`–`#99` merged, incl **`v1.1.0-alpha.3`**, Draw/marquee fix, repo cleanup, 3 core-seam refactorings, agent rules refresh, inline keysounding piano roll, compressed docs). Full sprint archive in `E:/Obsidian/Workspace/dawsu-rewrite/now.md`.

## Recent Landed Milestones

- **Post-v1.1 tech-debt cleanup** (`PR #88`–`#92`): Clippy CI gate (0 warnings), `main.rs` + `dawsu-application/src/lib.rs` split into focused submodules, Vite-bundled frontend & split Svelte modals, feature-gated ONNX stems, consolidated workspace deps.
- **Three Beginner Paths & Unified Mapset Intake** (`PR #93`): Path 1 (custom samples only), Path 2 (12 default osu! lanes w/ `Default osu! kit`), Path 3 (edit existing diff direct w/ Objects snap + used lanes), `MapsetIntakeCard`, safe lane additions, Shared Base shortcuts.
- **Start Screen Hero Grid, Export Review Coverage & Rhythm Gallery** (`PR #94`): 2-column Hero Cards, Export per-target coverage strips, 1-click Rhythm Gallery presets.
- **Elevated Obsidian Studio UI Redesign & Minimap Perf** (`PR #95`, `PR #96`): Obsidian Studio Start Hub & Editor chrome, 44 px multi-lane cached `Objects overview` minimap (`1.19 ms` p95 at 30×40k hits), blue-to-red Inspector volume slider, full-stage 1140×760 Export modal.
- **Live Tap Recording, osu! Reload-after-Save, SFZ/SF2 Sampler & Songs Grid Fix** (`PR #97`, `PR #98`): Numpad `1–9` / armed `A–L` keyboard tap recording, Windows/lazer reload-after-Save, pure-Rust `.sf2`/`.sfz` sampler engine, Songs folder card grid/artwork fix (`v1.1.0-alpha.3`).
- **Draw Paint-Stroke & Visual-Row Marquee Selection Fix** (`3e88201`): Drag-coalescing for `drawStrokeMove`, slim `recentMeta` omission during active scene edits, cached window titles, batch `select_box` across visible/collapsed rows.
- **Repo Cleanup, Core-Seam Refactorings & Agent Rules** (`b466a62`–`b629e01`, `f80592b`): dropped obsolete migration tests/artifacts; encapsulated 25 `Application` fields behind O(1) `ViewStateToken` (`761cb38`); collapsed 29 Tauri commands → 5 domain action seams (`835010a`, `112 → 88` cmds, `96 → 72` perms); collapsed 27 `main.js` wrappers & 169 globals into `Bridge`/`registry` w/ `exposeGlobals: false` (`283396a`, `41/41` `everyday-check.mjs` PASS); updated/compressed `AGENTS.md`, `NOW.md`, `verify-dawsu`, `.agents/skills.json`.
- **Inline Keysounding Piano Roll & v3.1.4 Parity** (`1b21edb`): inline `#editor-arrange` piano roll swap (`keysounds-active`), track mute/solo + `sync_to_objects` + `trim_overlaps`, audible-track ghost notes & beatmap object lines, v3.1.4 musical typing (`Alt+K`, `Z`/`X` octave, `C`/`V` velocity).

## Still Open Next

- DAW prior-art 5-slice plan (unblocked — both `refactor/deepen-core-seams` and inline keysounding piano roll landed).
- Live VST3 effect mixer returns.
- Licensed sound packs + online map search.
- macOS/Linux runtime verification + native ear & feel listening checks.

## Rules

- **Single source of truth for sprint tasks:** `E:/Obsidian/Workspace/dawsu-rewrite/now.md`. Mirror changes there, not in competing lists.
- **Real-app check:** run `npm --prefix apps/tauri/ui run build`, `npm --prefix apps/tauri/ui test`, `node --test apps/tauri/tests/*.test.mjs` (`pwsh`: `$tests = @(Get-ChildItem apps/tauri/tests/*.test.mjs | Sort-Object Name | ForEach-Object { $_.FullName }); node --test @tests`), `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `cargo build --release -p dawsu-tauri --locked`, `node tools/everyday-check.mjs --exe target/release/dawsu-tauri.exe` before merging.
