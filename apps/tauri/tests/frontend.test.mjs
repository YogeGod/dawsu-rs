import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

async function frontend() {
  const listeners = new Map();
  const calls = [];
  const responses = [];
  const element = { addEventListener() {}, append() {}, prepend() {}, dataset: {}, children: [] };
  const title = { setAttribute(name, value) { this[name] = value; } };
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
    getBoundingClientRect: () => ({x:0, y:0, left:0, top:0, width:100, height:77}) };
  const context = createContext({
    document: {
      getElementById: (id) => ({title, grid, controller, ruler, overview, "editor-arrange":arrange, "waveform-resize":grip})[id] ?? element,
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
  await runInContext(app, context);
  calls.length = 0;
  const evaluate = (code) => runInContext(code, context);
  return { listeners, rulerEvents, overviewEvents, gridEvents, barEvents, windowEvents, cursors, resizeEvents, properties, calls, evaluate, title, responses, settle: () => evaluate("pending") };
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
  assert.equal(properties.get("--wave-height"),"96px");
  action("pointercancel",140);
  assert.equal(properties.get("--wave-height"),"56px");
  action("keydown",0,"Home");
  assert.equal(properties.get("aria-valuenow"),"0");
  action("dblclick",100);
  assert.equal(properties.get("--wave-height"),"56px");
  assert.equal(calls.length,0);
});

test("clicks near bookmark flags seek to the exact bookmark, leaving the lower waveform free", async () => {
  const { rulerEvents, calls, evaluate, settle } = await frontend();
  const click = async y => {
    evaluate('scene = { path:"Edited.osu", viewStart:0, zoom:.12, bookmarks:[{time:1025}] }; view = {duration:5000}');
    rulerEvents.get("pointerdown")({button:0, clientX:120, clientY:y});
    await settle();
  };
  await click(30);
  await click(65);
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

test("the top bar shows the active difficulty filename and its full path", async () => {
  const { evaluate, title } = await frontend();
  evaluate('scene = { path: "C:\\\\Songs\\\\Mapset\\\\Hard.osu", title: "Map title" }; show({ hasDocument: true, dirty: false })');
  assert.equal(title.textContent, "Hard.osu");
  assert.equal(title.title, "C:\\Songs\\Mapset\\Hard.osu");

  evaluate('scene = { path: "/maps/another/Expert.osu" }; show({ hasDocument: true, dirty: true })');
  assert.equal(title.textContent, "● Expert.osu");
  assert.equal(title.title, "/maps/another/Expert.osu");

  evaluate("show({ hasDocument: false })");
  assert.equal(title.textContent, "");
  assert.equal(title.title, "");
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
  assert.match(cursors.grid.cursor, /cursor-draw\.svg.*4 4, crosshair/);
  windowEvents.get("keydown")({ ...mouse, key: "Control", altKey: true, ctrlKey: true });
  assert.equal(cursors.grid.cursor, "default");
  windowEvents.get("keydown")({ ...mouse, key: "Meta", altKey: true, metaKey: true });
  assert.equal(cursors.grid.cursor, "default");
  windowEvents.get("keyup")(mouse);

  evaluate('view.tool = "Erase"; refreshToolCursor()');
  assert.match(cursors.grid.cursor, /cursor-erase\.svg.*5 27, crosshair/);
  gridEvents.get("pointerleave")();
  barEvents.get("pointerenter")(mouse);
  assert.match(cursors.controller.cursor, /cursor-erase\.svg/);
  evaluate('view.tool = "Grab"; refreshToolCursor()');
  assert.equal(cursors.controller.cursor, "pointer");
  windowEvents.get("keydown")({ ...mouse, key: "Alt", altKey: true });
  assert.equal(cursors.controller.cursor, "pointer", "Grab keeps the legacy hand");
  evaluate('view.tool = "Mute"; refreshToolCursor()');
  assert.match(cursors.controller.cursor, /cursor-draw\.svg/);
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
