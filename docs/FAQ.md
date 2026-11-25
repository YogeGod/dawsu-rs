# Frequently Asked Questions

## Does dawsu! modify my `.osu` files when I open them?

No. Opening a difficulty only reads it into memory. Nothing is written to your mapset until you press **Save** (`Ctrl+S`) or click **Export**. Before overwriting any `.osu` file, dawsu! makes a backup outside your mapset folder and leaves every unedited line byte-identical.

## Where does dawsu! store projects, Shared Base data, stems, and backups?

Everything dawsu! creates for itself lives in its app data folder (`%LOCALAPPDATA%\dawsu-rs` on Windows), never inside your osu! mapset folder. That keeps your mapset clean for osu! Ranking Criteria.

## What if I move or rename a mapset folder in my Songs folder?

On the Start screen, click **Locate…** next to the recent map. dawsu! searches your configured Songs folder (or lets you pick the new folder directly) and migrates your saved project settings and Shared Base layer automatically.

## How does the green-line-only volume curve differ from editing hit volumes?

Editing individual hit volumes changes the relative loudness baked into mixed samples. The **Volume curve** (in **Map → Shared Base layer…**) scales only `[TimingPoints]` green lines using linear ramps or step holds, so changing a section's loudness creates zero extra sample files.

## Which osu! client does dawsu! follow?

dawsu! follows **osu!lazer**'s rules as its source of truth (verified against a pinned osu!lazer test oracle), while keeping exported `.osu` files and 16-bit WAV / Ogg Vorbis samples compatible with osu!stable as well.

## How do updates work?

By default, dawsu! checks GitHub Releases quietly when it launches (you can turn this off in **Preferences → Updates**). Downloads are always verified against `SHA256SUMS.txt` before installing on close — both for the Setup installer and for portable ZIP copies.
