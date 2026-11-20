import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const source = await readFile(new URL("../dist/docking.js", import.meta.url), "utf8");
const context = createContext({ window: {} });
runInContext(source, context);
const { defaultLayout, movePanel, visibleLayout } = context.window.editorDocking;
const plain = value => JSON.parse(JSON.stringify(value));

test("the default layout puts Browser and Inspector to the left of Timeline", () => {
  const layout = visibleLayout(defaultLayout(), new Set(["browser", "timeline", "inspector"]));
  assert.equal(layout.axis, "columns");
  assert.equal(layout.second.axis, "columns");
  assert.deepEqual(plain(layout.first.tabs), ["browser"]);
  assert.deepEqual(plain(layout.second.first.tabs), ["inspector"]);
  assert.deepEqual(plain(layout.second.second.tabs), ["timeline"]);
});

test("panel moves can split, stack, and unstack without losing the other panels", () => {
  const all = new Set(["browser", "timeline", "inspector"]);
  let layout = movePanel(defaultLayout(), "browser", "timeline", "below");
  assert.equal(layout.second.axis, "rows");
  assert.deepEqual(plain(layout.second.second.tabs), ["browser"]);
  layout = movePanel(layout, "inspector", "timeline", "tabs");
  assert.deepEqual(plain(layout.first.tabs), ["timeline", "inspector"]);
  assert.equal(layout.first.active, "inspector");
  layout = movePanel(layout, "timeline", "inspector", "right");
  assert.deepEqual(plain(visibleLayout(layout, all).first.first.tabs), ["inspector"]);
  assert.deepEqual(plain(visibleLayout(layout, all).first.second.tabs), ["timeline"]);
});

test("hiding the active tab shows a neighbour and preserves the hidden tab's dock location", () => {
  const layout = movePanel(defaultLayout(), "browser", "timeline", "tabs");
  const hidden = visibleLayout(layout, new Set(["timeline", "inspector"]));
  assert.deepEqual(plain(hidden.second.tabs), ["timeline"]);
  assert.equal(hidden.second.active, "timeline");
  const restored = visibleLayout(layout, new Set(["timeline", "inspector", "browser"]));
  assert.deepEqual(plain(restored.second.tabs), ["timeline", "browser"]);
  assert.equal(restored.second.active, "browser");
});

test("a panel can dock above the timeline in layout editing", () => {
  const layout = movePanel(defaultLayout(), "browser", "timeline", "above");
  assert.equal(layout.second.axis, "rows");
  assert.deepEqual(plain(layout.second.first.tabs), ["browser"]);
  assert.deepEqual(plain(layout.second.second.tabs), ["timeline"]);
});
test("the layout editor moves a Svelte part only as part of its whole ribbon group", () => {
  // Svelte owns the nodes inside [data-ui-mount]; moving one button out would break its updates.
  assert.match(source, /button\.closest\("[^"]*\[data-ui-mount\][^"]*"\)/);
});
