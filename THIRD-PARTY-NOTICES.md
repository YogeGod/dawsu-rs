# Third-party notices

dawsu! ships with no licence of its own (ADR 0023). Some of its code is ported from projects under the MIT Licence, and keeps their notice (ADR 0027).

## osu!lazer (ppy/osu)

Ported at commit `131fe72a15581049401a588a13684938d4d0e0c7` (release 2026.1026.0), into `crates/dawsu-hitsounding/src/lazer/`:

- `control_points.rs`: osu.Game/Beatmaps/Formats/LegacyBeatmapDecoder.cs, osu.Game/Beatmaps/Legacy/LegacyControlPointInfo.cs, osu.Game/Beatmaps/ControlPoints/
- `samples.rs`: osu.Game/Rulesets/Objects/Legacy/ConvertHitObjectParser.cs, osu.Game/Audio/HitSampleInfo.cs, osu.Game/Skinning/LegacySkin.cs, osu.Game/Skinning/LegacyBeatmapSkin.cs
- `slider_path.rs`: osu.Game/Rulesets/Objects/Legacy/ConvertHitObjectParser.cs, osu.Game/Rulesets/Objects/SliderPath.cs
- `slider_events.rs`: osu.Game/Rulesets/Objects/SliderEventGenerator.cs
- `crates/dawsu-hitsounding/src/rules.rs` follows osu.Game.Rulesets.Osu/Objects/Slider.cs, osu.Game.Rulesets.Osu/Beatmaps/OsuBeatmapConverter.cs, osu.Game/Rulesets/Objects/Legacy/ConvertSlider.cs and osu.Game/Rulesets/Objects/Legacy/LegacyRulesetExtensions.cs
- `crates/dawsu-osu-format/src/number.rs` and `records.rs` follow osu.Game/Beatmaps/Formats/Parsing.cs and ConvertHitObjectParser.cs

```
Copyright (c) 2025 ppy Pty Ltd <contact@ppy.sh>.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## osu!framework (ppy/osu-framework)

Ported at commit `06e28ecb30f9136304a4cd8889550884bf05a346` (release 2026.1019.0), into `crates/dawsu-hitsounding/src/lazer/path_approximator.rs`: osu.Framework/Utils/PathApproximator.cs, osu.Framework/Utils/CircularArcProperties.cs, osu.Framework/Utils/Precision.cs

```
Copyright (c) 2024 ppy Pty Ltd <contact@ppy.sh>.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

## Mapping Tools (OliBomby/Mapping_Tools)

`crates/dawsu-hitsounding/src/copy.rs` follows the behaviour and options of the Hitsound Copier in Mapping Tools (https://github.com/OliBomby/Mapping_Tools), by OliBomby and contributors, licensed under the MIT Licence. Its licence file holds no copyright line; the permission notice is the one above.

## Rust crates used by `dawsu-hitsounding`

Each is used under its MIT Licence (where a crate offers a choice, MIT is the one taken). The permission notice is the one above; the copyright lines are those of each crate's `LICENSE-MIT` file, and the crates whose file has none are listed by their authors.

- roxmltree 0.21.2 (reads Cubase `.drm` files, ADR 0046): Copyright (c) 2018 Yevhenii Reizner
- memchr 2.8.3 (used by roxmltree and serde_json): Copyright (c) 2015 Andrew Gallant
- serde 1.0.229, serde_core 1.0.229, serde_derive 1.0.229 and serde_json 1.0.154 (read and write `.dawsudrum` files, ADR 0047): Erick Tryzelaar and David Tolnay; no copyright line in the licence file
- itoa 1.0.19 and zmij 1.0.23 (used by serde_json): David Tolnay; no copyright line in the licence file
- At build time only (procedural macros, not shipped): proc-macro2, quote, syn and unicode-ident (MIT; unicode-ident's Unicode data is under the Unicode License v3)
