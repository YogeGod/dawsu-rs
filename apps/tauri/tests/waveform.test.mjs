import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const css = await readFile(new URL("../ui/src/legacy-chrome.css", import.meta.url), "utf8");

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

test("ipc fetches binary editor_waveform once per (path, previewKey), stores Float32Array, and re-fetches when songLoading finishes", async () => {
  const bins = [0.5, 0.25, 0.125, 0.0625, 0.5, 0.25, 0.75, 0.125, 0.5, 0.25, 0.625, 0.75];
  const raw = new Float32Array(bins);
  const calls = [];
  let waveformReply = new ArrayBuffer(0);
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
            if (name === "editor_shortcuts") return null;
            calls.push(name);
            if (name === "editor_waveform") return waveformReply.slice(0);
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
    show({ busy: false, hasDocument: true, songLoading: true, waveformRevision: 0, duration: 4000, scene: { ...s } });
    // Second frame while still loading must not re-fetch editor_waveform.
    show({ busy: false, hasDocument: true, songLoading: true, waveformRevision: 0, duration: 4000, scene: { ...s } });
  `, context);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(calls, ["editor_waveform"]);
  assert.equal(context.scene.waveform.length, 0);

  waveformReply = raw.buffer;
  runInContext(`
    show({ busy: false, hasDocument: true, songLoading: false, waveformRevision: 1, duration: 4000, scene: { ...s } });
  `, context);
  await new Promise(resolve => setTimeout(resolve, 10));
  paints = runInContext("globalThis.__paints", context);
  assert.deepEqual(calls, ["editor_waveform", "editor_waveform"]);
  assert.equal(context.scene.waveform instanceof Float32Array, true);
  assert.deepEqual(Array.from(context.scene.waveform), bins);
  assert.ok(paints >= 3, "paintSoon called on waveform arrival");
});

test("ipc re-fetches binary editor_waveform when stem toggle changes waveformRevision/stemActive or previewKey", async () => {
  const songBins = [0.5, 0.25, 0.125, 0.0625, 0.5, 0.25];
  const stemBins = [0.875, 0.75, 0.5, 0.25, 0.875, 0.875];
  const calls = [];
  let waveformReply = new Float32Array(songBins).buffer;
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
            if (name === "editor_shortcuts") return null;
            calls.push(name);
            if (name === "editor_waveform") return waveformReply.slice(0);
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
    paintSoon = () => {};
    const s = { path: "Map.osu", previewKey: 3, viewStart: 0, zoom: 0.2, rowHeight: 30, lanes: [], moments: [], sections: [[0, 500, 4]] };
    show({ busy: false, hasDocument: true, songLoading: false, waveformRevision: 1, stemLoaded: true, stemActive: false, stemName: "drums.wav", duration: 4000, scene: { ...s } });
  `, context);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(calls, ["editor_waveform"]);
  assert.deepEqual(Array.from(context.scene.waveform), songBins);
  assert.equal(get("song-stem").textContent, "Song");
  assert.equal(get("song-stem").ariaPressed, "false");

  // Toggle to stem (without a full scene payload — light action response).
  waveformReply = new Float32Array(stemBins).buffer;
  runInContext(`
    show({ busy: false, hasDocument: true, songLoading: false, waveformRevision: 2, stemLoaded: true, stemActive: true, stemName: "drums.wav", duration: 4000 });
  `, context);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(calls, ["editor_waveform", "editor_waveform"]);
  assert.deepEqual(Array.from(context.scene.waveform), stemBins);
  assert.equal(get("song-stem").textContent, "Stem");
  assert.equal(get("song-stem").ariaPressed, "true");

  // Toggle back to song via previewKey / revision bump.
  waveformReply = new Float32Array(songBins).buffer;
  runInContext(`
    show({ busy: false, hasDocument: true, songLoading: false, waveformRevision: 3, stemLoaded: true, stemActive: false, stemName: "drums.wav", duration: 4000, scene: { ...s, previewKey: 4 } });
  `, context);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(calls, ["editor_waveform", "editor_waveform", "editor_waveform"]);
  assert.deepEqual(Array.from(context.scene.waveform), songBins);
  assert.equal(get("song-stem").textContent, "Song");
});

test("the Song row shows a subtle Loading song label while the waveform is still decoding", () => {
  const elements = new Map();
  function element() {
    const texts = [];
    const ctx = new Proxy({
      fillText(text, x, y) { texts.push({ text, x, y, fill: this.fillStyle }); },
    }, { get: (target, name) => target[name] ?? (() => {}) });
    return {
      texts, getContext: () => ctx, clientWidth: 300, clientHeight: 80, scrollTop: 0,
      dataset: {}, style: {}, children: [], addEventListener() {}, append() {}, prepend() {},
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { devicePixelRatio: 1, __TAURI__: {
      core: { invoke: async () => ({ hasDocument: false, busy: false }) },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} }, requestAnimationFrame() {}, setInterval() {},
    console,
  });
  bootEditor(context, { autoStart: false });
  get("volume-controller-panel").hidden = true;
  runInContext(`
    view = { hasDocument: true, songLoading: true, duration: 1200, position: 0, left: 0, right: 1200 };
    scene = { path: "fixture.osu", viewStart: 0, zoom: .25, rowHeight: 30, moments: [], sections: [], waveform: [] };
    paint();`, context);
  assert.ok(get("ruler").texts.some(t => t.text === "Loading song…"), "shows Loading song… while decoding");
  get("ruler").texts.length = 0;
  runInContext(`
    view = { ...view, songLoading: false };
    scene = { ...scene, waveform: [[.4,.38,.12,.02]] };
    paint();`, context);
  assert.ok(!get("ruler").texts.some(t => t.text === "Loading song…"), "clears label once waveform arrives");
});

test("ipc decodes DWFP pyramid, preserves base Float32Array, selects coarser levels on zoom-out, and paints #ff453a clip ticks", async () => {
  // Build a valid 2-level DWFP buffer in JS (L0 = 1024 bins, L1 = 256 bins) with a +0.5 dBTP clip at L0[500] / L1[125].
  function makeDwfpBuffer() {
    const l0Count = 1024;
    const l1Count = 256;
    const l0MaskBytes = ((Math.ceil(l0Count / 8) + 3) >> 2) << 2;
    const l1MaskBytes = ((Math.ceil(l1Count / 8) + 3) >> 2) << 2;
    const totalBytes = 12 + 2 * 8 + l0Count * 24 + l0MaskBytes + l1Count * 24 + l1MaskBytes;
    const buf = new ArrayBuffer(totalBytes);
    const u8 = new Uint8Array(buf);
    const dv = new DataView(buf);
    u8[0] = 0x44; u8[1] = 0x57; u8[2] = 0x46; u8[3] = 0x50; // DWFP
    dv.setUint16(4, 1, true);
    dv.setUint16(6, 2, true);
    dv.setUint32(8, 6, true);
    dv.setUint32(12, l0Count, true); dv.setUint32(16, 1, true);
    dv.setUint32(20, l1Count, true); dv.setUint32(24, 4, true);

    let off = 28;
    for (let i = 0; i < l0Count; i++) {
      const pk = i === 500 ? 1.0 : 0.25;
      dv.setFloat32(off, pk, true);
      dv.setFloat32(off + 4, 0.5, true);
      dv.setFloat32(off + 8, 0.3, true);
      dv.setFloat32(off + 12, 0.2, true);
      dv.setFloat32(off + 16, pk, true);
      dv.setFloat32(off + 20, pk, true);
      off += 24;
    }
    u8[off + (500 >> 3)] |= 1 << (500 & 7);
    off += l0MaskBytes;

    for (let i = 0; i < l1Count; i++) {
      const pk = i === 125 ? 1.0 : 0.25;
      dv.setFloat32(off, pk, true);
      dv.setFloat32(off + 4, 0.5, true);
      dv.setFloat32(off + 8, 0.3, true);
      dv.setFloat32(off + 12, 0.2, true);
      dv.setFloat32(off + 16, pk, true);
      dv.setFloat32(off + 20, pk, true);
      off += 24;
    }
    u8[off + (125 >> 3)] |= 1 << (125 & 7);
    return buf;
  }

  const dwfp = makeDwfpBuffer();
  const calls = [];
  const rectOps = [];
  const element = () => ({
    clientWidth: 200, clientHeight: 80, scrollTop: 0, dataset: {}, style: {}, children: [],
    addEventListener() {}, append() {}, prepend() {}, setAttribute() {},
    getContext: () => ({
      _fillStyle: "",
      get fillStyle() { return this._fillStyle; },
      set fillStyle(v) { this._fillStyle = v; },
      fillRect(x, y, w, h) { rectOps.push({ style: this._fillStyle, x, y, w, h }); },
    }),
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
            if (name === "editor_shortcuts") return null;
            calls.push(name);
            if (name === "editor_waveform") return dwfp.slice(0);
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
    paintSoon = () => {};
    show({ busy: false, hasDocument: true, songLoading: false, waveformRevision: 1, duration: 102400,
      scene: { path: "Map.osu", previewKey: 5, viewStart: 0, zoom: 0.001, rowHeight: 30, lanes: [], moments: [], sections: [] } });
  `, context);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(calls, ["editor_waveform"]);
  assert.equal(context.scene.waveform instanceof Float32Array, true);
  assert.equal(context.scene.waveform.length, 1024 * 6);
  assert.equal(context.scene.waveformPyramid?.levels?.length, 2);

  // Zoomed out: binsPerPx = (102400/200)*(1024/102400) = 5.12 >= 4 -> renders from L1 + emits #ff453a clip ticks.
  context.waveform(get("ruler").getContext(), 200, 40, 30, 0, 102400);
  const clipTicks = rectOps.filter(op => op.style === "#ff453a" && op.w === 1 && op.h === 2);
  assert.ok(clipTicks.length >= 2, `expected top & bottom #ff453a clip ticks, got ${JSON.stringify(clipTicks)}`);
});

test("subscribe_telemetry stays gated without Channel, parses 64-byte 0x01 frames, and updates playhead/BPM without editor_view polls", async () => {
  const gatedCalls = [];
  const element = () => ({
    clientWidth: 200, clientHeight: 80, scrollTop: 0, dataset: {},
    style: { setProperty() {} }, children: [],
    addEventListener() {}, append() {}, prepend() {}, setAttribute() {}, replaceChildren() {},
    matches: () => false, closest: () => null,
    getContext: () => new Proxy({}, { get: () => () => {} }),
  });
  const gatedElements = new Map();
  const getGated = id => (gatedElements.has(id) ? gatedElements : gatedElements.set(id, element())).get(id);
  const gatedCtx = createContext({
    document: { getElementById: getGated, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: {
      devicePixelRatio: 1,
      __TAURI__: {
        core: {
          invoke: async (name) => {
            if (name === "editor_shortcuts") return null;
            gatedCalls.push(name);
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
  bootEditor(gatedCtx, { autoStart: true });
  await gatedCtx.pending;
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(gatedCalls, ["editor_view"], "subscribe_telemetry must stay uninvoked when Channel is absent");

  const calls = [];
  let createdChannel = null;
  let serverPlaying = false;
  let serverPos = 0;
  let serverMusical = "1.1.000 · 120.0 BPM · 4/4 · 1/4";
  class FakeChannel {
    constructor() {
      this.onmessage = null;
      createdChannel = this;
    }
  }
  const elements = new Map();
  const get = id => (elements.has(id) ? elements : elements.set(id, element())).get(id);
  const uiUpdates = [];
  const baseScene = {
    path: "Map.osu", previewKey: 1, viewStart: 0, zoom: 0.2, rowHeight: 30,
    lanes: [], moments: [], sections: [], difficulties: [],
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: {
      devicePixelRatio: 1,
      dawsuUi: { update: (v, s) => uiUpdates.push({ v, s }), lanes() {}, inspector() {}, browser() {}, mixer() {}, menu() {}, viewport() {}, laneOptions() {} },
      __TAURI__: {
        core: {
          Channel: FakeChannel,
          invoke: async (name) => {
            if (name === "editor_shortcuts") return null;
            calls.push(name);
            if (name === "editor_view") {
              return {
                busy: false, hasDocument: true, playing: serverPlaying, position: serverPos, duration: 60000,
                stateRev: 7, musicalPosition: serverMusical,
                scene: baseScene,
              };
            }
            return {};
          },
        },
        event: { listen() {} },
      },
    },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    performance: { now: () => 1000 },
    console,
  });
  bootEditor(context, { autoStart: true });
  await context.pending;
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual(calls, ["editor_view", "editor_waveform", "subscribe_telemetry"]);
  assert.ok(createdChannel && typeof createdChannel.onmessage === "function");

  const editorViewCallsAfterBoot = calls.filter(c => c === "editor_view").length;
  serverPlaying = true;
  serverPos = 2500.5;
  serverMusical = "3.2.120 · 195.0 BPM · 3/4 · 1/4";
  // Build a 64-byte 0x01 frame matching stateRev 7 crossing a 195.00 BPM 3/4 red line while playing.
  const frame = new ArrayBuffer(64);
  const dv = new DataView(frame);
  dv.setUint8(0, 0x01);
  dv.setUint8(1, 0x01 | 0x04); // playing + has_document
  dv.setUint16(2, 3, true); // meter_num
  dv.setUint16(4, 4, true); // meter_den
  dv.setUint16(6, 120, true); // sub_tick
  dv.setBigUint64(8, 7n, true); // same state_rev
  dv.setFloat64(16, 2500.5, true); // position
  dv.setUint32(24, 19500, true); // 195.00 BPM
  dv.setInt32(28, 3, true); // bar
  dv.setUint32(32, 2, true); // beat
  dv.setFloat32(36, 0.8, true); // master_l
  dv.setFloat32(40, 0.6, true); // master_r
  dv.setFloat32(44, 0.7, true); // song
  dv.setFloat32(48, 0.4, true); // hitsounds

  createdChannel.onmessage(frame);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(context.view.position, 2500.5);
  assert.equal(context.view.playing, true);
  assert.equal(calls.filter(c => c === "editor_view").length, editorViewCallsAfterBoot + 1, "playing flip triggers 1 sync poll; same stateRev avoids extra polls");

  // Second frame while already playing with same stateRev=7 triggers zero editor_view polls and updates BPM/meter live!
  dv.setFloat64(16, 2600.0, true);
  dv.setUint32(24, 21000, true); // red line jump to 210.00 BPM
  dv.setUint16(2, 5, true);
  dv.setUint16(4, 8, true);
  createdChannel.onmessage(frame);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(context.view.position, 2600.0);
  assert.equal(context.view.bpmCenti, 21000);
  assert.equal(context.view.meterNum, 5);
  assert.equal(context.view.meterDen, 8);
  assert.match(context.view.musicalPosition, /3\.2\.120 · 210\.0 BPM · 5\/8/);
  assert.equal(calls.filter(c => c === "editor_view").length, editorViewCallsAfterBoot + 1, "steady playback telemetry must not trigger editor_view JSON polls");
});