import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

test("transport speed buttons offer the C# editor's supported pitch-preserving speeds", async () => {
  const { PLAYBACK_SPEEDS } = await import("../ui/src/lib/statusbar.js");
  assert.deepEqual(PLAYBACK_SPEEDS, [0.25, 0.5, 0.75, 1, 1.25, 1.5]);
});

test("Audio controls offer the four playback start positions and Return on stop", async () => {
  const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
  for (const label of ["Start from", "Cursor", "Left locator", "Selection", "Selection or left locator", "Return on stop"]) {
    assert.ok(app.includes('"' + label + '"'), label);
  }
});
