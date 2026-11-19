import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const read = name => readFile(new URL("../dist/" + name, import.meta.url), "utf8");
const [app, css] = await Promise.all(["app.js", "style.css"].map(read));

async function frontend() {
  const controls = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, attributes: {}, classes: [],
      style: { setProperty(name, value) { this[name] = value; } },
      classList: { add(name) { this.owner.classes.push(name); }, toggle() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute(name, value) { this.attributes[name] = String(value); },
      append(...children) { this.children.push(...children); },
      prepend(...children) { this.children.unshift(...children); }, focus() {},
      replaceChildren(...children) { this.children = children; },
    };
  }
  const create = () => {
    const made = element();
    made.classList.owner = made;
    return made;
  };
  const get = id => {
    if (!controls.has(id)) controls.set(id, create());
    return controls.get(id);
  };
  const context = createContext({
    document: { getElementById: get, createElement: create, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(args?.action);
        return { busy: false, hasDocument: true, position: 0 };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame() {},
    setInterval() {},
    console,
  });
  await runInContext(app, context);
  return { calls, evaluate: code => runInContext(code, context), get };
}

const fixture = `
  view = { busy: false, hasDocument: true, activeLane: 1 };
  scene = { path: "a.osu", rowHeight: 30, moments: [], lanes: [
    { key: "n", name: "soft-hitnormal", family: "soft-hitnormal", group: "soft", index: 1, colourKey: "Skin|soft-hitnormal", hitCount: 3 },
    { key: "c2", name: "soft-hitclap2", family: "soft-hitclap", group: "soft", index: 2, colourKey: "Skin|soft-hitclap2", hitCount: 0 },
    { key: "s", name: "soft-sliderslide", family: "soft-sliderslide", group: "soft", index: 1, colourKey: "Skin|soft-sliderslide" },
    { key: "k", name: "Kick", sample: "kick.wav", colourKey: "File|kick.wav" },
  ] };
  renderLanes();
`;
const part = (row, className) => row.children.find(child => child.className === className);

test("a lane header reads as an icon tile, M, S, the sound name and its set as a tag", async () => {
  const { evaluate, get } = await frontend();
  evaluate(fixture);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",0,1,2,3]');
  const [, normal, clap, slide, kick] = get("lanes").children;
  const shown = [normal, clap, slide, kick].map(row => [
    row.children[0].textContent, part(row.children[0], "lane-tag")?.textContent, part(row, "lane-icon")?.dataset.sound,
  ]);
  assert.deepEqual(shown, [
    ["Normal", "Soft", "normal"], ["Clap 2", "Soft", "clap"], ["Slider slide", "Soft", "slide"], ["Kick", "File", "file"],
  ]);
  // Older tests and the lane menu still find the name, M and S at the start of the row.
  assert.deepEqual(normal.children.slice(0, 3).map(child => child.className), ["name", "lane-mute", "lane-solo"]);
  assert.equal(normal.style["--lane-sound"], "#f1ad67");
  assert.equal(clap.style["--lane-sound"], "#e28dd1");
  assert.equal(kick.style["--lane-sound"], "#62d6cf");
  assert.equal(normal.children[0].dataset.sample, undefined);
  // The content policy (img-src 'self') blocks data: images, so the icon is inline SVG.
  assert.match(part(normal, "lane-icon").innerHTML, /^<svg viewBox="0 0 16 16"><circle /);
  assert.doesNotMatch(css, /url\("data:/);

  evaluate('renamedLanes.set("Skin|soft-hitclap2", "Snare"); renderLanes();');
  const renamed = get("lanes").children[2].children[0];
  assert.equal(renamed.textContent, "Snare");
  assert.equal(part(renamed, "lane-tag").textContent, "Soft");

  evaluate("showLaneInfo = true; renderLanes();");
  assert.equal(get("lanes").children[1].children[0].textContent, "soft-hitnormal · idx 1");

  evaluate("showLaneInfo = false; view.panels = { laneNames: false }; renderLanes();");
  const hidden = get("lanes").children[1];
  assert.equal(hidden.children[0].textContent, "▶");
  assert.equal(part(hidden.children[0], "lane-tag"), undefined);
  assert.equal(part(hidden, "lane-icon"), undefined);
});

test("clicking the row outside its buttons selects and auditions the lane like its name", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(fixture);
  const clap = get("lanes").children[2];
  assert.equal(clap.dataset.laneRow, 1);
  clap.events.get("click")({ target: { closest: () => clap.children[1] } });
  clap.events.get("click")({ target: { closest: () => null } });
  await evaluate("pending");
  assert.deepEqual(calls.filter(Boolean).map(action => action.kind), ["lane"]);
  assert.deepEqual(JSON.parse(JSON.stringify(calls.filter(Boolean)[0])), { kind: "lane", row: 1, operation: "audition" });
  assert.equal(evaluate("JSON.stringify([...selectionLaneKeys])"), '["c2"]');
});

test("the Inspector and the lane filter use the same sound and set names", async () => {
  const { evaluate, get } = await frontend();
  evaluate(fixture + "renderActiveLane();");
  assert.equal(get("active-lane-name").textContent, "Soft Clap 2");
  evaluate(`scene.moments = [[1000, 70, true, [[0, true]]]];
    scene.selection = { count: 1, start: 1000, object: "Circle" }; renderSelection();`);
  assert.equal(get("selected-lanes").textContent, " · Soft Normal");
  const filter = get("lane-filter");
  filter.value = "soft clap";
  filter.events.get("input")();
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",1]');
});

test("group rows show a chevron, the set name and its lane count without set colours", async () => {
  const { evaluate, get } = await frontend();
  evaluate(fixture);
  const header = get("lanes").children[0];
  assert.equal(header.children[0].textContent, "Soft set");
  assert.equal(header.children[0].attributes["aria-expanded"], "true");
  assert.deepEqual(header.children.slice(1, 3).map(child => child.className), ["lane-mute", "lane-solo"]);
  assert.equal(part(header, "lane-count").textContent, "3 lanes");
  header.children[0].events.get("click")();
  const folded = get("lanes").children[0];
  assert.equal(folded.children[0].attributes["aria-expanded"], "false");
  assert.equal(part(folded, "lane-count").textContent, "3 lanes · 1 used");
  assert.doesNotMatch(css, /--sample-set-accent/);
});

test("the drum view keeps its rows and names and adds no tags", async () => {
  const { evaluate, get } = await frontend();
  evaluate(fixture + `view.drumView = { enabled: true, name: "Kit" };
    scene.drumRows = [{ row: 1, name: "Snare" }, { row: null, name: "Ghost" }, { row: 3, name: "Kick drum" }];
    renderLanes();`);
  const rows = get("lanes").children;
  assert.deepEqual(rows.map(row => row.children[0].textContent), ["Snare", "Ghost", "Kick drum"]);
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[1,null,3]");
  assert.equal(part(rows[0].children[0], "lane-tag"), undefined);
  assert.equal(part(rows[0], "lane-icon").dataset.sound, "clap");
  evaluate("view.drumView.enabled = false; renderLanes();");
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",0,1,2,3]');
});
