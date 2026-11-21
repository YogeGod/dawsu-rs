import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  AFTER_HIT, approach, ball, clock, fit, opacity, pathTo, pointAt, reverseArrow, sliderProgress, snake, visible,
} from "../src/lib/preview.js";
import MapPreview from "../src/parts/MapPreview.svelte";

// AR 5 (preempt 1200, fade-in 400) and a 100 px slider from 100,100 to 200,100 over 500 ms.
const circle = { kind: "circle", time: 2000, end: 2000, x: 50, y: 60, combo: 1, colour: 0 };
const slider = { kind: "slider", time: 3000, end: 3500, x: 100, y: 100, combo: 2, colour: 1,
  path: [100, 100, 200, 100], lengths: [0, 100], spans: 1 };
const twoSpans = { ...slider, end: 4000, spans: 2 };
const spinner = { kind: "spinner", time: 5000, end: 6000, x: 256, y: 192, combo: 1, colour: 0 };
const model = { radius: 36.5, preempt: 1200, fadeIn: 400, colours: ["#ff0000", "#00ff00"], objects: [circle, slider, spinner] };

test("the playfield fits a 640 × 480 screen inside the box, centred", () => {
  assert.deepEqual(fit(640, 480), { scale: 1, x: 64, y: 48 });
  const wide = fit(1280, 480);
  assert.equal(wide.scale, 1);
  assert.equal(wide.x, (1280 - 512) / 2);
});

test("the clock runs on from the editor's position only while playing, at the playback speed", () => {
  assert.equal(clock({ position: 1000, playing: false }, 0, 500), 1000);
  assert.equal(clock({ position: 1000, playing: true, speed: 1 }, 100, 150), 1050);
  assert.equal(clock({ position: 1000, playing: true, speed: 0.5 }, 100, 300), 1100);
  assert.equal(clock(null, 0, 0), 0);
});

test("an object shows from preempt before its time until it has faded after its end, latest drawn first", () => {
  assert.deepEqual(visible(model, 799), []);
  assert.deepEqual(visible(model, 800), [circle]);
  assert.deepEqual(visible(model, 2000 + AFTER_HIT), [slider, circle]);
  assert.deepEqual(visible(model, 2000 + AFTER_HIT + 1), [slider]);
  assert.deepEqual(visible(model, 6240), [spinner]);
  assert.deepEqual(visible(model, 6241), []);
});

test("objects fade in over the fade-in time and out after their end", () => {
  assert.equal(opacity(model, circle, 800), 0);
  assert.equal(opacity(model, circle, 1000), 0.5);
  assert.equal(opacity(model, circle, 1500), 1);
  assert.equal(opacity(model, circle, 2350), 0.5);
  assert.equal(opacity(model, spinner, 6120), 0.5);
});

test("the approach circle shrinks from 4× to 1× over the preempt and reaches 0.9 opacity", () => {
  assert.equal(approach(model, circle, 799), null);
  assert.deepEqual(approach(model, circle, 800), { scale: 4, alpha: 0 });
  const half = approach(model, circle, 1400);
  assert.equal(half.scale, 2.5);
  assert.equal(half.alpha, 0.9 * 600 / 800);
  assert.equal(approach(model, circle, 1999).alpha, 0.9);
  assert.equal(approach(model, circle, 2000), null);
  assert.equal(approach(model, spinner, 4500), null);
});

test("a slider snakes in over the first third of the preempt", () => {
  assert.equal(snake(model, slider, 1800), 0);
  assert.equal(snake(model, slider, 2000), 0.5);
  assert.equal(snake(model, slider, 2200), 1);
});

test("points along a path follow its cumulative lengths", () => {
  assert.deepEqual(pointAt(slider, 0), { x: 100, y: 100 });
  assert.deepEqual(pointAt(slider, 25), { x: 125, y: 100 });
  assert.deepEqual(pointAt(slider, 150), { x: 200, y: 100 });
  assert.deepEqual(pathTo(slider, 50), [{ x: 100, y: 100 }, { x: 150, y: 100 }]);
});

test("the ball goes there and back once per span", () => {
  assert.deepEqual(sliderProgress(twoSpans, 3250), { span: 0, along: 0.5 });
  assert.deepEqual(sliderProgress(twoSpans, 3750), { span: 1, along: 0.5 });
  assert.equal(sliderProgress(twoSpans, 4000).along, 0);
  assert.equal(sliderProgress(slider, 3500).along, 1);
  assert.deepEqual(ball(slider, 3250), { x: 150, y: 100 });
  assert.equal(ball(slider, 2999), null);
});

test("a reverse arrow sits on the end that turns the ball back, pointing back along the path", () => {
  assert.equal(reverseArrow(slider, 3200), null);
  const arrow = reverseArrow(twoSpans, 3200);
  assert.equal(arrow.x, 200);
  assert.equal(arrow.y, 100);
  assert.equal(arrow.angle, Math.PI);
  assert.equal(reverseArrow(twoSpans, 3600), null);
});

test("the preview shows only while View › Map preview is on, with a note before a map is open", () => {
  bridge.mapPreview = false;
  bridge.scene = null;
  assert.equal(render(MapPreview).body.replace(/<!--[^>]*-->/g, "").trim(), "");
  bridge.mapPreview = true;
  const body = render(MapPreview).body;
  assert.match(body, /Map preview/);
  assert.match(body, /Open a difficulty to see its preview/);
  bridge.mapPreview = false;
});
