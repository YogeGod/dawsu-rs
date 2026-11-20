import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const read = file => readFile(new URL("../dist/" + file, import.meta.url), "utf8");
const [html, css, app] = await Promise.all(["index.html", "style.css", "app.js"].map(read));

const ribbon = html.slice(html.indexOf('<div id="editor-ribbon"'), html.indexOf('<main id="editor-dock"'));
const statusbar = html.slice(html.indexOf('<div id="statusbar"'), html.indexOf('<dialog id="dialog"'));
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
  await runInContext(app, context);
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

test("the toolbar holds the tool pills, Snap, Audition, Follow, Return and the Hitsounds/Drums switch", () => {
  for (const id of ["grid-options", "follow", "transport-return", "samples-view", "drums-view"]) {
    assert.match(ribbon, new RegExp('id="' + id + '"'), id);
  }
  // The tool pills and the Audition pill are Svelte parts, each in its own ribbon group.
  for (const [group, part] of [["Tool", "tools"], ["Audition", "audition"], ["Grid", "snap"]]) {
    const start = ribbon.indexOf('aria-label="' + group + '"');
    const body = ribbon.slice(start, ribbon.indexOf('class="ribbon-group"', start));
    assert.match(body, new RegExp('data-ui-mount="' + part + '"'), group);
  }
  assert.doesNotMatch(ribbon, /id="tools"|id="toolbar-audition"/);
  const lanesGroup = ribbon.slice(ribbon.indexOf('aria-label="Lanes"'));
  const lanesPopover = lanesGroup.slice(0, lanesGroup.indexOf('class="ribbon-group"'));
  assert.doesNotMatch(lanesPopover, /id="samples-view"/, "the view switch sits on the toolbar, not in the Lanes pop-up");
  const transport = html.match(/<section id="editor-transport".*?<\/section>/s)?.[0] ?? "";
  assert.doesNotMatch(transport, /id="follow"|id="transport-return"/);
  assert.match(html, /<input[^>]*type="checkbox"[^>]*id="follow"/, "Follow stays a checkbox");
  assert.match(ribbon, /id="samples-view"[^>]*>Hitsounds<\/button>/, "the design names the sample-lane view Hitsounds");
  assert.match(ribbon, /id="drums-view"[^>]*>Drums<\/button>/);
  const revert = ribbon.match(/<button[^>]*data-command="revert"[^>]*>(.*?)<\/button>/s);
  assert.ok(revert, "Revert stays on the toolbar");
  assert.match(revert[0], /aria-label="Revert to saved…"/);
  assert.match(revert[1], /<svg[\s\S]*class="ribbon-secondary-label"/, "Revert can go icon-only");
});

test("the old tool buttons left style.css; Follow and Return keep their toggle look", () => {
  assert.ok(toolbarCss.length > 0, "toolbar CSS block");
  assert.doesNotMatch(css, /#tools\b|editor-tool-button|ribbon-tool-label/);
  assert.match(toolbarCss, /#editor-ribbon \.toolbar-toggle\s*\{[^}]*display:\s*inline-flex/);
  assert.match(toolbarCss, /#editor-ribbon \.ribbon-caption\s*\{[^}]*display:\s*none/);
  assert.doesNotMatch(toolbarCss, /:has\(|color-mix\(|aspect-ratio/);
  assert.match(toolbarCss, /\.toolbar-track[^{]*\{[^}]*position:\s*relative/);
  assert.match(toolbarCss, /@media\s*\(forced-colors:\s*active\)\s*\{[^{}]*aria-pressed=true[^{}]*\{[^}]*outline/);
});

test("toolIcons keeps all seven tools for the right-click picker, in the parts' order", async () => {
  const { TOOLS } = await import("../ui/src/lib/tools.js");
  const names = [...app.match(/const toolIcons = \{([^}]*)\}/)[1].matchAll(/(\w+):/g)].map(match => match[1]);
  assert.deepEqual(names, TOOLS);
  assert.equal(names.length, 7);
});

test("the status bar shows speeds first, then status and problems, with counts on the right", () => {
  const speeds = [...statusbar.matchAll(/data-playback-speed="([^"]+)"/g)].map(match => Number(match[1]));
  assert.deepEqual(speeds, [0.25, 0.5, 0.75, 1, 1.25, 1.5]);
  const order = ["data-playback-speed", 'id="status"', 'id="problem-summary"', 'id="hit-lane-counts"'].map(text => statusbar.indexOf(text));
  assert.ok(order.every(index => index >= 0), String(order));
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
  assert.equal([...html.matchAll(/data-playback-speed=/g)].length, 6, "speeds live only in the status bar");
});

test("show() fills the Snap label, Audition, the problem summary and the hit and lane counts", async () => {
  const { calls, evaluate, get, settle } = await frontend();
  evaluate(`show({ busy: false, hasDocument: true, audition: true, outputProblems: [{ time: 1 }, { time: 2 }],
    scene: ${sceneWith({})} })`);
  assert.equal(get("grid-options").textContent, undefined, "the Snap slider shows the snap; the menu button stays ▾");
  assert.equal(get("audition").checked, true, "the Inspector's Audition box follows the editor");
  assert.equal(get("problem-summary").hidden, false);
  assert.equal(get("problem-summary-text").textContent, "2 moments can't be saved");
  assert.equal(get("hit-lane-counts").hidden, false);
  assert.equal(get("hit-lane-counts").textContent, "3 hits · 2 lanes");
  assert.equal(get("playback-rates").hidden, false);

  get("audition").checked = false;
  get("audition").events.get("change")();
  await settle();
  assert.deepEqual(calls.at(-1), { kind: "audition", on: false });

  evaluate(`show({ busy: false, hasDocument: true, audition: false, outputProblems: [{ time: 1 }],
    scene: ${sceneWith({ snap: false, lanes: [{ key: "a" }], moments: [[100, 100, true, [[0, false]]]] })} })`);
  assert.equal(get("grid-options").textContent, undefined);
  assert.equal(get("audition").checked, false);
  assert.equal(get("problem-summary-text").textContent, "1 moment can't be saved");
  assert.equal(get("hit-lane-counts").textContent, "1 hit · 1 lane");

  evaluate("show({ busy: false, hasDocument: false, outputProblems: [] })");
  assert.equal(get("problem-summary").hidden, true);
  assert.equal(get("hit-lane-counts").hidden, true);
  assert.equal(get("playback-rates").hidden, true);
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
  assert.equal(get("hit-lane-counts").textContent, "0 hits · 2 lanes");
});

test("the status bar says when a toolbar part did not load, once the page has loaded", async () => {
  const { evaluate, get } = await frontend();
  const shown = `show({ busy: false, hasDocument: true, status: "Difficulty loaded", scene: ${sceneWith({})} })`;
  evaluate(`document.querySelectorAll = selector => selector === "[data-ui-mount]:not([data-ui-ready])" ? [{}] : []`);
  evaluate(shown);
  assert.equal(get("status").textContent, "Difficulty loaded", "the bundle runs after app.js, so wait for load");
  evaluate(`document.readyState = "complete"`);
  evaluate(shown);
  assert.equal(get("status").textContent, "Part of the toolbar did not load");
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
