import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

// The v3 frame: top bar, toolbar row 2, panel heads and rails, and the status bar.
const read = path => readFile(new URL(path, import.meta.url), "utf8");
const html = await read("../dist/index.html");
const app = await read("../dist/app.js");
const docking = await read("../dist/docking.js");
const menus = await read("../dist/menus.js");

const between = (text, start, end) => {
  const from = text.indexOf(start);
  assert.ok(from >= 0, "missing " + start);
  return text.slice(from, text.indexOf(end, from));
};
const inOrder = (text, markers, label) => {
  const places = markers.map(marker => [marker, text.indexOf(marker)]);
  for (const [marker, place] of places) assert.ok(place >= 0, label + ": missing " + marker);
  assert.deepEqual(places.map(([marker]) => marker), places.slice().sort((a, b) => a[1] - b[1]).map(([marker]) => marker), label);
};

test("the top bar reads brand, menus, transport, Export, Save, then the three panel switches", () => {
  const header = between(html, '<header id="editor-header"', "</header>");
  inOrder(header, ['class="brand"', 'id="editor-menus"', 'id="top-transport"', 'id="export-entry"',
    'class="header-divider"', 'id="save"', 'id="view-browser"', 'id="view-inspector"', 'id="view-controller"'], "top bar");
  assert.equal(header.split('class="header-divider"').length - 1, 2);
  assert.doesNotMatch(header, /id="title"|id="layout-edit-entry"/);
  assert.match(header, /id="save"[^>]*data-action="review_save"/);
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
  for (const id of ["go-home", "open", "folders", "undo", "redo", "panel-view", "help", "title", "lane-height-in",
    "lane-height-out", "lane-height"]) {
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
  const bar = between(html, '<div id="statusbar"', "<dialog");
  inOrder(bar, ['id="playback-rates"', 'id="play-state"', 'id="selection-count"', 'id="status"', 'id="problem-summary"',
    'id="next-output-problem"', 'id="warnings"', 'id="cancel-job"', 'id="file-name"', 'id="hit-lane-counts"',
    'id="status-view"', 'id="audio"'], "status bar");
  inOrder(between(bar, 'id="file-name"', "</span></span>") + "</span></span>", ['id="file-name-head"', 'id="file-name-tail"'], "file name");
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
    assert.doesNotMatch(aside, /role="tabpanel"|aria-labelledby="dock-tab/);
  }
  assert.doesNotMatch(docking, /dock-tab|tabButtons|function strip\(/);
});

test("the three panel switches use literal ids", () => {
  assert.match(app, /\{ browser: "view-browser", inspector: "view-inspector", controller: "view-controller" \}/);
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
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
    console,
  });
  await runInContext(app, context);
  calls.length = 0;
  return { element, calls, evaluate: code => runInContext(code, context) };
}

const dockingContext = createContext({ window: {} });
runInContext(docking, dockingContext);
const { effectivePanels, togglePlan } = dockingContext.window.editorDocking;

test("show() fills the chip, Save, play state, selection count, file name and output device", async () => {
  const { element, evaluate } = await page({ effectivePanels, togglePlan });
  evaluate(`scene = { path: "/maps/Set/Artist - Song (Me) [Hard].osu", mapTitle: "<b>Song</b>", version: "Hard",
    selection: { count: 4 }, history: [], historyPosition: 0, lanes: [] };
    show({ hasDocument: true, dirty: true, playing: true, outputDevice: "Speakers (Realtek)",
      outputProblems: [{ time: 1 }, { time: 2 }], panels: { browser: true, inspector: false, controller: true, laneNames: true } })`);
  // Metadata is text, never markup.
  assert.equal(element("map-title").textContent, "<b>Song</b>");
  assert.equal(element("map-version").textContent, "Hard");
  assert.equal(element("difficulty").title, "/maps/Set/Artist - Song (Me) [Hard].osu");
  assert.equal(element("save").dataset.dirty, "true");
  assert.equal(element("save").getAttribute("aria-label"), "Save (unsaved changes), 2 problems");
  assert.equal(element("save-problem-badge").textContent, "2");
  assert.equal(element("play-state").textContent, "Playing");
  assert.equal(element("selection-count").textContent, "4 hits selected");
  assert.equal(element("selection-count").hidden, false);
  assert.equal(element("audio").textContent, "Speakers (Realtek)");
  assert.equal(element("audio").getAttribute("aria-label"), "Audio output: Speakers (Realtek)");
  assert.equal(element("view-browser").ariaPressed, "true");
  assert.equal(element("view-inspector").ariaPressed, "false");
  assert.equal(element("view-controller").ariaPressed, "true");

  evaluate(`scene.selection = { count: 0 };
    show({ hasDocument: true, dirty: false, playing: false, outputDevice: "", outputProblems: [],
      panels: { browser: false, inspector: true, controller: false, laneNames: true } })`);
  assert.equal(element("save").dataset.dirty, "false");
  assert.equal(element("save").getAttribute("aria-label"), "Save");
  assert.equal(element("play-state").textContent, "Stopped");
  assert.equal(element("selection-count").hidden, true);
  assert.equal(element("audio").textContent, "Audio output");
  assert.equal(element("view-controller").ariaPressed, "false");
});

test("every entry point folds and opens a panel through togglePanel", async () => {
  const { element, evaluate } = await page({ effectivePanels, togglePlan });
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
  assert.match(menus, /kind === "panel"[^\n]*togglePanel\(first\)/);
});

test("folding while the window is narrow changes no saved preference", async () => {
  const { calls, evaluate } = await page({ effectivePanels, togglePlan });
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
