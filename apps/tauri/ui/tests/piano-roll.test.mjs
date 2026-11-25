import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { isBlackKey, pitchToY, previewDraggedNote, sustainSpanAt, yToPitch } from "../src/lib/pianoRoll.js";
import PianoKeyboard from "../src/parts/PianoKeyboard.svelte";

test("pianoRoll coordinate, local drag preview, and CC64 sustain helpers work accurately", () => {
  assert.equal(isBlackKey(61), true);
  assert.equal(isBlackKey(60), false);
  assert.equal(pitchToY(72, 84, 20), 240);
  assert.equal(yToPitch(240, 84, 20), 72);

  const moved = previewDraggedNote({ startMs: 1000, endMs: 1200, pitch: 60 }, 250, 4, "move");
  assert.deepEqual(moved, { startMs: 1250, endMs: 1450, pitch: 64 });

  const resized = previewDraggedNote({ startMs: 1000, endMs: 1200, pitch: 60 }, -500, 0, "resize");
  assert.equal(resized.endMs, 1020);

  const span = sustainSpanAt([{ start_ms: 500, end_ms: 1500 }], 900);
  assert.equal(span.start_ms, 500);
});

test("PianoKeyboard renders clickable keys across the visible pitch range", () => {
  const html = render(PianoKeyboard, { props: { lowPitch: 60, highPitch: 72, rowHeight: 18 } }).body;
  assert.match(html, /C4/);
  assert.match(html, /C5/);
});
