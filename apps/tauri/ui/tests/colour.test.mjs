import assert from "node:assert/strict";
import { test } from "node:test";
import {
  LANE_COLOUR_PALETTE, cycleLaneColour, folderGrey, folderShades, laneGrey, laneShades, mixIntoCanvas, oklchToHex,
} from "../src/lib/colour.js";

// Checked in the editor's WebView2 (2026-11-20): getComputedStyle of color-mix(in oklch, <colour> p%, #151515)
// and of oklch(), turned into hexes. Within 1 per channel.
const near = (actual, expected) => {
  const channels = hex => [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
  const [a, b] = [channels(actual), channels(expected)];
  assert.ok(a.every((value, i) => Math.abs(value - b[i]) <= 1), actual + " vs " + expected);
};

test("grey lanes and folders are v3's lightness with no chroma", () => {
  near(oklchToHex(0.70, 0, 0), "#9e9e9e");
  near(oklchToHex(0.55, 0, 0), "#717171");
  assert.equal(laneGrey, oklchToHex(0.70, 0, 0));
  assert.equal(folderGrey, oklchToHex(0.55, 0, 0));
});

test("a picked colour mixes into the canvas keeping its hue, like color-mix in oklch", () => {
  assert.equal(mixIntoCanvas("#d04932", 100), "#d04932");
  assert.equal(mixIntoCanvas("#d04932", 0), "#151515");
  near(mixIntoCanvas("#d04932", 18), "#33201c");
  near(mixIntoCanvas("#d04932", 36), "#532a22");
  near(mixIntoCanvas("#388ada", 26), "#213142");
  near(mixIntoCanvas("#9e9e9e", 50), "#545454");
  near(mixIntoCanvas("#717171", 34), "#323232");
});

test("lane and folder shades: strip, tile, header, hover and selected", () => {
  const red = laneShades("#d04932");
  assert.equal(red.strip, "#d04932");
  assert.equal(red.tile, mixIntoCanvas("#d04932", 26));
  assert.equal(red.header, mixIntoCanvas("#d04932", 18));
  assert.equal(red.hover, mixIntoCanvas("#d04932", 28));
  assert.equal(red.selected, mixIntoCanvas("#d04932", 36));
  assert.equal(laneShades(null).strip, laneGrey);
  assert.equal(laneShades("not a colour").strip, laneGrey);
  assert.deepEqual(folderShades(), { strip: folderGrey, header: mixIntoCanvas(folderGrey, 34) });
});

test("LANE_COLOUR_PALETTE matches the 72-swatch palette and cycleLaneColour wraps through null", () => {
  assert.equal(LANE_COLOUR_PALETTE.length, 72);
  assert.equal(LANE_COLOUR_PALETTE[0], "#d04932");
  assert.equal(LANE_COLOUR_PALETTE.at(-1), "#72385e");
  for (const hex of LANE_COLOUR_PALETTE) {
    assert.match(hex, /^#[\da-f]{6}$/);
  }
  assert.equal(cycleLaneColour(null, 1), "#d04932");
  assert.equal(cycleLaneColour(null, -1), "#72385e");
  assert.equal(cycleLaneColour("#d04932", 1), "#db642b");
  assert.equal(cycleLaneColour("#d04932", -1), null);
  assert.equal(cycleLaneColour("#72385e", 1), null);
  assert.equal(cycleLaneColour("#D04932", 1), "#db642b");
  assert.equal(cycleLaneColour("#123456", 1), "#d04932");
  assert.equal(cycleLaneColour("#123456", -1), "#72385e");
  assert.equal(cycleLaneColour(null, 0), null);
  assert.equal(cycleLaneColour("#D04932", 0), "#d04932");
});
