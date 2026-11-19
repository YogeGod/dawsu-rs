import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
const css = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");

// Values from the design handoff's tokens.css (--dark-* palette, --editor-* roles).
const surfaces = {
  "--base": "#151515", "--field": "#151515", "--lane-alt": "#151515",
  "--panel": "#1c1c1c", "--lane": "#1c1c1c",
  "--chrome": "#292929", "--raised": "#292929", "--group": "#292929",
  "--ruler": "#252525", "--button": "#2c2c2c", "--control": "#3c3c3c",
  "--line": "#383838", "--divider": "#383838", "--button-border": "#383838",
  "--text": "#f1f1f1", "--title-text": "#c4c4c4", "--muted": "#a3a3a3",
  "--hi": "#77bdfa", "--hi-hover": "#a1d0f7", "--on-accent": "#101010",
};
const blueGreys = [
  "#171b22", "#20252e", "#262d37", "#303a48", "#15181e", "#303946", "#394655", "#38414f",
  "#eff2f6", "#b0bac9", "#06121f", "#050b13", "#384454", "#78bfff", "#70b4ff",
];

test("every :root block gives the editor surfaces the design's neutral greys", () => {
  const blocks = [...css.matchAll(/:root\s*\{([^}]*)\}/g)].map(match => match[1]);
  assert.equal(blocks.length, 2);
  for (const [name, value] of Object.entries(surfaces)) {
    const declared = blocks.flatMap(block =>
      [...block.matchAll(new RegExp(`${name}:\\s*(#[0-9a-f]+)`, "gi"))].map(match => match[1].toLowerCase()));
    assert.ok(declared.length > 0, `${name} is declared`);
    assert.deepEqual(declared, declared.map(() => value), name);
  }
  for (const hex of blueGreys) assert.ok(!css.toLowerCase().includes(hex), `style.css still uses ${hex}`);
});

async function paintedCanvases() {
  const elements = new Map();
  function element() {
    const fills = [];
    const strokes = [];
    const arcs = [];
    const state = {};
    const ctx = new Proxy(state, {
      get(target, name) {
        if (name in target) return target[name];
        if (["fillRect", "fill", "fillText"].includes(name)) return () => fills.push(target.fillStyle);
        if (name === "stroke" || name === "strokeRect") return () => strokes.push(target.strokeStyle);
        if (name === "arc") return () => arcs.push({ fill: target.fillStyle, stroke: target.strokeStyle });
        return () => {};
      },
    });
    return {
      fills, strokes, arcs, getContext: () => ctx, clientWidth: 300, clientHeight: 80, scrollTop: 0,
      dataset: {}, style: {}, children: [], addEventListener() {}, append() {}, prepend() {},
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
  runInContext(`
    view = { hasDocument: true, duration: 1200, position: 300, left: 100, right: 700,
      objectLines: true, tool: "Select" };
    scene = { path: "fixture.osu", viewStart: 0, zoom: .25, rowHeight: 30, sections: [[0, 100, 4]],
      waveform: [[.4, .38, .12, .02], [.6, .18, .54, .17], [.8, .025, .19, .68]],
      gameplay: { points: [["circle", 200], ["slider", 600]], spans: [["slider", 600, 800], ["spinner", 900, 1000]] },
      lanes: [{ group: "soft" }, { group: "normal" }],
      moments: [[200, 50, true, [[0, true, false], [1, false, true]]]],
      range: [100, 300], rampPreview: [[100, 20], [300, 80]] };
    laneRows = [0, 1];
    laneVisualPositions = new Map([[0, [0]], [1, [1]]]);
    drag = { surface: grid, firstPoint: { x: 10, y: 5 }, lastPoint: { x: 60, y: 40 } };
    paint();`, context);
  return { ruler: get("ruler"), grid: get("grid"), overview: get("overview"), volume: get("controller") };
}

test("timeline canvases paint with the design's colours", async () => {
  const { ruler, grid, overview, volume } = await paintedCanvases();
  // Song waveform: teal shades on --editor-tealtint, bass to treble.
  assert.equal(ruler.fills[0], "#1c2727");
  for (const shade of ["#315c59", "#3c7673", "#46908c", "#51aaa5"]) {
    assert.ok(ruler.fills.includes(shade), `ruler waveform ${shade}`);
    assert.ok(overview.fills.includes(shade), `overview waveform ${shade}`);
  }
  // Gameplay objects: --editor-readout bodies ringed in --editor-tealtint, teal slider bodies.
  assert.deepEqual(ruler.arcs, [{ fill: "#e6eef2", stroke: "#1c2727" }, { fill: "#e6eef2", stroke: "#1c2727" }]);
  assert.ok(ruler.fills.includes("#62d6cfd9"), "slider body");
  assert.ok(ruler.fills.includes("#e6eef2"), "spinner body");
  assert.ok(ruler.fills.includes("#c4c4c4"), "ruler labels in --editor-secondary");
  assert.ok(ruler.strokes.includes("#858585"), "locators in --editor-disabled");
  // Grid lines stay brighter than the design's --editor-grid and --editor-major so beats
  // still outrank the osu!lazer snap ticks between them; the playhead is --editor-focus.
  assert.ok(grid.strokes.includes("#383838"), "beat lines");
  assert.ok(grid.strokes.includes("#545454"), "bar lines");
  for (const canvas of [grid, ruler, overview, volume]) assert.ok(canvas.strokes.includes("#a1d0f7"), "playhead");
  // Object lines take the spec's teal at the old 40% strength (10% vanishes on the rows);
  // the marked range and marquee use the accent.
  assert.ok(grid.strokes.includes("#62d6cf65"), "object lines");
  assert.ok(grid.fills.includes("#77bdfa35"), "marked range");
  assert.ok(grid.fills.includes("#77bdfa14"), "marquee fill");
  assert.ok(grid.strokes.includes("#77bdfa"), "marquee edge");
  // Selected and listen-muted hits keep the volume fill rules but lose the blue tint.
  assert.ok(grid.strokes.includes("#ffffff"), "selected hit outline");
  assert.ok(grid.fills.includes("#858585"), "listen-muted hit");
  assert.ok(volume.fills.includes("#a3a3a3"), "volume labels");
  assert.ok(volume.strokes.includes("#62d6cf"), "ramp preview");
  assert.ok(overview.fills.includes("#a3a3a3"), "overview hit marks");
  assert.ok(overview.fills.includes("#ffffff14"), "overview view box");
  assert.ok(overview.strokes.includes("#858585"), "overview view box edge");

  const oldColours = [
    "#334a63", "#47618b", "#657ab5", "#7aaac6", "#536a89d9", "#9e86c8", "#26364b", "#405a7a",
    "#dddddd", "#a6b0bf", "#4eafbb35", "#69809965", "#637484", "#e5fbff", "#74d3e02f", "#87dbe7",
    "#a5feff", "#72e3ed3b", "#6ebdc7", "#8e8e93", "#aaaaaa", "#eeeeee",
  ];
  for (const canvas of [ruler, grid, overview, volume]) {
    for (const colour of [...canvas.fills, ...canvas.strokes]) {
      assert.ok(!oldColours.includes(colour), `old colour ${colour}`);
    }
  }
});
