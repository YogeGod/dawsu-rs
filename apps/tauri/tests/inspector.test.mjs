import assert from "node:assert/strict";
import { readdirSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const read = name => readFile(new URL("../dist/" + name, import.meta.url), "utf8");
const parts = new URL("../ui/src/parts/", import.meta.url);
const [html, css, ...sections] = await Promise.all([
  read("index.html"), readFile(new URL("../ui/src/legacy-chrome.css", import.meta.url), "utf8"),
  ...readdirSync(parts).filter(name => /^Inspector(?!Head)\w+\.svelte$/.test(name)).sort()
    .map(name => readFile(new URL(name, parts), "utf8")),
]);
const part = sections.join("\n");
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
      dawsuUi: { inspector: model => handed.push(JSON.parse(JSON.stringify(model))), update() {}, lanes() {} },
    },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await bootEditor(context);
  calls.length = 0;
  const evaluate = code => runInContext(code, context);
  const sent = async () => {
    await evaluate("pending");
    return calls.splice(0).filter(Boolean).map(action => JSON.parse(JSON.stringify(action)));
  };
  const model = () => JSON.parse(evaluate("JSON.stringify(inspectorModel())"));
  return { evaluate, get, sent, model, handed };
}

test("every Inspector section is a Svelte part in a section Layout… can move", () => {
  assert.equal(sections.length, 5);
  const mounts = [...inspector.matchAll(/<section class="inspector-part" aria-label="([^"]+)"><div data-ui-mount="([\w-]+)"><\/div><\/section>/g)]
    .map(match => [match[1], match[2]]);
  assert.deepEqual(mounts, [["Map preview", "map-preview"], ["Selection", "inspector-selection"], ["Lane", "inspector-lane"], ["Difficulty", "inspector-difficulty"],
    ["Bookmarks", "inspector-bookmarks"], ["History", "inspector-history"]]);
  // The classic browser still docks between Bookmarks and History.
  assert.match(inspector, /data-ui-mount="inspector-bookmarks"><\/div><\/section>\s*<div id="classic-browser-host" hidden><\/div>\s*<section class="inspector-part" aria-label="History">/);
  assert.doesNotMatch(inspector, /<details|<summary/);
  for (const id of ["add-bookmark", "bookmarks", "bookmarks-empty", "jump-bookmark", "rename-bookmark", "remove-bookmark", "history"]) {
    assert.doesNotMatch(html, new RegExp(`id="${id}"`), id);
  }
  assert.doesNotMatch(html, /data-command="(previous|next)Bookmark"/);
  // docking.js keeps History's scroll place by this id, now on the Svelte list.
  assert.match(part, /id="history"/);
  // The moved sections' markup and its rules are gone from the old files.
  for (const id of ["selected", "selection-fields", "selection-start-input", "volume-mode", "slider-body-fields",
    "insert", "protect", "audition", "hs", "active-lane-name", "active-lane-colour-swatch", "edit-range"]) {
    assert.doesNotMatch(html, new RegExp(`id="${id}"`), id);
  }
  assert.doesNotMatch(html, /data-volume=/);
  for (const dead of ["#selection-start-input", "#active-lane-colour-swatch", ".inspector-row", ".inspector-controls",
    ".inspector-value-button", "#apply-slider-body", ".selection-value"]) assert.ok(!css.includes(dead), dead);
  for (const dead of ["#bookmarks", "#history", ".inspector-header-action", ".inspector-actions", "#editor-inspector summary",
    "#editor-inspector details"]) assert.ok(!css.includes(dead), dead);
  // menus.js and the New difficulty shortcut click these by id.
  for (const id of ["lane-colour", "replace-sample", "generate", "new", "copy", "merge-settings"]) {
    assert.match(part, new RegExp(`id="${id}"`), id);
  }
});

test("the Selection model has v3's summary, Start, Volume and On object", async () => {
  const { evaluate, model } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true };
    scene = { path: "a.osu",
      lanes: [{ key: "k", name: "normal-hitclap", group: "normal", family: "normal-hitclap", index: 1, sample: "kick.wav" },
        { key: "c", name: "Clap" }],
      moments: [[1000, 80, true, [[0, true], [1, false]]], [1500, 80, true, [[0, true, false, 60]]]],
      selection: { count: 2, start: 1000, object: "2 object moments" } };
  `);
  assert.deepEqual(model().selection, {
    summary: "2 hits selected · Clap (Normal)", start: "00:01.000", volume: "60–80%", object: "2 object moments",
    sliderBody: null,
  });
  assert.equal(model().scope, "hits");
  assert.equal(model().hint, "Changes only the selected hits.");

  evaluate(`scene = { ...scene, moments: [[1000, 80, true, [[0, true], [1, true]]]],
    selection: { count: 1, start: 1000, object: "Circle" } };`);
  assert.equal(model().selection.summary, "1 hit selected · 2 lanes");
  assert.equal(model().selection.volume, "80%");

  evaluate("scene = { lanes: [], selection: { count: 1, start: 5, object: \"No object\" } };");
  assert.equal(model().selection.summary, "1 hit selected");
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
  assert.equal(busy.busy, true);
  assert.equal(busy.doc, false);
  assert.equal(busy.range, true);
  assert.deepEqual([busy.insertVolume, busy.protect, busy.audition, busy.hs], [70, true, true, false]);
  evaluate("view.busy = false;");
  assert.equal(model().doc, true);
  evaluate('view.fault = "The editor stopped";');
  assert.deepEqual([model().busy, model().doc], [true, false], "a fault stops the Inspector like a job");
  evaluate("view.fault = null; scene = null;");
  assert.deepEqual([model().doc, model().range, model().lane], [false, false, null]);
  assert.equal("map" in model(), false);
});

test("Lane carries the picked colour or none, never the sound colour", async () => {
  const { evaluate, model } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0 };
    scene = { path: "a.osu", lanes: [{ key: "k", name: "Kick", sample: "kick.wav", family: "soft-hitclap",
      colourKey: "File|kick.wav", canReplace: true }] };
  `);
  assert.deepEqual(model().lane, { title: "Kick", sample: "kick.wav", colour: null, canColour: true, canReplace: true });
  evaluate('Object.assign(scene.lanes[0], { name: "soft-hitclap", group: "soft", index: 1 });');
  assert.equal(model().lane.title, "Clap · Soft set", "v3's lane title");
  assert.equal(evaluate("laneTitle(scene.lanes[0])"), "Soft Clap", "the lane filter's name is unchanged");
  evaluate('renamedLanes.set("File|kick.wav", "Kick 2");');
  assert.equal(model().lane.title, "Kick 2");
  evaluate('renamedLanes.clear(); Object.assign(scene.lanes[0], { name: "Kick", group: null });');
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
  evaluate(`view = { busy: false, activeLane: null }; scene = { path: "a.osu", lanes: [{ key: "k", name: "Kick" }],
    moments: [], selection: { count: 1, start: 0, object: "Circle" } };`);
  handed.length = 0;
  evaluate("renderSelection(); view.activeLane = 0; renderActiveLane();");
  assert.equal(handed.length, 2);
  assert.equal(handed[0].selection.summary, "1 hit selected");
  assert.equal(handed[1].lane.title, "Kick");
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify(currentInspector())")), handed[1]);
});

test("two show() calls with the same view hand the Inspector over at most once, while a changed selection updates immediately", async () => {
  const { evaluate, handed } = await frontend();
  const baseScene = {
    path: "a.osu",
    difficulties: ["a.osu"],
    lanes: [{ key: "k", name: "Kick" }],
    moments: [],
    viewStart: 0,
    zoom: 0.5,
    rowHeight: 30,
    bookmarks: [],
    history: [],
    historyPosition: 0,
    selection: { count: 1, start: 0, object: "Circle" },
  };
  const view = { busy: false, hasDocument: true, activeLane: 0, position: 0, duration: 9000, scene: baseScene };
  handed.length = 0;
  evaluate(`const sameView = ${JSON.stringify(view)}; show(sameView); show(sameView);`);
  assert.equal(handed.length, 1, "two show() calls with the same view hand the Inspector over once");
  assert.equal(handed[0].selection.summary, "1 hit selected");

  // A subsequent update without scene (e.g. playhead tick) with unchanged Inspector state also skips hand-over.
  evaluate(`show(${JSON.stringify({ ...view, position: 250, scene: undefined })});`);
  assert.equal(handed.length, 1, "an unchanged Inspector model skips redundant hand-overs");

  // A changed selection still reaches the Inspector immediately.
  const nextScene = {
    ...baseScene,
    moments: [[1000, 80, true, [[0, true]]]],
    selection: { count: 1, start: 1000, object: "Slider head" },
  };
  evaluate(`show(${JSON.stringify({ ...view, scene: nextScene })});`);
  assert.equal(handed.length, 2);
  assert.equal(handed[1].selection.start, "00:01.000");
  assert.equal(handed[1].selection.object, "Slider head");
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
  evaluate(`inspectorAction("insert", NaN); inspectorAction("insert", null); inspectorAction("insert", 0);
    inspectorAction("insert", 140); inspectorAction("toggle", "hs_mode", true);
    inspectorAction("toggle", "protect", false); inspectorAction("toggle", "audition", true);`);
  assert.deepEqual(await sent(), [
    // A blank Insert volume field sets 100 %, as before; 0 stays 0.
    { kind: "insert_volume", value: 100 }, { kind: "insert_volume", value: 100 }, { kind: "insert_volume", value: 0 },
    { kind: "insert_volume", value: 100 }, { kind: "hs_mode", on: true }, { kind: "protect", on: false },
    { kind: "audition", on: true },
  ]);
  evaluate(`inspectorAction("scope", "selected");`);
  assert.deepEqual(await sent(), [{ kind: "volume_scope", path: "Slider.osu", mode: "selected", lanes: [] }]);
  assert.equal(model().scope, "selected");
  assert.equal(model().hint, "Select lanes to change their volume.");
  evaluate(`inspectorAction("scope", "all");`);
  assert.deepEqual(await sent(), [{ kind: "volume_scope", path: "Slider.osu", mode: "all", lanes: null }]);
  assert.equal(model().scope, "all");
  assert.equal(model().hint, "Changes all sounds at the selected times.");
  evaluate(`inspectorAction("scope", "hits");`);
  assert.deepEqual(await sent(), [{ kind: "volume_scope", path: "Slider.osu", mode: "hits", lanes: null }]);
  assert.equal(model().scope, "hits");
  assert.equal(model().hint, "Changes only the selected hits.");
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
  assert.equal(evaluate("opened[3].some(([label]) => label === 'Clear colour')"), false);
  assert.equal(evaluate("opened[2][0].attributes['aria-label']"), "Lane colour");
  evaluate('opened[3].find(([label]) => label === "Apply")[1]()');
  assert.deepEqual(await sent(), []);
  assert.equal(evaluate("laneColours.size"), 0);
  // A pick from the custom colour field counts, even grey.
  evaluate('inspectorAction("laneColour"); opened[2][2].children[1].events.get("input")(); ' +
    'opened[3].find(([label]) => label === "Apply")[1]()');
  await sent();
  assert.equal(evaluate('laneColours.get("Skin|clap")'), "#9e9e9e");
});

test("every open, command and toggle a part sends reaches its inspectorAction case", async () => {
  const { evaluate, sent } = await frontend();
  const keys = kind => [...new Set([...part.matchAll(new RegExp(`on\\.${kind}\\("([\\w]+)"`, "g"))].map(match => match[1]))];
  const opens = keys("open");
  assert.deepEqual(opens.sort(), ["copy", "generate", "laneColour", "mergeSettings", "new", "replaceSample"]);
  const openers = { laneColour: "openLaneColour", replaceSample: "openReplaceSample", generate: "openGenerate",
    new: "openNewDifficulty", copy: "openCopy", mergeSettings: "openMergeSettings" };
  evaluate(`globalThis.reached = []; run = task => task();
    ${Object.values(openers).map(name => `${name} = () => reached.push("${name}");`).join(" ")}`);
  for (const kind of opens) evaluate(`inspectorAction("${kind}")`);
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify(reached)")), opens.map(kind => openers[kind]));

  const commands = keys("command");
  assert.deepEqual(commands, ["previousBookmark", "nextBookmark", "quantizeDialog", "repeat", "duplicate", "delete", "patternStamp", "editRange",
    "scaleByVolume"]);
  await sent();
  for (const id of commands) evaluate(`inspectorAction("command", "${id}")`);
  assert.deepEqual(await sent(), commands.map(id => ({ kind: "command", id })));
  // Each id is in the editor's command catalogue, as the old data-command buttons' ids were.
  const catalogue = (await Promise.all(["dawsu-application/src/commands.rs", "dawsu-editor/src/editing_commands.rs"]
    .map(path => readFile(new URL("../../../crates/" + path, import.meta.url), "utf8")))).join(" ");
  const known = new Set([...catalogue.matchAll(/(?:CommandDefinition::new|\bdef)\(\s*"(\w+)"/g)].map(match => match[1]));
  for (const id of commands) assert.ok(known.has(id), id + " is not a catalogue command");

  // Bookmarks: + and the buttons that act on the picked row, a row pick and a History step.
  const bookmarkKinds = keys("bookmark");
  assert.deepEqual(bookmarkKinds, ["add", "rename", "goTo", "remove"]);
  assert.match(part, /on\.pickBookmark\(row\.time\)/);
  assert.match(part, /on\.history\(step\.position\)/);
  evaluate(`view = { busy: false, position: 750 }; scene = { path: "a.osu", bookmarks: [{ time: 500, label: "Intro" }],
    history: ["Draw hit"], historyPosition: 1 }; editBookmark = mark => reached.push("edit " + (mark?.time ?? "new"));
    handOverInspector = () => {}; reached.length = 0;`);
  await sent();
  evaluate('inspectorAction("pickBookmark", 500)');
  for (const kind of bookmarkKinds) evaluate(`inspectorAction("bookmark", "${kind}")`);
  evaluate('inspectorAction("history", 0)');
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify(reached)")), ["edit new", "edit 500"]);
  assert.deepEqual(await sent(), [{ kind: "bookmark_jump", time: 500 }, { kind: "bookmark_jump", time: 500 },
    { kind: "bookmark_edit", path: "a.osu", time: 500, label: null }, { kind: "history", position: 0 }]);

  const toggles = keys("toggle");
  assert.deepEqual(toggles.sort(), ["audition", "protect"]);
  // Hitsounding mode moved to Preferences › This map, out of the Difficulty section.
  assert.doesNotMatch(part, /aria-label="Hitsounding mode"/);
  evaluate("view = { busy: false }; scene = { path: 'a.osu' };");
  for (const kind of toggles) evaluate(`inspectorAction("toggle", "${kind}", true)`);
  assert.deepEqual(await sent(), toggles.map(kind => ({ kind, on: true })));
});

test("Layout… moves each Inspector section whole, never the buttons inside a Svelte section", async () => {
  const docking = await read("docking.js");
  // Sections get a grip like ribbon groups and drop only beside another section of the Inspector.
  assert.match(docking, /const inspectorSections = \(\) => \[\.\.\.\(panels\.inspector\?\.children \?\? \[\]\)\]\.filter\(item => item\.matches\("section"\)\)/);
  assert.match(docking, /for \(const section of inspectorSections\(\)\) \{\s*barHandles\.set\(moveHandle\(section,/);
  assert.match(docking, /under\.closest\("#editor-inspector > section"\)/);
  // Cancel and Reset put the sections back.
  assert.match(docking, /inspector: \[\.\.\.panels\.inspector\.children\]/);
  assert.match(docking, /panels\.inspector\.append\(\.\.\.draft\.inspector\)/);
  assert.match(docking, /panels\.inspector\.append\(\.\.\.initialInspector\)/);
  assert.match(docking, /button\.closest\("[^"]*\[data-ui-mount\][^"]*"\)/);
  // History's list only exists once the bundle mounts it, after docking.js starts, so it is looked up each time.
  assert.match(docking, /const scrollAreas = \(\) => \[[^\]]*byId\("history"\)[^\]]*\]\.filter\(Boolean\)/);
  assert.match(docking, /for \(const area of scrollAreas\(\)\)/);
  // Each section names itself for the grip's tooltip.
  for (const [, label] of inspector.matchAll(/<section class="inspector-part" aria-label="([^"]+)">/g)) assert.ok(label);
  assert.match(css, /#editor-inspector > section > \.layout-move-handle/);
});

test("a problem mark in Difficulty moves the playhead to its time and centres it", async () => {
  const { evaluate, sent } = await frontend();
  evaluate(`view = { busy: false, hasDocument: true };
    scene = { path: "a.osu", lanes: [], moments: [], zoom: 0.5, rowHeight: 30, viewStart: 0 };
    globalThis.views = []; changeView = (start, zoom) => views.push([start, zoom]); grid.clientWidth = 1000;`);
  assert.equal(evaluate('inspectorAction("jump", 50000)'), true);
  assert.deepEqual(await sent(), [{ kind: "seek", ms: 50000 }]);
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify(views)")), [[49000, 0.5]]);
  assert.equal(evaluate('inspectorAction("jump", NaN)'), false);
  evaluate("view.busy = true;");
  assert.equal(evaluate('inspectorAction("jump", 50000)'), false, "nothing moves while a job runs");
});

test("the problem strip's data is handed over from the editor's state and kept until it changes", async () => {
  const { evaluate } = await frontend();
  evaluate(`grid.clientWidth = 1000;
    view = { busy: false, duration: 200000, outputProblems: [{ target: "a.osu", timeline: "a.osu", time: 500, message: "m" }] };
    scene = { path: "a.osu", version: "Hard", lanes: [], viewStart: 4000, zoom: 0.5, orphanedHits: [700],
      moments: [[500, 100, true, []], [700, 100, false, [[0, false]]], [900, 100, true, [[0, false]]]] };`);
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify(inspectorModel().problemStrip)")), {
    path: "a.osu", name: "Hard", duration: 200000, problems: [{ target: "a.osu", timeline: "a.osu", time: 500, message: "m" }],
    orphaned: [700], objects: [500, 900], viewStart: 4000, viewWidth: 2000,
  });
  assert.equal(evaluate("inspectorModel().problemStrip === inspectorModel().problemStrip"), true, "unchanged state, same data");
  evaluate("view = { ...view, outputProblems: [] };");
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify(inspectorModel().problemStrip.problems)")), []);
  evaluate("scene = null;");
  assert.equal(evaluate("inspectorModel().problemStrip"), null);
});
