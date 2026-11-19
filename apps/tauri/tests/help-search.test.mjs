import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("F1 help searches active command bindings and mouse actions", async () => {
  const elements = new Map();
  const listeners = new Map();
  const calls = [];
  const edits = [];
  function element(tag) {
    const handlers = new Map();
    return {
      tag, handlers, children: [], dataset: {}, style: {},
      addEventListener(name, listener) { handlers.set(name, listener); },
      append(...items) { this.children.push(...items); },
      prepend(...items) { this.children.unshift(...items); },
      replaceChildren(...items) { this.children = items; },
      showModal() { this.open = true; },
      close() { this.open = false; },
      setAttribute(name, value) { this[name] = value; },
      focus() { this.focused = true; },
      click() { handlers.get("click")(); },
    };
  }
  const context = createContext({
    document: {
      getElementById(id) {
        if (!elements.has(id)) elements.set(id, element("div"));
        return elements.get(id);
      },
      createElement: element,
      querySelectorAll: () => [],
      addEventListener: (name, listener) => listeners.set(name, listener),
    },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push(name);
        if (name === "editor_shortcuts") return {preset: "DAW", entries: [
          ["undo", "Undo", ["Ctrl+Z"]], ["play", "Play", ["Space"]],
        ]};
        if (name === "import_key_file" || name === "export_key_file") return true;
        if (args?.action?.kind === "change_shortcut") edits.push(JSON.parse(JSON.stringify(args.action)));
        if (name === "editor_help") return [
          ["Edit", [["Ctrl+Z", "Undo"], ["Alt+Down", "Decrease volume"]]],
          ["Timeline (mouse)", [["Ctrl+wheel", "Zoom around the pointer"]]],
        ];
        return { busy: false, hasDocument: false, shortcutHelp: args?.action?.key === "F1" };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await runInContext(app, context);

  listeners.get("keydown")({ key: "F1", target: { closest: () => null }, preventDefault() {} });
  await runInContext("pending", context);
  assert.equal(calls.at(-1), "editor_help");
  const body = elements.get("dialog-body");
  const [label, results] = body.children;
  const search = label.children.find((child) => child.tag === "input");
  assert.ok(search, "help should offer a search input");
  assert.equal(search.focused, true);
  const contents = () => results.children.map((node) => node.textContent);
  const filter = (query) => {
    search.value = query;
    search.handlers.get("input")();
    return contents();
  };
  assert.deepEqual(contents(), [
    "Edit", "Ctrl+Z — Undo", "Alt+Down — Decrease volume",
    "Timeline (mouse)", "Ctrl+wheel — Zoom around the pointer",
  ]);
  assert.deepEqual(filter(" CTRL+Z "), ["Edit", "Ctrl+Z — Undo"]);
  assert.deepEqual(filter("mouse pointer"), ["Timeline (mouse)", "Ctrl+wheel — Zoom around the pointer"]);
  assert.deepEqual(filter("unknown"), ["No matching shortcuts or mouse actions."]);
  assert.equal(filter("").length, 5);

  const actions = elements.get("dialog-actions");
  const click = label => actions.children.find(node => node.textContent === label).click();
  click("Customise shortcuts…");
  await runInContext("pending", context);
  assert.deepEqual(calls.slice(-2), ["editor_shortcuts", "editor_help"]);
  // The shortcut editor's rows, capture and conflicts are covered in shortcut-editor.test.mjs.
  const nodes = (node) => [node, ...(node.children ?? []).flatMap(child => typeof child === "object" ? nodes(child) : [])];
  const row = nodes(body).find(node => node.dataset?.command === "play");
  const keys = row.children.find(node => node.className === "shortcut-keys");
  keys.click();
  const capture = row.children.find(node => node.className === "shortcut-capture");
  capture.handlers.get("keydown")({
    key: "1", code: "Digit1", ctrlKey: true, shiftKey: true, preventDefault() {},
  });
  await runInContext("pending", context);
  assert.deepEqual(edits.at(-1), {kind: "change_shortcut", id: "play",
    chord: {key: "1", code: "Digit1", ctrl: true, shift: true, alt: false}});
  click("Import…");
  await runInContext("pending", context);
  click("Export…");
  await runInContext("pending", context);
  assert.ok(calls.includes("import_key_file") && calls.includes("export_key_file"));
});
