import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
const css = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");

test("workstation chrome uses the design's neutral grey surfaces and shared blue accent", () => {
  assert.match(css, /:root\s*\{[^}]*--base:\s*#151515\b/);
  assert.match(css, /:root\s*\{[^}]*--panel:\s*#1c1c1c\b/);
  assert.match(css, /:root\s*\{[^}]*--hi:\s*#77bdfa\b/);
});
test("the ruler paints a teal-tint and the overview a neutral background under the teal waveform frequency shades", async () => {
  const elements = new Map();
  function element() {
    const bands = [];
    const fills = [];
    const objects = [];
    const hitMarks = [];
    const ctx = new Proxy({
      arc(x, y, radius) { objects.push({ x, y, radius, afterWaveform: bands.length > 0 }); },
      fill() { fills.push(this.fillStyle); },
      fillRect(x, y, width, height) {
        fills.push(this.fillStyle);
        if (this.fillStyle === "#a3a3a3") hitMarks.push([x, y]);
        if (["#315c59", "#3c7673", "#46908c", "#51aaa5"].includes(this.fillStyle)) {
          bands.push({color: this.fillStyle, x, y, width, height});
        }
      },
    }, {get: (target, name) => target[name] ?? (() => {})});
    return {
      bands, fills, objects, hitMarks, getContext: () => ctx, clientWidth: 300, clientHeight: 80, scrollTop: 0,
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
  assert.equal(get("ruler").fills[0], "#1c2727");
  assert.equal(get("overview").fills[0], "#1c1c1c");
  assert.equal(get("grid").fills[0], "#151515");
  const waveformBands = { ruler: [...get("ruler").bands], overview: [...get("overview").bands] };
  runInContext(`
    scene.gameplay = {points: [["circle", 200], ["slider", 600]], spans: [["slider", 600, 800], ["spinner", 900, 1000]]};
    paint();`, context);
  assert.deepEqual(get("ruler").objects, [
    { x: 50, y: 50.5, radius: 9, afterWaveform: true },
    { x: 150, y: 50.5, radius: 9, afterWaveform: true },
  ]);
  assert.ok(get("ruler").fills.includes("#62d6cfd9"));
  assert.ok(get("ruler").fills.includes("#e6eef2"));
  assert.deepEqual(get("overview").objects, []);
  get("grid").fills.length = 0;
  runInContext(`
    scene.lanes = [{colourKey: "Skin|normal-hitclap"}, {colourKey: "Skin|normal-hitfinish"}];
    laneRows = [0, 1];
    hydrateLaneLayout({lane_colours: {"Skin|normal-hitclap": "#ff5500"}});
    paint();`, context);
  // v3: a picked colour tints its row 8 %.
  assert.deepEqual(get("grid").fills.slice(0, 3), ["#151515", "#ff550014", "#1c1c1c"]);
  get("grid").fills.length = 0;
  runInContext("hydrateLaneLayout(null); paint()", context);
  assert.ok(!get("grid").fills.includes("#ff55001f"));
  runInContext(`
    scene.lanes = [{group: "soft"}, {group: "normal"}, {group: "drum"}, {group: null}];
    scene.moments = [[200, 50, true, [[0], [1]]], [201, 50, false, [[1]]],
      [900, 50, true, [[2], [3]]], [1000, 50, true, []]];
    scene.zoom = 1;
    paint();`, context);
  const overview = get("overview");
  const marks = [[50, 3], [50, 17.8], [225, 32.6], [225, 47.4]];
  assert.deepEqual(overview.hitMarks.map(([x, y]) => [x, Math.round(y * 10) / 10]),
    marks);
  overview.hitMarks.length = 0;
  runInContext("scene.viewStart = 800; view.position = 900; paint()", context);
  assert.deepEqual(overview.hitMarks.map(([x]) => x), [50, 50, 225, 225]);
  overview.hitMarks.length = 0;
  runInContext("scene = {...scene, moments: [[900, 50, true, [[2]]]]}; paint()", context);
  assert.deepEqual(overview.hitMarks.map(([x]) => x), [225]);
  overview.hitMarks.length = 0;
  overview.clientWidth = 200;
  runInContext("paint()", context);
  assert.deepEqual(overview.hitMarks.map(([x]) => x), [150]);

  const expected = ["#3c7673", "#46908c", "#51aaa5"];
  for (const id of ["ruler", "overview"]) {
    for (const [segment, x] of [48, 148, 248].entries()) {
      const levels = waveformBands[id].filter(rect => rect.x === x);
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