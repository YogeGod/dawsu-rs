import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import KeysoundSetupBar from "../src/parts/KeysoundSetupBar.svelte";

test("KeysoundSetupBar renders track picker, undo/redo, .dawsuks save, and chord badge without Storyboard controls", () => {
  const html = render(KeysoundSetupBar, {
    props: {
      open: true,
      tracks: [{ id: 1, name: "Grand Piano", noteCount: 120 }],
      activeTrackId: 1,
      chordCount: 6,
      canUndo: true,
      canRedo: false,
      dirty: true,
    },
  }).body;
  assert.match(html, /Grand Piano \(120 notes\)/);
  assert.match(html, /Save \.dawsuks\*/);
  assert.match(html, /6 chords to resolve/);
  assert.doesNotMatch(html, /Storyboard/);
});
