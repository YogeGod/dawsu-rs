import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";


test("ruler markers merge nearby visible problems from the active difficulty", async () => {
  const canvas = new Proxy({}, { get: (target, name) => target[name] ?? (() => {}) });
  const element = {
    addEventListener() {}, append() {}, prepend() {}, children: [], style: {}, scrollTop: 0,
    clientWidth: 300, clientHeight: 80, getContext: () => canvas,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 300, height: 80 }),
  };
  const problems = [
    { timeline: "Hard.osu", target: "Hard.osu", time: 1000, message: "First" },
    { timeline: "Easy.osu", target: "Easy.osu", time: 1020, message: "Different difficulty" },
    { timeline: "Hard.osu", target: "Hard.osu", time: 1050, message: "Second" },
    { timeline: "Hard.osu", target: "Hard.osu", time: 3000, message: "Third" },
    { timeline: "Hard.osu", target: "Hard.osu", time: 9000, message: "Outside view" },
  ];
  const context = createContext({
    problems,
    document: {
      getElementById: () => element,
      createElement: () => ({ ...element }),
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async () => ({ busy: false, hasDocument: false }) },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  bootEditor(context, { autoStart: false });
  const markers = runInContext('visibleWarningMarkers(problems, "Hard.osu", 500, .1, 300)', context);
  assert.deepEqual(JSON.parse(JSON.stringify(markers.map(marker => ({
    x: marker.x, messages: marker.problems.map(problem => problem.message),
  })))), [
    { x: 50, messages: ["First", "Second"] },
    { x: 250, messages: ["Third"] },
  ]);

  const title = runInContext(`
    scene = {path:"Hard.osu", viewStart:0, zoom:1, rowHeight:30, moments:[], sections:[], waveform:[]};
    view = {duration:3000, position:0, left:0, right:3000, outputProblems:[
      {timeline:"Hard.osu", target:"Hard.osu", time:100, message:"Zoom warning"},
      {timeline:"Hard.osu", target:"Hard.osu", time:110, message:"Neighbor warning"},
    ]};
    paint();
    trackRulerPointer({clientX:100, clientY:74});
    ruler.title;`, context);
  assert.match(title, /Zoom warning/);
  assert.equal(runInContext("scene.viewStart = 60; paint(); ruler.title", context), "");
  assert.match(runInContext("scene.viewStart = 0; paint(); ruler.title", context), /Zoom warning/);
  assert.match(runInContext("trackRulerPointer({clientX:106, clientY:74}); ruler.title", context),
    /Neighbor warning/);
});
