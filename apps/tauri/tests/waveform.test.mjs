import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
test("the editor paints bass, middle and treble colors in ruler and overview canvases", async () => {
  const elements = new Map();
  function element() {
    const bands = [];
    const ctx = new Proxy({
      fillRect(x, y, width, height) {
        if (["#334a63", "#47618b", "#657ab5", "#7aaac6"].includes(this.fillStyle)) {
          bands.push({color: this.fillStyle, x, y, width, height});
        }
      },
    }, {get: (target, name) => target[name] ?? (() => {})});
    return {
      bands, getContext: () => ctx, clientWidth: 300, clientHeight: 80, scrollTop: 0,
      dataset: {}, style: {}, children: [], addEventListener() {}, append() {}, prepend() {},
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const context = createContext({
    document: {getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {}},
    window: {devicePixelRatio: 1, __TAURI__: {
      core: {invoke: async () => ({hasDocument: false, busy: false})},
      event: {listen() {}},
    }},
    ResizeObserver: class {observe() {}}, requestAnimationFrame() {}, setInterval() {},
    console,
  });
  await runInContext(app.replace(/\nact\(null\);\s*$/, ""), context);
  get("volume-controller-panel").hidden = true;
  runInContext(`
    view = {hasDocument: true, duration: 1200, position: 0, left: 0, right: 1200};
    scene = {path: "fixture.osu", viewStart: 0, zoom: .25, rowHeight: 30, moments: [], sections: [],
      waveform: [[.4,.38,.12,.02], [.6,.18,.54,.17], [.8,.025,.19,.68]]};
    paint();`, context);
  const expected = ["#47618b", "#657ab5", "#7aaac6"];
  for (const id of ["ruler", "overview"]) {
    for (const [segment, x] of [48, 148, 248].entries()) {
      const levels = get(id).bands.filter(rect => rect.x === x);
      assert.equal(levels.length, 4);
      const ordered = levels.slice(1).sort((a, b) => b.height - a.height);
      assert.equal(ordered[0].color, expected[segment]);
      assert.ok(ordered[0].height > ordered[1].height * 2);
      const center = ordered[0].y + ordered[0].height / 2;
      const paintedColor = levels.filter(rect =>
        center - ordered[0].height * .45 >= rect.y &&
        center - ordered[0].height * .45 < rect.y + rect.height).at(-1).color;
      assert.equal(paintedColor, expected[segment]);
    }
  }
});