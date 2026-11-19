import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
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
    document: { getElementById: get, createElement: element, querySelectorAll: () => [],
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
    ResizeObserver: class { observe() {} },
    HTMLElement: class {},
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await runInContext(app, context);
  calls.length = 0;
  status.hasDocument = true;
  const evaluate = (code) => runInContext(code, context);
  evaluate('view = { busy: false, hasDocument: true, duration: 10_000 }; scene = { path: "fixture.osu", viewStart: 1000, zoom: 1, rowHeight: 30 }');
  return { calls, documentEvents, evaluate, get, status, settle: () => evaluate("pending") };
}

test("Fit shows the whole difficulty from zero without changing lane height", async () => {
  const { calls, get, settle } = await frontend();
  const fit = get("fit").events.get("click");
  assert.equal(typeof fit, "function");
  fit();
  await settle();
  assert.deepEqual(calls, [{ scene: false, action: { kind: "view", start: 0, zoom: 0.1, row_height: 30 } }]);
});

test("Fit shows a 30-minute song in a 1000 px timeline, and wheel zoom out never zooms in", async () => {
  const { calls, evaluate, get, settle } = await frontend();
  const duration = 30 * 60_000;
  evaluate(`view.duration = ${duration}`);
  get("fit").events.get("click")();
  await settle();
  assert.deepEqual(calls.pop(), { scene: false, action: { kind: "view", start: 0, zoom: 1000 / duration, row_height: 30 } });
  evaluate(`scene.zoom = ${1000 / duration}; scene.viewStart = 0`);
  get("scroll").events.get("wheel")({ ctrlKey: true, deltaY: 100, deltaX: 0, clientX: 500, clientY: 0, preventDefault() {} });
  await settle();
  assert.ok(calls.pop().action.zoom <= 1000 / duration);
  const sliderMin = +html.match(/<input[^>]*min="([^"]+)"[^>]*id="zoom"/)[1];
  assert.ok(sliderMin <= Math.log10(1000 / duration), "the zoom slider reaches the Fit zoom");
});

test("the transport Return switch toggles the shared playback preference", async () => {
  const { calls, get, settle } = await frontend();
  const toggle = get("transport-return");
  const click = toggle.events.get("click");
  assert.equal(typeof click, "function");
  click();
  await settle();
  assert.deepEqual(calls.at(-1), { scene: false, action: { kind: "return_on_stop", on: true } });
  assert.equal(toggle.ariaPressed, "true");
  click();
  await settle();
  assert.deepEqual(calls.at(-1), { scene: false, action: { kind: "return_on_stop", on: false } });
  assert.equal(toggle.ariaPressed, "false");
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
  const follow = get("follow").events.get("change");
  assert.equal(typeof follow, "function");
  get("follow").checked = true;
  follow();
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
  assert.equal(get("lanes").children[0].style.height, "35px");
  assert.ok(Math.abs(get("scroll").scrollTop - (160 * 35 / 30 - 60)) < 0.001);
});

test("wheel follows the active keyboard preset and stops at the snap limits", async () => {
  const { calls, get, evaluate, status, settle } = await frontend();
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
  get("triplet-snap").checked = true;
  assert.equal((await roll({ ctrlKey: true, deltaY: -120 })).action.divisor, 6);
  assert.equal((await roll({ altKey: true, deltaY: -120 })).action.kind, "view");
  assert.equal((await roll({ deltaX: 35, deltaY: 0 })).action.kind, "view");

  setPreset("DAW");
  get("triplet-snap").checked = false;
  assert.deepEqual(await roll(), { prevented: false, action: undefined });
  assert.equal((await roll({ shiftKey: true })).action.kind, "view");
  assert.equal((await roll({ ctrlKey: true })).action.kind, "view");
  assert.equal((await roll({ altKey: true, deltaY: -120 })).action.divisor, 8);
  evaluate("scene.divisor = 16");
  assert.deepEqual(await roll({ altKey: true, deltaY: -120 }), { prevented: true, action: undefined });
});


test("ruler colours distinguish reduced beat divisions and reset at a tempo change", async () => {
  const { evaluate } = await frontend();
  evaluate('scene = { sections: [[0, 480, 4], [1700, 600, 3]], snap: true, snapMode: "grid", beats: 1, divisor: 4 }');
  const ticks = (zoom) => JSON.parse(evaluate('JSON.stringify(testTicks(' + zoom + '))'));
  evaluate('function testTicks(zoom) { const ticks = {}; let x; const ctx = { beginPath() {}, moveTo(at) { x = at - .5; }, lineTo() {}, stroke() { ticks[x / zoom] = this.strokeStyle; } }; drawTimingGrid(ctx, 0, 2500, zoom, 0, 21, true); return ticks; }');

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

test("lanes take their sound's design colour unless the mapper picked one", async () => {
  const { evaluate } = await frontend();
  const result = evaluate(`
    laneColours.set("picked", "#123456");
    const clap = { family: "soft-hitclap", colourKey: "unpicked" };
    JSON.stringify([
      ["normal-hitnormal", "soft-hitwhistle", "drum-hitfinish", "soft-hitclap", null]
        .map(family => laneColour({ family, colourKey: "unpicked" })),
      laneColour({ family: "soft-hitclap", colourKey: "picked" }),
      laneTint(clap), laneTint({ family: "soft-hitclap", colourKey: "picked" }),
    ])`);
  assert.deepEqual(JSON.parse(result), [
    ["#f1ad67", "#9c9cfc", "#a7d67d", "#e28dd1", "#62d6cf"], "#123456",
    "#e28dd114", "#1234561f",
  ]);
});

test("lane height buttons and slider send the same viewport action as wheel zoom", async () => {
  const { calls, get, settle } = await frontend();
  const larger = get("lane-height-in").events.get("click");
  assert.equal(typeof larger, "function");
  larger();
  await settle();
  assert.deepEqual(calls.pop(), { scene: false, action: { kind: "view", start: 1000, zoom: 1, row_height: 35 } });

  get("lane-height-out").events.get("click")();
  await settle();
  assert.equal(calls.pop().action.row_height, 30);

  get("lane-height").value = "42";
  get("lane-height").events.get("change")();
  await settle();
  assert.equal(calls.pop().action.row_height, 42);
});

test("ruler controls hide and restore the waveform and reuse timeline actions", async () => {
  const { calls, evaluate, get, settle } = await frontend();
  assert.match(html, /class="lane-title ruler-controls"[\s\S]*?id="ruler-waveform"[\s\S]*?id="ruler-fit"[\s\S]*?id="overview"/);
  const click = id => get(id).events.get("click")();
  evaluate("setWaveformHeight(88)");
  click("ruler-waveform");
  assert.equal(get("editor-arrange").style["--wave-height"], "0px");
  assert.equal(get("ruler-waveform")["aria-pressed"], "false");
  click("ruler-waveform");
  assert.equal(get("editor-arrange").style["--wave-height"], "88px");
  assert.equal(get("ruler-waveform")["aria-pressed"], "true");
  click("ruler-follow");
  await settle();
  assert.deepEqual(calls.pop().action, {kind:"follow", on:true});
  assert.equal(get("ruler-follow")["aria-pressed"], "true");
  click("ruler-zoom-out");
  click("ruler-zoom-in");
  await settle();
  assert.deepEqual(calls.splice(0).map(call=>call.action), [
    {kind:"command",id:"zoomOut"}, {kind:"command",id:"zoomIn"},
  ]);
  click("ruler-fit");
  await settle();
  assert.deepEqual(calls.pop().action, {kind:"view",start:0,zoom:.1,row_height:30});
});