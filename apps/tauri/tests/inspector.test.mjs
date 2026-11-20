import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const read = name => readFile(new URL("../dist/" + name, import.meta.url), "utf8");
const [app, html, css, part] = await Promise.all([
  ...["app.js", "index.html", "style.css"].map(read),
  readFile(new URL("../ui/src/parts/InspectorSections.svelte", import.meta.url), "utf8"),
]);
const inspector = html.match(/<aside id="editor-inspector"[^>]*>([\s\S]*?)<\/aside>/)?.[1] ?? "";

async function frontend() {
  const controls = new Map();
  const calls = [];
  const handed = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, attributes: {},
      style: { setProperty(name, value) { this[name] = value; } }, classList: { add() {}, toggle() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute(name, value) { this.attributes[name] = String(value); },
      append(...children) { this.children.push(...children); },
      prepend() {}, focus() {},
      replaceChildren(...children) { this.children = children; },
    };
  }
  const get = id => {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: {
      __TAURI__: {
        core: { invoke: async (_name, args) => { calls.push(args?.action); return { busy: false, hasDocument: true, position: 0 }; } },
        event: { listen() {} },
      },
      dawsuUi: { inspector: model => handed.push(JSON.parse(JSON.stringify(model))) },
    },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await runInContext(app, context);
  calls.length = 0;
  const evaluate = code => runInContext(code, context);
  const sent = async () => {
    await evaluate("pending");
    return calls.splice(0).filter(Boolean).map(action => JSON.parse(JSON.stringify(action)));
  };
  const model = () => JSON.parse(evaluate("JSON.stringify(inspectorModel())"));
  return { evaluate, get, sent, model, handed };
}

test("Selection, Lane and Difficulty are one Svelte part; Bookmarks and History stay folded sections", () => {
  assert.match(inspector, /^\s*<div data-ui-mount="inspector"><\/div>/);
  const headings = [...inspector.matchAll(/<section[^>]*>\s*<details open>\s*<summary><h2>([^<]*)<\/h2><\/summary>/g)]
    .map(match => match[1]);
  assert.deepEqual(headings, ["Bookmarks", "History"]);
  for (const [, summary] of inspector.matchAll(/<summary>([\s\S]*?)<\/summary>/g)) assert.doesNotMatch(summary, /<button/);
  for (const id of ["add-bookmark", "bookmarks", "jump-bookmark", "rename-bookmark", "remove-bookmark",
    "classic-browser-host", "history"]) assert.match(inspector, new RegExp(`id="${id}"`), id);
  for (const command of ["previousBookmark", "nextBookmark"]) assert.match(inspector, new RegExp(`data-command="${command}"`));
  // The moved sections' markup and its rules are gone from the old files.
  for (const id of ["selected", "selection-fields", "selection-start-input", "volume-mode", "slider-body-fields",
    "insert", "protect", "audition", "hs", "active-lane-name", "active-lane-colour-swatch", "edit-range"]) {
    assert.doesNotMatch(html, new RegExp(`id="${id}"`), id);
  }
  assert.doesNotMatch(html, /data-volume=/);
  for (const dead of ["#selection-start-input", "#active-lane-colour-swatch", ".inspector-row", ".inspector-controls",
    ".inspector-value-button", "#apply-slider-body", ".selection-value"]) assert.ok(!css.includes(dead), dead);
  assert.match(css, /#editor-inspector summary::-webkit-details-marker\s*\{\s*display:\s*none/);
  // menus.js and the New difficulty shortcut click these by id.
  for (const id of ["lane-colour", "replace-sample", "generate", "new", "copy", "merge-settings"]) {
    assert.match(part, new RegExp(`id="${id}"`), id);
  }
});

test("the Selection model has the count, lanes, Start, Volume and On object", async () => {
  const { evaluate, model } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true };
    scene = { path: "a.osu",
      lanes: [{ key: "k", name: "Kick", sample: "kick.wav" }, { key: "c", name: "Clap" }],
      moments: [[1000, 80, true, [[0, true], [1, false]]], [1500, 80, true, [[0, true, false, 60]]]],
      selection: { count: 2, start: 1000, object: "2 object moments" } };
  `);
  assert.deepEqual(model().selection, {
    count: "2 hits", lanes: " · Kick", start: "00:01.000", volume: "60–80%", object: "2 object moments", sliderBody: null,
  });
  assert.equal(model().scope, "all");
  assert.equal(model().hint, "Changes all sounds at the selected times.");

  evaluate(`scene = { ...scene, moments: [[1000, 80, true, [[0, true], [1, true]]]],
    selection: { count: 1, start: 1000, object: "Circle" } };`);
  assert.equal(model().selection.count, "1 hit");
  assert.equal(model().selection.lanes, " · 2 lanes");
  assert.equal(model().selection.volume, "80%");

  evaluate("scene = { lanes: [], selection: { count: 1, start: 5, object: \"No object\" } };");
  assert.equal(model().selection.lanes, "");
  assert.equal(model().selection.volume, "");

  evaluate("scene = { selection: null };");
  assert.equal(model().selection, null);
});

test("enablement follows the old markup: busy waits, New and Copy never did", async () => {
  const { evaluate, model } = await frontend();
  evaluate(`view = { busy: true, hasDocument: true, activeLane: 0, audition: true };
    scene = { path: "a.osu", range: [1, 2], insertVolume: 70, protect: true, hs: false,
      lanes: [{ key: "k", name: "Kick", colourKey: "File|kick.wav", canReplace: true }], moments: [], selection: null };`);
  const busy = model();
  assert.equal(busy.map, true);
  assert.equal(busy.busy, true);
  assert.equal(busy.doc, false);
  assert.equal(busy.range, true);
  assert.deepEqual([busy.insertVolume, busy.protect, busy.audition, busy.hs], [70, true, true, false]);
  evaluate("view.busy = false;");
  assert.equal(model().doc, true);
  evaluate("scene = null;");
  assert.deepEqual([model().map, model().doc, model().range, model().lane], [false, false, false, null]);
});

test("Lane carries the picked colour or none, never the sound colour", async () => {
  const { evaluate, model } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0 };
    scene = { path: "a.osu", lanes: [{ key: "k", name: "Kick", sample: "kick.wav", family: "soft-hitclap",
      colourKey: "File|kick.wav", canReplace: true }] };
  `);
  assert.deepEqual(model().lane, { title: "Kick", sample: "kick.wav", colour: null, canColour: true, canReplace: true });
  evaluate('laneColours.set("File|kick.wav", "#cc50a2");');
  assert.equal(model().lane.colour, "#cc50a2");
  evaluate('scene.lanes[0].sample = "Kick";');
  assert.equal(model().lane.sample, "", "the file name shows only when it differs from the lane name");
  evaluate("view.activeLane = null;");
  assert.equal(model().lane, null);
  assert.equal(evaluate("typeof soundColour + typeof laneColour"), "undefinedundefined");
});

test("renderSelection and renderActiveLane hand the model to the Svelte part", async () => {
  const { evaluate, handed } = await frontend();
  evaluate(`view = { busy: false, activeLane: 0 }; scene = { path: "a.osu", lanes: [{ key: "k", name: "Kick" }],
    moments: [], selection: { count: 1, start: 0, object: "Circle" } };`);
  handed.length = 0;
  evaluate("renderSelection(); renderActiveLane();");
  assert.equal(handed.length, 2);
  assert.equal(handed[0].selection.count, "1 hit");
  assert.equal(handed[1].lane.title, "Kick");
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify(currentInspector())")), handed[1]);
});

test("Start takes osu! editor times and plain milliseconds", async () => {
  const { evaluate, sent } = await frontend();
  evaluate(`view = { busy: false, hasDocument: true };
    scene = { path: "a.osu", lanes: [], moments: [], selection: { count: 1, start: 105263, object: "Circle" } };`);
  const move = async text => {
    const moved = evaluate(`inspectorAction("moveStart", ${JSON.stringify(text)})`);
    const [action] = await sent();
    assert.equal(moved, !!action, text);
    return action?.edit.position ?? null;
  };
  assert.equal(await move("01:45.263"), null, "the time already shown moves nothing");
  assert.equal(await move("01:46.5"), 106500);
  assert.equal(await move("1:05:020 (1,2) -"), 65020, "a modding timestamp");
  assert.equal(await move("1330.5"), 1330.5);
  assert.equal(await move(" 2000 "), 2000);
  assert.equal(await move("soon"), null);
  assert.equal(await move("12:3x"), null);
  evaluate(`inspectorAction("moveStart", "1330.5")`);
  assert.deepEqual(await sent(), [{ kind: "timeline_edit", path: "a.osu", edit: { kind: "selection_start", position: 1330.5 } }]);
});

test("Change volume, -5 and +5, the scope, the slider body and the switches send the old actions", async () => {
  const { evaluate, sent, model } = await frontend();
  evaluate(`view = { busy: false, hasDocument: true };
    scene = { path: "Slider.osu", lanes: [], moments: [], selection: { count: 1, start: 1000, object: "Slider head",
      sliderBody: { index: 2, set: "soft", whistleSet: null, whistle: false, volume: 60 } } };`);
  assert.deepEqual(model().selection.sliderBody,
    { key: '["Slider.osu",{"index":2,"set":"soft","whistleSet":null,"whistle":false,"volume":60}]',
      set: "soft", whistle: false, whistleSet: "auto", volume: 60 });
  evaluate(`inspectorAction("volume", -15, true); inspectorAction("volume", 0, false); inspectorAction("volume", 140, false);
    inspectorAction("volume", NaN, false); inspectorAction("nudge", -5); inspectorAction("nudge", 5);`);
  assert.deepEqual(await sent(), [
    { kind: "volume", value: -15, relative: true }, { kind: "volume", value: 0, relative: false },
    { kind: "volume", value: 100, relative: false },
    { kind: "volume", relative: true, value: -5 }, { kind: "volume", relative: true, value: 5 },
  ]);
  evaluate(`inspectorAction("sliderBody", { set: "drum", whistleSet: "normal", whistle: true, volume: 80 });
    inspectorAction("sliderBody", { set: "", whistleSet: "auto", whistle: false, volume: 101 });`);
  assert.deepEqual(await sent(), [
    { kind: "slider_body", path: "Slider.osu", index: 2, set: "drum", whistle_set: "normal", whistle: true, volume: 80 },
  ]);
  evaluate(`inspectorAction("insert", 140); inspectorAction("toggle", "hs_mode", true);
    inspectorAction("toggle", "protect", false); inspectorAction("toggle", "audition", true);`);
  assert.deepEqual(await sent(), [
    { kind: "insert_volume", value: 100 }, { kind: "hs_mode", on: true }, { kind: "protect", on: false },
    { kind: "audition", on: true },
  ]);
  evaluate(`inspectorAction("scope", "selected");`);
  assert.deepEqual(await sent(), [{ kind: "volume_scope", path: "Slider.osu", lanes: [] }]);
  assert.equal(model().scope, "selected");
  assert.equal(model().hint, "Select lanes to change their volume.");
  evaluate(`view.busy = true; inspectorAction("volume", 50, false); inspectorAction("moveStart", "5");`);
  assert.deepEqual(await sent(), [], "nothing is sent while a job runs");
});

test("the lane colour dialog opens on grey for an unpicked lane and writes nothing until a pick", async () => {
  const { evaluate, sent } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0 };
    scene = { path: "a.osu", rowHeight: 30, lanes: [{ name: "Clap", family: "normal-hitclap", colourKey: "Skin|clap" }], moments: [] };
    let opened;
    modal = (...args) => { opened = args; };
    closeModal = () => {};
  `);
  evaluate('inspectorAction("laneColour")');
  assert.equal(evaluate("opened[2][2].children[1].value"), "#9e9e9e");
  assert.equal(evaluate("opened[3].some(([label]) => label === 'Clear tint')"), false);
  evaluate('opened[3].find(([label]) => label === "Apply")[1]()');
  assert.deepEqual(await sent(), []);
  assert.equal(evaluate("laneColours.size"), 0);
  // A pick from the custom colour field counts, even grey.
  evaluate('inspectorAction("laneColour"); opened[2][2].children[1].events.get("input")(); ' +
    'opened[3].find(([label]) => label === "Apply")[1]()');
  await sent();
  assert.equal(evaluate('laneColours.get("Skin|clap")'), "#9e9e9e");
});

test("Bookmarks list as rows sized to the marks and say when there are none", async () => {
  const { evaluate, get } = await frontend();
  evaluate(`view = { busy: false }; scene = { path: "a.osu", bookmarks: [
    { time: 500, label: "Intro" }, { time: 1000, label: "" }, { time: 2000, label: "Kiai" }] }; renderBookmarks();`);
  assert.equal(get("bookmarks").size, 3);
  assert.equal(get("bookmarks").hidden, false);
  assert.equal(get("bookmarks-empty").hidden, true);
  evaluate(`scene = { path: "b.osu", bookmarks: Array.from({ length: 9 }, (_, i) => ({ time: i * 100, label: "" })) };
    renderBookmarks();`);
  assert.equal(get("bookmarks").size, 6);
  evaluate('scene = { path: "c.osu", bookmarks: [] }; renderBookmarks();');
  assert.equal(get("bookmarks").hidden, true);
  assert.equal(get("bookmarks-empty").hidden, false);
});
