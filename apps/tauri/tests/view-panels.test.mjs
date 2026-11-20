import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
const style = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");

test("View switches hide panels and reclaim their timeline tracks, then restore them", async () => {
  const elements = new Map();
  const calls = [];
  let openDocument = false;
  const enabled = { inspector: true, controller: true, laneNames: true };
  const panels = { ...enabled };
  function element(id) {
    if (!elements.has(id)) {
      elements.set(id, {
        listeners: new Map(),
        addEventListener(name, callback) { this.listeners.set(name, callback); },
        append(...children) { this.children.push(...children); },
        prepend() {},
        setAttribute() {},
        replaceChildren(...children) { this.children = children; },
        children: [],
        style: { setProperty(name, value) { this[name] = value; } },
        classList: { add() {} },
        dataset: {},
      });
    }
    return elements.get(id);
  }
  const context = createContext({
    document: {
      getElementById: element,
      createElement: () => element(Symbol()),
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        if (args.action?.kind === "panel") {
          calls.push(args.action);
          const key = args.action.panel === "lane_names" ? "laneNames" : args.action.panel;
          panels[key] = args.action.on;
        }
        return { busy: false, hasDocument: openDocument, panels: { ...panels } };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await runInContext(app, context);
  runInContext([
    "let viewControls;",
    "modal = (_kind, _heading, elements) => { viewControls = elements; };",
    "labeled = (text, control) => ({ text, control });",
    "scene = {};",
  ].join("\n"), context);
  element("panel-view").listeners.get("click")();
  const controls = runInContext("viewControls", context);
  for (const [label, name, layout, track] of [
    ["Inspector", "inspector", "editor-dock", "inspector"],
    ["Volume controller", "controller", "editor-arrange", "controller"],
    ["Lane names", "lane_names", "editor-arrange", "laneNames"],
  ]) {
    const { control } = controls.find(item => item.text === label);
    assert.equal(control.checked, true);
    control.checked = false;
    control.listeners.get("change")();
    await runInContext("pending", context);
    assert.equal(calls.at(-1).kind, "panel");
    assert.equal(calls.at(-1).panel, name);
    assert.equal(calls.at(-1).on, false);
    assert.equal(element(layout).dataset[track], "false");
    control.checked = true;
    control.listeners.get("change")();
    await runInContext("pending", context);
    assert.equal(element(layout).dataset[track], "true");
  }
  runInContext('scene = { rowHeight: 28, lanes: [{ name: "soft-hitclap", index: 0 }, { name: "soft-hitclap2", index: 2 }] };', context);
  const info = controls.find(item => item.text === "Lane info (sample indexes)").control;
  info.checked = true;
  info.listeners.get("change")();
  assert.deepEqual(JSON.parse(runInContext("JSON.stringify(laneHeaders.rows.map(row => row.name))", context)),
    ["soft-hitclap · skin", "soft-hitclap2 · idx 2"]);
  info.checked = false;
  info.listeners.get("change")();
  assert.deepEqual(JSON.parse(runInContext("JSON.stringify(laneHeaders.rows.map(row => row.name))", context)),
    ["soft-hitclap", "soft-hitclap2"]);
  assert.match(style, /\.dock\[data-inspector="false"\]\{grid-template-columns:minmax\(0,1fr\)\}/);
  assert.match(style, /\.arrange\[data-controller="false"\]\{[^}]*grid-template-rows:var\(--layout-timeline-rows,minmax\(20px,max-content\) var\(--ruler-row\) minmax\(0,1fr\)\)\}/);
  assert.match(style, /\.arrange\[data-lane-names="false"\]\{--lane-default-width:99px\}/);
  assert.match(style, /--lanes:clamp\(99px,var\(--layout-lane-width,var\(--lane-default-width\)\),/);
  assert.deepEqual(panels, enabled);
  openDocument = true;
  runInContext('scene = { path: "Fixture.osu", lanes: [] }; renderSampleBrowser = () => {}; renderPanelLayout();', context);
  const laneNamesToggle = element("toggle-lane-names");
  assert.equal(laneNamesToggle.ariaExpanded, "true");
  assert.equal(laneNamesToggle.ariaLabel, "Hide lane names");
  laneNamesToggle.listeners.get("click")();
  await runInContext("pending", context);
  assert.equal(calls.at(-1).panel, "lane_names");
  assert.equal(calls.at(-1).on, false);
  assert.equal(element("editor-arrange").dataset.laneNames, "false");
  assert.equal(laneNamesToggle.ariaExpanded, "false");
  assert.equal(laneNamesToggle.ariaLabel, "Show lane names");
  laneNamesToggle.listeners.get("click")();
  await runInContext("pending", context);
  assert.equal(calls.at(-1).on, true);
  assert.equal(laneNamesToggle.ariaExpanded, "true");
  for (const [id, panel, shown, track] of [
    ["view-inspector", "inspector", true, "true"],
    ["view-browser", "browser", false, "off"],
  ]) {
    const toggle = element(id);
    assert.equal(toggle.ariaPressed, String(shown));
    toggle.listeners.get("click")();
    await runInContext("pending", context);
    assert.equal(calls.at(-1).panel, panel);
    assert.equal(calls.at(-1).on, !shown);
    assert.equal(toggle.ariaPressed, String(!shown));
    toggle.listeners.get("click")();
    await runInContext("pending", context);
    assert.equal(element("editor-dock").dataset[panel], track);
    assert.equal(toggle.ariaPressed, String(shown));
  }
});
