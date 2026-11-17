import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

test("Save review shows two timed problems and requires an explicit continuation", async () => {
  const calls = [];
  const element = { addEventListener() {}, append() {}, prepend() {}, children: [] };
  const problems = [
    { target: "Hard.osu", timeline: "Hard.osu", time: 1000, message: "Cannot use this timing window." },
    { target: "Hard.osu", timeline: "Hard.osu", time: 2000, message: "This object cannot hold the change." },
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
      core: { invoke: async (_name, args) => {
        calls.push(args.action);
        return { busy: false, hasDocument: false };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await runInContext(app, context);
  calls.length = 0;
  runInContext([
    "modal = (kind, title, content, buttons) => { globalThis.output = { kind, title, text: content.map(node => node.textContent), buttons }; };",
    "closeModal = () => {};",
    "view = { saveProblemReview: true, outputProblems: problems }; renderDialogs();",
  ].join("\n"), context);
  assert.equal(context.output?.kind, "save-problems");
  assert.match(context.output.text.join(" "), /00:01\.000.*00:02\.000/);
  const saveReview = context.output;
  await runInContext('scene = { path: "A different difficulty.osu" }; showOutputProblem(0);', context);
  assert.equal(context.output.title, "Problem 1 of 2");
  await context.output.buttons.find(([label]) => label === "Next problem")[1]();
  assert.equal(context.output.title, "Problem 2 of 2");
  assert.match(context.output.text[0], /00:02\.000.*Hard\.osu/);
  context.output = saveReview;
  assert.equal(calls.length, 0);
  context.output.buttons.find(([label]) => label === "Continue to Save")[1]();
  await runInContext("pending", context);
  assert.equal(calls[0].kind, "save");
});
