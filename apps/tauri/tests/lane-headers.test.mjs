import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const read = name => readFile(new URL("../dist/" + name, import.meta.url), "utf8");
const [app, css] = await Promise.all(["app.js", "style.css"].map(read));

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
  await runInContext(app, context);
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
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",0,1,2,4,5,6,3]');
  const rows = headers(evaluate).rows;
  assert.equal(rows[0].type, "folder");
  assert.deepEqual(rows.slice(1).map(row => [row.name, row.tag, row.icon, row.number]), [
    ["Normal", "Soft", "normal", 1], ["Clap 2", "Soft", "clap", 2], ["Slide", "Soft", "slide", 3],
    ["Normal", "Soft · skin", "normal", 5], ["Slide whistle", "Soft", "whistle", 6], ["Tick", "Soft", "tick", 7],
    ["Kick", "File", "file", 4],
  ]);
  const [normal, clap, , , , , kick] = rows.slice(1);
  assert.deepEqual(normal, {
    type: "lane", key: "lane:0", row: 0, number: 1, name: "Normal", label: "soft-hitnormal", display: "soft-hitnormal",
    title: "soft-hitnormal · Select and audition · Ctrl/Cmd toggles · Shift selects a range",
    tag: "Soft", icon: "normal", colour: null, folder: "soft", active: false, selected: false,
    muted: false, solo: false, quiet: false, layer: false,
    family: { collapsed: false, label: "Collapse soft-hitnormal layers" },
    addLayer: "Add numbered soft-hitnormal layer",
  });
  assert.equal(clap.active, true, "view.activeLane");
  assert.equal(clap.layer, true, "a numbered layer above 1 is indented");
  assert.equal(kick.folder, null, "a map file lane sits outside the folders");
  assert.equal(get("lanes").style["--lane-row-height"], "30px");
  assert.equal(get("lanes").children.length, 0, "app.js builds no header nodes any more");

  evaluate('renamedLanes.set("Skin|soft-hitclap2", "Snare"); renderLanes();');
  assert.equal(lanes(evaluate)[1].name, "Snare");
  assert.equal(lanes(evaluate)[1].tag, "Soft");

  evaluate("showLaneInfo = true; renderLanes();");
  assert.equal(lanes(evaluate)[0].name, "soft-hitnormal · idx 1");
  assert.equal(lanes(evaluate)[0].display, "soft-hitnormal", "M and S keep the plain name");

  evaluate("showLaneInfo = false; view.panels = { laneNames: false }; renderLanes();");
  assert.equal(headers(evaluate).names, false);
});

test("a lane is grey until the mapper picks a colour; mute and solo grey the others out", async () => {
  const { evaluate } = await frontend();
  evaluate(fixture);
  assert.ok(lanes(evaluate).every(row => row.colour === null), "no sound-colour fallback");
  evaluate('laneColours.set("Skin|soft-hitclap2", "#d04932"); scene.lanes[0].muted = true; renderLanes();');
  assert.equal(lanes(evaluate)[1].colour, "#d04932");
  assert.deepEqual(lanes(evaluate).map(row => row.quiet), [true, false, false, false, false, false, false]);
  evaluate("scene.lanes[2].solo = true; renderLanes();");
  assert.deepEqual(lanes(evaluate).map(row => row.quiet), [true, true, false, true, true, true, true]);
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
  assert.equal(lanes(evaluate)[1].selected, true);
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
  const { evaluate, get } = await frontend();
  evaluate(fixture);
  assert.equal(evaluate("inspectorModel().lane.title"), "Clap 2 · Soft set");
  evaluate(`scene.moments = [[1000, 70, true, [[0, true]]]];
    scene.selection = { count: 1, start: 1000, object: "Circle" };`);
  assert.equal(evaluate("inspectorModel().selection.summary"), "1 hit selected · Normal (Soft)");
  const filter = get("lane-filter");
  filter.value = "soft clap";
  filter.events.get("input")();
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
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",0,1,2,4,5,6,3]');
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
