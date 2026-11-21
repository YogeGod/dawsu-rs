//! Built-in `.dawsugraph` presets (stored here so `dawsu-editor-nodal` never references `dawsu-graph-audio`).

use dawsu_graph::{DocumentError, GraphDocument, NodeId, NodeRegistry};

/// Canonical `.dawsugraph` v1 JSON for the "Kick & snare (classic)" preset.
pub const CLASSIC_KICK_SNARE_JSON: &str = r#"{
  "format": "dawsu-graph",
  "version": 1,
  "name": "Kick & snare (classic)",
  "description": "Reproduces dawsu! 3.1.4 drum hit detection.",
  "nodes": [
    { "id": "range", "type": "input.range", "typeVersion": 1, "params": { "mode": "selection" }, "muted": false, "ui": { "x": 0.0, "y": 0.0 } },
    { "id": "song", "type": "input.song", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 180.0, "y": 0.0 } },
    { "id": "timing", "type": "input.timing", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 180.0, "y": 280.0 } },
    { "id": "mono", "type": "audio.mono", "typeVersion": 1, "params": { "mix": "average", "peakNormalise": true, "normaliseOver": "range", "silenceBelow": 1e-6 }, "muted": false, "ui": { "x": 360.0, "y": 0.0 } },
    { "id": "stft", "type": "spectral.stft", "typeVersion": 1, "params": { "window": 2048, "windowShape": "hann", "hopMs": 10.0, "maxHz": 11000.0, "minHops": 4 }, "muted": false, "ui": { "x": 540.0, "y": 0.0 } },
    { "id": "hpss", "type": "spectral.hpss", "typeVersion": 1, "params": { "medianHalf": 8, "mask": "soft-squared", "epsilon": 1e-20, "keep": "percussive" }, "muted": false, "ui": { "x": 720.0, "y": 0.0 } },
    { "id": "flux_kick", "type": "onset.flux", "typeVersion": 1, "params": { "loHz": 30.0, "hiHz": 120.0, "compression": 3.0, "maxBins": 3, "normalise": "p90", "normaliseOver": "range" }, "muted": false, "ui": { "x": 920.0, "y": -160.0 } },
    { "id": "flux_body", "type": "onset.flux", "typeVersion": 1, "params": { "loHz": 150.0, "hiHz": 400.0, "compression": 3.0, "maxBins": 3, "normalise": "p90", "normaliseOver": "range" }, "muted": false, "ui": { "x": 920.0, "y": -20.0 } },
    { "id": "flux_wires", "type": "onset.flux", "typeVersion": 1, "params": { "loHz": 1500.0, "hiHz": 5000.0, "compression": 3.0, "maxBins": 3, "normalise": "p90", "normaliseOver": "range" }, "muted": false, "ui": { "x": 920.0, "y": 120.0 } },
    { "id": "flux_hat", "type": "onset.flux", "typeVersion": 1, "params": { "loHz": 7000.0, "hiHz": 11000.0, "compression": 3.0, "maxBins": 3, "normalise": "p90", "normaliseOver": "range" }, "muted": false, "ui": { "x": 920.0, "y": 260.0 } },
    { "id": "snare_curve", "type": "curve.math", "typeVersion": 1, "params": { "op": "geomean", "factor": 1.0, "normalise": "p90", "normaliseOver": "range" }, "muted": false, "ui": { "x": 1140.0, "y": 40.0 } },
    { "id": "peaks_kick", "type": "events.peaks", "typeVersion": 1, "params": { "threshold": 1.5, "floorScale": 0.3, "deltaScale": 0.12, "maxHalfFrames": 3, "meanBefore": 10, "meanAfter": 3, "minGapMs": 60.0, "biasMs": 4.0, "tag": "kick" }, "muted": false, "ui": { "x": 1340.0, "y": -160.0 } },
    { "id": "peaks_snare", "type": "events.peaks", "typeVersion": 1, "params": { "threshold": 1.5, "floorScale": 0.3, "deltaScale": 0.12, "maxHalfFrames": 3, "meanBefore": 10, "meanAfter": 3, "minGapMs": 60.0, "biasMs": 4.0, "tag": "snare" }, "muted": false, "ui": { "x": 1340.0, "y": 40.0 } },
    { "id": "measure_kick", "type": "events.measure", "typeVersion": 1, "params": { "name": "kick", "halfFrames": 2, "reduce": "max" }, "muted": false, "ui": { "x": 1540.0, "y": 40.0 } },
    { "id": "measure_body", "type": "events.measure", "typeVersion": 1, "params": { "name": "body", "halfFrames": 2, "reduce": "max" }, "muted": false, "ui": { "x": 1720.0, "y": 40.0 } },
    { "id": "measure_hat", "type": "events.measure", "typeVersion": 1, "params": { "name": "hat", "halfFrames": 2, "reduce": "max" }, "muted": false, "ui": { "x": 1900.0, "y": 40.0 } },
    {
      "id": "kick_bleed",
      "type": "events.condition",
      "typeVersion": 1,
      "params": {
        "action": "drop",
        "clauses": [
          [
            { "lhs": "strength", "op": "<", "factor": 1.0, "rhs": "const" },
            { "lhs": "strength", "op": "<", "factor": 0.75, "rhs": "kick" }
          ],
          [
            { "lhs": "body", "op": "<", "factor": 1.3, "rhs": "kick" },
            { "lhs": "hat", "op": "<", "factor": 0.5, "rhs": "kick" }
          ]
        ]
      },
      "muted": false,
      "ui": { "x": 2080.0, "y": 40.0 }
    },
    { "id": "merge", "type": "events.merge", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 2280.0, "y": -60.0 } },
    { "id": "snap", "type": "events.snap", "typeVersion": 1, "params": { "divisors": [4], "toleranceMs": 20.0, "measureLag": true, "maxLagMs": 50.0, "minEventsForLag": 8, "lagRefineWindowMs": 25.0, "lagRefinePasses": 2, "dropOffGrid": true }, "muted": false, "ui": { "x": 2460.0, "y": -60.0 } },
    { "id": "out_kick", "type": "output.lane", "typeVersion": 1, "params": { "slot": "kick", "tagFilter": "kick" }, "muted": false, "ui": { "x": 2660.0, "y": -140.0 } },
    { "id": "out_snare", "type": "output.lane", "typeVersion": 1, "params": { "slot": "snare", "tagFilter": "snare" }, "muted": false, "ui": { "x": 2660.0, "y": 20.0 } }
  ],
  "links": [
    { "from": { "node": "range", "socket": "range" }, "to": { "node": "song", "socket": "range" } },
    { "from": { "node": "song", "socket": "audio" }, "to": { "node": "mono", "socket": "audio" } },
    { "from": { "node": "mono", "socket": "audio" }, "to": { "node": "stft", "socket": "audio" } },
    { "from": { "node": "stft", "socket": "spectrogram" }, "to": { "node": "hpss", "socket": "spectrogram" } },
    { "from": { "node": "hpss", "socket": "spectrogram" }, "to": { "node": "flux_kick", "socket": "spectrogram" } },
    { "from": { "node": "hpss", "socket": "spectrogram" }, "to": { "node": "flux_body", "socket": "spectrogram" } },
    { "from": { "node": "hpss", "socket": "spectrogram" }, "to": { "node": "flux_wires", "socket": "spectrogram" } },
    { "from": { "node": "hpss", "socket": "spectrogram" }, "to": { "node": "flux_hat", "socket": "spectrogram" } },
    { "from": { "node": "flux_body", "socket": "curve" }, "to": { "node": "snare_curve", "socket": "a" } },
    { "from": { "node": "flux_wires", "socket": "curve" }, "to": { "node": "snare_curve", "socket": "b" } },
    { "from": { "node": "flux_kick", "socket": "curve" }, "to": { "node": "peaks_kick", "socket": "curve" } },
    { "from": { "node": "snare_curve", "socket": "curve" }, "to": { "node": "peaks_snare", "socket": "curve" } },
    { "from": { "node": "peaks_snare", "socket": "events" }, "to": { "node": "measure_kick", "socket": "events" } },
    { "from": { "node": "flux_kick", "socket": "curve" }, "to": { "node": "measure_kick", "socket": "curve" } },
    { "from": { "node": "measure_kick", "socket": "events" }, "to": { "node": "measure_body", "socket": "events" } },
    { "from": { "node": "flux_body", "socket": "curve" }, "to": { "node": "measure_body", "socket": "curve" } },
    { "from": { "node": "measure_body", "socket": "events" }, "to": { "node": "measure_hat", "socket": "events" } },
    { "from": { "node": "flux_hat", "socket": "curve" }, "to": { "node": "measure_hat", "socket": "curve" } },
    { "from": { "node": "measure_hat", "socket": "events" }, "to": { "node": "kick_bleed", "socket": "events" } },
    { "from": { "node": "peaks_kick", "socket": "events" }, "to": { "node": "merge", "socket": "events" } },
    { "from": { "node": "kick_bleed", "socket": "events" }, "to": { "node": "merge", "socket": "events" } },
    { "from": { "node": "merge", "socket": "events" }, "to": { "node": "snap", "socket": "events" } },
    { "from": { "node": "timing", "socket": "timing" }, "to": { "node": "snap", "socket": "timing" } },
    { "from": { "node": "snap", "socket": "events" }, "to": { "node": "out_kick", "socket": "events" } },
    { "from": { "node": "snap", "socket": "events" }, "to": { "node": "out_snare", "socket": "events" } }
  ],
  "outputs": [
    { "slot": "kick" },
    { "slot": "snare" }
  ]
}
"#;

/// Loads the built-in "Kick & snare (classic)" preset against `registry`.
pub fn classic_kick_snare_graph(registry: &NodeRegistry) -> Result<GraphDocument, DocumentError> {
    GraphDocument::from_json(CLASSIC_KICK_SNARE_JSON, registry)
}

/// Adjusts the peak-picking sensitivity threshold (`2.0` strict .. `1.0` loose) on the classic preset.
pub fn set_classic_sensitivity(doc: &mut GraphDocument, threshold: f64) {
    for id in ["peaks_kick", "peaks_snare"] {
        if let Some(node) = doc.node_mut(&NodeId::from(id)) {
            node.params
                .insert("threshold".to_owned(), serde_json::json!(threshold));
        }
    }
}

/// Mutes or unmutes the beat-snap node on the classic preset (for raw onset extraction or parity checks).
pub fn set_classic_snap_muted(doc: &mut GraphDocument, muted: bool) {
    if let Some(node) = doc.node_mut(&NodeId::from("snap")) {
        node.muted = muted;
    }
}

/// Starter recipe: straightforward kick detector (no bleed-filter chain).
pub const SIMPLE_KICK_JSON: &str = r#"{
  "format": "dawsu-graph",
  "version": 1,
  "name": "Simple Kick",
  "description": "Straightforward low-end kick detector.",
  "nodes": [
    { "id": "song", "type": "input.song", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 40.0, "y": 40.0 } },
    { "id": "mono", "type": "audio.mono", "typeVersion": 1, "params": { "mix": "average", "peakNormalise": true, "normaliseOver": "range", "silenceBelow": 1e-6 }, "muted": false, "ui": { "x": 270.0, "y": 40.0 } },
    { "id": "stft", "type": "spectral.stft", "typeVersion": 1, "params": { "window": 2048, "windowShape": "hann", "hopMs": 10.0, "maxHz": 11000.0, "minHops": 4 }, "muted": false, "ui": { "x": 500.0, "y": 40.0 } },
    { "id": "hpss", "type": "spectral.hpss", "typeVersion": 1, "params": { "medianHalf": 8, "mask": "soft-squared", "epsilon": 1e-20, "keep": "percussive" }, "muted": false, "ui": { "x": 730.0, "y": 40.0 } },
    { "id": "flux_kick", "type": "onset.flux", "typeVersion": 1, "params": { "loHz": 30.0, "hiHz": 120.0, "compression": 3.0, "maxBins": 3, "normalise": "p90", "normaliseOver": "range" }, "muted": false, "ui": { "x": 960.0, "y": 40.0 } },
    { "id": "peaks_kick", "type": "events.peaks", "typeVersion": 1, "params": { "threshold": 1.4, "floorScale": 0.3, "deltaScale": 0.12, "maxHalfFrames": 3, "meanBefore": 10, "meanAfter": 3, "minGapMs": 60.0, "biasMs": 4.0, "tag": "kick" }, "muted": false, "ui": { "x": 1190.0, "y": 40.0 } },
    { "id": "timing", "type": "input.timing", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 1190.0, "y": 170.0 } },
    { "id": "snap", "type": "events.snap", "typeVersion": 1, "params": { "divisors": [4], "toleranceMs": 20.0, "measureLag": true, "maxLagMs": 50.0, "minEventsForLag": 8, "lagRefineWindowMs": 25.0, "lagRefinePasses": 2, "dropOffGrid": true }, "muted": false, "ui": { "x": 1420.0, "y": 80.0 } },
    { "id": "out_kick", "type": "output.lane", "typeVersion": 1, "params": { "slot": "kick", "tagFilter": "kick" }, "muted": false, "ui": { "x": 1650.0, "y": 80.0 } }
  ],
  "links": [
    { "from": { "node": "song", "socket": "audio" }, "to": { "node": "mono", "socket": "audio" } },
    { "from": { "node": "mono", "socket": "audio" }, "to": { "node": "stft", "socket": "audio" } },
    { "from": { "node": "stft", "socket": "spectrogram" }, "to": { "node": "hpss", "socket": "spectrogram" } },
    { "from": { "node": "hpss", "socket": "spectrogram" }, "to": { "node": "flux_kick", "socket": "spectrogram" } },
    { "from": { "node": "flux_kick", "socket": "curve" }, "to": { "node": "peaks_kick", "socket": "curve" } },
    { "from": { "node": "peaks_kick", "socket": "events" }, "to": { "node": "snap", "socket": "events" } },
    { "from": { "node": "timing", "socket": "timing" }, "to": { "node": "snap", "socket": "timing" } },
    { "from": { "node": "snap", "socket": "events" }, "to": { "node": "out_kick", "socket": "events" } }
  ],
  "outputs": [{ "slot": "kick" }]
}
"#;

/// Starter recipe: high-frequency hi-hat / cymbal detector.
pub const SIMPLE_HIHAT_JSON: &str = r#"{
  "format": "dawsu-graph",
  "version": 1,
  "name": "Simple Hi-Hat",
  "description": "Detects bright hi-hat and cymbal transients.",
  "nodes": [
    { "id": "song", "type": "input.song", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 40.0, "y": 40.0 } },
    { "id": "mono", "type": "audio.mono", "typeVersion": 1, "params": { "mix": "average", "peakNormalise": true, "normaliseOver": "range", "silenceBelow": 1e-6 }, "muted": false, "ui": { "x": 270.0, "y": 40.0 } },
    { "id": "stft", "type": "spectral.stft", "typeVersion": 1, "params": { "window": 2048, "windowShape": "hann", "hopMs": 10.0, "maxHz": 12000.0, "minHops": 4 }, "muted": false, "ui": { "x": 500.0, "y": 40.0 } },
    { "id": "hpss", "type": "spectral.hpss", "typeVersion": 1, "params": { "medianHalf": 8, "mask": "soft-squared", "epsilon": 1e-20, "keep": "percussive" }, "muted": false, "ui": { "x": 730.0, "y": 40.0 } },
    { "id": "flux_hat", "type": "onset.flux", "typeVersion": 1, "params": { "loHz": 7000.0, "hiHz": 11000.0, "compression": 3.0, "maxBins": 3, "normalise": "p90", "normaliseOver": "range" }, "muted": false, "ui": { "x": 960.0, "y": 40.0 } },
    { "id": "peaks_hat", "type": "events.peaks", "typeVersion": 1, "params": { "threshold": 1.35, "floorScale": 0.3, "deltaScale": 0.12, "maxHalfFrames": 3, "meanBefore": 10, "meanAfter": 3, "minGapMs": 55.0, "biasMs": 4.0, "tag": "hat" }, "muted": false, "ui": { "x": 1190.0, "y": 40.0 } },
    { "id": "timing", "type": "input.timing", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 1190.0, "y": 170.0 } },
    { "id": "snap", "type": "events.snap", "typeVersion": 1, "params": { "divisors": [4, 8], "toleranceMs": 20.0, "measureLag": true, "maxLagMs": 50.0, "minEventsForLag": 8, "lagRefineWindowMs": 25.0, "lagRefinePasses": 2, "dropOffGrid": true }, "muted": false, "ui": { "x": 1420.0, "y": 80.0 } },
    { "id": "out_hat", "type": "output.lane", "typeVersion": 1, "params": { "slot": "hat", "tagFilter": "hat" }, "muted": false, "ui": { "x": 1650.0, "y": 80.0 } }
  ],
  "links": [
    { "from": { "node": "song", "socket": "audio" }, "to": { "node": "mono", "socket": "audio" } },
    { "from": { "node": "mono", "socket": "audio" }, "to": { "node": "stft", "socket": "audio" } },
    { "from": { "node": "stft", "socket": "spectrogram" }, "to": { "node": "hpss", "socket": "spectrogram" } },
    { "from": { "node": "hpss", "socket": "spectrogram" }, "to": { "node": "flux_hat", "socket": "spectrogram" } },
    { "from": { "node": "flux_hat", "socket": "curve" }, "to": { "node": "peaks_hat", "socket": "curve" } },
    { "from": { "node": "peaks_hat", "socket": "events" }, "to": { "node": "snap", "socket": "events" } },
    { "from": { "node": "timing", "socket": "timing" }, "to": { "node": "snap", "socket": "timing" } },
    { "from": { "node": "snap", "socket": "events" }, "to": { "node": "out_hat", "socket": "events" } }
  ],
  "outputs": [{ "slot": "hat" }]
}
"#;

/// Starter recipe: fast single-drum detector from an isolated drum stem audio file.
pub const DRUM_STEM_JSON: &str = r#"{
  "format": "dawsu-graph",
  "version": 1,
  "name": "Drum Stem (Fast)",
  "description": "Analyses a clean drum stem without needing melodic separation.",
  "nodes": [
    { "id": "stem", "type": "input.file", "typeVersion": 1, "params": { "path": "", "offsetMs": null, "expectLengthOfSong": false }, "muted": false, "ui": { "x": 40.0, "y": 40.0 } },
    { "id": "mono", "type": "audio.mono", "typeVersion": 1, "params": { "mix": "average", "peakNormalise": true, "normaliseOver": "range", "silenceBelow": 1e-6 }, "muted": false, "ui": { "x": 270.0, "y": 40.0 } },
    { "id": "stft", "type": "spectral.stft", "typeVersion": 1, "params": { "window": 2048, "windowShape": "hann", "hopMs": 10.0, "maxHz": 11000.0, "minHops": 4 }, "muted": false, "ui": { "x": 500.0, "y": 40.0 } },
    { "id": "flux_kick", "type": "onset.flux", "typeVersion": 1, "params": { "loHz": 30.0, "hiHz": 120.0, "compression": 3.0, "maxBins": 3, "normalise": "p90", "normaliseOver": "range" }, "muted": false, "ui": { "x": 730.0, "y": 40.0 } },
    { "id": "peaks_kick", "type": "events.peaks", "typeVersion": 1, "params": { "threshold": 1.35, "floorScale": 0.3, "deltaScale": 0.12, "maxHalfFrames": 3, "meanBefore": 10, "meanAfter": 3, "minGapMs": 60.0, "biasMs": 4.0, "tag": "kick" }, "muted": false, "ui": { "x": 960.0, "y": 40.0 } },
    { "id": "timing", "type": "input.timing", "typeVersion": 1, "params": {}, "muted": false, "ui": { "x": 960.0, "y": 170.0 } },
    { "id": "snap", "type": "events.snap", "typeVersion": 1, "params": { "divisors": [4], "toleranceMs": 20.0, "measureLag": true, "maxLagMs": 50.0, "minEventsForLag": 8, "lagRefineWindowMs": 25.0, "lagRefinePasses": 2, "dropOffGrid": true }, "muted": false, "ui": { "x": 1190.0, "y": 80.0 } },
    { "id": "out_kick", "type": "output.lane", "typeVersion": 1, "params": { "slot": "kick", "tagFilter": "kick" }, "muted": false, "ui": { "x": 1420.0, "y": 80.0 } }
  ],
  "links": [
    { "from": { "node": "stem", "socket": "audio" }, "to": { "node": "mono", "socket": "audio" } },
    { "from": { "node": "mono", "socket": "audio" }, "to": { "node": "stft", "socket": "audio" } },
    { "from": { "node": "stft", "socket": "spectrogram" }, "to": { "node": "flux_kick", "socket": "spectrogram" } },
    { "from": { "node": "flux_kick", "socket": "curve" }, "to": { "node": "peaks_kick", "socket": "curve" } },
    { "from": { "node": "peaks_kick", "socket": "events" }, "to": { "node": "snap", "socket": "events" } },
    { "from": { "node": "timing", "socket": "timing" }, "to": { "node": "snap", "socket": "timing" } },
    { "from": { "node": "snap", "socket": "events" }, "to": { "node": "out_kick", "socket": "events" } }
  ],
  "outputs": [{ "slot": "kick" }]
}
"#;

pub fn simple_kick_graph(registry: &NodeRegistry) -> Result<GraphDocument, DocumentError> {
    GraphDocument::from_json(SIMPLE_KICK_JSON, registry)
}

pub fn simple_hihat_graph(registry: &NodeRegistry) -> Result<GraphDocument, DocumentError> {
    GraphDocument::from_json(SIMPLE_HIHAT_JSON, registry)
}

pub fn drum_stem_graph(registry: &NodeRegistry) -> Result<GraphDocument, DocumentError> {
    GraphDocument::from_json(DRUM_STEM_JSON, registry)
}
