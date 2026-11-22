import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { sampleKey } from "../ui/src/lib/browser.js";
import { dockingRules } from "./docking-rules.mjs";
import { bootEditor, readEditorSource } from "./editor-runtime.mjs";

const app = readEditorSource();
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
          else invoked.push([command, JSON.parse(JSON.stringify(args ?? null))]);
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
  await bootEditor(context);
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
  const { context, calls, invoked, model, labels, action, settle } = await browserPage();
  assert.equal(model().source, "mapset");
  assert.equal(model().classic, false);
  assert.equal(model().root, "Mapset samples");
  assert.equal(model().defaults, undefined, "no greyed Default samples rows");
  assert.equal(model().target, "normal-hitnormal", "Use names the active lane");
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
  // The name shows at once; the length and outline follow the backend's answer.
  assert.deepEqual(model().preview, { name: "snare.wav", title: "percussion/snare.wav", ms: null, peaks: null });
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "lane", row: 1, operation: "preview_sample"});
  assert.deepEqual(invoked.at(-1), ["sample_preview", { source: { kind: "lane", row: 1 } }]);
  assert.equal(model().rows[1].selected, true);
  assert.deepEqual(model().preview, { name: "snare.wav", title: "percussion/snare.wav", ms: 210, peaks: [0.5, 1, 0.25] });
  assert.equal(model().canAssign, true);

  // Play and picking the same row again ask for the outline again (the file may have changed),
  // keeping the shown length meanwhile.
  const asked = invoked.length;
  action("play");
  assert.equal(model().preview.ms, 210);
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "lane", row: 1, operation: "preview_sample"});
  assert.equal(invoked.length, asked + 1);
  action("pick", "percussion/snare.wav");
  await settle();
  assert.equal(invoked.length, asked + 2);

  action("assign");
  await settle();
  assert.deepEqual(calls.at(-1), {kind: "replace_sample", target: {map: "copy.osu", lane: "normal"},
    path: "percussion/snare.wav", mapset: true});

  // A sample the backend can't decode shows its name only: no length and no outline.
  action("pick", "kick.wav");
  await settle();
  assert.deepEqual(model().preview, { name: "kick.wav", title: "kick.wav", ms: null, peaks: null });

  // The search reaches into closed folders: a folder with a match shows open, one without hides.
  action("toggle", "percussion");
  assert.equal(model().rows[0].open, false);
  action("filter", "SNARE");
  assert.deepEqual(labels(), ["percussion", "snare.wav"]);
  assert.equal(model().rows[0].open, true);
  assert.equal(model().filter, "SNARE");
  assert.equal(model().canAssign, false, "a sample the filter hides can't be used");
  action("filter", "nothing");
  assert.deepEqual(model().rows.map(row => [row.kind, row.label]), [["note", "No matching samples."]]);
  action("filter", "");

  // A picked sample whose lane is gone is dropped, so Play has nothing to play.
  runInContext("scene = {...scene, lanes: scene.lanes.slice(0, 2)}; renderSampleBrowser();", context);
  assert.equal(model().preview, null);
  const sent = calls.length;
  action("play");
  await settle();
  assert.equal(calls.length, sent);
});

test("ArrowDown and ArrowUp play the next sample row; Enter uses it in the active lane", async () => {
  const { calls, invoked, model, action, settle } = await browserPage();
  action("toggle", "percussion");
  // The rows BrowserList draws: one button per sample row, in the list's order.
  const list = { querySelectorAll: () => buttons };
  const focused = [];
  const buttons = model().rows.filter(row => row.kind === "sample").map(row => ({
    row: () => model().rows.find(item => item.key === row.key),
    dataset: { sample: row.key }, parentElement: list, focus() { focused.push(row.key); },
  }));
  const [snare, kick] = buttons;
  const press = (button, key, init = {}) => {
    const event = { key, currentTarget: button, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false,
      prevented: false, preventDefault() { this.prevented = true; }, ...init };
    sampleKey(event, button.row(), action);
    return event;
  };

  assert.equal(press(snare, "ArrowDown").prevented, true, "the timeline doesn't get the key");
  assert.deepEqual(focused, ["kick.wav"]);
  await settle();
  assert.equal(kick.row().selected, true);
  assert.deepEqual(model().preview.name, "kick.wav");
  assert.deepEqual(calls.at(-1), { kind: "lane", row: 2, operation: "preview_sample" });
  assert.deepEqual(invoked.at(-1), ["sample_preview", { source: { kind: "lane", row: 2 } }]);

  press(kick, "ArrowUp");
  await settle();
  assert.deepEqual(focused, ["kick.wav", "percussion/snare.wav"]);
  assert.equal(snare.row().selected, true);
  assert.deepEqual(invoked.at(-1), ["sample_preview", { source: { kind: "lane", row: 1 } }]);

  // Past either end, or with a modifier held, nothing moves or plays.
  const asked = invoked.length;
  press(snare, "ArrowUp");
  press(kick, "ArrowDown", { shiftKey: true });
  await settle();
  assert.equal(focused.length, 2);
  assert.equal(invoked.length, asked);
  assert.equal(snare.row().selected, true);

  // Enter on the picked row uses it in the active lane (normal-hitnormal).
  press(snare, "Enter");
  await settle();
  assert.deepEqual(calls.at(-1), { kind: "replace_sample", target: { map: "copy.osu", lane: "normal" },
    path: "percussion/snare.wav", mapset: true });
  // Enter on a row not yet picked picks it first, then uses it.
  assert.equal(press(kick, "Enter").prevented, true);
  await settle();
  assert.equal(kick.row().selected, true);
  assert.deepEqual(calls.at(-1), { kind: "replace_sample", target: { map: "copy.osu", lane: "normal" },
    path: "kick.wav", mapset: true });
});

test("the Library tab lists the hitsound library and previews its files", async () => {
  const { calls, invoked, model, labels, action, settle } = await browserPage();
  action("source", "library");
  await settle();
  assert.equal(model().source, "library");
  assert.equal(model().root, "library");
  assert.equal(model().chooseLibrary, false);
  assert.deepEqual(labels(), ["Drums"]);
  // The search opens folders not listed yet, like the Mapset tab: while one loads it shows open,
  // then it hides when nothing in it matches.
  action("filter", "zzz");
  assert.deepEqual(model().rows.map(row => [row.kind, row.label, row.open ?? null]),
    [["folder", "Drums", true], ["note", "Loading samples…", null]]);
  await settle();
  assert.deepEqual(model().rows.map(row => [row.kind, row.label]), [["note", "No matching samples."]]);
  action("filter", "SNARE");
  assert.deepEqual(labels(), ["Drums", "snare.wav"], "a match in a closed subfolder opens it");
  action("filter", "");
  assert.deepEqual(labels(), ["Drums"], "clearing the search folds what it opened");
  action("toggle", "Drums");
  assert.deepEqual(labels(), ["Drums", "clap.wav", "snare.wav"]);
  action("filter", "SNARE");
  assert.deepEqual(labels(), ["Drums", "snare.wav"]);
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

test("picks made while a preview is on its way replace each other: only the last one plays", async () => {
  const { calls, invoked, model, action, settle } = await browserPage();
  action("toggle", "percussion");
  await settle();
  const sent = calls.length;
  const asked = invoked.length;
  action("pick", "percussion/snare.wav");
  action("pick", "kick.wav");
  action("pick", "percussion/snare.wav");
  action("pick", "kick.wav");
  await settle();
  await settle();
  // Each pick replaced the one before it while it waited its turn.
  assert.deepEqual(calls.slice(sent).map(call => call.row), [2]);
  assert.deepEqual(invoked.slice(asked).map(([, args]) => args.source.row), [2]);
  assert.equal(model().preview.title, "kick.wav");
});

test("an unset Library offers to choose its folder there", async () => {
  const { context, invoked, model, action, settle } = await browserPage();
  runInContext("view = {...view, hitsoundLibrary: null}; renderSampleBrowser();", context);
  action("source", "library");
  assert.equal(model().chooseLibrary, true);
  assert.deepEqual(model().rows.map(row => row.label), ["No library folder chosen yet."]);
  action("chooseLibrary");
  await settle();
  assert.equal(invoked.at(-1)[0], "pick_hitsound_library");
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
  assert.equal(model().defaults, undefined, "Classic keeps to the folders it can open");
  assert.deepEqual(model().folder, { label: "Mapset samples", title: "", canUp: false });
  assert.deepEqual(labels(), ["percussion", "kick.wav"]);
  // Entering a folder replaces the rows, so focus moves to the first new row.
  let focused = 0;
  get("sample-browser-entries").querySelector = () => ({ focus() { focused++; } });
  action("toggle", "percussion");
  assert.equal(focused, 1);
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
  // The Library in Classic: one folder at a time too.
  action("source", "library");
  await settle();
  assert.deepEqual(model().folder, { label: "Library", title: "C:/fixtures/library", canUp: false });
  assert.deepEqual(labels(), ["Drums"]);
  action("toggle", "Drums");
  await settle();
  assert.deepEqual(labels(), ["clap.wav", "snare.wav"]);
  assert.deepEqual(model().folder, { label: "Drums", title: "C:/fixtures/library/Drums", canUp: true });
  action("up");
  assert.deepEqual(labels(), ["Drums"]);
  action("source", "mapset");
  // Close folds the Browser through the same path as every other switch.
  runInContext("var toggled = []; togglePanel = panel => toggled.push(panel);", context);
  get("sample-browser-close").listeners.get("click")();
  assert.deepEqual(JSON.parse(runInContext("JSON.stringify(toggled)", context)), ["browser"]);
  runInContext('act({ kind: "browser_style", classic: false });', context);
  await settle();
  assert.equal(model().classic, false);
  assert.deepEqual(labels(), ["percussion", "kick.wav"]);
  action("toggle", "percussion");
  assert.deepEqual(labels(), ["percussion", "snare.wav", "kick.wav"]);
});

test("a busy editor greys out the sample rows and both buttons", async () => {
  const { context, model, action } = await browserPage();
  action("pick", "kick.wav");
  runInContext("view.busy = true; renderSampleBrowser();", context);
  assert.equal(model().busy, true);
  assert.equal(model().canAssign, false);
  assert.equal(model().canPick, false);
});

test("opening another map clears the Library's picked sample and open folders too", async () => {
  const { context, model, labels, action, settle } = await browserPage();
  action("source", "library");
  await settle();
  action("toggle", "Drums");
  await settle();
  action("pick", "Drums/snare.wav");
  await settle();
  runInContext('scene = {...scene, path: "other.osu"}; renderSampleBrowser();', context);
  await settle();
  assert.equal(model().preview, null);
  assert.deepEqual(labels(), ["Drums"]);
});

test("a very long sample list stops at 500 rows and asks for a search", async () => {
  const { context, model, action } = await browserPage();
  runInContext(`scene = {...scene, lanes: Array.from({length: 600}, (_, i) =>
    ({key: "s" + i, sourceKey: "s" + i, name: "s" + i, sample: "s" + String(i).padStart(3, "0") + ".wav"}))};
    renderSampleBrowser();`, context);
  assert.equal(model().rows.length, 501);
  assert.deepEqual(model().rows.at(-1), { kind: "note", key: "note:500", label: "Search to see more samples.", depth: 0 });
  action("filter", "s599");
  assert.deepEqual(model().rows.map(row => row.label), ["s599.wav"]);
});

test("a sample dragged from the Browser onto a lane row replaces that lane's sample", async () => {
  const { context, calls, get, action, settle } = await browserPage();
  const marks = [];
  const classList = name => ({ toggle() {}, add: cls => marks.push([name, "+", cls]), remove: cls => marks.push([name, "-", cls]) });
  get("grid").classList = classList("grid");
  const header = { getAttribute: name => name === "data-lane-row" ? "0" : null, classList: classList("normal") };
  get("lanes").querySelector = selector => selector === '.lane[data-lane-row="0"]' ? header : null;
  get("lanes").contains = () => false;
  runInContext("laneRows = [0, 1, 2]; lanePositions = new Map([[0, 0], [1, 1], [2, 2]]);", context);
  const lanes = get("lanes").listeners;
  const drag = path => ({
    target: { closest: () => header },
    dataTransfer: { types: ["application/x-dawsu-sample"], getData: () => path, dropEffect: "" },
    prevented: false, preventDefault() { this.prevented = true; },
  });

  action("drag", "percussion/snare.wav");
  const over = drag("percussion/snare.wav");
  lanes.get("dragover")(over);
  assert.equal(over.prevented, true, "the lane row accepts the drop");
  assert.equal(over.dataTransfer.dropEffect, "copy");
  assert.ok(marks.some(([name, op, cls]) => name === "normal" && op === "+" && cls === "sample-drop-target"));
  const dropped = drag("percussion/snare.wav");
  lanes.get("drop")(dropped);
  await settle();
  assert.equal(dropped.prevented, true);
  // Rust replaces the active lane's sample, so the lane under the drop is activated first.
  assert.deepEqual(calls.slice(-2), [{ kind: "lane", row: 0, operation: "activate" },
    { kind: "replace_sample", target: { map: "copy.osu", lane: "normal" }, path: "percussion/snare.wav", mapset: true }]);
  assert.ok(marks.at(-1)[1] === "-", "the highlight clears on drop");

  // A lane whose sample can't change (no canReplace here: Snare) takes no drop.
  header.getAttribute = name => name === "data-lane-row" ? "1" : null;
  action("drag", "kick.wav");
  const refused = drag("kick.wav");
  lanes.get("dragover")(refused);
  assert.equal(refused.prevented, false);
  const asked = calls.length;
  lanes.get("drop")(drag("kick.wav"));
  await settle();
  assert.equal(calls.length, asked);

  // Over the timeline the row comes from the pointer's height: 10 px down a 20 px row is lane 0.
  runInContext("scene.rowHeight = 20;", context);
  get("grid").getBoundingClientRect = () => ({ left: 0, top: 0 });
  get("scroll").scrollTop = 0;
  const grid = get("grid").listeners;
  action("drag", "kick.wav");
  const overGrid = { ...drag("kick.wav"), clientX: 50, clientY: 10 };
  grid.get("dragover")(overGrid);
  assert.equal(overGrid.prevented, true);
  assert.ok(marks.some(([name, op, cls]) => name === "grid" && op === "+" && cls === "sample-drop"));
  grid.get("drop")({ ...drag("kick.wav"), clientX: 50, clientY: 10 });
  await settle();
  assert.deepEqual(calls.slice(-2), [{ kind: "lane", row: 0, operation: "activate" },
    { kind: "replace_sample", target: { map: "copy.osu", lane: "normal" }, path: "kick.wav", mapset: true }]);
  // Below the last lane there is no row to land on.
  action("drag", "kick.wav");
  const below = { ...drag("kick.wav"), clientX: 50, clientY: 70 };
  grid.get("dragover")(below);
  assert.equal(below.prevented, false);
});
