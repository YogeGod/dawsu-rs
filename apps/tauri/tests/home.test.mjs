import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("Home asks before leaving an edited difficulty and preserves recent and resume actions", async () => {
  const elements = new Map();
  const calls = [];
  function element() {
    return {
      listeners: new Map(), children: [], dataset: {}, style: {}, classList: { toggle() {} },
      addEventListener(name, listener) { this.listeners.set(name, listener); },
      setAttribute() {}, append(...children) { this.children.push(...children); },
      prepend() {}, replaceChildren(...children) { this.children = children; },
      showModal() { this.open = true; }, close() { this.open = false; },
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const scene = {path: "edited.osu", lanes: []};
  const status = {busy: false, hasDocument: true, pendingHome: false, showHome: false, recent: ["recent.osu"]};
  const context = createContext({
    document: {
      getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push(JSON.parse(JSON.stringify(args?.action ?? null)));
        switch (args?.action?.kind) {
          case "home": status.pendingHome = true; break;
          case "home_choice":
            status.pendingHome = false;
            status.showHome = args.action.choice === "discard";
            break;
          case "resume": status.showHome = false; break;
          case "open": status.showHome = false; break;
        }
        return {...status, scene: args?.scene && !status.showHome ? scene : null};
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
    console,
  });
  await runInContext(app.replace(/\n(?=act\(null\);\s*$)/, "\n// "), context);
  runInContext('renderScene = () => {}; renderSelection = () => {};', context);
  const click = (node) => node.listeners.get("click")();
  const reply = (label) => {
    const button = get("dialog-actions").children.find(child => child.textContent === label);
    assert.ok(button, label + " choice");
    click(button);
  };

  // File › Start screen runs goHome().
  const goHome = () => runInContext("goHome()", context);
  goHome();
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), {kind: "home"});
  assert.equal(get("home").hidden, true);
  assert.equal(get("dialog").open, true);
  assert.equal(get("dialog-title").textContent, "Save changes before going Home?");
  reply("Cancel");
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), {kind: "home_choice", choice: "cancel"});
  assert.equal(get("workspace").hidden, false);

  goHome();
  await runInContext("pending", context);
  reply("Discard");
  await runInContext("pending", context);
  assert.equal(get("home").hidden, false);
  assert.equal(get("workspace").hidden, true);
  runInContext("startScreenAction('resume')", context);
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), {kind: "resume"});
  assert.equal(get("workspace").hidden, false);

  goHome();
  await runInContext("pending", context);
  reply("Discard");
  await runInContext("pending", context);
  runInContext("startScreenAction('openRecent', 'recent.osu')", context);
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), {kind: "open", path: "recent.osu"});
});