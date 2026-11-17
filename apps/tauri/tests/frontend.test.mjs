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
  const title = {};
  const context = createContext({
    document: {
      getElementById: (id) => id === "title" ? title : element,
      createElement: () => ({ ...element }),
      querySelectorAll: () => [],
      addEventListener: (name, callback) => listeners.set(name, callback),
    },
    window: { __TAURI__: {
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
  return { listeners, calls, evaluate, title, responses, settle: () => evaluate("pending") };
}

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