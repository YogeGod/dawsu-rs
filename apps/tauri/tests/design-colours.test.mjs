import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const css = await readFile(new URL("../ui/src/legacy-chrome.css", import.meta.url), "utf8");

// Apple Logic Pro for iPad pure black & neutral obsidian surfaces (--dark-* palette, --editor-* roles).
const surfaces = {
  "--base": "#0e0e10", "--field": "#0e0e10", "--lane-alt": "#0e0e10",
  "--panel": "#151517", "--lane": "#151517",
  "--chrome": "#1c1c1f", "--raised": "#1c1c1f", "--group": "#1c1c1f",
  "--ruler": "#1a1a1d", "--button": "#242428", "--control": "#303036",
  "--line": "#2b2b30", "--divider": "#2b2b30", "--button-border": "#2b2b30",
  "--text": "#f5f5f7", "--title-text": "#c7c7cc", "--muted": "#98989f",
  "--hi": "#5ac8fa", "--hi-hover": "#8ee0ff", "--on-accent": "#08080a",
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
  assert.equal(root[lastDeclared("header.chrome", "background")], "#1c1c1f");
  assert.equal(root[lastDeclared(".tools.chrome", "background")], "#151517");
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
    const strokeRects = [];
    const arcs = [];
    const state = {};
    const ctx = new Proxy(state, {
      get(target, name) {
        if (name in target) return target[name];
        if (["fillRect", "fill", "fillText"].includes(name)) return () => fills.push(target.fillStyle);
        if (name === "strokeRect") return (...args) => { strokes.push(target.strokeStyle); strokeRects.push(args); };
        if (name === "stroke") return () => strokes.push(target.strokeStyle);
        if (name === "arc") return () => arcs.push({ fill: target.fillStyle, stroke: target.strokeStyle });
        return () => {};
      },
    });
    return {
      fills, strokes, strokeRects, arcs, getContext: () => ctx, clientWidth: 300, clientHeight: 80, scrollTop: 0,
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
  return {
    ruler: get("ruler"), grid: get("grid"), overview: get("overview"), volume: get("controller"),
    evaluate: code => runInContext(code, context),
  };
}

test("timeline canvases paint with the design's colours", async () => {
  const { ruler, grid, overview, volume } = await paintedCanvases();
  // Song waveform: osu!lazer blue/cyan trapezoid fills on --editor-tealtint.
  assert.equal(ruler.fills[0], "#121718");
  const isLazerWave = f => /^rgba\((?:[0-9]{1,2}|1\d\d|2[0-3]\d),\d+,(?:1[89]\d|2\d\d),\d+\.\d{3}\)$/.test(f);
  assert.ok(ruler.fills.some(isLazerWave), "ruler osu!lazer waveform fill");
  assert.ok(!overview.fills.some(isLazerWave), "no overview waveform");
  // Gameplay objects without combo data: --editor-readout circles with a white ring (timeline-spec §9);
  // slider tracks are that colour 38 % into #111111 with an 85 % white edge.
  assert.deepEqual(ruler.arcs, [{ fill: "#e6eef2", stroke: "#ffffff" }, { fill: "#e6eef2", stroke: "#ffffff" }]);
  assert.ok(ruler.fills.includes("#626567"), "slider body");
  assert.ok(ruler.strokes.includes("#ffffffd9"), "slider edge");
  assert.ok(ruler.fills.includes("#e6eef2"), "spinner body");
  assert.ok(ruler.fills.includes("#f5f5f7"), "ruler bar numbers crisp white (v3 §5)");
  assert.ok(ruler.strokes.includes("#8e8e96"), "locators in neutral grey");
  // v3 §5 (Yoge): grid lines stay neutral white at low alpha; snap colours show only on the ruler.
  // The playhead is crisp white (Logic Pro iPad).
  assert.ok(grid.strokes.includes("rgba(255,255,255,.07)"), "beat lines");
  assert.ok(grid.strokes.includes("rgba(255,255,255,.26)"), "bar lines");
  assert.ok(!grid.strokes.some(colour => /^#(ed1121|66ccff|8866ee)/.test(colour)), "no snap colours on the grid");
  for (const canvas of [grid, ruler, overview, volume]) assert.ok(canvas.strokes.includes("#f5f5f7"), "playhead");
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

test("canvas grid lines honour slateTimeline overrides (#103) and marquee box anchors to timestamps (#106)", async () => {
  const { grid, evaluate } = await paintedCanvases();
  grid.strokes.length = 0;
  grid.strokeRects.length = 0;
  evaluate(`
    slateTimeline.gridBar = "#ff3344";
    slateTimeline.gridBeat = "#cc2233";
    slateTimeline.gridSub = "#991122";
    scene.viewStart = 50;
    drag = {
      surface: grid,
      marquee: true,
      firstPoint: { x: 25, y: 5, ms: 100 },
      lastPoint: { x: 75, y: 40, ms: 300 },
    };
    paint();
  `);
  assert.ok(grid.strokes.includes("#ff3344"), "custom bar line drawn on grid");
  assert.ok(grid.strokes.includes("#cc2233"), "custom beat line drawn on grid");
  assert.ok(!grid.strokes.includes("rgba(255,255,255,.26)"), "default bar line replaced");
  // At viewStart=50, zoom=0.25: 100ms -> x=12.5, 300ms -> x=62.5 (width=50).
  assert.deepEqual(grid.strokeRects.at(-1), [12.5, 5, 50, 35]);
});

// Slice 11.3: a fixed dark surface must not take an inherited, themed foreground. A bright custom
// scheme turns --editor-text dark, so text on a literal dark background (or a fixed-dark token)
// would vanish. Each such rule sets a fixed light `color` (var(--ink-on-dark) or a literal light
// colour), or is listed here because it paints no text of its own.
const backdrop = "modal backdrop; its text sits in the themed card above it";
const DARK_SURFACE_ALLOW = new Map(Object.entries({
  "src/legacy-chrome.css :where(body.editor-page) dialog::backdrop": backdrop,
  "src/legacy-chrome.css input[type=checkbox]:checked::before": "switch knob, no text",
  "src/parts/CommandPalette.svelte .cmdp-backdrop": backdrop,
  "src/parts/DrumMapEditorModal.svelte .dme-backdrop": backdrop,
  "src/parts/ExportModal.svelte .export-backdrop": backdrop,
  "src/parts/KeysoundBounceModal.svelte .kb-backdrop": backdrop,
  "src/parts/MapsetIntakeCard.svelte .intake-backdrop": backdrop,
  "src/parts/MidiImportModal.svelte .midi-backdrop": backdrop,
  "src/parts/node-graph.css .ng-checklist-overlay": backdrop,
  "src/parts/preferences/preferences.css .pref-backdrop": backdrop,
  "src/parts/SharedBaseModal.svelte .sb-backdrop": backdrop,
  "src/parts/StemSeparationModal.svelte .ss-backdrop": backdrop,
  "src/parts/Vst3BounceModal.svelte .vb-backdrop": backdrop,
  "src/parts/Fader.svelte .track": "fader track line, no text",
  "src/parts/Fader.svelte .meter": "level meter well, no text",
  "src/parts/keysounds/KeysoundRollSurface.svelte .ks-keys": "key column; each .ks-key sets its own colour",
  "src/parts/PianoKeyboard.svelte .pk-col": "key column; each .pk-key sets its own colour",
  "src/parts/TransportBar.svelte .bar": "segment frame; each .seg sets its own colour on the themed --transport-seg",
  "src/parts/MapPreview.svelte .preview-frame": "canvas frame, no text",
  "src/parts/PreviewCanvas.svelte .frame canvas": "canvas, no text",
  "src/parts/SpectrumPanel.svelte .spectrum-canvas": "canvas, no text",
  "src/parts/SharedBaseModal.svelte .sb-curve-svg": "curve plot svg, no text",
  "src/parts/OszBudgetGauge.svelte .osz-track": "gauge track, no text",
  "src/parts/SnapSlider.svelte .between .thumb": "slider thumb, no text",
  "src/parts/MapsetIntakeCard.svelte .intake-hero-scrim": "image scrim, no text",
  "src/parts/StartScreen.svelte .home-song-scrim": "image scrim, no text",
  ...Object.fromEntries([0, 1, 2, 3, 4, 5].map(n =>
    [`src/parts/StartScreen.svelte .home-art-${n}`, "aria-hidden cover art backdrop, no text"])),
}));

async function styleSources() {
  const { readdir } = await import("node:fs/promises");
  const uiRoot = new URL("../ui/", import.meta.url);
  const out = [];
  async function walk(dir, rel) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      if (["node_modules", "dist", "tests"].includes(entry.name)) continue;
      const path = rel + entry.name;
      if (entry.isDirectory()) await walk(new URL(entry.name + "/", dir), path + "/");
      else if (/\.(css|svelte|html)$/.test(entry.name)) {
        let text = await readFile(new URL(entry.name, dir), "utf8");
        if (!entry.name.endsWith(".css")) text = [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map(m => m[1]).join("\n");
        out.push([path, text.replace(/\/\*[\s\S]*?\*\//g, "")]);
      }
    }
  }
  await walk(uiRoot, "");
  return out;
}

async function themedProperties() {
  const { applyColorConfig } = await import("../ui/src/lib/theme.js");
  const names = new Set();
  applyColorConfig({ scheme: "obsidian", rows: {} },
    { documentElement: { style: { setProperty: name => names.add(name) } } });
  return names;
}

// A literal colour as { hex, alpha }, or null.
function literalColour(value) {
  const hex = value.match(/^#([0-9a-f]{3,8})$/i)?.[1];
  if (hex) {
    const full = hex.length <= 4 ? [...hex].map(c => c + c).join("") : hex;
    return { hex: `#${full.slice(0, 6)}`, alpha: full.length === 8 ? parseInt(full.slice(6), 16) / 255 : 1 };
  }
  const rgb = value.match(/^rgba?\(([^)]*)\)$/i)?.[1].split(/[\s,/]+/).filter(Boolean).map(Number);
  if (rgb?.length >= 3) {
    return { hex: "#" + rgb.slice(0, 3).map(c => Math.round(c).toString(16).padStart(2, "0")).join(""), alpha: rgb[3] ?? 1 };
  }
  return null;
}

async function themeTokenContext() {
  const tokens = await readFile(new URL("../ui/src/tokens.css", import.meta.url), "utf8");
  const themed = await themedProperties();
  const defs = {};
  for (const source of [tokens, css]) {
    for (const [, block] of source.matchAll(/:root\s*\{([^}]*)\}/g)) {
      for (const [, name, value] of block.matchAll(/(--[\w-]+):\s*([^;]+);/g)) defs[name] ??= value.trim();
    }
  }
  // A token's fixed literal value, or null when any step of its chain is themed by applyColorConfig.
  const resolve = (value, depth = 0) => {
    const name = value.match(/^var\((--[\w-]+)\)$/)?.[1];
    if (!name) return literalColour(value);
    if (themed.has(name) || !defs[name] || depth > 8) return null;
    return resolve(defs[name], depth + 1);
  };
  const dropFallbacks = text => text.replace(/var\((--[\w-]+)\s*,(?:[^()]|\([^()]*\))*\)/g, "var($1)");
  const rules = [];
  for (const [file, text] of await styleSources()) {
    for (const [, rawSelector, rawBody] of text.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      rules.push({
        file,
        selector: rawSelector.trim().replace(/\s+/g, " "),
        body: dropFallbacks(rawBody),
      });
    }
  }
  const propOf = (body, property) =>
    [...body.matchAll(new RegExp(`(?:^|[;\\s])${property}:\\s*([^;]+)`, "g"))].at(-1)?.[1].trim() ?? null;
  return { tokens, themed, defs, resolve, rules, propOf };
}

test("text on a fixed dark surface takes a fixed light ink, not a themed one (slice 11.3)", async () => {
  const { tokens, resolve, rules, propOf } = await themeTokenContext();
  assert.match(tokens, /--ink-on-dark:\s*#f5f5f7;/);
  assert.match(tokens, /--readout-value:\s*var\(--ink-on-dark\);/);

  const darkLimit = luminance("#303030");
  const fixedDarkToken = /^var\(--(?:editor-ink|transport-rec-on|dark-[\w-]+)\)$/;

  const flagged = [];
  const failures = [];
  for (const { file, selector, body } of rules) {
    const surfaces = [...body.matchAll(/(?:^|[;\s])background(?:-color)?:\s*([^;]+)/g)]
      .flatMap(([, value]) => [...value.matchAll(/#[0-9a-f]{3,8}\b|rgba?\([^)]*\)|var\(--[\w-]+\)/gi)].map(m => m[0]))
      .map(colour => colour.startsWith("var(") ? (fixedDarkToken.test(colour) ? resolve(colour) : null) : literalColour(colour))
      // Half-transparent or fainter tints sit over a themed surface; they are not surfaces.
      .filter(colour => colour && colour.alpha > .5 && luminance(colour.hex) < darkLimit);
    if (!surfaces.length) continue;
    const key = `${file} ${selector}`;
    flagged.push(key);
    if (DARK_SURFACE_ALLOW.has(key)) continue;
    const colourValue = propOf(body, "color");
    const ink = colourValue && resolve(colourValue);
    const readable = ink && surfaces.every(surface => contrast(ink.hex, surface.hex) >= 4.5);
    if (!readable) failures.push(`${key} { color: ${colourValue ?? "(inherited)"} }`);
  }
  for (const key of ["src/parts/TransportBar.svelte .readout", "src/parts/browser.css .browser-search"]) {
    assert.ok(flagged.includes(key), `scan finds ${key}`);
  }
  for (const key of DARK_SURFACE_ALLOW.keys()) assert.ok(flagged.includes(key), `allow-list entry still matches: ${key}`);
  assert.deepEqual(failures, []);
});

test("dark-well children and hovers keep readable inks under bright schemes (slice 11.3)", async () => {
  const { resolve, rules, propOf } = await themeTokenContext();
  const findRule = (file, selector, property = "color") => {
    const matches = rules.filter(r => r.file === file && r.selector === selector);
    assert.ok(matches.length > 0, `missing rule: ${file} ${selector}`);
    const withProp = property ? matches.filter(r => propOf(r.body, property) !== null) : matches;
    return withProp.at(-1) ?? matches.at(-1);
  };
  const assertFixedInk = (file, selector, surfaceHex) => {
    const color = propOf(findRule(file, selector).body, "color");
    assert.ok(color, `${file} ${selector} declares color`);
    const ink = resolve(color);
    assert.ok(ink, `${file} ${selector} color ${color} must resolve to a fixed literal, not a themed token`);
    const ratio = contrast(ink.hex, surfaceHex);
    assert.ok(ratio >= 4.5, `${file} ${selector} ${ink.hex} on ${surfaceHex} is ${ratio.toFixed(2)}:1`);
  };
  const assertThemedColor = (file, selector) => {
    const color = propOf(findRule(file, selector).body, "color");
    assert.equal(color, "var(--editor-text)", `${file} ${selector} uses themed --editor-text`);
  };

  assertFixedInk("src/parts/pill.css", ".ui-pill", "#000000");
  assertThemedColor("src/parts/pill.css", ".ui-pill:hover:not(:disabled)");

  assertFixedInk("src/parts/PlayheadGroup.svelte", ".label", "#000000");
  assertFixedInk("src/parts/PlayheadGroup.svelte", ".toggle", "#000000");
  assertThemedColor("src/parts/PlayheadGroup.svelte", ".toggle:hover:not(:disabled)");

  for (const surface of ["#1e1e21", "#151517"]) {
    assertFixedInk("src/parts/ToastStack.svelte", ".toast-body", surface);
    assertFixedInk("src/parts/ToastStack.svelte", ".toast-dismiss", surface);
  }

  assertFixedInk("src/parts/browser.css", ".browser-search input", "#000000");
  assertFixedInk("src/parts/browser.css", ".browser-search input::placeholder", "#000000");

  assertFixedInk("src/parts/export/ExportOptionsForm.svelte", "select.field", "#000000");

  assertFixedInk("src/parts/inspector.css", "#editor-inspector .insp-pill-bar .insp-pill, .insp-pill", "#000000");
  assertFixedInk("src/parts/inspector.css", "#editor-inspector .insp-pill-bar .insp-pill.insp-pill-mixed, .insp-pill-mixed", "#000000");

  assertFixedInk("src/parts/SnapSlider.svelte", ".label", "#000000");
  assertFixedInk("src/parts/SnapSlider.svelte", ".mode-pill.objects, .mode-pill.objects:hover:not(:disabled)", "#000000");

  const selectionHover = findRule("src/parts/TransportBar.svelte", ".selection:hover:not(:disabled)").body;
  const selectionHoverBg = resolve(propOf(selectionHover, "background"));
  assert.ok(selectionHoverBg, "TransportBar .selection:hover background must resolve to a fixed dark surface");
  assertFixedInk("src/parts/TransportBar.svelte", ".selection:hover:not(:disabled)", selectionHoverBg.hex);
  assertThemedColor("src/parts/TransportBar.svelte", ".tab:hover:not(:disabled)");

  const chipHover = findRule("src/shell.css", "#editor-header .map-chip:hover").body;
  const chipHoverBg = resolve(propOf(chipHover, "background"));
  assert.ok(chipHoverBg, "shell.css #editor-header .map-chip:hover background must resolve to a fixed dark surface");
  assertFixedInk("src/shell.css", "#editor-header .map-chip:hover", chipHoverBg.hex);
  assertFixedInk("src/shell.css", ".map-chip-chevron", "#000000");

  assertThemedColor("src/parts/StartScreen.svelte", ":global(#home .home-recent-row .home-recent-map)");
  assertThemedColor("src/parts/StartScreen.svelte", ":global(#home .home-recent-row .home-recent-map:hover:not(:disabled))");

  assertFixedInk("src/timeline.css", ".ruler-sidebar .song-row", "#121718");
  assertFixedInk("src/timeline.css", ".song-title", "#121718");
  assertFixedInk("src/timeline.css", ".ruler-sidebar .song-row button", "#121718");
  assertFixedInk("src/timeline.css", ".ruler-sidebar .song-row button:hover:not(:disabled)", "#121718");

  assertFixedInk("src/parts/export/OsuDiffDrawer.svelte", ".osu-diff-more", "#000000");
  assertFixedInk("src/parts/PreviewWindow.svelte", ":global(body.preview-page .insp-summary)", "#0b0b0f");

test("the header's map chip keeps the difficulty whole and lets the title take the ellipsis", async () => {
  // Both shrank together, so "Echoes of Memoria [Hitsounds]" showed as "Echoes of ... Hitso...".
  const shell = await readFile(new URL("../ui/src/shell.css", import.meta.url), "utf8");
  assert.match(shell, /\.map-chip-version\s*\{[^}]*flex:\s*none;/);
  assert.match(shell, /\.map-chip-title\s*\{[^}]*flex:\s*1 1 auto;[^}]*min-width:\s*0;/);
});
