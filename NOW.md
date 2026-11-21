# NOW.md — `dawsu-rs`

Repo-side snapshot of active M3 work on `origin/main` (`2b8b94c`). The master sprint board lives in the design vault at `E:/Obsidian/Workspace/dawsu-rewrite/now.md`.

## Current state

- **Milestone:** M3 Hitsound Editor — near finish. Hard cutoff before M4 (Node Graph): finish deleting legacy `apps/tauri/dist/app.js` (~186 KB / 4,205 lines on `main`).
- **Already migrated to Svelte 5 runes (`apps/tauri/ui/src/parts/`):**
  - Top bar & transport: `SnapSlider`, `ToolPills`, `AuditionPill`, `TransportBar` (with `bridge.playhead` isolating 85 ms playback ticks and seeks from unrelated Svelte components), `PlayheadGroup`, `EditorMenu`, `GridOptions`, `LaneOptions`
  - Lanes & timeline chrome: `LaneHeaders`, `HitCard` (timeline surface itself stays on `<canvas>`)
  - Inspector & map preview: `InspectorHead`, `InspectorSelection`, `InspectorLane`, `InspectorDifficulty`, `InspectorBookmarks`, `InspectorHistory`, `ProblemMap`, `MapPreview`
  - Browser & bottom drawer: `BrowserHead`, `BrowserList`, `BrowserPreview`, `BottomPanel`, `Mixer`, `MixerStrip`, `Fader`, `StatusBar`
  - Second windows: `ExportWindow`, `DrumDetectWindow`
- **Still left in `apps/tauri/dist/app.js` (on `main`):** remaining editor wiring, dialogs, start screen, shortcut editor, and menus/ribbon glue.

## Next priorities (M3 finish)

1. **Retire `apps/tauri/dist/app.js` completely** — migrate remaining dialogs and editor wiring into `apps/tauri/ui/src/` one bounded slice at a time while keeping the timeline renderer on `<canvas>`.
2. **File safety & workflow closures** — land unused-file cleanup (`no-unused-files`), browser drag-to-add-lane, revert mapset to original, and Copy Merge/Union collision default.
3. **v3 polish & mapper gaps** — surface `Ramp` in Volume header and `...` More tools button, switch Save unsaved badge away from red, add sound-first warning toggle, numbered sample pairing, folder shortcuts, Symphonia MP3 offset calibration, and zoom-independent hitboxes.
4. **Audio mixer headroom/limiter** — prevent clipping when song + click peak above full scale.

## Parallel worktrees

- `E:/GitHub/dawsu-rs-claude` — Rust core, file safety, backend (`crates/`).
- `E:/GitHub/dawsu-rs-agy` — Svelte 5 UI, polish, ergonomics (`apps/tauri/ui/`).
- Never commit directly to `main`; review diffs before merging.
