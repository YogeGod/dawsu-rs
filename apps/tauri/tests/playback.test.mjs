import assert from "node:assert/strict";
import { test } from "node:test";

test("transport speed buttons offer the C# editor's supported pitch-preserving speeds", async () => {
  const { PLAYBACK_SPEEDS } = await import("../ui/src/lib/statusbar.js");
  assert.deepEqual(PLAYBACK_SPEEDS, [0.25, 0.5, 0.75, 1, 1.25, 1.5]);
});

import { readEditorSource } from "./editor-runtime.mjs";

test("Audio controls offer the four playback start positions and Return on stop", async () => {
  const editor = readEditorSource();
  for (const label of ["Start from", "Cursor", "Left locator", "Selection", "Selection or left locator", "Return on stop"]) {
    assert.ok(editor.includes('"' + label + '"'), label);
  }
});

test("Space from the Export modal plays here while it is the one modal open", () => {
  const editor = readEditorSource();
  const listener = editor.match(/listen\("preview-space", \(\) => \{([\s\S]*?)\n  \}\);/)[1];
  assert.match(listener, /spaceBlocked\(\)/);
  assert.doesNotMatch(listener, /modalOpen\(\)/, "an open Export modal no longer swallows its own Space");
  // Any other modal, Preferences, the palette, Keysounds or a <dialog> still blocks it.
  assert.match(editor, /const spaceBlocked = \(\) => otherModalOpen\(\) \|\| keysoundsMode\(\) \|\|\s*\(env === globalThis && hasOpenModal\(\) && !onlyModalOpen\("export"\)\);/);
});
