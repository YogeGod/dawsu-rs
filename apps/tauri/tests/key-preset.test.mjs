import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";
import { presetSummary } from "../ui/src/lib/controls.js";

// What editor_shortcuts answers for each preset (crates/dawsu-editor/src/tools.rs).
const toolShortcuts = preset => ({ preset, entries: [
  ["toolSelect", "Select and move", ["1"]], ["toolDraw", "Draw", [preset === "osu!" ? "2" : "3"]],
  ["toolErase", "Erase", ["4"]],
] });
const tick = () => new Promise(resolve => setTimeout(resolve, 0));


async function launch(preferences, { onShortcuts = null } = {}) {
  const elements = new Map();
  const options = [];
  const actions = [];
  const keys = [];
  const shortcutFetches = [];
  const documentEvents = new Map();
  const listeners = new Map();
  const createElement = () => ({
    events: new Map(), dataset: {}, children: [], open: false,
    append() {}, prepend() {}, setAttribute(name, value) { this[name] = value; },
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
    addEventListener(name, handler) { documentEvents.set(name, handler); },
  };
  const context = createContext({
    document,
    window: { __TAURI__: {
      core: {
        invoke: async (name, args = {}) => {
          if (name === "editor_shortcuts") {
            const current = preferences.preset ?? "DAW";
            shortcutFetches.push(current);
            return onShortcuts ? onShortcuts(current) : toolShortcuts(current);
          }
          assert.equal(name, "editor_view");
          const { action } = args;
          if (action?.kind === "key_preset") {
            actions.push(action.name);
            preferences.preset = action.name;
          }
          if (action?.kind === "key" || action?.kind === "command") keys.push(action);
          return { firstLaunch: preferences.preset == null, busy: false, hasDocument: false, keyPreset: preferences.preset ?? "DAW" };
        },
      },
      event: { listen(name, handler) { listeners.set(name, handler); } },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  options.push(...document.getElementById("dialog-actions").children);
  return {
    dialog: document.getElementById("dialog"),
    title: document.getElementById("dialog-title").textContent,
    options, actions, keys, shortcutFetches, settle: () => runInContext("pending", context),
    run: code => runInContext(code, context),
    get: id => document.getElementById(id), documentEvents, listeners, window: context.window,
    openTimeline: () => runInContext("scene = {}", context),
    pressKey(key, code, { ctrlKey = false, target = "button" } = {}) {
      const event = {
        key, code, ctrlKey, target: { closest: selector => selector.split(", ").includes(target) },
        defaultPrevented: false, preventDefault() { this.defaultPrevented = true; },
      };
      documentEvents.get("keydown")(event);
      return event.defaultPrevented;
    },
  };
}

test("a focused button preserves plain Enter and routes numeric or modified Enter shortcuts", async () => {
  const editor = await launch({ preset: "DAW" });
  assert.equal(editor.pressKey("Enter", "Enter"), false);
  assert.equal(editor.pressKey("Enter", "NumpadEnter"), true);
  assert.equal(editor.pressKey("Enter", "Enter", { ctrlKey: true }), true);
  await editor.settle();
  assert.deepEqual(editor.keys.map(({ code, ctrl }) => [code, ctrl]), [
    ["NumpadEnter", false], ["Enter", true],
  ]);
});

test("Space routes to the timeline shortcut from a focused button", async () => {
  const editor = await launch({ preset: "DAW" });
  assert.equal(editor.pressKey(" ", "Space"), false);
  editor.openTimeline();
  assert.equal(editor.pressKey(" ", "Space", { target: "input" }), false);
  editor.dialog.showModal();
  assert.equal(editor.pressKey(" ", "Space"), false);
  editor.dialog.close();
  assert.equal(editor.pressKey(" ", "Space"), true);
  await editor.settle();
  assert.deepEqual(editor.keys.map(({ key, code }) => [key, code]), [[" ", "Space"]]);
});

test("an open menu or Preferences keeps Space and Ctrl+Z away from the timeline", async () => {
  const editor = await launch({ preset: "DAW" });
  editor.openTimeline();
  editor.get("editor-menu").hidden = false;
  assert.equal(editor.pressKey(" ", "Space"), false, "menu open");
  editor.get("editor-menu").hidden = true;
  editor.window.dawsuPreferencesOpen = true;
  assert.equal(editor.pressKey("z", "KeyZ", { ctrlKey: true }), false, "Preferences open");
  editor.window.dawsuPreferencesOpen = false;
  assert.equal(editor.pressKey(" ", "Space"), true);
  await editor.settle();
  assert.deepEqual(editor.keys.map(({ key }) => key), [" "]);
});

test("Space in the map preview window plays here, unless a dialog, menu or Preferences is open", async () => {
  const editor = await launch({ preset: "DAW" });
  const space = editor.listeners.get("preview-space");
  space();
  editor.openTimeline();
  editor.get("editor-menu").hidden = false;
  space();
  editor.get("editor-menu").hidden = true;
  editor.window.dawsuPreferencesOpen = true;
  space();
  editor.window.dawsuPreferencesOpen = false;
  editor.dialog.showModal();
  space();
  editor.dialog.close();
  space();
  await editor.settle();
  assert.deepEqual(editor.keys.map(({ key, code }) => [key, code]), [[" ", "Space"]]);
});

test("a used select, checkbox or slider lets go of focus, so Space reaches the timeline again", async () => {
  const editor = await launch({ preset: "DAW" });
  const control = (tagName, type) => {
    const node = { tagName, type, blurred: 0, blur() { this.blurred++; } };
    node.closest = selector => selector === "dialog, [role=dialog]" ? null
      : selector.split(", ").some(part => part === tagName.toLowerCase() || part === "input[type=" + type + "]") ? node : null;
    return node;
  };
  const select = control("SELECT");
  const checkbox = control("INPUT", "checkbox");
  const range = control("INPUT", "range");
  const text = control("INPUT", "text");
  editor.documentEvents.get("pointerup")({ type: "pointerup", target: select });
  assert.equal(select.blurred, 0, "a select's pointerup opens its list");
  editor.documentEvents.get("change")({ type: "change", target: select });
  editor.documentEvents.get("pointerup")({ type: "pointerup", target: checkbox });
  editor.documentEvents.get("pointerup")({ type: "pointerup", target: range });
  editor.documentEvents.get("change")({ type: "change", target: text });
  assert.deepEqual([select.blurred, checkbox.blurred, range.blurred, text.blurred], [1, 1, 1, 0]);
  // A slider's change also comes from arrow keys, so only its pointerup lets go; a checkbox's label click sends only change.
  editor.documentEvents.get("change")({ type: "change", target: range });
  editor.documentEvents.get("change")({ type: "change", target: checkbox });
  assert.deepEqual([range.blurred, checkbox.blurred], [1, 2]);
  const inDialog = control("SELECT");
  inDialog.closest = selector => inDialog;
  editor.documentEvents.get("change")({ type: "change", target: inDialog });
  assert.equal(inDialog.blurred, 0, "a dialog keeps its focus");
});

test("Save in the top bar saves at once, as Ctrl+S does", async () => {
  const editor = await launch({ preset: "DAW" });
  editor.get("save").click();
  await editor.settle();
  assert.deepEqual(editor.keys, [], "nothing to save without a map");
  editor.openTimeline();
  editor.get("save").click();
  await editor.settle();
  assert.deepEqual(editor.keys, [{ kind: "command", id: "save" }]);
});

for (const [label, preset] of [["DAW-like", "DAW"], ["osu! Editor", "osu!"]]) {
  test("chooses and remembers the " + label + " keyboard preset", async () => {
    const preferences = {};
    const first = await launch(preferences);
    assert.equal(first.dialog.open, true);
    assert.equal(first.title, "Choose your keyboard preset");
    assert.deepEqual(first.options.map((button) => button.textContent), ["osu! Editor", "DAW-like"]);
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

test("the preset chooser says what each preset's wheel, zoom and tool keys do", async () => {
  const first = await launch({});
  const body = first.get("dialog-body").children.map(node => node.textContent);
  assert.deepEqual(body, [presetSummary("osu!"), presetSummary("DAW")]);
  assert.doesNotMatch(body[0], /1\s*[–-]\s*3/);
  for (const text of body) assert.match(text, /Wheel zooms/);
});

test("picking osu! Editor on first launch fetches the shortcuts again, so Draw's key is 2 at once", async () => {
  const first = await launch({});
  await tick();
  assert.deepEqual(first.shortcutFetches, ["DAW"], "the first view fetches the shortcuts");
  // A menu opened before the pick caches the DAW shortcuts.
  first.run(`setQuasimodeShortcuts(${JSON.stringify(toolShortcuts("DAW"))})`);
  assert.equal(first.run('toolKey("Draw")'), "3");
  first.options.find(button => button.textContent === "osu! Editor").click();
  await first.settle();
  await tick();
  assert.deepEqual(first.shortcutFetches, ["DAW", "osu!"]);
  assert.equal(first.run('toolKey("Draw")'), "2");
  assert.equal(first.run('toolDisplayKey("Erase")'), "4");
});

test("rapid preset switches drop stale editor_shortcuts answers and swallow rejections without looping", async () => {
  const pending = [];
  const ui = await launch({ preset: "DAW" }, {
    onShortcuts: preset => new Promise((resolve, reject) => pending.push({ preset, resolve, reject })),
  });
  assert.equal(pending.length, 1);
  pending.shift().reject(new Error("offline"));
  await tick();
  await ui.run("act(null)");
  await tick();
  assert.deepEqual(ui.shortcutFetches, ["DAW"], "a rejected fetch does not retry on every view");

  await ui.run('act({ kind: "key_preset", name: "osu!" })');
  await ui.run('act({ kind: "key_preset", name: "DAW" })');
  assert.equal(pending.length, 2);
  const [osuReq, dawReq] = pending;
  dawReq.resolve(toolShortcuts("DAW"));
  await tick();
  assert.equal(ui.run('toolKey("Draw")'), "3");
  osuReq.resolve(toolShortcuts("osu!"));
  await tick();
  assert.equal(ui.run('toolKey("Draw")'), "3", "stale osu! answer must not overwrite newer DAW shortcuts");
  assert.equal(ui.run('toolDisplayKey("Draw")'), "3");
});
