# dawsu! Rust editor — first development build

This branch contains the first real native Hitsound editor slice. It is **not the completed full-feature rewrite**. The complete target still includes the node graph, Live/Record, Keysound, VST3/SoundFont/SFZ instruments, mixer/effects, stems, client link and the rest of the feature ledger.

The UI uses real map data, the existing domain/compiler, the shared editing kernel, cpal playback, Symphonia decoding and signalsmith time stretching. The previously tested Windows package is in `E:/Codex-releases/dawsu-rs-timeline-preview/`. Build this branch to exercise subsequent source changes.

## Current testing path

Open a copied `.osu` or `.osz`; use the seven tools, selection/clipboard, undo/redo, snap modes, volume editing, lane mute/solo/audition, transport, cycle and pitch-preserving speed. Save goes through the format crate with a backup and an observed-disk-version check. New difficulty, Generate and Copy prepare a review before writing.

Save supports mixes on circles and spinner ends. Save and Copy support slider-node mixes where the compiler can safely place a restoring timing line. Generate and Copy stage supported mixes for review and audition. Copy can also stage timed storyboard samples at slider ticks and during slides. Hits with no object are stored in the editor project. Use HS mode or Generate to materialise them in a difficulty. Test this development build with copied mapsets.

The Windows timeline preview (executable SHA-256 `b7f62afacb0f2cb424997ba88066c96c058a574765dc40b07cf449dc4f7b0696`) was checked with the native picker on a copied difficulty. The recorded checks cover overview navigation, a volume edit and History restore. Three audio warnings appeared. Physical listening, plug-ins, osu! import and macOS/Linux runtime still need verification. See [the feature ledger](FEATURE-COVERAGE.md) for coverage by workflow.

## Build

Use the toolchain pinned by `rust-toolchain.toml` and the versions in `Cargo.lock`.

The root workspace builds the Tauri editor from `apps/tauri`, following ADR 0063. Use a copied map when testing.

- `cargo test --workspace --locked`
- `cargo build -p dawsu-tauri --locked`
- `cargo run -p dawsu-tauri --locked -- "path/to/a/copied-difficulty.osu"`
- `node --test apps/tauri/tests/*.test.mjs`

The Windows binary is `dawsu-tauri.exe`. The macOS and Linux build and launch checks remain open.

signalsmith-stretch needs a C++ compiler and libclang at build time. On this machine, set `LIBCLANG_PATH` to `E:/Claude/drum-onnx/venv311/Lib/site-packages/clang/native`. Cargo is `C:/Users/yogem/.cargo/bin/cargo.exe`. The isolated dependency cache for the unified workspace is `E:/dawsu-mixed-save-20261117/asio-local/cargo-home`. Keep build and temporary files on E:.

### Local ASIO build on Windows

ASIO is available in the optional Tauri development build with `--features asio-local`. Set `CPAL_ASIO_DIR` to the extracted Steinberg ASIO SDK, set `LIBCLANG_PATH`, and load the MSVC x64 developer environment before running Cargo. Use a separate target directory for this build. The standard Windows build uses WASAPI, DirectSound and MME.

The ASIO driver is loaded when selected. While it is active, switch to WASAPI before changing the ASIO driver or buffer. Publication of an ASIO build requires the agreement and attribution recorded in the vault's ASIO decision.

## User data and assets

The rewrite uses its own `dawsu-rs` app-data folder. This first slice saves its own preferences; legacy project import covers offset, locators, cycle and hits with no object. Other project fields and old preferences remain incomplete. Backups go in that separate folder, outside the mapset.

dawsu! ships osu!'s 21 default hit and slider sounds, unchanged from ppy/osu-resources. They are CC BY-NC 4.0, so no build of dawsu! may be sold ([THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)). A default sounds folder chosen in Preferences comes first, and the shipped sounds fill in what it lacks. On this development machine, a read-only copy of the old app's sounds is still used as that folder when none is saved. The supplied test map is a local staging copy and is not a licence grant to redistribute it.

See [the feature ledger](FEATURE-COVERAGE.md) for the full target.
