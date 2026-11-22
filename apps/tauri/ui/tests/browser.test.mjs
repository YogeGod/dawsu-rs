import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { Bridge, bridge } from "../src/lib/bridge.svelte.js";
import { emptyBrowser, outlineBars, previewLength } from "../src/lib/browser.js";
import BrowserHead from "../src/parts/BrowserHead.svelte";
import BrowserList from "../src/parts/BrowserList.svelte";
import BrowserPreview from "../src/parts/BrowserPreview.svelte";

const model = (more = {}) => ({
  ...emptyBrowser, source: "mapset", root: "Mapset samples", canPick: true,
  rows: [
    { kind: "folder", key: "percussion", label: "percussion", title: "percussion", depth: 0, open: true, selected: false },
    { kind: "sample", key: "percussion/snare.wav", label: "snare.wav", title: "percussion/snare.wav", depth: 1, selected: true },
    { kind: "sample", key: "kick.wav", label: "kick.wav", title: "kick.wav", depth: 0, selected: false },
  ],
  ...more,
});
const scene = { path: "a.osu" };
function shown(browser, part) {
  bridge.set(null, scene);
  bridge.browser = browser;
  return render(part).body;
}
const buttons = html => [...html.matchAll(/<button\b[^>]*>([^]*?)<\/button>/g)].map(match => ({
  tag: match[0].slice(0, match[0].indexOf(">") + 1), text: match[1].replace(/<[^>]+>/g, "").trim(),
}));
const button = (html, text) => buttons(html).find(item => item.text === text);
const disabled = tag => /\sdisabled/.test(tag);
const source = name => readFileSync(new URL(`../src/parts/${name}.svelte`, import.meta.url), "utf8");

test("the preview length reads in seconds, like v3's 0.21 s", () => {
  assert.equal(previewLength(208.3), "0.21 s");
  assert.equal(previewLength(1500), "1.50 s");
  assert.equal(previewLength(0), "0.00 s");
});

test("the outline draws one centred bar per peak, never thinner than a hairline", () => {
  assert.deepEqual(outlineBars([1, 0.5, 0]), [
    { x: 0.15, y: 0, height: 1 },
    { x: 1.15, y: 0.25, height: 0.5 },
    { x: 2.15, y: 0.48, height: 0.04 },
  ].map(bar => ({ ...bar, width: 0.7 })));
  assert.deepEqual(outlineBars(null), []);
});

test("the head shows Mapset and Library tabs and the search field", () => {
  const html = shown(model(), BrowserHead);
  assert.match(html, /id="sample-browser-filter"/);
  assert.match(html, /placeholder="Search samples"/);
  const mapset = button(html, "Mapset");
  const library = button(html, "Library");
  assert.match(mapset.tag, /id="sample-browser-mapset"/);
  // Two switch buttons, not a tab list: the rows below are the same list for both.
  assert.match(mapset.tag, /aria-pressed="true"/);
  assert.match(library.tag, /id="sample-browser-library"/);
  assert.match(library.tag, /aria-pressed="false"/);
  assert.doesNotMatch(html, /role="tab/);
  // One click to the mapset's and the library's folders, through open_editor_folder.
  const folders = buttons(html).filter(item => /browser-open-folder/.test(item.tag));
  assert.deepEqual(folders.map(item => item.tag.match(/aria-label="([^"]+)"/)[1]), ["Open mapset folder", "Open sample library folder"]);
  assert.ok(folders.every(item => !disabled(item.tag)));
  assert.match(source("BrowserHead"), /invoke\("open_editor_folder", \{ folder \}\)/);
  assert.match(source("BrowserHead"), /openFolder\("map"\)[^]*openFolder\("library"\)/);
  assert.equal(button(html, "Up"), undefined, "the tree has no Up button");
  const classic = shown(model({ classic: true, root: null, folder: { label: "percussion", title: "percussion", canUp: true } }), BrowserHead);
  assert.ok(!disabled(button(classic, "Up").tag));
  assert.match(classic, /<output[^>]*title="percussion"[^>]*>percussion<\/output>/);
});

test("the tree lists Mapset samples with file rows and the selected row, and no dead Default samples rows", () => {
  const html = shown(model(), BrowserList);
  assert.match(html, /Mapset samples/);
  const root = html.match(/<div class="browser-row browser-root"[^]*?<\/div>/)[0];
  assert.doesNotMatch(root, /m6 14 1\.5-2\.9/, "the top row uses the closed folder icon");
  const rows = buttons(html);
  assert.deepEqual(rows.map(row => row.text), ["percussion", "snare.wav", "kick.wav"]);
  assert.match(rows[0].tag, /aria-expanded="true"/);
  assert.match(rows[1].tag, /aria-pressed="true"/);
  assert.match(rows[1].tag, /browser-selected/);
  assert.doesNotMatch(rows[2].tag, /browser-selected/);
  assert.match(rows[1].tag, /--depth: 2/, "a file inside a folder under Mapset samples sits two steps in");
  assert.match(rows[1].tag, /data-sample="percussion\/snare\.wav"/, "ArrowUp and ArrowDown find the next sample by it");
  assert.doesNotMatch(html, /Default samples|osu! skin|Sample folders/);
  assert.equal(button(html, "Choose library folder…"), undefined);
  const unset = shown(model({ source: "library", root: null, chooseLibrary: true, busy: false,
    rows: [{ kind: "note", key: "note:0", label: "No library folder chosen yet.", depth: 0 }] }), BrowserList);
  assert.ok(!disabled(button(unset, "Choose library folder…").tag), "an unset Library offers its folder picker");
  const busy = shown(model({ busy: true }), BrowserList);
  assert.ok(disabled(button(busy, "snare.wav").tag));
  assert.ok(!disabled(button(busy, "percussion").tag), "folders still open while a job runs");
  const note = shown(model({ rows: [{ kind: "note", key: "note:0", label: "No matching samples.", depth: 0 }] }), BrowserList);
  assert.match(note, /No matching samples\./);
});

test("the preview shows the file name, its real length and its outline with a play button", () => {
  const preview = { name: "soft-hitclap2.wav", title: "soft-hitclap2.wav", ms: 210, peaks: [1, 0.5] };
  const html = shown(model({ preview, canAssign: true }), BrowserPreview);
  assert.match(html, /soft-hitclap2\.wav/);
  assert.match(html, /Sample preview · 0\.21 s/);
  assert.equal((html.match(/<rect\b/g) ?? []).length, 2);
  assert.match(button(html, "").tag, /aria-label="Play soft-hitclap2\.wav"/);
  assert.ok(!disabled(button(html, "Use in active lane").tag));
  const named = shown(model({ preview, canAssign: true, target: "Soft Clap" }), BrowserPreview);
  assert.ok(button(named, "Use in Soft Clap"), "Use names the lane it replaces");
  assert.ok(!disabled(button(html, "Browse…").tag));
  // Undecoded: the name only, never a made-up length or outline.
  const bare = shown(model({ preview: { ...preview, ms: null, peaks: null } }), BrowserPreview);
  assert.doesNotMatch(bare, /Sample preview/);
  assert.match(bare, /class="browser-preview-length"/, "the length line keeps its place while the length loads");
  assert.doesNotMatch(bare, /<rect\b/);
  assert.ok(disabled(button(bare, "Use in active lane").tag));
  const none = shown(model(), BrowserPreview);
  assert.doesNotMatch(none, /browser-preview-name/);
});

test("before the first hand-over every control is greyed out", () => {
  bridge.set(null, null);
  bridge.browser = null;
  for (const part of [BrowserHead, BrowserPreview]) {
    for (const item of buttons(render(part).body)) assert.ok(disabled(item.tag), item.text);
  }
});

test("the bridge writes the Browser model at once, like the lanes", () => {
  const local = new Bridge();
  let flushed = 0;
  local.handOverBrowser({ rows: [] }, () => flushed++);
  assert.deepEqual(local.browser, { rows: [] });
  assert.equal(flushed, 1);
});

test("the parts send only through sampleBrowserAction and use no static style", () => {
  for (const name of ["BrowserHead", "BrowserList", "BrowserPreview"]) {
    const text = source(name);
    assert.doesNotMatch(text, /\sstyle="/, name);
    assert.doesNotMatch(text, /data-(action|doc|map|command)\b/, name);
  }
});

test("the stylesheet keeps the selected tab on hover, caps the indent and dims a disabled row once", () => {
  const css = readFileSync(new URL("../src/parts/browser.css", import.meta.url), "utf8");
  // The declarations of the first rule whose selector list starts with `selector`.
  const rule = selector => {
    const at = css.indexOf("\n" + selector);
    return at < 0 ? "" : css.slice(css.indexOf("{", at) + 1, css.indexOf("}", at));
  };
  assert.match(css, /\.browser-tabs button\[aria-pressed="true"\]:hover:not\(:disabled\)/);
  assert.doesNotMatch(rule('.browser-tabs button[aria-pressed="true"]'), /box-shadow/);
  assert.match(rule(".browser-row {"), /min\(var\(--depth, 0\), 6\)/);
  assert.match(rule(".browser-row:disabled"), /opacity: 1/);
  assert.match(rule(".sample-browser-classic-head {"), /border-bottom/);
});
