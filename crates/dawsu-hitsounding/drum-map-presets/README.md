# Built-in drum maps

Ready-made drum maps for the ways mappers most often lay out a hitsound pack. A drum map gives
each sample of a map a drum's name (Kick, Snare, Crash...) and a drum note for drum renders. It
never adds, removes or renames a lane, and these carry no audio: the pack's samples stay yours.

## Using one

1. Open a difficulty in dawsu!.
2. Choose **View > Open drum map...** (or the **Drums** button in the toolbar, next to
   **Hitsounds**) and pick one of these files.
3. Turn on **View > Used lanes only** to hide the rows your map has no sample for.

Once a drum map is open, **Hitsounds** and **Drums** switch between your lanes as they are and
that drum map. To try another drum map, use **View > Open drum map...** again.

Each row names an unnumbered sample file, such as `soft-hitclap.wav` (custom set 1). The row also
finds the file saved as `.ogg` or `.mp3`, or with capital letters, the way osu! finds it. A sample
your map doesn't have shows as an empty row. Your other samples, numbered ones such as
`soft-hitclap2.wav` included, show under the preset's rows as before.

Whistle rows have no drum note, so a drum render leaves your map's own whistle samples alone. (A
whistle with no file in your map plays osu!'s default sound, and keeps the drum note it gets without
a drum map.) The notes are General MIDI drum notes: Kick 36, Kick 2 35, Snare 38, Snare 2 40, Clap
39, Rim 37, Tom 48, Floor Tom 41, Hi-Hat 42, Ride 51, Crash 49, Crash 2 57.

## Which one to pick

| Preset | Use it when your map has... |
| --- | --- |
| Kick on drum-hitnormal | the kick on `drum-hitnormal`, the snare on `soft-hitclap` |
| Kick on normal-hitnormal | the kick on `normal-hitnormal`, the snare on `soft-hitclap` |
| Drum kit, kick on drum-hitnormal | the kick on `drum-hitnormal`, the snare on `soft-hitclap`, toms on `drum-hitclap` and `drum-hitfinish`, and a hi-hat on `soft-hitnormal` |
| Drum kit, kick on normal-hitnormal | the same kit with the kick on `normal-hitnormal` |
| Full kit, all 12 samples | every hit sample, each named for what most maps put there |
| Two kicks | one kick on `drum-hitnormal` and another on `normal-hitnormal` |
| Kick and snare on hitnormals | the kick on `drum-hitnormal` and the snare on `normal-hitnormal` |
| Kick and snare on claps | the kick on `normal-hitclap` and the snare on `soft-hitclap` |
| Kick on whistle (older maps) | the kick on `soft-hitwhistle`, as many 2017-2019 maps do |
| Net0 hitsound kit | the sample names from Net0's hitsound guide |

## What each preset holds

- **Kick on drum-hitnormal**: Kick `drum-hitnormal`, Snare `soft-hitclap`, Snare 2
  `normal-hitclap`, Crash `soft-hitfinish`, Crash 2 `normal-hitfinish`, Whistle `soft-hitwhistle`,
  Whistle (normal) `normal-hitwhistle`.
- **Kick on normal-hitnormal**: the same with Kick on `normal-hitnormal`.
- **Drum kit, kick on drum-hitnormal**: Kick `drum-hitnormal`, Snare `soft-hitclap`, Snare 2
  `normal-hitclap`, Tom `drum-hitclap`, Floor Tom `drum-hitfinish`, Hi-Hat `soft-hitnormal`, Crash
  `soft-hitfinish`, Crash 2 `normal-hitfinish`, Whistle `soft-hitwhistle`, Whistle (normal)
  `normal-hitwhistle`, Whistle (drum) `drum-hitwhistle`.
- **Drum kit, kick on normal-hitnormal**: the same with Kick on `normal-hitnormal`.
- **Full kit, all 12 samples**: **Drum kit, kick on drum-hitnormal**, plus Kick 2 on
  `normal-hitnormal`.
- **Two kicks**: Kick `drum-hitnormal`, Kick 2 `normal-hitnormal`, then the snares, crashes and
  whistles of the first preset.
- **Kick and snare on hitnormals**: Kick `drum-hitnormal`, Snare `normal-hitnormal`, Clap
  `soft-hitclap`, Crash `soft-hitfinish`, Crash 2 `normal-hitfinish`, Whistle `soft-hitwhistle`,
  Whistle (normal) `normal-hitwhistle`.
- **Kick and snare on claps**: Kick `normal-hitclap`, Snare `soft-hitclap`, Crash
  `soft-hitfinish`, Crash (normal) `normal-hitfinish`, Whistle `soft-hitwhistle`, Whistle (normal)
  `normal-hitwhistle`. The crash and whistle are copied into both sets, so either can go with a
  kick (normal additions) or a snare (soft additions).
- **Kick on whistle (older maps)**: Kick `soft-hitwhistle`, Kick 2 `normal-hitwhistle`, Snare
  `soft-hitclap`, Snare 2 `normal-hitclap`, Crash `soft-hitfinish`, Crash 2 `normal-hitfinish`.
- **Net0 hitsound kit**: Kick `drum-hitnormal`, Snare 1 `normal-hitclap`, "Snare 2, Cowbell or
  Sticks" `normal-hitnormal`, "Rim or Clap" `soft-hitclap`, Rack Tom `drum-hitclap`, Floor Tom
  `drum-hitfinish`, Closed Hi-Hat `soft-hitnormal`, "Ride, Open Hi-Hat or Cowbell"
  `drum-hitwhistle`, Crash 1 `soft-hitfinish`, Crash 2 `normal-hitfinish`, Melody 1
  `soft-hitwhistle`, Melody 2 `normal-hitwhistle`.

## Common packs

- **Lasse's samples** (the most copied: his snare is in 118 of the maps checked, his kick in 82,
  his tom in 70, his cymbal in 35). With his kick on `drum-hitnormal` or `normal-hitnormal`, pick
  the matching "Kick on..." preset, or a "Drum kit" preset when his tom is on `drum-hitclap`. His
  guide's favourite way, kick and snare both on claps, is **Kick and snare on claps**; his way
  with osu!'s own sounds, kick on `drum-hitnormal` and snare on `normal-hitnormal`, is **Kick and
  snare on hitnormals**.
- **Mentai's samples and guide** (his kick is in 53 maps): he puts the kick on
  `normal-hitnormal`, so pick **Kick on normal-hitnormal**.
- **Net0's guide and sample folder**: **Net0 hitsound kit**.
- **osu!'s own classic sounds copied into the map folder** (the classic soft-hitwhistle is in 164
  maps, soft-hitfinish in 92, drum-hitfinish in 63): the whistle and finish fit every preset. The
  classic drum-hitclap and drum-hitfinish are toms, so they fit the "Drum kit" presets.

## How they were chosen

The file lists of 931 ranked osu! mapsets from 2017 to 2026 were read from two public beatmap
mirrors, without their audio. 818 of them (88%) have custom hit samples. To learn what the most
copied files sound like, about 70 of them were measured by a script (length, pitch, brightness);
none is included here. In the maps that have each file, the copies of those measured files hold:

| Sample file | Maps with it (of 931) | What the measured copies are |
| --- | --- | --- |
| soft-hitclap | 610 | snare or clap 95% |
| soft-hitfinish | 399 | crash 95% |
| drum-hitnormal | 378 | kick 95% |
| soft-hitwhistle | 367 | whistle 88%, kick 10% |
| normal-hitnormal | 309 | kick 80%, snare or clap 16% |
| drum-hitwhistle | 265 | whistle 82%, cymbal or hi-hat 13% |
| drum-hitclap | 238 | tom 76%, snare 11%, kick 8% |
| normal-hitclap | 216 | snare or clap 85%, kick 13% |
| drum-hitfinish | 212 | tom 72%, crash 19% |
| normal-hitwhistle | 177 | whistle 87%, kick 8% |
| normal-hitfinish | 172 | crash 94% |
| soft-hitnormal | 161 | hi-hat 86%, kick 8% |

The layouts follow the guides these packs come with: Lasse's hitsounding guide
(<https://docs.google.com/document/d/1tS5udToW_SEiCKNFSg7DUuZmsdroUOO8cRYEcwZV9P8>, samples at
<http://hitsounds.lasse.moe>), Mentai's "[Guide] Hitsounding (in current year)"
(<https://osu.ppy.sh/community/forums/topics/722435>) and Net0's "Custom Hitsounds Guide"
(<https://hackmd.io/@Net0/HitsoundsGuide>).

For 622 of the 716 mapsets with unnumbered custom hit samples (87%), one of these presets names
every such sample, and no row calls a measured sample the wrong drum. Of the 818 mapsets with
custom hit samples, 102 (12%) use only numbered files; these presets name none of them. 16% of the
mapsets with custom hit samples have at least one `.ogg` sample (27% of those ranked since 2024),
which is why a row finds `.ogg` and `.mp3` copies of its file.

## Changing a preset

Each file is exactly the text dawsu! writes for its drum map, with LF line endings and one line
break added at the end; a test checks that it still is. Edit a file by hand only if the result is
what dawsu! would write, and keep each file's name equal to the map's name plus `.dawsudrum`. A new
preset needs its own file, its own GUIDs and an entry in `BUILT_IN_DRUM_MAPS` in `src/drums.rs`.
