# dawsu! v1.1 (Alpha)

dawsu! is a desktop editor for osu! hitsounds. It is a Tauri app with a Rust core and a Svelte 5 UI. This is an **Alpha**. Test it on copies of your mapsets. Downloads are on the [releases page](https://github.com/YogeGod/dawsu-rs/releases).

## What it does

- **Hitsound timeline editor.** Open a `.osu` or `.osz`. Edit hits on real sample lanes with seven tools, selection and clipboard, undo/redo, snap modes, volume editing, lane mute/solo/audition, bookmarks, cycle and pitch-preserving playback speed.
- **Shared Base Layer across diffs.** Hitsounds you place once can carry over to the other difficulties of the mapset.
- **Drum VST3 and Keysounding Piano Roll.** Host a drum VST3, and play it live from the timeline. Write keysounds in a piano roll with live MIDI recording.
- **Node Graph and Drum Detection.** Detect kick and snare hits and turn them into hitsounds you can review before they are applied.
- **Optional drum stem separation.** An on-demand CPU ONNX model splits the drums from the song. It is off until you ask for it.
- **Safe export.** Save, Generate, Copy, `.osz` and Hitsound-only export write byte-identical output where nothing changed, keep backups outside the mapset, and show a plain-English review first.
- **Auto-updater.** Downloads are checked against a SHA-256 hash before they install.

See [the feature ledger](FEATURE-COVERAGE.md) for what is verified and what is still open, and [NOW.md](NOW.md) for the current sprint.

## Build and test

Use the toolchain pinned by `rust-toolchain.toml` and the versions in `Cargo.lock`. The root workspace builds the Tauri editor from `apps/tauri` (ADR 0063 in the design vault).

- `npm --prefix apps/tauri/ui run build`
- `npm --prefix apps/tauri/ui test`
- `node --test apps/tauri/tests/*.test.mjs`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `cargo build -p dawsu-tauri --locked`
- `cargo run -p dawsu-tauri --locked -- "path/to/a/copied-difficulty.osu"`

The Windows binary is `dawsu-tauri.exe`. macOS and Linux build and runtime checks remain open.

signalsmith-stretch needs a C++ compiler and libclang at build time. On this machine, set `LIBCLANG_PATH` to `E:/Claude/drum-onnx/venv311/Lib/site-packages/clang/native`. Cargo is `C:/Users/yogem/.cargo/bin/cargo.exe`. The isolated dependency cache for the unified workspace is `E:/dawsu-mixed-save-20261117/asio-local/cargo-home`. Keep build and temporary files on E:.

### Local ASIO build on Windows

ASIO is available in the optional Tauri development build with `--features asio-local`. Set `CPAL_ASIO_DIR` to the extracted Steinberg ASIO SDK, set `LIBCLANG_PATH`, and load the MSVC x64 developer environment before running Cargo. Use a separate target directory for this build. The standard Windows build uses WASAPI, DirectSound and MME.

The ASIO driver is loaded when selected. While it is active, switch to WASAPI before changing the ASIO driver or buffer. Publication of an ASIO build requires the agreement and attribution recorded in the vault's ASIO decision.

## Tools

- `node tools/everyday-check.mjs --exe target/debug/dawsu-tauri.exe`: runs against a built exe. It uses the real editor like a mapper on a copy of a fixture map and reports one line per step ([AGENTS.md](AGENTS.md#verification)). Options: `--map`, `--shots`, `--json`, `--keep-profile`.
- `node tools/ui-screenshot.mjs`: screenshots the built editor on a copied map.
- `node tools/perf-measure.mjs`: measures load and playhead timings.
- `tools/lazer-oracle`: checks osu! behaviour against osu!lazer ([its README](tools/lazer-oracle/README.md)).

These need Windows (WebView2), except the oracle.

## User data and assets

dawsu! uses its own `dawsu-rs` app-data folder. Backups go there, outside the mapset. dawsu! never changes an original mapset unless you press Save.

dawsu! ships osu!'s 21 default hit and slider sounds, unchanged from ppy/osu-resources. They are CC BY-NC 4.0, so no build of dawsu! may be sold ([THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)). A default sounds folder chosen in Preferences comes first, and the shipped sounds fill in what it lacks.
