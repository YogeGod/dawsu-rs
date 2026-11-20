# Third-party notices

dawsu! ships with no licence of its own (ADR 0023). Some of its code is ported from projects under the MIT Licence, and keeps their notice (ADR 0027).

## osu!lazer (ppy/osu)

Ported at commit `131fe72a15581049401a588a13684938d4d0e0c7` (release 2026.1026.0). Short file names are in `crates/dawsu-hitsounding/src/lazer/`; files elsewhere are named by their full path:

- `control_points.rs`: osu.Game/Beatmaps/Formats/LegacyBeatmapDecoder.cs, osu.Game/Beatmaps/Legacy/LegacyControlPointInfo.cs, osu.Game/Beatmaps/ControlPoints/
- `samples.rs`: osu.Game/Rulesets/Objects/Legacy/ConvertHitObjectParser.cs, osu.Game/Audio/HitSampleInfo.cs, osu.Game/Skinning/LegacySkin.cs, osu.Game/Skinning/LegacyBeatmapSkin.cs
- `slider_path.rs`: osu.Game/Rulesets/Objects/Legacy/ConvertHitObjectParser.cs, osu.Game/Rulesets/Objects/SliderPath.cs
- `slider_events.rs`: osu.Game/Rulesets/Objects/SliderEventGenerator.cs
- `crates/dawsu-hitsounding/src/rules.rs` follows osu.Game.Rulesets.Osu/Objects/Slider.cs, osu.Game.Rulesets.Osu/Beatmaps/OsuBeatmapConverter.cs, osu.Game/Rulesets/Objects/Legacy/ConvertSlider.cs and osu.Game/Rulesets/Objects/Legacy/LegacyRulesetExtensions.cs
- `crates/dawsu-osu-format/src/number.rs` and `records.rs` follow osu.Game/Beatmaps/Formats/Parsing.cs and ConvertHitObjectParser.cs
- `crates/dawsu-editor/src/tempo.rs` (section lookup and snapping) follows osu.Game/Beatmaps/ControlPoints/ControlPointInfo.cs
- `apps/tauri/dist/app.js` (snap colours) follows osu.Game/Screens/Edit/BindableBeatDivisor.cs and osu.Game/Graphics/OsuColour.cs

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

## Rust crates used by `dawsu-osu-format`

Used to read and write `.osz` archives (ADR 0048). Each is used under its MIT Licence where it offers a choice; the permission notice is the one above, and the copyright lines are those of each crate's licence file.

- zip 9.6.0: Copyright (c) 2014 Mathijs van de Nes
- flate2 1.1.11: Copyright (c) 2014-2026 Alex Crichton
- crc32fast 1.5.5: Copyright (c) 2018 Sam Rijs, Alex Crichton and contributors
- cfg-if 1.0.5: Copyright (c) 2014 Alex Crichton
- hashbrown 0.17.1: Copyright (c) 2016 Amanieu d'Antras
- indexmap 2.14.2 and equivalent 1.0.3: the indexmap authors (licence files without a name)
- typed-path 0.13.0: Chip Senkbeil (no copyright line in the licence file)
- zeroize 1.9.1 and zeroize_derive 1.5.0: Copyright (c) 2018-2026 The RustCrypto Project Developers
- getrandom 0.4.3: Copyright (c) 2018-2026 The rust-random Project Developers
- windows-sys 0.61.2 and windows-link 0.2.1 (Windows only): Copyright (c) Microsoft Corporation
- constant_time_eq 0.6.1: used under MIT-0, which asks for no notice; listed for completeness
- zlib-rs 0.6.8: (C) 2024 Trifecta Tech Foundation, under the zlib licence, which asks only that its origin is not misrepresented and that its notice stays in its source

## Svelte (sveltejs/svelte)

The editor's Svelte parts in `apps/tauri/dist/ui/ui.js` include the Svelte 5.57.3 runtime, compiled in by Vite. It is used under the MIT Licence; the text below is Svelte's `LICENSE.md`. Vite and the Svelte compiler run only at build time and ship nothing.

```
Copyright (c) 2016-2025 [Svelte Contributors](https://github.com/sveltejs/svelte/graphs/contributors)

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```
