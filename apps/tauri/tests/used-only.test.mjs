import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const app = await readFile(new URL("../dist/app.js", import.meta.url), "utf8");

async function frontend() {
  const controls = new Map();
  const calls = [];
  function element() {
    return {
      events: new Map(), children: [], dataset: {}, style: {}, classList: { toggle() {} },
      addEventListener(name, listener) { this.events.set(name, listener); },
      setAttribute() {},
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
  await runInContext(app, context);
  calls.length = 0;
  const evaluate = (code) => runInContext(code, context);
  return { calls, evaluate, get };
}

test("loading another difficulty restores its own saved folder order and collapsed rows", async () => {
  const { calls, evaluate } = await frontend();
  evaluate(`
    const fixtureScene = {
      path: "first.osu", rowHeight: 30, viewStart: 0, zoom: 1,
      difficulties: ["first.osu", "second.osu"], history: [],
      lanes: [
        { key: "normal", name: "Hitnormal", group: "normal" },
        { key: "drum", name: "Drum finish", group: "drum" }
      ],
      moments: [[1000, 80, true, [[1, false]]]],
      laneLayout: {
        folder_order: ["drum", "soft", "normal"],
        collapsed_folders: ["drum"],
        collapsed_families: []
      }
    };
    const loaded = { busy: false, hasDocument: true, status: "Difficulty loaded" };
    show({ ...loaded, scene: fixtureScene });
  `);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["drum","normal",0]');
  assert.equal(evaluate("previewParents.get(1)"), "drum");
  evaluate(`show({ ...loaded, scene: { ...fixtureScene, path: "second.osu", laneLayout: null } });`);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["normal",0,"drum",1]');
  evaluate("show({ ...loaded, scene: fixtureScene });");
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["drum","normal",0]');
  assert.equal(calls.length, 0);
});

test("filtering by lane name or source preserves the original lane row", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0, tool: "Draw" };
    scene = {
      path: "fixture.osu", rowHeight: 30,
      lanes: [{ key: "A", name: "Kick" }, { key: "B", name: "Clap", sample: "percussion/snare.wav" }],
      moments: [[1000, 80, true, [[1, false]]]]
    };
  `);
  const field = get("lane-filter");
  field.value = "  SNARE ";
  field.events.get("input")();
  assert.deepEqual(get("lanes").children.map(row => row.children[0].textContent), ["Clap"]);
  get("lanes").children[0].children[1].events.get("click")();
  await evaluate("pending");
  assert.equal(calls.at(-1).action.row, 1);
  field.value = "kick";
  field.events.get("input")();
  assert.deepEqual(get("lanes").children.map(row => row.children[0].textContent), ["Kick"]);
});

test("Used only compacts rows without renumbering the lanes sent to Rust", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0, tool: "Draw" };
    scene = {
      path: "fixture.osu", rowHeight: 30, viewStart: 0, zoom: 1,
      lanes: [{ key: "A", name: "A" }, { key: "B", name: "B" }, { key: "C", name: "C" }],
      moments: [[1000, 80, true, [[0, false], [2, false]]]]
    };
  `);

  const toggle = get("used-only").events.get("change");
  assert.equal(typeof toggle, "function");
  get("used-only").checked = true;
  toggle();
  assert.deepEqual(get("lanes").children.map(row => row.children[0].textContent), ["A", "C"]);
  get("lanes").children[1].children[1].events.get("click")();
  await evaluate("pending");
  assert.deepEqual(calls.pop(), { scene: true, action: { kind: "lane", row: 2, operation: "mute" } });

  const event = { clientX: 160, clientY: 44 };
  const pointer = evaluate(`
    drag = { surface: grid, start: 0, zoom: 1, rows: laneRows };
    pointer(${JSON.stringify(event)}, "down");
  `);
  assert.equal(pointer.row, 2);
  assert.equal(evaluate("lanePositions.get(2)"), 1);
  evaluate("drag = null; scene.moments[0][3] = [[2, false]]; renderLanes();");
  assert.deepEqual(get("lanes").children.map(row => row.children[0].textContent), ["A", "C"]);
  get("used-only").checked = false;
  toggle();
  assert.deepEqual(get("lanes").children.map(row => row.children[0].textContent), ["A", "B", "C"]);
  get("used-only").checked = true;
  toggle();
  assert.deepEqual(get("lanes").children.map(row => row.children[0].textContent), ["C"]);
});



test("collapsing sample layers keeps their model rows separate and lets them expand", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(
    'view = { busy: false, hasDocument: true, activeLane: 0, tool: "Select" };' +
    'scene = { path: "fixture.osu", rowHeight: 30,' +
    ' lanes: [{ key:"A", name:"Default clap", family:"clap", index:0 },' +
    ' { key:"B", name:"Map clap", family:"clap", index:1 },' +
    ' { key:"C", name:"Clap 2", family:"clap", index:2 }, { key:"D", name:"Free sample" }],' +
    ' moments: [[1000, 80, true, [[2, false]]]] }; renderLanes();'
  );
  const collapse = get("collapse-layers").events.get("click");
  assert.equal(typeof collapse, "function");
  collapse();
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[1,3]");
  assert.equal(evaluate("previewParents.get(2)"), 1);
  assert.equal(evaluate("scene.moments[0][3][0][0]"), 2);
  await evaluate("pending");
  assert.deepEqual(calls.splice(0).map(call => call.action), [{
    kind: "lane_layout",
    value: { folder_order: ["normal", "soft", "drum"], collapsed_folders: [], collapsed_families: ["clap"] }
  }]);

  const disclosure = get("lanes").children[0].children[0];
  disclosure.events.get("click")();
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[0,1,2,3]");
  assert.equal(evaluate("previewParents.has(2)"), false);
  get("lanes").children[2].children[1].events.get("click")();
  await evaluate("pending");
  assert.deepEqual(calls.pop(), { scene: true, action: { kind: "lane", row: 2, operation: "mute" } });
  assert.equal(calls.pop().action.value.collapsed_families.length, 0);

  collapse();
  assert.equal(evaluate("revealLayerPreview(1, 1000, 15)"), true);
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[0,1,2,3]");
  await evaluate("pending");
  assert.deepEqual(calls.map(call => call.action.kind), ["lane_layout", "lane_layout"]);
  assert.deepEqual(calls.map(call => call.action.value.collapsed_families), [["clap"], []]);
});
test("sample-set folders reorder headers and send group controls without changing lane identities", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(
    'view = { busy: false, hasDocument: true, activeLane: 0, tool: "Draw" };' +
    'scene = { path: "fixture.osu", rowHeight: 30, viewStart: 0, zoom: 1,' +
    ' lanes: [{ key: "normal", name: "Hitnormal", group: "normal" },' +
    ' { key: "soft", name: "Soft clap", group: "soft" },' +
    ' { key: "drum", name: "Drum finish", group: "drum" },' +
    ' { key: "free", name: "Free sample" }],' +
    ' moments: [[1000, 80, true, [[2, false]]]] }; renderLanes();'
  );
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["normal",0,"soft",1,"drum",2,3]');

  const normal = get("lanes").children[0];
  normal.children[1].events.get("click")();
  await evaluate("pending");
  assert.deepEqual(calls.pop(), { scene: true, action: { kind: "group_mix", set: "normal", operation: "mute" } });

  const drum = get("lanes").children[4];
  const transfer = {
    data: new Map(), setData(type, value) { this.data.set(type, value); },
    getData(type) { return this.data.get(type) ?? ""; },
    get types() { return [...this.data.keys()]; },
  };
  drum.events.get("dragstart")({ dataTransfer: transfer });
  normal.events.get("drop")({ dataTransfer: transfer, clientY: 0, preventDefault() {} });
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["drum",2,"normal",0,"soft",1,3]');
  const hit = evaluate('drag = { surface: grid, start: 0, zoom: 1, rows: laneRows }; pointer({clientX: 1000, clientY: 45}, "down")');
  assert.equal(hit.row, 2);

  evaluate("drag = null;");
  get("lanes").children[0].children[0].events.get("click")();
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["drum","normal",0,"soft",1,3]');
  assert.equal(evaluate("previewParents.get(2)"), "drum");
  assert.equal(evaluate('revealLayerPreview("drum", 1000, 15)'), true);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["drum",2,"normal",0,"soft",1,3]');
  await evaluate("pending");
  assert.deepEqual(calls.map(call => call.action.kind), ["lane_layout", "lane_layout", "lane_layout"]);
  assert.deepEqual(calls.map(call => call.action.value.collapsed_folders), [[], ["drum"], []]);
  assert.deepEqual(calls.at(-1).action.value.folder_order, ["drum", "normal", "soft"]);
});
