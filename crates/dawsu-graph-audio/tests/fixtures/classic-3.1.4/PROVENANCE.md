# Classic drum detection fixture (dawsu! 3.1.4)

Produced by the dump test `DrumParityDump.cs.txt` (a copy kept here) run inside the old app's
test project at `E:/GitHub/dawsu-hitsounding-editor`, commit
`c81774eb212c530e928cd184f739a379cecccc0b` (dawsu! 3.1.4). The test was added there without a
commit and calls 3.1.4's own code; it changed none of it. Run with:

    DAWSU_PARITY_OUT=<folder> dotnet test tests/OsuHitsound.App.Tests --filter FullyQualifiedName~DrumParityDump

Run on 2026-11-20: .NET SDK 10.0.404, runtime .NET 10.0.15, Windows 10.0.26200, x64.

| File | Content |
|---|---|
| `pattern.f32` | `DrumOnsetsTests.Pattern()` audio (120 BPM, 17 s, 44.1 kHz mono), raw little-endian f32. It exceeds ±1.0, so it is not a WAV. |
| `window.json` | Hann window and its sum; per FFT stage the cos/sin step and the whole twiddle recurrence, as f64 bit patterns. |
| `fft-frame50.json` | Frame 50 (500 ms), windowed, before and after 3.1.4's private `Fft`, as f64 bit patterns. |
| `curve-*.f64` | Normalised onset curves (kick, body, noise, hat, snare), raw little-endian f64, one value per 10 ms frame. |
| `flux-kick.f64` | The kick band's flux before normalising. |
| `meta.json` | Sizes, band bins, SHA-256 of the magnitude and percussive spectrograms (f32 LE), `ln(1 + 3v)` bit patterns. |
| `onsets.json` | `DrumOnsets.Detect` at threshold scales 2.0, 1.5, 1.25 and 1.0. |
| `create.json` | `DrumAutoFillPlan.Create` inputs and results (hits, counters, lag) for the `DrumAutoFillTests` cases and for the detected pattern. |

SHA-256 at the time of the dump:

    f41c4cfd8ace167627999ffa8ca8834a1e6103c2652f134c6e10ff4c9c904cbd  DrumParityDump.cs.txt
    6f9cdaeaf54425424531d02ecb082afa8cf9d4786aaa7feb6d77f05b97f8d212  create.json
    ccd44de59f51c7359b1cd110effe26cc592505a73d9a0034c8ba7c4e68ade284  curve-body.f64
    3230354259d37c7f87b0546bcb6c8d07cfc2a7b6e469a6f907897bfa47399ee5  curve-hat.f64
    eabc46e8893f3a5c46e3fd6583131b906ee79745123611ef3cae317837a3163c  curve-kick.f64
    dd45b34e668b6f81d40074aab5297a7ff0a4a8d1c6809755c3d5e10ca5bed50d  curve-noise.f64
    29b52612d612672ad3fd3749f8e7a755aced3d62a88e5ccced515c1f082a54dd  curve-snare.f64
    bf8f0307daecdfc8465528edb00fd949b8ff65d7acd09ac09df0d2dc987a2ddd  fft-frame50.json
    c5493d170aabcd25b35dcc8ac59203932239b1ac1f1e076ed72aa7264ee3381f  flux-kick.f64
    5966f121d5d0e6fd3274ccec9ca29572cbccbd5285ecc68df002540ec6b841d0  meta.json
    4ea0482b840ec2a896021f5f27bc3a873b4ee0a0b4ae6811a22773a41ca520b9  onsets.json
    d52ec29bab1b23308763745816de0f7976e17ec7e4381337017fa5bc7f335416  pattern.f32
    e66499053a9748d15918c672cdf0f30bb45ebffe3cbc61877d0b32d92fa98293  window.json
