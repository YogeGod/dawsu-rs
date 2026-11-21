import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { clampSnapPart, snapPopoverAction, stepLaneHeight, tripletSnapAction } from "../src/lib/popovers.js";
import GridOptions from "../src/parts/GridOptions.svelte";
import LaneOptions from "../src/parts/LaneOptions.svelte";

const map = { path: "Hard.osu", snap: true, snapMode: "grid", beats: 1, divisor: 4, rowHeight: 30 };

test("popover helpers clamp snap inputs and build snap and lane-height actions", () => {
  assert.equal(clampSnapPart("0", 4), 1);
  assert.equal(clampSnapPart("99", 4), 64);
  assert.equal(clampSnapPart("", 4), 4);
  assert.equal(clampSnapPart("6", 4), 6);
  assert.deepEqual(snapPopoverAction(map, { mode: "relative", divisor: "8" }), {
    kind: "snap", on: true, mode: "relative", beats: 1, divisor: 8,
  });
  assert.deepEqual(tripletSnapAction(map, true), {
    kind: "snap", on: true, mode: "grid", beats: 1, divisor: 6,
  });
  assert.deepEqual(tripletSnapAction(map, false), {
    kind: "snap", on: true, mode: "grid", beats: 1, divisor: 4,
  });
  assert.equal(stepLaneHeight(30, 1.15), 35);
  assert.equal(stepLaneHeight(35, 1 / 1.15), 30);
});

test("GridOptions renders all More Snaps controls and reflects Triplets group and disabled states", () => {
  bridge.set({ busy: false, objectLines: true }, map);
  let html = render(GridOptions).body;
  for (const id of ["object-lines", "snap", "snap-mode", "beats", "divisor", "triplet-snap", "time-signatures"]) {
    assert.match(html, new RegExp('id="' + id + '"'), id);
  }
  assert.match(html, /id="object-lines"[^>]*checked/);
  assert.match(html, /id="snap"[^>]*checked/);
  assert.doesNotMatch(html, /id="triplet-snap"[^>]*checked/);
  assert.doesNotMatch(html, /\sstyle=|data-doc/);

  bridge.set({ busy: false, objectLines: false }, { ...map, divisor: 3 });
  html = render(GridOptions).body;
  assert.match(html, /id="triplet-snap"[^>]*checked/);

  bridge.set({ busy: true, objectLines: false }, map);
  html = render(GridOptions).body;
  assert.match(html, /id="triplet-snap"[^>]*disabled/);
  assert.match(html, /id="time-signatures"[^>]*disabled/);
});

test("LaneOptions renders lane zoom, collapse, used-only and filter controls", () => {
  bridge.set({ busy: false }, { ...map, rowHeight: 42 });
  bridge.setLaneOptions(true, "snare");
  let html = render(LaneOptions).body;
  for (const id of ["lane-height-out", "lane-height", "lane-height-in", "collapse-layers", "used-only", "lane-filter"]) {
    assert.match(html, new RegExp('id="' + id + '"'), id);
  }
  assert.match(html, /id="lane-height"[^>]*value="42"/);
  assert.match(html, /id="used-only"[^>]*checked/);
  assert.match(html, /id="lane-filter"[^>]*value="snare"/);
  assert.doesNotMatch(html, /\sstyle=|data-doc/);

  bridge.setLaneOptions(false, "");
  bridge.set({ busy: true }, map);
  html = render(LaneOptions).body;
  assert.match(html, /id="collapse-layers"[^>]*disabled/);
  assert.match(html, /id="lane-filter"[^>]*disabled/);
});
