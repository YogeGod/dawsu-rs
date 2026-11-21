import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
const enabled = { browser: true, inspector: true, controller: true, laneNames: true };

test("folder tree and Classic share the same mapset samples and preserve explicit assignment", async () => {
  const elements = new Map();
  const calls = [];
  function element() {
    return {
      listeners: new Map(), children: [], dataset: {}, style: {}, classList: { toggle() {} },
      addEventListener(name, listener) { this.listeners.set(name, listener); },
      setAttribute(name, value) { this[name] = value; },
      append(...children) {
        for (const child of children) {
          if (child.parentNode) child.parentNode.children = child.parentNode.children.filter(item => item !== child);
          child.parentNode = this;
          this.children.push(child);
        }
      },
      prepend() {},
      replaceChildren(...children) { this.children = []; this.append(...children); },
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  let classicBrowser = false;
  const context = createContext({
    document: {
      getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async (command, args) => {
        calls.push(JSON.parse(JSON.stringify(args?.action ?? null)));
        if (command === "editor_view" && args?.action?.kind === "browser_style") {
          classicBrowser = args.action.classic;
        }
        if (command === "list_library") {
          assert.equal(args.location.root, "C:/fixtures/library");
          return args.location.path === ""
            ? [["Drums", true]]
            : [["clap.wav", false], ["snare.wav", false]];
        }
        return {busy: false, activeLane: 0, hasDocument: true, panels: {...enabled},
          classicBrowser, hitsoundLibrary: "C:/fixtures/library"};
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
    console,
  });
  await runInContext(app, context);
  runInContext(`scene = {path: "copy.osu", lanes: [
    {key: "normal", sourceKey: "normal", name: "normal-hitnormal", canReplace: true},
    {key: "clap", sourceKey: "clap", name: "Snare", sample: "percussion/snare.wav"},
  ]};
  renderPanelLayout();`, context);
  const rows = get("sample-browser-entries");
  const click = (node) => node.listeners.get("click")();
  assert.deepEqual(rows.children.map(row => row.textContent), ["percussion"]);
  click(rows.children[0]);
  assert.deepEqual(rows.children.map(row => row.textContent), ["percussion", "snare.wav"]);
  click(rows.children[1]);
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), {kind: "lane", row: 1, operation: "preview_sample"});

  runInContext(`let controls;
    modal = (_kind, _title, body) => { controls = body; };
    labeled = (label, control) => ({label, control});`, context);
  runInContext("openViewSettings()", context);
  const style = runInContext('controls.find(item => item.label === "File browser style").control', context);
  style.value = "classic";
  style.listeners.get("change")();
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), {kind: "browser_style", classic: true});
  assert.deepEqual(rows.children.map(row => row.textContent), ["percussion"]);
  click(rows.children[0]);
  assert.deepEqual(rows.children.map(row => row.textContent), ["snare.wav"]);
  assert.equal(get("sample-browser-up").disabled, false);
  click(get("sample-browser-up"));
  assert.deepEqual(rows.children.map(row => row.textContent), ["percussion"]);
  click(rows.children[0]);
  click(rows.children[0]);
  await runInContext("pending", context);
  click(get("sample-browser-assign"));
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), {kind: "replace_sample", target: {map: "copy.osu", lane: "normal"},
    path: "percussion/snare.wav", mapset: true});

  const source = get("sample-browser-source");
  source.value = "library";
  source.listeners.get("change")();
  await runInContext("pending", context);
  assert.deepEqual(rows.children.map(row => row.textContent), ["Drums"]);
  click(rows.children[0]);
  await runInContext("pending", context);
  assert.deepEqual(rows.children.map(row => row.textContent), ["clap.wav", "snare.wav"]);
  const filter = get("sample-browser-filter");
  filter.value = "SNARE";
  filter.listeners.get("input")();
  assert.deepEqual(rows.children.map(row => row.textContent), ["snare.wav"]);
  click(rows.children[0]);
  await runInContext("pending", context);
  const location = {root: "C:/fixtures/library", path: "Drums/snare.wav"};
  assert.deepEqual(calls.at(-1), {kind: "preview_library_sample", location});
  click(get("sample-browser-assign"));
  await runInContext("pending", context);
  assert.deepEqual(calls.at(-1), {kind: "replace_library_sample", location,
    target: {map: "copy.osu", lane: "normal"}});
  runInContext('act({ kind: "browser_style", classic: false });', context);
  await runInContext("pending", context);
  assert.deepEqual(rows.children.map(row => row.textContent), ["Drums"]);
  click(rows.children[0]);
  assert.deepEqual(rows.children.map(row => row.textContent), ["Drums", "snare.wav"]);
});
