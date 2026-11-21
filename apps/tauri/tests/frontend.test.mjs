import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";


async function frontend() {
  const listeners = new Map();
  const calls = [];
  const responses = [];
  const element = { addEventListener() {}, append() {}, prepend() {}, setAttribute() {}, dataset: {}, children: [] };
  const title = { setAttribute(name, value) { this[name] = value; } };
  const fileHead = {}, fileTail = {};
  const rulerEvents = new Map(), overviewEvents = new Map();
  const overview = { ...element, clientWidth: 100, getBoundingClientRect: () => ({ left: 10 }),
    setPointerCapture() {}, addEventListener(name, callback) { overviewEvents.set(name, callback); } };
  const gridEvents = new Map(), barEvents = new Map(), windowEvents = new Map();
  const cursors = { grid: {}, controller: {}, ruler: {} };
  const grid = { ...element, style: cursors.grid, addEventListener(name, callback) { gridEvents.set(name, callback); } };
  const controller = { ...element, style: cursors.controller, addEventListener(name, callback) { barEvents.set(name, callback); } };
  const resizeEvents = new Map();
  const properties = new Map();
  const arrange = { ...element, style: { setProperty(name, value) { properties.set(name, value); } } };
  const grip = { ...element, addEventListener(name, callback) { resizeEvents.set(name, callback); },
    setPointerCapture() {}, releasePointerCapture() {}, hasPointerCapture() {return true;},
    setAttribute(name, value) { properties.set(name, value); } };
  const ruler = { ...element, style: cursors.ruler, addEventListener: (name, callback) => rulerEvents.set(name, callback),
    setPointerCapture() {}, releasePointerCapture() {}, hasPointerCapture() { return true; },
    getBoundingClientRect: () => ({x:0, y:0, left:0, top:0, width:100, height:77}) };
  const context = createContext({
    document: {
      getElementById: (id) => ({"file-name":title, "file-name-head":fileHead, "file-name-tail":fileTail, grid, controller, ruler, overview, "editor-arrange":arrange, "waveform-resize":grip})[id] ?? element,
      createElement: () => ({ ...element }),
      querySelectorAll: () => [],
      addEventListener: (name, callback) => listeners.set(name, callback),
    },
    window: { addEventListener(name, callback) { windowEvents.set(name, callback); }, __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push({ name, args });
        return responses.shift() ?? { busy: false, hasDocument: ["view", "key"].includes(args.action?.kind), position: 1000 };
      } },
      event: { listen: (name, callback) => listeners.set(name, callback) },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  calls.length = 0;
  const evaluate = (code) => runInContext(code, context);
  return { listeners, rulerEvents, overviewEvents, gridEvents, barEvents, windowEvents, cursors, resizeEvents, properties, calls, evaluate, title, fileHead, fileTail, responses, settle: () => evaluate("pending") };
}

test("the resolved Open shortcut offers sibling difficulties from the current mapset", async () => {
  const { calls, evaluate, responses } = await frontend();
  evaluate(`
    show = () => {};
    let chooser;
    modal = (kind, title, content, buttons) => { chooser = { kind, title, content, buttons }; };
    scene = { path: "Expert.osu" };
    const elementFor = document.getElementById;
    const listOptions = [{text: "Expert", value: "Expert.osu"}, {text: "Hard", value: "Hard.osu"}];
    document.getElementById = id => id === "difficulty"
      ? { options: listOptions, getClientRects: () => [1] } : elementFor(id);
    document.createElement = () => ({
      addEventListener() {}, setAttribute() {}, focus() {}, append(...options) { this.options = options; }
    });
    globalThis.Option = function (text, value) { this.text = text; this.value = value; };
  `);
  responses.push({ shortcutOpen: true });
  await evaluate('act({ kind: "key", key: "o", ctrl: true })');
  assert.deepEqual(calls.map(call => call.name), ["editor_view"]);
  assert.equal(evaluate('chooser.title'), "Switch difficulty");
  assert.equal(evaluate('chooser.content[0].value'), "Expert.osu");
  evaluate('closeModal = () => {}; chooser.content[0].value = "Hard.osu"; chooser.buttons.at(-1)[1]()');
  await evaluate("pending");
  assert.equal(calls.at(-1).args.action.kind, "open");
  assert.equal(calls.at(-1).args.action.path, "Hard.osu");

  evaluate('scene = null');
  responses.push({ shortcutOpen: true }, {});
  await evaluate('act({ kind: "key", key: "o", ctrl: true })');
  assert.equal(calls.at(-1).name, "pick_map");
});

test("waveform resizing previews, cancels and resets without touching the document", async () => {
  const { resizeEvents, properties, calls } = await frontend();
  const action = (name, y, key) => resizeEvents.get(name)({button:0, pointerId:1, clientY:y, key,
    preventDefault() {}, stopPropagation() {}});
  assert.ok(resizeEvents.has("pointerdown"));
  action("pointerdown",100);
  action("pointermove",140);
  assert.equal(properties.get("--wave-height"),"124px");
  action("pointercancel",140);
  assert.equal(properties.get("--wave-height"),"84px");
  action("keydown",0,"Home");
  assert.equal(properties.get("aria-valuenow"),"0");
  action("dblclick",100);
  assert.equal(properties.get("--wave-height"),"84px");
  assert.equal(calls.length,0);
});

test("clicks near bookmark flags seek to the exact bookmark, leaving the lower waveform free", async () => {
  const { rulerEvents, calls, evaluate, settle } = await frontend();
  const click = async y => {
    evaluate('scene = { path:"Edited.osu", viewStart:0, zoom:.12, bookmarks:[{time:1025}] }; view = {duration:5000}');
    rulerEvents.get("pointerdown")({button:0, clientX:120, clientY:y});
    await settle();
  };
  await click(50);
  await click(70);
  assert.deepEqual(calls.filter(call=>call.args.action?.kind === "seek").map(call=>call.args.action.ms), [1025, 1000]);
});

test("native drop sends the first supported map through the editor open action", async () => {
  const { listeners, calls, settle } = await frontend();
  const drop = listeners.get("tauri://drag-drop");
  assert.equal(typeof drop, "function", "subscribe to native file drops");
  drop({ payload: { paths: ["E:\\notes.txt", "E:\\Map.OSU", "E:\\another.osz"] } });
  await settle();
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{
    name: "editor_view",
    args: { scene: true, action: { kind: "open", path: "E:\\Map.OSU" } },
  }]);
});

test("native drop ignores a path outside the supported map extensions", async () => {
  const { listeners, calls, settle } = await frontend();
  const drop = listeners.get("tauri://drag-drop");
  assert.equal(typeof drop, "function", "subscribe to native file drops");
  drop({ payload: { paths: ["E:\\sample.wav", "E:\\map.osu.txt"] } });
  await settle();
  assert.equal(calls.length, 0);
});


test("panning and zooming retain the scene and request only a status update", async () => {
  const { calls, evaluate } = await frontend();
  await evaluate('view = { busy: false, hasDocument: true, duration: 10_000 }; scene = { viewStart: 0, zoom: 1, rowHeight: 30, moments: [[1000]], waveform: [[0.5]] }; grid.clientWidth = 1000; changeView(1000, 2)');
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [{
    name: "editor_view",
    args: { scene: false, action: { kind: "view", start: 1000, zoom: 2, row_height: 30 } },
  }]);
  assert.equal(evaluate("scene.viewStart"), 1000);
  assert.equal(evaluate("scene.zoom"), 2);
  assert.equal(evaluate("scene.moments[0][0]"), 1000);
  assert.equal(evaluate("scene.waveform[0][0]"), 0.5);
});

test("the map's timing sections and waveform stay once loading ends with a frame that leaves them out", async () => {
  const { evaluate } = await frontend();
  const base = '{ path: "Map.osu", previewKey: 7, viewStart: 0, zoom: .1, rowHeight: 30, lanes: [], moments: [] }';
  // This harness has no lane DOM; only the data show() keeps is checked here.
  evaluate("renderScene = () => {}");
  // The editor sends the static map data once, in the first frame of the open map.
  evaluate(`show({ busy: true, hasDocument: true, duration: 4000, status: "Loading",
    scene: { ...${base}, sections: [[0, 500, 4]], waveform: [[0.5]], gameplay: null, combos: null } })`);
  evaluate(`show({ busy: false, hasDocument: true, duration: 4000, status: "Difficulty loaded", scene: ${base} })`);
  assert.equal(evaluate("JSON.stringify(scene.sections)"), "[[0,500,4]]");
  assert.equal(evaluate("scene.waveform[0][0]"), 0.5);
  assert.doesNotThrow(() => evaluate("sectionBars(scene.sections)"));
});

test("a reloaded screen asks the editor again for the map's static data it never got", async () => {
  const { evaluate, calls, responses } = await frontend();
  evaluate("renderScene = () => {}");
  const base = { path: "Map.osu", previewKey: 7, viewStart: 0, zoom: .1, rowHeight: 30, lanes: [], moments: [] };
  // The editor sent the static data to the page before the reload, so this frame leaves it out.
  responses.push({ busy: false, hasDocument: true, duration: 4000, scene: base },
    { busy: false, hasDocument: true, duration: 4000, scene: { ...base, sections: [[0, 500, 4]], waveform: [] } });
  await evaluate("act(null)");
  assert.deepEqual(JSON.parse(JSON.stringify(calls.map(call => call.args))),
    [{ scene: true, action: null }, { scene: true, action: null, fresh: true }]);
  assert.equal(evaluate("JSON.stringify(scene.sections)"), "[[0,500,4]]");
});

test("native drop sends an archive to Open without unpacking it in the frontend", async () => {
  const { listeners, calls, settle } = await frontend();
  listeners.get("tauri://drag-drop")({ payload: { paths: ["E:\\Map.OSZ"] } });
  await settle();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].args.action.kind, "open");
  assert.equal(calls[0].args.action.path, "E:\\Map.OSZ");
});

function keyEvent(key, code, modifiers = {}) {
  const event = {
    key, code, target: { closest: () => null, matches: () => false },
    preventDefault: () => { event.prevented = true; }, ...modifiers,
  };
  return event;
}

test("shortcut routing sends filtered and ordered lane rows with the timeline span", async () => {
  const { listeners, calls, evaluate, settle } = await frontend();
  evaluate(`view = { busy: false, hasDocument: true }; scene = { path: "Edited.osu", zoom: .5 };
    laneRows = ["Soft", 6, null, 2, 6, 9]; grid.clientWidth = 600`);
  listeners.get("keydown")(keyEvent("ArrowDown", "ArrowDown", { shiftKey: true }));
  await settle();
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0].args.action.lane_view)), {
    path: "Edited.osu", rows: [6, 2, 9], span: 1200,
  });
});

test("timeline shortcuts send both logical keys and physical codes to Rust", async () => {
  const { listeners, calls, evaluate, settle } = await frontend();
  evaluate('view = { busy: false, hasDocument: true }; scene = {}');
  const nextStep = keyEvent("ArrowRight", "ArrowRight", { ctrlKey: true });
  listeners.get("keydown")(nextStep);
  await settle();
  assert.equal(nextStep.prevented, true);
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0].args.action)), {
    kind: "key", key: "ArrowRight", code: "ArrowRight", ctrl: true, shift: false, alt: false,
  });

  const snap = keyEvent("!", "Digit1", { shiftKey: true });
  listeners.get("keydown")(snap);
  await settle();
  assert.equal(calls[1].args.action.code, "Digit1");
  assert.equal(calls[1].args.action.shift, true);
});

test("focused text controls keep their editing keys", async () => {
  const { listeners, calls, evaluate, settle } = await frontend();
  evaluate('view = { busy: false, hasDocument: true }; scene = {}');
  const event = keyEvent("z", "KeyZ", {
    ctrlKey: true, target: { closest: () => ({}), matches: () => true },
  });
  listeners.get("keydown")(event);
  await settle();
  assert.equal(calls.length, 0);
  assert.equal(event.prevented, undefined);
});

test("the status bar shows the open difficulty's file name, cut in the middle, with its full path", async () => {
  const { splitFileName } = await import("../ui/src/lib/statusbar.js");
  const first = splitFileName("C:\\Songs\\Mapset\\Artist - Title (Mapper) [Hard].osu");
  assert.equal(first.head + first.tail, "Artist - Title (Mapper) [Hard].osu");
  assert.equal(first.tail, "[Hard].osu");

  const second = splitFileName("/maps/another/Expert.osu");
  assert.equal(second.head + second.tail, "Expert.osu");

  assert.deepEqual(splitFileName(""), { head: "", tail: "" });
});

test("a changed lane count refreshes the scene in the background", async () => {
  const { calls, evaluate, responses } = await frontend();
  evaluate("view = { busy: false, hasDocument: true, laneCount: 12 };");
  responses.push(
    { busy: false, hasDocument: true, laneCount: 13 },
    { busy: false, hasDocument: true, laneCount: 13 },
  );
  await evaluate("update(null, false)");
  assert.deepEqual(calls.map(call => call.args.scene), [false, true]);
});

test("Tab follows the active bindings and remains focus navigation when unbound", async () => {
  const { listeners, calls, evaluate, settle } = await frontend();
  const handle = listeners.get("keydown");
  evaluate('view = {busy: false, hasDocument: true, tabRoutes: []}; scene = {};');
  const unbound = keyEvent("Tab", "Tab");
  handle(unbound);
  assert.equal(unbound.prevented, undefined);
  assert.equal(calls.length, 0);
  for (const ctrlKey of [false, true]) {
    evaluate(`view.tabRoutes = [[${ctrlKey}, false, false, false]];`);
    const bound = keyEvent("Tab", "Tab", {ctrlKey});
    handle(bound);
    await settle();
    assert.equal(bound.prevented, true);
    assert.equal(calls.at(-1).args.action.key, "Tab");
    assert.equal(calls.at(-1).args.action.ctrl, ctrlKey);
  }
  evaluate('view = {busy: false, hasDocument: true, tabRoutes: [[false, false, false, false]]}; scene = null;');
  const homeTab = keyEvent("Tab", "Tab");
  handle(homeTab);
  assert.equal(homeTab.prevented, undefined);
});

test("Home sends plain-letter rebinds to the Open and Help resolver", async () => {
  const { listeners, calls, evaluate, settle } = await frontend();
  evaluate('view = {busy: false, hasDocument: false}; scene = null;');
  const event = keyEvent("h", "KeyH");
  listeners.get("keydown")(event);
  await settle();
  assert.equal(calls.at(-1)?.args.action.key, "h");
});
test("cursors follow the current editing tool, Alt and cycle ruler geometry", async () => {
  const { evaluate, gridEvents, barEvents, rulerEvents, windowEvents, cursors } = await frontend();
  const mouse = { altKey: false, ctrlKey: false, metaKey: false };
  evaluate('view.tool = "Select"');
  gridEvents.get("pointerenter")(mouse);
  assert.equal(cursors.grid.cursor, "default");
  windowEvents.get("keydown")({ ...mouse, key: "Alt", altKey: true });
  assert.equal(cursors.grid.cursor, "var(--cursor-draw)");
  windowEvents.get("keydown")({ ...mouse, key: "Control", altKey: true, ctrlKey: true });
  assert.equal(cursors.grid.cursor, "default");
  windowEvents.get("keydown")({ ...mouse, key: "Meta", altKey: true, metaKey: true });
  assert.equal(cursors.grid.cursor, "default");
  windowEvents.get("keyup")(mouse);

  evaluate('view.tool = "Erase"; refreshToolCursor()');
  assert.equal(cursors.grid.cursor, "var(--cursor-erase)");
  gridEvents.get("pointerleave")();
  barEvents.get("pointerenter")(mouse);
  assert.equal(cursors.controller.cursor, "var(--cursor-erase)");
  evaluate('view.tool = "Grab"; refreshToolCursor()');
  assert.equal(cursors.controller.cursor, "pointer");
  windowEvents.get("keydown")({ ...mouse, key: "Alt", altKey: true });
  assert.equal(cursors.controller.cursor, "pointer", "Grab keeps the legacy hand");
  evaluate('view.tool = "Mute"; refreshToolCursor()');
  assert.equal(cursors.controller.cursor, "var(--cursor-draw)");
  const css = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");
  assert.match(css, /--cursor-draw:url\("studio-assets\/icons\/cursor-draw\.svg"\) 4 4,crosshair/);
  assert.match(css, /--cursor-erase:url\("studio-assets\/icons\/cursor-erase\.svg"\) 5 27,crosshair/);
  windowEvents.get("blur")();
  assert.equal(cursors.controller.cursor, "crosshair", "blur clears a held modifier");

  evaluate('scene = { viewStart: 0, zoom: 1 }; view.left = 30; view.right = 70;');
  for (const [clientX, clientY, cursor] of [[30,10,"ew-resize"],[71,10,"ew-resize"],[50,10,"pointer"],[50,40,"default"],[90,10,"default"]]) {
    rulerEvents.get("pointermove")({ clientX, clientY });
    assert.equal(cursors.ruler.cursor, cursor);
  }
  evaluate('rulerDrag = { mode: "right", id: 1, x: 70 };');
  rulerEvents.get("pointermove")({ pointerId: 1, clientX: 120, clientY: 40 });
  assert.equal(cursors.ruler.cursor, "ew-resize");
});

test("the stationary ruler cursor follows locator and viewport changes", async () => {
  const { evaluate, rulerEvents, cursors } = await frontend();
  evaluate('scene = { path: "Expert.osu", viewStart: 0, zoom: 1, rowHeight: 20 };');
  rulerEvents.get("pointermove")({ clientX: 30, clientY: 10 });
  assert.equal(cursors.ruler.cursor, "default");
  evaluate('show({ hasDocument: true, left: 30, right: 70 });');
  assert.equal(cursors.ruler.cursor, "ew-resize");
  evaluate('scene.viewStart = 20; show({ hasDocument: true, left: 30, right: 70 });');
  assert.equal(cursors.ruler.cursor, "pointer");
  rulerEvents.get("pointerleave")();
  evaluate('show({ hasDocument: true, left: 50, right: 70 });');
  assert.equal(cursors.ruler.cursor, "default");
});

test("overview drag follows the captured pointer and keeps only the latest queued position", async () => {
  const { calls, evaluate, overviewEvents, responses, settle } = await frontend();
  for (let i = 0; i < 3; i++) responses.push({ hasDocument: true, busy: false, duration: 10000 });
  evaluate('view = { busy: false, hasDocument: true, duration: 10000 }; scene = { path: "First.osu", viewStart: 0, zoom: .1, rowHeight: 30 }; grid.clientWidth = 200');
  const draw = (type, clientX, pointerId = 5, button = 0) => {
    overviewEvents.get(type)({ clientX, pointerId, button, preventDefault() {} });
  };
  const starts = () => calls.filter(call => call.args.action?.kind === "view").map(call => call.args.action.start);

  draw("pointerdown", 50);
  draw("pointermove", 65);
  draw("pointermove", 90, 6);
  draw("pointermove", 80);
  draw("pointerup", 90);
  await evaluate("overviewNavigation");
  await settle();
  assert.deepEqual(starts(), [3000, 7000]);
  assert.equal(evaluate("scene.viewStart"), 7000);

  draw("pointermove", 25);
  draw("pointerdown", 40, 5, 2);
  await settle();
  assert.deepEqual(starts(), [3000, 7000]);

  draw("pointerdown", 40);
  draw("pointermove", 95);
  draw("pointercancel", 95);
  await evaluate("overviewNavigation");
  assert.deepEqual(starts(), [3000, 7000, 2000]);
});

test("the L and R flags on the ruler drag their locator, even when both sit on the same time", async () => {
  const { rulerEvents, calls, evaluate, settle, cursors } = await frontend();
  evaluate('scene = { path: "Expert.osu", viewStart: 0, zoom: 1, rowHeight: 20 }; view = { busy: false, hasDocument: true, left: 50, right: 50, cycle: false, duration: 1000 }');
  // The left flag hangs right of its line, the right flag left of it.
  assert.equal(evaluate("rulerGestureAt(60, 6)"), "left");
  assert.equal(evaluate("rulerGestureAt(40, 6)"), "right");
  assert.equal(evaluate("rulerGestureAt(60, 18)"), "draw", "below the flags the band still draws a range");
  rulerEvents.get("pointermove")({ clientX: 60, clientY: 6 });
  assert.equal(cursors.ruler.cursor, "ew-resize");
  // A drag keeps where the flag was grabbed: the locator moves by the pointer's distance.
  rulerEvents.get("pointerdown")({ button: 0, pointerId: 3, clientX: 60, clientY: 6 });
  rulerEvents.get("pointermove")({ pointerId: 3, clientX: 40, clientY: 6 });
  assert.deepEqual([...evaluate("rulerDrag.preview")], [30, 50, false]);
  // L never passes R: dragged past it, it stops on R.
  rulerEvents.get("pointermove")({ pointerId: 3, clientX: 80, clientY: 6 });
  assert.deepEqual([...evaluate("rulerDrag.preview")], [50, 50, false]);
  rulerEvents.get("pointerup")({ pointerId: 3, clientX: 80, clientY: 6 });
  await settle();
  const sent = calls.filter(call => call.args.action?.kind === "ruler_locators").map(call => call.args.action);
  assert.deepEqual(sent.map(({ gesture, to }) => [gesture, to]), [["left", 50]]);
  // R never passes L either.
  assert.equal(evaluate('locatorDragTo({ mode: "right", left: 50, right: 60, x: 60, zoom: 1, duration: 1000 }, 20)'), 50);
  assert.equal(evaluate('locatorDragTo({ mode: "right", left: 50, right: 60, x: 60, zoom: 1, duration: 1000 }, 2000)'), 1000);
  // Near the canvas edge a flag flips so it stays on screen.
  evaluate('scene = { path: "Expert.osu", viewStart: 0, zoom: 1 }; view = { left: 95, right: 2 }');
  assert.equal(evaluate("rulerGestureAt(88, 6)"), "left");
  assert.equal(evaluate("rulerGestureAt(10, 6)"), "right");
});

test("Shift-click on an L or R flag jumps the playhead to that locator, as Shift-click on the old chips did", async () => {
  const { rulerEvents, calls, evaluate, settle } = await frontend();
  evaluate('scene = { path: "Expert.osu", viewStart: 0, zoom: 1, rowHeight: 20 }; view = { busy: false, hasDocument: true, left: 20, right: 70, cycle: false, duration: 1000 }');
  rulerEvents.get("pointerdown")({ button: 0, pointerId: 4, clientX: 64, clientY: 6, shiftKey: true });
  await settle();
  assert.deepEqual(JSON.parse(JSON.stringify(calls.map(call => call.args.action).filter(Boolean))),
    [{ kind: "locator", end: "right" }]);
  assert.equal(evaluate("rulerDrag"), null, "no drag starts");
});

test("on a short range the L and R flags hang outward, so neither covers the other", async () => {
  const { evaluate } = await frontend();
  evaluate('scene = { path: "Expert.osu", viewStart: 0, zoom: 1, rowHeight: 20 }; view = { left: 40, right: 50 }');
  const flags = evaluate("locatorFlags(100)");
  assert.deepEqual([flags.left.from, flags.right.from], [26, 50]);
  assert.equal(evaluate("rulerGestureAt(30, 6)"), "left");
  assert.equal(evaluate("rulerGestureAt(58, 6)"), "right");
  // A wider range keeps both flags inside it.
  evaluate("view = { left: 20, right: 70 }");
  const wide = evaluate("locatorFlags(100)");
  assert.deepEqual([wide.left.from, wide.right.from], [20, 56]);
});

test("Shift-click below the ruler band near a locator line does not jump to the locator", async () => {
  const { rulerEvents, calls, evaluate, settle } = await frontend();
  evaluate('scene = { path: "Expert.osu", viewStart: 0, zoom: 1, rowHeight: 20, bookmarks: [] }; view = { busy: false, hasDocument: true, left: 20, right: 70, cycle: false, duration: 1000 }');
  rulerEvents.get("pointerdown")({ button: 0, pointerId: 5, clientX: 71, clientY: 60, shiftKey: true });
  await settle();
  assert.deepEqual(calls.map(call => call.args.action?.kind).filter(Boolean), ["seek"]);
});

test("a plain click on the ruler band moves the playhead there, inside or outside the locator range", async () => {
  const { rulerEvents, calls, evaluate, settle } = await frontend();
  evaluate('scene = { path: "Expert.osu", viewStart: 0, zoom: 1, rowHeight: 20, bookmarks: [] }; view = { busy: false, hasDocument: true, left: 20, right: 70, cycle: false, duration: 1000 }');
  const click = (clientX, clientY, pointerId) => {
    rulerEvents.get("pointerdown")({ button: 0, pointerId, clientX, clientY });
    rulerEvents.get("pointerup")({ pointerId, clientX: clientX + 1, clientY });
  };
  click(45, 16, 1);
  click(90, 16, 2);
  click(30, 6, 3);
  await settle();
  const sent = JSON.parse(JSON.stringify(calls.map(call => call.args.action).filter(Boolean)));
  const seek = ms => ({ kind: "ruler_locators", path: "Expert.osu", gesture: "seek", from: ms, to: ms });
  assert.deepEqual(sent, [seek(45), seek(90), seek(30)], "no Cycle toggle and no locator change");
  assert.equal(evaluate("rulerDrag"), null);
});

test("dragging on the ruler band still draws, moves and drags the locators", async () => {
  const { rulerEvents, calls, evaluate, settle } = await frontend();
  evaluate('scene = { path: "Expert.osu", viewStart: 0, zoom: 1, rowHeight: 20, bookmarks: [] }; view = { busy: false, hasDocument: true, left: 20, right: 70, cycle: false, duration: 1000 }');
  const drag = (from, to, clientY, pointerId) => {
    rulerEvents.get("pointerdown")({ button: 0, pointerId, clientX: from, clientY });
    rulerEvents.get("pointermove")({ pointerId, clientX: to, clientY });
    rulerEvents.get("pointerup")({ pointerId, clientX: to, clientY });
  };
  drag(80, 95, 16, 1);
  drag(45, 55, 16, 2);
  drag(25, 35, 6, 3);
  await settle();
  const sent = calls.map(call => call.args.action).filter(Boolean).map(({ kind, gesture, from, to }) => [kind, gesture, from, to]);
  assert.deepEqual(sent, [["ruler_locators", "draw", 80, 95], ["ruler_locators", "move", 45, 55], ["ruler_locators", "left", 25, 30]]);
});
