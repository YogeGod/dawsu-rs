import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";


async function editor({ imported = [] } = {}) {
  let importedRows = [];
  const elements = new Map();
  const calls = [];
  const edits = [];
  let focused = null;
  function element(tag) {
    const handlers = new Map();
    return {
      tag, handlers, children: [], dataset: {}, style: {}, attributes: {}, className: "",
      addEventListener(name, listener) { handlers.set(name, listener); },
      append(...items) { this.children.push(...items); },
      prepend(...items) { this.children.unshift(...items); },
      replaceChildren(...items) { this.children = items; },
      setAttribute(name, value) { this.attributes[name] = String(value); },
      getAttribute(name) { return this.attributes[name] ?? null; },
      showModal() { this.open = true; },
      close() { this.open = false; },
      focus() { focused = this; },
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
      addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push(name);
        if (name === "editor_shortcuts") return {preset: "DAW", entries: [
          ["undo", "Undo", ["Ctrl+Z"]],
          ["play", "Play or pause", ["Space"]],
          ["volumeUp", "Increase volume by 5", ["Ctrl+Numpad +", "Alt+Up"]],
          ["generate", "Generate hitsound difficulty", []],
          ...importedRows,
        ]};
        if (name === "import_key_file") { importedRows = imported; return true; }
        if (name === "editor_help") return [
          ["Hitsound – Hit editing", [["Ctrl+Z", "Undo"]]],
          ["Hitsound – Hit editing (mouse)", [["Click", "Play or pause"]]],
          ["Hitsound – Volume control", [["Ctrl+Numpad +, Alt+Up", "Increase volume by 5"]]],
          ["Hitsound – Playback", [["Space", "Play or pause"]]],
        ];
        if (args?.action) edits.push(JSON.parse(JSON.stringify(args.action)));
        return { busy: false, hasDocument: false };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  await runInContext("run(showShortcutEditor)", context);
  await runInContext("pending", context);

  const nodes = (root = elements.get("dialog-body")) =>
    [root, ...(root.children ?? []).flatMap(child => typeof child === "object" ? nodes(child) : [])];
  const text = node => typeof node === "string" ? node
    : node.children?.length ? node.children.map(text).join("") : String(node.textContent ?? "");
  const byClass = (name, root) => nodes(root).filter(node => node.className?.split?.(" ").includes(name));
  const row = id => nodes().find(node => node.dataset?.command === id);
  return {
    context, calls, edits, elements, nodes, text, byClass, row,
    focused: () => focused,
    search: () => nodes().find(node => node.tag === "input" && node.type === "search"),
    groups: () => byClass("shortcut-group").map(group => text(group.children[0])),
    rows: () => byClass("shortcut-row").map(node => node.dataset.command),
    chips: id => nodes(row(id)).filter(node => node.tag === "kbd").map(node => node.textContent),
    button: (label, root) => nodes(root).find(node => node.tag === "button" && text(node) === label),
    keys: id => byClass("shortcut-keys", row(id))[0],
    capture: id => byClass("shortcut-capture", row(id))[0],
    settle: () => runInContext("pending", context),
  };
}

test("the shortcut editor lists commands by area with key chips and a preset switch", async () => {
  const ui = await editor();
  assert.deepEqual(ui.calls.slice(-2), ["editor_shortcuts", "editor_help"]);
  assert.deepEqual(ui.groups(), [
    "Hitsound – Hit editing", "Hitsound – Volume control", "Hitsound – Playback", "Other",
  ]);
  assert.deepEqual(ui.rows(), ["undo", "volumeUp", "play", "generate"]);
  assert.deepEqual(ui.chips("undo"), ["Ctrl", "Z"]);
  assert.deepEqual(ui.chips("volumeUp"), ["Ctrl", "Numpad +", "Alt", "Up"]);
  assert.equal(ui.text(ui.keys("generate")), "Unassigned");

  const presets = ui.byClass("shortcut-presets")[0];
  const pressed = () => presets.children.map(button => [ui.text(button), button.getAttribute("aria-pressed")]);
  assert.deepEqual(pressed(), [["osu! editor controls", "false"], ["DAW-like keys", "true"]]);
  ui.button("osu! editor controls").click();
  await ui.settle();
  assert.deepEqual(ui.edits.at(-1), {kind: "key_preset", name: "osu!"});
});

test("the editor opens at the top even when the help list was scrolled", async () => {
  const ui = await editor();
  ui.elements.get("dialog-body").scrollTop = 400;
  await runInContext("closeModal(); run(showShortcutEditor)", ui.context);
  await ui.settle();
  assert.equal(ui.elements.get("dialog-body").scrollTop, 0);
});

test("searching finds commands by name or by key", async () => {
  const ui = await editor();
  const search = ui.search();
  assert.equal(ui.focused(), search);
  const filter = (query) => {
    search.value = query;
    search.handlers.get("input")();
    return ui.rows();
  };
  assert.deepEqual(filter(" CTRL+Z "), ["undo"]);
  assert.deepEqual(filter("volume"), ["volumeUp"]);
  assert.deepEqual(ui.groups(), ["Hitsound – Volume control"]);
  assert.deepEqual(filter("numpad"), ["volumeUp"]);
  assert.deepEqual(filter("nothing like this"), []);
  assert.ok(ui.nodes().some(node => node.textContent === "No commands match."));
  assert.equal(filter("").length, 4);
});

test("clicking a row's keys waits for a key and adds a free key at once", async () => {
  const ui = await editor();
  ui.keys("undo").click();
  const capture = ui.capture("undo");
  assert.ok(capture, "the row should wait for keys");
  assert.equal(ui.text(capture), "Press keys… · Esc cancels");
  assert.equal(ui.focused(), capture);
  assert.match(ui.row("undo").className, /capturing/);
  capture.handlers.get("keydown")({ key: "Shift", code: "ShiftLeft", shiftKey: true, preventDefault() {} });
  assert.equal(ui.edits.length, 0);
  let prevented = false;
  capture.handlers.get("keydown")({
    key: "!", code: "Digit1", ctrlKey: true, shiftKey: true, preventDefault() { prevented = true; },
  });
  await ui.settle();
  assert.ok(prevented);
  assert.deepEqual(ui.edits.at(-1), {kind: "change_shortcut", id: "undo",
    chord: {key: "!", code: "Digit1", ctrl: true, shift: true, alt: false}});
  assert.equal(ui.capture("undo"), undefined);
  assert.equal(ui.focused(), ui.keys("undo"));
});

test("a key that belongs to another command asks before moving it", async () => {
  const ui = await editor();
  ui.keys("play").click();
  ui.capture("play").handlers.get("keydown")({ key: "z", code: "KeyZ", ctrlKey: true, preventDefault() {} });
  await ui.settle();
  assert.equal(ui.edits.length, 0, "a taken key is not sent before the mapper confirms");
  assert.match(ui.row("play").className, /conflict/);
  const conflict = ui.byClass("shortcut-conflict", ui.row("play"))[0];
  assert.equal(ui.text(conflict), "Also undo · Ctrl+Z");
  ui.button("Move here", ui.row("play")).click();
  await ui.settle();
  assert.deepEqual(ui.edits.at(-1), {kind: "change_shortcut", id: "play",
    chord: {key: "z", code: "KeyZ", ctrl: true, shift: false, alt: false}});

  ui.keys("play").click();
  ui.capture("play").handlers.get("keydown")({ key: "ArrowUp", code: "ArrowUp", altKey: true, preventDefault() {} });
  assert.equal(ui.text(ui.byClass("shortcut-conflict", ui.row("play"))[0]),
    "Also increase volume by 5 · Alt+Up");
  ui.button("Cancel", ui.row("play")).click();
  assert.equal(ui.byClass("shortcut-conflict", ui.row("play")).length, 0);
  assert.equal(ui.edits.length, 1);
});

test("pressing a key the command already has, or leaving the row, changes nothing", async () => {
  const ui = await editor();
  ui.keys("play").click();
  ui.capture("play").handlers.get("keydown")({ key: " ", code: "Space", preventDefault() {} });
  assert.equal(ui.capture("play"), undefined);
  ui.keys("play").click();
  ui.capture("play").handlers.get("blur")();
  assert.equal(ui.capture("play"), undefined);
  ui.keys("play").click();
  ui.capture("play").handlers.get("keydown")({ key: "F13", code: "F13", preventDefault() {} });
  assert.match(ui.text(ui.capture("play")), /can't/);
  await ui.settle();
  assert.equal(ui.edits.length, 0);
});

test("Tab leaves the row to the browser and Escape cancels instead of being assigned", async () => {
  const ui = await editor();
  ui.keys("play").click();
  assert.equal(ui.text(ui.capture("play")), "Press keys… · Esc cancels");
  let prevented = false;
  ui.capture("play").handlers.get("keydown")({ key: "Tab", code: "Tab", shiftKey: true, preventDefault() { prevented = true; } });
  assert.equal(prevented, false, "Tab and Shift+Tab move focus; the blur then cancels");
  assert.ok(ui.capture("play"));
  ui.capture("play").handlers.get("keydown")({ key: "Escape", code: "Escape", preventDefault() { prevented = true; } });
  assert.ok(prevented, "Escape must not also close the window");
  assert.equal(ui.capture("play"), undefined);
  assert.equal(ui.focused(), ui.keys("play"));
  assert.deepEqual(ui.chips("play"), ["Space"]);
  await ui.settle();
  assert.equal(ui.edits.length, 0);
});

test("Clear removes a command's keys; Import and Export stay", async () => {
  const ui = await editor();
  assert.equal(ui.button("Clear", ui.row("generate")).disabled, true);
  ui.button("Clear", ui.row("undo")).click();
  await ui.settle();
  assert.deepEqual(ui.edits.at(-1), {kind: "change_shortcut", id: "undo", chord: null});
  const actions = ui.elements.get("dialog-actions");
  assert.deepEqual(actions.children.map(ui.text), ["Import…", "Export…", "Close"]);
});

test("key names match the editor's key list", async () => {
  const ui = await editor();
  const name = (key, code, mods = {}) => runInContext(`shortcutName(${JSON.stringify({
    key, code, ctrl: false, shift: false, alt: false, ...mods,
  })})`, ui.context);
  assert.equal(name("!", "Digit1", {ctrl: true, shift: true}), "Ctrl+Shift+1");
  assert.equal(name("z", "KeyZ"), "Z");
  assert.equal(name("ArrowLeft", "ArrowLeft"), "Left");
  assert.equal(name("Delete", "Delete"), "Del");
  assert.equal(name("Escape", "Escape"), "Esc");
  assert.equal(name(" ", "Space"), "Space");
  assert.equal(name("*", "NumpadMultiply"), "Numpad *");
  assert.equal(name("Enter", "NumpadEnter"), "Numpad Enter");
  assert.equal(name("1", "Numpad1"), "Numpad 1");
  assert.equal(name("+", "Equal", {shift: true}), "Shift+=");
  assert.equal(name("Dead", "Quote"), "'");
  assert.equal(name("F12", "F12", {alt: true}), "Alt+F12");
  assert.equal(name("PageDown", "PageDown"), "PageDown");
  assert.equal(name("F13", "F13"), null);
});

test("Import… fetches the shortcuts again, so a tool's new key works without opening a menu", async () => {
  const ui = await editor({ imported: [["toolDraw", "Draw", ["D"]]] });
  assert.notEqual(runInContext('toolKey("Draw")', ui.context), "d");
  ui.button("Import…", ui.elements.get("dialog-actions")).click();
  await ui.settle();
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(runInContext('toolKey("Draw")', ui.context), "d");
  assert.equal(runInContext('toolDisplayKey("Draw")', ui.context), "D");
});
