import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor, readEditorSource } from "./editor-runtime.mjs";

const read = name => readFile(new URL("../dist/" + name, import.meta.url), "utf8");
const app = readEditorSource();
const css = await readFile(new URL("../ui/src/legacy-chrome.css", import.meta.url), "utf8");

async function frontend() {
  const controls = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, attributes: {}, classes: [],
      style: { setProperty(name, value) { this[name] = value; } },
      classList: { add(name) { this.owner.classes.push(name); }, toggle() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute(name, value) { this.attributes[name] = String(value); },
      append(...children) { this.children.push(...children); },
      prepend(...children) { this.children.unshift(...children); }, focus() {},
      replaceChildren(...children) { this.children = children; },
    };
  }
  const create = () => {
    const made = element();
    made.classList.owner = made;
    return made;
  };
  const get = id => {
    if (!controls.has(id)) controls.set(id, create());
    return controls.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: create, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(args?.action);
        return { busy: false, hasDocument: true, position: 0 };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await bootEditor(context);
  return { calls, evaluate: code => runInContext(code, context), get };
}

const fixture = `
  view = { busy: false, hasDocument: true, activeLane: 1 };
  scene = { path: "a.osu", rowHeight: 30, moments: [], lanes: [
    { key: "n", name: "soft-hitnormal", family: "soft-hitnormal", group: "soft", index: 1, colourKey: "Skin|soft-hitnormal", hitCount: 3, canAddLayer: true },
    { key: "c2", name: "soft-hitclap2", family: "soft-hitclap", group: "soft", index: 2, colourKey: "Skin|soft-hitclap2", hitCount: 0 },
    { key: "s", name: "soft-sliderslide", family: "soft-sliderslide", group: "soft", index: 1, colourKey: "Skin|soft-sliderslide" },
    { key: "k", name: "Kick", sample: "kick.wav", colourKey: "File|kick.wav" },
    { key: "n0", name: "soft-hitnormal", family: "soft-hitnormal", group: "soft", index: 0, colourKey: "Skin|soft-hitnormal0" },
    { key: "sw", name: "soft-sliderwhistle", family: "soft-sliderwhistle", group: "soft", index: 1, colourKey: "Skin|soft-sliderwhistle" },
    { key: "t", name: "soft-slidertick", family: "soft-slidertick", group: "soft", index: 1, colourKey: "Skin|soft-slidertick" },
  ] };
  renderLanes();
`;
// renderLanes() hands the Svelte "lanes" part plain rows (ui/tests/lane-headers checks the markup).
const headers = evaluate => JSON.parse(evaluate("JSON.stringify(laneHeaders)"));
const lanes = evaluate => headers(evaluate).rows.filter(row => row.type === "lane");

test("a lane header row carries its number, sound name, set tag, icon and controls", async () => {
  const { evaluate, get } = await frontend();
  evaluate(fixture);
  // Normal's skin lane is a layer of Normal (index 1), so it sits under it.
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",0,4,1,2,5,6,3]');
  const rows = headers(evaluate).rows;
  assert.equal(rows[0].type, "folder");
  assert.deepEqual(rows.slice(1).map(row => [row.name, row.tag, row.icon, row.number]), [
    ["Normal", "Soft", "normal", 1], ["Normal", "Soft · skin", "normal", 2], ["Clap 2", "Soft", "clap", 3],
    ["Slide", "Soft", "slide", 4], ["Slide whistle", "Soft", "whistle", 6], ["Tick", "Soft", "tick", 7],
    ["Kick", "File", "file", 5],
  ]);
  const [normal, skin, clap, , , , kick] = rows.slice(1);
  assert.deepEqual(normal, {
    type: "lane", key: "lane:0", row: 0, number: 1, name: "Normal", label: "soft-hitnormal", display: "soft-hitnormal",
    title: "soft-hitnormal. Click to select and audition; Ctrl/Cmd toggles; Shift selects a range",
    tag: "Soft", icon: "normal", colour: null, folder: "soft", active: false, selected: false,
    muted: false, solo: false, quiet: false, layer: false,
    family: { collapsed: false, label: "Collapse soft-hitnormal layers" },
    addLayer: "Add numbered soft-hitnormal layer",
    remove: { label: "Remove Normal", disabled: true },
  });
  assert.equal(clap.active, true, "view.activeLane");
  assert.equal(skin.layer, true, "a layer under its main lane is indented");
  assert.equal(clap.layer, false, "a numbered layer without its main lane is an ordinary lane");
  assert.equal(kick.folder, null, "a map file lane sits outside the folders");
  assert.equal(get("lanes").style["--lane-row-height"], "30px");
  assert.equal(get("lanes").children.length, 0, "app.js builds no header nodes any more");

  evaluate('renamedLanes.set("Skin|soft-hitclap2", "Snare"); renderLanes();');
  assert.equal(lanes(evaluate)[2].name, "Snare");
  assert.equal(lanes(evaluate)[2].tag, "Soft");

  evaluate("showLaneInfo = true; renderLanes();");
  assert.equal(lanes(evaluate)[0].name, "soft-hitnormal (idx 1)");
  assert.equal(lanes(evaluate)[0].display, "soft-hitnormal", "M and S keep the plain name");

  evaluate("showLaneInfo = false; view.panels = { laneNames: false }; renderLanes();");
  assert.equal(headers(evaluate).names, false);
});

test("a lane is grey until the mapper picks a colour; mute and solo grey the others out", async () => {
  const { evaluate } = await frontend();
  evaluate(fixture);
  assert.ok(lanes(evaluate).every(row => row.colour === null), "no sound-colour fallback");
  evaluate('laneColours.set("Skin|soft-hitclap2", "#d04932"); scene.lanes[0].muted = true; renderLanes();');
  assert.equal(lanes(evaluate)[2].colour, "#d04932");
  assert.deepEqual(lanes(evaluate).map(row => row.quiet), [true, false, false, false, false, false, false]);
  evaluate("scene.lanes[2].solo = true; renderLanes();");
  assert.deepEqual(lanes(evaluate).map(row => row.quiet), [true, true, true, false, true, true, true]);
});

test("the part gets the rows only when they change", async () => {
  const { evaluate } = await frontend();
  evaluate("const handed = []; window.dawsuUi = { lanes: rows => handed.push(rows) };" + fixture + "renderLanes();");
  assert.equal(evaluate("handed.length"), 1);
  evaluate("view.busy = true; renderLanes();");
  assert.equal(evaluate("handed.length"), 2, "busy disables Add layer");
  assert.equal(evaluate("currentLaneHeaders() === handed[1]"), true);
});

test("clicking a lane selects and auditions it like its name", async () => {
  const { calls, evaluate } = await frontend();
  evaluate(fixture);
  evaluate("laneHeaderAction('select', 1, {})");
  await evaluate("pending");
  assert.deepEqual(JSON.parse(JSON.stringify(calls.filter(Boolean))), [{ kind: "lane", row: 1, operation: "audition" }]);
  assert.equal(evaluate("JSON.stringify([...selectionLaneKeys])"), '["c2"]');
  assert.equal(lanes(evaluate)[2].selected, true);
  // Ctrl, Cmd and Shift change the selection without auditioning.
  evaluate("laneHeaderAction('select', 0, { ctrlKey: true })");
  await evaluate("pending");
  assert.equal(calls.filter(Boolean).length, 1);
  assert.equal(evaluate("JSON.stringify([...selectionLaneKeys])"), '["c2","n"]');
  // Nothing happens while a job runs.
  evaluate("view.busy = true; laneHeaderAction('select', 2, {})");
  assert.equal(evaluate("JSON.stringify([...selectionLaneKeys])"), '["c2","n"]');
});

test("the Inspector and the lane filter use the same sound and set names", async () => {
  const { evaluate } = await frontend();
  evaluate(fixture);
  assert.equal(evaluate("inspectorModel().lane.title"), "Clap 2 · Soft set");
  evaluate(`scene.moments = [[1000, 70, true, [[0, true]]]];
    scene.selection = { count: 1, start: 1000, object: "Circle" };`);
  assert.equal(evaluate("inspectorModel().selection.summary"), "1 hit selected · Normal (Soft)");
  evaluate('popoverAction("laneFilter", "soft clap")');
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",1]');
});

test("folder rows carry the set name, lane count, used count and fold state", async () => {
  const { evaluate } = await frontend();
  evaluate(fixture);
  assert.deepEqual(headers(evaluate).rows[0], {
    type: "folder", key: "folder:soft", set: "soft", name: "Soft set", count: 6, used: 1,
    collapsed: false, muted: false, solo: false,
  });
  evaluate("laneHeaderAction('folder', 'soft')");
  assert.equal(headers(evaluate).rows[0].collapsed, true);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",3]');
});

test("the drum view keeps its rows and names and adds no tags", async () => {
  const { evaluate } = await frontend();
  evaluate(fixture + `view.drumView = { enabled: true, name: "Kit" };
    scene.drumRows = [{ row: 1, name: "Snare" }, { row: null, name: "Ghost" }, { row: 3, name: "Kick drum" }];
    renderLanes();`);
  const rows = headers(evaluate).rows;
  assert.deepEqual(rows.map(row => row.name), ["Snare", "Ghost", "Kick drum"]);
  assert.deepEqual(rows.map(row => row.type), ["lane", "drum", "lane"]);
  assert.deepEqual(rows.map(row => row.number), [1, undefined, 3]);
  assert.equal(rows[1].title, "Ghost · Unassigned");
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[1,null,3]");
  assert.equal(rows[0].tag, null);
  assert.equal(rows[0].folder, null);
  assert.equal(rows[0].icon, "clap");
  evaluate("view.drumView.enabled = false; renderLanes();");
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",0,4,1,2,5,6,3]');
});

async function drumSetupEditor() {
  const invoked = [];
  const controls = new Map();
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, attributes: {}, classes: [],
      style: { setProperty(name, value) { this[name] = value; } },
      classList: { add(name) { this.owner.classes.push(name); }, toggle() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute(name, value) { this.attributes[name] = String(value); },
      append(...children) { this.children.push(...children); },
      prepend(...children) { this.children.unshift(...children); }, focus() {},
      showModal() { this.open = true; }, close() { this.open = false; },
      replaceChildren(...children) { this.children = children; },
    };
  }
  const create = () => { const made = element(); made.classList.owner = made; return made; };
  const get = id => { if (!controls.has(id)) controls.set(id, create()); return controls.get(id); };
  const context = createContext({
    document: { getElementById: get, createElement: create, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        invoked.push([name, args]);
        return { busy: false, hasDocument: true, position: 0 };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await bootEditor(context);
  return { invoked, get, context };
}

test("Drum Map Setup lists built-in presets and lets the mapper assign any drum row", async () => {
  const { invoked, get, context } = await drumSetupEditor();
  runInContext(fixture + `view.drumView = {
    enabled: true,
    name: "Two kicks",
    presets: ["Full kit, all 12 samples", "Two kicks"],
    rows: [
      { index: 0, name: "Kick", laneKey: "Skin|soft-hitnormal", assigned: true },
      { index: 1, name: "Ghost", laneKey: null, assigned: false },
    ],
    lanes: [{ key: "Skin|soft-hitnormal", sceneKey: "n", name: "soft-hitnormal" }],
  };
  openDrumMapSetup(1);`, context);
  assert.equal(runInContext("modalKind", context), "drum-map-setup");
  const body = get("dialog-body").children;
  const presetSelect = body[0].children[1].children[0];
  presetSelect.value = "Full kit, all 12 samples";
  presetSelect.events.get("change")();
  await runInContext("pending", context);
  const ghostSelect = body[1].children[1].children[1];
  ghostSelect.value = "Skin|soft-hitnormal";
  ghostSelect.events.get("change")();
  await runInContext("pending", context);
  assert.deepEqual(invoked.filter(([name]) => name === "drum_map_action"), [
    ["drum_map_action", { map: "a.osu", action: { kind: "load_builtin", name: "Full kit, all 12 samples" } }],
    ["drum_map_action", { map: "a.osu", action: { kind: "assign_row", row_index: 1, lane_key: "Skin|soft-hitnormal" } }],
  ]);
});

test("Drum Map Setup switches Auto-detect and Hide unassigned, detects now, and moves rows by button or drag", async () => {
  const { invoked, get, context } = await drumSetupEditor();
  runInContext(fixture + `view.drumView = {
    enabled: true, name: "Two kicks", autoDetect: true, hideUnassigned: false,
    presets: ["Two kicks"],
    rows: [
      { index: 0, name: "Kick", laneKey: "Skin|soft-hitnormal", assigned: true },
      { index: 1, name: "Snare", laneKey: null, assigned: false },
      { index: 2, name: "Ghost", laneKey: null, assigned: false },
    ],
    lanes: [{ key: "Skin|soft-hitnormal", sceneKey: "n", name: "soft-hitnormal" }],
  };
  openDrumMapSetup();`, context);
  const body = get("dialog-body").children;
  const [autoDetect, hide, detectNow] = body[2].children;
  assert.equal(autoDetect.children[1].checked, true);
  assert.equal(hide.children[1].checked, false);
  const [first, , last] = body[1].children;
  assert.equal(first.children[2].disabled, true, "the first row cannot move up");
  assert.equal(last.children[3].disabled, true, "the last row cannot move down");

  const pending = () => runInContext("pending", context);
  hide.children[1].checked = true;
  hide.children[1].events.get("change")();
  await pending();
  autoDetect.children[1].checked = false;
  autoDetect.children[1].events.get("change")();
  await pending();
  detectNow.events.get("click")();
  await pending();
  first.children[3].events.get("click")();
  await pending();
  last.children[2].events.get("click")();
  await pending();
  const drag = { preventDefault() {}, dataTransfer: {} };
  last.events.get("dragstart")(drag);
  first.events.get("dragover")(drag);
  first.events.get("drop")(drag);
  await pending();
  assert.deepEqual(invoked.filter(([name]) => name === "drum_map_action").map(([, args]) => args.action), [
    { kind: "set_hide_unassigned", on: true },
    { kind: "set_auto_detect", on: false },
    { kind: "auto_detect" },
    { kind: "move_row", from_index: 0, to_index: 1 },
    { kind: "move_row", from_index: 2, to_index: 1 },
    { kind: "move_row", from_index: 2, to_index: 0 },
  ]);
});

test("the drum lane column shows a one-click Hide unassigned switch that sends the drum map action", async () => {
  const part = await readFile(new URL("../ui/src/parts/LaneHeaders.svelte", import.meta.url), "utf8");
  assert.match(part, /bridge\.callAction\("drumMapAction", \{ kind: "set_hide_unassigned", on: !drumView\.hideUnassigned \}\)/);
  assert.match(part, /aria-pressed=\{!!drumView\.hideUnassigned\}/);
});

test("the header column keeps the design's 250px width and colours every web view draws", async () => {
  // The design's 250px lane column, still clamped and still resizable.
  assert.match(css, /--lane-default-width:clamp\(170px,18vw,250px\)/);
  // The old header rules left style.css with the part.
  assert.doesNotMatch(css, /\.lane[.:\s{[]|sample-folder|--lane-sound/);
  const part = await readFile(new URL("../ui/src/parts/LaneHeaders.svelte", import.meta.url), "utf8");
  assert.doesNotMatch(part, /color-mix|oklch\(|aspect-ratio/, "colours come as hexes from ui/src/lib/colour.js");
});

test("Song M and S light like the lane M and S: M accent, S warning, ink text (v3)", () => {
  assert.match(css, /#song-mute\[aria-pressed=true\] \{ background:var\(--mute-on\); color:var\(--editor-ink\); \}/);
  assert.match(css, /#song-solo\[aria-pressed=true\] \{ background:var\(--solo-on\); color:var\(--editor-ink\); \}/);
});

test("app.js and the v3 tokens keep nothing the lane part replaced", async () => {
  assert.doesNotMatch(app, /sampleFolderDragType/);
  const tokens = await readFile(new URL("../ui/src/tokens.css", import.meta.url), "utf8");
  assert.doesNotMatch(tokens, /--lane-|--folder-|hue = 25/);
});

test("Shift-click and Ctrl-click multi-lane selection includes the active anchor and the bottom lane", async () => {
  const { evaluate } = await frontend();
  evaluate(fixture);
  // fixture has view.activeLane = 1 ("c2") and laneRows = ["soft", 0, 4, 1, 2, 5, 6, 3].
  // Shift-clicking the bottom-most lane (row 3, key "k") right after load selects from activeLane through the bottom lane.
  evaluate("laneHeaderAction('select', 3, { shiftKey: true })");
  assert.equal(evaluate("JSON.stringify([...selectionLaneKeys])"), '["c2","s","sw","t","k"]');
  const selectedRows = lanes(evaluate).filter(row => row.selected).map(row => row.row);
  assert.deepEqual(selectedRows, [1, 2, 5, 6, 3]);
  assert.equal(lanes(evaluate).at(-1).selected, true, "bottom-most lane header is marked selected");

  // Ctrl-clicking from a fresh activeLane (with empty selectionLaneKeys) keeps both activeLane and the clicked lane.
  evaluate("selectionLaneKeys.clear(); selectionLaneAnchor = null; laneHeaderAction('select', 3, { ctrlKey: true })");
  assert.equal(evaluate("JSON.stringify([...selectionLaneKeys])"), '["c2","k"]');
  assert.equal(lanes(evaluate).at(-1).selected, true, "bottom-most lane header is marked selected after Ctrl-click");
});

test("Drum Map Setup shows a merged badge without disturbing children[0..5]", async () => {
  const { get, context } = await drumSetupEditor();
  runInContext(fixture + `view.drumView = {
    enabled: true,
    name: "Auto",
    presets: ["Auto"],
    rows: [
      { index: 0, name: "Kick", laneKey: "BeatmapFile|a.wav", mergedLaneKeys: ["BeatmapFile|a.wav", "BeatmapFile|b.wav", "BeatmapFile|c.wav"], assigned: true },
    ],
    lanes: [{ key: "BeatmapFile|a.wav", sceneKey: "a", name: "a.wav" }],
  };
  openDrumMapSetup();`, context);
  const firstRow = get("dialog-body").children[1].children[0];
  assert.equal(firstRow.children[1].value, "BeatmapFile|a.wav");
  assert.equal(firstRow.children[2].disabled, true);
  assert.equal(firstRow.children[6].textContent, "+2 merged");
  assert.equal(firstRow.children[6].title, "BeatmapFile|a.wav\nBeatmapFile|b.wav\nBeatmapFile|c.wav");
});

test("Ctrl+scroll cycling on a lane strip updates colour, coalesces undo, and persists layout", async () => {
  const { calls, evaluate } = await frontend();
  evaluate(fixture + `let now = 1000; Date.now = () => now;`);

  evaluate("laneHeaderAction('cycleColour', 1, 1)");
  await evaluate("pending");
  assert.equal(evaluate('laneColours.get("Skin|soft-hitclap2")'), "#d04932");
  assert.equal(lanes(evaluate)[2].colour, "#d04932");

  evaluate("laneHeaderAction('cycleColour', 1, 1)");
  await evaluate("pending");
  assert.equal(evaluate('laneColours.get("Skin|soft-hitclap2")'), "#db642b");

  // One Undo restores null, confirming both rapid wheel steps coalesced into one entry; Redo restores #db642b.
  evaluate('act({ kind: "command", id: "undo" })');
  await evaluate("pending");
  assert.equal(evaluate('laneColours.has("Skin|soft-hitclap2")'), false);
  evaluate('act({ kind: "command", id: "redo" })');
  await evaluate("pending");
  assert.equal(evaluate('laneColours.get("Skin|soft-hitclap2")'), "#db642b");

  // Cycling forward then back to the starting colour within 600 ms pops the no-op undo entry.
  evaluate("now = 2000; laneHeaderAction('cycleColour', 1, 1)");
  await evaluate("pending");
  assert.equal(evaluate('laneColours.get("Skin|soft-hitclap2")'), "#d78628");
  evaluate("now = 2100; laneHeaderAction('cycleColour', 1, -1)");
  await evaluate("pending");
  assert.equal(evaluate('laneColours.get("Skin|soft-hitclap2")'), "#db642b");
  evaluate('act({ kind: "command", id: "undo" })');
  await evaluate("pending");
  assert.equal(evaluate('laneColours.has("Skin|soft-hitclap2")'), false);
  evaluate('act({ kind: "command", id: "redo" })');
  await evaluate("pending");
  assert.equal(evaluate('laneColours.get("Skin|soft-hitclap2")'), "#db642b");

  // Ignored while busy.
  evaluate("view.busy = true; laneHeaderAction('cycleColour', 1, 1)");
  assert.equal(evaluate('laneColours.get("Skin|soft-hitclap2")'), "#db642b");

  const layoutSaves = calls.filter(a => a?.kind === "lane_layout");
  assert.ok(layoutSaves.length >= 2);
  assert.equal(layoutSaves.at(-1).persist, true);
  assert.equal(layoutSaves.at(-1).value.lane_colours["Skin|soft-hitclap2"], "#db642b");
});
