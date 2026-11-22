import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

// The v3 frame: top bar, toolbar row 2, panel heads and rails, and the status bar.
const read = path => readFile(new URL(path, import.meta.url), "utf8");
const html = await read("../dist/index.html");
const statusBarSvelte = await read("../ui/src/parts/StatusBar.svelte");
const app = readEditorSource();
const docking = await read("../dist/docking.js");
const menus = await read("../dist/menus.js");

const between = (text, start, end) => {
  const from = text.indexOf(start);
  assert.ok(from >= 0, "missing " + start);
  return text.slice(from, text.indexOf(end, from));
};
// Each marker must appear after the one before it.
const inOrder = (text, markers, label) => {
  let from = 0;
  for (const marker of markers) {
    const place = text.indexOf(marker, from);
    assert.ok(place >= 0, label + ": " + marker + " missing or out of order");
    from = place + marker.length;
  }
};

test("the top bar reads brand, menus, transport, Export, Save, then the three panel switches", () => {
  const header = between(html, '<header id="editor-header"', "</header>");
  assert.match(header, /<\/strong>\s*<span class="header-divider" aria-hidden="true"><\/span>\s*<nav id="editor-menus"/);
  inOrder(header, ['class="brand"', 'id="editor-menus"', 'id="top-transport"', 'id="export-entry"',
    'class="header-divider"', 'id="save"', 'id="view-browser"', 'id="view-inspector"', 'id="view-controller"'], "top bar");
  assert.equal(header.split('class="header-divider"').length - 1, 3);
  // v3 2.1: an accent diamond, not the old image logo.
  assert.match(header, /class="brand"><span class="brand-diamond" aria-hidden="true"><\/span>dawsu!<\/strong>/);
  assert.doesNotMatch(header, /id="title"|id="layout-edit-entry"/);
  // Save sends the save command, as Ctrl+S does (boot.js), not review_save's file-by-file review.
  assert.match(header, /id="save"/);
  assert.doesNotMatch(header, /data-action="review_save"/);
});

test("row 2 holds the map chip, Hitsounds/Drums, tools, Audition, Playhead and Snap, in that order", () => {
  const ribbon = between(html, '<div id="editor-ribbon"', '<main id="editor-dock"');
  const groups = [...ribbon.matchAll(/class="ribbon-group[^"]*" role="group" aria-label="([^"]+)"/g)].map(match => match[1]);
  assert.deepEqual(groups, ["Map", "Lane mode", "Tool", "Audition", "Playhead", "Grid"]);
  const chip = between(ribbon, 'aria-label="Map"', 'aria-label="Lane mode"');
  inOrder(chip, ['id="map-title"', 'id="map-version"', 'id="difficulty"'], "map chip");
  // Ctrl+O lists sibling difficulties from the select, so it stays laid out under the chip.
  assert.doesNotMatch(chip, /id="difficulty"[^>]*hidden/);
  assert.doesNotMatch(app, /\$\("difficulty"\)\.hidden/);
});

test("the removed controls are gone and each keeps a path in a menu or a key", async () => {
  for (const id of ["go-home", "open", "folders", "undo", "redo", "panel-view", "help", "title"]) {
    assert.doesNotMatch(html, new RegExp(`id="${id}"`), id);
  }
  assert.doesNotMatch(html, /data-command="revert"|popovertarget="options-Lanes"|Dock…/);
  // The Lanes popover itself moves to body level; Ctrl+F still opens it.
  assert.ok(html.indexOf('id="options-Lanes"') > html.indexOf("</header>"));
  assert.ok(!between(html, '<div id="editor-ribbon"', '<main id="editor-dock"').includes('id="options-Lanes"'));
  const commands = await read("../ui/src/lib/commands.js");
  for (const [row, run] of [["startScreen", '["call", "goHome"]'], ["openMap", '["call", "openMap"]'],
    ["folders", '["call", "openFolders"]'], ["viewSettings", '["call", "openViewSettings"]'], ["shortcuts", '["call", "showHelp"]']]) {
    assert.match(commands, new RegExp(`id: "${row}"[^}]*run: ${run.replace(/[[\]]/g, "\\$&")}`), row);
  }
  for (const [row, command] of [["filterLanes", "filter"], ["tallerLanes", "rowsIn"], ["shorterLanes", "rowsOut"]]) {
    assert.match(commands, new RegExp(`id: "${row}"[^}]*command: "${command}"`), row);
    assert.doesNotMatch(between(commands, "export const keyOnly", "];"), new RegExp(`"${command}"`));
  }
  for (const name of ["goHome", "openMap", "openFolders", "openViewSettings", "showHelp"]) {
    assert.match(menus, new RegExp(`${name}: \\(\\) =>`), name);
  }
  // Undo and Redo read the history, not the old buttons.
  assert.doesNotMatch(menus, /\$\("(undo|redo)"\)/);
});

test("the status bar reads speed, play state and selection, then messages, then file, counts and device", () => {
  assert.match(html, /<div id="statusbar" data-ui-mount="status-bar">/);
  inOrder(statusBarSvelte, ['id="playback-rates"', 'id="play-state"', 'id="selection-count"', 'id="status"', 'id="problem-summary"',
    'id="next-output-problem"', 'id="warnings"', 'id="cancel-job"', 'id="file-name"', 'id="hit-lane-counts"',
    'id="status-view"', 'id="audio"'], "status bar");
  inOrder(between(statusBarSvelte, 'id="file-name"', 'id="hit-lane-counts"'), ['id="file-name-head"', 'id="file-name-tail"'], "file name");
});

test("each side panel has a head with a fold button and a rail whose section icons open it at that section", () => {
  for (const [panel, host, sections] of [
    ["browser", "tree-browser-host", ["browse", "search", "library"]],
    ["inspector", "editor-inspector", ["selection", "lane", "difficulty", "bookmarks"]],
  ]) {
    const aside = between(html, `<aside id="${host}"`, "</aside>");
    assert.match(aside, new RegExp(`class="panel-head"[\\s\\S]*id="${panel}-fold"`));
    assert.match(aside, new RegExp(`class="panel-rail"[\\s\\S]*id="${panel}-unfold"`));
    for (const section of sections) assert.match(aside, new RegExp(`id="rail-${panel}-${section}"`), section);
    // Fold, open and rail buttons wait for a map like the switches.
    for (const button of aside.match(/<button id="(?:\w+-fold|\w+-unfold|rail-[\w-]+)"[^>]*>/g)) {
      assert.match(button, /\sdata-doc/, button);
    }
    assert.doesNotMatch(aside, /role="tabpanel"|aria-labelledby="dock-tab/);
  }
  assert.doesNotMatch(docking, /dock-tab|tabButtons|function strip\(|dock-preview/);
  // Rail buttons are named after what they open.
  assert.match(html, /id="rail-browser-browse"[^>]*aria-label="Mapset samples"/);
  assert.match(html, /id="rail-browser-library"[^>]*aria-label="Library"/);
  assert.match(html, /id="rail-browser-search"[^>]*aria-label="Search samples"/);
  assert.match(html, /<div popover id="options-Lanes"[^>]*role="group"/);
});

test("the three panel switches use literal ids", () => {
  assert.match(app, /browser:\s*"view-browser",\s*inspector:\s*"view-inspector",\s*controller:\s*"view-controller"/);
  // docking.js always loads first; the editor keeps no second copy of its rules.
  assert.doesNotMatch(app, /editorDocking\?\.(effectivePanels|togglePlan)|\[data-command\]/);
  assert.doesNotMatch(app, /\$\("view-" \+ panel\)/);
});

// A vm run of app.js with a fake page that keeps every element it hands out.
async function page(dockingApi) {
  const elements = new Map();
  const calls = [];
  function element(id) {
    if (!elements.has(id)) {
      elements.set(id, {
        id, listeners: new Map(), attributes: new Map(), children: [], dataset: {}, hidden: false,
        style: { setProperty() {}, removeProperty() {} },
        classList: { add() {}, remove() {}, toggle() {} },
        addEventListener(name, callback) { this.listeners.set(name, callback); },
        append(...children) { this.children.push(...children); }, prepend() {}, before() {}, after() {},
        replaceChildren(...children) { this.children = children; },
        setAttribute(name, value) { this.attributes.set(name, String(value)); },
        getAttribute(name) { return this.attributes.get(name) ?? null; },
        removeAttribute(name) { this.attributes.delete(name); },
        toggleAttribute(name, on) { if (on) this.attributes.set(name, ""); else this.attributes.delete(name); },
        querySelector: () => null, querySelectorAll: () => [], closest: () => null,
        getClientRects: () => [], focus() {}, scrollIntoView() {},
      });
    }
    return elements.get(id);
  }
  const context = createContext({
    document: { getElementById: element, createElement: () => element(Symbol()), querySelectorAll: () => [],
      addEventListener() {}, readyState: "loading" },
    window: { editorDocking: dockingApi, __TAURI__: {
      core: { invoke: async (_name, args) => {
        if (args.action) calls.push(args.action);
        return { busy: false, hasDocument: true };
      } },
      event: { listen() {} },
    } },
    // A still clock: the playhead is drawn where the editor last put it.
    performance: { now: () => 0 },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
    console,
  });
  await bootEditor(context);
  calls.length = 0;
  return { element, calls, evaluate: code => runInContext(code, context) };
}

import { dockingRules } from "./docking-rules.mjs";
import { bootEditor, readEditorSource } from "./editor-runtime.mjs";
const { effectivePanels, togglePlan } = dockingRules;

test("the problem tooltip lists ten moments and how many more", async () => {
  const { problemTooltip } = await import("../ui/src/lib/statusbar.js");
  const lines = problemTooltip(Array.from({ length: 13 }, (_, i) => ({ time: i, message: "Moment " + i }))).split("\n");
  assert.equal(lines.length, 11);
  assert.equal(lines[9], "Moment 9");
  assert.equal(lines[10], "and 3 more");
});

test("show() fills the chip, Save and panel switches", async () => {
  const { element, evaluate } = await page({ effectivePanels, togglePlan, attach: () => undefined });
  evaluate(`scene = { path: "/maps/Set/Artist - Song (Me) [Hard].osu", mapTitle: "<b>Song</b>", version: "Hard",
    selection: { count: 4 }, history: [], historyPosition: 0, lanes: [] };
    show({ hasDocument: true, dirty: true, playing: true, outputDevice: "Speakers (Realtek)",
      outputProblems: [{ time: 1 }, { time: 2 }], panels: { browser: true, inspector: false, controller: true, laneNames: true } })`);
  // Metadata is text, never markup.
  assert.equal(element("map-title").textContent, "<b>Song</b>");
  assert.equal(element("map-version").textContent, "Hard");
  assert.equal(element("difficulty").title, "/maps/Set/Artist - Song (Me) [Hard].osu");
  assert.equal(element("save").dataset.dirty, "true");
  // Icon-only below 1440 px, so Save's tooltip says what its spoken name says. One wording for the
  // count everywhere: the badge, Save and the status bar.
  assert.equal(element("save").getAttribute("aria-label"), "Save (unsaved changes), 2 moments can't be saved");
  assert.equal(element("save").title, element("save").getAttribute("aria-label"));
  assert.equal(element("save-problem-badge").textContent, "2");
  assert.equal(element("save-problem-badge").title, "2 moments can't be saved");
  assert.equal(element("view-browser").ariaPressed, "true");
  assert.equal(element("view-inspector").ariaPressed, "false");
  assert.equal(element("view-controller").ariaPressed, "true");

  evaluate(`scene.selection = { count: 0 };
    show({ hasDocument: true, dirty: false, playing: false, outputDevice: "", outputProblems: [],
      panels: { browser: false, inspector: true, controller: false, laneNames: true } })`);
  assert.equal(element("save").dataset.dirty, "false");
  assert.equal(element("save").getAttribute("aria-label"), "Save");
  assert.equal(element("save").title, "Save");
  assert.equal(element("view-controller").ariaPressed, "false");
});

test("every entry point folds and opens a panel through togglePanel", async () => {
  const { element, evaluate } = await page({ effectivePanels, togglePlan, attach: () => undefined });
  evaluate(`scene = { path: "a.osu", lanes: [] }; view = { busy: false, hasDocument: true,
    panels: { browser: true, inspector: true, controller: true, laneNames: true } };
    var toggled = []; togglePanel = panel => { toggled.push(panel); };`);
  for (const [id, panel] of [["view-browser", "browser"], ["view-inspector", "inspector"], ["view-controller", "controller"],
    ["browser-fold", "browser"], ["browser-unfold", "browser"], ["inspector-fold", "inspector"],
    ["inspector-unfold", "inspector"], ["sample-browser-close", "browser"]]) {
    evaluate("toggled = []");
    element(id).listeners.get("click")();
    assert.deepEqual(JSON.parse(evaluate("JSON.stringify(toggled)")), [panel], id);
  }
  // Rail icons open their panel first when it is folded.
  evaluate("view.panels = { browser: false, inspector: false, controller: true, laneNames: true }");
  for (const [id, panel] of [["rail-browser-browse", "browser"], ["rail-browser-search", "browser"],
    ["rail-browser-library", "browser"], ["rail-inspector-selection", "inspector"], ["rail-inspector-lane", "inspector"],
    ["rail-inspector-difficulty", "inspector"], ["rail-inspector-bookmarks", "inspector"]]) {
    evaluate("toggled = []");
    await element(id).listeners.get("click")();
    assert.deepEqual(JSON.parse(evaluate("JSON.stringify(toggled)")), [panel], id);
  }
  assert.match(menus, /kind === "panel"[^\n]*togglePanel\(first\)/);
});

test("folding while the window is narrow changes no saved preference", async () => {
  const { calls, evaluate } = await page({ effectivePanels, togglePlan, attach: () => undefined });
  evaluate(`scene = { path: "a.osu", lanes: [] }; view = { busy: false, hasDocument: true,
    panels: { browser: true, inspector: true, controller: true, laneNames: true } };
    dockState = () => ({ narrow: true, override: null }); var overrides = []; setDockOverride = value => overrides.push(value);`);
  evaluate('togglePanel("browser")');
  await evaluate("pending");
  assert.deepEqual(calls, []);
  assert.deepEqual(JSON.parse(evaluate("JSON.stringify(overrides)")), [true]);
  evaluate('togglePanel("inspector")');
  await evaluate("pending");
  assert.deepEqual(calls.map(call => [call.kind, call.panel, call.on]), [["panel", "inspector", false]]);
});
