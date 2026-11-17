# The lazer oracle

A small .NET tool that runs osu!lazer's own packages over `.osu` files and writes lazer's expected results as JSON (ADRs 0026 and 0032 in the design vault). The Rust tests compare dawsu!'s answers with these files. It is a tool: it never ships with dawsu!.

- **lazer version:** the packages `ppy.osu.Game` and the osu!, taiko, catch and mania rulesets at 2026.1026.0, the vault's M2 pin (`ppy/osu` at `131fe72`), on .NET 10. `packages.lock.json` fixes every package it pulls in.
- **What it does per file:** it reads the `Mode` line itself (missing counts as 0), decodes the file with lazer's decoder, stops with an error if the ruleset lazer chose differs from that mode, and builds the playable beatmap in the map's own ruleset with no timeout. It then writes a header (the input's SHA-256, the lazer version, the mode, the ruleset and the format version) and every hit object in lazer's order: its type, start and end time, for an osu! slider its beat length, slider velocity, velocity, path distance, span duration, tick distance and repeat count, every sample with its lookup names, a slider's sliding and tail samples, and its nested objects (ticks, repeats, tail) the same way.
- **Output:** JSON with LF line endings, one file per input map at `<output folder>/<input folder name>/<relative path>.json`. It is written to a temporary folder first and moved into place only when every file succeeded, so a failed run leaves the old results as they were. Two runs over the same files write the same bytes.

## Running it

From the repo root:

```
dotnet run -c Release --project tools/lazer-oracle -- tests/fixtures/lazer-oracle tests/fixtures/save-pack tests/fixtures/oracle-cases
```

The save pack's and the hand-made cases' results are committed (ADR 0026 point 2, ADR 0039). Results for the local corpus are not: write them under `target/`, which git ignores, for example `dotnet run -c Release --project tools/lazer-oracle -- target/lazer-oracle-corpus <corpus folder>`.

### Where a song's decode starts (`--audio`)

```
dotnet run -c Release --project tools/lazer-oracle -- --audio target/lazer-oracle-audio <song folder>
```

Every `.mp3`, `.ogg` and `.wav` file is decoded with BASS the way lazer's tracks are: option 68 (`BASS_CONFIG_MP3_OLDGAPS`) set to 1 before BASS starts (osu!framework `AudioManager.cs:430-434`), then a decode stream with `Decode | Prescan` (`TrackBass.cs:159`), read as floats. The JSON gives the sample rate, the channel count, the number of decoded frames, and the first frame whose sample is above 0, 0.0001, 0.001 and 0.01. dawsu!'s decoder must give the same figures (TDD: Audio, Responsibility 12). Songs are not committed; write results under `target/`.

Whoever moves the lazer pin runs it again and commits the new results in the same change (ADR 0026 point 4).

## Notes

- NuGet warns about known vulnerabilities in two libraries lazer's packages pull in (AutoMapper and SixLabors.ImageSharp). The tool only reads local test maps and never ships, so they are not fixed here; they change when the lazer pin moves.
