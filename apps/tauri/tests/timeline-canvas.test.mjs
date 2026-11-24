import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";


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
  bootEditor(context, { autoStart: false });
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
// Combo numbers sit on the Song row, under the 40 px ruler band that holds the bar numbers.
const numbers = canvas => canvas.calls.filter(call => call.name === "fillText" && /^\d+$/.test(call.args[0]) && call.args[2] > 40)
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
  assert.ok(strokes.some(call => call.stroke === "#8e8e96" && call.lineWidth === 1), "locator lines");
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
  const xs = [];
  for (const call of canvas.calls) {
    if (call.name === "fillRect" && call.fill?.startsWith("rgba(") && call.args[2] === 1 && call.args[3] > 4) {
      xs.push(call.args[0]);
    }
  }
  return [Math.min(...xs), Math.max(...xs)];
}

test("the drawn waveform moves with the song offset on the ruler", async () => {
  const { get, paint } = await canvases();
  // One loud bin from 300 to 600 ms of the song at 0.25 px/ms.
  paint(`view.duration = 1200; scene.zoom = .25; scene.gameplay = null;
    scene.waveform = [[0, 0, 0, 0], [1, 1, 1, 1], [0, 0, 0, 0], [0, 0, 0, 0]]`);
  const before = loudColumns(get("ruler"));
  assert.equal(before[0], 75);
  assert.ok(before[1] >= 148 && before[1] <= 150);
  // Playback reads the song at position - offset, so +400 ms draws it 100 px later.
  paint("view.offset = 400");
  assert.deepEqual(loudColumns(get("ruler")), before.map(x => x + 100));
  paint("view.offset = -200");
  assert.deepEqual(loudColumns(get("ruler")), before.map(x => x - 50));
});

test("the cycle region is a faint accent band under a warning-yellow bar on the ruler", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  const rects = () => ruler.calls.filter(call => call.name === "fillRect").map(call => [call.fill, ...call.args]);
  // Locators at 100 and 700 ms at 0.25 px/ms: the region runs from x 25 to 175.
  paint("scene.zoom = .25; Object.assign(view, { left: 100, right: 700, cycle: true })");
  assert.ok(rects().some(rect => rect.join() === ["#5ac8fa14", 25, 0, 150, 39].join()), "faint accent band");
  assert.ok(rects().some(rect => rect.join() === ["#ffd60a", 25, 0, 150, 5].join()), "warning-yellow bar");
  assert.ok(!ruler.calls.some(call => [call.fill, call.stroke].some(colour => colour?.startsWith("#2fbf4a"))), "no green");
  assert.ok(ruler.calls.some(call => call.name === "stroke" && call.stroke === "#8e8e96"), "locator lines stay");
  // With the cycle off the locator span stays grey; reversed locators stay orange.
  paint("view.cycle = false");
  assert.ok(rects().some(rect => rect.join() === ["#8e8e9622", 25, 0, 150, 39].join()));
  assert.ok(!rects().some(rect => rect[0] === "#ffd60a"));
  paint("Object.assign(view, { left: 700, right: 100, cycle: true })");
  assert.ok(rects().some(rect => rect.join() === ["#f0aa7046", 25, 0, 150, 39].join()));
  assert.ok(!rects().some(rect => rect[0] === "#ffd60a"));
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
  assert.ok(ruler.calls.some(call => call.name === "fill" && call.fill === "#8e8e96"), "grey flags");
  paint("view.cycle = true");
  assert.ok(ruler.calls.some(call => call.name === "fill" && call.fill === "#ffd60a"), "cycle-yellow flags while cycling");
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
      if (call.name === "fill" && ["#8e8e96", "#ffd60a"].includes(call.fill)) spans.push([Math.min(...xs), Math.max(...xs)]);
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
  assert.deepEqual(tops(), [45 - 8, 75 - 8, 105 - 8], "each hit once, on its own row");

  paint('laneHeaderAction("family", 0)');
  // Row 0 is the folder header, row 1 the main lane: its own hit filled, its layers' outlined.
  assert.deepEqual(named("fill").map(call => call.fill), [volume(80)]);
  assert.deepEqual(named("stroke").map(call => call.stroke), [volume(30), volume(100)]);
  assert.deepEqual(tops(), [45 - 8, 45 - 9.5, 45 - 9.5], "all three on the main lane's row, outlines just outside a hit");

  // Two layers and the main lane hit one moment: the main hit fills first, then each layer's outline
  // nests a little larger, so every volume colour shows.
  evaluate(`scene.moments = [[1000, 80, true, [[0, false], [1, false, false, 30], [2, false, false, 100]]]]`);
  paint("");
  const order = grid.calls.filter((call, at) => ["fill", "stroke"].includes(call.name) && grid.calls[at - 1]?.name === "closePath")
    .map(call => call.name === "fill" ? ["fill", call.fill] : ["stroke", call.stroke]);
  assert.deepEqual(order, [["fill", volume(80)], ["stroke", volume(30)], ["stroke", volume(100)]]);
  assert.deepEqual(tops(), [45 - 8, 45 - 9.5, 45 - 12]);
});

test("the ruler band is 40 px of #0c0c0c with a 1 px --editor-selected bottom edge", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  paint("scene.zoom = .1");
  assert.ok(ruler.calls.some(call => call.name === "fillRect" && call.fill === "#08080a" &&
    call.args.join() === "0,0,300,40"), "band");
  const edge = ruler.calls.findIndex(call => call.name === "lineTo" && call.args[1] === 39.5 && call.args[0] === 300.5);
  assert.ok(edge > 0 && ruler.calls[edge - 1].args.join() === "0.5,39.5", "bottom edge");
  assert.ok(ruler.calls.slice(edge).some(call => call.name === "stroke" && call.stroke === "#2b2b30"));
});

test("the Add lane row's drop hint stays hidden until a Browser drop adds a lane", async () => {
  const { get, paint, evaluate } = await canvases();
  const grid = get("grid");
  get("scroll").clientHeight = 200;
  evaluate(`scene.lanes = [{ key: "A", colourKey: "Skin|normal-hitclap", name: "normal-hitclap", group: "normal" }];
    scene.moments = []; renderLanes();`);
  paint("");
  const rows = evaluate("laneRows.length");
  // Yoge: the hint waits for the drag from the Browser, which comes with the unused-files work.
  assert.ok(!grid.calls.some(call => call.name === "fillText"), "no hint");
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
  assert.deepEqual(cards(), [{ key: "1:100:10:45:Clap · Normal set:Volume 60% · on circle 1", title: "Clap · Normal set", time: "00:00.100",
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

  // A new scene that removes the hovered hit (Delete, Undo) takes the card away without a pointer move.
  move(12, 45);
  assert.equal(cards().at(-1).title, "Clap · Normal set");
  // An edit that changes the hit's volume (or Undo) updates the card in place.
  evaluate(`scene = { ...scene, moments: [[100, 90, true, [[0, false]]], ...scene.moments.slice(1)] }; refreshHitCard()`);
  assert.equal(cards().at(-1).detail, "Volume 90% · on circle 1");
  // show() calls refreshHitCard() with each new scene.
  evaluate(`scene = { ...scene, moments: scene.moments.slice(1) }; refreshHitCard()`);
  assert.equal(cards().at(-1), null);
  // A zoom by key moves the hit under the still pointer: the card follows it, or goes when the hit
  // leaves the pointer.
  move(41, 45);
  evaluate(`scene = { ...scene, viewStart: 205, zoom: .2 }; refreshHitCard()`);
  assert.equal(cards().at(-1).x, 39);
  evaluate(`scene = { ...scene, viewStart: 0 }; refreshHitCard()`);
  assert.equal(cards().at(-1), null);
  // Home, a closed map or a job starting takes the card away (show() refreshes it every time).
  move(12, 45);
  evaluate("view.busy = true; refreshHitCard(); view.busy = false");
  assert.equal(cards().at(-1), null);
  assert.match(evaluate("show.toString()"), /^\s+(?:ctx\.)?refreshHitCard\(\);$/m);
  // No card while a menu is open.
  evaluate("$('editor-menu').hidden = false");
  move(12, 45);
  assert.equal(cards().at(-1), null);
  evaluate("$('editor-menu').hidden = true");
});

// timeline-spec "Overview strip": the whole song in 300 columns of 5 px diamonds whose strength shows
// how many objects fall in them, problems as 7 px warning diamonds, and the viewport box.
test("the overview draws object diamonds by density, problem diamonds and the viewport box", async () => {
  const { get, paint } = await canvases();
  const overview = get("overview");
  paint(`view.duration = 4000; scene.zoom = .1; scene.waveform = [[1, 1, 1, 1]];
    scene.gameplay = { points: [["circle", 100, 0], ["circle", 105, 1], ["circle", 2000, 2]], spans: [] };
    view.outputProblems = [{ timeline: "fixture.osu", time: 3000 }, { timeline: "other.osu", time: 1000 }]`);
  // A diamond is moveTo its top point, three lineTo, closePath, then fill.
  const diamonds = overview.calls.flatMap((call, at) => call.name === "fill" && overview.calls[at - 1]?.name === "closePath" &&
    overview.calls[at - 5]?.name === "moveTo" ? [[round(overview.calls[at - 5].args[0]), overview.calls[at - 5].args[1], call.fill]] : []);
  assert.deepEqual(diamonds, [
    // Two objects in column 7 (the densest, full strength), one in column 150; 80 px tall, middle at 40.
    [7.5, 36.5, "rgba(196,196,196,1.00)"],
    [150.5, 36.5, "rgba(196,196,196,0.65)"],
    // Only this difficulty's problems, above the viewport box.
    [225, 35, "#fbbf24"],
  ]);
  assert.ok(overview.calls.some(call => call.name === "stroke" && call.stroke === "#151517"), "problem edge");
  // The viewport box: 3000 ms of 4000 on screen, 1 px --editor-text with a 2 px radius.
  const box = overview.calls.find(call => call.name === "roundRect");
  assert.deepEqual(box.args.map(round), [.5, 3.5, 224, 73, 2]);
  assert.ok(overview.calls.some(call => call.name === "stroke" && call.stroke === "#f1f1f1"));
  assert.ok(overview.calls.some(call => call.name === "fill" && call.fill === "rgba(241,241,241,.06)"));
  // Diamonds, not the song's waveform.
  assert.ok(!overview.calls.some(call => ["#315c59", "#3c7673", "#46908c", "#51aaa5"].includes(call.fill)));
  // A tiny viewport keeps an 8 px box.
  paint("scene.zoom = 3");
  assert.equal(overview.calls.find(call => call.name === "roundRect").args[2], 7);
  // At the song's end the box stays inside the strip.
  paint("scene.zoom = .1; scene.viewStart = 3900");
  const end = overview.calls.find(call => call.name === "roundRect").args;
  assert.ok(end[0] + end[2] <= 300, JSON.stringify(end));
});

test("with the waveform switched off the Song row keeps its circles and digits, without the waveform", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  paint(`${combos}; scene.zoom = .1; scene.waveform = [[1, 1, 1, 1]]; view.duration = 4000; waveformShown = false`);
  assert.equal(loudColumns(ruler).filter(Number.isFinite).length, 0, "no waveform");
  assert.deepEqual(circles(ruler).map(circle => circle.diameter), [34, 34, 22.44, 34]);
  assert.deepEqual(numbers(ruler).map(([text]) => text), ["1", "2"]);
  paint("waveformShown = true");
  const cols = loudColumns(ruler);
  assert.equal(cols[0], 0);
  assert.ok(cols[1] >= 298);
});

test("combo digits carry a dark outline so they read on yellow and green", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  paint(`${combos}; scene.zoom = .1`);
  const outlines = ruler.calls.filter(call => call.name === "strokeText").map(({ args: [text], stroke }) => [text, stroke]);
  assert.deepEqual(outlines, [["1", "rgba(0,0,0,.55)"], ["2", "rgba(0,0,0,.55)"]]);
});

test("zoomed out, circles under their neighbour are skipped and covered digits are left out", async () => {
  const { get, paint } = await canvases();
  const ruler = get("ruler");
  const three = `scene.combos = { colours: ["#ff0000", "#00ff00"], objects: [[1, 0], [2, 1], [3, 1]] };
    scene.gameplay = { points: [["circle", 100, 0], ["circle", 300, 1], ["circle", 1000, 2]], spans: [] }`;
  // 0.06 px/ms: circles at x 6, 18 and 60. The second circle covers the first one's digit.
  paint(`${three}; scene.zoom = .06`);
  assert.deepEqual(numbers(ruler).map(([text]) => text), ["2", "3"]);
  assert.equal(circles(ruler).length, 3);
  // At 0.001 px/ms the first two sit under 2 px from the next one: only the last is drawn.
  paint(`${three}; scene.zoom = .001`);
  assert.equal(circles(ruler).length, 1);
});

test("the overview counts each object once, spinners included, and merges problems on one pixel", async () => {
  const { get, paint } = await canvases();
  const overview = get("overview");
  // A slider's three nodes are one object; a spinner counts too; two problems share a pixel.
  paint(`view.duration = 3000; scene.gameplay = { points: [["slider", 100, 0], ["slider", 200, 0], ["slider", 300, 0]],
    spans: [["slider", 100, 300, 0], ["spinner", 1500, 2000, 1]] };
    view.outputProblems = [{ timeline: "fixture.osu", time: 1000 }, { timeline: "fixture.osu", time: 1002 }]`);
  const fills = overview.calls.filter((call, at) => call.name === "fill" && overview.calls[at - 1]?.name === "closePath").map(call => call.fill);
  assert.deepEqual(fills, ["rgba(196,196,196,1.00)", "rgba(196,196,196,1.00)", "#fbbf24"]);
});

test("zoomed out, problems under 8 px apart on the overview are one amber diamond; Sound-first hides them all", async () => {
  const { get, paint, evaluate } = await canvases();
  const overview = get("overview"), ruler = get("ruler");
  // 300 px for 3000 ms: 1000 and 1050 ms sit 5 px apart (one diamond), 2000 ms stands alone.
  const problems = `view.duration = 3000; scene.gameplay = { points: [], spans: [] };
    view.outputProblems = [{ timeline: "fixture.osu", time: 1000 }, { timeline: "fixture.osu", time: 1050 },
      { timeline: "fixture.osu", time: 2000 }]`;
  const amber = () => overview.calls.filter((call, at) => call.name === "fill" && overview.calls[at - 1]?.name === "closePath" &&
    call.fill === "#fbbf24").length;
  paint(problems);
  assert.equal(amber(), 2);
  assert.ok(!ruler.calls.some(call => call.fill === "#f0aa70" || call.fill === "#efc16a"), "no old red-orange markers");
  paint(`${problems}; soundFirst = true`);
  assert.equal(amber(), 0);
  assert.deepEqual(evaluate("rulerWarningMarkers"), []);
  assert.ok(!ruler.calls.some(call => call.name === "fillRect" && call.fill === "#f59e0b"), "no ruler warning squares");
  paint(`${problems}; soundFirst = false`);
  assert.equal(amber(), 2);
  assert.ok(ruler.calls.some(call => call.name === "fillRect" && call.fill === "#f59e0b"), "amber ruler warning squares");
  // The Song row's one-click switch flips it and shows its state.
  get("ruler-sound-first").listeners.click[0]();
  assert.equal(evaluate("soundFirst"), true);
  paint("0");
  assert.equal(amber(), 0);
  get("ruler-sound-first").listeners.click[0]();
  assert.equal(evaluate("soundFirst"), false);
});

test("a click on two circles at one spot picks the later object, the one drawn on top", async () => {
  const { evaluate } = await canvases();
  evaluate("scene.zoom = .1; scene.gameplay = { points: [['circle', 100, 0], ['circle', 100, 1]], spans: [] }");
  assert.equal(evaluate("waveformTarget(100, 60, 80).object"), 1);
});

test("classic skin textures draw full-height slider tracks, tinted circles, overlays, reverse arrows and digits", async () => {
  const { get, paint, evaluate } = await canvases();
  const ruler = get("ruler");
  evaluate(`
    const img = (name, w = 128, h = 128) => ({ name, naturalWidth: w, naturalHeight: h, width: w, height: h });
    skinTextures = {
      ready: true,
      hitcircle: img("hitcircle"),
      overlay: img("overlay"),
      reverseArrow: img("reverseArrow"),
      digits: Array.from({ length: 10 }, (_, i) => img("d" + i, 64, 96)),
      circleFill: 0.92,
      tinted: new Map(),
    };
  `);
  paint(`${combos}; scene.zoom = .1`);
  // Slider body is 0.66 × 34 px = 22.44 px tall (v3 timeline-spec §9) with rounded pill ends.
  const tracks = ruler.calls.filter(call => call.name === "roundRect");
  assert.deepEqual(tracks.map(call => call.args.map(round)), [
    [40, 48.78, 40, 22.44, 11.22],
    [41, 49.78, 38, 20.44, 10.22],
  ]);
  // No fallback vector arcs are drawn when skin textures are active.
  assert.equal(circles(ruler).length, 0);
  // Tinted hitcircle canvases are cached per combo colour (2 colours: #ff0000 and #00ff00).
  assert.equal(evaluate("skinTextures.tinted.size"), 2);
  // Drawn images include overlay, reverseArrow (for the repeat), and combo digits d1 and d2.
  const drawn = ruler.calls.filter(call => call.name === "drawImage").map(call => call.args[0]?.name ?? "tinted");
  assert.ok(drawn.includes("overlay"), "hitcircleoverlay drawn");
  assert.ok(drawn.includes("reverseArrow"), "reversearrow drawn on slider repeat");
  assert.ok(drawn.includes("d1") && drawn.includes("d2"), "skin combo digits drawn on circle and slider head");
});

test("while playing, a frame copies the cached layers back and draws only the playhead until something changes", async () => {
  const { get, paint, evaluate } = await canvases();
  const grid = get("grid");
  const names = canvas => canvas.calls.map(call => call.name);
  const clear = () => { for (const id of ["grid", "ruler", "overview"]) get(id).calls.length = 0; };
  paint("view.playing = true; view.position = 1000");
  assert.ok(names(grid).includes("fillRect"), "the first frame paints every layer");
  assert.equal(evaluate("staticLayers"), null, "one paint is not copied: Centre may scroll every frame");
  evaluate("paintFrame()");
  assert.notEqual(evaluate("staticLayers"), null, "the same view painted twice is copied");
  // The editor's playback poll: a new view object, a later position, no scene.
  evaluate("show({ ...view, position: 1085 })");
  clear();
  evaluate("paintFrame()");
  // Copy back (transform reset, drawImage, transform restored), then one playhead line.
  assert.deepEqual(names(grid), ["setTransform", "drawImage", "setTransform", "beginPath", "moveTo", "lineTo", "stroke"]);
  const drawnAt = grid.calls.find(call => call.name === "moveTo").args[0];
  assert.ok(Math.abs(drawnAt - (evaluate("playheadPosition()") * 0.1 + 0.5)) < 1, "the playhead is drawn where it is now");
  assert.ok(!names(get("ruler")).includes("fillText"), "the ruler's bar numbers come from the copy");
  // A poll that changes what is drawn (here the locators) paints every layer again.
  evaluate("show({ ...view, position: 1170, right: 2000 })");
  clear();
  evaluate("paintFrame()");
  assert.ok(names(grid).includes("fillRect"));
  // A scroll in time, or anything else paintSoon() hears about, does too.
  evaluate("paintFrame()");
  clear();
  evaluate("scene.viewStart = 50; paintFrame()");
  assert.ok(names(grid).includes("fillRect"));
  evaluate("paintFrame()");
  clear();
  evaluate("paintSoon(); paintFrame()");
  assert.ok(names(grid).includes("fillRect"));
  // Stopped, nothing is copied.
  evaluate("view.playing = false; paint(); paint()");
  assert.equal(evaluate("staticLayers"), null);
});

test("the waveform builds each band colour once and reuses it", async () => {
  const source = await readFile(new URL("../ui/src/editor/canvas.js", import.meta.url), "utf8");
  assert.match(source, /waveColours\[index\] \?\?= blendWaveColour/);
});

test("volume editor lane shows only the active lane unless hits are selected across lanes", async () => {
  const { get, paint, evaluate } = await canvases();
  const controller = get("controller");
  get("volume-controller-panel").hidden = false;
  get("scroll").clientHeight = 200;
  evaluate(`
    view = { ...view, activeLane: 0 };
    scene.lanes = [
      { key: "l0", colourKey: "Skin|normal-hitnormal", name: "normal-hitnormal", group: "normal" },
      { key: "l1", colourKey: "Skin|normal-hitclap", name: "normal-hitclap", group: "normal" },
      { key: "l2", colourKey: "Skin|normal-hitfinish", name: "normal-hitfinish", group: "normal" },
    ];
    scene.moments = [
      [500, 80, true, [[0, false, false, null]]],
      [1000, 60, true, [[1, false, false, null]]],
      [1500, 90, true, [[0, false, false, 70], [1, false, false, 40]]],
      [2000, 50, true, [[2, false, false, null]]],
    ];
    renderLanes();
  `);
  // Volume bars are 5 px wide rects (bar body + 5x5 top cap); group by rounded x centre.
  const barXs = () => [...new Set(
    controller.calls
      .filter(call => call.name === "fillRect" && call.args[2] === 5 && call.args[3] !== 5)
      .map(call => Math.round(call.args[0] + 2.5)),
  )];
  const ticks = () => controller.calls
    .filter(call => call.name === "fillRect" && call.args[2] === 6 && call.fill === "#ffffff")
    .map(call => Math.round(call.args[0] + 3));

  // Only activeLane (0) hits at 500 ms (x=50) and 1500 ms (x=150) appear; no stray tick from lane 1.
  paint("");
  assert.deepEqual(barXs(), [50, 150]);
  assert.deepEqual(ticks(), []);

  // Switching activeLane to 1 shows only lane 1's moments (1000 ms -> x=100, 1500 ms -> x=150).
  paint("view.activeLane = 1");
  assert.deepEqual(barXs(), [100, 150]);
  assert.deepEqual(ticks(), []);

  // Selecting hits across lane 0 and lane 2 shows bars for lanes 0 and 2 (500, 1500, 2000 ms), not lane 1 (1000 ms).
  paint("view.activeLane = 0; scene.moments[0][3][0][1] = true; scene.moments[3][3][0][1] = true;");
  assert.deepEqual(barXs(), [50, 150, 200]);

  // Selecting hits across lane 0 and lane 1 shows both and draws the white tick at 1500 ms for lane 1's differing volume.
  paint("scene.moments[3][3][0][1] = false; scene.moments[1][3][0][1] = true;");
  assert.deepEqual(barXs(), [50, 100, 150]);
  assert.deepEqual(ticks(), [150]);
});
