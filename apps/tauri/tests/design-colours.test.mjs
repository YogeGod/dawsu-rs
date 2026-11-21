import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const css = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");

// Logic Pro for iPad cool slate surfaces (--dark-* palette, --editor-* roles).
const surfaces = {
  "--base": "#0f1218", "--field": "#0f1218", "--lane-alt": "#0f1218",
  "--panel": "#161a23", "--lane": "#161a23",
  "--chrome": "#1f2430", "--raised": "#1f2430", "--group": "#1f2430",
  "--ruler": "#1b202b", "--button": "#252c3b", "--control": "#2f384b",
  "--line": "#2c3445", "--divider": "#2c3445", "--button-border": "#2c3445",
  "--text": "#f2f5fa", "--title-text": "#c4cddb", "--muted": "#96a3b8",
  "--hi": "#4fa8ff", "--hi-hover": "#84c5ff", "--on-accent": "#08111c",
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

// WCAG 2 relative luminance and contrast ratio.
function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map(c => c <= .03928 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4);
  return .2126 * r + .7152 * g + .0722 * b;
}
function contrast(first, second) {
  const [light, dark] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (light + .05) / (dark + .05);
}
const root = Object.fromEntries([...css.matchAll(/:root\s*\{([^}]*)\}/g)]
  .flatMap(match => [...match[1].matchAll(/(--[a-z-]+):\s*(#[0-9a-f]{6})\b/gi)].map(([, name, value]) => [name, value])));
// The value a property takes in the last rule for this exact selector that sets it.
function lastDeclared(selector, property) {
  const escaped = selector.replace(/[.#[\]"=]/g, "\\$&");
  const rules = [...css.matchAll(new RegExp(`(?:^|[}\\s])${escaped}\\s*\\{([^}]*)\\}`, "g"))];
  const values = rules.flatMap(([, body]) =>
    [...body.matchAll(new RegExp(`(?:^|[;{\\s])${property}:\\s*var\\((--[a-z-]+)\\)`, "g"))].map(match => match[1]));
  return values.at(-1);
}

test("the top bar stays chrome while the ribbon sits on the darker panel", () => {
  assert.equal(root[lastDeclared("header.chrome", "background")], "#1f2430");
  assert.equal(root[lastDeclared(".tools.chrome", "background")], "#161a23");
});

test("muted and secondary text reads at 4.5:1 or better on every bar", () => {
  const status = lastDeclared("#statusbar", "color");
  const bars = {
    "header.chrome": ["--muted", "--title-text"],
    ".tools.chrome": ["--muted", "--title-text"],
    // The status bar's text takes its own colour; nothing muted sits on it.
    "#statusbar": [status, "--title-text"],
  };
  for (const [bar, texts] of Object.entries(bars)) {
    const background = root[lastDeclared(bar, "background")];
    for (const text of texts) {
      const ratio = contrast(root[text], background);
      assert.ok(ratio >= 4.5, `${text} on ${bar} is ${ratio.toFixed(2)}:1`);
    }
  }
});

test("top-bar buttons sit flat on the bar, with the menus as one panel group (v3 2.3)", async () => {
  // v3 drops the edged buttons: the menus are one --editor-panel group, Export a hover fill.
  const edges = [...css.matchAll(/([^{}]*)\{[^}]*border-color:\s*(#[0-9a-f]{6})[^}]*\}/gi)]
    .filter(([, selector]) => /header[^,{]*#editor-menus button/.test(selector));
  assert.equal(edges.length, 0);
  const shell = await readFile(new URL("../ui/src/shell.css", import.meta.url), "utf8");
  assert.match(shell, /#editor-header #editor-menus \{[^}]*background: var\(--editor-panel\)/);
  assert.match(shell, /#editor-header #editor-menus button \{[^}]*border: 0/);
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
  bootEditor(context, { autoStart: false });
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
  // Song waveform: osu!lazer blue/cyan trapezoid fills on --editor-tealtint.
  assert.equal(ruler.fills[0], "#132229");
  const isLazerWave = f => /^rgba\((?:[0-9]{1,2}|1\d\d|2[0-3]\d),\d+,(?:1[89]\d|2\d\d),\d+\.\d{3}\)$/.test(f);
  assert.ok(ruler.fills.some(isLazerWave), "ruler osu!lazer waveform fill");
  assert.ok(!overview.fills.some(isLazerWave), "no overview waveform");
  // Gameplay objects without combo data: --editor-readout circles with a white ring (timeline-spec §9);
  // slider tracks are that colour 38 % into #111111 with an 85 % white edge.
  assert.deepEqual(ruler.arcs, [{ fill: "#e6eef2", stroke: "#ffffff" }, { fill: "#e6eef2", stroke: "#ffffff" }]);
  assert.ok(ruler.fills.includes("#626567"), "slider body");
  assert.ok(ruler.strokes.includes("#ffffffd9"), "slider edge");
  assert.ok(ruler.fills.includes("#e6eef2"), "spinner body");
  assert.ok(ruler.fills.includes("#f2f5fa"), "ruler bar numbers crisp white (v3 §5)");
  assert.ok(ruler.strokes.includes("#7b8ba3"), "locators in cool slate");
  // v3 §5 (Yoge): grid lines stay neutral white at low alpha; snap colours show only on the ruler.
  // The playhead is --editor-focus.
  assert.ok(grid.strokes.includes("rgba(255,255,255,.045)"), "beat lines");
  assert.ok(grid.strokes.includes("rgba(255,255,255,.22)"), "bar lines");
  assert.ok(!grid.strokes.some(colour => /^#(ed1121|66ccff|8866ee)/.test(colour)), "no snap colours on the grid");
  for (const canvas of [grid, ruler, overview, volume]) assert.ok(canvas.strokes.includes("#84c5ff"), "playhead");
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
  // timeline-spec "Overview strip": object diamonds in #c4c4c4, the viewport box in --editor-text.
  assert.ok(overview.fills.some(fill => fill.startsWith("rgba(196,196,196,")), "overview object diamonds");
  assert.ok(overview.fills.includes("rgba(241,241,241,.06)"), "overview view box");
  assert.ok(overview.strokes.includes("#f1f1f1"), "overview view box edge");

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
