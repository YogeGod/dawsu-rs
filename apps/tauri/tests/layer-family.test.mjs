import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { bootEditor } from "./editor-runtime.mjs";


// A numbered layer sits under its family's main lane, which folds like a folder.
async function frontend() {
  const controls = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, style: { setProperty(key, value) { this[key] = value; } }, classList: { add() {}, toggle() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute() {}, focus() {},
      append(...children) { this.children.push(...children); },
      prepend() {},
      replaceChildren(...children) { this.children = children; },
      getBoundingClientRect: () => ({ x: 0, y: 0 }),
    };
  }
  function get(id) {
    if (!controls.has(id)) controls.set(id, element());
    return controls.get(id);
  }
  get("scroll").scrollTop = 0;
  const context = createContext({
    document: { getElementById: get, createElement: element, querySelectorAll: () => [], addEventListener() {} },
    window: { __TAURI__: {
      core: { invoke: async (_name, args) => {
        calls.push(JSON.parse(JSON.stringify(args)));
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
  await bootEditor(context);
  calls.length = 0;
  const evaluate = code => runInContext(code, context);
  return { calls, evaluate, get };
}

const finish = (index, more = {}) => ({
  key: "F" + index, sourceKey: "F" + index, colourKey: "Skin|normal-hitfinish" + (index || ""),
  name: "normal-hitfinish" + (index > 1 ? index : ""), family: "normal-hitfinish", index, group: "normal",
  canAddLayer: true, canDelete: true, hitCount: 1, ...more,
});
const clap = { key: "C", sourceKey: "C", colourKey: "Skin|normal-hitclap", name: "normal-hitclap",
  family: "normal-hitclap", index: 0, group: "normal", canAddLayer: true, canDelete: true, hitCount: 0 };

// Finish (skin), Clap, then Finish 3 and Finish 2 added later, as in Yoge's screenshot.
function load(evaluate, { lanes = [finish(0), clap, finish(3), finish(2)], laneOrder, layout = null } = {}) {
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0, tool: "Select" };
    show({ busy: false, hasDocument: true, activeLane: 0, tool: "Select", status: "",
      scene: { path: "fixture.osu", rowHeight: 30, viewStart: 0, zoom: 1, laneRevision: 7, history: [], difficulties: ["fixture.osu"],
        lanes: ${JSON.stringify(lanes)}, ${laneOrder ? "laneOrder: " + JSON.stringify(laneOrder) + "," : ""}
        moments: [[1000, 80, true, [[2, false], [3, false]]]], laneLayout: ${JSON.stringify(layout)} } });
  `);
}
const rows = evaluate => JSON.parse(evaluate("JSON.stringify(laneHeaders.rows)"));
const order = evaluate => JSON.parse(evaluate("JSON.stringify(laneRows)"));
const action = (evaluate, ...args) => evaluate("laneHeaderAction(" + args.map(arg => JSON.stringify(arg)).join(", ") + ")");

test("a numbered layer sits directly under its main lane, by number, as an indented child", async () => {
  const { evaluate } = await frontend();
  load(evaluate);
  assert.deepEqual(order(evaluate), ["normal", 0, 3, 2, 1]);
  const lanes = rows(evaluate).filter(row => row.type === "lane");
  assert.deepEqual(lanes.map(row => [row.name, row.number, row.layer]),
    [["Finish", 1, false], ["Finish 2", 2, true], ["Finish 3", 3, true], ["Clap", 4, false]]);
  assert.deepEqual(lanes[0].family, { collapsed: false, label: "Collapse normal-hitfinish layers" });
  assert.equal(lanes[3].family, null);
});

test("a saved lane order that splits a family does not split it; the file at index 1 is the main lane", async () => {
  const { evaluate } = await frontend();
  load(evaluate, {
    lanes: [finish(0), clap, finish(3), finish(2), finish(1)],
    layout: { lane_order: ["Skin|normal-hitfinish3", "Skin|normal-hitclap", "Skin|normal-hitfinish1"] },
  });
  assert.deepEqual(order(evaluate), ["normal", 1, 4, 0, 3, 2]);
  assert.deepEqual(rows(evaluate).filter(row => row.type === "lane").map(row => row.layer), [false, false, true, true, true]);
});

test("the main lane's arrow folds its layers, saved in collapsed_families", async () => {
  const { calls, evaluate } = await frontend();
  load(evaluate);
  action(evaluate, "family", 0);
  assert.deepEqual(order(evaluate), ["normal", 0, 1]);
  assert.equal(evaluate("previewParents.get(2)"), 0);
  assert.equal(evaluate("previewParents.get(3)"), 0);
  assert.deepEqual(rows(evaluate)[1].family, { collapsed: true, label: "Expand normal-hitfinish layers" });
  await evaluate("pending");
  assert.deepEqual(calls.pop().action.value.collapsed_families, ["normal-hitfinish"]);
  action(evaluate, "family", 0);
  assert.deepEqual(order(evaluate), ["normal", 0, 3, 2, 1]);
});

test("without a shown main lane, layers are ordinary lanes and a fold never hides them", async () => {
  const { evaluate, get } = await frontend();
  const folded = { collapsed_families: ["normal-hitfinish"] };
  // Main lane removed (not in the lane order).
  load(evaluate, { laneOrder: [1, 2, 3], layout: folded });
  assert.deepEqual(order(evaluate), ["normal", 1, 2, 3]);
  assert.ok(rows(evaluate).every(row => !row.layer && !row.family));
  // Main lane filtered out.
  load(evaluate, { layout: folded });
  get("lane-filter").value = "hitfinish2";
  get("lane-filter").events.get("input")();
  assert.deepEqual(order(evaluate), ["normal", 3]);
  get("lane-filter").value = "";
  get("lane-filter").events.get("input")();
  // Main lane unused under Used only.
  evaluate("scene.moments = [[1000, 80, true, [[2, false], [3, false]]]]");
  get("used-only").checked = true;
  get("used-only").events.get("change")();
  assert.deepEqual(order(evaluate), ["normal", 3, 2]);
});

test("+ asks for the next layer and unfolds a folded family so the new layer shows under it", async () => {
  const { calls, evaluate } = await frontend();
  load(evaluate, { layout: { collapsed_families: ["normal-hitfinish"] } });
  assert.deepEqual(order(evaluate), ["normal", 0, 1]);
  assert.equal(rows(evaluate)[1].addLayer, "Add numbered normal-hitfinish layer");
  action(evaluate, "addLayer", 0);
  await evaluate("pending");
  assert.deepEqual(calls.pop().action, { kind: "lane", row: 0, operation: "add_layer" });
  // The editor answers with Finish 4 (the same path for Ctrl+L and the lane menu).
  evaluate(`show({ ...view, scene: { ...scene, lanes: [...scene.lanes, ${JSON.stringify(finish(4))}] } })`);
  assert.deepEqual(order(evaluate), ["normal", 0, 3, 2, 4, 1]);
  await evaluate("pending");
  assert.deepEqual(calls.pop().action.value.collapsed_families, []);
});

test("the drum view shows no + and adds no layer", async () => {
  const { calls, evaluate } = await frontend();
  load(evaluate);
  evaluate(`view = { ...view, drumView: { enabled: true } }; scene.drumRows = [{ row: 0, name: "Crash" }]; renderLanes();`);
  assert.equal(rows(evaluate)[0].addLayer, null);
  assert.equal(rows(evaluate)[0].remove, null);
  action(evaluate, "addLayer", 0);
  action(evaluate, "remove", 0);
  await evaluate("pending");
  assert.equal(calls.length, 0);
});

test("− removes that lane with its revision, and is disabled where Delete lane is", async () => {
  const { calls, evaluate } = await frontend();
  load(evaluate, { lanes: [finish(0), { ...clap, canDelete: false }, finish(3), finish(2)] });
  const lanes = rows(evaluate).filter(row => row.type === "lane");
  assert.deepEqual(lanes.map(row => row.remove), [
    { label: "Remove Finish", disabled: false }, { label: "Remove Finish 2", disabled: false },
    { label: "Remove Finish 3", disabled: false }, { label: "Remove Clap", disabled: true },
  ]);
  action(evaluate, "remove", 1);
  await evaluate("pending");
  assert.equal(calls.length, 0);
  action(evaluate, "remove", 2);
  await evaluate("pending");
  assert.deepEqual(calls.pop().action, { kind: "delete_lane", target: { map: "fixture.osu", lane: "F3" }, revision: 7 });
  evaluate("view = { ...view, busy: true }");
  action(evaluate, "remove", 2);
  await evaluate("pending");
  assert.equal(calls.length, 0);
});

test("moving a main lane moves its family; a layer keeps its place by number", async () => {
  const { evaluate } = await frontend();
  load(evaluate);
  assert.equal(evaluate("adjacentLaneForMove(3, 1)"), null);
  assert.equal(evaluate("adjacentLaneForMove(2, -1)"), null);
  assert.equal(evaluate("adjacentLaneForMove(0, 1)"), 1);
  evaluate("moveLane(0, 1)");
  assert.deepEqual(order(evaluate), ["normal", 1, 0, 3, 2]);
  assert.equal(evaluate("adjacentLaneForMove(1, 1)"), 0);
  evaluate("moveLane(0, -1)");
  assert.deepEqual(order(evaluate), ["normal", 0, 3, 2, 1]);
});

test("a lane move saves the saved order with one lane moved, never the family order", async () => {
  const { calls, evaluate } = await frontend();
  load(evaluate);
  const saved = async () => {
    await evaluate("pending");
    return calls.filter(call => call.action?.kind === "lane_layout").at(-1).action.value.lane_order;
  };
  evaluate("moveLane(0, 1)");
  assert.deepEqual(await saved(), ["Skin|normal-hitclap", "Skin|normal-hitfinish", "Skin|normal-hitfinish3", "Skin|normal-hitfinish2"]);
  evaluate("moveLane(0, -1)");
  assert.deepEqual(await saved(), ["Skin|normal-hitfinish", "Skin|normal-hitclap", "Skin|normal-hitfinish3", "Skin|normal-hitfinish2"]);
});

test("on a folded main lane, Draw gets a click on a layer's outline; Select and Erase unfold the family", async () => {
  const { evaluate } = await frontend();
  const folded = { layout: { collapsed_families: ["normal-hitfinish"] } };
  load(evaluate, folded);
  evaluate('view = { ...view, tool: "Draw" }');
  assert.equal(evaluate("revealLayerPreview(0, 1000, 15)"), false, "Draw stacks a hit on the main lane");
  assert.deepEqual(order(evaluate), ["normal", 0, 1]);
  for (const tool of ["Select", "Erase"]) {
    const { evaluate } = await frontend();
    load(evaluate, folded);
    evaluate(`view = { ...view, tool: "${tool}" }`);
    assert.equal(evaluate("revealLayerPreview(0, 1000, 15)"), true, tool);
    assert.deepEqual(order(evaluate), ["normal", 0, 3, 2, 1], tool);
  }
});

test("a family unfolds when one of its layers comes back: Add lane or Undo of −", async () => {
  const { calls, evaluate } = await frontend();
  load(evaluate, { laneOrder: [0, 1, 3], layout: { collapsed_families: ["normal-hitfinish"] } });
  assert.deepEqual(order(evaluate), ["normal", 0, 1]);
  evaluate("show({ ...view, scene: { ...scene, laneOrder: [0, 1, 2, 3] } })");
  assert.deepEqual(order(evaluate), ["normal", 0, 3, 2, 1]);
  await evaluate("pending");
  assert.deepEqual(calls.pop().action.value.collapsed_families, []);
});

test("− presses while a removal is on its way are ignored", async () => {
  const { calls, evaluate } = await frontend();
  load(evaluate);
  action(evaluate, "remove", 2);
  action(evaluate, "remove", 3);
  await evaluate("pending");
  assert.deepEqual(calls.filter(call => call.action?.kind === "delete_lane").map(call => call.action.target.lane), ["F3"]);
  action(evaluate, "remove", 3);
  await evaluate("pending");
  assert.equal(calls.filter(call => call.action?.kind === "delete_lane").length, 2);
});

test("the status bar counts shown lanes, so a removed lane drops out of it", async () => {
  const { evaluate, get } = await frontend();
  load(evaluate, { laneOrder: [0, 1, 3] });
  assert.equal(get("hit-lane-counts").textContent, "2 hits · 3 lanes");
});

test("− names a renamed lane by its own name, without the lane info", async () => {
  const { evaluate } = await frontend();
  load(evaluate);
  evaluate('renamedLanes.set("Skin|normal-hitfinish3", "Crash"); showLaneInfo = true; renderLanes();');
  assert.equal(rows(evaluate).find(row => row.row === 2).remove.label, "Remove Crash");
});
