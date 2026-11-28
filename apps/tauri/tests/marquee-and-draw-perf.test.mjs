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
    difficulties: ["PerfMap.osu"],
    waveform: [],
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
    laneLayout: { folder_order: ["soft", "normal", "drum"] },
    ...overrides,
  };
}

async function setupHarness({ deferInvoke = false } = {}) {
  const controls = new Map();
  const calls = [];
  const resolvers = [];
  let failNextInvoke = null;
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
            if (_name === "editor_waveform") return Promise.resolve(new Float32Array(0));
            // The first view fetches the shortcuts (Slice 6); not a pointer or scene IPC.
            if (_name === "editor_shortcuts") return Promise.resolve(null);
            calls.push(args ? JSON.parse(JSON.stringify(args)) : undefined);
            const buildReply = () => ({
              busy: false,
              hasDocument: true,
              tool: editor.state.view.tool,
              duration: 10_000,
              playing: false,
              position: 0,
              ...(args?.scene ? {
                scene: makeScene({
                  viewStart: editor.state.scene?.viewStart ?? 0,
                  zoom: editor.state.scene?.zoom ?? 0.5,
                }),
              } : {}),
            });
            if (failNextInvoke) {
              const err = failNextInvoke;
              failNextInvoke = null;
              return Promise.reject(err);
            }
            if (deferInvoke) {
              return new Promise((resolve, reject) => {
                resolvers.push({ resolve: () => resolve(buildReply()), reject });
              });
            }
            return Promise.resolve(buildReply());
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
    scroll: get("scroll"),
    calls,
    resolvers,
    flushOne: async () => {
      for (let i = 0; i < 10 && resolvers.length === 0; i++) {
        await new Promise(r => setTimeout(r, 0));
      }
      assert.ok(resolvers.length > 0, "expected a pending invoke to flush");
      resolvers.shift().resolve();
      await new Promise(r => setTimeout(r, 0));
    },
    rejectOne: async (err = new Error("ipc failed")) => {
      for (let i = 0; i < 10 && resolvers.length === 0; i++) {
        await new Promise(r => setTimeout(r, 0));
      }
      assert.ok(resolvers.length > 0, "expected a pending invoke to reject");
      resolvers.shift().reject(err);
      await new Promise(r => setTimeout(r, 0));
    },
    setFailNextInvoke: (err) => { failNextInvoke = err; },
    flushAll: async () => {
      for (let i = 0; i < 200 && (resolvers.length > 0 || editor.state.requests > 0); i++) {
        while (resolvers.length) resolvers.shift().resolve();
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

test("Draw optimistic hit persists across slim pointerdown, idle polls, and overlapping clicks until authoritative scene arrives", async () => {
  const { editor, grid, calls, resolvers, flushOne, rejectOne, flushAll } = await setupHarness({ deferInvoke: true });
  editor.state.view.tool = "Draw";

  // 1. Normal click timing: pointerdown (scene: false) completes BEFORE pointerup fires.
  //    x = 100 -> 200 ms -> snaps to 250 ms; row 9 is visual row 1 -> y = 45.
  grid.events.get("pointerdown")(pt(100, 45, 51));
  assert.equal(editor.state.pendingDrawHits.length, 1);
  assert.equal(editor.state.pendingDrawHits[0].time, 250);

  // Flush slim pointerdown while mouse button is still held down.
  await flushOne();
  await editor.state.pending;
  assert.equal(editor.state.requests, 0);
  assert.equal(calls.at(-1).scene, false);
  assert.equal(
    editor.state.pendingDrawHits.length,
    1,
    "optimistic draw hit must NOT disappear when slim pointerdown (scene: false) finishes",
  );

  // Interleaved scene: false background poll while mouse is held / before scene arrives.
  const pollPromise = editor.act(null, false);
  await flushOne();
  await pollPromise;
  assert.equal(
    editor.state.pendingDrawHits.length,
    1,
    "interleaved scene: false poll must NOT clear optimistic draw hit",
  );

  // Release pointer -> sends scene: true; hit remains visible while IPC is in flight, clears on arrival.
  grid.events.get("pointerup")(pt(100, 45, 51));
  assert.equal(editor.state.pendingDrawHits.length, 1, "hit stays queued while pointerup scene is in flight");
  await flushOne();
  await editor.state.pending;
  assert.equal(calls.at(-1).scene, true);
  assert.equal(editor.state.pendingDrawHits.length, 0, "hit clears when authoritative scene arrives");

  // 2. Overlapping rapid clicks: click #1's scene arrival must prune ONLY click #1, keeping click #2 alive.
  grid.events.get("pointerdown")(pt(100, 45, 52)); // 250 ms
  grid.events.get("pointerup")(pt(100, 45, 52));
  await flushOne(); // resolve down1 (scene: false)
  assert.equal(resolvers.length, 1, "up1 (scene: true) should now be in flight");

  // Click #2 starts BEFORE up1 finishes: x = 250 -> 500 ms.
  grid.events.get("pointerdown")(pt(250, 45, 53));
  assert.deepEqual(editor.state.pendingDrawHits.map(h => h.time), [250, 500]);

  // up1's scene arrives: only click #1 (250 ms) is covered by up1's enqueue seq; click #2 (500 ms) must stay!
  await flushOne();
  assert.deepEqual(
    editor.state.pendingDrawHits.map(h => h.time),
    [500],
    "earlier click scene arrival must preserve newer in-flight click's optimistic hit",
  );

  grid.events.get("pointerup")(pt(250, 45, 53));
  await flushAll();
  assert.equal(editor.state.pendingDrawHits.length, 0);

  // 3. pointercancel removes only the cancelled stroke's hit while keeping earlier committed pending hits.
  grid.events.get("pointerdown")(pt(100, 45, 54)); // 250 ms
  grid.events.get("pointerup")(pt(100, 45, 54));
  await flushOne(); // resolve down (scene: false), keep up (scene: true) in flight

  grid.events.get("pointerdown")(pt(250, 45, 55)); // 500 ms
  assert.deepEqual(editor.state.pendingDrawHits.map(h => h.time), [250, 500]);
  grid.events.get("pointercancel")(pt(250, 45, 55));
  assert.deepEqual(
    editor.state.pendingDrawHits.map(h => h.time),
    [250],
    "pointercancel removes only the cancelled stroke's optimistic hit",
  );
  await flushAll();
  assert.equal(editor.state.pendingDrawHits.length, 0);

  // 4. IPC failure cleanup: rejecting pointerup clears orphaned optimistic hits once idle with no active drag.
  grid.events.get("pointerdown")(pt(100, 45, 56));
  grid.events.get("pointerup")(pt(100, 45, 56));
  assert.equal(editor.state.pendingDrawHits.length, 1);
  await flushOne(); // down succeeds
  await rejectOne(new Error("simulated ipc failure")); // up rejects
  await editor.state.pending;
  assert.equal(editor.state.requests, 0);
  assert.equal(editor.state.drag, null);
  assert.equal(editor.state.pendingDrawHits.length, 0, "failed IPC cleans up pendingDrawHits when idle");
});

test("problemStripData keeps stable object identity across hitsound edits on standard maps", async () => {
  const { editor } = await setupHarness();
  const first = editor.problemStripData();
  // Simulate a new scene object returned after placing a hitsound on an existing object
  editor.state.scene = makeScene({ laneRevision: 2 });
  const second = editor.problemStripData();
  assert.equal(second, first, "problemStripData must reuse cached object when objects/problems/viewport did not change");
});

test("Select marquee allows vertical and horizontal wheel scrolling in DAW and osu! presets", async () => {
  const { editor, grid, scroll, calls } = await setupHarness();
  scroll.clientHeight = 120;
  scroll.scrollHeight = 360;
  const wheel = scroll.events.get("wheel");
  const wh = (extra) => wheel({
    clientX: 300, clientY: 80, ctrlKey: false, shiftKey: false, altKey: false,
    deltaX: 0, deltaY: 0, preventDefault() {}, ...extra,
  });

  // Start marquee on empty space at x=100 (200ms) row 0 ("soft", y=10)
  grid.events.get("pointerdown")(pt(100, 10, 31));
  // Bare vertical scroll while dragging marquee in DAW preset
  wh({ deltaY: 90 });
  assert.equal(scroll.scrollTop, 90, "vertical wheel scrolls during marquee in DAW preset");
  // osu! preset vertical scroll during marquee must scroll vertically instead of being swallowed
  editor.state.view.keyPreset = "osu!";
  wh({ deltaY: 30 });
  assert.equal(scroll.scrollTop, 120, "vertical wheel scrolls during marquee in osu! preset");
  // Horizontal wheel pan via Shift+wheel and trackpad deltaX
  wh({ shiftKey: true, deltaY: 200 });
  await editor.state.pending;
  const afterShiftPan = editor.state.scene.viewStart;
  assert.ok(afterShiftPan > 0, "Shift+wheel pans horizontally during marquee");
  wh({ deltaX: 100 });
  await editor.state.pending;
  assert.ok(editor.state.scene.viewStart > afterShiftPan, "deltaX pans horizontally during marquee");
  assert.ok(editor.state.drag.lastPoint.ms > 200, "lastPoint updates against panned viewStart");

  // Finish marquee at pointer x=400, localY=50 (scrollTop=120 => y=170 => visual row 5 = lane 4)
  grid.events.get("pointerup")(pt(400, 50, 31));
  await editor.state.pending;

  const commit = calls.filter(c => c.action?.kind === "select_box").at(-1);
  assert.ok(commit, "marquee commits select_box after scrolling");
  assert.equal(commit.action.from_ms, 200);
  assert.equal(commit.action.to_ms, editor.state.scene.viewStart + 400 / 0.5);
  assert.deepEqual(commit.action.rows, [9, 10, 3, 4]);
});

test("Samples view pointer events attach visible_rows matching visual lane order", async () => {
  const { editor, grid, calls } = await setupHarness();
  editor.state.view.drumView = null;
  // Hit at 1000 ms is on lane 3 (visual row 4 -> y=135 at rowHeight=30).
  grid.events.get("pointerdown")(pt(500, 135, 41));
  await editor.state.pending;
  const down = calls.find(c => c.action?.kind === "pointer" && c.action.phase === "down");
  assert.ok(down, "pointer down sent");
  assert.deepEqual([...down.action.visible_rows], [9, 10, 3, 4]);
});
