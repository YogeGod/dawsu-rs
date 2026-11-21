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
      calls, getContext: () => ctx, clientWidth: 300, clientHeight: 80, scrollTop: 0, listeners: {},
      dataset: {}, style: { setProperty() {} }, children: [], append() {}, prepend() {}, setAttribute() {},
      addEventListener(name, listener) { (this.listeners[name] ??= []).push(listener); },
      getBoundingClientRect: () => ({ x: 0, y: 0, left: 0, top: 0, width: 300, height: 80 }),
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

// v3 timeline-spec §9: object circles are a fixed 34 px at every zoom, filled in the map's combo
// colour, with the combo number always shown. Plain circles: the osu! skin textures are CC BY-NC.
const combos = 'scene.combos = { colours: ["#ff0000", "#00ff00"], objects: [[1, 0], [2, 1], [1, 1]] }';
const numbers = canvas => canvas.calls.filter(call => call.name === "fillText" && /^\d+$/.test(call.args[0]))
  .map(({ args: [text, x, y], fill }) => [text, round(x), y, fill]);

test("object circles are 34 px at every zoom, in the combo colour with a white ring and the combo number", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  // Ruler 80 px tall: the 40 px band, then the 40 px Song row with its middle at y 60.
  for (const zoom of [.05, .1, .25]) {
    paint(`${combos}; scene.zoom = ${zoom}`);
    assert.deepEqual(circles(ruler).map(circle => [circle.diameter, circle.fill, circle.stroke]), [
      [34, "#ff0000", "#ffffff"],
      [34, "#00ff00", "#ffffff"],
      // A repeat sits on the track at the track's height; the slider end is a full circle.
      [22.44, "#00ff00", "#ffffff"],
      [34, "#00ff00", "#ffffff"],
    ], `zoom ${zoom}`);
  }
  paint(`${combos}; scene.zoom = .1`);
  assert.deepEqual(circles(ruler).map(circle => [circle.x, circle.y]), [[10, 60], [40, 60], [60, 60], [80, 60]]);
  // Numbers on a circle and a slider head only, centred, always shown.
  assert.deepEqual(numbers(ruler), [["1", 10, 60, "#ffffff"], ["2", 40, 60, "#ffffff"]]);
  // A 2 px white ring; the ruler's ticks and locator lines stay 1 px.
  const strokes = ruler.calls.filter(call => call.name === "stroke");
  assert.ok(strokes.some(call => call.stroke === "#ffffff" && call.lineWidth === 2), "ring");
  assert.ok(strokes.filter(call => call.lineWidth === 2).every(call => ["#ffffff", "#ffffffd9"].includes(call.stroke)));
  assert.ok(strokes.some(call => call.stroke === "#858585" && call.lineWidth === 1), "locator lines");
  // Spinners keep their thin bar.
  assert.ok(ruler.calls.some(call => call.name === "fillRect" && call.fill === "#e6eef2" && call.args[3] === 5));
});

test("without combo data the circles stay readout white and unnumbered", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  paint("scene.zoom = .1");
  assert.deepEqual([...new Set(circles(ruler).map(circle => circle.fill))], ["#e6eef2"]);
  assert.deepEqual(numbers(ruler), []);
});

test("a slider track is the combo colour 38 % into #111111 with a 2 px inset white edge", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  paint(`${combos}; scene.zoom = .1`);
  const tracks = ruler.calls.filter(call => call.name === "roundRect");
  // 34 × .66 = 22.44 px tall with fully rounded ends, from x 40 to 80; the edge 1 px inside it.
  assert.deepEqual(tracks.map(call => call.args.map(round)), [[40, 48.78, 40, 22.44, 11.22], [41, 49.78, 38, 20.44, 10.22]]);
  assert.ok(ruler.calls.some(call => call.name === "fill" && call.fill === "#0b6b0b"), "#00ff00 38 % into #111111");
  assert.ok(ruler.calls.some(call => call.name === "stroke" && call.stroke === "#ffffffd9" && call.lineWidth === 2), "edge");
});

test("slider tracks keep their rounded ends where the canvas has no roundRect()", async () => {
  const { get, paint } = await canvases({ roundRect: false });
  const ruler = get("ruler");
  paint(`${combos}; scene.zoom = .1`);
  assert.ok(!ruler.calls.some(call => call.name === "roundRect"));
  const start = ruler.calls.findIndex(call => call.name === "moveTo" && call.fill === "#0b6b0b");
  const path = ruler.calls.slice(start, start + 7).map(call => [call.name, ...call.args.map(round)]);
  // A 40 × 22.44 px track from x 40 with 11.22 px corners: two half circles joined by straight edges.
  assert.deepEqual(path, [
    ["moveTo", 51.22, 48.78],
    ["arcTo", 80, 48.78, 80, 71.22, 11.22],
    ["arcTo", 80, 71.22, 40, 71.22, 11.22],
    ["arcTo", 40, 71.22, 40, 48.78, 11.22],
    ["arcTo", 40, 48.78, 80, 48.78, 11.22],
    ["closePath"],
    ["fill"],
  ]);
});

test("clicking anywhere on a drawn circle or slider track picks that object", async () => {
  const { evaluate } = await canvases();
  // At 0.2 px/ms: the circle at x 20, the slider's head, repeat and end at x 80, 120 and 160.
  evaluate("scene.zoom = .2; scene.gameplay.points.push(['circle', 2200, 3])");
  const target = (ms, y) => evaluate(`JSON.stringify(waveformTarget(${ms}, ${y}, 80))`);
  const pick = (object, time, node) => JSON.stringify({ object, time, node });
  assert.equal(target(180, 60), pick(0, 100, true), "16 px right of the circle's centre");
  assert.equal(target(100, 76), pick(0, 100, true), "16 px below the circle's centre");
  assert.equal(target(190, 60), "null", "18 px away misses");
  assert.equal(target(680, 71), pick(1, 400, false), "the track 11 px below its middle");
  assert.equal(target(680, 72), "null", "12 px below misses the 22.44 px track");
  assert.equal(target(2280, 60), pick(3, 2200, true), "16 px into a later circle");
});

test("object circles never grow past the Song row", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  ruler.clientHeight = 60;
  paint(`${combos}; scene.zoom = .25`);
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
  assert.ok(rects().some(rect => rect.join() === ["#77bdfa0f", 25, 0, 150, 39].join()), "faint accent band");
  assert.ok(rects().some(rect => rect.join() === ["#efc16a", 25, 1, 150, 4].join()), "warning-yellow bar");
  assert.ok(!ruler.calls.some(call => [call.fill, call.stroke].some(colour => colour?.startsWith("#2fbf4a"))), "no green");
  assert.ok(ruler.calls.some(call => call.name === "stroke" && call.stroke === "#858585"), "locator lines stay");
  // With the cycle off the locator span stays grey; reversed locators stay orange.
  paint("view.cycle = false");
  assert.ok(rects().some(rect => rect.join() === ["#85858522", 25, 0, 150, 39].join()));
  assert.ok(!rects().some(rect => rect[0] === "#efc16a"));
  paint("Object.assign(view, { left: 700, right: 100, cycle: true })");
  assert.ok(rects().some(rect => rect.join() === ["#f0aa7046", 25, 0, 150, 39].join()));
  assert.ok(!rects().some(rect => rect[0] === "#efc16a"));
});

test("a click on a slider repeat counts only inside its drawn size", async () => {
  const { evaluate } = await canvases();
  evaluate("scene.zoom = .2");
  const target = (ms, y) => evaluate(`JSON.stringify(waveformTarget(${ms}, ${y}, 80))`);
  const pick = (object, time, node) => JSON.stringify({ object, time, node });
  // The repeat at 600 ms is drawn at the track's 22.44 px, not the 34 px of the head and end.
  assert.equal(target(600, 71), pick(1, 600, true), "11 px below the repeat's centre");
  assert.equal(target(680, 60), pick(1, 400, false), "16 px beside it is the track");
  assert.equal(target(600, 73), "null", "13 px below misses");
});

test("slider-track clicks work where arrays have no findLast()", async () => {
  const { evaluate } = await canvases();
  // Older WebKit WebViews on macOS and Linux lack Array.prototype.findLast.
  evaluate("scene.zoom = .2; delete Array.prototype.findLast");
  assert.equal(evaluate("JSON.stringify(waveformTarget(700, 66, 80))"),
    JSON.stringify({ object: 1, time: 400, node: false }));
  // Overlapping spans: the later one wins, as before.
  evaluate("scene.gameplay.spans.push(['slider', 650, 750, 5])");
  assert.equal(evaluate("waveformTarget(700, 66, 80).object"), 5);
});

test("the left and right locators are L and R flags at the top of the ruler", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  const letters = () => ruler.calls.filter(call => call.name === "fillText" && /^[LR]$/.test(call.args[0]))
    .map(({ args: [text, x], fill }) => [text, Math.round(x), fill]);
  // Locators at 100 and 700 ms at 0.25 px/ms: lines at x 25 and 175, flags inside the range.
  paint("scene.zoom = .25; Object.assign(view, { left: 100, right: 700, cycle: false })");
  const [left, right] = letters();
  assert.equal(left[0], "L");
  assert.ok(left[1] > 25 && left[1] < 39, "L sits right of its line");
  assert.equal(right[0], "R");
  assert.ok(right[1] > 161 && right[1] < 175, "R sits left of its line");
  assert.ok(ruler.calls.some(call => call.name === "fill" && call.fill === "#858585"), "grey flags");
  paint("view.cycle = true");
  assert.ok(ruler.calls.some(call => call.name === "fill" && call.fill === "#efc16a"), "cycle-yellow flags while cycling");
  // Both on one time, and the left one at the right edge of the 300 px canvas: both stay visible.
  paint("Object.assign(view, { left: 1180, right: 1180 })");
  const [l, r] = letters();
  assert.ok(l[1] < 295 && r[1] < 295 && l[1] !== r[1], JSON.stringify([l, r]));
});

test("a pushed L or R flag never paints over the other one, at either edge of the ruler", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  // Each flag is one filled path: moveTo, lineTo… then fill in the flag colour.
  const flagSpans = () => {
    const spans = [];
    let xs = [];
    for (const call of ruler.calls) {
      if (call.name === "beginPath") xs = [];
      if (call.name === "moveTo" || call.name === "lineTo") xs.push(call.args[0]);
      if (call.name === "fill" && ["#858585", "#efc16a"].includes(call.fill)) spans.push([Math.min(...xs), Math.max(...xs)]);
    }
    return spans;
  };
  // Locators 5 and 8 px from the left edge, 2 and 2 px, and both at the right edge (300 px canvas).
  for (const [left, right] of [[20, 32], [8, 8], [1188, 1192]]) {
    paint(`scene.zoom = .25; Object.assign(view, { left: ${left}, right: ${right}, cycle: false })`);
    const [l, r] = flagSpans();
    assert.ok(l && r, "two flags");
    assert.ok(l[1] <= r[0] || r[1] <= l[0], `[${left}, ${right}] flags overlap: ${JSON.stringify([l, r])}`);
    for (const [from, to] of [l, r]) assert.ok(from >= 0 && to <= 300, `[${left}, ${right}] flag off the ruler`);
  }
});

// Only while folded, a main lane also shows its layers' hits, as outline-only diamonds in each
// hit's volume colour, so they show volume and can't pass for muted hits.
test("a folded main lane outlines its layers' hits in their volume colours; unfolded, each hit draws once on its own row", async () => {
  const { get, paint, evaluate } = await canvases();
  const grid = get("grid");
  get("scroll").clientHeight = 200;
  evaluate(`
    view = { ...view, activeLane: null };
    scene.lanes = [
      { key: "F0", colourKey: "Skin|normal-hitfinish", name: "normal-hitfinish", family: "normal-hitfinish", index: 0, group: "normal" },
      { key: "F2", colourKey: "Skin|normal-hitfinish2", name: "normal-hitfinish2", family: "normal-hitfinish", index: 2, group: "normal" },
      { key: "F3", colourKey: "Skin|normal-hitfinish3", name: "normal-hitfinish3", family: "normal-hitfinish", index: 3, group: "normal" },
    ];
    scene.moments = [[1000, 80, true, [[0, false]]], [1500, 30, true, [[1, false]]], [2000, 100, true, [[2, false]]]];
    renderLanes();
  `);
  // Diamonds only: a fill or stroke right after the diamond's closePath (grid lines stroke too).
  const named = name => grid.calls.filter((call, at) => call.name === name && grid.calls[at - 1]?.name === "closePath");
  // A diamond starts at its top point, then goes down and right to its right point.
  const tops = () => grid.calls.filter((call, at) => call.name === "moveTo" && grid.calls[at + 1]?.name === "lineTo" &&
    grid.calls[at + 1].args[1] - call.args[1] > 0 && grid.calls[at + 1].args[1] - call.args[1] === grid.calls[at + 1].args[0] - call.args[0])
    .map(call => call.args[1]);
  const volume = value => evaluate(`volumeColour(${value})`);

  paint("");
  assert.deepEqual(named("fill").map(call => call.fill), [volume(80), volume(30), volume(100)]);
  assert.deepEqual(named("stroke"), [], "unfolded: no outlines");
  assert.deepEqual(tops(), [45 - 6, 75 - 6, 105 - 6], "each hit once, on its own row");

  paint('laneHeaderAction("family", 0)');
  // Row 0 is the folder header, row 1 the main lane: its own hit filled, its layers' outlined.
  assert.deepEqual(named("fill").map(call => call.fill), [volume(80)]);
  assert.deepEqual(named("stroke").map(call => call.stroke), [volume(30), volume(100)]);
  assert.deepEqual(tops(), [45 - 6, 45 - 7.5, 45 - 7.5], "all three on the main lane's row, outlines just outside a hit");

  // Two layers and the main lane hit one moment: the main hit fills first, then each layer's outline
  // nests a little larger, so every volume colour shows.
  evaluate(`scene.moments = [[1000, 80, true, [[0, false], [1, false, false, 30], [2, false, false, 100]]]]`);
  paint("");
  const order = grid.calls.filter((call, at) => ["fill", "stroke"].includes(call.name) && grid.calls[at - 1]?.name === "closePath")
    .map(call => call.name === "fill" ? ["fill", call.fill] : ["stroke", call.stroke]);
  assert.deepEqual(order, [["fill", volume(80)], ["stroke", volume(30)], ["stroke", volume(100)]]);
  assert.deepEqual(tops(), [45 - 6, 45 - 7.5, 45 - 10]);
});

test("the ruler band is 40 px of #0c0c0c with a 1 px --editor-selected bottom edge", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  paint("scene.zoom = .1");
  assert.ok(ruler.calls.some(call => call.name === "fillRect" && call.fill === "#0c0c0c" &&
    call.args.join() === "0,0,300,40"), "band");
  const edge = ruler.calls.findIndex(call => call.name === "lineTo" && call.args[1] === 39.5 && call.args[0] === 300.5);
  assert.ok(edge > 0 && ruler.calls[edge - 1].args.join() === "0.5,39.5", "bottom edge");
  assert.ok(ruler.calls.slice(edge).some(call => call.name === "stroke" && call.stroke === "#454545"));
});

test("under the last lane the timeline shows the Add lane row's hint", async () => {
  const { get, paint, evaluate } = await canvases();
  const grid = get("grid");
  get("scroll").clientHeight = 200;
  evaluate(`scene.lanes = [{ key: "A", colourKey: "Skin|normal-hitclap", name: "normal-hitclap", group: "normal" }];
    scene.moments = []; renderLanes();`);
  paint("");
  const rows = evaluate("laneRows.length");
  const hint = grid.calls.find(call => call.name === "fillText" && call.args[0] === "Drop a sample from the Browser to add a lane");
  assert.ok(hint, "hint");
  // Centred in the row after the last lane row, 30 px rows.
  assert.equal(hint.args[2], rows * 30 + 15);
  // In the hint's row the grid draws no lane row of its own.
  assert.equal(evaluate("laneRows[" + rows + "]"), undefined);
});

// v3 f1 and states.md: hovering a hit shows a card with its lane and set, time, volume and object.
test("hovering a hit hands over one card per hit and never repaints the timeline", async () => {
  const { get, evaluate } = await canvases();
  const grid = get("grid");
  get("scroll").clientHeight = 200;
  evaluate(`
    window.dawsuUi = { cards: [], hitCard(card) { this.cards.push(card); } };
    view = { ...view, tool: "Select", activeLane: null };
    scene.combos = { colours: ["#ff0000"], objects: [[1, 0], [2, 0], [3, 0]] };
    scene.lanes = [{ key: "A", colourKey: "Skin|normal-hitclap", name: "normal-hitclap", family: "normal-hitclap", index: 0, group: "normal" }];
    scene.moments = [[100, 60, true, [[0, false]]], [400, 80, true, [[0, false, false, 45]]], [1500, 70, false, [[0, false]]]];
    renderLanes();`);
  const move = (x, y) => {
    for (const canvas of [grid, get("ruler"), get("overview")]) canvas.calls.length = 0;
    for (const listener of grid.listeners.pointermove) listener({ clientX: x, clientY: y, pointerId: 1 });
  };
  const cards = () => JSON.parse(evaluate("JSON.stringify(window.dawsuUi.cards)"));
  // Row 0 is the Normal set folder, row 1 the Clap lane (30 px rows); 0.1 px/ms.
  move(12, 45);
  assert.deepEqual(cards(), [{ key: "1:100", title: "Clap · Normal set", time: "00:00.100",
    detail: "Volume 60% · on circle 1", x: 10, y: 45 }]);
  for (const canvas of [grid, get("ruler"), get("overview")]) assert.deepEqual(canvas.calls, [], "no repaint");
  // Still on the same hit: nothing new. Its own hit volume wins over the moment's.
  move(9, 47);
  assert.equal(cards().length, 1);
  move(41, 45);
  assert.equal(cards().at(-1).detail, "Volume 45% · on slider 2");
  move(150, 45);
  assert.equal(cards().at(-1).detail, "Volume 70% · no object");
  // Away from every hit the card goes, once.
  move(100, 45);
  move(110, 45);
  assert.deepEqual(cards().slice(-2), [cards().at(-2), null]);
  assert.equal(cards().filter(card => card === null).length, 1);
  move(12, 45);
  for (const listener of grid.listeners.pointerleave) listener({});
  assert.equal(cards().at(-1), null);
});
