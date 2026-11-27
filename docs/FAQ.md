# Frequently Asked Questions

For the full mapper documentation set, see the [Setting Up guide](wiki/Setting-Up.md) and [First Project walkthrough](wiki/First-Project.md).

## Does dawsu! modify my `.osu` files when I open them?

No. Opening a difficulty only reads it into memory. Nothing is written to your mapset until you press **Save** (`Ctrl+S`), apply changes across all difficulties in **Shared Base**, or run **Export** (**File → Export…**) with **Write to: Mapset (with backups)**. Before overwriting any `.osu` file, dawsu! snapshots/backs up originals outside your mapset folder and leaves unedited lines byte-identical.

## How can I try dawsu! without risking my maps?

On the Start screen, click **Try practice mapset**. It opens a temporary 2-difficulty synthetic sandbox (`[Hard]` and `[Normal]`) with built-in audio and samples, completely isolated from your Songs folder, recent list, projects, and backups.

## Where does dawsu! store projects, Shared Base data, stems, crash logs, and backups?

Everything dawsu! creates for itself lives in its app data folder (`%LOCALAPPDATA%\dawsu-rs` on Windows), never inside your osu! mapset folder. That keeps your mapset clean for osu! Ranking Criteria.

You can also export into a standalone **Hitsound-only folder** in the Export window if you want to inspect the output before touching your real mapset.

## Can I undo a mistake across the whole mapset?

Yes. Beyond normal `Ctrl+Z` Undo/Redo (which survives Save), **File → Revert mapset to original…** restores every `.osu` file in the mapset back to the snapshot captured when dawsu! first opened it — and saves a pre-revert backup first so **File → Undo revert** can restore your work if needed.

## What if I move or rename a mapset folder in my Songs folder?

On the Start screen, click **Locate…** next to the recent map. dawsu! searches your configured Songs folder (or lets you pick the new folder directly) and migrates your saved project settings and Shared Base layer automatically.

## Which osu! clients and modes does dawsu! support?

- **Ruleset:** dawsu! follows **osu!lazer**'s timing, slider, combo, and audio rules as its source of truth, while keeping exported `.osu` files and 16-bit WAV / Ogg Vorbis samples compatible with **osu!stable**.
- **Game modes:** Editing and intake target **osu!standard** difficulties. You can also import keysounds from an osu!mania (Mode 3) difficulty via **File → Import mania hitsounds…**.
- **Client links:** Works with **osu!stable** (window detection / `Ctrl+L` reload + Mapping Tools IPC) and **osu!lazer** (`File → Edit externally`).

## How does the green-line-only Volume curve differ from editing hit volumes?

Editing individual hit volumes (or dragging bars in the timeline's **Volume** panel) adjusts the loudness baked into per-object or mixed samples. The **Volume curve** in **Map → Shared Base layer…** scales only `[TimingPoints]` green lines using linear ramps or step holds, so shaping section loudness creates zero extra sample files.

## What is the pre-write read-back check in Export?

Before **Export** overwrites any mapset file or writes a **Hitsound-only folder**, dawsu! runs a 4-part pre-write read-back check (re-parse round-trip, structure check, compiled object/envelope preservation, and sample resolution). If anything fails, export is blocked and **Read-back check failed — Export blocked** explains which difficulty and timestamp failed.

## Does AI drum stem separation upload my music anywhere?

No. **Playback → Separate drums (AI)…** runs 100% locally on your CPU using the HT-Demucs ONNX model. The ~166 MB model and runtime are downloaded from official releases only when you ask for it, verified against pinned SHA-256 hashes, and can be deleted anytime from the same dialog.

## Why did Windows SmartScreen warn when launching the installer or EXE?

Early Alpha builds are not commercially code-signed yet. Always download directly from the [GitHub Releases page](https://github.com/YogeGod/dawsu-rs/releases) and verify the file hash against `SHA256SUMS.txt` if you want extra peace of mind.

## How do updates work?

By default, dawsu! checks GitHub Releases quietly when it launches (you can toggle this in **Preferences → Updates**). Both the Setup installer and Portable ZIP are always verified against `SHA256SUMS.txt` before installing on close.

## Can I switch between osu! editor shortcuts and DAW shortcuts?

Yes — open **Preferences → General** (or **Preferences → Shortcuts**) to switch between **osu! editor controls** and **DAW-like keys** anytime, rebind individual commands, or press `Ctrl+K` to run any action from the Command Palette.

## Is macOS or Linux supported yet?

Not yet. **Windows 10/11 (x64)** is the supported Alpha platform right now; macOS and Linux builds are planned once Windows workflows settle.

## How do I report a bug?

See [Reporting Bugs](wiki/Reporting-Bugs.md) for where to find `crash.log` via **Help → Open crash log folder** and what to include.
