import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { dockingRules } from "./docking-rules.mjs";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
const enabled = { browser: true, inspector: true, controller: true, laneNames: true };

// app.js builds the Browser as a plain model and hands it to the Svelte parts; they answer through
// sampleBrowserAction(). The fake bundle keeps the last model.
async function browserPage() {
  const elements = new Map();
  const calls = [];
  const invoked = [];
  const handed = [];
  function element() {
    return {
      listeners: new Map(), children: [], dataset: {}, style: {}, classList: { toggle() {} },
      addEventListener(name, listener) { this.listeners.set(name, listener); },
      setAttribute(name, value) { this[name] = value; },
      append(...children) {
        for (const child of children) {
          if (child.parentNode) child.parentNode.children = child.parentNode.children.filter(item => item !== child);
          child.parentNode = this;
          this.children.push(child);
        }
      },
      prepend() {},
      replaceChildren(...children) { this.children = []; this.append(...children); },
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  let classicBrowser = false;
  const previews = { "lane:1": { ms: 210, peaks: [0.5, 1, 0.25] }, "library:Drums/snare.wav": { ms: 1500, peaks: [1, 0.5] } };
  const context = createContext({
    document: {
      getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {},
    },
    window: { editorDocking: dockingRules,
      dawsuUi: { browser: model => handed.push(JSON.parse(JSON.stringify(model))), update() {}, lanes() {}, inspector() {} },
      __TAURI__: {
        core: { invoke: async (command, args) => {
          if (command === "editor_view") calls.push(JSON.parse(JSON.stringify(args?.action ?? null)));
          else invoked.push([command, JSON.parse(JSON.stringify(args))]);
          if (command === "editor_view" && args?.action?.kind === "browser_style") {
            classicBrowser = args.action.classic;
          }
          if (command === "list_library") {
            assert.equal(args.location.root, "C:/fixtures/library");
            return args.location.path === ""
              ? [["Drums", true]]
              : [["clap.wav", false], ["snare.wav", false]];
          }
          if (command === "sample_preview") {
            const { kind, row, location } = args.source;
            const answer = previews[kind === "lane" ? "lane:" + row : "library:" + location.path];
            if (!answer) throw "Decoding failed";
            return answer;
          }
          return {busy: false, activeLane: 0, hasDocument: true, panels: {...enabled},
            classicBrowser, hitsoundLibrary: "C:/fixtures/library"};
        } },
        event: { listen() {} },
      } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
    console,
  });
  await runInContext(app, context);
  runInContext(`scene = {path: "copy.osu", lanes: [
    {key: "normal", sourceKey: "normal", name: "normal-hitnormal", canReplace: true},
    {key: "clap", sourceKey: "clap", name: "Snare", sample: "percussion/snare.wav"},
    {key: "kick", sourceKey: "kick", name: "Kick", sample: "kick.wav"},
  ]};
  renderPanelLayout();`, context);
  const action = (...args) => context.sampleBrowserAction(...args);
  const settle = () => runInContext("pending", context);
  const model = () => handed.at(-1);
  const labels = () => model().rows.map(row => row.label);
  return { context, calls, invoked, handed, model, labels, action, settle, get };
}

test("the tree lists map folder samples under Mapset samples, folds folders and previews a sample", async () => {
  const { calls, invoked, model, labels, action, settle } = await browserPage();
  assert.equal(model().source, "mapset");
  assert.equal(model().classic, false);
  assert.equal(model().root, "Mapset samples");
  assert.equal(model().defaults, true, "the Default samples section shows on the Mapset tab");
  assert.deepEqual(labels(), ["percussion", "kick.wav"]);
  assert.deepEqual(model().rows.map(row => [row.kind, row.depth, row.open ?? null, row.selected]),
    [["folder", 0, false, false], ["sample", 0, null, false]]);
  assert.equal(model().preview, null);
  assert.equal(model().canAssign, false);
  assert.equal(model().canPick, true);

  action("toggle", "percussion");
  assert.deepEqual(labels(), ["percussion", "snare.wav", "kick.wav"]);
  assert.deepEqual(model().rows.map(row => row.depth), [0, 1, 0]);
  assert.equal(model().rows[0].open, true);

  action("pick", "percussion/snare.wav");
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "lane", row: 1, operation: "preview_sample"});
  assert.deepEqual(invoked.at(-1), ["sample_preview", { source: { kind: "lane", row: 1 } }]);
  assert.equal(model().rows[1].selected, true);
  assert.deepEqual(model().preview, { name: "snare.wav", title: "percussion/snare.wav", ms: 210, peaks: [0.5, 1, 0.25] });
  assert.equal(model().canAssign, true);

  // Play replays the selected sample without asking for its outline again.
  const asked = invoked.length;
  action("play");
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "lane", row: 1, operation: "preview_sample"});
  assert.equal(invoked.length, asked);

  action("assign");
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "replace_sample", target: {map: "copy.osu", lane: "normal"},
    path: "percussion/snare.wav", mapset: true});

  // A sample the backend can't decode shows its name only: no length and no outline.
  action("pick", "kick.wav");
  await settle();
  assert.deepEqual(model().preview, { name: "kick.wav", title: "kick.wav", ms: null, peaks: null });

  action("filter", "SNARE");
  assert.deepEqual(labels(), ["percussion", "snare.wav"]);
  assert.equal(model().filter, "SNARE");
  assert.equal(model().canAssign, false, "a sample the filter hides can't be used");
  action("filter", "nothing");
  assert.deepEqual(model().rows.map(row => [row.kind, row.label]), [["folder", "percussion"], ["note", "No matching samples."]]);
});

test("the Library tab lists the hitsound library and previews its files", async () => {
  const { calls, invoked, model, labels, action, settle } = await browserPage();
  action("source", "library");
  await settle();
  assert.equal(model().source, "library");
  assert.equal(model().root, "library");
  assert.equal(model().defaults, false);
  assert.deepEqual(labels(), ["Drums"]);
  action("toggle", "Drums");
  assert.deepEqual(model().rows.map(row => [row.kind, row.label]), [["folder", "Drums"], ["note", "Loading samples…"]]);
  await settle();
  assert.deepEqual(labels(), ["Drums", "clap.wav", "snare.wav"]);
  action("pick", "Drums/snare.wav");
  await settle();
  const location = {root: "C:/fixtures/library", path: "Drums/snare.wav"};
  assert.deepEqual(calls.at(-1), {kind: "preview_library_sample", location});
  assert.deepEqual(invoked.at(-1), ["sample_preview", { source: { kind: "library", location } }]);
  assert.deepEqual(model().preview, { name: "snare.wav", title: "Drums/snare.wav", ms: 1500, peaks: [1, 0.5] });
  action("assign");
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "replace_library_sample", location,
    target: {map: "copy.osu", lane: "normal"}});
  // Switching tabs clears the selection and its preview.
  action("source", "mapset");
  assert.equal(model().preview, null);
});

test("Classic opens one folder at a time with Up, inside the Inspector", async () => {
  const { context, calls, model, labels, action, settle, get } = await browserPage();
  runInContext(`let controls;
    modal = (_kind, _title, body) => { controls = body; };
    labeled = (label, control) => ({label, control});`, context);
  runInContext("openViewSettings()", context);
  const style = runInContext('controls.find(item => item.label === "Browser style").control', context);
  style.value = "classic";
  style.listeners.get("change")();
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "browser_style", classic: true});
  assert.equal(model().classic, true);
  assert.equal(model().root, null);
  assert.deepEqual(model().folder, { label: "Map folder", title: "", canUp: false });
  assert.deepEqual(labels(), ["percussion", "kick.wav"]);
  action("toggle", "percussion");
  assert.deepEqual(labels(), ["snare.wav"]);
  assert.deepEqual(model().folder, { label: "percussion", title: "percussion", canUp: true });
  action("up");
  assert.deepEqual(labels(), ["percussion", "kick.wav"]);
  action("toggle", "percussion");
  action("pick", "percussion/snare.wav");
  await settle();
  action("assign");
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "replace_sample", target: {map: "copy.osu", lane: "normal"},
    path: "percussion/snare.wav", mapset: true});
  // Close folds the Browser through the same path as every other switch.
  runInContext("var toggled = []; togglePanel = panel => toggled.push(panel);", context);
  get("sample-browser-close").listeners.get("click")();
  assert.deepEqual(JSON.parse(runInContext("JSON.stringify(toggled)", context)), ["browser"]);
  runInContext('act({ kind: "browser_style", classic: false });', context);
  await settle();
  assert.equal(model().classic, false);
});

test("a busy editor greys out the sample rows and both buttons", async () => {
  const { context, model, action } = await browserPage();
  action("pick", "kick.wav");
  runInContext("view.busy = true; renderSampleBrowser();", context);
  assert.equal(model().busy, true);
  assert.equal(model().canAssign, false);
  assert.equal(model().canPick, false);
});
