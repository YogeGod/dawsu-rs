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
  ...emptyBrowser, source: "mapset", root: "Mapset samples", defaults: true, canPick: true,
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
  assert.match(mapset.tag, /aria-selected="true"/);
  assert.match(library.tag, /id="sample-browser-library"/);
  assert.match(library.tag, /aria-selected="false"/);
  assert.equal(button(html, "Up"), undefined, "the tree has no Up button");
  const classic = shown(model({ classic: true, root: null, folder: { label: "percussion", title: "percussion", canUp: true } }), BrowserHead);
  assert.ok(!disabled(button(classic, "Up").tag));
  assert.match(classic, /<output[^>]*title="percussion"[^>]*>percussion<\/output>/);
});

test("the tree lists Mapset samples with file rows, the selected row, and greyed Default samples", () => {
  const html = shown(model(), BrowserList);
  assert.match(html, /Mapset samples/);
  const rows = buttons(html);
  assert.deepEqual(rows.map(row => row.text), ["percussion", "snare.wav", "kick.wav", "osu! skin", "Sample folders"]);
  assert.match(rows[0].tag, /aria-expanded="true"/);
  assert.match(rows[1].tag, /aria-pressed="true"/);
  assert.match(rows[1].tag, /browser-selected/);
  assert.doesNotMatch(rows[2].tag, /browser-selected/);
  assert.match(rows[1].tag, /--depth: 2/, "a file inside a folder under Mapset samples sits two steps in");
  assert.ok(disabled(rows[3].tag) && disabled(rows[4].tag), "osu! skin and Sample folders have no code yet");
  assert.match(html, /Default samples/);
  const library = shown(model({ source: "library", root: "library", defaults: false }), BrowserList);
  assert.doesNotMatch(library, /Default samples/);
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
  assert.ok(!disabled(button(html, "Browse…").tag));
  // Undecoded: the name only, never a made-up length or outline.
  const bare = shown(model({ preview: { ...preview, ms: null, peaks: null } }), BrowserPreview);
  assert.doesNotMatch(bare, /Sample preview/);
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
