import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";

const css = await readFile(new URL("../ui/src/legacy-chrome.css", import.meta.url), "utf8");

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
  await bootEditor(context);
  calls.length = 0;
  return { context, get };
}

const twoProblems = () => [
  { target: "Hard.osu", timeline: "Hard.osu", time: 1000, message: "Cannot use this timing window." },
  { target: "Hard.osu", timeline: "Hard.osu", time: 2000, message: "This object cannot hold the change." },
];

test("Save review shows two timed problems before Review Save, and its Save writes the reviewed plan", async () => {
  const calls = [];
  const { context } = await load(twoProblems(), calls);
  runInContext([
    "modal = (kind, title, content, buttons) => { globalThis.output = { kind, title, text: content.map(node => node.textContent), buttons }; };",
    "closeModal = () => {};",
    'view = { saveProblemReview: true, saveReview: [["Replace difficulty", "Hard.osu"]], outputProblems: problems }; renderDialogs();',
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
  context.output.buttons.find(([label]) => label === "Save")[1]();
  await runInContext("pending", context);
  assert.deepEqual({ ...calls[0] }, { kind: "answer_save_review", apply: true });
});

test("Save hitsounds says the rest is written and how left-out moments come back", async () => {
  const calls = [];
  const { context } = await load(twoProblems(), calls);
  runInContext([
    "modal = (kind, title, content, buttons) => { globalThis.output = { kind, title, text: content.map(node => node.textContent), buttons }; };",
    'view = { saveProblemReview: true, saveReview: [], outputProblems: problems }; renderDialogs();',
  ].join("\n"), context);
  const note = context.output.text[1];
  assert.equal(note, "These parts won't sound in osu! the way they do here. You can still save; the rest is written as drawn. " +
    "Left-out moments stay in dawsu! and on this list, and Revert to saved brings them back.");
});

test("without a project Save hitsounds keeps its old copy and continues to Save", async () => {
  const calls = [];
  const { context, get } = await load(twoProblems(), calls);
  runInContext('scene = { path: "Hard.osu" }; view = { saveProblemReview: true, outputProblems: problems }; renderDialogs();', context);
  assert.match(get("dialog-body").children[1].textContent, /Save writes nothing until they are fixed/);
  const actions = get("dialog-actions").children;
  assert.deepEqual(actions.map(button => button.textContent), ["Cancel", "Show first problem", "Continue to Save"]);
  await actions[2].click();
  await runInContext("pending", context);
  assert.equal(calls[0].kind, "save");
});

test("Save hitsounds counts the problems and lists each with its time and a Show button", async () => {
  const calls = [];
  const problems = [
    ...twoProblems(),
    { target: "Insane.osu", timeline: "Insane.osu", time: 3000, message: "Another lookup overlaps." },
  ];
  const { context, get } = await load(problems, calls);
  const render = () => runInContext("renderDialogs()", context);
  runInContext('scene = { path: "Hard.osu" }; view = { saveProblemReview: true, saveReview: [], outputProblems: problems };', context);
  render();
  const title = get("dialog-title");
  assert.equal(title.textContent, "Save hitsounds3 problems");
  assert.equal(title.children.filter(child => child.className === "problem-count").length, 1);
  render();
  assert.equal(title.children.filter(child => child.className === "problem-count").length, 1, "re-rendering keeps one chip");
  assert.match(get("dialog").className, /\bdialog-save-problems\b/, "the dialog gets its width from a class, not :has()");

  const body = get("dialog-body").children;
  assert.equal(body.find(child => child.className === "save-problem-difficulty").textContent, "DifficultyHard.osu");
  const list = body.find(child => child.className === "save-problem-list");
  assert.ok(list, "the dialog body holds a problem list");
  const rows = list.children;
  assert.equal(rows.length, 3);
  const cells = row => row.children.map(child => child.textContent);
  assert.deepEqual(cells(rows[0]), ["Cannot use this timing window.", "00:01.000", "Show"]);
  assert.deepEqual(cells(rows[1]), ["This object cannot hold the change.", "00:02.000", "Show"]);
  const other = rows[2].children[0].children.at(-1);
  assert.equal(other?.className, "save-problem-target", "a problem in another difficulty names it on its row");
  assert.equal(other.textContent, "Insane.osu");
  assert.equal(rows[0].children[0].children.length, 0, "rows in the open difficulty don't repeat its name");
  const buttons = get("dialog-actions").children;
  assert.deepEqual(buttons.map(button => button.textContent), ["Show first problem", "Cancel", "Save"]);
  assert.equal(buttons[0].className, "save-problems-show", "Show first problem sits on the left");
  assert.equal(buttons[2].className, "primary");

  await rows[2].children[2].click();
  await runInContext("pending", context);
  assert.equal(calls[0].kind, "dismiss_save_problem_review", "Show leaves the save review before jumping");
  assert.equal(title.textContent, "Problem 3 of 3");
  assert.doesNotMatch(get("dialog").className, /\bdialog-save-problems\b/);
});

test("the Save hitsounds styles avoid :has() and color-mix(), which macOS and Linux WebViews may lack", () => {
  const block = css.slice(css.indexOf("/* Save with problems (design f8)"));
  assert.match(block, /#dialog\.dialog-save-problems\{/);
  assert.match(block, /#dialog-actions \.save-problems-show\{margin-right:auto\}/);
  assert.doesNotMatch(block, /:has\(|color-mix\(/);
});

test("Save and quit waits through the Save hitsounds summary and quits after its Save", async () => {
  const listeners = new Map();
  let tick = null;
  let clock = 0;
  let state = { busy: false, hasDocument: false, dirty: true };
  const invoked = [];
  const elements = new Map();
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        invoked.push(name === "editor_view" ? args.action?.kind ?? "refresh" : name);
        return name === "editor_view" ? { ...state } : undefined;
      } },
      event: { listen(name, handler) { listeners.set(name, handler); } },
    } },
    performance: { now: () => (clock += 5000) },
    ResizeObserver: class { observe() {} },
    setInterval(callback) { tick = callback; },
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  runInContext("modal = (kind, title, content, buttons) => { globalThis.output = { kind, buttons }; }; closeModal = () => {};", context);
  listeners.get("editor-close-requested")({ payload: "The difficulty has unsaved changes." });
  state = { busy: true, hasDocument: false, dirty: true };
  await context.output.buttons.find(([label]) => label === "Save and quit")[1]();
  // Save stops at the summary: no quitting yet, and still waiting.
  state = { busy: false, hasDocument: false, dirty: true, saveProblemReview: true };
  await tick();
  await tick();
  assert.ok(!invoked.includes("close_editor"));
  // The summary's Save writes; then dawsu! quits.
  state = { busy: false, hasDocument: false, dirty: false };
  await tick();
  await runInContext("pending", context);
  assert.ok(invoked.includes("close_editor"), "quits after the summary's Save");
});

test("Save and quit stops when the summary is cancelled", async () => {
  const listeners = new Map();
  let tick = null;
  let clock = 0;
  let state = { busy: false, hasDocument: false, dirty: true };
  const invoked = [];
  const elements = new Map();
  const get = id => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (name, args) => {
        invoked.push(name === "editor_view" ? args.action?.kind ?? "refresh" : name);
        return name === "editor_view" ? { ...state } : undefined;
      } },
      event: { listen(name, handler) { listeners.set(name, handler); } },
    } },
    performance: { now: () => (clock += 5000) },
    ResizeObserver: class { observe() {} },
    setInterval(callback) { tick = callback; },
    requestAnimationFrame() {},
  });
  await bootEditor(context);
  runInContext("modal = (kind, title, content, buttons) => { globalThis.output = { kind, buttons }; }; closeModal = () => {};", context);
  listeners.get("editor-close-requested")({ payload: "The difficulty has unsaved changes." });
  state = { busy: true, hasDocument: false, dirty: true };
  await context.output.buttons.find(([label]) => label === "Save and quit")[1]();
  state = { busy: false, hasDocument: false, dirty: true, saveProblemReview: true };
  await tick();
  state = { busy: false, hasDocument: false, dirty: true };
  await tick();
  state = { busy: false, hasDocument: false, dirty: false };
  await tick();
  await runInContext("pending", context);
  assert.ok(!invoked.includes("close_editor"), "a cancelled summary doesn't quit later");
});

test("an open difficulty whose sound set has a newer version asks Update or Keep", async () => {
  const calls = [];
  const { context } = await load([], calls);
  runInContext([
    "globalThis.asked = [];",
    "showSoundSetPrompt = prompt => asked.push(prompt);",
    "pendingSoundSetPrompt = () => view.soundSetPrompts?.[0];",
    'scene = { path: "Hard.osu" }; view = { soundSetPrompts: [{ setId: "a", name: "Kick stack", placements: 2 }] }; renderDialogs();',
    "scene = null; renderDialogs();",
  ].join("\n"), context);
  assert.deepEqual(JSON.parse(JSON.stringify(context.asked)), [{ setId: "a", name: "Kick stack", placements: 2 }], "only with a difficulty open");
});
