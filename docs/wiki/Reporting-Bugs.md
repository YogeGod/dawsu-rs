# Reporting Bugs & Requesting Features

[Setting Up](Setting-Up.md) · [First Project](First-Project.md) · [Shared Base & Export](Shared-Base-and-Export.md) · [Drums, Stems & Keysounds](Drums-Stems-and-Keysounds.md) · [FAQ](FAQ.md) · [Reporting Bugs](Reporting-Bugs.md)

Found a bug, crash, audio glitch, or export issue in **dawsu!**? Clear reports help track down fixes quickly.

## 1. Open the Issue Form

- Inside dawsu!, open **Help → Report a bug…** to jump straight to the [GitHub issue chooser](https://github.com/YogeGod/dawsu-rs/issues/new).
- Pick **Bug Report** for crashes, broken behaviour, audio issues, or export problems, or **Feature Request** for workflow improvements.

## 2. Grab Your Crash Logs (If dawsu! Crashed or Panicked)

When dawsu! encounters an unexpected error, it writes a timestamped report (with version, thread, message, and backtrace) to `crash.log` (rotating to `crash.log.1` at 512 KiB) inside `%LOCALAPPDATA%\dawsu-rs`.

1. Reopen dawsu! and click **Help → Open crash log folder** (you can open it straight from the Start screen menu bar without loading a map).
2. Open `crash.log` in Notepad or any text editor and copy the newest `=== dawsu! crash report ===` entry at the bottom.
3. **Privacy tip:** File paths inside the log may include your Windows username (`C:\Users\YourName\...`). Feel free to replace your username before pasting.

## 3. What to Include in a Bug Report

- **dawsu! version:** Found in **Preferences → Updates** or the installer/ZIP filename (for example `v1.3.0-alpha.1`).
- **Install method & OS:** Windows Setup Installer or Portable ZIP, plus your Windows version and osu! client (**osu!stable**, **osu!lazer**, or both).
- **Exact steps to reproduce:** Numbered steps from opening dawsu!. Let us know if the bug also happens on **Try practice mapset** — that makes reproducing it instant.
- **Expected vs. actual behaviour:** What you expected to happen vs. what happened instead (include exact text from any red error toast, status bar notice, or **Read-back check failed** card).
- **Audio / VST3 / MIDI bugs:** Note your selected **Backend** (`WASAPI`, `DirectSound`, `MME`), **Device**, **Buffer (ms)** from **Preferences → Audio output**, and the exact VST3 plugin or MIDI controller model if relevant.
- **Screenshots or `.osu` snippets:** Dragging in a screenshot or pasting the affected `[TimingPoints]` / `[HitObjects]` lines helps tremendously. Please **do not** upload copyrighted song audio files.

## 4. Requesting Features

Before opening a **Feature Request**, check [FEATURE-COVERAGE.md](../../FEATURE-COVERAGE.md) and the other wiki pages to see if the capability is already planned or built. When filing a request, explain the mapping problem you want to solve, how you imagine it working in the editor, and how you currently work around it.
