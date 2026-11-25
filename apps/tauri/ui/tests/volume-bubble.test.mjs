import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  formatVolumeTime,
  parseVolumeInput,
  placeVolumeBubble,
  volumeBubbleAtMoment,
} from "../src/lib/volumeBubble.js";
import BaseDiffStrip from "../src/parts/BaseDiffStrip.svelte";
import VolumeBubble from "../src/parts/VolumeBubble.svelte";

test("parseVolumeInput parses percentages and clamps to osu!'s 5..100 range", () => {
  assert.equal(parseVolumeInput("65"), 65);
  assert.equal(parseVolumeInput(" 82% "), 82);
  assert.equal(parseVolumeInput("2"), 5);
  assert.equal(parseVolumeInput("150"), 100);
  assert.equal(parseVolumeInput("49.6"), 50);
  assert.equal(parseVolumeInput(""), null);
  assert.equal(parseVolumeInput("abc"), null);
});

test("placeVolumeBubble sits above by default, flips below near top, and clamps horizontally", () => {
  assert.deepEqual(placeVolumeBubble({ barX: 300, barTopY: 400, viewportWidth: 1200 }), {
    x: 300,
    y: 390,
    below: false,
  });
  assert.deepEqual(placeVolumeBubble({ barX: 20, barTopY: 30, viewportWidth: 1200 }), {
    x: 64,
    y: 44,
    below: true,
  });
  assert.equal(formatVolumeTime(72500), "1:12.500");
});

test("volumeBubbleAtMoment builds a positioned bubble for the nearest moment", () => {
  const scene = {
    viewStart: 1000,
    zoom: 0.2,
    moments: [
      [1500, 75, true, [[0, false]]],
      [2000, 100, true, [[0, false]]],
    ],
  };
  const rect = { left: 200, top: 500, bottom: 600, width: 800, height: 100 };
  const hover = volumeBubbleAtMoment(scene, 1502, rect);
  assert.equal(hover.ms, 1500);
  assert.equal(hover.volume, 75);
  assert.equal(hover.x, 300);
  assert.equal(hover.editing, false);

  const edit = volumeBubbleAtMoment(scene, 2000, rect, { editing: true });
  assert.equal(edit.editing, true);
  assert.equal(edit.volume, 100);
  assert.equal(volumeBubbleAtMoment(scene, 9999, rect), null);
});

test("VolumeBubble renders hover readout and inline numeric input in editing mode", () => {
  bridge.volumeBubble = {
    key: "1500:75:hover",
    ms: 1500,
    volume: 75,
    timeLabel: "0:01.500",
    x: 300,
    y: 420,
    below: false,
    editing: false,
  };
  const hoverHtml = render(VolumeBubble).body;
  assert.match(hoverHtml, /75%/);
  assert.match(hoverHtml, /0:01\.500/);

  bridge.volumeBubble = { ...bridge.volumeBubble, editing: true };
  const editHtml = render(VolumeBubble).body;
  assert.match(editHtml, /type="number"/);
  assert.match(editHtml, /aria-label="Hit volume percent"/);
  bridge.volumeBubble = null;
});

test("BaseDiffStrip renders Base tab and every difficulty tab on the main timeline", () => {
  bridge.set(
    { busy: false, fault: null },
    {
      path: "E:/Songs/Song/Artist - Song (Mapper) [Hard].osu",
      difficulties: [
        "E:/Songs/Song/Artist - Song (Mapper) [Normal].osu",
        "E:/Songs/Song/Artist - Song (Mapper) [Hard].osu",
        "E:/Songs/Song/Artist - Song (Mapper) [Insane].osu",
      ],
      hsDifficulties: [],
    },
  );
  const html = render(BaseDiffStrip).body;
  assert.match(html, /role="tablist"/);
  assert.match(html, /Base/);
  assert.match(html, /Normal/);
  assert.match(html, /Hard/);
  assert.match(html, /Insane/);
  assert.match(html, /Base Studio…/);
  assert.match(html, /Capture/);
});
