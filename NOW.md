# NOW.md — Current Sprint (`dawsu-rs`)

`origin/main` is `c2f942b` (`PR #93`–`#96` merged). Next prerelease prepared on `agy/v1.1-alpha3-tap-reload-sfz`: **`v1.1.0-alpha.3`**. The full sprint archive lives in `E:/Obsidian/Workspace/dawsu-rewrite/now.md`.

## Recent Landed Milestones (`origin/main` `c2f942b`)

- **Post-v1.1 tech-debt cleanup** (`PR #88`–`#92`): Clippy CI gate (0 warnings), `main.rs` and `dawsu-application/src/lib.rs` split into focused submodules, Vite-bundled frontend & split Svelte modals, feature-gated ONNX stems, consolidated workspace dependencies.
- **Three Beginner Paths & Unified Mapset Intake** (`PR #93`): Path 1 (custom samples only), Path 2 (12 default osu! lanes with `Default osu! kit`), Path 3 (edit existing diff directly with Objects snap + used lanes), `MapsetIntakeCard`, safe lane additions, and Shared Base shortcuts.
- **Start Screen Hero Grid, Export Review Coverage & Rhythm Gallery** (`PR #94`): 2-column Hero Cards, Export per-target coverage strips, and 1-click Rhythm Gallery presets.
- **Elevated Obsidian Studio UI Redesign & Minimap Perf** (`PR #95`, `PR #96`): Obsidian Studio Start Hub & Editor chrome, 44 px multi-lane cached `Objects overview` minimap (`1.19 ms` p95 at 30×40k hits), blue-to-red Inspector volume slider, full-stage 1140×760 Export modal.

## Ready in `v1.1.0-alpha.3` (`agy/v1.1-alpha3-tap-reload-sfz`)

1. **Live keyboard lane tapping & tap-recording (F028 / F029):** Numpad `1–9` plays visible lanes 1–9 live anytime; arming Record (Transport or `Playback › Tap record`) lets `A–L` and `Numpad 1–9` record taps onto the timeline (respecting active snap and volume) and commit them as one single Undo step (`"Record taps"`).
2. **osu! editor reload-after-Save (F011):** `File › Reload in osu!` plus opt-in `Preferences › General › Reload osu! editor after Save` (Windows osu!stable `Ctrl+L` + `Enter` port of `OsuKeys.cs` when the open window matches the saved difficulty; cross-platform osu!lazer external-edit detection reminding mappers to click Finish).
3. **Pure-Rust SoundFont (`.sf2`) & SFZ (`.sfz`) sampler engine:** Zero new crates (`dawsu-audio-instruments/src/sfz.rs` + `sf2.rs`); velocity layers, pitch shifting, stereo pan, attack/release envelopes, choke groups (`off_by`), multi-preset `.sf2` switching, live Piano Roll preview, `[Keysounds]` bouncing/deduplication, and `Preferences › MIDI Input & VST3` tester.
4. **Docs refresh & `1.1.0-alpha.3` version bump.**

## Still Open After `v1.1.0-alpha.3`

- Live VST3 effect mixer returns.
- Licensed sound packs and online map search.
- macOS/Linux runtime verification and native ear & feel listening checks.

## Rules

- **Single source of truth for sprint tasks:** `E:/Obsidian/Workspace/dawsu-rewrite/now.md`. Mirror changes there, not in competing lists.
- **Real-app check:** run `npm --prefix apps/tauri/ui run build`, `npm --prefix apps/tauri/ui test`, `node --test apps/tauri/tests/*.test.mjs`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `cargo build --release -p dawsu-tauri --locked`, and `node tools/everyday-check.mjs --exe target/release/dawsu-tauri.exe` before merging.
