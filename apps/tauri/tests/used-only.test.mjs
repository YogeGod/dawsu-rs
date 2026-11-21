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
        if (_name === "lane_add_candidates") {
          return [{ key: "snare", name: "Snare" }, { key: "new-kick", name: "New kick.wav" }];
        }
        if (_name === "pick_drum_map") return null;
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

// renderLanes() hands the Svelte lane headers plain rows; their clicks come back through laneHeaderAction().
const headerRows = evaluate => JSON.parse(evaluate("JSON.stringify(laneHeaders.rows)"));
const laneNames = evaluate => headerRows(evaluate).map(row => row.name);
const headerAction = (evaluate, ...args) => evaluate("laneHeaderAction(" + args.map(arg => JSON.stringify(arg)).join(", ") + ")");
const rowAt = (evaluate, position) => headerRows(evaluate)[position].row;

test("Add restores a chosen lane and Delete keeps its confirmed difficulty, identity and revision", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0 };
    scene = { path: "first.osu", laneRevision: 7, rowHeight: 30,
      lanes: [
        { sourceKey: "clap", name: "Clap", hitCount: 2, canDelete: true },
        { sourceKey: "snare", name: "Snare", hitCount: 0, canDelete: true }
      ],
      laneOrder: [0],
      moments: []
    };
    let opened;
    modal = (...args) => { opened = args; };
    closeModal = () => {};
    renderLanes();
  `);
  evaluate("openAddLane()");
  await evaluate("pending");
  assert.deepEqual(calls.shift(), { map: "first.osu" });
  assert.equal(evaluate("opened[1]"), "Add lane");
  evaluate('opened[2][0].children[1].value = "new-kick"');
  evaluate('opened[3].find(([label]) => label === "Add")[1]()');
  await evaluate("pending");
  assert.deepEqual(calls.pop().action, { kind: "show_lane", target: { map: "first.osu", lane: "new-kick" } });

  evaluate("view.activeLane = 0;");
  evaluate("deleteActiveLane()");
  assert.equal(evaluate("opened[1]"), "Delete lane");
  assert.match(evaluate("opened[2][0].textContent"), /Clap.*2 hits/);
  assert.equal(calls.length, 0);
  evaluate('view.activeLane = 1; scene.laneRevision = 8; opened[3].find(([label]) => label === "Delete")[1]()');
  await evaluate("pending");
  assert.deepEqual(calls.pop().action, {
    kind: "delete_lane", target: { map: "first.osu", lane: "clap" }, revision: 7,
  });
});

test("the active lane colour picker applies and resets a faint tint for its difficulty", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0 };
    scene = { path: "first.osu", rowHeight: 30,
      lanes: [{ name: "Clap", family: "normal-hitclap", colourKey: "Skin|normal-hitclap" }], moments: [] };
    let opened;
    modal = (...args) => { opened = args; };
    closeModal = () => {};
  `);
  const open = () => {
    evaluate("view.activeLane = 0");
    evaluate("inspectorAction('laneColour')");
  };
  open();
  assert.equal(evaluate("opened[2][0].children.length"), 72);
  evaluate('opened[2][0].children[12].events.get("change")()');
  evaluate('opened[3].find(([label]) => label === "Apply")[1]()');
  await evaluate("pending");
  assert.equal(evaluate("laneHeaders.rows[0].colour"), "#388ada");
  assert.equal(evaluate("laneTint(scene.lanes[0])"), "#388ada14");
  const saved = calls.pop().action;
  assert.equal(saved.path, "first.osu");
  assert.equal(saved.persist, true);
  assert.deepEqual(saved.value.lane_colours, { "Skin|normal-hitclap": "#388ada" });

  open();
  evaluate('opened[3].find(([label]) => label === "Clear colour")[1]()');
  await evaluate("pending");
  // Clearing the mapper's tint leaves the lane grey: no colour on the header, no row tint.
  assert.equal(evaluate("laneHeaders.rows[0].colour"), null);
  assert.equal(evaluate("laneTint(scene.lanes[0])"), null);
  assert.deepEqual(calls.pop().action.value.lane_colours, {});

  open();
  evaluate('scene.path = "second.osu"; opened[3].find(([label]) => label === "Apply")[1]()');
  await evaluate("pending");
  assert.equal(calls.length, 0);
});

test("Drums view keeps aliases at distinct positions and routes them to the same sample lane", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0, drumView: { enabled: true, name: "Kit" } };
    scene = { path: "fixture.osu", rowHeight: 30, viewStart: 0, zoom: 1, lanes: [
      {key: "clap", name: "Clap", sample: "clap.wav"},
      {key: "finish", name: "Finish"},
    ], laneOrder: [0, 1], moments: [[1000, 100, true, [[0, false]]]],
    drumRows: [
      {row: 0, name: "Snare"}, {row: null, name: "Ghost snare"},
      {row: 0, name: "Rim"}, {row: 1, name: null}
    ]};
    renderLanes();
  `);
  assert.deepEqual(laneNames(evaluate), ["Snare", "Ghost snare", "Rim", "Finish"]);
  assert.equal(evaluate("JSON.stringify(laneVisualPositions.get(0))"), "[0,2]");
  headerAction(evaluate, "select", rowAt(evaluate, 2), {});
  await evaluate("pending");
  assert.deepEqual(calls.pop().action, { kind: "lane", row: 0, operation: "audition" });
  const action = evaluate(`drag = { surface: grid, start: 0, zoom: 1, rows: laneRows };
    pointer({clientX: 1000, clientY: 75}, "down")`);
  assert.equal(action.row, 0);
  evaluate("drag = null; view.drumView = { enabled: false, name: 'Kit' }; renderLanes();");
  assert.deepEqual(laneNames(evaluate), ["Clap", "Finish"]);
  get("drums-view").events.get("click")();
  await evaluate("pending");
  assert.deepEqual(calls.pop(), { map: "fixture.osu", on: true });
  evaluate("delete view.drumView;");
  get("drums-view").events.get("click")();
  await evaluate("pending");
  assert.deepEqual(calls.pop(), { map: "fixture.osu" });
});

test("lane selection scopes volume independently of hit selection and skips invisible rows in a Shift range", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 1 };
    scene = { path: "Scope.osu", rowHeight: 30, lanes: [
      {key: "A", name: "A"}, {key: "hidden", name: "Hidden"},
      {key: "C", name: "C"}, {key: "D", name: "D"}
    ], laneOrder: [0, 2, 3], selection: {count: 2, start: 1000, object: "Circle"}, moments: [] };
    renderLanes();
  `);
  evaluate("inspectorAction('scope', 'selected')");
  await evaluate("pending");
  assert.deepEqual(calls.pop().action, {kind: "volume_scope", path: "Scope.osu", lanes: []});
  const click = (row, event = {}) => headerAction(evaluate, "select", rowAt(evaluate, row), event);
  click(0);
  await evaluate("pending");
  calls.length = 0;
  click(2, {shiftKey: true});
  await evaluate("pending");
  assert.deepEqual(calls.splice(0).map(call => call.action), [
    {kind: "volume_scope", path: "Scope.osu", lanes: ["A", "C", "D"]},
  ]);
  assert.equal(evaluate("scene.selection.count"), 2);
  assert.ok(headerRows(evaluate)[1].selected);
  click(1, {metaKey: true});
  await evaluate("pending");
  assert.deepEqual(calls.pop().action.lanes, ["A", "D"]);
  evaluate("inspectorAction('scope', 'all')");
  await evaluate("pending");
  assert.deepEqual(calls.pop().action.lanes, null);
});

test("lane mix buttons send the selected sample identities or select a new single lane", async () => {
  const { calls, evaluate, get } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true };
    scene = { path: "Mix.osu", rowHeight: 30, moments: [], lanes: [
      {key: "slot-a", sourceKey: "sample-a", name: "A"},
      {key: "slot-b", sourceKey: "sample-b", name: "B"},
      {key: "slot-c", sourceKey: "sample-c", name: "C"}
    ]};
    renderLanes();
  `);
  const click = (row, button, event) => button
    ? headerAction(evaluate, "mix", rowAt(evaluate, row), ["mute", "solo"][button - 1])
    : headerAction(evaluate, "select", rowAt(evaluate, row), event);
  click(0, 0, {});
  click(1, 0, { ctrlKey: true });
  await evaluate("pending");
  calls.length = 0;
  click(1, 1);
  await evaluate("pending");
  assert.deepEqual(calls.pop().action, {
    kind: "lane_selection_mix", target: { map: "Mix.osu", lane: "sample-b" },
    operation: "mute", lanes: ["sample-a", "sample-b"],
  });
  click(2, 2);
  await evaluate("pending");
  assert.deepEqual(calls.pop().action, {
    kind: "lane_selection_mix", target: { map: "Mix.osu", lane: "sample-c" },
    operation: "solo", lanes: ["sample-c"],
  });
  assert.equal(evaluate("JSON.stringify([...selectionLaneKeys])"), '["slot-c"]');
});

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
  const { calls, evaluate } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0, tool: "Draw" };
    scene = {
      path: "fixture.osu", rowHeight: 30,
      lanes: [{ key: "A", name: "Kick" }, { key: "B", name: "Clap", sourceKey: "snare-file", sample: "percussion/snare.wav" }],
      moments: [[1000, 80, true, [[1, false]]]]
    };
  `);
  evaluate('popoverAction("laneFilter", "  SNARE ")');
  assert.deepEqual(laneNames(evaluate), ["Clap"]);
  headerAction(evaluate, "mix", rowAt(evaluate, 0), "mute");
  await evaluate("pending");
  assert.equal(calls.at(-1).action.target.lane, "snare-file");
  evaluate('popoverAction("laneFilter", "kick")');
  assert.deepEqual(laneNames(evaluate), ["Kick"]);
});

test("Used only compacts rows without renumbering the lanes sent to Rust", async () => {
  const { calls, evaluate } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0, tool: "Draw" };
    scene = {
      path: "fixture.osu", rowHeight: 30, viewStart: 0, zoom: 1,
      lanes: [{ key: "A", name: "A" }, { key: "B", name: "B" }, { key: "C", sourceKey: "sample-c", name: "C" }],
      moments: [[1000, 80, true, [[0, false], [2, false]]]]
    };
  `);

  evaluate('popoverAction("usedOnly", true)');
  assert.deepEqual(laneNames(evaluate), ["A", "C"]);
  headerAction(evaluate, "mix", rowAt(evaluate, 1), "mute");
  await evaluate("pending");
  assert.deepEqual(calls.pop(), { scene: true, action: { kind: "lane_selection_mix", target: { map: "fixture.osu", lane: "sample-c" }, operation: "mute", lanes: ["sample-c"] } });

  const event = { clientX: 160, clientY: 44 };
  const pointer = evaluate(`
    drag = { surface: grid, start: 0, zoom: 1, rows: laneRows };
    pointer(${JSON.stringify(event)}, "down");
  `);
  assert.equal(pointer.row, 2);
  assert.equal(evaluate("lanePositions.get(2)"), 1);
  evaluate("drag = null; scene.moments[0][3] = [[2, false]]; renderLanes();");
  assert.deepEqual(laneNames(evaluate), ["A", "C"]);
  evaluate('popoverAction("usedOnly", false)');
  assert.deepEqual(laneNames(evaluate), ["A", "B", "C"]);
  evaluate('popoverAction("usedOnly", true)');
  assert.deepEqual(laneNames(evaluate), ["C"]);
});



test("collapsing sample layers keeps their model rows separate and lets them expand", async () => {
  const { calls, evaluate } = await frontend();
  evaluate(
    'view = { busy: false, hasDocument: true, activeLane: 0, tool: "Select" };' +
    'scene = { path: "fixture.osu", rowHeight: 30,' +
    ' lanes: [{ key:"A", name:"Default clap", family:"clap", index:0 },' +
    ' { key:"B", name:"Map clap", family:"clap", index:1 },' +
    ' { key:"C", sourceKey:"sample-c", name:"Clap 2", family:"clap", index:2 }, { key:"D", name:"Free sample" }],' +
    ' moments: [[1000, 80, true, [[2, false]]]] }; renderLanes();'
  );
  const collapse = () => evaluate('popoverAction("collapseLayers")');
  collapse();
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[1,3]");
  assert.equal(evaluate("previewParents.get(2)"), 1);
  assert.equal(evaluate("scene.moments[0][3][0][0]"), 2);
  await evaluate("pending");
  assert.deepEqual(calls.splice(0).map(call => call.action), [{
    kind: "lane_layout", path: "fixture.osu", persist: false,
    value: { folder_order: ["normal", "soft", "drum"], collapsed_folders: [], collapsed_families: ["clap"], lane_colours: {} }
  }]);

  assert.deepEqual(headerRows(evaluate)[0].family, { collapsed: true, label: "Expand Map clap layers" });
  headerAction(evaluate, "family", rowAt(evaluate, 0));
  // The main lane (index 1) comes first, then its layers by number.
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[1,0,2,3]");
  assert.equal(evaluate("previewParents.has(2)"), false);
  headerAction(evaluate, "mix", rowAt(evaluate, 2), "mute");
  await evaluate("pending");
  assert.deepEqual(calls.pop(), { scene: true, action: { kind: "lane_selection_mix", target: { map: "fixture.osu", lane: "sample-c" }, operation: "mute", lanes: ["sample-c"] } });
  assert.equal(calls.pop().action.value.collapsed_families.length, 0);

  // With Select, a click on a layer's outline on the folded main lane unfolds it.
  collapse();
  evaluate('view.tool = "Select"');
  assert.equal(evaluate("revealLayerPreview(1, 1000, 15)"), true);
  assert.equal(evaluate("JSON.stringify(laneRows)"), "[1,0,2,3]");
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

  headerAction(evaluate, "folderMix", "normal", "mute");
  await evaluate("pending");
  assert.deepEqual(calls.pop(), { scene: true, action: { kind: "group_mix", set: "normal", operation: "mute" } });

  // Dropping Drum set on the top half of Normal set's header (the part works out the half).
  headerAction(evaluate, "folderMove", "drum", "normal", false);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["drum",2,"normal",0,"soft",1,3]');
  const hit = evaluate('drag = { surface: grid, start: 0, zoom: 1, rows: laneRows }; pointer({clientX: 1000, clientY: 45}, "down")');
  assert.equal(hit.row, 2);

  evaluate("drag = null;");
  headerAction(evaluate, "folder", "drum");
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["drum","normal",0,"soft",1,3]');
  assert.equal(evaluate("previewParents.get(2)"), "drum");
  assert.equal(evaluate('revealLayerPreview("drum", 1000, 15)'), true);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["drum",2,"normal",0,"soft",1,3]');
  await evaluate("pending");
  assert.deepEqual(calls.map(call => call.action.kind), ["lane_layout", "lane_layout", "lane_layout"]);
  assert.deepEqual(calls.map(call => call.action.value.collapsed_folders), [[], ["drum"], []]);
  assert.deepEqual(calls.at(-1).action.value.folder_order, ["drum", "normal", "soft"]);
});

test("lane names and moves keep sample identities and persist within one difficulty", async () => {
  const { calls, evaluate } = await frontend();
  evaluate(`
    view = { busy: false, hasDocument: true, activeLane: 0 };
    scene = { path: "Layout.osu", rowHeight: 30, moments: [],
      lanes: [{colourKey:"Skin|normal-hitnormal", key:"normal", name:"Kick", group:"normal"},
        {colourKey:"Skin|normal-hitclap", key:"clap", name:"Clap", group:"normal"},
        {colourKey:"Skin|soft-hitnormal", key:"soft", name:"Soft", group:"soft"}] };
    modal = (_kind, title, content, buttons) => { globalThis.renameDialog = { title, content, buttons }; };
    closeModal = () => {};
    renderLanes();
    renameLane(1);
  `);
  const rename = evaluate("renameDialog");
  assert.equal(rename.title, "Rename lane");
  rename.content[0].children[1].value = "  Snare  ";
  rename.buttons[1][1]();
  await evaluate("pending");
  assert.equal(calls.at(-1).action.value.lane_names["Skin|normal-hitclap"], "Snare");
  evaluate('popoverAction("laneFilter", "snare")');
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["normal",1]');
  evaluate('popoverAction("laneFilter", "")');
  evaluate("moveLane(1,-1); moveLane(1,-1); moveLane(2,-1)");
  await evaluate("pending");
  const saved = calls.at(-1).action;
  assert.equal(saved.persist, true);
  assert.deepEqual(saved.value.lane_order, ["Skin|normal-hitclap", "Skin|normal-hitnormal", "Skin|soft-hitnormal"]);
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["normal",1,0,"soft",2]');
  evaluate("hydrateLaneLayout(null); renderLanes()");
  assert.equal(evaluate("laneDisplayName(scene.lanes[1])"), "Clap");
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["normal",0,1,"soft",2]');
  evaluate("hydrateLaneLayout(" + JSON.stringify(saved.value) + "); renderLanes(); renameLane(1);");
  assert.equal(evaluate("laneDisplayName(scene.lanes[1])"), "Snare");
  assert.equal(evaluate("JSON.stringify(laneRows)"), '["normal",1,0,"soft",2]');
  const reset = evaluate("renameDialog");
  reset.content[0].children[1].value = "";
  reset.buttons[1][1]();
  await evaluate("pending");
  assert.equal(calls.at(-1).action.value.lane_names, undefined);
  assert.equal(evaluate("laneDisplayName(scene.lanes[1])"), "Clap");
});