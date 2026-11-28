import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  AFTER_HIT, SPINNER_FADE, approach, ball, clock, fit, followPointRange, opacity, pathTo, pointAt, reverseArrow,
  sliderProgress, snake, visible,
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

test("while playing the clock holds still on a small step back and stops at the song's end", () => {
  const view = { position: 1000, playing: true, speed: 1, duration: 5000 };
  assert.equal(clock(view, 0, 0, 1020), 1020);
  assert.equal(clock(view, 0, 0, 1200), 1000);
  assert.equal(clock({ ...view, position: 4990 }, 0, 100), 5000);
  assert.equal(clock({ ...view, playing: false }, 0, 0, 1020), 1000);
});

test("the preview asks for the objects once per difficulty, not again when the answer arrives", async () => {
  // Effects do not run under svelte/server, so this reads the source of the fetch effect: it may
  // set `model` (its own answer does) but read it only inside untrack, or it asks again forever.
  const { readFileSync } = await import("node:fs");
  const source = readFileSync(new URL("../src/parts/PreviewCanvas.svelte", import.meta.url), "utf8");
  const call = source.indexOf('invoke("map_preview_objects"');
  const effect = source.slice(source.lastIndexOf("$effect(() => {", call), source.indexOf("});", call));
  assert.ok(effect.includes("untrack(() => model?.path)"));
  const reads = effect
    .replace(/untrack\(\(\) => [^)]*\)/g, "")
    .replace(/\/\/.*$/gm, "")
    .replace(/\bmodel = /g, "")
    .match(/\bmodel\b/g);
  assert.equal(reads, null);
});

test("ticks show partway through their span and are gone once passed", async () => {
  const { ticksShown } = await import("../src/lib/preview.js");
  const model = { preempt: 1200, fadeIn: 400 };
  // A 200 px slider from 2000 to 3000 with a tick at 2500 (half way).
  const slider = { kind: "slider", time: 2000, end: 3000, x: 0, y: 0, path: [0, 0, 200, 0], lengths: [0, 200], spans: 1, ticks: [2500] };
  // First span: shows at 2500 − (250 + 0.66 × 1200) = 1458.
  assert.deepEqual(ticksShown(model, slider, 1457), []);
  const [tick] = ticksShown(model, slider, 1460);
  assert.equal(tick.x, 100);
  assert.ok(Math.abs(tick.since - 2) < 1e-9);
  assert.deepEqual(ticksShown(model, slider, 2500), []);
});

test("reverse arrows show once the body has snaked in, the later ones two spans ahead", async () => {
  const { arrowsShown } = await import("../src/lib/preview.js");
  const model = { preempt: 1200, fadeIn: 400 };
  // Three spans of 500 ms: turns at 2500 (at the end) and 3000 (back at the start).
  const slider = { kind: "slider", time: 2000, end: 3500, x: 0, y: 0, path: [0, 0, 200, 0], lengths: [0, 200], spans: 3 };
  assert.deepEqual(arrowsShown(model, slider, 1199), []);
  const first = arrowsShown(model, slider, 1200);
  assert.equal(first.length, 1);
  assert.equal(first[0].x, 200);
  assert.ok(Math.abs(Math.abs(first[0].angle) - Math.PI) < 1e-9);
  // The second turn shows two spans ahead, at 2000.
  assert.equal(arrowsShown(model, slider, 2000).length, 2);
  assert.equal(arrowsShown(model, slider, 2600)[0].x, 0);
  assert.deepEqual(arrowsShown(model, slider, 3000), []);
});

test("passed pieces fade over the editor's 700 ms and bodies over 240 ms after their end", async () => {
  const { bodyOpacity, pieceOpacity } = await import("../src/lib/preview.js");
  const model = { preempt: 1200, fadeIn: 400 };
  const slider = { kind: "slider", time: 2000, end: 3000 };
  assert.equal(pieceOpacity(model, slider, 1000), 0.5);
  assert.equal(pieceOpacity(model, slider, 2350), 0.5);
  assert.equal(bodyOpacity(model, slider, 2900), 1);
  assert.equal(bodyOpacity(model, slider, 3120), 0.5);
});

test("the panel offers its own window, and says where the preview is while that is open", () => {
  bridge.mapPreview = true;
  assert.match(render(MapPreview).body, /Open in its own window/);
  bridge.previewWindow = true;
  const body = render(MapPreview).body;
  assert.match(body, /The map preview is in its own window/);
  assert.match(body, /Show it here/);
  assert.doesNotMatch(body, /<canvas/);
  bridge.previewWindow = false;
  bridge.mapPreview = false;
});

test("the spinner disc is nothing until half the approach, a dot at its time, then grows", async () => {
  const { spinnerGrowth } = await import("../src/lib/argon.js");
  assert.equal(spinnerGrowth(599, 1200), 0);
  assert.ok(Math.abs(spinnerGrowth(1200 - 1e-9, 1200) - 0.2) < 1e-6);
  assert.ok(spinnerGrowth(1200, 1200) - 0.2 < 1e-9);
  assert.equal(spinnerGrowth(1800, 1200), 1);
});

function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function linearVisible(map, time) {
  if (!map?.objects) return [];
  const fadeEnd = obj => obj.end + (obj.kind === "spinner" ? SPINNER_FADE : AFTER_HIT);
  return map.objects.filter(obj => time >= obj.time - map.preempt && time <= fadeEnd(obj)).reverse();
}

function linearPointAt(object, distance) {
  const path = object.path ?? [];
  const lengths = object.lengths ?? [];
  const count = path.length / 2;
  if (!count) return { x: object.x, y: object.y };
  let index = lengths.findIndex(length => length >= distance);
  if (index < 0) index = count;
  if (index <= 0) return { x: path[0], y: path[1] };
  if (index >= count) return { x: path[2 * count - 2], y: path[2 * count - 1] };
  const [d0, d1] = [lengths[index - 1], lengths[index]];
  const [x0, y0, x1, y1] = [path[2 * index - 2], path[2 * index - 1], path[2 * index], path[2 * index + 1]];
  if (Math.abs(d1 - d0) < 1e-3) return { x: x0, y: y0 };
  const w = (distance - d0) / (d1 - d0);
  return { x: x0 + (x1 - x0) * w, y: y0 + (y1 - y0) * w };
}

function linearPathTo(object, distance) {
  const path = object.path ?? [];
  const lengths = object.lengths ?? [];
  const points = [];
  for (let i = 0; i < path.length / 2 && lengths[i] <= distance; i++) points.push({ x: path[2 * i], y: path[2 * i + 1] });
  points.push(linearPointAt(object, distance));
  return points;
}

test("visible matches the linear filter across long sliders, spinners, boundaries, and random times", () => {
  const complexModel = Object.freeze({
    radius: 36.5,
    preempt: 1200,
    fadeIn: 400,
    colours: ["#ff0000"],
    objects: [
      { kind: "circle", time: 1000, end: 1000, x: 10, y: 10, combo: 1, colour: 0 },
      // Long slider starting early and outlasting several later objects
      { kind: "slider", time: 1200, end: 12000, x: 50, y: 50, combo: 2, colour: 0, path: [50, 50, 300, 50], lengths: [0, 250], spans: 1 },
      { kind: "circle", time: 2000, end: 2000, x: 80, y: 80, combo: 3, colour: 0 },
      // Long spinner starting before later circles, with shorter 240 ms fadeout
      { kind: "spinner", time: 3000, end: 8000, x: 256, y: 192, combo: 1, colour: 0 },
      { kind: "circle", time: 4000, end: 4000, x: 120, y: 120, combo: 1, colour: 0 },
      { kind: "slider", time: 8100, end: 8500, x: 150, y: 150, combo: 2, colour: 0, path: [150, 150, 250, 150], lengths: [0, 100], spans: 1 },
      { kind: "circle", time: 12500, end: 12500, x: 200, y: 200, combo: 3, colour: 0 },
      { kind: "spinner", time: 15000, end: 16000, x: 256, y: 192, combo: 1, colour: 0 },
    ],
  });

  const boundaryTimes = [
    -500, -200, -199, 0, 1000, 1700, 1701, 2700, 2701,
    8239, 8240, 8241, 10000, 12699, 12700, 12701, 16240, 16241, NaN, Infinity, -Infinity,
  ];
  for (const t of boundaryTimes) {
    assert.deepEqual(visible(complexModel, t), linearVisible(complexModel, t), `boundary time ${t}`);
  }

  const rng = mulberry32(0x5eed1234);
  for (let i = 0; i < 400; i++) {
    const t = -500 + rng() * 18000;
    assert.deepEqual(visible(complexModel, t), linearVisible(complexModel, t), `random time ${t}`);
  }

  assert.deepEqual(visible({ preempt: 1200, objects: [] }, 1000), []);
  assert.deepEqual(visible(null, 1000), []);
});

test("pointAt and pathTo match linear search across 0, end, duplicate lengths, and random distances", () => {
  const dupSlider = {
    kind: "slider",
    time: 1000,
    end: 2000,
    x: 10,
    y: 20,
    path: [10, 20, 10, 20, 60, 20, 60, 20, 60, 20.0005, 130, 90, 130, 90],
    lengths: [0, 0, 50, 50, 50.0005, 148.99, 148.99],
  };
  const emptySlider = { kind: "slider", time: 1000, end: 2000, x: 42, y: 84, path: [], lengths: [] };

  const edgeDistances = [-10, 0, 1e-6, 25, 50, 50.0002, 50.0005, 100, 148.99, 200, NaN];
  for (const d of edgeDistances) {
    assert.deepEqual(pointAt(dupSlider, d), linearPointAt(dupSlider, d), `pointAt edge ${d}`);
    assert.deepEqual(pathTo(dupSlider, d), linearPathTo(dupSlider, d), `pathTo edge ${d}`);
    assert.deepEqual(pointAt(emptySlider, d), linearPointAt(emptySlider, d), `empty pointAt ${d}`);
    assert.deepEqual(pathTo(emptySlider, d), linearPathTo(emptySlider, d), `empty pathTo ${d}`);
  }

  const rng = mulberry32(0xbeef9876);
  for (let i = 0; i < 300; i++) {
    const d = -20 + rng() * 200;
    assert.deepEqual(pointAt(dupSlider, d), linearPointAt(dupSlider, d), `random pointAt ${d}`);
    assert.deepEqual(pathTo(dupSlider, d), linearPathTo(dupSlider, d), `random pathTo ${d}`);
  }
});

test("followPointRange includes every active consecutive pair across long objects and random times", () => {
  const objs = [
    { kind: "circle", time: 500, end: 500, combo: 1 },
    { kind: "slider", time: 1000, end: 9000, combo: 2 },
    { kind: "circle", time: 9400, end: 9400, combo: 3 },
    { kind: "circle", time: 9800, end: 9800, combo: 4 },
    { kind: "spinner", time: 10500, end: 13000, combo: 1 },
    { kind: "circle", time: 13500, end: 13500, combo: 1 },
    { kind: "circle", time: 13900, end: 13900, combo: 2 },
  ];
  const fpPreempt = 800;
  const activePairsLinear = t => {
    const out = [];
    for (let i = 0; i < objs.length - 1; i++) {
      const startTime = objs[i].end ?? objs[i].time;
      const endTime = objs[i + 1].time;
      if (t >= startTime - fpPreempt && t <= endTime + 150) out.push(i);
    }
    return out;
  };
  const activePairsBsearch = t => {
    const { start, end } = followPointRange(objs, t, fpPreempt);
    const out = [];
    for (let i = start; i < end; i++) {
      const startTime = objs[i].end ?? objs[i].time;
      const endTime = objs[i + 1].time;
      if (t >= startTime - fpPreempt && t <= endTime + 150) out.push(i);
    }
    return out;
  };

  const rng = mulberry32(0xcafe1111);
  for (let i = 0; i < 300; i++) {
    const t = -500 + rng() * 16000;
    assert.deepEqual(activePairsBsearch(t), activePairsLinear(t), `followPointRange time ${t}`);
  }
  assert.deepEqual(activePairsBsearch(NaN), []);
});

test("the map preview's Skin picker gets a full-width row so a skin name like 'Argon (osu!lazer)' shows whole", async () => {
  // Label, picker and Browse... shared one 210 px row and the picker showed "Argon (osu!l...".
  const { readFileSync } = await import("node:fs");
  const css = readFileSync(new URL("../src/parts/MapPreview.svelte", import.meta.url), "utf8").split("<style>")[1];
  const rule = selector => css.match(new RegExp(selector.replace(/[.]/g, "\.") + String.raw`\s*\{([^}]*)\}`))?.[1] ?? "";
  assert.match(rule(".skin-row"), /display:\s*grid;/);
  assert.match(rule(".skin-select"), /grid-column:\s*1 \/ -1;/);
});
