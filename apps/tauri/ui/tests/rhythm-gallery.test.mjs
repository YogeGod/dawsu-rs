import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { activeBarMs, rhythmPresets, rhythmStampAction } from "../src/lib/rhythmGallery.js";
import InspectorSelection from "../src/parts/InspectorSelection.svelte";

test("activeBarMs uses patternStampStep when present and falls back cleanly", () => {
  assert.equal(activeBarMs({ patternStampStep: 1500 }), 1500);
  assert.equal(activeBarMs({ beatLengthMs: 400, beatsPerBar: 3 }), 1200);
  assert.equal(activeBarMs(null), 2000);
});

test("rhythmPresets computes exact beat/bar subdivisions for 120 BPM 4/4 (2000 ms bar)", () => {
  const presets = rhythmPresets(2000, 4);
  const byId = Object.fromEntries(presets.map(p => [p.id, p]));
  assert.equal(byId["half-bar"].stepMs, 1000);
  assert.equal(byId["bar"].stepMs, 2000);
  assert.equal(byId["beat"].stepMs, 500);
  assert.equal(byId["offbeat"].stepMs, 250);
  assert.equal(byId["roll"].stepMs, 125);
  assert.equal(byId["two-bars"].stepMs, 4000);

  const action = rhythmStampAction(byId["half-bar"]);
  assert.deepEqual(action, {
    kind: "pattern_stamp",
    count: 4,
    step_ms: 1000,
    snap_to_objects: true,
    quantize_volumes: false,
  });
});

test("InspectorSelection renders Rhythm gallery chips when hits are selected and hides them when empty", () => {
  bridge.scene = { patternStampStep: 2000 };
  bridge.inspector = {
    busy: false,
    doc: true,
    selection: {
      summary: "2 hits selected",
      start: "00:01.000",
      volume: "80%",
      object: "Circle",
      sampleSet: "normal",
      additions: { whistle: "off", finish: "off", clap: "on" },
      sliderBody: null,
    },
    scope: "all",
    hint: "",
    range: false,
    insertVolume: 100,
    protect: false,
    audition: true,
  };
  const selectedHtml = render(InspectorSelection).body;
  assert.match(selectedHtml, /aria-label="Rhythm gallery"/);
  assert.match(selectedHtml, />1 &amp; 3</);
  assert.match(selectedHtml, />1\/2</);
  assert.match(selectedHtml, />1\/4</);
  assert.match(selectedHtml, />Custom…</);

  bridge.inspector = { ...bridge.inspector, selection: null };
  const emptyHtml = render(InspectorSelection).body;
  assert.doesNotMatch(emptyHtml, /aria-label="Rhythm gallery"/);
});
