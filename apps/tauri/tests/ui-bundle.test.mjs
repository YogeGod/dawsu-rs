import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

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

test("parts use none of the selectors app.js wires and no static style attribute", () => {
  // app.js adds listeners to these by selector at load, so a part using one would act twice.
  const shared = /data-(?:action|command|volume|locator|map|doc|playback-speed)\b|editor-tool-button/;
  for (const { name, text } of sources()) {
    assert.doesNotMatch(text, shared, name);
    // The window's policy blocks style="" attributes; parts set custom properties or classes.
    assert.doesNotMatch(text, /\sstyle\s*=/, name);
    assert.doesNotMatch(text, /url\(/, name + " (fonts and images stay in style.css)");
  }
});

test("v3 tokens never redefine a custom property style.css already sets", () => {
  const old = definedProperties(read(new URL("dist/style.css", tauri)));
  for (const { name, text } of sources().filter(file => /\.(css|svelte)$/.test(file.name))) {
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

test("index.html loads the bundle after the old styles and scripts", () => {
  const head = read(new URL("dist/index.html", tauri)).match(/<head>(.*?)<\/head>/s)[1];
  const order = [...head.matchAll(/(?:href|src)="([^"]+\.(?:css|js))"/g)].map(match => match[1]);
  assert.deepEqual(order, ["style.css", "ui/ui.css", "docking.js", "app.js", "menus.js", "drum-detect.js", "ui/ui.js"]);
  assert.match(head, /<script defer src="ui\/ui\.js"><\/script>/);
});
