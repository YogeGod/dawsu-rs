import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor, readEditorSource } from "./editor-runtime.mjs";

const read = file => readFile(new URL("../dist/" + file, import.meta.url), "utf8");
const [html, css, statusBarSvelte] = await Promise.all([
  read("index.html"), read("style.css"),
  readFile(new URL("../ui/src/parts/StatusBar.svelte", import.meta.url), "utf8"),
]);
const app = readEditorSource();

const ribbon = html.slice(html.indexOf('<div id="editor-ribbon"'), html.indexOf('<main id="editor-dock"'));
const toolbarCss = css.slice(css.indexOf("/* Toolbar and status bar (design f1)"));

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
  const status = { busy: false, hasDocument: false };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(JSON.parse(JSON.stringify(args?.action ?? null)));
        return { ...status };
      } },
      event: { listen() {} },
    } },
    Option: function (text, value) { this.text = text; this.value = value; },
    ResizeObserver: class { observe() {} },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await bootEditor(context);
  calls.length = 0;
  const evaluate = code => runInContext(code, context);
  evaluate("renderLanes = () => {};");
  return { calls, evaluate, get, settle: () => evaluate("pending") };
}

const sceneWith = fields => JSON.stringify({
  path: "Hard.osu", lanes: [{ key: "a" }, { key: "b" }], difficulties: [], history: [], historyPosition: 0,
  snap: true, beats: 1, divisor: 4, moments: [[100, 100, true, [[0, false], [1, false]]], [200, 100, true, [[0, false]]]],
  ...fields,
});

test("the toolbar holds the tool pills, Snap, Audition, the Playhead group and the Hitsounds/Drums switch", () => {
  for (const id of ["grid-options", "samples-view", "drums-view"]) {
    assert.match(ribbon, new RegExp('id="' + id + '"'), id);
  }
  // The tool pills, the Audition pill and the Playhead group are Svelte parts, each in its own ribbon group.
  for (const [group, part] of [["Tool", "tools"], ["Audition", "audition"], ["Grid", "snap"], ["Playhead", "playhead"]]) {
    const start = ribbon.indexOf('aria-label="' + group + '"');
    const body = ribbon.slice(start, ribbon.indexOf('class="ribbon-group"', start));
    assert.match(body, new RegExp('data-ui-mount="' + part + '"'), group);
  }
  assert.doesNotMatch(ribbon, /id="tools"|id="toolbar-audition"/);
  // The Lanes pop-up has no toolbar button now (Ctrl+F and View › Filter lanes… open it).
  assert.doesNotMatch(ribbon, /options-Lanes/);
  assert.doesNotMatch(html, /id="follow"|id="transport-return"/, "Follow and Return moved into the Playhead part");
  assert.match(ribbon, /id="samples-view"[^>]*>.*>Hitsounds<\/span><\/button>/, "the design names the sample-lane view Hitsounds");
  assert.match(ribbon, /id="drums-view"[^>]*>.*>Drums<\/span><\/button>/);
  // Revert moved to File › Revert to saved….
  assert.doesNotMatch(ribbon, /data-command="revert"/);
});

test("the old tool buttons and the old Follow and Return toggles left style.css", () => {
  assert.ok(toolbarCss.length > 0, "toolbar CSS block");
  assert.doesNotMatch(css, /#tools\b|editor-tool-button|ribbon-tool-label/);
  assert.doesNotMatch(css, /\.toolbar-toggle[^-]|\.toolbar-track|toggle-input/);
  assert.match(toolbarCss, /#editor-ribbon \.ribbon-caption\s*\{[^}]*display:\s*none/);
  assert.doesNotMatch(toolbarCss, /:has\(|color-mix\(|aspect-ratio/);
  assert.match(toolbarCss, /@media\s*\(forced-colors:\s*active\)\s*\{[^{}]*aria-pressed=true[^{}]*\{[^}]*outline/);
});

test("the right-click picker keeps all seven tools, in the parts' order", async () => {
  const { TOOLS } = await import("../ui/src/lib/tools.js");
  const { items, menus } = await import("../ui/src/lib/commands.js");
  const names = menus.tool.map(id => items.find(item => item.id === id).label);
  assert.deepEqual(names, TOOLS);
  assert.equal(names.length, 7);
});

test("the status bar shows speeds first, then status and problems, with counts on the right", async () => {
  const { PLAYBACK_SPEEDS } = await import("../ui/src/lib/statusbar.js");
  assert.deepEqual(PLAYBACK_SPEEDS, [0.25, 0.5, 0.75, 1, 1.25, 1.5]);
  const order = ["PLAYBACK_SPEEDS", 'id="status"', 'id="problem-summary"', 'id="hit-lane-counts"'].map(text => statusBarSvelte.indexOf(text));
  assert.ok(order.every(index => index >= 0), String(order));
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
});

test("show() fills the Inspector's Audition and passes status bar state to the Svelte bundle", async () => {
  const { calls, evaluate, settle } = await frontend();
  evaluate("window.dawsuUi = { update(view, scene) { this.view = view; this.scene = scene; }, statusNotice(text) { this.notice = text; } }");
  evaluate(`show({ busy: false, hasDocument: true, audition: true, outputProblems: [{ time: 1 }, { time: 2 }],
    scene: ${sceneWith({})} })`);
  assert.equal(evaluate("inspectorModel().audition"), true, "the Inspector's Audition box follows the editor");
  assert.equal(evaluate("window.dawsuUi.view.outputProblems.length"), 2);
  assert.equal(evaluate("window.dawsuUi.scene.lanes.length"), 2);

  evaluate("inspectorAction('toggle', 'audition', false)");
  await settle();
  assert.deepEqual(calls.at(-1), { kind: "audition", on: false });

  evaluate(`show({ busy: false, hasDocument: true, audition: false, outputProblems: [{ time: 1 }],
    scene: ${sceneWith({ snap: false, lanes: [{ key: "a" }], moments: [[100, 100, true, [[0, false]]]] })} })`);
  assert.equal(evaluate("inspectorModel().audition"), false);
  assert.equal(evaluate("window.dawsuUi.view.outputProblems.length"), 1);

  evaluate("show({ busy: false, hasDocument: false, outputProblems: [] })");
  assert.equal(evaluate("window.dawsuUi.view.outputProblems.length"), 0);
});

test("show() hands the view and the last scene to the Svelte parts, even when a part throws", async () => {
  const { evaluate, get } = await frontend();
  evaluate("window.dawsuUi = { update(view, scene) { this.got = JSON.stringify([view.audition, scene?.divisor]); } }");
  evaluate(`show({ busy: false, hasDocument: true, audition: true, scene: ${sceneWith({})} })`);
  assert.equal(evaluate("window.dawsuUi.got"), "[true,4]");
  // Light polls carry no scene; the parts keep the last one.
  evaluate("show({ busy: false, hasDocument: true, audition: false })");
  assert.equal(evaluate("window.dawsuUi.got"), "[false,4]");
  evaluate("console = { error() {} }; window.dawsuUi.update = () => { throw new Error('broken part'); }");
  evaluate(`show({ busy: false, hasDocument: true, audition: true, scene: ${sceneWith({ moments: [] })} })`);
  assert.equal(get("save").dataset.dirty, "false");
});

test("the status bar says when a toolbar part did not load, once the page has loaded", async () => {
  const { evaluate, get } = await frontend();
  const shown = `show({ busy: false, hasDocument: true, status: "Difficulty loaded", scene: ${sceneWith({})} })`;
  evaluate(`document.querySelectorAll = selector => selector === "[data-ui-mount]:not([data-ui-ready])" ? [{}] : []`);
  evaluate(shown);
  assert.equal(get("status").textContent, "Difficulty loaded", "the bundle runs after app.js, so wait for load");
  evaluate(`document.readyState = "complete"`);
  evaluate(shown);
  assert.equal(get("status").textContent, "Part of the editor did not load");
  evaluate(`document.querySelectorAll = () => []`);
  evaluate(shown);
  assert.equal(get("status").textContent, "Difficulty loaded");
});

test("the Snap menu button beside the slider is only a ▾ named More snaps", () => {
  const button = ribbon.match(/<button id="grid-options"[^>]*>(.*?)<\/button>/)?.[0] ?? "";
  assert.match(button, /aria-label="More snaps"/);
  assert.match(button, /title="More snaps"/);
  assert.match(button, />▾<\/button>$/);
  assert.doesNotMatch(app, /\$\("grid-options"\)/, "show() no longer rewrites the button");
});

test("a fault reaches the Svelte parts as a new view with fault set, so they disable", async () => {
  const { evaluate } = await frontend();
  evaluate("window.dawsuUi = { update(view, scene) { this.views = (this.views ?? []).concat([view]); this.scene = scene; } }");
  evaluate(`show({ busy: false, hasDocument: true, tool: "Draw", scene: ${sceneWith({})} })`);
  evaluate("modal = () => {}");
  evaluate('show({ fault: "The audio device stopped" })');
  assert.equal(evaluate("window.dawsuUi.views.length"), 2);
  assert.equal(evaluate("window.dawsuUi.views[1].fault"), "The audio device stopped");
  assert.equal(evaluate("window.dawsuUi.views[1].tool"), "Draw");
  assert.equal(evaluate("window.dawsuUi.views[0] !== window.dawsuUi.views[1]"), true, "a new object, so Svelte sees the change");
  assert.equal(evaluate("window.dawsuUi.scene.divisor"), 4);
});
