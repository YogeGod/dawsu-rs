import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { tooltipDelay } from "../src/lib/hitCard.js";
import HitCard from "../src/parts/HitCard.svelte";

const card = { key: "1:100", title: "Clap · Normal set", time: "00:00.100", detail: "Volume 60% · on circle 1", x: 300, y: 200 };

test("no card before a hit is hovered", () => {
  bridge.hitCard = null;
  assert.doesNotMatch(render(HitCard).body, /hit-card/);
});

// states.md Tooltips: line 1 the name and the time, line 2 the detail. No gesture hint: Alt draws
// on the timeline and no Alt-drag copy exists.
test("the card shows the lane and set, the time, then volume and object", () => {
  bridge.hitCard = card;
  const html = render(HitCard).body;
  assert.match(html, /<div class="hit-card[^"]*" role="tooltip"/);
  assert.match(html, /<span class="title[^"]*">Clap · Normal set<\/span>\s*<span class="time[^"]*">00:00.100<\/span>/);
  assert.match(html, /<div class="detail[^"]*">Volume 60% · on circle 1<\/div>/);
  assert.doesNotMatch(html, /Alt-drag|Del removes/);
  // Placed at the hit through style: directives (CSSOM in the window, which the CSP allows).
  assert.match(html, /--hit-x: 300px/);
  assert.match(html, /--hit-y: 200px/);
});

test("the card waits 500 ms, or shows at once within 1 s of the last one", () => {
  assert.equal(tooltipDelay(10_000, null), 500);
  assert.equal(tooltipDelay(10_000, 9_200), 0);
  assert.equal(tooltipDelay(10_000, 8_900), 500);
});

test("the card stays inside the window", async () => {
  const { readFile } = await import("node:fs/promises");
  const source = await readFile(new URL("../src/parts/HitCard.svelte", import.meta.url), "utf8");
  assert.match(source, /left: clamp\(/);
  assert.match(source, /top: max\(/);
});
