import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor, readEditorSource } from "./editor-runtime.mjs";

const app = readEditorSource();

async function frontend() {
  const calls = [];
  const handed = [];
  const elements = new Map();
  const makeElement = () => ({
    events: new Map(), children: [], dataset: {}, selectedIndex: 0,
    addEventListener(name, callback) { this.events.set(name, callback); },
    append(...children) { this.children.push(...children); },
    prepend(child) { this.children.unshift(child); },
    replaceChildren(...children) { this.children = children; },
    focus() {}, setAttribute() {},
    classList: { add() {}, remove() {}, toggle() {} },
  });
  const get = id => {
    if (!elements.has(id)) elements.set(id, makeElement());
    return elements.get(id);
  };
  const labels = [];
  const context = createContext({
    document: {
      getElementById: get,
      createElement: makeElement,
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: {
      __TAURI__: {
        core: { invoke: async (name, params) => {
          calls.push(params.action);
          // A test can answer with a view of its own; otherwise no difficulty is open.
          return context.answer?.() ?? { busy: false, hasDocument: false };
        } },
        event: { listen() {} },
      },
      dawsuUi: { inspector: model => handed.push(model), update() {}, lanes() {} },
    },
    ResizeObserver: class {
      observe() {}
    },
    Option: class {
      constructor(text, value) { this.text = text; this.value = value; }
    },
    canvas: { fillRect() {}, fillText(...args) { labels.push(args); } },
    setInterval() {},
    requestAnimationFrame() {},
    console,
  });
  await bootEditor(context);
  calls.length = 0;
  const evaluate = code => runInContext(code, context);
  const sent = async () => {
    await evaluate("pending");
    return calls.splice(0).filter(Boolean).map(action => JSON.parse(JSON.stringify(action)));
  };
  const model = () => JSON.parse(evaluate("JSON.stringify(inspectorModel())"));
  return { context, evaluate, sent, model, handed, labels };
}

const setup = 'scene = { path: "Example.osu", bookmarks: [{time: 500, label: "Intro"}, {time: 1000, label: "Drop"}],' +
  ' history: [], historyPosition: 0 };' +
  'view = { busy: false, position: 750 };' +
  'modal = (_kind, title, content, buttons) => { globalThis.bookmarkModal = { title, content, buttons }; };' +
  'closeModal = () => {};';

test("the current difficulty's bookmarks reach the Inspector and the ruler and can be edited", async () => {
  const { context, evaluate, sent, model, labels } = await frontend();
  evaluate(setup);
  evaluate("drawBookmarks(canvas, scene.bookmarks, 0, 0.5, 400);");
  assert.deepEqual(model().bookmarks, [
    { time: 500, label: "Intro", clock: "00:00.500" }, { time: 1000, label: "Drop", clock: "00:01.000" },
  ]);
  assert.equal(model().bookmark, null);
  assert.deepEqual(labels.map(([label]) => label), ["Intro"]);

  evaluate('inspectorAction("bookmark", "add")');
  assert.equal(context.bookmarkModal.title, "Add bookmark");
  context.bookmarkModal.content[1].children[1].value = "Break";
  context.bookmarkModal.buttons[1][1]();
  assert.deepEqual(await sent(), [{ kind: "bookmark_add", path: "Example.osu", time: 750, label: "Break" }]);

  // Each update with no difficulty clears the scene; the fakes put it back before each step.
  evaluate(setup);
  // From here the editor answers with the playhead on the picked bookmark, as after its jump.
  evaluate("renderScene = () => {}; renderPanelLayout = () => {};");
  context.answer = () => ({ busy: false, hasDocument: true, position: 1000, scene: runInContext("scene", context) });
  // Clicking a row picks it and goes to it, as picking then Go to did.
  evaluate('inspectorAction("pickBookmark", 1000)');
  assert.equal(model().bookmark, 1000);
  assert.deepEqual(await sent(), [{ kind: "bookmark_jump", time: 1000 }]);
  evaluate(setup);
  assert.equal(model().bookmark, 1000, "the pick outlives the update");
  evaluate('inspectorAction("bookmark", "goTo")');
  assert.deepEqual(await sent(), [{ kind: "bookmark_jump", time: 1000 }]);
  evaluate(setup);
  evaluate('inspectorAction("bookmark", "rename")');
  assert.equal(context.bookmarkModal.title, "Rename bookmark");
  assert.equal(context.bookmarkModal.content[1].children[1].value, "Drop");
  context.bookmarkModal.buttons[1][1]();
  assert.deepEqual(await sent(), [{ kind: "bookmark_edit", path: "Example.osu", time: 1000, label: "Drop" }]);
  evaluate(setup);
  evaluate('inspectorAction("bookmark", "remove")');
  assert.deepEqual(await sent(), [{ kind: "bookmark_edit", path: "Example.osu", time: 1000, label: null }]);

  // Another difficulty drops the pick; its bookmarks replace the list.
  evaluate('scene = { path: "Second.osu", bookmarks: [{ time: 1000, label: "" }], history: [], historyPosition: 0 };');
  assert.equal(model().bookmark, null);
  evaluate('inspectorAction("bookmark", "rename"); inspectorAction("bookmark", "remove")');
  assert.deepEqual(await sent(), []);
});

test("a pick survives updates while its bookmark stays, and goes when it is removed", async () => {
  const { evaluate, model, handed } = await frontend();
  evaluate(setup);
  handed.length = 0;
  evaluate('inspectorAction("pickBookmark", 500)');
  // No show() follows a pick, so the Inspector gets it at once.
  assert.equal(handed.at(-1).bookmark, 500);
  evaluate('scene = { ...scene, bookmarks: [{ time: 500, label: "Intro, renamed" }, { time: 2000, label: "" }] };');
  assert.equal(model().bookmark, 500);
  evaluate('scene = { ...scene, bookmarks: [{ time: 2000, label: "" }] };');
  assert.equal(model().bookmark, null);
});

test("busy stops the bookmark actions; History steps go to their position as before", async () => {
  const { evaluate, sent, model } = await frontend();
  evaluate(setup + 'view = { busy: true, position: 0 };');
  for (const call of ['"pickBookmark", 500', '"bookmark", "goTo"', '"bookmark", "remove"']) {
    assert.equal(evaluate(`inspectorAction(${call})`), false, call);
  }
  assert.deepEqual(await sent(), []);
  evaluate('scene = { ...scene, history: ["Draw hit", "Move 4 hits"], historyPosition: 1 }; view = { busy: false };');
  assert.deepEqual([model().history, model().historyPosition], [["Opened difficulty", "Draw hit", "Move 4 hits"], 1]);
  evaluate('inspectorAction("history", 0)');
  assert.deepEqual(await sent(), [{ kind: "history", position: 0 }]);
});

test("the bookmark and history lists stay the same arrays while nothing in them changes", async () => {
  const { evaluate } = await frontend();
  evaluate(setup);
  assert.equal(evaluate("inspectorModel().bookmarks === inspectorModel().bookmarks"), true);
  assert.equal(evaluate("inspectorModel().history === inspectorModel().history"), true);
  evaluate('scene = { ...scene, bookmarks: [{ time: 500, label: "Intro" }] };');
  assert.equal(evaluate("inspectorModel().bookmarks.length"), 1);
  // No difficulty: nothing to list.
  evaluate("scene = null;");
  assert.deepEqual([evaluate("inspectorModel().bookmarks.length"), evaluate("inspectorModel().history.length")], [0, 0]);
});

// show() with the scene parts the fakes cannot draw left out.
const showView = (evaluate, more) => evaluate(`show({ busy: false, hasDocument: true, position: 0, duration: 9000,
  scene: { path: "A.osu", bookmarks: [{ time: 500, label: "Intro" }, { time: 1000, label: "Drop" }], history: [],
    historyPosition: 0 }, ...${JSON.stringify(more ?? {})} })`);
async function shown() {
  const harness = await frontend();
  harness.evaluate("renderScene = () => {}; renderPanelLayout = () => {};");
  return harness;
}

test("the pick waits for its jump, then goes once the playhead moves off its bookmark", async () => {
  const { evaluate, model } = await shown();
  showView(evaluate, { position: 200 });
  evaluate('inspectorAction("pickBookmark", 1000)');
  // The jump's own update can still carry the old position (the audio thread had not seeked yet).
  showView(evaluate, { position: 200 });
  assert.equal(model().bookmark, 1000, "an update before the jump lands keeps the pick");
  showView(evaluate, { position: 1000 });
  assert.equal(model().bookmark, 1000);
  showView(evaluate, { position: 1000 });
  assert.equal(model().bookmark, 1000, "a playhead resting on it keeps it");
  // A seek, playback or click elsewhere that moves the playhead clears it, even back to where it was.
  showView(evaluate, { position: 200 });
  assert.equal(model().bookmark, null);
  for (const kind of ["goTo", "rename", "remove"]) assert.equal(evaluate(`inspectorAction("bookmark", "${kind}")`), false, kind);

  evaluate('inspectorAction("pickBookmark", 500)');
  showView(evaluate, { position: 500 });
  showView(evaluate, { position: 512.5, playing: true });
  assert.equal(model().bookmark, null, "playback off the bookmark clears it");
});

test("with audio open the position arrives an update late; the pick still waits for its jump, but not for ever", async () => {
  const { evaluate, model } = await shown();
  evaluate("let now = 0; Date.now = () => now;");
  // Seen in the app: a seek's position shows only in the next update, here the one that also steps.
  showView(evaluate, { position: 1000, bookmarkStep: 1000 });
  showView(evaluate, { position: 200 });
  showView(evaluate, { position: 300, bookmarkStep: 500 });
  assert.equal(model().bookmark, 500, "neither the old nor the stale position clears a pick that is on its way");
  showView(evaluate, { position: 500 });
  assert.equal(model().bookmark, 500);
  // A jump that never shows up (another seek overtook it) stops waiting after a moment.
  evaluate('inspectorAction("pickBookmark", 1000)');
  evaluate("now = 400;");
  showView(evaluate, { position: 700 });
  assert.equal(model().bookmark, 1000);
  evaluate("now = 1500;");
  showView(evaluate, { position: 700 });
  assert.equal(model().bookmark, null);
});

test("the pick follows Previous and Next to the bookmark they jumped to, from the buttons, menus or keys", async () => {
  const { evaluate, model } = await shown();
  showView(evaluate, { position: 750 });
  assert.equal(model().bookmark, null);
  // The editor names the bookmark a step jumped to once, in the update that ran it.
  showView(evaluate, { position: 1000, bookmarkStep: 1000 });
  assert.equal(model().bookmark, 1000);
  showView(evaluate, { position: 500, bookmarkStep: 500 });
  assert.equal(model().bookmark, 500);
  // Even when that update still has the old position.
  showView(evaluate, { position: 500, bookmarkStep: 1000 });
  assert.equal(model().bookmark, 1000);
  showView(evaluate, { position: 1000 });
  assert.equal(model().bookmark, 1000);
  // A bookmark past the song's end is jumped to at the end.
  showView(evaluate, { position: 9000, bookmarkStep: 12000,
    scene: { path: "A.osu", bookmarks: [{ time: 500, label: "Intro" }, { time: 12000, label: "" }], history: [], historyPosition: 0 } });
  assert.equal(model().bookmark, 12000);
});

test("the pick drops when the difficulty changes, also on the way back (A, B, A)", async () => {
  const { evaluate, model } = await shown();
  showView(evaluate, { position: 1000, bookmarkStep: 1000 });
  assert.equal(model().bookmark, 1000);
  showView(evaluate, { position: 1000, scene: { path: "B.osu", bookmarks: [{ time: 1000, label: "" }], history: [], historyPosition: 0 } });
  assert.equal(model().bookmark, null);
  showView(evaluate, { position: 1000 });
  assert.equal(model().bookmark, null, "back on A at the same time, the old pick stays gone");
  // Closing the map drops it too.
  showView(evaluate, { position: 1000, bookmarkStep: 1000 });
  evaluate("show({ busy: false, hasDocument: false, position: 0 })");
  showView(evaluate, { position: 1000 });
  assert.equal(model().bookmark, null);
});

test("an update that is asked again keeps the bookmark step of the first answer", async () => {
  const { evaluate } = await shown();
  assert.match(evaluate("update.toString()"), /next\.bookmarkStep \?\?= step/);
});

test("busy stops Add and Rename as well", async () => {
  const { evaluate, sent } = await frontend();
  evaluate(setup + 'view = { busy: true, position: 0 }; editBookmark = () => { throw new Error("opened"); };');
  for (const call of ['"bookmark", "add"', '"bookmark", "rename"']) assert.equal(evaluate(`inspectorAction(${call})`), false, call);
  assert.deepEqual(await sent(), []);
});
