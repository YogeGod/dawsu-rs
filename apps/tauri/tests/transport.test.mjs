import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor, readEditorSource } from "./editor-runtime.mjs";

const read = file => readFile(new URL("../dist/" + file, import.meta.url), "utf8");
const [html, css, docking, statusBarSvelte] = await Promise.all([
  read("index.html"), readFile(new URL("../ui/src/legacy-chrome.css", import.meta.url), "utf8"), read("docking.js"),
  readFile(new URL("../ui/src/parts/StatusBar.svelte", import.meta.url), "utf8"),
]);
const app = readEditorSource();

const header = html.match(/<header id="editor-header".*?<\/header>/s)?.[0] ?? "";
const statusbar = html.slice(html.indexOf('<div id="statusbar"'), html.indexOf('<div data-ui-mount="hit-card"'));
const topbarCss = css.slice(css.indexOf("/* Top bar transport (design f1, D2)"));

async function frontend() {
  const controls = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, style: { setProperty() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      append(...children) { this.children.push(...children); }, prepend() {},
      replaceChildren(...children) { this.children = children; },
      setAttribute(name, value) { this[name] = value; },
      classList: { toggle() {}, add() {}, remove() {} },
      getBoundingClientRect: () => ({ x: 0, top: 0, width: 1000 }),
    };
  }
  const get = id => {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  };
  // Every control index.html marks data-map, keyed by its id.
  const mapOnly = [...html.matchAll(/<(?:button|input)\b[^>]*\bdata-map\b[^>]*>/g)].map(([tag]) =>
    get(tag.match(/\bid="([^"]+)"/)[1]));
  const context = createContext({
    document: {
      getElementById: get, createElement: element, addEventListener() {},
      querySelectorAll: selector => selector === "[data-map]" ? mapOnly : [],
    },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(JSON.parse(JSON.stringify(args?.action ?? null)));
        return { busy: false, hasDocument: false };
      } },
      event: { listen() {} },
    } },
    Option: function (text, value) { this.text = text; this.value = value; },
    // A still clock: the playhead is drawn where the editor last put it.
    performance: { now: () => 0 },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await bootEditor(context);
  calls.length = 0;
  const evaluate = code => runInContext(code, context);
  evaluate("renderLanes = () => {};");
  return { calls, evaluate, get, mapOnly };
}

test("the transport sits in the top bar after the menus as the Svelte part, with no L or R chips", () => {
  assert.doesNotMatch(html, /id="editor-transport"/);
  // The map name left the top bar (v3); the file name is in the status bar.
  assert.ok(!header.includes('id="title"'));
  const transport = header.slice(header.indexOf('id="editor-menus"'));
  // The bar and readout are the TransportBar part, mounted inside the kept #top-transport group.
  assert.match(transport, /<div id="top-transport"[^>]*>\s*<div data-ui-mount="transport"><\/div>\s*<\/div>/);
  for (const gone of ['data-action="start"', 'data-action="stop"', 'data-action="play"', 'id="cycle"', 'id="position"',
    'id="transport-bpm"', 'id="transport-meter"', 'id="musical-position"', "data-locator", 'id="locator-left"',
    'id="locator-right"', 'id="play-icon"', 'id="play-label"']) {
    assert.ok(!html.includes(gone), gone);
  }
  assert.ok(header.indexOf('id="save"') > header.indexOf('data-ui-mount="transport"'), "Save stays at the right end");
  assert.doesNotMatch(topbarCss, /:has\(|color-mix\(|aspect-ratio/);
  assert.doesNotMatch(topbarCss, /\.top-transport\s*\{[^}]*overflow:\s*hidden/, "the transport never clips silently");
  assert.doesNotMatch(topbarCss, /button:disabled\s*\{\s*opacity:\s*1\s*\}/);
  for (const dead of [/\.transport-cluster/, /\.transport-cycle/, /\.transport-display/, /\.transport-readout/,
    /\.transport-locators/, /#musical-position/, /#position\b/, /\.transport-bar\b/]) {
    assert.doesNotMatch(css, dead);
  }
  assert.match(css, /#editor-header\[data-empty="true"\] \.top-transport\{opacity:\.35\}/, "the part fades with no map");
});

test("the old bottom transport bar leaves no CSS behind", () => {
  for (const dead of [/#editor-transport/, /\.transport\s*\{/, /\.transport\.chrome/, /\.transport-flags/, /\.transport-navigation/,
    /\.transport label/, /\.transport input/, /\.transport \.transport-display/]) {
    assert.doesNotMatch(css, dead);
  }
});

test("the status bar's right end keeps its size so zoom and Audio stay on screen", () => {
  assert.match(css, /\.status-right\s*\{[^}]*flex:\s*none/);
});

test("speeds, Fit, zoom and Audio live in the StatusBar Svelte part", () => {
  assert.match(statusbar, /<div id="statusbar" data-ui-mount="status-bar">/);
  for (const id of ["fit", "zoom", "audio"]) assert.match(statusBarSvelte, new RegExp('id="' + id + '"'), id);
  assert.match(statusBarSvelte, /<input[^>]*min=\{ZOOM_MIN\}[^>]*id="zoom"/);
  assert.ok(statusBarSvelte.indexOf('id="audio"') > statusBarSvelte.indexOf('id="hit-lane-counts"'));
});

test("the layout editor no longer places the transport above or below the timeline", () => {
  assert.doesNotMatch(html, /id="layout-transport"/);
  assert.doesNotMatch(html, /<option value="transport">/);
  for (const dead of ["editor-transport", "placeTransport", "transportPicker", "data-transport-top", '"Transport"']) {
    assert.ok(!docking.includes(dead), dead);
  }
  assert.match(docking, /const defaultFrame = \[header, ribbon, root, status\];/);
});

test("show() leaves the transport, locators and status bar to the Svelte parts and still fades the header with no map", async () => {
  const { evaluate, get } = await frontend();
  const transportIds = ["position", "transport-bpm", "transport-meter", "musical-position", "locator-left",
    "locator-right", "cycle", "play", "play-icon", "play-label", "stop"];
  for (const id of transportIds) assert.ok(!app.includes('$("' + id + '")'), id);
  evaluate(`show({ busy: false, hasDocument: true, position: 106052, left: 104947, right: 107473, playing: true, cycle: true,
    musicalPosition: "38.2.240 · 190.0 BPM · 7/8 · 1/4",
    scene: { path: "Hard.osu", lanes: [], difficulties: [], history: [], historyPosition: 0, moments: [] } })`);
  assert.equal(get("editor-header").dataset.empty, "false");

  evaluate("show({ busy: false, hasDocument: false, position: 5000, left: 0, right: 10000, musicalPosition: null })");
  assert.equal(get("editor-header").dataset.empty, "true");
});
