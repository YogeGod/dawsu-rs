import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const read = file => readFile(new URL("../dist/" + file, import.meta.url), "utf8");
const [html, css, app, docking] = await Promise.all(["index.html", "style.css", "app.js", "docking.js"].map(read));

const header = html.match(/<header id="editor-header".*?<\/header>/s)?.[0] ?? "";
const statusbar = html.slice(html.indexOf('<div id="statusbar"'), html.indexOf('<dialog id="dialog"'));
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
  const mapOnly = ["play", "stop", "cycle", "zoom"].map(get);
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
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await runInContext(app, context);
  calls.length = 0;
  const evaluate = code => runInContext(code, context);
  evaluate("renderLanes = () => {};");
  return { calls, evaluate, get, mapOnly };
}

test("the transport sits in the top bar after the map name and the bottom bar is gone", () => {
  assert.doesNotMatch(html, /id="editor-transport"/);
  assert.ok(header.indexOf('id="title"') >= 0);
  const transport = header.slice(header.indexOf('id="title"'));
  for (const control of ['data-action="start"', 'data-action="stop"', 'data-action="play"', 'id="cycle"',
    'id="position"', 'id="transport-bpm"', 'id="transport-meter"', 'id="musical-position"',
    'data-locator="left"', 'data-locator="right"', 'id="locator-left"', 'id="locator-right"']) {
    assert.ok(transport.includes(control), control);
  }
  for (const control of ['data-action="play"', 'data-action="stop"', 'id="cycle"', 'data-locator="left"', 'data-locator="right"']) {
    assert.match(header, new RegExp("<[^>]*" + control + "[^>]*data-map"), control + " waits for a map");
  }
  assert.ok(header.indexOf('data-action="review_save"') > header.indexOf('id="locator-right"'), "Save stays at the right end");
  assert.match(topbarCss, /#position[^{]*\{[^}]*Roboto Mono/);
  assert.doesNotMatch(topbarCss, /:has\(|color-mix\(|aspect-ratio/);
});

test("speeds, Fit, zoom and Audio live in the status bar", () => {
  for (const id of ["fit", "zoom", "audio"]) assert.match(statusbar, new RegExp('id="' + id + '"'), id);
  assert.match(statusbar, /<input[^>]*min="[^"]+"[^>]*id="zoom"[^>]*data-map/);
  assert.ok(statusbar.indexOf('id="audio"') > statusbar.indexOf('id="hit-lane-counts"'));
});

test("the layout editor no longer places the transport above or below the timeline", () => {
  assert.doesNotMatch(html, /id="layout-transport"/);
  assert.doesNotMatch(html, /<option value="transport">/);
  for (const dead of ["editor-transport", "placeTransport", "transportPicker", "data-transport-top", '"Transport"']) {
    assert.ok(!docking.includes(dead), dead);
  }
  assert.match(docking, /const defaultFrame = \[header, ribbon, root, status\];/);
});

test("show() fills the time, BPM, time signature, bar and locators, and fades them with no map", async () => {
  const { evaluate, get, mapOnly } = await frontend();
  evaluate(`show({ busy: false, hasDocument: true, position: 106052, left: 104947, right: 107473,
    musicalPosition: "38.2.240 · 190.0 BPM · 7/8 · 1/4",
    scene: { path: "Hard.osu", lanes: [], difficulties: [], history: [], historyPosition: 0, moments: [] } })`);
  assert.equal(get("position").textContent, "01:46.052");
  assert.equal(get("transport-bpm").textContent, "190.0");
  assert.equal(get("transport-meter").textContent, "7/8");
  assert.equal(get("musical-position").textContent, "38.2.240");
  assert.equal(get("locator-left").textContent, "01:44.947");
  assert.equal(get("locator-right").textContent, "01:47.473");
  assert.equal(get("editor-header").dataset.empty, "false");
  assert.equal(get("status-view").hidden, false);
  assert.ok(mapOnly.every(control => control.disabled === false));

  evaluate("show({ busy: false, hasDocument: false, position: 5000, left: 0, right: 10000, musicalPosition: null })");
  assert.equal(get("position").textContent, "00:00.000");
  assert.equal(get("transport-bpm").textContent, "–");
  assert.equal(get("transport-meter").textContent, "–");
  assert.equal(get("musical-position").textContent, "–");
  assert.equal(get("editor-header").dataset.empty, "true");
  assert.equal(get("title")["data-placeholder"], "No map open");
  assert.equal(get("status-view").hidden, true);
  assert.ok(mapOnly.every(control => control.disabled === true));
});
