import assert from "node:assert/strict";
import { performance as nodePerformance } from "node:perf_hooks";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const LANE_COUNT = 30;
const TOTAL_HITS = 40_000;
const SONG_DURATION_MS = 180_000;
const HITS_PER_MOMENT = 5;
const MOMENT_COUNT = TOTAL_HITS / HITS_PER_MOMENT; // 8,000 moments x 5 hits = 40,000 hits
const FRAME_COUNT = 30;
const WARMUP_FRAMES = 4;
const FRAME_DT_MS = 1000 / 144;
const P95_BUDGET_MS = 16;

function createMockCanvasContext() {
  return {
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    globalAlpha: 1,
    globalCompositeOperation: "source-over",
    font: "",
    textAlign: "left",
    textBaseline: "alphabetic",
    setTransform() {},
    clearRect() {},
    fillRect() {},
    strokeRect() {},
    beginPath() {},
    closePath() {},
    moveTo() {},
    lineTo() {},
    arc() {},
    arcTo() {},
    roundRect() {},
    fill() {},
    stroke() {},
    fillText() {},
    measureText(text) { return { width: String(text).length * 6.5 }; },
    save() {},
    restore() {},
    drawImage() {},
  };
}

function createHarness() {
  const elements = new Map();
  function createElement() {
    const ctx2d = createMockCanvasContext();
    return {
      width: 0,
      height: 0,
      clientWidth: 1400,
      clientHeight: 640,
      scrollTop: 0,
      hidden: false,
      dataset: {},
      style: { setProperty() {} },
      children: [],
      listeners: {},
      getContext: () => ctx2d,
      append() {},
      prepend() {},
      replaceChildren() {},
      setAttribute() {},
      classList: { toggle() {}, add() {}, remove() {}, contains: () => false },
      addEventListener(name, listener) { (this.listeners[name] ??= []).push(listener); },
      getBoundingClientRect: () => ({ x: 0, y: 0, left: 0, top: 0, width: 1400, height: 640 }),
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, createElement());
    return elements.get(id);
  };

  let simNow = 0;
  let rafQueue = [];
  const context = createContext({
    document: {
      documentElement: { dataset: { uiScale: "100" } },
      getElementById: get,
      createElement,
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: {
      devicePixelRatio: 1,
      __TAURI__: {
        core: { invoke: async () => ({ hasDocument: true, busy: false }) },
        event: { listen() {} },
      },
    },
    performance: { now: () => simNow },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame(cb) { rafQueue.push(cb); return rafQueue.length; },
    cancelAnimationFrame() {},
    setInterval() {},
    console,
  });

  const editorCtx = bootEditor(context, { autoStart: false });

  // Realistic 1080p-class editor panel dimensions.
  get("scroll").clientWidth = 1400;
  get("scroll").clientHeight = 640;
  get("grid").clientWidth = 1400;
  get("grid").clientHeight = 640;
  get("ruler").clientWidth = 1400;
  get("ruler").clientHeight = 90;
  get("controller").clientWidth = 1400;
  get("controller").clientHeight = 96;
  get("overview").clientWidth = 1400;
  get("overview").clientHeight = 28;
  get("volume-controller-panel").hidden = false;
  get("volume-controller-panel").dataset.bottom = "volume";
  get("workspace").hidden = false;
  get("editor-arrange").hidden = false;

  // Build synthetic dense scene: 30 lanes x 40,000 hits across a 3-minute song.
  const lanes = [];
  const laneRows = [];
  const lanePositions = new Map();
  const laneVisualPositions = new Map();
  for (let i = 0; i < LANE_COUNT; i++) {
    lanes.push({
      key: `lane-${i}`,
      name: `Sample ${i + 1}`,
      group: i < 10 ? "Drums" : i < 20 ? "Percussion" : "FX",
      solo: false,
      muted: false,
    });
    laneRows.push(i);
    lanePositions.set(i, i);
    laneVisualPositions.set(i, [i]);
  }

  const moments = new Array(MOMENT_COUNT);
  const momentStep = SONG_DURATION_MS / MOMENT_COUNT;
  for (let m = 0; m < MOMENT_COUNT; m++) {
    const time = Math.round(m * momentStep);
    const hits = new Array(HITS_PER_MOMENT);
    for (let h = 0; h < HITS_PER_MOMENT; h++) {
      const row = (m * 7 + h * 5) % LANE_COUNT;
      const volume = 40 + ((m + h * 13) % 61);
      hits[h] = [row, false, false, volume];
    }
    moments[m] = [time, 80, m % 4 === 0, hits];
  }

  // Typed 6-float-per-bin waveform (10 ms per bin => 18,000 bins for 3 minutes).
  const binCount = 18_000;
  const waveform = new Float32Array(binCount * 6);
  for (let i = 0; i < binCount; i++) {
    const base = i * 6;
    const phase = (i % 64) / 64;
    const amp = 0.25 + 0.65 * Math.abs(Math.sin(i * 0.17));
    waveform[base] = amp;
    waveform[base + 1] = 0.4 + 0.5 * phase;
    waveform[base + 2] = 0.3 + 0.4 * (1 - phase);
    waveform[base + 3] = 0.2 + 0.3 * Math.abs(Math.cos(i * 0.09));
    waveform[base + 4] = amp;
    waveform[base + 5] = amp * 0.95;
  }

  // Gameplay objects across the 3-minute song (~1,200 objects).
  const points = [];
  const spans = [];
  for (let t = 500, obj = 0; t < SONG_DURATION_MS - 500; t += 150, obj++) {
    if (obj % 3 === 0) {
      points.push(["slider", t, obj], ["slider", t + 100, obj]);
      spans.push(["slider", t, t + 100, obj]);
    } else {
      points.push(["circle", t, obj]);
    }
  }

  const state = editorCtx.state;
  state.laneRows = laneRows;
  state.lanePositions = lanePositions;
  state.laneVisualPositions = laneVisualPositions;
  state.previewParents = new Map();
  state.waveformShown = true;
  state.waveformHeight = 48;
  state.view = {
    hasDocument: true,
    busy: false,
    playing: true,
    position: 30_000,
    duration: SONG_DURATION_MS,
    speed: 1,
    offset: 0,
    left: 20_000,
    right: 60_000,
    cycle: true,
    activeLane: 0,
    objectLines: true,
    tool: "Draw",
    hitsoundsMuted: false,
    outputProblems: [],
  };
  state.scene = {
    path: "synthetic-dense-30x40k.osu",
    viewStart: 28_000,
    zoom: 0.2, // ~7,000 ms visible across 1400 px
    rowHeight: 24,
    snap: true,
    snapMode: "beat",
    beats: 1,
    divisor: 4,
    sections: [[0, 333.3333, 4]], // 180 BPM
    lanes,
    moments,
    waveform,
    gameplay: { points, spans },
    bookmarks: [
      { time: 30_000, label: "Kiai 1" },
      { time: 60_000, label: "Break" },
      { time: 120_000, label: "Kiai 2" },
    ],
  };

  return {
    editorCtx,
    state,
    setSimNow(ms) { simNow = ms; },
    evaluate: code => runInContext(code, context),
  };
}

function percentile(sorted, p) {
  if (!sorted.length) return 0;
  const idx = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[Math.max(0, idx)];
}

function measureFrames(count, fn) {
  const samples = new Array(count);
  for (let i = 0; i < count; i++) {
    const t0 = nodePerformance.now();
    fn(i);
    samples[i] = nodePerformance.now() - t0;
  }
  const sorted = [...samples].sort((a, b) => a - b);
  return {
    p50: percentile(sorted, 50),
    p95: percentile(sorted, 95),
    max: sorted[sorted.length - 1],
  };
}

test("144 Hz frame budget: 30 lanes x 40,000 hits stays under 16 ms p95 (scrolling full paint & cached playback)", (t) => {
  const harness = createHarness();
  const { editorCtx, state } = harness;

  // Sanity check synthetic scene density.
  const hitCount = state.scene.moments.reduce((sum, m) => sum + m[3].length, 0);
  assert.equal(state.scene.lanes.length, LANE_COUNT);
  assert.equal(hitCount, TOTAL_HITS);

  // 1. Worst-case 144 Hz Centre-mode scrolling (full viewport paint every frame).
  for (let i = 0; i < WARMUP_FRAMES; i++) {
    const tMs = i * FRAME_DT_MS;
    harness.setSimNow(tMs);
    state.view.position = 30_000 + tMs;
    state.scene.viewStart = 28_000 + tMs;
    editorCtx.anchorPlayhead(state.view);
    editorCtx.paint();
  }
  const scrollStats = measureFrames(FRAME_COUNT, (i) => {
    const tMs = (WARMUP_FRAMES + i) * FRAME_DT_MS;
    harness.setSimNow(tMs);
    state.view.position = 30_000 + tMs;
    state.scene.viewStart = 28_000 + tMs;
    editorCtx.anchorPlayhead(state.view);
    editorCtx.paint();
  });

  // 2. Stationary-viewport 144 Hz playback (static layer cache + playhead + hit-lighting).
  state.scene.viewStart = 30_000;
  state.lastLayerKey = null;
  state.staticLayers = null;
  for (let i = 0; i < 2; i++) {
    harness.setSimNow(i * FRAME_DT_MS);
    state.view.position = 31_000 + i * FRAME_DT_MS;
    editorCtx.anchorPlayhead(state.view);
    editorCtx.paintFrame();
  }
  assert.ok(state.staticLayers, "static layer cache should engage for stationary playback");
  const cachedStats = measureFrames(FRAME_COUNT, (i) => {
    const tMs = (2 + i) * FRAME_DT_MS;
    harness.setSimNow(tMs);
    state.view.position = 31_000 + tMs;
    editorCtx.anchorPlayhead(state.view);
    editorCtx.paintFrame();
  });

  const summary =
    `[frame-budget 30x40k @144Hz] ` +
    `scroll-paint p50=${scrollStats.p50.toFixed(2)}ms p95=${scrollStats.p95.toFixed(2)}ms max=${scrollStats.max.toFixed(2)}ms | ` +
    `cached-playhead p50=${cachedStats.p50.toFixed(2)}ms p95=${cachedStats.p95.toFixed(2)}ms max=${cachedStats.max.toFixed(2)}ms`;
  t.diagnostic(summary);
  console.log(summary);

  assert.ok(
    scrollStats.p95 < P95_BUDGET_MS,
    `Scrolling full-paint p95 (${scrollStats.p95.toFixed(2)} ms) exceeded ${P95_BUDGET_MS} ms CI budget`,
  );
  assert.ok(
    cachedStats.p95 < P95_BUDGET_MS,
    `Cached playback p95 (${cachedStats.p95.toFixed(2)} ms) exceeded ${P95_BUDGET_MS} ms CI budget`,
  );
});
