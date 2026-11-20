import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { nearestStop, onStop, snapAction, stepStop } from "../src/lib/snap.js";
import SnapSlider from "../src/parts/SnapSlider.svelte";

const STOPS = [1, 2, 3, 4, 6, 8, 12, 16];

test("a snap the slider has no stop for sits at the nearest stop, off the stop", () => {
  const cases = [[4, 1, 1], [2, 1, 1], [3, 1, 1], [1, 5, 6], [1, 64, 16]];
  for (const [beats, divisor, stop] of cases) {
    assert.equal(STOPS[nearestStop(beats, divisor)], stop, beats + "/" + divisor);
    assert.equal(onStop(beats, divisor), false, beats + "/" + divisor);
  }
  for (const stop of STOPS) assert.equal(STOPS[nearestStop(1, stop)], stop);
  assert.ok(onStop(1, 4));
  assert.ok(onStop(2, 4), "2/4 is the same length as 1/2");
  assert.equal(STOPS[nearestStop(2, 4)], 2);
});

test("moving the slider keeps snap on or off and its mode, and always sends one beat", () => {
  assert.deepEqual(snapAction({ snap: false, snapMode: "relative", beats: 3, divisor: 1 }, 5),
    { kind: "snap", on: false, mode: "relative", beats: 1, divisor: 8 });
  assert.deepEqual(snapAction({ snap: true, snapMode: "grid", beats: 1, divisor: 4 }, 0),
    { kind: "snap", on: true, mode: "grid", beats: 1, divisor: 1 });
});

test("arrow keys step to the next finer or coarser stop, Home and End to the ends", () => {
  const step = (beats, divisor, key) => STOPS[stepStop(beats, divisor, key)];
  assert.equal(step(1, 4, "ArrowRight"), 6);
  assert.equal(step(1, 4, "ArrowUp"), 6);
  assert.equal(step(1, 4, "ArrowLeft"), 3);
  assert.equal(step(1, 4, "ArrowDown"), 3);
  assert.equal(step(1, 5, "ArrowRight"), 6);
  assert.equal(step(1, 5, "ArrowLeft"), 4);
  assert.equal(step(4, 1, "ArrowRight"), 1, "4/1 is coarser than every stop");
  assert.equal(step(4, 1, "ArrowLeft"), 1);
  assert.equal(step(1, 64, "ArrowRight"), 16);
  assert.equal(step(1, 64, "ArrowLeft"), 16, "1/64 is finer than every stop");
  assert.equal(step(1, 16, "ArrowRight"), 16);
  assert.equal(step(1, 4, "Home"), 1);
  assert.equal(step(1, 4, "End"), 16);
  assert.equal(stepStop(1, 4, "PageUp"), null);
});

function shown(scene, view = { busy: false }) {
  bridge.set(view, scene);
  return render(SnapSlider).body;
}
const scene = fields => ({ snap: true, snapMode: "grid", beats: 1, divisor: 4, ...fields });
const lit = html => [...html.matchAll(/class="tick[^"]*\blit\b/g)].length;

test("the slider shows the real snap in its chip and the nearest stop on its track", () => {
  let html = shown(scene({}));
  assert.match(html, /role="slider"/);
  assert.match(html, /aria-valuemin="1" aria-valuemax="16" aria-valuenow="4" aria-valuetext="1\/4"/);
  assert.match(html, /class="chip[^"]*">1\/4</);
  assert.equal(lit(html), 4);
  assert.doesNotMatch(html, /\bbetween\b|\boff\b|aria-disabled="true"/);

  html = shown(scene({ divisor: 5 }));
  assert.match(html, /aria-valuenow="6" aria-valuetext="1\/5"/);
  assert.match(html, /class="chip[^"]*">1\/5</);
  assert.match(html, /\bbetween\b/);
  assert.equal(lit(html), 5);

  html = shown(scene({ beats: 4, divisor: 1 }));
  assert.match(html, /aria-valuenow="1" aria-valuetext="4\/1"/);
  assert.equal(lit(html), 1);
});

test("with snap off the slider is dimmed but still works; with no map or while busy it is disabled", () => {
  let html = shown(scene({ snap: false }));
  assert.match(html, /\boff\b/);
  assert.match(html, /aria-valuetext="1\/4, snap off"/);
  assert.doesNotMatch(html, /aria-disabled="true"/);

  html = shown(null);
  assert.match(html, /aria-disabled="true"/);
  assert.match(html, /class="chip[^"]*">–</);
  html = shown(scene({}), { busy: true });
  assert.match(html, /aria-disabled="true"/);
});
