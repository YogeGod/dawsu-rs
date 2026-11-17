import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const app = readFileSync(new URL("../dist/app.js", import.meta.url), "utf8");

test("folder dialog uses named native actions without sending paths", async () => {
  const elements = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {},
      addEventListener(name, callback) { this.events.set(name, callback); },
      append(...children) { this.children.push(...children); },
      prepend(child) { this.children.unshift(child); },
      replaceChildren(...children) { this.children = children; },
      showModal() { this.open = true; },
      close() { this.open = false; },
    };
  }
  const get = (id) => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const context = vm.createContext({
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push({ name, args });
        return { busy: false, hasDocument: false,
          hitsoundLibrary: name === "pick_hitsound_library" ? "C:/fixture/Library" : null };
      } },
      event: { listen() {} },
    } },
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
  });
  await vm.runInContext(app, context);
  calls.length = 0;
  vm.runInContext("scene = { path: 'C:/fixture/Hard.osu' }", context);
  const open = get("folders").events.get("click");
  assert.equal(typeof open, "function", "bind the Folders button");
  open();
  const buttons = get("dialog-actions").children;
  assert.deepEqual(buttons.map((button) => button.textContent),
    ["Cancel", "Open map folder", "Open project folder", "Choose Songs folder…", "Choose hitsound library…"]);
  buttons[1].events.get("click")();
  await vm.runInContext("pending", context);
  open();
  get("dialog-actions").children[2].events.get("click")();
  await vm.runInContext("pending", context);
  vm.runInContext("scene = null", context);
  open();
  assert.deepEqual(get("dialog-actions").children.map((button) => button.textContent),
    ["Cancel", "Choose Songs folder…", "Choose hitsound library…"]);
  get("dialog-actions").children[1].events.get("click")();
  await vm.runInContext("pending", context);
  open();
  get("dialog-actions").children[2].events.get("click")();
  await vm.runInContext("pending", context);
  open();
  const library = get("dialog-actions").children.find(button => button.textContent === "Open hitsound library");
  library.events.get("click")();
  await vm.runInContext("pending", context);
  assert.deepEqual(JSON.parse(JSON.stringify(calls)), [
    { name: "open_editor_folder", args: { folder: "map" } },
    { name: "open_editor_folder", args: { folder: "project" } },
    { name: "pick_songs_folder" },
    { name: "pick_hitsound_library" },
    { name: "open_editor_folder", args: { folder: "library" } },
  ]);
});
