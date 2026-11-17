import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("review shows what will change for each file before applying the write", async () => {
  const calls = [];
  const makeElement = () => ({
    events: new Map(), children: [], dataset: {},
    addEventListener(name, callback) { this.events.set(name, callback); },
    append(...children) { this.children.push(...children); },
    prepend(child) { this.children.unshift(child); },
  });
  const element = makeElement();
  const context = createContext({
    document: {
      getElementById: () => element,
      createElement: makeElement,
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push({ name, args });
        return { busy: false, hasDocument: false };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await runInContext(app, context);
  calls.length = 0;

  const review = {
    label: "Write difficulties",
    details: ["Check each difficulty before writing."],
    mixes: ["Mapset/mix.wav"],
    files: [
      { path: "New.osu", replace: false, changed: 0, added: 4, removed: 0, timing: 2 },
      { path: "Hard.osu", replace: true, changed: 3, added: 0, removed: 1, timing: 5 },
    ],
  };
  context.review = review;
  runInContext(`
    modal = (kind, title, content, buttons) => {
      globalThis.output = { kind, title, text: content.map(node => node.textContent).filter(Boolean), content, buttons };
    };
    closeModal = () => {};
    view = { review };
    renderDialogs();
  `, context);

  assert.deepEqual(Array.from(context.output.text), [
    "Files this action will write",
    "Create: New.osu",
    "0 objects changed · 4 added · 0 removed · 2 timing lines affected",
    "Replace: Hard.osu",
    "3 objects changed · 0 added · 1 removed · 5 timing lines affected",
    "Sample mixes",
    "Check each difficulty before writing.",
  ]);
  assert.equal(calls.length, 0);
  const [mixLabel, audition] = context.output.content.find(node => node.className === "row").children;
  assert.equal(mixLabel.textContent, "mix.wav");
  assert.equal(mixLabel.title, "Mapset/mix.wav");
  audition.events.get("click")();
  await runInContext("pending", context);
  assert.equal(calls[0].args.action.kind, "audition_output_mix");
  assert.equal(calls[0].args.action.index, 0);
  calls.length = 0;
  const [, write] = context.output.buttons.find(([label]) => label === "Write files");
  write();
  await runInContext("pending", context);
  assert.equal(calls[0].name, "editor_view");
  assert.equal(calls[0].args.action.kind, "output");
  assert.equal(calls[0].args.action.apply, true);
});

test("editing a Generate merge rule discards the old review and stages a fresh one", async () => {
  const map = "Mapset/Hard.osu";
  const key = '["hit:BeatmapFile:soft-hitclap3.wav@HASH"]';
  const settings = { PairSamples: true, Priorities: ["normal-hitclap", "drum-hitclap"], Rules: {} };
  const calls = [];
  const makeElement = () => ({
    events: new Map(), children: [], dataset: {},
    addEventListener(name, callback) { this.events.set(name, callback); },
    setAttribute(name, value) { this[name] = value; },
    replaceChildren(...children) { this.children = children; },
    append(...children) { this.children.push(...children); },
    prepend(child) { this.children.unshift(child); },
  });
  const element = makeElement();
  const context = createContext({
    map, key,
    document: { getElementById: () => element, createElement: makeElement, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        calls.push([name, args ? JSON.parse(JSON.stringify(args)) : undefined]);
        if (name === "editor_export_settings") return [map, structuredClone(settings)];
        if (name === "save_merge_preset") return true;
        return { busy: false, hasDocument: Boolean(args?.action) };
      } },
      event: { listen() {} },
    } },
    Option: class { constructor(label, value) { this.label = label; this.value = value; } },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {}, setInterval() {}, console,
  });
  await runInContext(app, context);
  calls.length = 0;
  runInContext(`
    modal = (kind, title, content, buttons) => {
      modalKind = kind;
      globalThis.output = { content, buttons };
    };
    show = next => { view = next; };
    closeModal = () => { modalKind = ""; };
    scene = { path: map };
    view = { busy: false, hasDocument: true, review: {
      label: "Generate hitsound difficulty", files: [], details: [], mixes: [],
      includeSliderAudio: true,
      combinations: [{ key, time: 1000, layers: ["soft-hitclap3 (50%)"], destination: "normal-hitclap" }],
    } };
    renderDialogs();
  `, context);
  context.output.buttons.find(([label]) => label === "Edit merge rules…")[1]();
  await runInContext("pending", context);
  const output = context.output;
  const [, slot] = output.content[0].children.find(node => node.children[0] === "Output slot").children;
  slot.value = "drum-hitclap";
  slot.events.get("change")();
  await output.buttons.find(([label]) => label === "Save preset…")[1]();
  output.buttons.find(([label]) => label === "Review again")[1]();
  await runInContext("pending", context);
  assert.deepEqual(calls.map(([name, args]) => args?.action?.kind ?? name), [
    "output", "editor_export_settings", "save_merge_preset", "export_settings", "generate",
  ]);
  assert.equal(calls[0][1].action.apply, false);
  assert.equal(calls[2][1].value.Rules[key], "drum-hitclap");
  assert.equal(calls[3][1].action.value.Rules[key], "drum-hitclap");
  assert.equal(calls[3][1].action.map, map);
  assert.equal(calls[4][1].action.include_slider_audio, true);
});
