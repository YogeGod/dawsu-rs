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

test("the toolbar holds the tool pills, Snap, Audition, Follow, Return and the Samples/Drums switch", () => {
  for (const id of ["tools", "grid-options", "toolbar-audition", "follow", "transport-return", "samples-view", "drums-view"]) {
    assert.match(ribbon, new RegExp('id="' + id + '"'), id);
  }
  const lanesGroup = ribbon.slice(ribbon.indexOf('aria-label="Lanes"'));
  const lanesPopover = lanesGroup.slice(0, lanesGroup.indexOf('class="ribbon-group"'));
  assert.doesNotMatch(lanesPopover, /id="samples-view"/, "the view switch sits on the toolbar, not in the Lanes pop-up");
  const transport = html.match(/<section id="editor-transport".*?<\/section>/s)?.[0] ?? "";
  assert.doesNotMatch(transport, /id="follow"|id="transport-return"/);
  assert.match(html, /<input[^>]*type="checkbox"[^>]*id="follow"/, "Follow stays a checkbox");
});

test("tool pills put the label beside the icon, with no :has() or color-mix()", () => {
  assert.ok(toolbarCss.length > 0, "toolbar CSS block");
  assert.match(toolbarCss, /#editor-ribbon #tools button[^{]*\{[^}]*display:\s*inline-flex/);
  assert.match(toolbarCss, /#editor-ribbon \.ribbon-caption\s*\{[^}]*display:\s*none/);
  assert.doesNotMatch(toolbarCss, /:has\(|color-mix\(|aspect-ratio/);
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
  assert.equal(get("grid-options").textContent, "Snap 1/4 ▾");
  assert.equal(get("toolbar-audition").ariaPressed, "true");
  assert.equal(get("problem-summary").hidden, false);
  assert.equal(get("problem-summary-text").textContent, "2 moments can't be saved");
  assert.equal(get("hit-lane-counts").hidden, false);
  assert.equal(get("hit-lane-counts").textContent, "3 hits · 2 lanes");
  assert.equal(get("playback-rates").hidden, false);

  get("toolbar-audition").events.get("click")();
  await settle();
  assert.deepEqual(calls.at(-1), { kind: "audition", on: false });

  evaluate(`show({ busy: false, hasDocument: true, audition: false, outputProblems: [{ time: 1 }],
    scene: ${sceneWith({ snap: false, lanes: [{ key: "a" }], moments: [[100, 100, true, [[0, false]]]] })} })`);
  assert.equal(get("grid-options").textContent, "Snap off ▾");
  assert.equal(get("toolbar-audition").ariaPressed, "false");
  assert.equal(get("problem-summary-text").textContent, "1 moment can't be saved");
  assert.equal(get("hit-lane-counts").textContent, "1 hit · 1 lane");

  evaluate("show({ busy: false, hasDocument: false, outputProblems: [] })");
  assert.equal(get("problem-summary").hidden, true);
  assert.equal(get("hit-lane-counts").hidden, true);
  assert.equal(get("playback-rates").hidden, true);
});
