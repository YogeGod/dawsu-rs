import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";


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
  await bootEditor(context);
  calls.length = 0;

  const review = {
    label: "Write difficulties",
    details: ["Check each difficulty before writing.", "Hard.osu · 1,250 ms · slider tick · sample.wav"],
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
    scene = { path: "Mapset/Source.osu" };
    view = { review };
    renderDialogs();
  `, context);

  assert.deepEqual(Array.from(context.output.text), [
    "Editing Source.osu",
    "Files this action will write",
    "Create: New.osu",
    "0 objects changed · 4 added · 0 removed · 2 timing lines affected",
    "Replace: Hard.osu",
    "3 objects changed · 0 added · 1 removed · 5 timing lines affected",
    "Sample mixes",
    "Check each difficulty before writing.",
    "Hard.osu · 1,250 ms · slider tick · sample.wav",
  ]);
  assert.equal(calls.length, 0);
  const [mixLabel, audition] = context.output.content.find(node => node.className === "row").children;
  assert.equal(mixLabel.textContent, "Mapset/mix.wav");
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
  await bootEditor(context);
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

test("review lists new and reused .wav files apart and says once how volumes are written", async () => {
  const calls = [];
  const makeElement = () => ({
    events: new Map(), children: [], dataset: {},
    addEventListener(name, callback) { this.events.set(name, callback); },
    append(...children) { this.children.push(...children); },
    prepend(child) { this.children.unshift(child); },
  });
  const element = makeElement();
  const context = createContext({
    document: { getElementById: () => element, createElement: makeElement, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => { calls.push({ name, args }); return { busy: false, hasDocument: false }; } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  calls.length = 0;
  const volume = "Per-object .osu volumes only: 3 object(s) updated, 0 new timing lines needed.";
  const wavs = "Sample mixes: 1 new .wav created, 1 existing .wav reused.";
  context.review = {
    label: "Save hitsounds",
    details: ["Check each difficulty before writing.", volume, wavs],
    mixes: ["Mapset/soft-hitclap2.wav", "Mapset/normal-hitwhistle3.wav"],
    createdMixes: ["Mapset/normal-hitwhistle3.wav"],
    reusedMixes: ["Mapset/soft-hitclap2.wav"],
    volumeSummary: [volume, wavs],
    files: [{ path: "Hard.osu", replace: true, changed: 3, added: 0, removed: 0, timing: 0 }],
  };
  runInContext(`
    modal = (kind, title, content, buttons) => {
      globalThis.output = { kind, title, text: content.map(node => node.textContent).filter(Boolean), content, buttons };
    };
    closeModal = () => {};
    scene = { path: "Mapset/Source.osu" };
    view = { review };
    renderDialogs();
  `, context);
  assert.deepEqual(Array.from(context.output.text), [
    "Editing Source.osu",
    "Files this action will write",
    "Replace: Hard.osu",
    "3 objects changed · 0 added · 0 removed · 0 timing lines affected",
    "New .wav files dawsu! creates",
    "Existing .wav files reused (left as they are)",
    "Volumes", volume, wavs,
    "Check each difficulty before writing.",
  ]);
  const rows = context.output.content.filter(node => node.className === "row");
  assert.deepEqual(rows.map(row => row.children[0].textContent), ["Mapset/normal-hitwhistle3.wav", "Mapset/soft-hitclap2.wav"]);
  rows[0].children[1].events.get("click")();
  await runInContext("pending", context);
  assert.equal(calls[0].args.action.kind, "audition_output_mix");
  assert.equal(calls[0].args.action.index, 1, "Audition plays the mix by its place in mixes");
});

test("Revert mapset to original asks first, with Cancel as the default, then invokes the revert and refreshes", async () => {
  const calls = [];
  const makeElement = () => ({
    events: new Map(), children: [], dataset: {},
    addEventListener(name, callback) { this.events.set(name, callback); },
    append(...children) { this.children.push(...children); },
    prepend(child) { this.children.unshift(child); },
  });
  const element = makeElement();
  const context = createContext({
    document: { getElementById: () => element, createElement: makeElement, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => { calls.push({ name, args }); return name === "revert_mapset_to_original" ? 2 : { busy: false, hasDocument: false }; } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  calls.length = 0;
  runInContext(`
    globalThis.opened = [];
    modal = (kind, title, content, buttons) => { opened.push({ kind, title, text: content.map(node => node.textContent), buttons }); };
    closeModal = () => {};
    scene = { path: "Mapset/Hard.osu" };
    view = { hasDocument: true, hasOriginalSnapshot: false, busy: false };
    confirmRevertToOriginal();
  `, context);
  assert.equal(context.opened.length, 0, "no first-open copy, no dialog");
  runInContext(`view = { hasDocument: true, hasOriginalSnapshot: true, busy: false }; confirmRevertToOriginal();`, context);
  const [dialog] = context.opened;
  assert.equal(dialog.kind, "revert-original");
  assert.equal(dialog.title, "Revert mapset to original?");
  assert.match(dialog.text.join(" "), /saved and unsaved/);
  assert.match(dialog.text.join(" "), /backup/);
  const [cancel, confirm] = dialog.buttons;
  assert.deepEqual([cancel[0], cancel[2]], ["Cancel", true], "Cancel is the default button");
  assert.equal(confirm[0], "Revert mapset");
  confirm[1]();
  await runInContext("pending", context);
  assert.deepEqual(calls.map(call => call.name), ["revert_mapset_to_original", "editor_view"]);
});
