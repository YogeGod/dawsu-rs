import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("the current difficulty's bookmarks render in the inspector and ruler and can be edited", async () => {
  const calls = [];
  const elements = new Map();
  const makeElement = () => ({
    events: new Map(), children: [], dataset: {}, selectedIndex: 0,
    addEventListener(name, callback) { this.events.set(name, callback); },
    append(...children) { this.children.push(...children); },
    prepend(child) { this.children.unshift(child); },
    replaceChildren(...children) { this.children = children; },
    focus() {},
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
    window: { __TAURI__: {
      core: { invoke: async (name, params) => {
        calls.push(params.action);
        return { busy: false, hasDocument: false };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    Option: class {
      constructor(text, value) { this.text = text; this.value = value; }
    },
    canvas: { fillRect() {}, fillText(...args) { labels.push(args); } },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await runInContext(app, context);
  calls.length = 0;
  const setup = () => runInContext(
    'scene = { path: "Example.osu", bookmarks: [{time: 500, label: "Intro"}, {time: 1000, label: "Drop"}] };' +
    'view = {position: 750};' +
    'modal = (_kind, title, content, buttons) => { globalThis.bookmarkModal = { title, content, buttons }; };' +
    'closeModal = () => {};', context,
  );
  setup();
  runInContext("renderBookmarks(); drawBookmarks(canvas, scene.bookmarks, 0, 0.5, 400);", context);
  assert.deepEqual(get("bookmarks").children.map(option => option.text), ["Intro · 00:00.500", "Drop · 00:01.000"]);
  assert.deepEqual(labels.map(([label]) => label), ["Intro"]);

  get("add-bookmark").events.get("click")();
  assert.equal(context.bookmarkModal.title, "Add bookmark");
  context.bookmarkModal.content[1].children[1].value = "Break";
  context.bookmarkModal.buttons[1][1]();
  await runInContext("pending", context);
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), {
    kind: "bookmark_add", path: "Example.osu", time: 750, label: "Break",
  });

  setup();
  get("bookmarks").selectedIndex = 1;
  get("jump-bookmark").events.get("click")();
  await runInContext("pending", context);
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), { kind: "bookmark_jump", time: 1000 });

  runInContext('scene = { path: "Second.osu", bookmarks: [] }; renderBookmarks();', context);
  assert.equal(get("bookmarks").children.length, 0);
  assert.equal(get("rename-bookmark").disabled, true);
});
