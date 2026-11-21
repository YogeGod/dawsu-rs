import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { dockingRules } from "./docking-rules.mjs";

// The bottom panel (v3 F1-F3): Volume and Mixer tabs, the folded strip and the Mixer's strips.
// ui/tests/bottom-panel.test.mjs checks the Svelte markup.
const read = name => readFile(new URL("../dist/" + name, import.meta.url), "utf8");
const [html, app] = await Promise.all(["index.html", "app.js"].map(read));

async function frontend(ui) {
  const elements = new Map();
  const calls = [];
  function element(id) {
    if (!elements.has(id)) {
      elements.set(id, {
        id, listeners: new Map(), attributes: new Map(), children: [], dataset: {}, hidden: false,
        style: { setProperty() {}, removeProperty() {} },
        classList: { add() {}, remove() {}, toggle() {} },
        addEventListener(name, callback) { this.listeners.set(name, callback); },
        append(...children) { this.children.push(...children); }, prepend() {}, before() {}, after() {},
        replaceChildren(...children) { this.children = children; },
        setAttribute(name, value) { this.attributes.set(name, String(value)); },
        getAttribute(name) { return this.attributes.get(name) ?? null; },
        removeAttribute(name) { this.attributes.delete(name); },
        toggleAttribute(name, on) { if (on) this.attributes.set(name, ""); else this.attributes.delete(name); },
        querySelector: () => null, querySelectorAll: () => [], closest: () => null,
        getClientRects: () => [], focus() {}, scrollIntoView() {},
      });
    }
    return elements.get(id);
  }
  const { effectivePanels, togglePlan } = dockingRules;
  const context = createContext({
    document: { getElementById: element, createElement: () => element(Symbol()), querySelectorAll: () => [],
      addEventListener() {}, readyState: "loading" },
    window: { dawsuUi: ui, editorDocking: { effectivePanels, togglePlan, attach: () => undefined }, __TAURI__: {
      core: { invoke: async (_name, args) => {
        if (args.action) calls.push(args.action);
        return { busy: false, hasDocument: true };
      } },
      event: { listen() {} },
    } },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    setInterval() {},
    requestAnimationFrame() {},
    console,
  });
  await runInContext(app, context);
  calls.length = 0;
  return { element, calls, evaluate: code => runInContext(code, context) };
}

test("the bottom panel mounts beside the plain volume canvas and ramp shape", () => {
  const panel = html.slice(html.indexOf('<div id="volume-controller-panel"'), html.indexOf('<div class="overview">'));
  const title = panel.slice(panel.indexOf('<div class="lane-title volume-title">'), panel.indexOf('<canvas id="controller"'));
  // main.js empties a mount, so #ramp-shape and the canvas stay outside the mounts.
  assert.match(title, /^<div class="lane-title volume-title">\s*<div data-ui-mount="bottom-panel"><\/div>\s*<select id="ramp-shape"/);
  assert.match(panel, /<canvas id="controller"[^>]*><\/canvas>\s*<div class="mixer-host" data-ui-mount="mixer"><\/div>\s*<\/div>/);
  assert.doesNotMatch(panel, /id="volume-controller-panel"[^>]*data-ui-mount|class="lane-title volume-title"[^>]*data-ui-mount/);
});

const open = { browser: true, inspector: true, controller: true, laneNames: true };

test("folding draws the strip instead of hiding the panel; the tab comes from the bundle", async () => {
  let tab = "volume";
  const { element, evaluate } = await frontend({ bottomTab: () => tab });
  const panel = element("volume-controller-panel");
  evaluate(`scene = { path: "a.osu", lanes: [] }; view = { busy: false, hasDocument: true,
    panels: ${JSON.stringify(open)} }; renderSampleBrowser = () => {}; renderPanelLayout();`);
  assert.equal(panel.dataset.bottom, "volume");
  assert.equal(panel.hidden, false);
  tab = "mixer";
  evaluate("renderPanelLayout()");
  assert.equal(panel.dataset.bottom, "mixer");
  evaluate("view.panels = { ...view.panels, controller: false }; renderPanelLayout()");
  assert.equal(panel.dataset.bottom, "folded");
  assert.equal(panel.hidden, false, "the folded strip stays on screen");
  assert.equal(element("view-controller").ariaPressed, "false");
});

test("without the bundle the panel shows the Volume tab", async () => {
  const { element, evaluate } = await frontend(undefined);
  evaluate(`scene = { path: "a.osu", lanes: [] }; view = { busy: false, hasDocument: true,
    panels: ${JSON.stringify(open)} }; renderSampleBrowser = () => {}; renderPanelLayout();`);
  assert.equal(element("volume-controller-panel").dataset.bottom, "volume");
});

test("the volume canvas is drawn only while the Volume tab shows", () => {
  assert.match(app, /const v = volumePanel\.hidden \|\| volumePanel\.dataset\?\.layoutBarHidden !== undefined \|\|\s*\(volumePanel\.dataset\?\.bottom \?\? "volume"\) !== "volume" \? null : prepare\(controller\);/);
});

const fixture = `
  view = { busy: false, hasDocument: true, activeLane: 1, panels: ${JSON.stringify(open)} };
  scene = { path: "a.osu", rowHeight: 30, moments: [], lanes: [
    { key: "n", name: "soft-hitnormal", family: "soft-hitnormal", group: "soft", index: 1, colourKey: "Skin|soft-hitnormal", hitCount: 3 },
    { key: "c2", name: "soft-hitclap2", family: "soft-hitclap", group: "soft", index: 2, colourKey: "Skin|soft-hitclap2", hitCount: 2, solo: true },
    { key: "s", name: "soft-sliderslide", family: "soft-sliderslide", group: "soft", index: 1, colourKey: "Skin|soft-sliderslide", hitCount: 0 },
    { key: "k", name: "Kick", sample: "kick.wav", colourKey: "File|kick.wav", hitCount: 4, muted: true },
    { key: "n0", name: "soft-hitnormal", family: "soft-hitnormal", group: "soft", index: 0, colourKey: "Skin|soft-hitnormal0", hitCount: 1 },
    { key: "dn", name: "drum-hitnormal", family: "drum-hitnormal", group: "drum", index: 1, colourKey: "Skin|drum-hitnormal", hitCount: 5 },
  ] };
  renderLanes();
`;

// Lane-list order: the folders in their order, then the map file lanes, as the lane headers show them.
test("the Mixer gets one strip per used lane in lane-list order, with the header's number", async () => {
  let handed = null;
  const { evaluate } = await frontend({ lanes() {}, mixer(model) { handed = model; } });
  evaluate(fixture);
  const strips = JSON.parse(evaluate("JSON.stringify(currentMixer())"));
  const headers = JSON.parse(evaluate("JSON.stringify(laneHeaders.rows.filter(row => row.type === 'lane'))"));
  const numbers = new Map(headers.map(row => [row.row, row.number]));
  // Slide has no hits, so it gets no strip; the numbers still match the lane headers.
  assert.deepEqual(strips.map(strip => [strip.name, strip.set, strip.icon, strip.number]),
    [["Normal", "Soft", "normal", 1], ["Normal", "Soft", "normal", 2], ["Clap 2", "Soft", "clap", 3],
      ["Normal", "Drum", "normal", 6], ["Kick", "File", "file", 5]]);
  for (const strip of strips) assert.equal(strip.number, numbers.get(strip.row), strip.name);
  assert.deepEqual(strips[0], { key: "lane:0", row: 0, number: 1, name: "Normal", set: "Soft", icon: "normal",
    colour: null, selected: false, muted: false, solo: false, quiet: true });
  assert.deepEqual(strips.map(strip => [strip.muted, strip.solo, strip.quiet]).slice(2),
    [[false, true, false], [false, false, true], [true, false, true]]);
  assert.deepEqual(JSON.parse(JSON.stringify(handed)), strips, "handed to the bundle");

  // A folded folder or the lane filter hides header rows, never strips.
  evaluate('collapsedFolders.add("soft"); renderLanes();');
  assert.equal(JSON.parse(evaluate("JSON.stringify(currentMixer())")).length, 5);

  // Grey until the mapper picks a colour; a renamed lane keeps the mapper's name.
  evaluate('laneColours.set("Skin|soft-hitclap2", "#d05050"); renamedLanes.set("Skin|soft-hitclap2", "Snare"); renderLanes();');
  const clap = JSON.parse(evaluate("JSON.stringify(currentMixer())"))[2];
  assert.equal(clap.colour, "#d05050");
  assert.equal(clap.name, "Snare");
  assert.equal(handed[2].name, "Snare");
});

test("the switch, View settings and the View menu call the panel Volume and Mixer", async () => {
  const button = html.match(/<button id="view-controller"[^>]*>/)[0];
  assert.match(button, /aria-label="Volume and Mixer"/);
  assert.match(button, /title="Volume and Mixer"/);
  assert.match(app, /\["Volume and Mixer", "controller", view\.panels\.controller\]/);
  const { items } = await import("../ui/src/lib/commands.js");
  assert.equal(items.find(item => item.id === "volumePanel").label, "Volume and Mixer");
  assert.doesNotMatch(html + app, /Volume panel/);
});
