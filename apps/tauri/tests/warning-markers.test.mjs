import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("ruler markers merge nearby visible problems from the active difficulty", async () => {
  const element = { addEventListener() {}, append() {}, prepend() {}, children: [] };
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
  await runInContext(app, context);
  const markers = runInContext('visibleWarningMarkers(problems, "Hard.osu", 500, .1, 300)', context);
  assert.deepEqual(JSON.parse(JSON.stringify(markers.map(marker => ({
    x: marker.x, messages: marker.problems.map(problem => problem.message),
  })))), [
    { x: 50, messages: ["First", "Second"] },
    { x: 250, messages: ["Third"] },
  ]);
});
