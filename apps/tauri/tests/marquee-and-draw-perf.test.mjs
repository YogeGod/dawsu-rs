import assert from "node:assert/strict";
import { test } from "node:test";
import { createContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

function makeScene(overrides = {}) {
  return {
    path: "PerfMap.osu",
    previewKey: 1,
    version: "Hard",
    viewStart: 0,
    zoom: 0.5,
    rowHeight: 30,
    snap: true,
    snapMode: "grid",
    beats: 1,
    divisor: 4,
    insertVolume: 85,
    laneRevision: 1,
    sections: [[0, 500, 4]],
    orphanedHits: [],
    bookmarks: [],
    history: [],
    historyPosition: 0,
    selection: null,
    moments: [
      [1000, 100, true, [[3, false], [10, false]]],
      [1250, 100, true, [[4, false], [9, false], [6, false]]],
      [1500, 100, true, [[3, false], [10, false]]],
    ],
    lanes: Array.from({ length: 12 }, (_, i) => ({
      key: `lane-${i}`,
      colourKey: `lane-${i}`,
      name: `lane-${i}`,
      group: i < 6 ? "normal" : "soft",
      hitCount: i === 3 || i === 4 || i === 9 || i === 10 ? 2 : 0,
      muted: false,
      solo: false,
      canDelete: true,
      canReplace: true,
      canAddLayer: false,
    })),
    laneOrder: [3, 4, 9, 10],
    ...overrides,
  };
}

async function setupHarness({ deferInvoke = false } = {}) {
  const controls = new Map();
  const calls = [];
  const resolvers = [];
  let paintSoonCount = 0;
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
      getBoundingClientRect: () => ({ x: 0, y: 0, left: 0, top: 0, width: 1000, height: 300 }),
      setPointerCapture() {},
      hasPointerCapture: () => true,
      releasePointerCapture() {},
      blur() {},
    };
  }
  function get(id) {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  }
  get("grid").clientWidth = 1000;
  get("scroll").scrollTop = 0;
  let now = 1000;
  const context = createContext({
    document: {
      documentElement: { dataset: {} },
      getElementById: get,
      createElement: element,
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: {
      __TAURI__: {
        core: {
          invoke: (_name, args) => {
            calls.push(JSON.parse(JSON.stringify(args)));
            if (deferInvoke) {
              return new Promise((resolve) => {
                resolvers.push(() => resolve({ busy: false, hasDocument: true, tool: editor.state.view.tool, duration: 10_000, playing: false, position: 0 }));
              });
            }
            return Promise.resolve({ busy: false, hasDocument: true, tool: editor.state.view.tool, duration: 10_000, playing: false, position: 0 });
          },
        },
        event: { listen() {} },
      },
    },
    performance: { now: () => now },
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
  const origPaintSoon = editor.paintSoon;
  editor.paintSoon = (...a) => { paintSoonCount++; return origPaintSoon(...a); };
  editor.state.view = { busy: false, hasDocument: true, tool: "Select", activeLane: 3, duration: 10_000, playing: false, position: 0 };
  editor.state.scene = makeScene();
  // Visual order intentionally puts Soft (rows 9, 10) ABOVE Normal (rows 3, 4), with folder header strings!
  editor.state.laneRows = ["soft", 9, 10, "normal", 3, 4];
  editor.state.lanePositions = new Map([["soft", 0], [9, 1], [10, 2], ["normal", 3], [3, 4], [4, 5]]);
  editor.state.laneVisualPositions = new Map([[9, [1]], [10, [2]], [3, [4]], [4, [5]]]);
  calls.length = 0;
  return {
    editor,
    grid: get("grid"),
    calls,
    resolvers,
    flushOne: () => resolvers.shift()?.(),
    flushAll: async () => {
      for (let i = 0; i < 200 && (resolvers.length > 0 || editor.state.requests > 0); i++) {
        while (resolvers.length) resolvers.shift()();
        await new Promise(r => setTimeout(r, 0));
      }
      await editor.state.pending;
    },
    advanceTime: (ms) => { now += ms; },
    getPaintCount: () => paintSoonCount,
  };
}

const pt = (x, y, id = 1, extra = {}) => ({
  clientX: x, clientY: y, pointerId: id, button: 0,
  altKey: false, ctrlKey: false, metaKey: false, shiftKey: false,
  preventDefault() {},
  ...extra,
});

test("Select marquee can start and end on folder headers, uses visual lane order, and sends zero IPCs during move", async () => {
  const { editor, grid, calls, advanceTime } = await setupHarness();
  // Visual rows: 0="soft", 1=row9, 2=row10, 3="normal", 4=row3, 5=row4 (rowHeight=30)
  // Drag from bottom-right on row 3 (y=135, x=900 -> 1800ms) up to folder header "soft" (y=10, x=400 -> 800ms).
  // Visually enclosed numeric lanes: 9, 10, 3 (NOT row 4, NOT hidden rows 5..8!).
  grid.events.get("pointerdown")(pt(900, 135, 11));
  const afterDownCount = calls.length;
  assert.equal(calls.filter(c => c.scene === true).length, 0, "marquee pointerdown must not request a full scene");
  for (let i = 1; i <= 10; i++) {
    advanceTime(16);
    grid.events.get("pointermove")(pt(900 - i * 50, 135 - i * 12.5, 11));
  }
  assert.equal(calls.length, afterDownCount, "marquee pointermove must send ZERO Rust IPCs");
  grid.events.get("pointerup")(pt(400, 10, 11));
  await editor.state.pending;

  assert.equal(calls.filter(c => c.scene === true).length, 1, "marquee commits with exactly one full-scene IPC");
  const commit = calls.at(-1);
  assert.equal(commit.action.kind, "select_box");
  assert.equal(commit.action.from_ms, 800);
  assert.equal(commit.action.to_ms, 1800);
  assert.deepEqual(commit.action.rows, [9, 10, 3]);
  assert.equal(commit.action.additive, false);

  // Starting a marquee directly on folder header "soft" (y=10) and dragging down across row 10 and row 3
  calls.length = 0;
  grid.events.get("pointerdown")(pt(400, 10, 12));
  grid.events.get("pointermove")(pt(800, 135, 12));
  grid.events.get("pointerup")(pt(800, 135, 12));
  await editor.state.pending;
  assert.equal(calls.length, 1, "starting on a folder header must still marquee-select");
  assert.equal(calls[0].action.kind, "select_box");
  assert.deepEqual(calls[0].action.rows, [9, 10, 3]);
});

test("Draw single-click previews optimistically, sends slim pointerdown + full pointerup, and coalesces rapid clicks", async () => {
  const { editor, grid, calls, flushAll } = await setupHarness({ deferInvoke: true });
  editor.state.view.tool = "Draw";

  // Rapidly click 4 empty spots (row 9 is visual row 1 -> y=45) without waiting for IPC
  for (let i = 0; i < 4; i++) {
    const x = 100 + i * 62.5; // 200ms, 325ms, 450ms, 575ms
    grid.events.get("pointerdown")(pt(x, 45, 20 + i));
    assert.ok(editor.state.pendingDrawHits?.length > i, "optimistic draw hit appears synchronously on pointerdown");
    grid.events.get("pointerup")(pt(x, 45, 20 + i));
  }
  await flushAll();

  assert.equal(calls.length, 8, "4 clicks still send 4 down + 4 up pointer actions");
  const fullSceneCalls = calls.filter(c => c.scene === true);
  assert.equal(fullSceneCalls.length, 1, "only the final queued pointerup should request a full scene");
  assert.equal(calls.at(-1).scene, true);
  assert.equal(editor.state.pendingDrawHits?.length ?? 0, 0, "optimistic draw hits clear once scene settles");
});

test("problemStripData keeps stable object identity across hitsound edits on standard maps", async () => {
  const { editor } = await setupHarness();
  const first = editor.problemStripData();
  // Simulate a new scene object returned after placing a hitsound on an existing object
  editor.state.scene = makeScene({ laneRevision: 2 });
  const second = editor.problemStripData();
  assert.equal(second, first, "problemStripData must reuse cached object when objects/problems/viewport did not change");
});
