import assert from "node:assert/strict";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const playheadColour = "#a1d0f7";

// The editor with fake canvases, a clock and animation frames the test runs by hand. The editor
// answers each refresh with `state`; the test changes it as playback goes on.
async function editor() {
  const elements = new Map();
  function element() {
    const calls = [];
    const ctx = new Proxy({}, {
      get(target, name) {
        if (name in target) return target[name];
        return (...args) => calls.push({ name, args, stroke: target.strokeStyle });
      },
    });
    return {
      calls, getContext: () => ctx, clientWidth: 300, showModal() { this.open = true; }, close() { this.open = false; }, clientHeight: 80, scrollTop: 0, listeners: {},
      dataset: {}, style: { setProperty() {} }, children: [], append() {}, prepend() {}, setAttribute() {},
      replaceChildren() {}, classList: { toggle() {}, add() {}, remove() {}, contains: () => false },
      addEventListener(name, listener) { (this.listeners[name] ??= []).push(listener); },
      getBoundingClientRect: () => ({ x: 0, y: 0, left: 0, top: 0, width: 300, height: 80 }),
    };
  }
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  let now = 0;
  let frames = [];
  let tick = null;
  const invoked = [];
  // `roundTrip`: how far the clock moves while the editor answers.
  const harness = { state: null, roundTrip: 0 };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { devicePixelRatio: 1, __TAURI__: {
      core: { invoke: async (name, args) => {
        invoked.push(name === "editor_view" ? args.action?.kind ?? "refresh" : name);
        now += harness.roundTrip;
        return name === "editor_view" ? { ...harness.state } : undefined;
      } },
      event: { listen() {} },
    } },
    performance: { now: () => now },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame(callback) { frames.push(callback); return frames.length; },
    cancelAnimationFrame() {},
    setInterval(callback) { tick = callback; },
    console,
  });
  bootEditor(context, { autoStart: false });
  get("volume-controller-panel").hidden = true;
  get("grid").clientWidth = 300;
  const evaluate = code => runInContext(code, context);
  evaluate(`scene = { path: "fixture.osu", viewStart: 0, zoom: .1, rowHeight: 30, moments: [], waveform: [],
    lanes: [], difficulties: [], sections: [[0, 500, 4]], gameplay: { points: [], spans: [] } };`);
  const base = { hasDocument: true, duration: 60000, left: 0, right: 0, follow: false, speed: 1 };
  return {
    invoked,
    evaluate,
    // A view from the editor arrives at time `at`.
    show(at, fields) {
      now = at;
      harness.state = { ...base, ...fields };
      evaluate(`show(${JSON.stringify(harness.state)})`);
    },
    // The next animation frame runs at time `at`; returns where the ruler's playhead line is.
    frame(at) {
      now = at;
      const ruler = get("ruler");
      ruler.calls.length = 0;
      const run = frames;
      frames = [];
      for (const callback of run) callback(at);
      const drawn = ruler.calls.filter(call => call.name === "moveTo" && call.stroke === playheadColour);
      return drawn.length ? Math.round((drawn.at(-1).args[0] - .5) * 100) / 100 : null;
    },
    roundTrip(ms) {
      harness.roundTrip = ms;
    },
    // What the editor answers next, without a view arriving now.
    answer(fields) {
      harness.state = { ...base, ...fields };
    },
    async tick(at, state) {
      now = at;
      if (state) harness.state = { ...base, ...state };
      await tick();
    },
  };
}

// At .1 px per ms with the view at 0, the line is at a tenth of the playhead time.
const x = ms => Math.round(ms * .1 * 100) / 100;

test("while playing, the editor is asked for its position on every tick of the 85 ms timer", async () => {
  const page = await editor();
  page.show(0, { playing: true, position: 1000 });
  // Timers fire a little early or late; the 85 ms gate used to skip every other tick.
  for (const at of [84, 168, 253, 337, 421]) await page.tick(at, { playing: true, position: 1000 + at });
  assert.equal(page.invoked.filter(kind => kind === "refresh").length, 5);
});

test("while stopped, the editor is still asked only about once a second", async () => {
  const page = await editor();
  page.show(0, { playing: false, position: 1000 });
  for (const at of [85, 170, 255, 340, 425, 510, 595, 680, 765, 850, 935, 1020]) await page.tick(at);
  assert.equal(page.invoked.filter(kind => kind === "refresh").length, 1);
});

test("between the editor's views the playhead line moves on every frame at the playback speed", async () => {
  const page = await editor();
  page.show(0, { playing: true, position: 1000 });
  assert.equal(page.frame(16), x(1016));
  assert.equal(page.frame(33), x(1033));
  assert.equal(page.frame(50), x(1050));
  // At half speed the song moves half as far.
  page.show(100, { playing: true, position: 2000, speed: .5 });
  assert.equal(page.frame(140), x(2020));
});

test("a stopped playhead stays where the editor put it", async () => {
  const page = await editor();
  page.show(0, { playing: false, position: 1500 });
  assert.equal(page.frame(16), x(1500));
  assert.equal(page.frame(500), null, "no frames are asked for while stopped");
});

test("a report a few ms behind the drawn line doesn't pull it back; a seek moves it at once", async () => {
  const page = await editor();
  page.show(0, { playing: true, position: 1000 });
  assert.equal(page.frame(85), x(1085));
  // The audio clock reports in buffer-sized steps: 8 ms behind the line, so the line holds.
  page.show(85, { playing: true, position: 1077 });
  assert.equal(page.frame(101), x(1101));
  // A seek back to 500 ms while playing.
  page.show(120, { playing: true, position: 500 });
  assert.equal(page.frame(136), x(516));
});

test("without a new view the line stops 250 ms past the last report instead of running on", async () => {
  const page = await editor();
  page.show(0, { playing: true, position: 1000 });
  assert.equal(page.frame(2000), x(1250));
});

test("while cycling the line waits at the right locator for the editor to wrap", async () => {
  const page = await editor();
  page.show(0, { playing: true, position: 1950, cycle: true, left: 1000, right: 2000 });
  assert.equal(page.frame(40), x(1990));
  assert.equal(page.frame(80), x(2000));
  // The editor wraps to the left locator.
  page.show(90, { playing: true, position: 1010, cycle: true, left: 1000, right: 2000 });
  assert.equal(page.frame(106), x(1026));
});

test("Centre scrolls the view on every frame and tells the editor once per view, not per frame", async () => {
  const page = await editor();
  page.evaluate("window.dawsuUi = { centre: () => true, update() {}, snapGroup() {} }");
  page.show(0, { playing: true, position: 10000, follow: true });
  const before = page.invoked.filter(kind => kind === "view").length;
  // 300 px at .1 px per ms shows 3000 ms: the line sits in the middle, the view moves under it.
  for (const at of [16, 33, 50, 66]) {
    assert.equal(page.frame(at), 150);
    assert.equal(page.evaluate("scene.viewStart"), 10000 + at - 1500);
  }
  assert.equal(page.invoked.filter(kind => kind === "view").length, before, "no request per frame");
});

test("zooming while Centre follows keeps the new zoom: a view arriving in between doesn't put the old one back", async () => {
  const page = await editor();
  page.evaluate("window.dawsuUi = { centre: () => true, update() {}, snapGroup() {} }");
  page.show(0, { playing: true, position: 10000, follow: true });
  await page.evaluate("pending");
  // A refresh is on its way when the wheel zooms in; the refresh's view arrives first and
  // re-centres, then the zoom is applied.
  const refresh = page.tick(85, { playing: true, position: 10085, follow: true });
  page.evaluate("changeView(scene.viewStart, .2)");
  await refresh;
  await page.evaluate("pending");
  await page.evaluate("pending");
  assert.equal(page.evaluate("scene.zoom"), .2);
  // 300 px at .2 px per ms shows 1500 ms: still centred on the playhead, at the new zoom.
  assert.equal(page.frame(100), 150);
});

test("a fault while playing stops the frames", async () => {
  const page = await editor();
  page.show(0, { playing: true, position: 1000 });
  page.frame(16);
  page.evaluate('show({ fault: "boom" })');
  page.frame(33);
  page.frame(50);
  assert.equal(page.frame(66), null, "no frame paints behind the Editor stopped dialog");
});

test("the hold lets go: a line held once is pulled back to the editor's report next time", async () => {
  const page = await editor();
  page.show(0, { playing: true, position: 1000 });
  page.frame(85);
  // 15 ms behind: held once.
  page.show(85, { playing: true, position: 1070 });
  assert.equal(page.frame(85), x(1085));
  // Still 15 ms behind on the next view: the line goes to the report.
  page.show(170, { playing: true, position: 1155 });
  assert.equal(page.frame(170), x(1155));
});

test("Follow's page turn is asked once, not again on every frame while it waits its turn", async () => {
  const page = await editor();
  page.show(0, { playing: true, position: 2990, follow: true });
  // Hold the queue: frames run while no view request has had its turn.
  page.evaluate("pending = new Promise(done => { globalThis.release = done; })");
  for (const at of [16, 33, 50, 66, 83]) page.frame(at);
  page.answer({ playing: true, position: 3073, follow: true });
  page.evaluate("release()");
  await page.evaluate("pending");
  await page.evaluate("pending");
  assert.deepEqual(page.invoked.filter(kind => kind === "view"), ["view"], JSON.stringify(page.invoked));
});

test("in Centre a full scene from the editor keeps the view the frames scrolled to, and stopping tells the editor", async () => {
  const page = await editor();
  page.evaluate("window.dawsuUi = { centre: () => true, update() {}, snapGroup() {}, lanes() {}, mixer() {}, inspector() {} }");
  page.show(0, { playing: true, position: 10000, follow: true });
  await page.evaluate("pending");
  page.frame(60);
  assert.equal(page.evaluate("scene.viewStart"), 8560);
  // A full scene carries the editor's older start.
  page.show(70, { playing: true, position: 10070, follow: true, scene: { path: "fixture.osu", viewStart: 8500, zoom: .1,
    rowHeight: 30, moments: [], waveform: [], lanes: [], difficulties: [], sections: [[0, 500, 4]], gameplay: { points: [], spans: [] } } });
  assert.equal(page.evaluate("scene.viewStart"), 8560);
  page.frame(80);
  await page.evaluate("pending");
  await page.evaluate("pending");
  const sent = page.invoked.filter(kind => kind === "view").length;
  // Stop: the editor hears where the view ended up.
  page.show(90, { playing: false, position: 10090, follow: true });
  page.frame(100);
  await page.evaluate("pending");
  await page.evaluate("pending");
  assert.equal(page.invoked.filter(kind => kind === "view").length, sent + 1);
});

test("at high zoom, Centre's own answer doesn't ask the editor again at once", async () => {
  const page = await editor();
  page.evaluate("window.dawsuUi = { centre: () => true, update() {}, snapGroup() {} }");
  page.evaluate("scene.zoom = 3");
  page.roundTrip(5);
  page.show(0, { playing: true, position: 10000, follow: true });
  for (let i = 0; i < 10; i++) await page.evaluate("pending");
  assert.equal(page.invoked.filter(kind => kind === "view").length, 1);
});
