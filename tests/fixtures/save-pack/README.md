# Save pack

Files that must survive a save from dawsu! with every unedited line byte-identical (ADR 0028 in the design vault). They are copied byte for byte from the old app's repo, `YogeGod/dawsu-hitsounding-editor`, at commit `c81774e`, folder `tests/fixtures/save-pack/` (ADR 0033). Only this README was rewritten.

- `synthetic/`: small hand-written files, one per edge case: BOM, mixed or missing line endings, format v3, v7, v14 and v128, lines that are not valid UTF-8, storyboard and colours, unknown sections and comments, custom samples and slider edges, mania, taiko and catch.
- `owner/`: Yoge's own maps, copied unchanged from osu!stable's Songs folder: Catalyst [Impetus] (standard), Catalyst [Hitsounds] (a hitsound diff), Mukduk (written by osu!lazer: v128 with a BOM) and Periphery - Satellites [18K MIDI Volumes] (mania with per-note keysounds). Only the `.osu` files, no audio.

No other mappers' maps go here; the corpus stays local and is not committed.

`.gitattributes` at the repo root marks this folder `-text`, so git never rewrites the line endings these tests depend on. Tests never write here: each test copies what it needs into its own temporary folder first.
