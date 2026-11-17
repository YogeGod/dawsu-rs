# dawsu! Rust editor — first development build

This branch contains the first real native Hitsound editor slice. It is **not the completed full-feature rewrite**. The complete target still includes the node graph, Live/Record, Keysound, VST3/SoundFont/SFZ instruments, mixer/effects, stems, client link and the rest of the feature ledger.

The UI uses real map data, the existing domain/compiler, the shared editing kernel, cpal playback, Symphonia decoding and signalsmith time stretching. The previously tested Windows package is in `E:/Codex-releases/dawsu-rs-timeline-preview/`. Build this branch to exercise subsequent source changes.

## Current testing path

Open a copied `.osu` or `.osz`; use the seven tools, selection/clipboard, undo/redo, snap modes, volume editing, lane mute/solo/audition, transport, cycle and pitch-preserving speed. Save goes through the format crate with a backup and an observed-disk-version check. New difficulty, Generate and Copy prepare a review before writing.

**Limits:** sample mixes are refused rather than silently omitted from a save/export. Per-map project persistence, sample replacement/folder watching, full command coverage, sound sets, movable tabs and other M3 requirements remain incomplete. Hits with no object are stored in the editor project. Use HS mode or Generate to materialise them in a difficulty. Do not use this development build as the only copy of a project.

The Windows timeline preview (executable SHA-256 `b7f62afacb0f2cb424997ba88066c96c058a574765dc40b07cf449dc4f7b0696`) was checked with the native picker on a copied difficulty. The recorded checks cover overview navigation, a volume edit and History restore. Three audio warnings appeared. Physical listening, plug-ins, osu! import and macOS/Linux runtime still need verification. See [the feature ledger](FEATURE-COVERAGE.md) for coverage by workflow.

## Build

Use the toolchain pinned by `rust-toolchain.toml` and the versions in `Cargo.lock`.

- `cargo test --workspace --locked`
- `cargo build -p dawsu-desktop --release --locked`
- `cargo run -p dawsu-desktop --release -- "path/to/a/copied-difficulty.osu"`

signalsmith-stretch needs a C++ compiler and libclang at build time. On this machine, set `LIBCLANG_PATH` to `E:/Claude/drum-onnx/venv311/Lib/site-packages/clang/native`. Cargo is `C:/Users/yogem/.cargo/bin/cargo.exe`. The shared dependency cache is `E:/dawsu-rewrite-spikes/egui-ui/.cargo-home`; build/temp files stay on E:.

## User data and assets

The rewrite uses its own `dawsu-rs` app-data folder. This first slice saves its own preferences; legacy project import covers offset, locators, cycle and hits with no object. Other project fields and old preferences remain incomplete. Backups go in that separate folder, outside the mapset.

Default samples can be selected in Preferences. On this development machine, an available read-only copy in the old app's source is used as a fallback. Those assets are CC BY-NC and are **not bundled**. The supplied test map is a local staging copy and is not a licence grant to redistribute it.

See [the feature ledger](FEATURE-COVERAGE.md) for the full target.
