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
    { key: "n", name: "soft-hitnormal", family: "soft-hitnormal", group: "soft", index: 1, colourKey: "Skin|soft-hitnormal", hitCount: 3, canAddLayer: true },
    { key: "c2", name: "soft-hitclap2", family: "soft-hitclap", group: "soft", index: 2, colourKey: "Skin|soft-hitclap2", hitCount: 0 },
    { key: "s", name: "soft-sliderslide", family: "soft-sliderslide", group: "soft", index: 1, colourKey: "Skin|soft-sliderslide" },
    { key: "k", name: "Kick", sample: "kick.wav", colourKey: "File|kick.wav" },
    { key: "n0", name: "soft-hitnormal", family: "soft-hitnormal", group: "soft", index: 0, colourKey: "Skin|soft-hitnormal0" },
    { key: "sw", name: "soft-sliderwhistle", family: "soft-sliderwhistle", group: "soft", index: 1, colourKey: "Skin|soft-sliderwhistle" },
    { key: "t", name: "soft-slidertick", family: "soft-slidertick", group: "soft", index: 1, colourKey: "Skin|soft-slidertick" },
  ] };
  renderLanes();
`;
const part = (row, className) => row.children.find(child => child.className?.split(" ").includes(className));
const label = row => part(row, "name").textContent;

test("a lane header reads as an icon tile, M, S, the sound name and its set as a tag", async () => {
  const { evaluate, get } = await frontend();
  evaluate(fixture);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",0,1,2,4,5,6,3]');
  const rows = get("lanes").children.slice(1);
  assert.deepEqual(rows.map(row => [label(row), part(row, "lane-tag")?.textContent, part(row, "lane-icon")?.dataset.sound]), [
    ["Normal", "Soft", "normal"], ["Clap 2", "Soft", "clap"], ["Slide", "Soft", "slide"],
    ["Normal", "Soft · skin", "normal"], ["Slide whistle", "Soft", "whistle"], ["Tick", "Soft", "tick"],
    ["Kick", "File", "file"],
  ]);
  // The page order is the drawn order, so Tab reaches M and S before the name.
  assert.deepEqual(rows[0].children.map(child => child.className),
    ["lane-icon", "lane-mute", "lane-solo", "lane-disclosure", "name", "lane-tag", "lane-add-layer"]);
  assert.deepEqual(rows[1].children.map(child => child.className), ["lane-icon", "lane-mute", "lane-solo", "name", "lane-tag"]);
  // The name button holds only the name; the tag is its own element.
  assert.equal(part(rows[0], "name").children.length, 0);
  const [normal, clap, , , , , kick] = rows;
  assert.equal(normal.style["--lane-sound"], "#f1ad67");
  assert.equal(normal.style["--lane-sound-tint"], "#f1ad6724");
  assert.equal(normal.style["--lane-tile"], "22px");
  assert.equal(clap.style["--lane-sound"], "#e28dd1");
  assert.equal(kick.style["--lane-sound"], "#62d6cf");
  assert.equal(part(normal, "name").dataset.sample, undefined);
  // The content policy (img-src 'self') blocks data: images, so the icon is inline SVG.
  assert.match(part(normal, "lane-icon").innerHTML, /^<svg viewBox="0 0 16 16"><circle /);
  assert.doesNotMatch(css, /url\("data:/);

  evaluate('renamedLanes.set("Skin|soft-hitclap2", "Snare"); renderLanes();');
  assert.equal(label(get("lanes").children[2]), "Snare");
  assert.equal(part(get("lanes").children[2], "lane-tag").textContent, "Soft");

  evaluate("showLaneInfo = true; renderLanes();");
  assert.equal(label(get("lanes").children[1]), "soft-hitnormal · idx 1");

  evaluate("showLaneInfo = false; scene.rowHeight = 20; renderLanes();");
  assert.equal(get("lanes").children[1].style["--lane-tile"], "16px");

  evaluate("view.panels = { laneNames: false }; renderLanes();");
  const hidden = get("lanes").children[2];
  assert.deepEqual(hidden.children.map(child => child.className), ["lane-mute", "lane-solo", "name"]);
  assert.equal(label(hidden), "▶");
});

test("lane header styles stay within what every platform's web view draws", () => {
  const laneRules = css.match(/[^}]*\.lane[^{]*\{[^}]*\}/g).filter(rule => !rule.includes("tint-preview"));
  for (const rule of laneRules) assert.doesNotMatch(rule, /color-mix|aspect-ratio|order:\s*-?\d/, rule);
  // The 3px sound-colour bar comes from the lane-tinted box shadow; nothing later replaces it.
  const bar = css.lastIndexOf(".lane.lane-tinted{box-shadow:inset 3px 0 var(--lane-colour)}");
  assert.ok(bar > 0);
  assert.doesNotMatch(css.slice(bar + 20), /\.lane[^{]*\{[^}]*box-shadow/);
  // Group M and S stay in the row and reachable by Tab: dimmed while off, never removed.
  assert.doesNotMatch(css, /sample-folder[^{]*lane-(mute|solo)[^{]*\{[^}]*display:\s*none/);
  assert.match(css, /\.lane\.sample-folder > :is\(\.lane-mute,\.lane-solo\)\s*\{[^}]*opacity:\s*\.45/);
  // The design's 250px lane column, still clamped and still resizable.
  assert.match(css, /--lane-default-width:clamp\(170px,18vw,250px\)/);
});

test("clicking the row outside its buttons selects and auditions the lane like its name", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(fixture);
  const clap = get("lanes").children[2];
  assert.equal(clap.dataset.laneRow, 1);
  clap.events.get("click")({ target: { closest: () => part(clap, "lane-mute") } });
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
  assert.deepEqual(header.children.map(child => child.className), ["name", "lane-count", "lane-mute", "lane-solo"]);
  assert.equal(part(header, "lane-count").textContent, "6 lanes");
  header.children[0].events.get("click")();
  const folded = get("lanes").children[0];
  assert.equal(folded.children[0].attributes["aria-expanded"], "false");
  assert.equal(part(folded, "lane-count").textContent, "6 lanes · 1 used");
  assert.doesNotMatch(css, /--sample-set-accent/);
});

test("the drum view keeps its rows and names and adds no tags", async () => {
  const { evaluate, get } = await frontend();
  evaluate(fixture + `view.drumView = { enabled: true, name: "Kit" };
    scene.drumRows = [{ row: 1, name: "Snare" }, { row: null, name: "Ghost" }, { row: 3, name: "Kick drum" }];
    renderLanes();`);
  const rows = get("lanes").children;
  assert.deepEqual(rows.map(label), ["Snare", "Ghost", "Kick drum"]);
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[1,null,3]");
  assert.equal(part(rows[0], "lane-tag"), undefined);
  assert.equal(part(rows[0], "lane-icon").dataset.sound, "clap");
  evaluate("view.drumView.enabled = false; renderLanes();");
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["soft",0,1,2,4,5,6,3]');
});
