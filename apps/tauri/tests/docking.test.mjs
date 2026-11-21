import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const source = await readFile(new URL("../dist/docking.js", import.meta.url), "utf8");
const context = createContext({ window: {} });
runInContext(source, context);
const { defaultLayout, movePanel, visibleLayout, minimum, effectivePanels, togglePlan } = context.window.editorDocking;
const plain = value => JSON.parse(JSON.stringify(value));

test("the default layout is v3's: Browser, then Timeline, then Inspector", () => {
  const layout = visibleLayout(defaultLayout(), new Set(["browser", "timeline", "inspector"]));
  assert.equal(layout.axis, "columns");
  assert.equal(layout.second.axis, "columns");
  assert.deepEqual(plain(layout.first.tabs), ["browser"]);
  assert.deepEqual(plain(layout.second.first.tabs), ["timeline"]);
  assert.deepEqual(plain(layout.second.second.tabs), ["inspector"]);
  assert.match(layout.second.size.basis, /var\(--dock-inspector-width\)/);
});

test("panel moves split and stack without losing the other panels, and never make tabs", () => {
  const all = new Set(["browser", "timeline", "inspector"]);
  let layout = movePanel(defaultLayout(), "browser", "timeline", "below");
  assert.equal(layout.axis, "columns");
  assert.equal(layout.first.axis, "rows");
  assert.deepEqual(plain(layout.first.second.tabs), ["browser"]);
  layout = movePanel(layout, "inspector", "timeline", "left");
  const shown = visibleLayout(layout, all);
  const groups = [];
  const walk = node => node.tabs ? groups.push(plain(node.tabs)) : (walk(node.first), walk(node.second));
  walk(shown);
  assert.deepEqual(groups, [["inspector"], ["timeline"], ["browser"]]);
  for (const tabs of groups) assert.equal(tabs.length, 1);
  assert.doesNotMatch(source, /"tabs"\)|placement === "tabs"|Tab with/);
});

test("a panel can dock above the timeline in layout editing", () => {
  const layout = movePanel(defaultLayout(), "browser", "timeline", "above");
  assert.equal(layout.first.axis, "rows");
  assert.deepEqual(plain(layout.first.first.tabs), ["browser"]);
  assert.deepEqual(plain(layout.first.second.tabs), ["timeline"]);
});

test("a folded panel needs only its 44 px rail", () => {
  const layout = defaultLayout();
  assert.equal(minimum(layout.first, "columns", new Set()), 170);
  assert.equal(minimum(layout.first, "columns", new Set(["browser"])), 44);
  assert.equal(minimum(layout.second, "columns", new Set(["inspector"])), 274 + 6 + 44);
  // In a rows split a folded panel is a 44 px strip.
  assert.equal(minimum(layout.first, "rows", new Set(["browser"])), 44);
});

test("the effective panels follow the preferences, the classic Browser and narrow windows", () => {
  const cases = [
    // browserOpen, inspectorOpen, classic, narrow -> browser, inspector
    [true, true, false, false, "open", "open"],
    [true, false, false, false, "open", "folded"],
    [false, true, false, false, "folded", "open"],
    [false, false, false, false, "folded", "folded"],
    [true, true, true, false, "classic", "open"],
    [true, false, true, false, "open", "folded"],
    [false, true, true, false, "off", "open"],
    [false, false, true, false, "off", "folded"],
    [true, true, false, true, "folded", "open"],
    [true, false, false, true, "folded", "folded"],
    [false, true, false, true, "folded", "open"],
    [false, false, false, true, "folded", "folded"],
    [true, true, true, true, "classic", "open"],
    [true, false, true, true, "off", "folded"],
    [false, true, true, true, "off", "open"],
    [false, false, true, true, "off", "folded"],
  ];
  for (const [browserOpen, inspectorOpen, classic, narrow, browser, inspector] of cases) {
    assert.deepEqual(plain(effectivePanels({ browserOpen, inspectorOpen, classic }, narrow, null)), { browser, inspector },
      JSON.stringify({ browserOpen, inspectorOpen, classic, narrow }));
  }
  // Opening the Browser while narrow is a session override.
  assert.equal(effectivePanels({ browserOpen: false, inspectorOpen: true, classic: false }, true, true).browser, "open");
  assert.equal(effectivePanels({ browserOpen: true, inspectorOpen: false, classic: true }, true, true).browser, "open");
  // The override means nothing at full width.
  assert.equal(effectivePanels({ browserOpen: false, inspectorOpen: true, classic: false }, false, true).browser, "folded");
});

test("every way to fold or open a panel goes through one plan; narrow windows never save", () => {
  const prefs = (browserOpen, inspectorOpen, classic = false, controllerOpen = true) =>
    ({ browserOpen, inspectorOpen, classic, controllerOpen });
  const panel = (name, on) => ({ act: { kind: "panel", panel: name, on } });
  assert.deepEqual(plain(togglePlan("browser", prefs(true, true), false, null)), panel("browser", false));
  assert.deepEqual(plain(togglePlan("browser", prefs(false, true), false, null)), panel("browser", true));
  assert.deepEqual(plain(togglePlan("inspector", prefs(true, true), false, null)), panel("inspector", false));
  assert.deepEqual(plain(togglePlan("inspector", prefs(true, false), true, null)), panel("inspector", true));
  assert.deepEqual(plain(togglePlan("controller", prefs(true, true, false, true), false, null)), panel("controller", false));
  // Narrow: only the session override changes.
  assert.deepEqual(plain(togglePlan("browser", prefs(true, true), true, null)), { override: true });
  assert.deepEqual(plain(togglePlan("browser", prefs(false, true), true, null)), { override: true });
  assert.deepEqual(plain(togglePlan("browser", prefs(false, true), true, true)), { override: null });
  // The classic Browser sits in the Inspector, so narrow windows don't fold it.
  assert.deepEqual(plain(togglePlan("browser", prefs(true, true, true), true, null)), panel("browser", false));
  assert.deepEqual(plain(togglePlan("browser", prefs(false, true, true), true, null)), panel("browser", true));
});

test("the layout editor moves a Svelte part only as part of its whole ribbon group", () => {
  // Svelte owns the nodes inside [data-ui-mount]; moving one button out would break its updates.
  assert.match(source, /button\.closest\("[^"]*\[data-ui-mount\][^"]*"\)/);
  // Panel heads and rails stay with their panel.
  assert.match(source, /button\.closest\("[^"]*\.panel-head, \.panel-rail[^"]*"\)/);
});

test("matchMedia is read only inside attach()", () => {
  const before = source.slice(0, source.indexOf("function attach("));
  assert.doesNotMatch(before, /matchMedia/);
  assert.match(source, /matchMedia\?\.\("\(max-width: 1179px\)"\)/);
});

test("hiding a panel leaves the others where they were and brings it back in its place", () => {
  const all = new Set(["browser", "timeline", "inspector"]);
  const layout = movePanel(defaultLayout(), "browser", "inspector", "below");
  const hidden = visibleLayout(layout, new Set(["timeline", "inspector"]));
  assert.deepEqual(plain(hidden.first.tabs), ["timeline"]);
  assert.deepEqual(plain(hidden.second.tabs), ["inspector"]);
  const restored = visibleLayout(layout, all);
  assert.equal(restored.second.axis, "rows");
  assert.deepEqual(plain(restored.second.second.tabs), ["browser"]);
});

// attach() on a fake page: what it draws, when it redraws and where focus goes.
async function attached({ narrow = false } = {}) {
  const { fakeDocument, editorPage } = await import("./fake-dom.mjs");
  const dom = fakeDocument({ narrow });
  const page = editorPage(dom.build);
  dom.document.body.append(...page.childNodes);
  const sandbox = createContext({ window: dom.window, document: dom.document, MutationObserver: dom.MutationObserver,
    waveformHeight: 56, visibleWaveformHeight: 56 });
  runInContext(source, sandbox);
  const update = sandbox.window.editorDocking.attach(dom.document.getElementById("editor-dock"));
  return { dom, update, byId: id => dom.document.getElementById(id) };
}
const tree = (prefs = {}) => ({ browserStyle: "tree", browserOpen: true, inspectorOpen: true, ...prefs });

test("attach draws once per change of what the frame shows (M5)", async () => {
  const { update } = await attached();
  update(tree());
  update(tree());
  assert.equal(update.renders(), 1);
  update(tree({ browserOpen: false }));
  assert.equal(update.renders(), 2);
});

test("a folded panel locks its split to the rail and keeps the mapper's width for later (M2)", async () => {
  const { update, byId } = await attached();
  update(tree({ browserOpen: false }));
  const outer = byId("editor-dock").children[0];
  assert.equal(outer.getAttribute("data-locked"), "");
  assert.equal(outer.style.getPropertyValue("--dock-first"), "var(--rail-width)");
  assert.equal(outer.children[1].getAttribute("role"), null, "the grip rests while folded");
  assert.equal(byId("tree-browser-host").getAttribute("data-folded"), "");
  assert.equal(byId("sample-browser").hidden, true);
  assert.equal(byId("browser-unfold").parentNode.hidden, false);
  update(tree());
  const open = byId("editor-dock").children[0];
  assert.equal(open.hasAttribute("data-locked"), false);
  assert.match(open.style.getPropertyValue("--dock-first"), /var\(--dock-browser-width\)/);
});

test("the classic Browser moves into an open Inspector and back to the tree panel when it folds (M4)", async () => {
  const { update, byId } = await attached();
  update(tree({ browserStyle: "classic" }));
  assert.equal(byId("sample-browser").parentNode, byId("classic-browser-host"));
  assert.equal(byId("classic-browser-host").hidden, false);
  assert.equal(byId("tree-browser-host").getClientRects().length, 0, "no left panel or rail in classic");
  update(tree({ browserStyle: "classic", inspectorOpen: false }));
  assert.equal(byId("sample-browser").parentNode, byId("tree-browser-host"));
  assert.equal(byId("classic-browser-host").hidden, true);
  update(tree({ browserStyle: "tree" }));
  assert.equal(byId("sample-browser").parentNode, byId("tree-browser-host"));
});

test("focus follows a panel that folds or opens, also when a narrow window folds it", async () => {
  const { dom, update, byId } = await attached();
  update(tree());
  byId("browser-fold").focus();
  update(tree({ browserOpen: false }));
  assert.equal(dom.document.activeElement, byId("browser-unfold"));
  update(tree());
  assert.equal(dom.document.activeElement, byId("browser-fold"));
  // The window narrows: app.js hands the same preferences over again and the Browser folds.
  dom.window.setNarrow(true);
  update(tree());
  assert.equal(update.state().narrow, true);
  assert.equal(dom.document.activeElement, byId("browser-unfold"));
  byId("inspector-fold").focus();
  update(tree({ inspectorOpen: false }));
  assert.equal(dom.document.activeElement, byId("inspector-unfold"));
});
