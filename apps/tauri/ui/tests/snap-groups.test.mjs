import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { COMMON, divisorColour, nextGroup, snapColour, stepInGroup, TRIPLETS } from "../src/lib/snap.js";
import SnapSlider from "../src/parts/SnapSlider.svelte";

// osu!lazer BeatDivisorPresetCollection.cs:21-23 at 131fe72a15581049401a588a13684938d4d0e0c7.
test("the snap groups are lazer's Common and Triplets", () => {
  assert.deepEqual(COMMON.presets, [1, 2, 4, 8, 16]);
  assert.deepEqual(TRIPLETS.presets, [1, 3, 6, 12]);
});

// BindableBeatDivisor.cs:43-51 (SetArbitraryDivisor), BeatDivisorPresetCollection.cs:25-39 (Custom).
test("a picked snap keeps the group that has it, else Common, else Triplets, else its own whole divisors", () => {
  assert.equal(nextGroup(null, 1, 4), COMMON, "a map opens in Common when its snap is in Common");
  assert.equal(nextGroup(null, 1, 1), COMMON, "1/1 opens in Common");
  assert.equal(nextGroup(null, 1, 6), TRIPLETS);
  assert.equal(nextGroup(COMMON, 1, 3), TRIPLETS);
  assert.equal(nextGroup(TRIPLETS, 1, 12), TRIPLETS);
  assert.equal(nextGroup(TRIPLETS, 1, 1), TRIPLETS, "1/1 keeps the group");
  assert.equal(nextGroup(TRIPLETS, 1, 4), COMMON);
  assert.equal(nextGroup(TRIPLETS, 1, 2), COMMON);
  assert.deepEqual(nextGroup(COMMON, 1, 5).presets, [1, 5]);
  assert.deepEqual(nextGroup(TRIPLETS, 1, 24).presets, [1, 2, 3, 4, 6, 8, 12, 24]);
  assert.equal(nextGroup(TRIPLETS, 2, 6), TRIPLETS, "2/6 is 1/3");
  assert.equal(nextGroup(TRIPLETS, 2, 4), COMMON, "2/4 is 1/2");
  assert.equal(nextGroup(TRIPLETS, 4, 1), TRIPLETS, "a snap longer than a beat keeps the group");
  assert.equal(nextGroup(null, 4, 1), COMMON);
  assert.equal(nextGroup(null, 3, 4), COMMON, "3/4 is no 1/x; a new map starts in Common");
});

// BindableBeatDivisor.cs:71-83; Ctrl+wheel up is next (GlobalActionContainer.cs:158-159).
test("the wheel steps through the group only and stops at either end", () => {
  const step = (group, divisor, finer) => stepInGroup(group, 1, divisor, finer);
  assert.equal(step(TRIPLETS, 3, true), 6);
  assert.equal(step(TRIPLETS, 6, true), 12);
  assert.equal(step(TRIPLETS, 12, true), null, "no wrap past 1/12");
  assert.equal(step(TRIPLETS, 3, false), 1);
  assert.equal(step(TRIPLETS, 1, false), null, "no wrap below 1/1");
  assert.equal(step(TRIPLETS, 1, true), 3);
  assert.equal(step(COMMON, 4, true), 8);
  assert.equal(step(COMMON, 4, false), 2);
  assert.equal(step(COMMON, 16, true), null);
  const custom = nextGroup(COMMON, 1, 5);
  assert.equal(step(custom, 5, false), 1);
  assert.equal(step(custom, 1, true), 5);
  assert.equal(step(custom, 5, true), null);
  assert.equal(stepInGroup(COMMON, 4, 1, true), 1, "4/1 steps finer to 1/1");
  assert.equal(stepInGroup(COMMON, 4, 1, false), null);
  assert.equal(stepInGroup(TRIPLETS, 2, 6, true), 6, "2/6 is 1/3");
  assert.equal(stepInGroup(COMMON, 3, 4, true), 2, "3/4 steps to the next finer snap in the group");
});

// BindableBeatDivisor.cs:104-140, hexes from OsuColour.cs.
test("each divisor has lazer's colour", () => {
  const colours = {
    1: "#ffffff", 2: "#ed1121", 3: "#8866ee", 4: "#66ccff", 5: "#b3d944", 6: "#eeaa00",
    7: "#b3d944", 8: "#ffcc22", 9: "#b3d944", 12: "#cc6600", 16: "#6644cc",
  };
  for (const [divisor, colour] of Object.entries(colours)) assert.equal(divisorColour(Number(divisor)), colour, divisor);
  for (const divisor of [10, 11, 24, 32, 64]) assert.equal(divisorColour(divisor), "#ff0000", String(divisor));
  assert.equal(snapColour(1, 6), "#eeaa00");
  assert.equal(snapColour(2, 4), "#ed1121", "2/4 takes the colour of 1/2");
  assert.equal(snapColour(4, 1), "#ffffff", "4/1 takes the colour of 1/1");
  assert.equal(snapColour(3, 4), "#66ccff", "3/4 takes the colour of its divisor");
});

function shown(scene) {
  bridge.set({ busy: false }, null);
  bridge.set({ busy: false }, scene);
  return render(SnapSlider).body;
}
const scene = fields => ({ path: "a.osu", snap: true, snapMode: "grid", beats: 1, divisor: 4, ...fields });
const ticks = html => [...html.matchAll(/<span class="tick([^"]*)"[^>]*style="--tick: (#[\da-f]{6});?"/g)]
  .map(([, classes, colour]) => ({ colour, dim: /\bdim\b/.test(classes) }));

test("the stops wear their colours and the other group's stops are dimmed", () => {
  let html = shown(scene({}));
  assert.deepEqual(ticks(html).map(tick => tick.colour),
    ["#ffffff", "#ed1121", "#8866ee", "#66ccff", "#eeaa00", "#ffcc22", "#cc6600", "#6644cc"]);
  assert.deepEqual(ticks(html).map(tick => tick.dim), [false, false, true, false, true, false, true, false]);
  assert.match(html, /class="snap[^"]*"[^>]*style="--snap: #66ccff;?"/);

  html = shown(scene({ divisor: 6 }));
  assert.deepEqual(ticks(html).map(tick => tick.dim), [false, true, false, true, false, true, false, true]);
  assert.match(html, /class="snap[^"]*"[^>]*style="--snap: #eeaa00;?"/);

  html = shown(scene({ divisor: 5 }));
  assert.deepEqual(ticks(html).map(tick => tick.dim), [false, true, true, true, true, true, true, true],
    "1/5 makes the group {1, 5}: only 1/1 has a stop");
  assert.match(html, /class="snap[^"]*"[^>]*style="--snap: #b3d944;?"/);
});

test("the bridge keeps the group a pick chose, and a new map starts again from its own snap", () => {
  bridge.set({ busy: false }, null);
  bridge.set({ busy: false }, scene({ divisor: 3 }));
  bridge.set({ busy: false }, scene({ divisor: 1 }));
  assert.equal(bridge.snapGroup, TRIPLETS, "1/1 after 1/3 stays in Triplets");
  bridge.set({ busy: false }, scene({ path: "b.osu", divisor: 1 }));
  assert.equal(bridge.snapGroup, COMMON, "another map opens in Common");
  bridge.set({ busy: false }, scene({ path: "b.osu", divisor: 6 }));
  bridge.set({ busy: false }, null);
  bridge.set({ busy: false }, scene({ path: "b.osu", divisor: 1 }));
  assert.equal(bridge.snapGroup, COMMON, "closing the map forgets the group");
});
