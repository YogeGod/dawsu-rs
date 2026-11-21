import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { followGroup, nextGroup, stepInGroup } from "../ui/src/lib/snap.js";
import { bootEditor, readEditorSource } from "./editor-runtime.mjs";

const app = readEditorSource();
const html = await readFile(new URL("../dist/index.html", import.meta.url), "utf8");

async function frontend() {
  const controls = new Map();
  const documentEvents = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, style: { setProperty(name, value) { this[name] = value; } },
      addEventListener(name, listener) { this.events.set(name, listener); },
      append() {}, prepend() {}, setAttribute(name, value) { this[name] = value; },
      getBoundingClientRect: () => ({ x: 0, top: 100, width: 1000 }),
    };
  }
  function get(id) {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  }
  get("grid").clientWidth = 1000;
  get("scroll").scrollTop = 100;
  for (let row = 0; row < 20; row++) get("lanes").children.push(element());
  const status = { busy: false, hasDocument: false, duration: 10_000, playing: false, position: 0 };
  const context = createContext({
    document: { documentElement: { dataset: {} }, getElementById: get, createElement: element, querySelectorAll: () => [],
      addEventListener(name, handler) { documentEvents.set(name, handler); } },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(JSON.parse(JSON.stringify(args)));
        if (args.action?.kind === "follow") status.follow = args.action.on;
        if (args.action?.kind === "return_on_stop") status.returnOnStop = args.action.on;
        if (args.action?.kind === "ramp_shape") status.rampShape = args.action.value;
        return { ...status };
      } },
      event: { listen() {} },
    } },
    // A still clock: the playhead is drawn where the editor last put it.
    performance: { now: () => 0 },
    ResizeObserver: class { observe() {} },
    HTMLElement: class {},
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  const ctx = bootEditor(context, { autoStart: false });
  await ctx.act(null);
  calls.length = 0;
  status.hasDocument = true;
  const evaluate = (code) => runInContext(code, context);
  evaluate('view = { busy: false, hasDocument: true, duration: 10_000 }; scene = { path: "fixture.osu", viewStart: 1000, zoom: 1, rowHeight: 30 }');
  return { calls, documentEvents, evaluate, get, state: ctx.state, status, settle: () => evaluate("pending") };
}

// The snap part of the Svelte bundle (ui/src/main.js and bridge.svelte.js), on the real ui/src/lib/snap.js.
function snapBundle() {
  let group = null, shown = null;
  return {
    update(_view, scene) { group = followGroup(group, shown, scene); shown = scene; },
    wheelSnap: finer => shown && stepInGroup(nextGroup(group, shown.beats, shown.divisor), shown.beats, shown.divisor, finer),
    snapGroup: () => group?.name,
  };
}

test("Fit shows the whole difficulty from zero without changing lane height", async () => {
  const { calls, evaluate, settle } = await frontend();
  evaluate("statusBarAction('fit')");
  await settle();
  assert.deepEqual(calls, [{ scene: false, action: { kind: "view", start: 0, zoom: 0.1, row_height: 30 } }]);
});

test("Fit shows a 30-minute song in a 1000 px timeline, and wheel zoom out never zooms in", async () => {
  const { calls, evaluate, get, settle } = await frontend();
  const duration = 30 * 60_000;
  evaluate(`view.duration = ${duration}`);
  evaluate("statusBarAction('fit')");
  await settle();
  assert.deepEqual(calls.pop(), { scene: false, action: { kind: "view", start: 0, zoom: 1000 / duration, row_height: 30 } });
  evaluate(`scene.zoom = ${1000 / duration}; scene.viewStart = 0`);
  get("scroll").events.get("wheel")({ ctrlKey: true, deltaY: 100, deltaX: 0, clientX: 500, clientY: 0, preventDefault() {} });
  await settle();
  assert.ok(calls.pop().action.zoom <= 1000 / duration);
});

test("the zoom slider starts at the timeline's zoom-out limit", async () => {
  const { evaluate } = await frontend();
  const { ZOOM_MIN } = await import("../ui/src/lib/statusbar.js");
  assert.equal(ZOOM_MIN, +Math.log10(evaluate("minZoom")).toFixed(3));
  assert.ok(evaluate("minZoom") <= 1000 / (30 * 60_000), "Fit shows a 30-minute song in 1000 px");
});

test("the Playhead group's Return sends the shared playback preference as a light update", async () => {
  const { calls, evaluate, settle } = await frontend();
  // The Svelte part sends act(..., false) with the opposite of view.returnOnStop (ui/tests/playhead-group).
  await evaluate('act({ kind: "return_on_stop", on: true }, false)');
  await settle();
  assert.deepEqual(calls.at(-1), { scene: false, action: { kind: "return_on_stop", on: true } });
  assert.equal(evaluate("view.returnOnStop"), true);
});

test("volume pointer values use the same scale as the rendered bars", async () => {
  const { evaluate, get } = await frontend();
  get("controller").getBoundingClientRect = () => ({ x: 0, y: 0, height: 63 });
  const input = evaluate(`
    drag = { surface: controller, start: 1000, zoom: 1, rows: [] };
    pointer({clientX: 125, clientY: 33, altKey: true}, "down");
  `);
  assert.equal(input.ms, 1125);
  assert.equal(input.value, 50);
  assert.equal(input.alt, true);
});

test("Grab pans from raw pointer coordinates and scrolls vertically without editing a lane", async () => {
  const { calls, evaluate, get, settle } = await frontend();
  const grid = get("grid");
  grid.getBoundingClientRect = () => ({ x: 0, y: 0, width: 1000 });
  grid.setPointerCapture = () => {};
  evaluate(`view.tool = "Grab"; view.activeLane = 7;
    laneRows = ["normal", 0, 1, 2, 3, 4, 5];
    scene.moments = [[1015, 100, true, [[3, false]]]];`);
  const click = { clientX: 12, clientY: 20, pointerId: 1, button: 0,
    altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, preventDefault() {} };
  grid.events.get("pointerdown")(click);
  grid.events.get("pointerup")({ ...click, clientX: 30, clientY: -20 });
  await settle();
  assert.equal(calls[0].action.ms, 1012);
  assert.equal(calls[0].action.row, 7);
  assert.equal(calls.at(-1).action.ms, 1030);
  assert.equal(get("scroll").scrollTop, 140);
});

test("Escape cancels a captured grid Draw before its pointer release can commit", async () => {
  const { calls, documentEvents, evaluate, get, settle } = await frontend();
  const grid = get("grid");
  grid.getBoundingClientRect = () => ({ x: 0, y: 0 });
  grid.setPointerCapture = () => {};
  grid.hasPointerCapture = () => true;
  let released = null;
  grid.releasePointerCapture = id => { released = id; };
  evaluate('view.tool = "Draw"; laneRows = [0, 1, 2, 3, 4, 5];');
  const pointer = { clientX: 12, clientY: 20, pointerId: 1, button: 0,
    altKey: false, ctrlKey: false, metaKey: false, shiftKey: false, preventDefault() {} };
  grid.events.get("pointerdown")(pointer);
  documentEvents.get("keydown")({ key: "Escape", code: "Escape",
    target: { closest: () => null }, preventDefault() {} });
  grid.events.get("pointerup")(pointer);
  await settle();
  assert.equal(released, 1);
  assert.deepEqual(calls.map(({ action }) => [action.kind, action.phase]), [
    ["pointer", "down"], ["pointer", "cancel"],
  ]);
});

test("the volume controller offers the four legacy ramp shapes through its private action", async () => {
  const { calls, get, settle } = await frontend();
  const chooser = html.match(/<select id="ramp-shape"[^>]*>([\s\S]*?)<\/select>/)?.[1];
  assert.ok(chooser);
  assert.deepEqual([...chooser.matchAll(/<option value="([^"]+)">/g)].map(match => match[1]),
    ["Linear", "EaseIn", "EaseOut", "SCurve"]);
  const input = get("ramp-shape");
  input.value = "SCurve";
  input.events.get("change")();
  await settle();
  assert.deepEqual(calls, [{ scene: false, action: { kind: "ramp_shape", value: "SCurve" } }]);
  assert.equal(input.value, "SCurve");
});

test("Follow pages toward an offscreen playhead, including a backward cycle wrap", async () => {
  const { calls, get, evaluate, status, settle } = await frontend();
  await evaluate('act({ kind: "follow", on: true }, false)');
  await settle();
  assert.deepEqual(calls.shift(), { scene: false, action: { kind: "follow", on: true } });

  Object.assign(status, { playing: true, position: 2300 });
  await evaluate("act(null, false)");
  await settle();
  assert.equal(evaluate("scene.viewStart"), 2280);
  assert.deepEqual(calls.pop(), { scene: false, action: { kind: "view", start: 2280, zoom: 1, row_height: 30 } });

  status.position = 500;
  await evaluate("act(null, false)");
  await settle();
  assert.equal(evaluate("scene.viewStart"), 480);

  status.playing = false;
  status.position = 9500;
  await evaluate("act(null, false)");
  await settle();
  assert.equal(evaluate("scene.viewStart"), 480);
});

test("Centre keeps the playhead in the middle while following, and only while Follow is on", async () => {
  const { calls, evaluate, status, settle } = await frontend();
  let centre = true;
  evaluate("window").dawsuUi = { update() {}, centre: () => centre };
  Object.assign(status, { playing: true, position: 2300 });
  await evaluate("act(null, false)");
  await settle();
  assert.equal(evaluate("scene.viewStart"), 1000, "Follow is off: the view stays");
  status.follow = true;
  await evaluate("act(null, false)");
  await settle();
  // 1000 px at 1 px/ms: the playhead sits at the middle, 500 ms after the view's start.
  assert.equal(evaluate("scene.viewStart"), 1800);
  assert.deepEqual(calls.pop(), { scene: false, action: { kind: "view", start: 1800, zoom: 1, row_height: 30 } });
  status.position = 2400;
  await evaluate("act(null, false)");
  await settle();
  assert.equal(evaluate("scene.viewStart"), 1900);
  // Near the start the view cannot scroll further back.
  status.position = 200;
  await evaluate("act(null, false)");
  await settle();
  assert.equal(evaluate("scene.viewStart"), 0);
  calls.length = 0;
  await evaluate("act(null, false)");
  await settle();
  assert.ok(calls.every(call => call.action?.kind !== "view"), "no view change when the view already shows it");
  // Centre off: the old paging, which leaves an on-screen playhead alone.
  centre = false;
  status.position = 900;
  await evaluate("act(null, false)");
  await settle();
  assert.equal(evaluate("scene.viewStart"), 0);
});

test("Ctrl+Shift+wheel zooms lane heights around the pointer without panning", async () => {
  const { calls, get, evaluate, settle } = await frontend();
  const wheel = get("scroll").events.get("wheel");
  assert.equal(typeof wheel, "function");
  let prevented = false;
  wheel({
    ctrlKey: true, shiftKey: true, deltaY: -120, clientY: 160,
    preventDefault() { prevented = true; },
  });
  await settle();
  assert.equal(prevented, true);
  assert.deepEqual(calls, [{ scene: false, action: { kind: "view", start: 1000, zoom: 1, row_height: 35 } }]);
  assert.equal(evaluate("scene.rowHeight"), 35);
  assert.equal(get("lanes").style["--lane-row-height"], "35px", "every header row takes the canvas row height");
  assert.ok(Math.abs(get("scroll").scrollTop - (160 * 35 / 30 - 60)) < 0.001);
});

test("wheel follows the active keyboard preset and stops at the snap limits", async () => {
  const { calls, get, evaluate, status, settle } = await frontend();
  evaluate("window").dawsuUi = snapBundle();
  const wheel = get("scroll").events.get("wheel");
  get("grid").getBoundingClientRect = () => ({ x: 100 });
  const roll = async (options = {}) => {
    let prevented = false;
    wheel({
      ctrlKey: false, shiftKey: false, altKey: false, deltaX: 0, deltaY: 120, clientX: 300,
      ...options, preventDefault() { prevented = true; },
    });
    await settle();
    return { prevented, action: calls.pop()?.action };
  };
  const setPreset = (preset) => {
    status.keyPreset = preset;
    evaluate(`view.keyPreset = "${preset}"`);
  };
  evaluate('Object.assign(scene, { snap: false, snapMode: "relative", beats: 1, divisor: 4 })');
  setPreset("osu!");
  assert.deepEqual(await roll(), { prevented: true, action: { kind: "command", id: "seekForward" } });
  assert.deepEqual(await roll({ deltaY: -120, shiftKey: true }), {
    prevented: true, action: { kind: "command", id: "seekBackFar" },
  });
  assert.deepEqual(await roll({ clientX: 50 }), { prevented: false, action: undefined });
  assert.deepEqual(await roll({ ctrlKey: true, deltaY: -120 }), {
    prevented: true, action: { kind: "snap", on: false, mode: "relative", beats: 1, divisor: 8 },
  });
  evaluate('popoverAction("tripletSnap", true)');
  await settle();
  assert.equal(calls.pop().action.divisor, 6, "ticking Triplets picks 1/6, as lazer's Triplets button does");
  assert.equal((await roll({ altKey: true, deltaY: -120 })).action.kind, "view");
  assert.equal((await roll({ deltaX: 35, deltaY: 0 })).action.kind, "view");

  setPreset("DAW");
  assert.deepEqual(await roll(), { prevented: false, action: undefined });
  assert.equal((await roll({ shiftKey: true })).action.kind, "view");
  assert.equal((await roll({ ctrlKey: true })).action.kind, "view");
  assert.equal((await roll({ altKey: true, deltaY: -120 })).action.divisor, 8);
  evaluate("scene.divisor = 16");
  assert.deepEqual(await roll({ altKey: true, deltaY: -120 }), { prevented: true, action: undefined });
});

test("after 1/3 is picked, Ctrl+wheel steps only through 1/1, 1/3, 1/6 and 1/12", async () => {
  const { calls, get, evaluate, status, settle } = await frontend();
  evaluate("window").dawsuUi = snapBundle();
  const wheel = get("scroll").events.get("wheel");
  get("grid").getBoundingClientRect = () => ({ x: 100 });
  status.keyPreset = "osu!";
  evaluate('view.keyPreset = "osu!"');
  // The editor answers each snap action; show() hands the new scene to the bundle.
  const pick = divisor => evaluate(`scene.beats = 1; scene.divisor = ${divisor}; show({ ...view })`);
  const roll = async deltaY => {
    calls.length = 0;
    wheel({ ctrlKey: true, shiftKey: false, altKey: false, deltaX: 0, deltaY, clientX: 300, preventDefault() {} });
    await settle();
    return calls.find(call => call.action.kind === "snap")?.action.divisor;
  };
  evaluate('Object.assign(scene, { snap: true, snapMode: "grid", sections: [], lanes: [] })');
  pick(4);
  assert.equal(evaluate("window.dawsuUi.snapGroup()"), "common");
  assert.equal(await roll(-120), 8, "Common: 1/4 to 1/8");
  pick(3);
  assert.equal(evaluate("window.dawsuUi.snapGroup()"), "triplets", "the Triplets box shows the group");
  assert.equal(await roll(-120), 6);
  pick(6);
  assert.equal(await roll(-120), 12);
  pick(12);
  assert.equal(await roll(-120), undefined, "stops at 1/12");
  assert.equal(await roll(120), 6);
  pick(1);
  assert.equal(await roll(-120), 3, "1/1 keeps Triplets");
  assert.equal(await roll(120), undefined, "stops at 1/1");
  pick(4);
  assert.equal(await roll(120), 2, "picking 1/4 goes back to Common");
});


test("ruler colours distinguish reduced beat divisions and reset at a tempo change", async () => {
  const { evaluate } = await frontend();
  evaluate('scene = { sections: [[0, 480, 4], [1700, 600, 3]], snap: true, snapMode: "grid", beats: 1, divisor: 4 }');
  const ticks = (zoom) => JSON.parse(evaluate('JSON.stringify(testTicks(' + zoom + '))'));
  evaluate('function testTicks(zoom) { const ticks = {}; let x; const ctx = { beginPath() {}, moveTo(at) { x = at - .5; }, lineTo() {}, stroke() { ticks[x / zoom] = this.strokeStyle; }, fillText() {} }; drawTimingGrid(ctx, 0, 2500, zoom, 0, 21, true); return ticks; }');

  const quarters = ticks(1);
  assert.equal(quarters[0], "#ffffff");
  assert.equal(quarters[120], "#66ccff");
  assert.equal(quarters[240], "#ed1121");
  assert.equal(quarters[1700], "#ffffff");
  assert.equal(quarters[1850], "#66ccff");
  assert.equal(quarters[2000], "#ed1121");

  evaluate("scene.divisor = 3");
  const thirds = ticks(1);
  assert.equal(thirds[120], undefined);
  assert.equal(thirds[160], "#8866ee");
  assert.equal(thirds[1900], "#8866ee");
  assert.equal(Object.values(ticks(0.01)).includes("#8866ee"), false);

  evaluate('scene.snapMode = "objects"');
  assert.equal(Object.values(ticks(1)).includes("#8866ee"), false);
  evaluate('scene.snapMode = "grid"; scene.snap = false');
  assert.equal(Object.values(ticks(1)).includes("#8866ee"), false);
});

test("the 40 px ruler draws bars full height, beats 12 px and snap ticks at the divisor's height", async () => {
  const { evaluate } = await frontend();
  evaluate('scene = { sections: [[0, 480, 4]], snap: true, snapMode: "grid", beats: 1, divisor: 4 }');
  // Each tick as [top, colour] at its time; every tick ends at the ruler's bottom (40 px).
  evaluate(`function rulerTicks(zoom) {
    const ticks = {}; let x, top;
    const ctx = { beginPath() {}, moveTo(at, y) { x = at - .5; top = y; }, lineTo(_, y) { this.bottom = y; },
      stroke() { ticks[x / zoom] = [top, this.bottom, this.strokeStyle]; }, fillText() {} };
    drawTimingGrid(ctx, 0, 1000, zoom, 0, 40, true);
    return ticks;
  }`);
  const ticks = () => JSON.parse(evaluate("JSON.stringify(rulerTicks(1))"));
  const quarters = ticks();
  assert.deepEqual(quarters[0], [0, 40, "#ffffff"], "bar line, full height");
  // A long snap (2 beats) is a beat tick on the ruler, not a full-height bar line.
  evaluate("scene.beats = 2; scene.divisor = 1");
  assert.deepEqual(ticks()[960], [28, 40, "#ffffff"], "long snap, 12 px");
  evaluate("scene.beats = 1; scene.divisor = 4");
  assert.deepEqual(quarters[480], [28, 40, "#ffffff"], "beat tick, 12 px");
  assert.deepEqual(quarters[240], [31, 40, "#ed1121"], "1/2, 9 px");
  assert.deepEqual(quarters[120], [33, 40, "#66ccff"], "1/4, 7 px");
  evaluate("scene.divisor = 3");
  assert.deepEqual(ticks()[160], [33, 40, "#8866ee"], "1/3, 7 px");
  evaluate("scene.divisor = 8");
  assert.deepEqual(ticks()[60], [35, 40, "#ffcc22"], "1/8, 5 px");
  evaluate("scene.divisor = 6");
  assert.deepEqual(ticks()[80], [35, 40, "#eeaa00"], "1/6, 5 px");
  evaluate("scene.divisor = 16");
  assert.deepEqual(ticks()[30], [36, 40, "#6644cc"], "1/16, 4 px");
  evaluate("scene.divisor = 12");
  assert.deepEqual(ticks()[40], [36, 40, "#cc6600"], "1/12, 4 px");
});

test("editor grid lines stay neutral: sub-beats follow the snap, then beats and bars", async () => {
  const { evaluate } = await frontend();
  evaluate('scene = { sections: [[0, 480, 4]], snap: true, snapMode: "grid", beats: 1, divisor: 4 }');
  evaluate(`function testGridLines() {
    const lines = {}; let x, top;
    const ctx = { beginPath() {}, moveTo(at, y) { x = at - .5; top = y; }, lineTo(_, y) { this.bottom = y; },
      stroke() { lines[x] = [top, this.bottom, this.strokeStyle]; }, fillText() {} };
    drawTimingGrid(ctx, 0, 2000, 1, 0, 300, false);
    return lines;
  }`);
  const lines = () => JSON.parse(evaluate("JSON.stringify(testGridLines())"));
  const quarters = lines();
  // v3 §5 (Yoge): only the ruler is coloured; grid lines run the full height in white at low alpha.
  assert.deepEqual(quarters[0], [0, 300, "rgba(255,255,255,.22)"], "bar");
  assert.deepEqual(quarters[480], [0, 300, "rgba(255,255,255,.045)"], "beat");
  assert.deepEqual(quarters[120], [0, 300, "rgba(255,255,255,.025)"], "1/4");
  assert.deepEqual(quarters[240], [0, 300, "rgba(255,255,255,.025)"], "1/2");
  evaluate("scene.divisor = 3");
  assert.deepEqual(lines()[160], [0, 300, "rgba(255,255,255,.025)"], "sub-beats follow the divisor");
  assert.equal(lines()[120], undefined);
  evaluate("scene.snap = false");
  assert.equal(lines()[160], undefined, "no sub-beats with snap off");
  evaluate("scene.editorBars = [{ start: 0, beat: 480, numerator: 3, bar: 1 }]");
  assert.deepEqual(lines()[1440], [0, 300, "rgba(255,255,255,.22)"], "a time signature's bar");
});

test("bar numbers sit 10 px from the ruler's top; bar lines run its full height", async () => {
  const { evaluate } = await frontend();
  const result = evaluate(`
    scene = { sections: [[0, 500, 4]], snap: false, editorBars: [{ start: 0, beat: 500, numerator: 4, bar: 1 }] };
    const labels = [], lines = [];
    let from;
    const ctx = { beginPath() {}, moveTo(x, y) { from = [x - .5, y]; }, lineTo(_, y) { lines.push([...from, y, this.strokeStyle]); },
      stroke() {}, fillText(text, x, y) { labels.push([text, x - 5, y, this.fillStyle]); } };
    drawTimingGrid(ctx, 0, 4500, .1, 0, 40, true);
    // The beat tick at 200 px draws first; the editor bar line draws over it.
    JSON.stringify([labels[1], lines.filter(line => line[0] === 200).at(-1)]);
  `);
  assert.deepEqual(JSON.parse(result), [["2", 200, 10, "#f5f5f7"], [200, 0, 40, "#f5f5f7"]]);
});

test("without time signatures the ruler numbers bars as the editor does", async () => {
  const { evaluate } = await frontend();
  // 4/4 at 500 ms from 0, then a red line at 3000 ms in 3/4 at 400 ms. The second section starts
  // at bar 3: a bar and a half before it count as two (tempo.rs bar_beat_at rounds up).
  const labels = at => JSON.parse(evaluate(`
    scene = { sections: [[0, 500, 4], [3000, 400, 3]], snap: false };
    var barLabels = [];
    drawTimingGrid({ beginPath() {}, moveTo() {}, lineTo() {}, stroke() {},
      fillText(text, x, y) { barLabels.push([text, Math.round(x - 5), y]); } }, 0, 6000, ${at}, 0, 40, true);
    JSON.stringify(barLabels);
  `));
  assert.deepEqual(labels(.1), [["1", 0, 10], ["2", 200, 10], ["3", 300, 10], ["4", 420, 10], ["5", 540, 10]]);
  // Zoomed out, bars narrower than 60 px label only every few bars so the numbers never run together,
  // counted from each section's first bar so a later section keeps its first number.
  assert.deepEqual(labels(.02).map(([text]) => text), ["1", "3"]);
  // A label that would sit on the next section's label is left out.
  const close = JSON.parse(evaluate(`
    scene = { sections: [[0, 500, 4], [2050, 400, 3]], snap: false };
    var closeLabels = [];
    drawTimingGrid({ beginPath() {}, moveTo() {}, lineTo() {}, stroke() {},
      fillText(text, x) { closeLabels.push([text, Math.round(x - 5)]); } }, 0, 4000, .1, 0, 40, true);
    JSON.stringify(closeLabels);
  `));
  assert.deepEqual(close, [["1", 0], ["3", 205], ["4", 325]]);
});

test("time signature bars draw at their new positions while snap colours stay on the map grid", async () => {
  const { evaluate } = await frontend();
  const result = evaluate(`
    scene = {
      sections: [[0, 500, 4]], snap: true, snapMode: "grid", beats: 1, divisor: 2,
      editorBars: [{ start: 0, beat: 500, numerator: 4, bar: 1 }, { start: 2000, beat: 250, numerator: 7, bar: 2 }],
    };
    const labels = [];
    const colours = {};
    let x;
    const ctx = {
      beginPath() {}, moveTo(at) { x = at - .5; }, lineTo() {}, stroke() { colours[x] = this.strokeStyle; },
      fillText(text, at) { labels.push([text, at - 5]); },
    };
    drawTimingGrid(ctx, 1800, 4500, 1, 0, 21, true);
    JSON.stringify([labels, colours[450]]);
  `);
  assert.deepEqual(JSON.parse(result), [[["2", 200], ["3", 1950]], "#ed1121"]);
});

test("hit and controller levels use the legacy blue-purple-red palette", async () => {
  const { evaluate } = await frontend();
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify([-4, 0, 25, 50, 75, 100, 120].map(volumeColour))")), [
    "rgb(58, 108, 240)", "rgb(58, 108, 240)", "rgb(106, 90, 228)",
    "rgb(154, 72, 216)", "rgb(195, 65, 137)", "rgb(236, 58, 58)", "rgb(236, 58, 58)",
  ]);
});

// The timeline row tint, like the Inspector's Lane tint (part 7), is the picked colour or none.
test("lane rows tint only in a colour the mapper picked", async () => {
  const { evaluate } = await frontend();
  const result = evaluate(`
    laneColours.set("picked", "#123456");
    const clap = { family: "soft-hitclap", colourKey: "unpicked" };
    JSON.stringify([
      laneTint(clap), laneTint({ family: "soft-hitclap", colourKey: "picked" }),
      laneTint({ family: "soft-hitclap", colourKey: "picked" }, true),
    ])`);
  assert.deepEqual(JSON.parse(result), [
    // v3: 8 % on a row, 16 % on the active lane's row.
    null, "#12345614", "#12345629",
  ]);
});

test("lane height buttons and slider send the same viewport action as wheel zoom", async () => {
  const { calls, evaluate, settle } = await frontend();
  evaluate('popoverAction("laneHeightStep", 1.15)');
  await settle();
  assert.deepEqual(calls.pop(), { scene: false, action: { kind: "view", start: 1000, zoom: 1, row_height: 35 } });

  evaluate('popoverAction("laneHeightStep", 1 / 1.15)');
  await settle();
  assert.equal(calls.pop().action.row_height, 30);

  evaluate('popoverAction("laneHeight", 42, false)');
  await settle();
  assert.equal(calls.pop().action.row_height, 42);
});

test("the lane toolbar holds zoom, Fit all and lane height; the Song row holds the waveform switch", async () => {
  const { calls, evaluate, get, settle } = await frontend();
  // v3 §4: the lane toolbar above the lanes, beside the ruler. The overview cell keeps no buttons.
  const toolbar = html.slice(html.indexOf('class="lane-title lane-title-controls'), html.indexOf('class="lane-title song-row'));
  const order = ["ruler-zoom-out", "ruler-zoom-in", "ruler-fit", "lane-shorter", "lane-taller"].map(id => toolbar.indexOf(`id="${id}"`));
  assert.ok(order.every((at, i) => at >= 0 && (i === 0 || at > order[i - 1])), "toolbar order " + order);
  const overviewCell = html.slice(html.indexOf('class="overview"'), html.indexOf('id="overview"'));
  assert.doesNotMatch(overviewCell, /<button/);
  // v3 §4 and §8: the waveform switch sits in the Song row, not the lane toolbar.
  const songRow = html.slice(html.indexOf('class="lane-title song-row'), html.indexOf('id="ruler"'));
  assert.match(songRow, /id="song-mute"[\s\S]*id="song-solo"[\s\S]*id="ruler-waveform"/);
  // v3 §4: the Song · objects header, a teal strip, its name, then M, S and the waveform switch.
  assert.match(songRow, /class="song-strip"[\s\S]*Song · objects[\s\S]*id="song-mute"/);
  // Follow lives in the Playhead group and the Playback menu (v3 §8 drops the toolbar's locate button).
  assert.doesNotMatch(html + app, /ruler-follow/);
  // v3 §4 (Yoge): no Delete lane or lane names button; the lane menu, Edit and View rows stay,
  // and none of them presses a removed button.
  const commands = await readFile(new URL("../ui/src/lib/commands.js", import.meta.url), "utf8");
  const menus = await readFile(new URL("../dist/menus.js", import.meta.url), "utf8");
  assert.doesNotMatch(html + app + commands + menus, /id="delete-lane"|\$\("delete-lane"\)|"click", "delete-lane"|toggle-lane-names/);
  assert.match(commands, /id: "deleteActiveLane"[^}]*run: \["call", "deleteActiveLane"\]/);
  assert.match(commands, /id: "laneDelete"[^}]*run: \["onLane", "call", "deleteActiveLane"\]/);
  assert.match(commands, /id: "laneNames"[^}]*run: \["panel", "lane_names", "laneNames"\]/);

  const click = id => get(id).events.get("click")();
  evaluate("setWaveformHeight(88)");
  // v3 §4 (Yoge): the switch hides only the waveform; the Song row keeps its height.
  click("ruler-waveform");
  assert.equal(get("editor-arrange").style["--wave-height"], "88px");
  assert.equal(get("ruler-waveform")["aria-pressed"], "false");
  assert.equal(evaluate("waveformShown"), false);
  click("ruler-waveform");
  assert.equal(get("editor-arrange").style["--wave-height"], "88px");
  assert.equal(get("ruler-waveform")["aria-pressed"], "true");
  // Dragged to nothing, the Song row shows no waveform, so the switch shows off.
  evaluate("setWaveformHeight(0)");
  assert.equal(get("ruler-waveform")["aria-pressed"], "false");
  evaluate("setWaveformHeight(88)");
  assert.equal(get("ruler-waveform")["aria-pressed"], "true");
  // v3 §4: a greyed move-vertical glyph heads the lane height buttons; Fit all and the waveform
  // switch wait while a job runs (data-doc).
  assert.match(toolbar, /class="lane-tool-glyph" aria-hidden="true"[\s\S]*id="lane-shorter"/);
  assert.match(html, /id="ruler-fit"[^>]*data-doc/);
  assert.match(html, /id="ruler-waveform"[^>]*data-doc/);
  click("ruler-zoom-out");
  click("ruler-zoom-in");
  click("lane-shorter");
  click("lane-taller");
  await settle();
  // Shorter and Taller run the same commands as View › Shorter lanes and Taller lanes.
  assert.deepEqual(calls.splice(0).map(call=>call.action), [
    {kind:"command",id:"zoomOut"}, {kind:"command",id:"zoomIn"},
    {kind:"command",id:"rowsOut"}, {kind:"command",id:"rowsIn"},
  ]);
  click("ruler-fit");
  await settle();
  assert.deepEqual(calls.pop().action, {kind:"view",start:0,zoom:.1,row_height:30});
});

test("the Song row opens 84 px tall (v3) and drops its name when the row is too short for two lines", async () => {
  const { evaluate, get } = await frontend();
  assert.equal(evaluate("defaultWaveformHeight"), 84);
  evaluate("setWaveformHeight(84)");
  assert.equal(get("editor-arrange").dataset.songRow, "tall");
  evaluate("setWaveformHeight(51)");
  assert.equal(get("editor-arrange").dataset.songRow, "short");
  evaluate("setWaveformHeight(0)");
  assert.equal(get("editor-arrange").dataset.songRow, "short");
});

test("a lane toolbar button at its limit shows disabled", async () => {
  const { evaluate, get } = await frontend();
  const limits = () => ["ruler-zoom-out", "ruler-zoom-in", "lane-shorter", "lane-taller"].map(id => !!get(id).disabled);
  evaluate("refreshLaneToolbar()");
  assert.deepEqual(limits(), [false, false, false, false]);
  // The editor's limits: zoom .0005 to 3 px per ms, lanes 20 to 65 px.
  evaluate("scene.zoom = 3; scene.rowHeight = 65; refreshLaneToolbar()");
  assert.deepEqual(limits(), [false, true, false, true]);
  evaluate("scene.zoom = .0005; scene.rowHeight = 20; refreshLaneToolbar()");
  assert.deepEqual(limits(), [true, false, true, false]);
  evaluate("view.busy = true; scene.zoom = 1; scene.rowHeight = 30; refreshLaneToolbar()");
  assert.deepEqual(limits(), [true, true, true, true]);
});
test("the song overview sits at the bottom of the timeline, below the volume lane, with the ruler on top", async () => {
  const docking = await readFile(new URL("../dist/docking.js", import.meta.url), "utf8");
  const style = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");
  const arrange = html.slice(html.indexOf('id="editor-arrange"'), html.indexOf('id="editor-inspector"'));
  const order = ['class="ruler"', 'id="scroll"', 'id="volume-controller-panel"', 'class="overview"'].map(mark => arrange.indexOf(mark));
  assert.ok(order.every((at, i) => at > 0 && (i === 0 || at > order[i - 1])), `timeline rows in order: ${order}`);
  assert.match(docking, /const defaultTimeline = \[ruler, grid, controller, overview\];/);
  // The rows before the layout code runs follow the same order: ruler, lanes, volume lane, overview.
  assert.match(style, /\.arrange\{grid-template-rows:var\(--layout-timeline-rows,var\(--ruler-row\) minmax\(0,1fr\) auto minmax\(20px,max-content\)\)\}/);
  assert.match(style, /\.arrange\[data-controller="false"\]\{[^}]*grid-template-rows:var\(--layout-timeline-rows,var\(--ruler-row\) minmax\(0,1fr\) minmax\(20px,max-content\)\)\}/);
});

test("the overview canvas never sizes its own row, so its drawn height can't hold the strip open", async () => {
  // The row is max-content: a canvas taller than the head once kept the strip at that height.
  const timelineCss = await readFile(new URL("../ui/src/timeline.css", import.meta.url), "utf8");
  assert.match(timelineCss, /\.arrange > \.overview > #overview \{\s*height: 0;\s*min-height: 100%;/);
});

test("timeline hitboxes stay 20 px wide across zoom levels for clicks, Erase sweeps and volume bars", async () => {
  const { calls, evaluate, get, settle, status } = await frontend();
  const grid = get("grid");
  const controller = get("controller");
  grid.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, width: 1000, height: 300 });
  controller.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, width: 1000, height: 80 });
  grid.setPointerCapture = () => {};
  controller.setPointerCapture = () => {};
  get("scroll").scrollTop = 0;
  status.tool = "Select";
  evaluate(`
    laneRows = [0, 1, 2];
    scene.viewStart = 0;
    scene.zoom = 0.05;
    scene.rowHeight = 30;
    scene.moments = [
      [1000, 80, true, [[0, 80, false, false]]],
      [1300, 60, true, [[0, 60, false, false]]],
      [3000, 90, true, [[0, 90, false, false]]],
    ];
    view.tool = "Select";
  `);
  // At zoom 0.05, hit 1000 ms is at x=50 px and hit 1300 ms is at x=65 px.
  // Clicking 8 px off-center (x=58 px -> 1160 ms, 160 ms away from 1000) is closer to 1300 (7 px) than 1000 (8 px).
  const fire = (el, type, init) => el.events.get(type)({
    type, button: 0, pointerId: 1, clientX: 0, clientY: 15,
    shiftKey: false, ctrlKey: false, metaKey: false, altKey: false,
    preventDefault() {}, ...init,
  });
  calls.length = 0;
  fire(grid, "pointerdown", { clientX: 42 }); // 8 px left of 1000 ms (x=50)
  fire(grid, "pointerup", { clientX: 42 });
  await settle();
  assert.equal(calls.find(c => c.action?.kind === "pointer" && c.action.phase === "down").action.ms, 1000);

  // Between two close hits (x=50 and x=65), the closer one (x=65 -> 1300 ms) wins.
  calls.length = 0;
  fire(grid, "pointerdown", { clientX: 58 });
  fire(grid, "pointerup", { clientX: 58 });
  await settle();
  assert.equal(calls.find(c => c.action?.kind === "pointer" && c.action.phase === "down").action.ms, 1300);

  // Outside ±10 px (x=38 -> 12 px left of x=50) stays at raw time (760 ms).
  calls.length = 0;
  fire(grid, "pointerdown", { clientX: 38 });
  fire(grid, "pointerup", { clientX: 38 });
  await settle();
  assert.equal(calls.find(c => c.action?.kind === "pointer" && c.action.phase === "down").action.ms, 760);

  // Erase drag sweep snaps to hits within ±10 px on pointermove even when zoomed out.
  status.tool = "Erase";
  evaluate('view.tool = "Erase";');
  calls.length = 0;
  fire(grid, "pointerdown", { clientX: 10 });
  evaluate("drag.last = -100;");
  fire(grid, "pointermove", { clientX: 142 }); // 8 px left of 3000 ms (x=150)
  fire(grid, "pointerup", { clientX: 142 });
  await settle();
  const eraseMove = calls.find(c => c.action?.kind === "pointer" && c.action.phase === "move");
  assert.equal(eraseMove.action.ms, 3000);

  // Volume controller lane bar drag snaps within ±10 px; Alt-paint keeps raw pointer time.
  calls.length = 0;
  fire(controller, "pointerdown", { clientX: 158, clientY: 20 }); // 8 px right of 3000 ms (x=150)
  fire(controller, "pointerup", { clientX: 158, clientY: 20 });
  await settle();
  assert.equal(calls.find(c => c.action?.kind === "pointer" && c.action.phase === "down").action.ms, 3000);

  calls.length = 0;
  fire(controller, "pointerdown", { clientX: 158, clientY: 20, altKey: true });
  fire(controller, "pointerup", { clientX: 158, clientY: 20, altKey: true });
  await settle();
  assert.equal(calls.find(c => c.action?.kind === "pointer" && c.action.phase === "down").action.ms, 3160);
});

test("at 150% UI scale, grid clicks/hitboxes and Volume-lane hover resolve to exact unzoomed timestamps", async () => {
  const { calls, evaluate, get, state, settle, status } = await frontend();
  const grid = get("grid");
  const controller = get("controller");
  grid.getBoundingClientRect = () => ({ x: 150, y: 90, left: 150, top: 90, width: 1500, height: 450 });
  controller.getBoundingClientRect = () => ({ x: 150, y: 540, left: 150, top: 540, width: 1500, height: 120 });
  grid.setPointerCapture = () => {};
  controller.setPointerCapture = () => {};
  get("scroll").scrollTop = 0;
  status.tool = "Select";
  evaluate(`
    document.documentElement.dataset.uiScale = "150";
    laneRows = [0, 1, 2];
    scene.viewStart = 0;
    scene.zoom = 0.5;
    scene.rowHeight = 30;
    scene.lanes = [
      { family: "normal-hitnormal", group: "normal", colourKey: "k0" },
      { family: "normal-hitclap", group: "normal", colourKey: "k1" },
      { family: "normal-hitwhistle", group: "normal", colourKey: "k2" },
    ];
    // At zoom 0.5, 1000 ms is logical x=500 and 1500 ms is logical x=750 (the 1.5x drift decoy).
    scene.moments = [
      [1000, 80, true, [[1, 80, false, false]]],
      [1500, 60, true, [[1, 60, false, false]]],
    ];
    view.tool = "Select";
  `);
  const fire = (el, type, init) => el.events.get(type)({
    type, button: 0, pointerId: 1, clientX: 0, clientY: 157.5,
    shiftKey: false, ctrlKey: false, metaKey: false, altKey: false,
    preventDefault() {}, ...init,
  });

  // Logical x=200 (400 ms), logical y=45 (row 1) -> visual (450, 157.5) at 150% scale.
  calls.length = 0;
  fire(grid, "pointerdown", { clientX: 450, clientY: 157.5 });
  fire(grid, "pointerup", { clientX: 450, clientY: 157.5 });
  await settle();
  const rawClick = calls.find(c => c.action?.kind === "pointer" && c.action.phase === "down").action;
  assert.equal(rawClick.ms, 400);
  assert.equal(rawClick.row, 1);

  // Logical x=494 (6 logical px left of 1000 ms at x=500) -> visual x=891; must snap to 1000 ms, not 1500 ms.
  calls.length = 0;
  fire(grid, "pointerdown", { clientX: 891, clientY: 157.5 });
  fire(grid, "pointerup", { clientX: 891, clientY: 157.5 });
  await settle();
  const hitClick = calls.find(c => c.action?.kind === "pointer" && c.action.phase === "down").action;
  assert.equal(hitClick.ms, 1000);
  assert.equal(hitClick.row, 1);

  // Grid hover card at the same zoomed coordinates resolves to row 1, 1000 ms.
  const cardKey = evaluate("hitCardAt({ clientX: 891, clientY: 157.5 })?.key ?? null");
  assert.ok(cardKey?.startsWith("1:1000:"), `expected hit card for row 1 at 1000 ms, got ${cardKey}`);

  // Volume-lane hover at visual x=900 (logical x=500 -> 1000 ms) must set volumeHoverTime to 1000, not 1500.
  fire(controller, "pointermove", { clientX: 900, clientY: 570 });
  assert.equal(state.volumeHoverTime, 1000);

  // Moving to visual x=1275 (logical x=750 -> 1500 ms) hovers the 1500 ms bar.
  fire(controller, "pointermove", { clientX: 1275, clientY: 570 });
  assert.equal(state.volumeHoverTime, 1500);

  // Moving to empty space at visual x=450 (logical x=200 -> 400 ms) clears volumeHoverTime.
  fire(controller, "pointermove", { clientX: 450, clientY: 570 });
  assert.equal(state.volumeHoverTime, null);
});
