# osu!'s default sounds

These 21 WAV files are osu!'s default hit and slider sounds: hitnormal, hitwhistle, hitfinish, hitclap, sliderslide, slidertick and sliderwhistle for the normal, soft and drum sets. dawsu! plays them when the default sounds folder chosen in Preferences does not have the sound (`src/default_sounds.rs`).

- **Source:** [`osu.Game.Resources/Skins/Legacy/`](https://github.com/ppy/osu-resources/tree/46d344264e5938e168cfbbd2c8fab5f572ebeda4/osu.Game.Resources/Skins/Legacy) in ppy/osu-resources, tag `2026.1021.0` (commit `46d344264e5938e168cfbbd2c8fab5f572ebeda4`). That is the resources version osu!lazer `131fe72` uses (`osu.Game/osu.Game.csproj:42`) and the folder its default classic skin reads (`osu.Game/Skinning/DefaultLegacySkin.cs:42`).
- **Creator and copyright:** ppy Pty Ltd and the osu-resources contributors.
- **Licence:** [Creative Commons Attribution-NonCommercial 4.0 International](https://creativecommons.org/licenses/by-nc/4.0/) (CC BY-NC 4.0). The full text, as osu-resources ships it, is in [LICENCE.md](LICENCE.md). The licence gives no warranty (its Section 5).
- **Changes:** none. The files are byte-for-byte copies. The 12 hit sounds are the same files dawsu! 3.1.4 used.

These sounds keep their own licence, separate from dawsu!. They may be shared only for non-commercial purposes: dawsu! and any build that contains these files must not be sold or used mainly for commercial gain. Using them does not mean ppy endorses dawsu!.
