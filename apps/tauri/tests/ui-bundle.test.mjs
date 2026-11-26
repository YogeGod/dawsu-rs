import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

// Checks on the Svelte parts that need no node_modules: the committed bundle in dist/ui/ is
// what the exe ships, so these run with the rest of the frontend suite.
const tauri = new URL("../", import.meta.url);
const uiSource = new URL("ui/src/", tauri);
const bundle = new URL("dist/ui/", tauri);
const read = url => readFileSync(url, "utf8");

function sources(dir = uiSource) {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => entry.isDirectory()
    ? sources(new URL(entry.name + "/", dir))
    : [{ name: fileURLToPath(new URL(entry.name, dir)), text: read(new URL(entry.name, dir)) }]);
}

const definedProperties = css => new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map(match => match[1]));

test("parts use none of the selectors the editor runtime wires and no static style attribute", () => {
  // editor/boot.js adds listeners to these by selector at load, so a Svelte part using one would act twice.
  const shared = /data-(?:action|command|volume|locator|map|doc|playback-speed)\b|editor-tool-button/;
  const partsDir = fileURLToPath(new URL("parts/", uiSource));
  for (const { name, text } of sources()) {
    if (name.startsWith(partsDir)) assert.doesNotMatch(text, shared, name);
    // The window's policy blocks style="" attributes; parts set custom properties or classes.
    assert.doesNotMatch(text, /\sstyle\s*=/, name);
    assert.doesNotMatch(text, /url\(/, name + " (fonts and images stay in style.css)");
  }
});

test("v3 tokens never redefine a custom property legacy-chrome.css already sets", () => {
  const legacyPath = fileURLToPath(new URL("ui/src/legacy-chrome.css", tauri));
  const old = definedProperties(read(new URL("ui/src/legacy-chrome.css", tauri)));
  for (const { name, text } of sources().filter(file => /\.(css|svelte)$/.test(file.name) && file.name !== legacyPath)) {
    assert.deepEqual([...definedProperties(text)].filter(property => old.has(property)), [], name);
  }
});

test("the bundle keeps to the editor's content security policy", () => {
  assert.deepEqual(readdirSync(bundle).sort(), ["build-stamp.json", "ui.css", "ui.js"]);
  for (const file of ["ui.js", "ui.css"]) {
    const text = read(new URL(file, bundle));
    for (const blocked of ["data:", "process.env", "url(", "eval(", "new Function", ' style="']) {
      assert.ok(!text.includes(blocked), file + " contains " + blocked);
    }
  }
  // The target includes Safari 15.0 and older WebKitGTK, which lack these CSS features.
  const styles = read(new URL("ui.css", bundle));
  for (const missing of [":has(", "color-mix(", "aspect-ratio", "oklch("]) assert.ok(!styles.includes(missing), "ui.css uses " + missing);
  const script = read(new URL("ui.js", bundle));
  // The target includes Safari 15.0 and older WebKitGTK, which lack these (Safari 15.4 added them).
  for (const missing of ["findLast", ".at("]) assert.ok(!script.includes(missing), "ui.js uses " + missing);
  assert.match(script, /^(?:\/\*[^]*?\*\/\s*)?var dawsuUi\s*=/, "ui.js is a classic script that sets window.dawsuUi");
  assert.doesNotMatch(script, /^\s*(?:import|export)\b/m);
});

test("the committed bundle matches its sources", async () => {
  const { buildStamp } = await import(new URL("ui/stamp.mjs", tauri));
  const saved = JSON.parse(read(new URL("build-stamp.json", bundle)));
  const fresh = buildStamp(fileURLToPath(new URL("ui/", tauri)), fileURLToPath(bundle));
  assert.equal(saved.inputs, fresh.inputs, "ui sources changed: run npm run build in apps/tauri/ui");
  assert.deepEqual(saved.outputs, fresh.outputs, "dist/ui was edited by hand or built from other sources");
});

test("index.html loads stylesheets and scripts in the expected order", () => {
  const head = read(new URL("dist/index.html", tauri)).match(/<head>(.*?)<\/head>/s)[1];
  const order = [...head.matchAll(/(?:href|src)="([^"]+\.(?:css|js))"/g)].map(match => match[1]);
  assert.deepEqual(order, ["fonts.css", "ui/ui.css", "docking.js", "ui/ui.js"]);
  assert.match(head, /<script defer src="ui\/ui\.js"><\/script>/);
});

test("dist/ui/ui.js boots the editor with menus.js on index.html and stays idle on secondary windows", async () => {
  const uiBundle = read(new URL("ui.js", bundle));
  const makeNode = () => ({
    dataset: {}, style: { setProperty() {} }, children: [], attributes: {},
    classList: { add() {}, remove() {}, toggle() {}, contains: () => false },
    addEventListener() {}, removeEventListener() {}, append() {}, prepend() {},
    replaceChildren() {}, setAttribute(k, v) { this.attributes[k] = String(v); },
    getAttribute(k) { return this.attributes[k] ?? null; }, removeAttribute(k) { delete this.attributes[k]; },
    contains: () => false, closest: () => null, querySelector: () => null, querySelectorAll: () => [],
    getBoundingClientRect: () => ({ x: 0, y: 0, left: 0, top: 0, right: 0, bottom: 0, width: 100, height: 40 }),
  });

  // Secondary window (export.html / drum.html): no #grid -> editor never boots, never invokes editor_view.
  const secondaryCalls = [];
  const secondaryContext = createContext({
    document: { getElementById: () => null, createElement: makeNode, querySelectorAll: () => [], addEventListener() {} },
    window: {
      __TAURI__: { core: { invoke: async (cmd) => { secondaryCalls.push(cmd); return {}; } }, event: { listen() {} } },
      addEventListener() {},
    },
    ResizeObserver: class { observe() {} }, requestAnimationFrame() {}, setInterval() {}, queueMicrotask, console,
  });
  secondaryContext.window.window = secondaryContext.window;
  secondaryContext.globalThis = secondaryContext;
  runInContext(uiBundle, secondaryContext);
  assert.deepEqual(secondaryCalls, [], "secondary windows must never boot the main editor");
  assert.equal(typeof secondaryContext.dawsuUi?.commands, "object");

  // Main editor window (index.html): #grid present -> ui.js boots editor, exposes globals, menus.js attaches cleanly.
  const mainCalls = [];
  const mainNodes = new Map();
  const getMain = id => {
    if (!mainNodes.has(id)) mainNodes.set(id, makeNode());
    return mainNodes.get(id);
  };
  const mainContext = createContext({
    document: { getElementById: getMain, createElement: makeNode, querySelectorAll: () => [], addEventListener() {} },
    window: {
      __TAURI__: {
        core: { invoke: async (cmd) => { mainCalls.push(cmd); return { busy: false, hasDocument: false }; } },
        event: { listen() {} },
      },
      addEventListener() {},
    },
    ResizeObserver: class { observe() {} }, requestAnimationFrame() {}, setInterval() {}, queueMicrotask, console,
  });
  mainContext.window.window = mainContext.window;
  mainContext.globalThis = mainContext;
  runInContext(uiBundle, mainContext);
  mainContext.window.dawsuUi = mainContext.dawsuUi;
  await mainContext.pending;
  assert.deepEqual(mainCalls, ["editor_view"]);
  assert.equal(typeof mainContext.window.openEditorMenu, "function");
  assert.equal(typeof mainContext.window.refreshEditorMenu, "function");
});
