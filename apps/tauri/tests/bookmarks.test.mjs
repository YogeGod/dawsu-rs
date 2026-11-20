import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

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
          return { busy: false, hasDocument: false };
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
  await runInContext(app, context);
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
