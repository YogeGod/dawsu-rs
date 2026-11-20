import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");
const drum = await readFile(new URL("../dist/drum-detect.js", import.meta.url), "utf8");
const html = await readFile(new URL("../dist/index.html", import.meta.url), "utf8");
const menus = await readFile(new URL("../dist/menus.js", import.meta.url), "utf8");

const lanes = [{ key: "kick", name: "kick.wav" }, { key: "snare", name: "snare.wav" }, { key: "clap", name: "Soft clap" }];

async function frontend() {
  const controls = new Map();
  const calls = [];
  function element(type) {
    return {
      type, events: new Map(), children: [], dataset: {}, value: "", textContent: "", open: false,
      style: { setProperty(name, value) { this[name] = value; } },
      addEventListener(name, listener) { this.events.set(name, listener); },
      append(...children) { this.children.push(...children); },
      prepend(child) { this.children.unshift(child); },
      replaceChildren(...children) { this.children = children; },
      setAttribute(name, value) { this[name] = value; },
      showModal() { this.open = true; }, close() { this.open = false; },
      getBoundingClientRect: () => ({ x: 0, top: 100, width: 1000 }),
    };
  }
  function get(id) {
    if (!controls.has(id)) controls.set(id, element("div"));
    return controls.get(id);
  }
  get("grid").clientWidth = 1000;
  for (let row = 0; row < 20; row++) get("lanes").children.push(element("div"));
  let status = { busy: false, hasDocument: true, duration: 10_000, drumDetect: { state: "idle", lanes, kick: "kick", snare: null } };
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(JSON.parse(JSON.stringify(args)));
        return { ...status };
      } },
      event: { listen() {} },
    } },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    ResizeObserver: class { observe() {} },
    HTMLElement: class {},
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await runInContext(app + "\n" + drum, context);
  calls.length = 0;
  const evaluate = (code) => runInContext(code, context);
  evaluate(`view = ${JSON.stringify(status)}; scene = { path: "fixture.osu", viewStart: 0, zoom: 1, rowHeight: 30 }`);
  const dialog = get("dialog");
  const body = () => get("dialog-body").children;
  const button = (label) => get("dialog-actions").children.find(b => b.textContent === label);
  const text = () => body().map(node => node.textContent || node.children?.[0]).join(" | ");
  const setStatus = (next) => { status = next; evaluate(`view = ${JSON.stringify(next)}`); };
  return { calls, dialog, body, button, text, evaluate, setStatus, settle: () => evaluate("pending") };
}

test("the Map menu opens drum hit detection and the page loads its script", () => {
  assert.match(html, /<script defer src="drum-detect\.js"><\/script>/);
  assert.match(menus, /menuItem\("Drum hit detection…", "Drumstick", \(\) => openDrumDetection\(\)\)/);
});

test("the lane prompt starts with the saved lanes and starts detection on those lanes", async () => {
  const { calls, dialog, body, button, evaluate, settle } = await frontend();
  evaluate("openDrumDetection()");
  assert.equal(dialog.open, true);
  assert.equal(get_title(evaluate), "Drum hit detection");
  const [kick, snare] = body().map(label => label.children[1]);
  assert.equal(body()[0].children[0], "Kick lane");
  assert.equal(kick.value, "kick");
  assert.equal(snare.value, "");
  assert.deepEqual(kick.children.map(option => option.text), ["None", "kick.wav", "snare.wav", "Soft clap"]);
  snare.value = "clap";
  button("Detect").events.get("click")();
  await settle();
  assert.deepEqual(calls.at(-1).action, { kind: "drum_detect_start", map: "fixture.osu", kick: "kick", snare: "clap" });
});

function get_title(evaluate) {
  return evaluate('$("dialog-title").textContent');
}

test("with no lane picked the prompt asks for lanes and starts nothing", async () => {
  const { calls, body, button, evaluate, settle } = await frontend();
  evaluate("openDrumDetection()");
  body()[0].children[1].value = "";
  button("Detect").events.get("click")();
  await settle();
  assert.equal(calls.length, 0);
  assert.equal(body()[0].textContent, "Pick kick and snare lanes first.");
});

test("progress shows while detecting, and Cancel stops the run", async () => {
  const { calls, text, button, evaluate, setStatus, settle } = await frontend();
  setStatus({ busy: false, hasDocument: true, drumDetect: { state: "running", done: 25, total: 100, lanes } });
  evaluate("openDrumDetection()");
  assert.match(text(), /Detecting drum hits… 25%/);
  setStatus({ busy: false, hasDocument: true, drumDetect: { state: "running", done: 50, total: 100, lanes } });
  evaluate("refreshDrumDetection()");
  assert.match(text(), /50%/);
  button("Cancel").events.get("click")();
  await settle();
  assert.deepEqual(calls.at(-1).action, { kind: "drum_detect_cancel" });
});

test("the result shows its counts and Apply sends the reviewed stamp", async () => {
  const { calls, dialog, text, button, evaluate, setStatus, settle } = await frontend();
  const counts = { added: 30, onObjects: 16, loose: 14, flagged: 2, duplicates: 1 };
  setStatus({ busy: false, hasDocument: true, drumDetect: { state: "ready", review: "123", counts, kicks: 16, snares: 17, lanes } });
  evaluate("openDrumDetection()");
  assert.match(text(), /Found 16 kick and 17 snare hits\./);
  assert.match(text(), /16 on objects, 14 on grid lines\./);
  assert.match(text(), /2 too far from the grid are left out\./);
  assert.match(text(), /1 already there\./);
  setStatus({ busy: false, hasDocument: true, drumDetect: { state: "ready", review: "124", counts: { ...counts, added: 29, duplicates: 2 }, kicks: 16, snares: 17, lanes } });
  evaluate("refreshDrumDetection()");
  assert.match(text(), /2 already there\./);
  button("Apply").events.get("click")();
  await settle();
  assert.deepEqual(calls.at(-1).action, { kind: "drum_apply", map: "fixture.osu", review: "124" });
  assert.equal(dialog.open, false);
});

test("a refused start ends the flow so the menu can start it again", async () => {
  const { button, evaluate, setStatus, settle } = await frontend();
  evaluate("openDrumDetection()");
  button("Detect").events.get("click")();
  await settle();
  setStatus({ busy: false, hasDocument: true, error: "This difficulty's song could not be loaded.", drumDetect: { state: "idle", lanes } });
  evaluate("refreshDrumDetection()");
  assert.equal(evaluate("drumDetectMap"), null);
});
