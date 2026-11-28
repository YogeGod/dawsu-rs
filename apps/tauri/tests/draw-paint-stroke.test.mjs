import assert from "node:assert/strict";
import { test } from "node:test";
import { createContext } from "node:vm";
import {
  computeDrawStrokeSteps,
  MAX_DRAW_STROKE_STEPS,
  snapTimeInScene,
} from "../ui/src/editor/helpers.js";
import { bootEditor } from "./editor-runtime.mjs";

function sampleScene(overrides = {}) {
  return {
    path: "Test.osu",
    viewStart: 0,
    zoom: 1,
    rowHeight: 30,
    snap: true,
    snapMode: "grid",
    beats: 1,
    divisor: 4,
    insertVolume: 70,
    previewKey: 1,
    difficulties: ["Test.osu"],
    waveform: [],
    history: [],
    laneOrder: [0, 1],
    // 120 BPM => 500 ms beat => 125 ms 1/4 beat step.
    sections: [[0, 500, 4]],
    moments: [
      [1000, 100, true, [[0, false, false, 100]]],
      [1250, 90, false, [[1, false, false, 90]]],
      [1500, 100, true, [[0, false, false, 100]]],
    ],
    lanes: [
      { key: "normal-hitnormal", name: "normal-hitnormal" },
      { key: "normal-hitclap", name: "normal-hitclap" },
    ],
    ...overrides,
  };
}

test("computeDrawStrokeSteps computes snapped 1/4 steps, skips occupied steps, and handles reverse drags", () => {
  const scene = sampleScene();
  const forward = computeDrawStrokeSteps(scene, 1000, 1500, 1);
  assert.deepEqual(forward.steps, [1000, 1125, 1250, 1375, 1500]);
  assert.deepEqual(forward.newSteps, [1000, 1125, 1375, 1500], "skips already-occupied 1250 ms on row 1");

  const backward = computeDrawStrokeSteps(scene, 1500, 1000, 1);
  assert.deepEqual(backward.steps, [1500, 1375, 1250, 1125, 1000]);
  assert.deepEqual(backward.newSteps, [1500, 1375, 1125, 1000]);
});

test("computeDrawStrokeSteps respects Objects snap mode, snap off, and 512-step cap", () => {
  const objScene = sampleScene({ snapMode: "objects" });
  const objStroke = computeDrawStrokeSteps(objScene, 990, 1510, 1);
  assert.deepEqual(objStroke.steps, [1000, 1500]);
  assert.deepEqual(objStroke.newSteps, [1000, 1500]);

  const unsnapped = sampleScene({ snap: false });
  assert.equal(snapTimeInScene(unsnapped, 1037), 1037);
  const offStroke = computeDrawStrokeSteps(unsnapped, 1000, 1500, 1);
  assert.deepEqual(offStroke.steps, [1500]);

  const hugeStroke = computeDrawStrokeSteps(sampleScene(), 0, 200_000, 1);
  assert.equal(hugeStroke.steps.length, MAX_DRAW_STROKE_STEPS);

  // Across a tempo change where the next red line is closer than the active section's grid step,
  // snapTimeInScene must pick the red line to match Rust's TempoMap::snap.
  const tempoChange = sampleScene({
    sections: [
      [0, 500, 4],     // 125 ms steps: 1000, 1125, 1250
      [1180, 400, 4],  // red line at 1180 ms
    ],
  });
  assert.equal(snapTimeInScene(tempoChange, 1170), 1180, "must snap to closer next tempo section red line");
  assert.equal(snapTimeInScene(tempoChange, 1130), 1125, "must keep active section step when closer");
});

async function drawFrontend({ deferInvoke = false } = {}) {
  const controls = new Map();
  const documentEvents = new Map();
  const calls = [];
  const resolvers = [];
  let defer = false;
  function element() {
    return {
      events: new Map(),
      children: [],
      dataset: {},
      style: { setProperty(name, value) { this[name] = value; } },
      addEventListener(name, listener) { this.events.set(name, listener); },
      append() {},
      prepend() {},
      replaceChildren() {},
      setAttribute(name, value) { this[name] = value; },
      getBoundingClientRect: () => ({ x: 0, y: 0, top: 0, width: 1000, height: 300 }),
      setPointerCapture() {},
      hasPointerCapture: () => true,
      releasePointerCapture() {},
    };
  }
  function get(id) {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  }
  get("grid").clientWidth = 1000;
  get("scroll").scrollTop = 0;
  const status = { busy: false, hasDocument: true, tool: "Draw", duration: 10_000, playing: false, position: 0 };
  const context = createContext({
    document: {
      documentElement: { dataset: {} },
      getElementById: get,
      createElement: element,
      querySelectorAll: () => [],
      addEventListener(name, handler) { documentEvents.set(name, handler); },
    },
    window: {
      __TAURI__: {
        core: {
          invoke: (_name, args) => {
            if (_name === "editor_waveform") return Promise.resolve(new Float32Array(0));
            calls.push(args ? JSON.parse(JSON.stringify(args)) : undefined);
            const reply = () => ({
              ...status,
              ...(args?.scene ? { scene: sampleScene({ viewStart: 0, zoom: 0.5 }) } : {}),
            });
            if (defer) {
              return new Promise((resolve) => {
                resolvers.push(() => resolve(reply()));
              });
            }
            return Promise.resolve(reply());
          },
        },
        event: { listen() {} },
      },
    },
    performance: { now: () => 0 },
    requestAnimationFrame: () => {},
    cancelAnimationFrame: () => {},
    setInterval: () => {},
    ResizeObserver: class { observe() {} },
    HTMLElement: class {},
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    Event: class { constructor(type) { this.type = type; } },
    console,
  });
  const editor = bootEditor(context, { autoStart: false });
  await editor.act(null);
  defer = deferInvoke;
  calls.length = 0;
  const settle = () => editor.state.pending;
  const flushOne = async () => {
    for (let i = 0; i < 10 && resolvers.length === 0; i++) {
      await new Promise(r => setTimeout(r, 0));
    }
    assert.ok(resolvers.length > 0, "expected a pending invoke to flush");
    resolvers.shift()();
    await new Promise(r => setTimeout(r, 0));
  };
  const flushAll = async () => {
    for (let i = 0; i < 200 && (resolvers.length > 0 || editor.state.requests > 0); i++) {
      while (resolvers.length) resolvers.shift()();
      await new Promise(r => setTimeout(r, 0));
    }
    await editor.state.pending;
  };
  return { calls, documentEvents, editor, get, resolvers, flushOne, flushAll, settle };
}

test("Draw tool drag previews ghost steps and commits a single paint_draw_stroke on pointerup", async () => {
  const { calls, editor, get, settle } = await drawFrontend();
  const grid = get("grid");
  editor.state.view = { busy: false, hasDocument: true, tool: "Draw", activeLane: 1 };
  editor.state.scene = sampleScene({ viewStart: 0, zoom: 0.5 });
  editor.state.laneRows = [0, 1];

  const pt = (x, y) => ({
    clientX: x,
    clientY: y,
    pointerId: 7,
    button: 0,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    preventDefault() {},
  });

  // Row 1 is y = 45 (rowHeight 30). At zoom 0.5, x = 500 => 1000 ms, x = 750 => 1500 ms.
  grid.events.get("pointerdown")(pt(500, 45));
  grid.events.get("pointermove")(pt(750, 45));
  assert.deepEqual(editor.state.drawStrokePreview, {
    row: 1,
    steps: [1000, 1125, 1375, 1500],
    volume: 70,
  });

  grid.events.get("pointerup")(pt(750, 45));
  await settle();
  assert.equal(editor.state.drawStrokePreview, null, "preview clears on pointerup");
  assert.deepEqual(
    calls.map(({ action }) => [action.kind, action.phase ?? action.row]),
    [
      ["pointer", "down"],
      ["pointer", "cancel"],
      ["paint_draw_stroke", 1],
    ],
  );
  assert.equal(calls[2].action.start_ms, 1000);
  assert.equal(calls[2].action.end_ms, 1500);
});

test("Draw tool single-click keeps pointer down/up intact without invoking paint_draw_stroke", async () => {
  const { calls, editor, get, settle } = await drawFrontend();
  const grid = get("grid");
  editor.state.view = { busy: false, hasDocument: true, tool: "Draw", activeLane: 1 };
  editor.state.scene = sampleScene({ viewStart: 0, zoom: 0.5 });
  editor.state.laneRows = [0, 1];

  const pt = {
    clientX: 500,
    clientY: 45,
    pointerId: 9,
    button: 0,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    preventDefault() {},
  };
  grid.events.get("pointerdown")(pt);
  grid.events.get("pointerup")(pt);
  await settle();
  assert.deepEqual(
    calls.map(({ action }) => [action.kind, action.phase]),
    [
      ["pointer", "down"],
      ["pointer", "up"],
    ],
  );
});

test("computeDrawStrokeSteps skips steps occupied on any member of a merged Set", () => {
  const scene = sampleScene({
    moments: [
      [1000, 100, true, [[0, false, false, 100]]],
      [1250, 90, false, [[1, false, false, 90]]],
    ],
  });
  const merged = computeDrawStrokeSteps(scene, 1000, 1500, new Set([0, 1]));
  assert.deepEqual(merged.steps, [1000, 1125, 1250, 1375, 1500]);
  assert.deepEqual(merged.newSteps, [1125, 1375, 1500]);
});

test("Draw drag keeps optimistic hits alive when slim pointerdown and cancel finish before paint_draw_stroke returns scene", async () => {
  const { calls, editor, get, resolvers, flushOne, flushAll } = await drawFrontend({ deferInvoke: true });
  const grid = get("grid");
  editor.state.view = { busy: false, hasDocument: true, tool: "Draw", activeLane: 1 };
  editor.state.scene = sampleScene({ viewStart: 0, zoom: 0.5 });
  editor.state.laneRows = [0, 1];

  const pt = (x, y) => ({
    clientX: x,
    clientY: y,
    pointerId: 11,
    button: 0,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    preventDefault() {},
  });

  // Start on empty snapped 1125 ms (x = 562.5 at zoom 0.5) on row 1 (y = 45).
  grid.events.get("pointerdown")(pt(562.5, 45));
  assert.deepEqual(editor.state.pendingDrawHits.map(h => h.time), [1125]);

  // Slim pointerdown (scene: false) returns BEFORE pointermove/pointerup.
  await flushOne();
  await editor.state.pending;
  assert.equal(editor.state.requests, 0);
  assert.deepEqual(editor.state.pendingDrawHits.map(h => h.time), [1125], "initial hit stays visible across early pointerdown reply");

  // Drag across 1125..1500 ms -> ghost stroke preview appears, then release.
  grid.events.get("pointermove")(pt(750, 45));
  grid.events.get("pointerup")(pt(750, 45));
  assert.equal(editor.state.drawStrokePreview, null);
  assert.ok(editor.state.pendingDrawHits.some(h => h.time === 1375), "stroke steps pushed optimistically");

  // Flush the preceding cancel (scene: false) IPC, leaving paint_draw_stroke (scene: true) in flight.
  await flushOne();
  assert.equal(calls.at(-2).action.phase, "cancel");
  assert.equal(calls.at(-2).scene, false);
  assert.equal(resolvers.length, 1, "paint_draw_stroke still pending");
  assert.ok(
    editor.state.pendingDrawHits.some(h => h.time === 1375),
    "cancel (scene: false) arrival must NOT clear pendingDrawHits pushed for paint_draw_stroke",
  );

  // Flush paint_draw_stroke (scene: true) -> now optimistic hits clear seamlessly.
  await flushAll();
  assert.equal(calls.at(-1).action.kind, "paint_draw_stroke");
  assert.equal(calls.at(-1).scene, true);
  assert.equal(editor.state.pendingDrawHits.length, 0, "optimistic draw hits clear once paint_draw_stroke scene arrives");
});
