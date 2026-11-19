import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

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
  await runInContext(app, context);
  calls.length = 0;
  runInContext('view = { busy: false }; scene = { selection: { count: 2, start: 1250.5, object: "Circle" } }; renderSelection();', context);
  assert.equal(element("selected").textContent, "2 hits");
  assert.equal(element("selection-start-input").value, "00:01.251");
  assert.equal(element("selection-object").textContent, "Circle");

  element("volume-mode").value = "relative";
  element("volume").valueAsNumber = -15;
  element("selection-volume").listeners.get("submit")({ preventDefault() {} });
  await runInContext("pending", context);
  assert.equal(calls[0].kind, "volume");
  assert.equal(calls[0].relative, true);
  assert.equal(calls[0].value, -15);

  element("volume-mode").value = "absolute";
  element("volume").valueAsNumber = 0;
  element("selection-volume").listeners.get("submit")({ preventDefault() {} });
  await runInContext("pending", context);
  assert.equal(calls[1].relative, false);
  assert.equal(calls[1].value, 0);

  runInContext("scene = { selection: null }; renderSelection();", context);
  assert.equal(element("selected").textContent, "Nothing selected");
  assert.equal(element("selection-fields").hidden, true);
  assert.equal(element("slider-body-fields").hidden, true);

  runInContext('scene = { path: "Slider.osu", selection: { count: 1, start: 1000, object: "Slider head", sliderBody: { index: 2, set: "soft", whistleSet: "auto", whistle: false, volume: 60 } } }; renderSelection();', context);
  assert.equal(element("slider-body-fields").hidden, false);
  assert.equal(element("slider-body-volume").value, 60);
  assert.equal(element("slider-body-set").value, "soft");
  assert.equal(element("slider-body-whistle-set").value, "auto");
  element("slider-body-whistle-set").value = "normal";
  element("slider-body-set").value = "drum";
  element("slider-body-whistle").checked = true;
  element("slider-body-volume").valueAsNumber = 80;
  element("apply-slider-body").listeners.get("click")();
  await runInContext("pending", context);
  assert.deepEqual(JSON.parse(JSON.stringify(calls[2])), {
    kind: "slider_body", path: "Slider.osu", index: 2, set: "drum", whistle_set: "normal", whistle: true, volume: 80,
  });

  element("slider-body-whistle-set").value = "auto";
  element("apply-slider-body").listeners.get("click")();
  await runInContext("pending", context);
  assert.equal(calls.at(-1).whistle_set, "auto");

  runInContext('view.editDialog = "repeat"; scene.repeatGap = 3; ' +
    'modal = (_kind, title, content, buttons) => { globalThis.editModal = { title, content, buttons }; }; ' +
    'closeModal = () => {}; renderDialogs();', context);
  assert.equal(context.editModal.title, "Repeat");
  runInContext('view.editDialog = null; scene.path = "Selected.osu"; scene.selection = { count: 2, start: 1120, object: "2 moments" }; renderSelection();', context);
  assert.equal(element("selection-start-input").value, "00:01.120");
  element("selection-start-input").value = "1330.5";
  element("selection-start-edit").listeners.get("submit")({ preventDefault() {} });
  await runInContext("pending", context);
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), {
    kind: "timeline_edit", path: "Selected.osu", edit: { kind: "selection_start", position: 1330.5 },
  });
});
