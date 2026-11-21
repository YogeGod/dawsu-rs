import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";


test("sample chooser previews a mapset sound without moving the active lane", async () => {
  const controls = new Map();
  const calls = [];
  const events = new Map();
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, style: {}, classList: { toggle() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute(name, value) { this[name] = value; },
      append(...children) { this.children.push(...children); },
      prepend() {},
      replaceChildren(...children) { this.children = children; },
      showModal() { this.open = true; },
      close() { this.open = false; },
    };
  }
  function get(id) {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  }
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push([name, JSON.parse(JSON.stringify(args))]);
        return {busy: false, hasDocument: true, activeLane: 0, position: 0};
      } },
      event: { listen: (name, callback) => events.set(name, callback) },
    } },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await bootEditor(context);
  calls.length = 0;
  runInContext(`view.activeLane = 0;
    scene = {path: "copy.osu", lanes: [{key: "normal", sourceKey: "normal", name: "normal-hitnormal", canReplace: true},
      {key: "clap", sourceKey: "clap", name: "soft-hitclap", sample: "soft-hitclap.wav"}]};`, context);
  runInContext("inspectorAction('replaceSample')", context);
  const select = get("dialog-body").children[0].children[1];
  select.value = "1";
  const preview = get("dialog-actions").children.find(button => button.textContent === "Audition");
  preview.events.get("click")();
  await runInContext("pending", context);
  assert.deepEqual(calls.pop(), ["editor_view", {scene: false, action: {kind: "lane", row: 1, operation: "preview_sample"}}]);
  assert.equal(runInContext("view.activeLane", context), 0);
  const replace = get("dialog-actions").children.find(button => button.textContent === "Replace");
  replace.events.get("click")();
  await runInContext("pending", context);
  assert.deepEqual(calls.pop(), ["editor_view", {scene: true, action: {kind: "replace_sample",
    target: {map: "copy.osu", lane: "normal"}, path: "soft-hitclap.wav", mapset: true}}]);

  runInContext("inspectorAction('replaceSample')", context);
  get("dialog-actions").children.find(button => button.textContent === "Browse…").events.get("click")();
  await runInContext("pending", context);
  assert.deepEqual(calls.pop(), ["pick_sample", {target: {map: "copy.osu", lane: "normal"}}]);

  events.get("tauri://drag-drop")({payload: {paths: ["C:/Samples/Snare.OGG"]}});
  await runInContext("pending", context);
  assert.deepEqual(calls.pop(), ["editor_view", {scene: true, action: {kind: "replace_sample",
    target: {map: "copy.osu", lane: "normal"}, path: "C:/Samples/Snare.OGG", mapset: false}}]);
});