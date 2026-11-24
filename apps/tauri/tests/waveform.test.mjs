import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const css = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");

test("workstation chrome uses the design's neutral grey surfaces and shared blue accent", () => {
  assert.match(css, /:root\s*\{[^}]*--base:\s*#0e0e10\b/);
  assert.match(css, /:root\s*\{[^}]*--panel:\s*#151517\b/);
  assert.match(css, /:root\s*\{[^}]*--hi:\s*#5ac8fa\b/);
});
test("the ruler paints the teal waveform frequency shades on a teal tint; the overview keeps a neutral background", async () => {
  const elements = new Map();
  function element() {
    const bands = [];
    const fills = [];
    const objects = [];
    const hitMarks = [];
    const ctx = new Proxy({
      arc(x, y, radius) { objects.push({ x, y, radius, afterWaveform: bands.length > 0 }); },
      fill() {
        fills.push(this.fillStyle);
        if (typeof this.fillStyle === "string" && this.fillStyle.startsWith("rgba(") && this.fillStyle !== "rgba(241,241,241,.06)") {
          bands.push({ color: this.fillStyle });
        }
      },
      fillRect(x, y, width, height) {
        fills.push(this.fillStyle);
        if (typeof this.fillStyle === "string" && this.fillStyle.startsWith("rgba(") && width === 1 && height > 2) {
          bands.push({ color: this.fillStyle });
        }
        if (this.fillStyle === "#a3a3a3") hitMarks.push([x, y]);
      },
    }, {get: (target, name) => target[name] ?? (() => {})});
    return {
      bands, fills, objects, hitMarks, getContext: () => ctx, clientWidth: 300, clientHeight: 80, scrollTop: 0,
      dataset: {}, style: {}, children: [], addEventListener() {}, append() {}, prepend() {},
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const context = createContext({
    document: {getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {}},
    window: {devicePixelRatio: 1, __TAURI__: {
      core: {invoke: async () => ({hasDocument: false, busy: false})},
      event: {listen() {}},
    }},
    ResizeObserver: class {observe() {}}, requestAnimationFrame() {}, setInterval() {},
    console,
  });
  bootEditor(context, { autoStart: false });
  get("volume-controller-panel").hidden = true;
  runInContext(`
    view = {hasDocument: true, duration: 1200, position: 0, left: 0, right: 1200};
    scene = {path: "fixture.osu", viewStart: 0, zoom: .25, rowHeight: 30, moments: [], sections: [],
      waveform: [[.4,.38,.12,.02], [.6,.18,.54,.17], [.8,.025,.19,.68]]};
    paint();`, context);
  assert.equal(get("ruler").fills[0], "#121718");
  assert.equal(get("overview").fills[0], "#151517");
  assert.equal(get("grid").fills[0], "#0e0e10");
  const waveformBands = { ruler: [...get("ruler").bands], overview: [...get("overview").bands] };
  runInContext(`
    scene.gameplay = {points: [["circle", 200], ["slider", 600]], spans: [["slider", 600, 800], ["spinner", 900, 1000]]};
    paint();`, context);
  assert.deepEqual(get("ruler").objects, [
    { x: 50, y: 60, radius: 17, afterWaveform: true },
    { x: 150, y: 60, radius: 17, afterWaveform: true },
  ]);
  assert.ok(get("ruler").fills.includes("#626567"), "slider track");
  assert.ok(get("ruler").fills.includes("#e6eef2"));
  assert.deepEqual(get("overview").objects, []);
  get("grid").fills.length = 0;
  runInContext(`
    scene.lanes = [{colourKey: "Skin|normal-hitclap"}, {colourKey: "Skin|normal-hitfinish"}];
    laneRows = [0, 1];
    hydrateLaneLayout({lane_colours: {"Skin|normal-hitclap": "#ff5500"}});
    paint();`, context);
  // v3: a picked colour tints its row 8 %.
  assert.deepEqual(get("grid").fills.slice(0, 3), ["#0e0e10", "#ff550014", "#151517"]);
  get("grid").fills.length = 0;
  runInContext("hydrateLaneLayout(null); paint()", context);
  assert.ok(!get("grid").fills.includes("#ff55001f"));
  // The overview draws object diamonds, not the song's waveform (timeline-spec "Overview strip").
  assert.deepEqual(waveformBands.overview, []);
  assert.ok(waveformBands.ruler.length > 0, "ruler paints osu!lazer waveform trapezoids");
  // Low-dominant segment should blend brightest toward osu!lazer's #ddffff (high R/G/B).
  const firstColor = waveformBands.ruler[0].color;
  const lastColor = waveformBands.ruler.at(-1).color;
  assert.notEqual(firstColor, lastColor, "different frequency bands produce distinct colours");
});

test("waveform() draws identical fillRect and fillStyle calls from Float32Array and nested arrays", () => {
  const nestedBins = [
    [0.5, 0.375, 0.125, 0.0625, 0.5, 0.25],
    [0.75, 0.125, 0.625, 0.25, 0.625, 0.75],
    [0.875, 0.0625, 0.25, 0.75, 0.875, 0.5],
    [0.25, 0.5, 0.5, 0.5, 0.25, 0.125],
  ];
  const typedBins = new Float32Array(nestedBins.flat());

  function drawWith(bins) {
    const context = createContext({
      document: {
        getElementById: () => ({
          clientWidth: 200, clientHeight: 80, scrollTop: 0, dataset: {}, style: {}, children: [],
          addEventListener() {}, append() {}, prepend() {}, setAttribute() {},
          getContext: () => ({}),
        }),
        createElement: () => ({}),
        querySelectorAll: () => [],
        addEventListener() {},
      },
      window: {
        devicePixelRatio: 1,
        __TAURI__: { core: { invoke: async () => ({}) }, event: { listen() {} } },
      },
      ResizeObserver: class { observe() {} },
      requestAnimationFrame() {},
      setInterval() {},
      console,
    });
    bootEditor(context, { autoStart: false });
    const ops = [];
    const mockCtx = {
      _fillStyle: "",
      get fillStyle() { return this._fillStyle; },
      set fillStyle(v) { this._fillStyle = v; ops.push(["fillStyle", v]); },
      fillRect(x, y, w, h) { ops.push(["fillRect", x, y, w, h]); },
    };
    runInContext(`
      view = { hasDocument: true, duration: 1000, offset: 0 };
      scene = { path: "fixture.osu", viewStart: 0, zoom: 0.2, rowHeight: 30, moments: [], sections: [] };
    `, context);
    context.scene.waveform = bins;
    context.waveform(mockCtx, 200, 40, 30, 0, 1000);
    return ops;
  }

  const fromNested = drawWith(nestedBins);
  const fromTyped = drawWith(typedBins);
  assert.ok(fromNested.length > 0, "waveform() emitted draw calls");
  assert.deepEqual(fromTyped, fromNested);
});

test("ipc fetches binary editor_waveform once per (path, previewKey), stores Float32Array, and repaints", async () => {
  const bins = [0.5, 0.25, 0.125, 0.0625, 0.5, 0.25, 0.75, 0.125, 0.5, 0.25, 0.625, 0.75];
  const raw = new Float32Array(bins);
  const calls = [];
  let paints = 0;
  const element = () => ({
    clientWidth: 200, clientHeight: 80, scrollTop: 0, dataset: {}, style: {}, children: [],
    addEventListener() {}, append() {}, prepend() {}, setAttribute() {},
    getContext: () => new Proxy({}, { get: () => () => {} }),
  });
  const elements = new Map();
  const get = id => (elements.has(id) ? elements : elements.set(id, element())).get(id);
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: {
      devicePixelRatio: 1,
      __TAURI__: {
        core: {
          invoke: async (name) => {
            calls.push(name);
            if (name === "editor_waveform") return raw.buffer.slice(0);
            return { busy: false, hasDocument: false };
          },
        },
        event: { listen() {} },
      },
    },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  bootEditor(context, { autoStart: false });
  runInContext(`
    renderScene = () => {};
    renderSelection = () => {};
    paintSoon = () => { globalThis.__paints = (globalThis.__paints || 0) + 1; };
    const s = { path: "Map.osu", previewKey: 3, viewStart: 0, zoom: 0.2, rowHeight: 30, lanes: [], moments: [], sections: [[0, 500, 4]] };
    show({ busy: false, hasDocument: true, duration: 4000, scene: { ...s } });
    // Second frame with same (path, previewKey) must not re-fetch editor_waveform.
    show({ busy: false, hasDocument: true, duration: 4000, scene: { ...s } });
  `, context);
  await new Promise(resolve => setTimeout(resolve, 10));
  paints = runInContext("globalThis.__paints", context);
  assert.deepEqual(calls, ["editor_waveform"]);
  assert.equal(context.scene.waveform instanceof Float32Array, true);
  assert.deepEqual(Array.from(context.scene.waveform), bins);
  assert.ok(paints >= 2, "paintSoon called on waveform arrival");
});