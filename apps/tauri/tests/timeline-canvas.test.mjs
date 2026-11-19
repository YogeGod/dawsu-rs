import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

// Loads the editor with fake canvases that record every 2D call and the paint state it used.
// `roundRect: false` fakes an older WebView whose canvas has no roundRect().
async function canvases({ roundRect = true } = {}) {
  const elements = new Map();
  function element() {
    const calls = [];
    const ctx = new Proxy(roundRect ? {} : { roundRect: undefined }, {
      get(target, name) {
        if (name in target) return target[name];
        return (...args) => calls.push({ name, args,
          fill: target.fillStyle, stroke: target.strokeStyle, lineWidth: target.lineWidth });
      },
    });
    return {
      calls, getContext: () => ctx, clientWidth: 300, clientHeight: 80, scrollTop: 0,
      dataset: {}, style: {}, children: [], addEventListener() {}, append() {}, prepend() {}, setAttribute() {},
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { devicePixelRatio: 1, __TAURI__: {
      core: { invoke: async () => ({ hasDocument: false, busy: false }) },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} }, requestAnimationFrame() {}, setInterval() {},
    console,
  });
  await runInContext(app.replace(/\nact\(null\);\s*$/, ""), context);
  get("volume-controller-panel").hidden = true;
  runInContext(`
    view = { hasDocument: true, duration: 4000, position: 0, left: 0, right: 0 };
    scene = { path: "fixture.osu", viewStart: 0, zoom: .1, rowHeight: 30, moments: [], waveform: [],
      sections: [[0, 500, 4], [2000, 250, 4]],
      gameplay: {
        points: [["circle", 100, 0], ["slider", 400, 1], ["slider", 600, 1], ["slider", 800, 1]],
        spans: [["slider", 400, 800, 1], ["spinner", 1000, 1200, 2]],
      } };`, context);
  const paint = code => {
    for (const canvas of elements.values()) canvas.calls.length = 0;
    runInContext(code + "; paint()", context);
  };
  return { get, paint, evaluate: code => runInContext(code, context) };
}

const round = value => Math.round(value * 100) / 100;
const circles = canvas => canvas.calls.filter(call => call.name === "arc")
  .map(({ args: [x, y, radius], fill, stroke }) => ({ x: round(x), y, diameter: round(radius * 2), fill, stroke }));

test("object circles scale with zoom: half a beat on screen × 0.8, between 18 and 30 px", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  // 500 ms beats at 0.1 px/ms: half a beat is 25 px, so circles are 20 px.
  paint("scene.zoom = .1");
  assert.deepEqual(circles(ruler), [
    { x: 10, y: 50.5, diameter: 20, fill: "#e6eef2", stroke: "#1c2727" },
    { x: 40, y: 50.5, diameter: 20, fill: "#e6eef2", stroke: "#1c2727" },
    // A repeat sits on the track at the track's height; the slider end is full size.
    { x: 60, y: 50.5, diameter: 13.2, fill: "#e6eef2", stroke: "#1c2727" },
    { x: 80, y: 50.5, diameter: 20, fill: "#e6eef2", stroke: "#1c2727" },
  ]);
  // The slider track is 66% of the circle height with fully rounded ends, teal as before.
  const track = ruler.calls.find(call => call.name === "roundRect");
  assert.deepEqual(track.args.map(round), [40, 43.9, 40, 13.2, 6.6]);
  assert.ok(ruler.calls.some(call => call.name === "fill" && call.fill === "#62d6cfd9"), "slider track fill");
  // Spinners keep their thin bar.
  assert.ok(ruler.calls.some(call => call.name === "fillRect" && call.fill === "#e6eef2" && call.args[3] === 5));

  paint("scene.zoom = .25");
  assert.ok(circles(ruler).filter(circle => circle.diameter !== 19.8).every(circle => circle.diameter === 30), "30 px cap");
  paint("scene.zoom = .05");
  assert.ok(circles(ruler).filter(circle => circle.diameter !== 11.88).every(circle => circle.diameter === 18), "18 px floor");
  // Without timing every circle is 18 px.
  paint("scene.zoom = .1; scene.sections = []");
  assert.deepEqual(circles(ruler).map(circle => circle.diameter), [18, 18, 11.88, 18]);
});

test("each circle takes its size from the timing at its own time", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  // At 0.2 px/ms, 500 ms beats give 30 px circles and the 250 ms beats from 2000 ms give 20 px.
  paint(`scene.zoom = .2; scene.viewStart = 1800;
    scene.gameplay.points.push(["circle", 1900, 3], ["circle", 2200, 4])`);
  assert.deepEqual(circles(ruler).map(circle => [circle.x, circle.diameter]), [[20, 30], [80, 20]]);
});

test("slider tracks keep their rounded ends where the canvas has no roundRect()", async () => {
  const { get, paint } = await canvases({ roundRect: false });
  const ruler = get("ruler");
  paint("scene.zoom = .1");
  assert.ok(!ruler.calls.some(call => call.name === "roundRect"));
  const start = ruler.calls.findIndex(call => call.name === "moveTo" && call.fill === "#62d6cfd9");
  const path = ruler.calls.slice(start, start + 7).map(call => [call.name, ...call.args.map(round)]);
  // A 40 × 13.2 px track from x 40 with 6.6 px corners: two half circles joined by straight edges.
  assert.deepEqual(path, [
    ["moveTo", 46.6, 43.9],
    ["arcTo", 80, 43.9, 80, 57.1, 6.6],
    ["arcTo", 80, 57.1, 40, 57.1, 6.6],
    ["arcTo", 40, 57.1, 40, 43.9, 6.6],
    ["arcTo", 40, 43.9, 80, 43.9, 6.6],
    ["closePath"],
    ["fill"],
  ]);
  assert.equal(ruler.calls[start + 6].fill, "#62d6cfd9");
});

test("clicking anywhere on a drawn circle or slider track picks that object", async () => {
  const { evaluate } = await canvases();
  // Ruler 80 px tall: the waveform lane's middle is y 50.5. At 0.1 px/ms circles are 20 px
  // and the slider track 13.2 px; after 2000 ms the faster timing makes circles 18 px.
  evaluate("scene.zoom = .1; scene.gameplay.points.push(['circle', 2200, 3])");
  const target = (ms, y) => evaluate(`JSON.stringify(waveformTarget(${ms}, ${y}, 80))`);
  const pick = (object, time, node) => JSON.stringify({ object, time, node });
  assert.equal(target(190, 50.5), pick(0, 100, true), "9 px right of the circle's centre");
  assert.equal(target(100, 59.5), pick(0, 100, true), "9 px below the circle's centre");
  assert.equal(target(210, 50.5), "null", "11 px away misses");
  assert.equal(target(700, 56.5), pick(1, 400, false), "the track 6 px below its middle");
  assert.equal(target(700, 57.5), "null", "7 px below misses the 13.2 px track");
  assert.equal(target(2285, 50.5), pick(3, 2200, true), "8.5 px into an 18 px circle");
  assert.equal(target(2295, 50.5), "null", "9.5 px misses an 18 px circle");
});

test("object circles never grow past the waveform lane", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  ruler.clientHeight = 41;
  paint("scene.zoom = .25");
  assert.deepEqual(circles(ruler).map(circle => circle.diameter), [20, 20, 13.2, 20]);
});

// The x range of waveform columns that draw a bar taller than nothing.
function loudColumns(canvas) {
  const shades = ["#315c59", "#3c7673", "#46908c", "#51aaa5"];
  const xs = canvas.calls.filter(call => call.name === "fillRect" && shades.includes(call.fill) && call.args[3] > 0)
    .map(call => call.args[0]);
  return [Math.min(...xs), Math.max(...xs)];
}

test("the drawn waveform moves with the song offset on the ruler and the overview", async () => {
  const { get, paint } = await canvases();
  // One loud bin from 300 to 600 ms of the song; 0.25 px/ms on both canvases.
  paint(`view.duration = 1200; scene.zoom = .25; scene.gameplay = null;
    scene.waveform = [[0, 0, 0, 0], [1, 1, 1, 1], [0, 0, 0, 0], [0, 0, 0, 0]]`);
  const before = { ruler: loudColumns(get("ruler")), overview: loudColumns(get("overview")) };
  assert.deepEqual(before.ruler, [74, 148]);
  assert.deepEqual(before.overview, [74, 148]);
  // Playback reads the song at position - offset, so +400 ms draws it 100 px later.
  paint("view.offset = 400");
  for (const id of ["ruler", "overview"]) {
    assert.deepEqual(loudColumns(get(id)), before[id].map(x => x + 100), id);
  }
  paint("view.offset = -200");
  for (const id of ["ruler", "overview"]) {
    assert.deepEqual(loudColumns(get(id)), before[id].map(x => x - 50), id);
  }
});

test("the cycle region is a faint accent band under a warning-yellow bar on the ruler", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  const rects = () => ruler.calls.filter(call => call.name === "fillRect").map(call => [call.fill, ...call.args]);
  // Locators at 100 and 700 ms at 0.25 px/ms: the region runs from x 25 to 175.
  paint("scene.zoom = .25; Object.assign(view, { left: 100, right: 700, cycle: true })");
  assert.ok(rects().some(rect => rect.join() === ["#77bdfa0f", 25, 0, 150, 20].join()), "faint accent band");
  assert.ok(rects().some(rect => rect.join() === ["#efc16a", 25, 1, 150, 4].join()), "warning-yellow bar");
  assert.ok(!ruler.calls.some(call => [call.fill, call.stroke].some(colour => colour?.startsWith("#2fbf4a"))), "no green");
  assert.ok(ruler.calls.some(call => call.name === "stroke" && call.stroke === "#858585"), "locator lines stay");
  // With the cycle off the locator span stays grey; reversed locators stay orange.
  paint("view.cycle = false");
  assert.ok(rects().some(rect => rect.join() === ["#85858522", 25, 0, 150, 20].join()));
  assert.ok(!rects().some(rect => rect[0] === "#efc16a"));
  paint("Object.assign(view, { left: 700, right: 100, cycle: true })");
  assert.ok(rects().some(rect => rect.join() === ["#f0aa7046", 25, 0, 150, 20].join()));
  assert.ok(!rects().some(rect => rect[0] === "#efc16a"));
});

test("the 2 px object ring leaves the ruler's ticks and locator lines at 1 px", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  paint("scene.zoom = .25; Object.assign(view, { left: 100, right: 700, cycle: true })");
  const strokes = ruler.calls.filter(call => call.name === "stroke");
  assert.ok(strokes.some(call => call.stroke === "#1c2727" && call.lineWidth === 2), "object ring");
  const lines = strokes.filter(call => call.stroke !== "#1c2727");
  assert.ok(lines.some(call => call.stroke === "#858585"), "locator lines");
  assert.deepEqual([...new Set(lines.map(call => call.lineWidth))], [1]);
});

test("a click on a slider repeat counts only inside its drawn size", async () => {
  const { evaluate } = await canvases();
  evaluate("scene.zoom = .1");
  const target = (ms, y) => evaluate(`JSON.stringify(waveformTarget(${ms}, ${y}, 80))`);
  const pick = (object, time, node) => JSON.stringify({ object, time, node });
  // The repeat at 600 ms is drawn at the track's 13.2 px, not the 20 px of the head and end.
  assert.equal(target(600, 56.5), pick(1, 600, true), "6 px below the repeat's centre");
  assert.equal(target(680, 50.5), pick(1, 400, false), "8 px beside it is the track");
  assert.equal(target(600, 58.5), "null", "8 px below misses");
});

test("circle sizes and clicks stay right with thousands of timing sections", async () => {
  const { get, paint, evaluate } = await canvases();
  const ruler = get("ruler");
  // 2000 sections a second apart from 100 ms, alternating 500 ms and 250 ms beats:
  // at 0.2 px/ms those give 30 px and 20 px circles. Before the first section its timing applies.
  paint("scene.zoom = .2; scene.sections = Array.from({ length: 2000 }, (_, i) => [100 + i * 1000, i % 2 ? 250 : 500, 4])");
  const cases = [[50, 30], [600, 30], [1600, 20], [999_600, 20], [1_000_600, 30], [1_999_600, 20], [2_100_000, 20]];
  for (const [time, diameter] of cases) {
    paint(`scene.viewStart = ${time - 100}; scene.gameplay = { points: [["circle", ${time}, 0]], spans: [] }`);
    assert.deepEqual(circles(ruler).map(circle => circle.diameter), [diameter], `drawn at ${time} ms`);
    // A click just inside the circle's edge picks it; just outside misses.
    const inside = time + (diameter / 2 - .5) / .2;
    const outside = time + (diameter / 2 + .5) / .2;
    assert.equal(evaluate(`waveformTarget(${inside}, 50.5, 80)?.time`), time, `click inside at ${time} ms`);
    assert.equal(evaluate(`waveformTarget(${outside}, 50.5, 80)`), null, `click outside at ${time} ms`);
  }
});
