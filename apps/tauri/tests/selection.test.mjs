import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";


test("selection card shows its first hit and distinguishes relative and absolute volume", async () => {
  const elements = new Map();
  const calls = [];
  function element(id) {
    if (!elements.has(id)) {
      const listeners = new Map();
      elements.set(id, {
        addEventListener: (name, fn) => listeners.set(name, fn),
        append() {}, prepend() {}, setAttribute() {}, blur() {}, children: [], dataset: {}, listeners,
      });
    }
    return elements.get(id);
  }
  const context = createContext({
    document: {
      getElementById: element,
      createElement: () => ({ ...element("created") }),
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(args.action);
        return { busy: false, hasDocument: true, position: 0 };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  calls.length = 0;
  const model = () => JSON.parse(runInContext("JSON.stringify(inspectorModel())", context));
  runInContext('view = { busy: false }; scene = { selection: { count: 2, start: 1250.5, object: "Circle" } };', context);
  assert.equal(model().selection.summary, "2 hits selected");
  assert.equal(model().selection.start, "00:01.251");
  assert.equal(model().selection.object, "Circle");

  runInContext("inspectorAction('volume', -15, true)", context);
  await runInContext("pending", context);
  assert.equal(calls[0].kind, "volume");
  assert.equal(calls[0].relative, true);
  assert.equal(calls[0].value, -15);

  runInContext("inspectorAction('volume', 0, false)", context);
  await runInContext("pending", context);
  assert.equal(calls[1].relative, false);
  assert.equal(calls[1].value, 0);

  runInContext("scene = { selection: null };", context);
  assert.equal(model().selection, null);

  runInContext('scene = { path: "Slider.osu", selection: { count: 1, start: 1000, object: "Slider head", sliderBody: { index: 2, set: "soft", whistleSet: "auto", whistle: false, volume: 60 } } };', context);
  const body = model().selection.sliderBody;
  assert.equal(body.volume, 60);
  assert.equal(body.set, "soft");
  assert.equal(body.whistleSet, "auto");
  runInContext('inspectorAction("sliderBody", { set: "drum", whistleSet: "normal", whistle: true, volume: 80 })', context);
  await runInContext("pending", context);
  assert.deepEqual(JSON.parse(JSON.stringify(calls[2])), {
    kind: "slider_body", path: "Slider.osu", index: 2, set: "drum", whistle_set: "normal", whistle: true, volume: 80,
  });

  runInContext('inspectorAction("sliderBody", { set: "drum", whistleSet: "auto", whistle: true, volume: 80 })', context);
  await runInContext("pending", context);
  assert.equal(calls.at(-1).whistle_set, "auto");

  runInContext('view.editDialog = "repeat"; scene.repeatGap = 3; ' +
    'modal = (_kind, title, content, buttons) => { globalThis.editModal = { title, content, buttons }; }; ' +
    'closeModal = () => {}; renderDialogs();', context);
  assert.equal(context.editModal.title, "Repeat");
  // Edit › Pattern stamp…: Rust suggests the step; Stamp sends the four choices as one action.
  const plainCreate = context.document.createElement;
  context.document.createElement = () => ({ children: [], dataset: {}, addEventListener() {}, setAttribute() {},
    append(...nodes) { this.children.push(...nodes); } });
  runInContext('view.editDialog = "pattern_stamp"; scene.patternStampStep = 500; renderDialogs();', context);
  context.document.createElement = plainCreate;
  assert.equal(context.editModal.title, "Pattern stamp");
  const fields = context.editModal.content.filter(node => node.children?.length === 2).map(node => node.children[1]);
  assert.deepEqual(fields.map(field => field.value), [4, 500, "", ""]);
  assert.equal(fields[2].checked, true, "Snap to gameplay objects starts on");
  Object.assign(fields[0], { valueAsNumber: 8 });
  Object.assign(fields[1], { valueAsNumber: 250 });
  fields[3].checked = true;
  context.editModal.buttons.find(([label]) => label === "Stamp")[1]();
  await runInContext("pending", context);
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), {
    kind: "pattern_stamp", count: 8, step_ms: 250, snap_to_objects: true, quantize_volumes: true,
  });
  runInContext('view.editDialog = null; scene.path = "Selected.osu"; scene.selection = { count: 2, start: 1120, object: "2 moments" };', context);
  assert.equal(model().selection.start, "00:01.120");
  runInContext('inspectorAction("moveStart", "1330.5")', context);
  await runInContext("pending", context);
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), {
    kind: "timeline_edit", path: "Selected.osu", edit: { kind: "selection_start", position: 1330.5 },
  });
});
