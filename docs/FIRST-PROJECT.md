# First Project Walkthrough

This guide walks you through a complete hitsounding session in **dawsu!**, from opening a mapset to checking coverage and exporting safely.

## 1. Create a practice mapset (optional)

If you want a safe practice mapset with two difficulties and built-in samples, run:

```sh
node tools/make-demo-mapset.mjs demo-mapset
```

This creates `[Hard].osu`, `[Normal].osu`, a short synthetic song, and three hitsound samples.

## 2. Open a difficulty

1. Launch **dawsu!**.
2. On the Start screen, either search your osu! Songs folder, drop a `.osu` or `.osz` file into the window, or click **Open map…** and choose `[Hard].osu`.
3. Every sample in the mapset appears as a lane on the timeline over the song's waveform.

## 3. Listen, slow down, or isolate drums

- Press **Space** to play or pause.
- Use the playback speed buttons in the bottom bar (**50%**, **75%**, **100%**) to slow tricky passages down. Click **Pitch** to toggle pitch preservation on or off.
- Open **Audio → Separate drum stem…** to isolate the song's drum stem on your CPU. Clicking the button downloads the verified model automatically, and pressing **T** switches between the song and the drum stem.

## 4. Place and balance hits

- Pick **Draw** (or turn on **Pads** in the bottom bar for compact beat-step pads) and click a lane to place hits on the beat snap grid.
- Drag a bar in the **volume controller** below the timeline to adjust loudness, or use the **Selection** card in the Inspector to change sample sets (`Soft | Normal | Drum`) and additions (`W | F | C`).
- Every write dawsu! makes is a **byte-identical save**: lines you did not edit stay untouched byte for byte.

## 5. Share hitsounds across difficulties

1. Open **Map → Shared Base layer…** and click **Capture current difficulty as Base**.
2. Switch to the **Live diff tabs** list to inspect how `[Hard]`'s hits project onto `[Normal]` (`Matched`, `Empty`, and `Orphaned` counts).
3. In the **Lane rules** tab, add per-difficulty rules (for example, mute a clap lane during a quiet section on `[Normal]` without changing the Base).
4. In the **Volume curve** tab, draw a green-line-only volume curve (linear ramps or flat steps) to shape loudness across sections without creating extra sample mixes.

## 6. Review and export

1. Press **Ctrl+E** (or **File → Export…**) to open the Export window.
2. Check **Auto-fix samples** if you want dawsu! to trim leading silence and pad samples under 25 ms for osu! Ranking Criteria.
3. Click **▶ Listen** on any target difficulty to audition its planned hitsounds before writing.
4. Choose **Write to: Mapset (with backups)** to write directly with automatic backups, or **Hitsound-only folder** to export a separate standalone folder without touching your mapset.
