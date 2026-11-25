import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import KeysoundBounceModal from "../src/parts/KeysoundBounceModal.svelte";

test("KeysoundBounceModal reuses OszBudgetGauge and BounceSampleBank with dedup stats", () => {
  const html = render(KeysoundBounceModal, {
    props: {
      open: true,
      sourceNotes: 100,
      baseBytes: 3 * 1024 * 1024,
      bouncedBytes: 2 * 1024 * 1024,
      samples: [
        { filename: "ks-c5-v100.ogg", durationMs: 300, peakDbfs: -1.5, usageCount: 28, silent: false },
      ],
    },
  }).body;
  assert.match(html, /100 MIDI notes/);
  assert.match(html, /99% deduplicated/);
  assert.match(html, /28×/);
  assert.match(html, /Ranking safe/);
  assert.doesNotMatch(html, /Storyboard/);
});
