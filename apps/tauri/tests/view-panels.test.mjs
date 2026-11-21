import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { dockingRules } from "./docking-rules.mjs";
import { bootEditor } from "./editor-runtime.mjs";

const style = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");

test("View switches fold and open panels and reclaim their timeline tracks, then restore them", async () => {
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
    window: { editorDocking: dockingRules, __TAURI__: {
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
  await bootEditor(context);
  runInContext([
    "let viewControls;",
    "modal = (_kind, _heading, elements) => { viewControls = elements; };",
    "labeled = (text, control) => ({ text, control });",
    "scene = {};",
  ].join("\n"), context);
  // The View settings dialog opens only with a map, and each change keeps it open.
  openDocument = true;
  runInContext("openViewSettings()", context);
  const controls = runInContext("viewControls", context);
  for (const [label, name, layout, track] of [
    ["Inspector", "inspector", "view-inspector", "ariaPressed"],
    ["Volume and Mixer", "controller", "editor-arrange", "controller"],
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
    const shown = target => track === "ariaPressed" ? element(layout).ariaPressed : element(layout).dataset[track];
    assert.equal(shown(), "false");
    control.checked = true;
    control.listeners.get("change")();
    await runInContext("pending", context);
    assert.equal(shown(), "true");
  }
  runInContext('scene = { rowHeight: 28, lanes: [{ name: "soft-hitclap", index: 0 }, { name: "soft-hitclap2", index: 2 }] };', context);
  const info = controls.find(item => item.text === "Lane info (sample indexes)").control;
  info.checked = true;
  info.listeners.get("change")();
  assert.deepEqual(JSON.parse(runInContext("JSON.stringify(laneHeaders.rows.map(row => row.name))", context)),
    ["soft-hitclap (skin)", "soft-hitclap2 (idx 2)"]);
  info.checked = false;
  info.listeners.get("change")();
  assert.deepEqual(JSON.parse(runInContext("JSON.stringify(laneHeaders.rows.map(row => row.name))", context)),
    ["soft-hitclap", "soft-hitclap2"]);
  // The frame folds panels in docking.js; the old .dock[data-…] columns are gone.
  assert.doesNotMatch(style, /\.dock\[data-(inspector|browser)/);
  assert.match(style, /\.arrange\[data-controller="false"\]\{[^}]*grid-template-rows:var\(--layout-timeline-rows,var\(--ruler-row\) minmax\(0,1fr\) minmax\(20px,max-content\)\)\}/);
  assert.match(style, /\.arrange\[data-lane-names="false"\]\{--lane-default-width:99px\}/);
  assert.match(style, /--lanes:clamp\(99px,var\(--layout-lane-width,var\(--lane-default-width\)\),/);
  assert.deepEqual(panels, enabled);
  openDocument = true;
  runInContext('scene = { path: "Fixture.osu", lanes: [] }; renderSampleBrowser = () => {}; renderPanelLayout();', context);
  // Lane names switch from View › Lane names and View settings only (v3: no toolbar button).
  runInContext('act({ kind: "panel", panel: "lane_names", on: false })', context);
  await runInContext("pending", context);
  assert.equal(calls.at(-1).panel, "lane_names");
  assert.equal(element("editor-arrange").dataset.laneNames, "false");
  runInContext('act({ kind: "panel", panel: "lane_names", on: true })', context);
  await runInContext("pending", context);
  assert.equal(element("editor-arrange").dataset.laneNames, "true");
  for (const [id, panel, shown] of [
    ["view-inspector", "inspector", true],
    ["view-browser", "browser", false],
    ["view-controller", "controller", true],
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
    assert.equal(toggle.ariaPressed, String(shown));
  }
});
