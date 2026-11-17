import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

async function launch(preferences) {
  const elements = new Map();
  const options = [];
  const actions = [];
  const createElement = () => ({
    events: new Map(), dataset: {}, children: [], open: false,
    append() {}, prepend() {},
    replaceChildren(...children) { this.children = children; },
    addEventListener(name, handler) { this.events.set(name, handler); },
    showModal() { this.open = true; },
    close() { this.open = false; },
    click() { this.events.get("click")?.(); },
  });
  const document = {
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, createElement());
      return elements.get(id);
    },
    createElement,
    querySelectorAll: () => [],
    addEventListener() {},
  };
  const context = createContext({
    document,
    window: { __TAURI__: {
      core: {
        invoke: async (name, { action }) => {
          assert.equal(name, "editor_view");
          if (action?.kind === "key_preset") {
            actions.push(action.name);
            preferences.preset = action.name;
          }
          return { firstLaunch: preferences.preset == null, busy: false, hasDocument: false };
        },
      },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await runInContext(app, context);
  options.push(...document.getElementById("dialog-actions").children);
  return {
    dialog: document.getElementById("dialog"),
    title: document.getElementById("dialog-title").textContent,
    options, actions, settle: () => runInContext("pending", context),
  };
}

for (const [label, preset] of [["DAW-like", "DAW"], ["osu!", "osu!"]]) {
  test("chooses and remembers the " + label + " keyboard preset", async () => {
    const preferences = {};
    const first = await launch(preferences);
    assert.equal(first.dialog.open, true);
    assert.equal(first.title, "Choose your keyboard preset");
    assert.deepEqual(first.options.map((button) => button.textContent), ["DAW-like", "osu!"]);
    first.dialog.events.get("cancel")({ preventDefault() {} });
    assert.equal(first.dialog.open, true);
    first.options.find((button) => button.textContent === label).click();
    await first.settle();
    assert.deepEqual(first.actions, [preset]);
    assert.equal(preferences.preset, preset);
    assert.equal(first.dialog.open, false);

    const returning = await launch(preferences);
    assert.equal(returning.dialog.open, false);
    assert.deepEqual(returning.actions, []);
  });
}
