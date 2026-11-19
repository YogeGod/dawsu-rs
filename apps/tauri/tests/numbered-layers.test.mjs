import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("the plus button adds a numbered layer to its sample family", async () => {
  const controls = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, classList: { toggle() {}, add() {} },
      style: { setProperty(name, value) { this[name] = value; } },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute(name, value) { this[name] = value; },
      append(...children) { this.children.push(...children); },
      prepend() {},
      replaceChildren(...children) { this.children = children; },
    };
  }
  function get(id) {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  }
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(JSON.parse(JSON.stringify(args)));
        return { busy: false, hasDocument: true, position: 0 };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await runInContext(app, context);
  calls.length = 0;
  runInContext('scene = {rowHeight: 28, lanes: [{key: "file", name: "custom.wav"}, {key: "clap", name: "soft-hitclap", canAddLayer: true}]}; renderLanes();', context);
  const [custom, family] = get("lanes").children;
  assert.equal(custom.children.some(button => button.textContent === "+"), false);
  const plus = family.children.find(button => button.textContent === "+");
  assert.equal(plus?.["aria-label"], "Add numbered soft-hitclap layer");
  plus.events.get("click")();
  await runInContext("pending", context);
  assert.deepEqual(calls.pop(), { scene: true, action: {kind: "lane", row: 1, operation: "add_layer"} });
});
