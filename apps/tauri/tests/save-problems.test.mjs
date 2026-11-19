import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

function element(type = "div") {
  return {
    type, children: [], events: new Map(), dataset: {}, attributes: {}, style: { setProperty() {} }, ownText: "",
    get textContent() {
      return this.ownText + this.children.map(child => typeof child === "string" ? child : child.textContent ?? "").join("");
    },
    set textContent(value) { this.ownText = String(value); this.children = []; },
    addEventListener(name, listener) { this.events.set(name, listener); },
    append(...nodes) { this.children.push(...nodes); },
    prepend(...nodes) { this.children.unshift(...nodes); },
    replaceChildren(...nodes) { this.ownText = ""; this.children = nodes; },
    setAttribute(name, value) { this.attributes[name] = String(value); },
    getBoundingClientRect: () => ({ x: 0, top: 0, width: 1000 }),
    showModal() { this.open = true; },
    close() { this.open = false; },
    click() { return this.events.get("click")?.(); },
  };
}

async function load(problems, calls) {
  const elements = new Map();
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const context = createContext({
    problems,
    document: {
      getElementById: get,
      createElement: element,
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(args.action);
        return { busy: false, hasDocument: false, outputProblems: problems };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    setInterval() {},
    requestAnimationFrame() {},
  });
  await runInContext(app, context);
  calls.length = 0;
  return { context, get };
}

const twoProblems = () => [
  { target: "Hard.osu", timeline: "Hard.osu", time: 1000, message: "Cannot use this timing window." },
  { target: "Hard.osu", timeline: "Hard.osu", time: 2000, message: "This object cannot hold the change." },
];

test("Save review shows two timed problems and requires an explicit continuation", async () => {
  const calls = [];
  const { context } = await load(twoProblems(), calls);
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

test("Save hitsounds counts the problems and lists each with its time and a Show button", async () => {
  const calls = [];
  const { context, get } = await load(twoProblems(), calls);
  const render = () => runInContext("renderDialogs()", context);
  runInContext('scene = { path: "A different difficulty.osu" }; view = { saveProblemReview: true, outputProblems: problems };', context);
  render();
  const title = get("dialog-title");
  assert.equal(title.textContent, "Save hitsounds2 problems");
  assert.equal(title.children.filter(child => child.className === "problem-count").length, 1);
  render();
  assert.equal(title.children.filter(child => child.className === "problem-count").length, 1, "re-rendering keeps one chip");

  const list = get("dialog-body").children.find(child => child.className === "save-problem-list");
  assert.ok(list, "the dialog body holds a problem list");
  const rows = list.children;
  assert.equal(rows.length, 2);
  const cells = row => row.children.map(child => child.textContent);
  assert.deepEqual(cells(rows[0]), ["Cannot use this timing window.", "00:01.000", "Show"]);
  assert.deepEqual(cells(rows[1]), ["This object cannot hold the change.", "00:02.000", "Show"]);
  const actions = get("dialog-actions").children.map(button => button.textContent);
  assert.deepEqual(actions, ["Cancel", "Show first problem", "Continue to Save"]);

  await rows[1].children[2].click();
  await runInContext("pending", context);
  assert.equal(calls[0].kind, "dismiss_save_problem_review", "Show leaves the save review before jumping");
  assert.equal(title.textContent, "Problem 2 of 2");
});
