import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { budgetTone, dedupPercent, formatMegabytes } from "../src/lib/bounce.js";
import Vst3BounceModal from "../src/parts/Vst3BounceModal.svelte";

test("bounce budget and deduplication helpers compute thresholds accurately", () => {
  assert.equal(formatMegabytes(5 * 1024 * 1024), "5.00 MB");
  assert.equal(budgetTone(5 * 1024 * 1024), "safe");
  assert.equal(budgetTone(9 * 1024 * 1024), "caution");
  assert.equal(budgetTone(11 * 1024 * 1024), "over");
  assert.equal(dedupPercent(100, 20), 80);
});

test("Vst3BounceModal renders Ogg/WAV format selector, dBFS normalization, budget gauge, and sample bank", () => {
  const html = render(Vst3BounceModal, {
    props: {
      open: true,
      baseBytes: 4 * 1024 * 1024,
      bouncedBytes: 1 * 1024 * 1024,
      samples: [
        { filename: "drum-kick-hard.ogg", durationMs: 240, peakDbfs: -1.5, usageCount: 42, silent: false },
        { filename: "drum-hat-ghost.ogg", durationMs: 90, peakDbfs: -99, usageCount: 4, silent: true },
      ],
    },
  }).body;
  assert.match(html, /Ogg Vorbis/);
  assert.match(html, /Uniform kit peak normalize \(-1\.5 dBFS\)/);
  assert.match(html, /Ranking safe/);
  assert.match(html, /42×/);
  assert.match(html, /Silent render/);
});
