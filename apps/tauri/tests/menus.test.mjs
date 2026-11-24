import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

import * as commandList from "../ui/src/lib/commands.js";

const code = await readFile(new URL("../dist/menus.js", import.meta.url), "utf8");
const commandsSource = await readFile(new URL("../ui/src/lib/commands.js", import.meta.url), "utf8");
const html = await readFile(new URL("../dist/index.html", import.meta.url), "utf8");
// The Inspector's Lane and Difficulty buttons live in a Svelte part and keep the ids the menus click.
const inspectorPart = (await Promise.all(["Selection", "Lane", "Difficulty"].map(name =>
  readFile(new URL(`../ui/src/parts/Inspector${name}.svelte`, import.meta.url), "utf8")))).join("\n");
// The Add lane row ends the lane column, a Svelte part, and keeps the id the menus click.
const lanesPart = await readFile(new URL("../ui/src/parts/LaneHeaders.svelte", import.meta.url), "utf8");
const statusBarPart = await readFile(new URL("../ui/src/parts/StatusBar.svelte", import.meta.url), "utf8");
const gridOptionsPart = await readFile(new URL("../ui/src/parts/GridOptions.svelte", import.meta.url), "utf8");
const laneOptionsPart = await readFile(new URL("../ui/src/parts/LaneOptions.svelte", import.meta.url), "utf8");
const css = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");
const catalogue = (await Promise.all([
  "../../../crates/dawsu-application/src/commands.rs",
  "../../../crates/dawsu-editor/src/editing_commands.rs",
  "../../../crates/dawsu-editor/src/tools.rs",
].map(path => readFile(new URL(path, import.meta.url), "utf8")))).join(" ");

function fixture() {
  const elements = new Map();
  const calls = [];
  const modals = [];
  const responses = {};
  const listeners = new Map();
  const document = {
    addEventListener(name, handler) { listeners.set(name, handler); },
    createElement: element,
    createTextNode(textContent) { return { textContent }; },
  };
  function element(type) {
    return {
      type, children: [], style: {}, events: new Map(), attributes: {}, textContent: "",
      addEventListener(name, handler) { this.events.set(name, handler); },
      setAttribute(name, value) { this.attributes[name] = value; },
      removeAttribute(name) { delete this.attributes[name]; },
      getAttribute(name) { return this.attributes[name] ?? null; },
      append(...children) { this.children.push(...children); },
      prepend(child) { this.children.unshift(child); },
      replaceChildren(...children) { this.children = children; },
      focus() { document.activeElement = this; },
      click() { if (!this.disabled) this.events.get("click")?.(); },
      contains(target) { return this === target || this.children.some(child => child.contains?.(target)); },
      getBoundingClientRect() { return { left: 8, bottom: 45 }; },
      querySelectorAll(type) {
        return this.children.flatMap(child => [...(child.type === type ? [child] : []), ...child.querySelectorAll?.(type) ?? []]);
      },
    };
  }
  // Stands in for EditorMenu.svelte: draws the rows menus.js hands over into #editor-menu, keeping
  // each button between refreshes of the same open menu, as Svelte updates them in place.
  let drawn = null;
  function drawMenu(model) {
    const popup = get("editor-menu");
    if (!model) {
      drawn = null;
      popup.replaceChildren();
      return;
    }
    if (drawn?.open !== model.open) drawn = { open: model.open, buttons: new Map() };
    const button = row => {
      const node = drawn.buttons.get(row.index) ?? element("button");
      drawn.buttons.set(row.index, node);
      Object.assign(node, { textContent: row.label, className: row.className, disabled: row.disabled, tabIndex: -1 });
      node.attributes.role = row.role;
      if (row.checked === undefined) delete node.attributes["aria-checked"];
      else node.attributes["aria-checked"] = String(row.checked);
      node.children = [
        ...row.icon ? [Object.assign(element("img"), { className: "button-icon", src: "studio-assets/icons/" + row.icon + ".svg" })] : [],
        ...row.chip ? [Object.assign(element("kbd"), { textContent: row.keys, hidden: !row.keys })] : [],
        ...row.switch ? [Object.assign(element("span"), { className: "menu-switch" })] : [],
      ];
      node.events.set("click", () => context.menuAction(row.index));
      return node;
    };
    popup.replaceChildren(...model.rows.map(row => {
      if (row.type === "item") return button(row.item);
      const node = element("div");
      if (row.type === "separator") return Object.assign(node, { className: "menu-separator", attributes: { role: "separator" } });
      const heading = Object.assign(element("div"), { className: "menu-choice-label", textContent: row.label });
      const choices = Object.assign(element("div"), { className: "menu-choices" });
      choices.append(...row.choices.map(button));
      node.append(heading, choices);
      return Object.assign(node, { className: "menu-choice-group", attributes: { role: "group", "aria-label": row.label } });
    }));
  }
  function get(id) {
    if (!elements.has(id)) elements.set(id, Object.assign(element("div"), { id }));
    return elements.get(id);
  }
  get("editor-menus").append(...["file", "edit", "playback", "map", "view"].map(name => get("menu-" + name)));
  get("editor-menu").offsetWidth = 330;
  get("editor-menu").offsetHeight = 220;
  const speedButtons = [.25, .5, .75, 1, 1.25, 1.5].map(value => ({
    dataset: { playbackSpeed: String(value) }, textContent: (value * 100) + "%",
  }));
  const view = { speed: 1, playbackStart: "cursor", follow: false, activeLane: 0 };
  const scene = { path: "Test.osu", selection: {count:1}, lanes: [
    {sourceKey:"first", canDelete:false}, {sourceKey:"second", canDelete:true, canReplace:true, canAddLayer:true, colourKey:"second"},
  ] };
  const commands = { entries: [["undo", "Undo", ["Alt+U"]], ["save", "Save", ["Ctrl+S"]],
    ["cycle", "Cycle", ["Alt+C"]], ["addNumberedLayer", "Add numbered layer", ["Ctrl+L"]]] };
  const window = { innerWidth: 900, innerHeight: 640, addEventListener() {},
    dawsuUi: { commands: commandList, menu: drawMenu } };
  const context = createContext({
    $: get, document, window, view, scene, speedButtons, drag: null, dialog: {},
    lanePositions: new Map([[0,0],[1,1]]),
    // app.js: the active lane when it can be deleted, and the Delete lane dialog.
    deletableLane: () => scene.lanes[view.activeLane]?.canDelete ? scene.lanes[view.activeLane] : null,
    deleteActiveLane() { calls.push({ deleteActiveLane: view.activeLane }); },
    adjacentLaneForMove: () => null,
    run(task) { return task(); },
    async update(action) { calls.push(JSON.parse(JSON.stringify(action))); view.activeLane = action.row; },
    playbackStartChoices: [
      ["cursor", "Cursor"], ["left", "Left locator"], ["selection", "Selection"],
      ["selection_or_left", "Selection or left locator"],
    ],
    makeButton(label, onClick) {
      const button = element("button");
      button.textContent = label;
      button.addEventListener("click", onClick);
      return button;
    },
    command(id) { calls.push({ id }); },
    act(action) {
      calls.push(JSON.parse(JSON.stringify(action)));
      if (action.kind === "playback_start") view.playbackStart = action.value;
      if (action.kind === "follow") view.follow = action.on;
    },
    async invoke(name, args) {
      if (name !== "editor_shortcuts") calls.push([name, args]);
      if (name in responses) return typeof responses[name] === "function" ? responses[name](args) : responses[name];
      return name === "editor_shortcuts" ? commands : { placed: true };
    },
    modal(kind, title, content, buttons, cancel) { modals.push({ kind, title, content, buttons, cancel }); },
    closeModal() { calls.push("closeModal"); },
    paragraph(textContent) { return Object.assign(element("p"), { textContent }); },
    input(type, value) { return Object.assign(element("input"), { type, value }); },
    labeled(label, control) { const node = element("label"); node.append(label, control); return node; },
    show(next) { calls.push({ show: next }); },
    openSiblingDifficulty(path) { calls.push({ sibling: path }); },
    panelOn: panel => !!view.panels?.[panel],
    togglePanel(panel) { calls.push({ togglePanel: panel }); },
  });
  runInContext(code, context);
  function key(key, handler = listeners.get("keydown")) {
    const event = { key, preventDefault() { this.defaultPrevented = true; } };
    handler(event);
    return !!event.defaultPrevented;
  }
  return { document, get, view, scene, window, calls, listeners, key, modals, responses, context };
}

test("menus read current shortcut chips and keep arrow, Escape, Tab and outside-click handling local", async () => {
  const { document, get, calls, listeners, key } = fixture();
  const trigger = get("menu-edit");
  const popup = get("editor-menu");
  assert.equal(key("ArrowDown", trigger.events.get("keydown")), true);
  await Promise.resolve();
  assert.equal(document.activeElement.textContent, "Undo");
  const undo = document.activeElement;
  assert.equal(undo.children.find(child => child.type === "kbd").textContent, "Alt+U");
  assert.equal(trigger.attributes["aria-expanded"], "true");

  assert.equal(key("ArrowDown"), true);
  assert.equal(document.activeElement.textContent, "Cut");
  assert.equal(key("Escape"), true);
  assert.equal(popup.hidden, true);
  assert.equal(document.activeElement, trigger);
  assert.equal(trigger.attributes["aria-expanded"], "false");

  trigger.click();
  document.activeElement.click();
  assert.deepEqual(calls, [{ id: "undo" }]);
  assert.equal(popup.hidden, true);
  get("menu-map").click();
  assert.equal(key("Tab"), false);
  assert.equal(document.activeElement, get("menu-map"));
  get("menu-map").click();
  listeners.get("pointerdown")({ target: get("outside") });
  assert.equal(popup.hidden, true);
});

test("lane menu activates the right-clicked lane without audition before opening an existing action", async () => {
  const {document,get,calls,key}=fixture();
  const lanes=get("lanes"), name=get("second-name"), menu=get("editor-menu");
  name.dataset={laneRow:"1"}; name.closest=()=>name; lanes.append(name);
  const event={target:name,clientX:100,clientY:140,preventDefault(){this.defaultPrevented=true;}};
  assert.equal(typeof lanes.events.get("contextmenu"),"function");
  lanes.events.get("contextmenu")(event);
  assert.equal(event.defaultPrevented,true);
  assert.equal(menu.attributes["aria-label"],"Lane actions");
  assert.deepEqual(calls,[]);
  assert.equal(key("Escape"),true);
  assert.equal(document.activeElement,name);

  lanes.events.get("contextmenu")(event);
  // The lane toolbar has no Delete lane button (v3): the row runs app.js's deleteActiveLane().
  menu.children.find(button=>button.textContent==="Delete lane…").click();
  await new Promise(setImmediate);
  assert.deepEqual(calls,[{kind:"lane",row:1,operation:"activate"},{deleteActiveLane:1}]);
});

test("grid right-click suppresses the browser menu without opening the tool picker; openEditorMenu('tool') opens the seven tools", () => {
  const { document, get, calls, key, view, context } = fixture();
  const popup = get("editor-menu"), grid = get("grid");
  view.tool = "Erase";
  const click = { clientX: 895, clientY: 637, preventDefault() { this.defaultPrevented = true; } };
  assert.equal(typeof grid.events.get("contextmenu"), "function", "grid suppresses browser context menu");
  grid.events.get("contextmenu")(click);
  assert.equal(click.defaultPrevented, true);
  assert.notEqual(popup.hidden, false, "right-click on grid does not open the tool picker");
  assert.equal(popup.children.length, 0);
  assert.deepEqual(calls, []);

  context.window.openEditorMenu("tool", false, { x: 895, y: 637, trigger: grid, label: "Timeline tools" });
  assert.equal(popup.attributes["aria-label"], "Timeline tools");
  assert.equal(popup.style.left, "566px");
  assert.equal(popup.style.top, "416px");
  assert.deepEqual(popup.children.map(button => button.textContent), [
    "Select", "Draw", "Erase", "Mute", "Grab", "Range", "Ramp",
  ]);
  assert.equal(popup.children[2].attributes["aria-checked"], "true");
  assert.deepEqual(calls, []);
  assert.equal(key("Escape"), true);
  assert.equal(document.activeElement, grid);

  context.window.openEditorMenu("tool", false, { x: 895, y: 637, trigger: grid, label: "Timeline tools" });
  key("ArrowDown");
  assert.equal(document.activeElement.textContent, "Draw");
  document.activeElement.click();
  assert.deepEqual(calls, [{ kind: "tool", name: "Draw" }]);
  assert.equal(popup.hidden, true);
  assert.equal(document.activeElement, grid);
});

test("Playback keeps its segmented choices and switches open, Map keeps Generate per-object and File saves", () => {
  const { get, calls } = fixture();
  const popup = get("editor-menu");
  get("menu-playback").click();
  const group = label => popup.children.find(child => child.attributes["aria-label"] === label);
  const choices = label => group(label).children[1].children;
  assert.equal(choices("Start playing from").length, 4);
  const left = choices("Start playing from").find(button => button.textContent === "Left locator");
  left.click();
  assert.equal(left.attributes["aria-checked"], "true");
  assert.equal(popup.hidden, false);
  const follow = popup.children.find(child => child.textContent === "Follow playhead");
  assert.equal(follow.attributes.role, "menuitemcheckbox");
  follow.click();
  assert.equal(follow.attributes["aria-checked"], "true");
  assert.equal(choices("Speed · pitch stays the same").length, 6);
  choices("Speed · pitch stays the same").at(-1).click();
  assert.deepEqual(calls, [
    { kind: "playback_start", value: "left" }, { kind: "follow", on: true }, { kind: "speed", value: 1.5 },
  ]);

  get("menu-map").click();
  const style = choices("Generate export style")[0];
  assert.equal(style.textContent, "Per-object");
  assert.equal(style.attributes["aria-checked"], "true");
  assert.equal(style.disabled, true);
  get("menu-file").click();
  popup.children.find(child => child.textContent === "Save").click();
  assert.deepEqual(calls.at(-1), { id: "save" });
  assert.equal(popup.hidden, true);
});

const menuButtons = popup => popup.children.flatMap(child =>
  child.className === "menu-choice-group" ? child.children[1].children : [child])
  .filter(child => child.type === "button");
const menuLabels = popup => popup.children.map(child => child.className === "menu-choice-group"
  ? child.attributes["aria-label"] + ": " + child.children[1].children.map(choice => choice.textContent).join(" | ")
  : child.attributes.role === "separator" ? "—" : child.textContent);

test("the menu bar runs File, Edit, Playback, Map and View in the design's order", () => {
  const menus = html.match(/<nav id="editor-menus"[^>]*>(.*?)<\/nav>/s)?.[1];
  assert.deepEqual([...menus.matchAll(/id="menu-(\w+)"/g)].map(match => match[1]), commandList.menuBar);
  assert.deepEqual(commandList.menuBar, ["file", "edit", "playback", "map", "view"]);
  assert.ok(code.includes("const menuNames = " + JSON.stringify(commandList.menuBar).replaceAll(",", ", ") + ";"),
    "menus.js wires the bar's buttons in the list's order");
  const { document, get, key } = fixture();
  get("menu-file").focus();
  key("ArrowRight", get("menu-file").events.get("keydown"));
  assert.equal(document.activeElement, get("menu-edit"));
  key("ArrowRight", get("menu-edit").events.get("keydown"));
  assert.equal(document.activeElement, get("menu-playback"));
  key("ArrowLeft", get("menu-file").events.get("keydown"));
  assert.equal(document.activeElement, get("menu-view"));
  key("ArrowRight", get("menu-view").events.get("keydown"));
  assert.equal(document.activeElement, get("menu-file"));
});

test("each menu groups the editor's existing commands like the design, with no submenus", () => {
  const { get } = fixture();
  get("difficulty").options = [{ value: "Test.osu", text: "Hard" }, { value: "Insane.osu", text: "Insane" }];
  const popup = get("editor-menu");
  const open = name => { get("menu-" + name).click(); return menuLabels(popup); };
  assert.deepEqual(open("file"), [
    "Open map…", "Open from osu!", "Import .osz…", "Import MIDI…", "Import mania hitsounds…", "Open map folder", "—",
    "Save", "Revert to saved…", "Revert mapset to original…", "Undo revert", "Export…", "Export MIDI…", "—",
    "Start screen", "Folders…", "—", "Preferences…",
  ]);
  assert.deepEqual(open("edit"), [
    "Undo", "Redo", "—",
    "Cut", "Copy", "Paste at playhead", "Duplicate", "Delete", "Select all", "—",
    "Quantize…", "Repeat…", "Pattern stamp…", "Edit range…", "Scale by volume…", "Place sound set…", "Save selection as sound set…",
    "Sound set placement…", "—",
    "Protect hitnormals from deletion", "Audition when placing hits",
  ]);
  assert.deepEqual(open("playback"), [
    "Play / pause", "Stop", "Go to start", "Go to end", "—",
    "Start playing from: Cursor | Left locator | Selection | Selection or left locator", "—",
    "Return on stop", "Follow playhead", "Cycle between L and R", "Locators to selection", "Play locator range", "—",
    "Speed · pitch stays the same: 25 | 50 | 75 | 100 | 125 | 150", "—",
    "Mute song", "Monitor drum stem", "Attach drum stem…", "Clear drum stem", "—", "Audio output…",
  ]);
  assert.deepEqual(open("map"), [
    "Difficulty: Hard | Insane", "—",
    "New hitsound difficulty…", "—",
    "Generate hitsound difficulty…", "Copy hitsounds…", "Shared Base layer…", "Drum hit detection…", "Node graph…",
    "Keysounds & VST3…", "Generate export style: Per-object", "—",
    "Hitsounding mode", "—",
    "Add bookmark", "Previous bookmark", "Next bookmark", "—",
    "Add lane…", "Delete active lane…",
  ]);
  assert.deepEqual(open("view"), [
    "Browser", "Inspector", "Volume and Mixer", "Map preview", "Lane names", "—",
    "Song waveform", "Sound-first · hide missing-object warnings", "Object lines", "Used lanes only", "Collapse sample layers", "Filter lanes…", "Taller lanes",
    "Shorter lanes", "—",
    "Zoom in", "Zoom out", "Fit difficulty", "—",
    "Time signatures…", "Open drum map…", "Edit layout…", "View settings…", "—",
    "Quick start tour", "Shortcuts",
  ]);
  for (const name of ["file", "edit", "playback", "map", "view"]) {
    get("menu-" + name).click();
    for (const button of menuButtons(popup)) assert.notEqual(button.attributes["aria-haspopup"], "menu");
  }
});

test("menus only click ribbon controls that exist in the window", () => {
  const ids = new Set([
    ...[...code.matchAll(/(?:click|\$)\("([\w-]+)"\)/g)].map(match => match[1]),
    ...[...commandsSource.matchAll(/\["(?:onLane", ")?click", "([\w-]+)"\]/g)].map(match => match[1]),
  ]);
  for (const id of ["audio", "used-only", "ruler-waveform", "ruler-sound-first", "export-entry"]) assert.ok(ids.has(id), id);
  for (const id of ids) assert.match(html + inspectorPart + lanesPart + statusBarPart + gridOptionsPart + laneOptionsPart, new RegExp('id="' + id + '"'), id);
});

const plain = value => JSON.parse(JSON.stringify(value));
const settle = () => new Promise(resolve => setTimeout(resolve, 0));
const button = (modal, label) => modal.buttons.find(([text]) => text === label);

function editItem(get, label) {
  get("menu-edit").click();
  return menuButtons(get("editor-menu")).find(item => item.textContent === label);
}

test("Place sound set lists this map's sets, then the library, and places one by its id", async () => {
  const { get, calls, modals, responses } = fixture();
  responses.sound_set_library = { configured: true, sets: [
    { id: "a", name: "Kick", sounds: 2, inMap: true, duplicate: false },
    { id: "b", name: "Bell", sounds: 1, inMap: false, duplicate: false },
    { id: "c", name: "Crash", sounds: 3, inMap: false, duplicate: true },
  ], unreadable: ["broken.dawsuset"], reading: false };
  editItem(get, "Place sound set…").click();
  await settle();
  const [dialog] = modals;
  assert.equal(dialog.title, "Place sound set");
  const list = dialog.content[0];
  assert.equal(list.type, "select");
  assert.deepEqual(plain(list.children.map(group => [group.label, group.children.map(option => [option.textContent, !!option.disabled])])), [
    ["In this map", [["Kick · 2 sounds", false]]],
    ["Library", [["Bell · 1 sound", false], ["Crash · 3 sounds · in the library twice, remove one copy", true],
      ["Could not read broken.dawsuset", true]]],
  ]);
  const message = dialog.content[1];
  assert.equal(message.textContent, "");
  assert.equal(list.value, "a");
  assert.deepEqual(plain(dialog.buttons.map(([label]) => label)), ["Cancel", "Browse…", "Place"]);
  button(dialog, "Place")[1]();
  assert.deepEqual(JSON.parse(JSON.stringify(calls.slice(-2))), ["closeModal", { kind: "place_library_sound_set", map: "Test.osu", id: "a" }]);
  // A set in the library twice can't be placed.
  list.value = "c";
  button(dialog, "Place")[1]();
  list.events.get("dblclick")({ target: { tagName: "OPTION", disabled: true } });
  assert.equal(calls.filter(call => call.kind === "place_library_sound_set").length, 1);
  assert.equal(message.textContent, "Choose a sound set to place.");
  list.value = "b";
  list.events.get("dblclick")({ target: { tagName: "OPTION", disabled: false } });
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), { kind: "place_library_sound_set", map: "Test.osu", id: "b" }, "double-clicking a set places it");
  button(dialog, "Browse…")[1]();
  await settle();
  assert.deepEqual(JSON.parse(JSON.stringify(calls.slice(-3))), ["closeModal", ["pick_sound_set", { map: "Test.osu" }], { show: { placed: true } }]);
});

test("Place sound set with an empty library says so and still offers Browse", async () => {
  const { get, modals, responses } = fixture();
  responses.sound_set_library = { configured: true, sets: [] };
  editItem(get, "Place sound set…").click();
  await settle();
  assert.deepEqual(plain(modals[0].content.map(node => node.textContent)),
    ["The library has no sound sets yet. Use Edit > Save selection as sound set to add one."]);
  assert.deepEqual(plain(modals[0].buttons.map(([label]) => label)), ["Cancel", "Browse…"]);
});

test("Place sound set says when the library is still being read or its folder is gone", async () => {
  const { get, calls, modals, responses } = fixture();
  responses.sound_set_library = { configured: true, sets: [], unreadable: [], reading: true };
  editItem(get, "Place sound set…").click();
  await settle();
  assert.deepEqual(plain(modals[0].content.map(node => node.textContent)), ["Still reading the sound set library. Try again."]);
  responses.sound_set_library = { configured: true, error: "Hitsound library folder not found: D:/Library", sets: [], unreadable: [] };
  button(modals[0], "Try again")[1]();
  await settle();
  assert.equal(calls.filter(call => Array.isArray(call) && call[0] === "sound_set_library").length, 2);
  assert.deepEqual(plain(modals[1].content.map(node => node.textContent)), ["Hitsound library folder not found: D:/Library"]);
  assert.deepEqual(plain(modals[1].buttons.map(([label]) => label)), ["Cancel", "Browse…", "Choose library folder…"]);
});

test("a list with only sets that can't be placed says so", async () => {
  const { get, modals, responses } = fixture();
  responses.sound_set_library = { configured: true, sets: [{ id: "c", name: "Crash", sounds: 3, inMap: false, duplicate: true }],
    unreadable: [], reading: false };
  editItem(get, "Place sound set…").click();
  await settle();
  assert.equal(modals[0].content[1].textContent, "None of these sound sets can be placed.");
});

test("without a hitsound library, both sound set items ask for one first", async () => {
  const { get, calls, view, modals, responses } = fixture();
  responses.sound_set_library = { configured: false, sets: [] };
  responses.pick_hitsound_library = { hitsoundLibrary: "Library" };
  editItem(get, "Place sound set…").click();
  await settle();
  assert.deepEqual(plain(modals[0].content.map(node => node.textContent)), ["Sound sets are kept in your hitsound library."]);
  assert.deepEqual(plain(modals[0].buttons.map(([label]) => label)), ["Cancel", "Browse…", "Choose library folder…"]);
  responses.sound_set_library = { configured: true, sets: [{ id: "a", name: "Kick", sounds: 2, inMap: false, duplicate: false }] };
  button(modals[0], "Choose library folder…")[1]();
  await settle();
  await settle();
  assert.ok(calls.some(call => Array.isArray(call) && call[0] === "pick_hitsound_library"));
  assert.equal(modals.at(-1).content[0].type, "select", "the list opens once a folder is chosen");
  view.hitsoundLibrary = null;
  editItem(get, "Save selection as sound set…").click();
  assert.equal(modals.at(-1).title, "Save selection as sound set");
  assert.deepEqual(plain(modals.at(-1).buttons.map(([label]) => label)), ["Cancel", "Choose library folder…"]);
});

test("Save selection as sound set asks for a name and asks again when the library has it", async () => {
  const { get, calls, view, scene, modals, responses } = fixture();
  view.hitsoundLibrary = "Library";
  scene.selection = null;
  assert.equal(editItem(get, "Save selection as sound set…").disabled, true, "needs a selection");
  get("menu-edit").click();
  scene.selection = { count: 3 };
  editItem(get, "Save selection as sound set…").click();
  const dialog = modals.at(-1);
  assert.equal(dialog.title, "Save selection as sound set");
  const [label, message] = dialog.content;
  const field = label.children[1];
  assert.equal(label.children[0], "Name");
  responses.save_sound_set = { nameTaken: "The library already has a sound set named \"Kick\". Choose another name." };
  field.value = "  ";
  button(dialog, "Save")[1]();
  await settle();
  assert.ok(!calls.some(call => Array.isArray(call) && call[0] === "save_sound_set"), "a blank name is not sent");
  assert.equal(message.textContent, "Name the sound set.");
  field.value = "Kick";
  button(dialog, "Save")[1]();
  await settle();
  assert.deepEqual(JSON.parse(JSON.stringify(calls.at(-1))), ["save_sound_set", { map: "Test.osu", name: "Kick" }]);
  assert.match(message.textContent, /already has a sound set named "Kick"/);
  assert.ok(!calls.includes("closeModal"), "the name prompt stays open");
  responses.save_sound_set = () => { throw "Select sounds at one time to save a set."; };
  button(dialog, "Save")[1]();
  await settle();
  assert.equal(message.textContent, "Select sounds at one time to save a set.");
  responses.save_sound_set = { status: "Saved sound set \"Kicks\"." };
  field.value = "Kicks";
  button(dialog, "Save")[1]();
  button(dialog, "Save")[1]();
  await settle();
  assert.equal(calls.filter(call => Array.isArray(call) && call[0] === "save_sound_set" && call[1].name === "Kicks").length, 1,
    "a second click while saving saves nothing more");
  assert.deepEqual(JSON.parse(JSON.stringify(calls.slice(-2))), ["closeModal", { show: { status: "Saved sound set \"Kicks\"." } }]);
});

test("Undo and Redo name the edit they would change", () => {
  const { get, scene } = fixture();
  const popup = get("editor-menu");
  Object.assign(scene, { history: ["Draw hit", "Move 4 hits"], historyPosition: 2 });
  get("menu-edit").click();
  assert.deepEqual(menuLabels(popup).slice(0, 2), ["Undo · Move 4 hits", "Redo"]);
  scene.historyPosition = 1;
  get("menu-edit").click();
  get("menu-edit").click();
  assert.deepEqual(menuLabels(popup).slice(0, 2), ["Undo · Draw hit", "Redo · Move 4 hits"]);
});

test("menu switches drive the existing toggles and the difficulty row opens a sibling", () => {
  const { get, calls, view, scene } = fixture();
  const popup = get("editor-menu");
  const item = label => menuButtons(popup).find(button => button.textContent === label);
  Object.assign(view, { audition: true, cycle: false });
  scene.protect = false;
  get("menu-edit").click();
  assert.equal(item("Protect hitnormals from deletion").attributes.role, "menuitemcheckbox");
  assert.equal(item("Protect hitnormals from deletion").attributes["aria-checked"], "false");
  assert.equal(item("Audition when placing hits").attributes["aria-checked"], "true");
  item("Protect hitnormals from deletion").click();
  item("Audition when placing hits").click();
  get("menu-playback").click();
  item("Cycle between L and R").click();
  item("Go to start").click();
  view.duration = 201500;
  get("menu-playback").click();
  item("Go to end").click();
  assert.deepEqual(calls, [
    { kind: "protect", on: true }, { kind: "audition", on: false }, { kind: "cycle", on: true }, { id: "start" },
    { kind: "seek", ms: 201500 },
  ]);

  let waveform = 0, used = 0;
  get("ruler-waveform").attributes["aria-pressed"] = "true";
  get("ruler-waveform").addEventListener("click", () => { waveform++; });
  get("used-only").addEventListener("click", () => { used++; get("used-only").checked = true; });
  get("menu-view").click();
  assert.equal(item("Song waveform").attributes["aria-checked"], "true");
  assert.equal(item("Used lanes only").attributes["aria-checked"], "false");
  item("Song waveform").click();
  item("Used lanes only").click();
  assert.deepEqual([waveform, used], [1, 1]);
  assert.equal(item("Used lanes only").attributes["aria-checked"], "true");
  assert.equal(popup.hidden, false);

  get("difficulty").options = [{ value: "Test.osu", text: "Hard" }, { value: "Insane.osu", text: "Insane" }];
  get("menu-map").click();
  assert.equal(item("Hard").attributes["aria-checked"], "true");
  item("Insane").click();
  assert.deepEqual(calls.at(-1), { sibling: "Insane.osu" });
  assert.equal(popup.hidden, true);
});

test("the bar-menu look stays off the right-click tool picker and the lane menu", () => {
  const block = css.slice(css.indexOf("/* Menus (design f6)"), css.indexOf("/* Save with problems (design f8)"));
  const rules = [...block.matchAll(/([^{}]+)\{[^}]*\}/g)].map(match => match[1].replace(/\/\*.*?\*\//s, "").trim());
  assert.ok(rules.length > 10);
  for (const selector of rules.flatMap(rule => rule.split(","))) {
    assert.match(selector.trim(), /^(?:header #editor-menus\b|#editor-menu\[aria-labelledby\](?: |$))/, selector);
  }
  assert.doesNotMatch(block, /:has\(|color-mix\(/, "macOS and Linux WebViews may lack :has() and color-mix()");

  const { get, scene, context } = fixture();
  const popup = get("editor-menu");
  get("menu-edit").click();
  assert.equal(popup.attributes["aria-labelledby"], "menu-edit");
  const plain = menuButtons(popup).find(button => button.textContent === "Place sound set…");
  const icon = menuButtons(popup).find(button => button.textContent === "Select all");
  assert.match(plain.className, /\bmenu-item-plain\b/);
  assert.doesNotMatch(icon.className, /\bmenu-item-plain\b/);

  const grid = get("grid");
  context.window.openEditorMenu("tool", false, { x: 10, y: 10, trigger: grid, label: "Timeline tools" });
  assert.equal(popup.attributes["aria-labelledby"], undefined);
  assert.equal(popup.attributes["aria-label"], "Timeline tools");

  const lanes = get("lanes"), name = get("second-name");
  name.dataset = { laneRow: "1" };
  name.closest = () => name;
  lanes.events.get("contextmenu")({ target: name, clientX: 10, clientY: 10, preventDefault() {} });
  assert.equal(popup.attributes["aria-labelledby"], undefined);
  assert.equal(popup.attributes["aria-label"], "Lane actions");
  assert.ok(scene);
});

test("every command a menu runs or shows a key for is in the editor's command catalogue", () => {
  // tools.rs names the tool commands in a tuple; the Rust test in dawsu-tauri also checks the other way.
  const known = new Set([...catalogue.matchAll(/(?:CommandDefinition::new|\bdef)\(\s*"(\w+)"|\(\s*"(tool\w+)",/g)]
    .map(match => match[1] ?? match[2]));
  for (const id of ["undo", "save", "importOsz", "addBookmark", "start", "cycle", "toolSelect", "toolRamp"]) {
    assert.ok(known.has(id), "catalogue reader finds " + id);
  }
  assert.ok(!known.has("notACommand"));
  const used = commandList.items.map(item => item.command).filter(Boolean);
  assert.ok(used.length > 25, "commands.js commands are found");
  for (const id of [...used, ...commandList.keyOnly]) {
    assert.ok(known.has(id) || /^(?:seek\w+|snap\d)$/.test(id), id + " is not a catalogue command");
  }
  assert.doesNotMatch(code, /command\("\w+"\)/, "menus.js names no command of its own; commands.js does");
});

test("the tool picker can be opened by the Svelte tool pills", () => {
  assert.match(code, /^window\.openEditorMenu = openEditorMenu;$/m);
});

test("Sound set placement needs a selected placement and shows its set, edits, volume, reset and unlink", async () => {
  const { get, calls, scene, modals, responses } = fixture();
  scene.placement = null;
  assert.equal(editItem(get, "Sound set placement…").disabled, true, "needs a selected placement");
  get("menu-edit").click();
  scene.placement = 7;
  responses.sound_set_placement = { id: 7, name: "Kick stack", edited: 2, sounds: 3, gain: 100, frozen: null, newer: false };
  editItem(get, "Sound set placement…").click();
  await settle();
  assert.deepEqual(plain(calls.find(call => Array.isArray(call) && call[0] === "sound_set_placement")), ["sound_set_placement", { map: "Test.osu", id: 7 }]);
  const dialog = modals.at(-1);
  assert.equal(dialog.kind, "sound-set-placement");
  assert.equal(dialog.title, "Sound set placement");
  const [name, edited, label, message] = dialog.content;
  assert.deepEqual(plain([name.textContent, edited.textContent, label.children[0], message.textContent]), ["Kick stack", "2 of 3 sounds edited", "Volume %", ""]);
  const field = label.children[1];
  assert.equal(field.value, "100");
  assert.deepEqual(plain(dialog.buttons.map(([text]) => text)), ["Close", "Unlink", "Reset edits", "Set volume"]);
  field.value = "150";
  button(dialog, "Set volume")[1]();
  assert.equal(message.textContent, "Volume can be at most 100%.");
  field.value = "4";
  button(dialog, "Set volume")[1]();
  assert.equal(message.textContent, "Volume must be at least 5%.");
  message.textContent = "";
  field.value = "";
  button(dialog, "Set volume")[1]();
  assert.equal(message.textContent, "Volume must be at least 5%.", "a blank field is no volume");
  assert.ok(!calls.some(call => call.op === "gain"));
  field.value = "50";
  button(dialog, "Set volume")[1]();
  assert.deepEqual(plain(calls.slice(-2)), ["closeModal", { kind: "sound_set_placement", map: "Test.osu", id: 7, op: "gain", gain: 50 }]);
  button(dialog, "Reset edits")[1]();
  assert.deepEqual(plain(calls.at(-1)), { kind: "sound_set_placement", map: "Test.osu", id: 7, op: "reset" });
  button(dialog, "Unlink")[1]();
  assert.deepEqual(plain(calls.at(-1)), { kind: "sound_set_placement", map: "Test.osu", id: 7, op: "unlink" });
});

test("a placement not in the library can be unlinked but not set louder, and one on an older version offers Update", async () => {
  const { get, scene, calls, modals, responses } = fixture();
  scene.placement = 3;
  responses.sound_set_placement = { id: 3, setId: "9e7b1d18", name: null, edited: 0, sounds: 2, gain: 80, frozen: "Not in library", newer: false };
  editItem(get, "Sound set placement…").click();
  await settle();
  let dialog = modals.at(-1);
  assert.deepEqual(plain(dialog.content.map(node => node.textContent)), ["Not in library", "0 of 2 sounds edited"]);
  assert.equal(dialog.content[0].title, "9e7b1d18", "the id is a tooltip, never the name");
  assert.deepEqual(plain(dialog.buttons.map(([text]) => text)), ["Close", "Unlink"]);
  responses.sound_set_placement = { id: 3, name: "Kick stack", edited: 1, sounds: 2, gain: 80, frozen: "The library has another version of this set.", newer: true };
  editItem(get, "Sound set placement…").click();
  await settle();
  dialog = modals.at(-1);
  assert.deepEqual(plain(dialog.content.map(node => node.textContent)), ["Kick stack", "1 of 2 sounds edited", "The library has another version of this set."]);
  assert.deepEqual(plain(dialog.buttons.map(([text]) => text)), ["Close", "Unlink", "Reset edits", "Update"]);
  button(dialog, "Update")[1]();
  assert.deepEqual(plain(calls.at(-1)), { kind: "sound_set_placement", map: "Test.osu", id: 3, op: "update" });
});

test("the open-time question answers Update or Keep, and Escape closes it until the next open", () => {
  const { context, calls, modals, view } = fixture();
  view.soundSetPrompts = [{ setId: "a", name: "Kick stack", placements: 2, editedKeepFiles: 2, open: 1 },
    { setId: "b", name: "Bell", placements: 1, editedKeepFiles: 0, open: 1 }];
  context.showSoundSetPrompt(context.pendingSoundSetPrompt());
  const dialog = modals.at(-1);
  assert.equal(dialog.kind, "sound-set-update");
  assert.equal(dialog.title, "Sound set changed");
  assert.deepEqual(plain(dialog.content.map(node => node.textContent)),
    ["2 placements use an older version of Kick stack.", "2 edited sounds keep their own files."]);
  assert.deepEqual(plain(dialog.buttons.map(([text]) => text)), ["Keep", "Update"]);
  button(dialog, "Update")[1]();
  assert.deepEqual(plain(calls.slice(-2)), ["closeModal", { kind: "sound_set_prompt", set_id: "a", update: true }]);
  button(dialog, "Keep")[1]();
  assert.deepEqual(plain(calls.at(-1)), { kind: "sound_set_prompt", set_id: "a", update: false });
  const sent = calls.length;
  dialog.cancel();
  assert.deepEqual(plain(calls.slice(sent)), ["closeModal"], "Escape sends nothing");
  assert.equal(context.pendingSoundSetPrompt().setId, "b", "a closed question stays closed this open");
  context.showSoundSetPrompt(context.pendingSoundSetPrompt());
  assert.deepEqual(plain(modals.at(-1).content.map(node => node.textContent)), ["1 placement uses an older version of Bell."]);
  view.soundSetPrompts = [{ setId: "a", name: "Kick stack", placements: 2, editedKeepFiles: 1, open: 2 }];
  assert.equal(context.pendingSoundSetPrompt().setId, "a", "it comes back on the next open");
  context.showSoundSetPrompt(context.pendingSoundSetPrompt());
  assert.equal(modals.at(-1).content[1].textContent, "1 edited sound keeps its own file.");
});

test("Place sound set lists a set this map uses that the library lacks under In this map, greyed out", async () => {
  const { get, modals, responses } = fixture();
  responses.sound_set_library = { configured: true, sets: [
    { id: "9e7b1d18", name: "", sounds: 0, inMap: true, duplicate: false, missing: true },
  ], unreadable: [], reading: false };
  editItem(get, "Place sound set…").click();
  await settle();
  const [list, message] = modals[0].content;
  assert.deepEqual(plain(list.children.map(group => [group.label, group.children.map(option => [option.textContent, !!option.disabled])])), [
    ["In this map", [["Not in library", true]]],
  ]);
  assert.equal(list.children[0].children[0].title, "9e7b1d18");
  assert.equal(message.textContent, "None of these sound sets can be placed.");
});

test("Map > Drum hit detection… opens the drum detection window", async () => {
  const { get, calls } = fixture();
  get("menu-map").click();
  const item = menuButtons(get("editor-menu")).find(node => node.textContent === "Drum hit detection…");
  calls.length = 0;
  item.click();
  await settle();
  assert.deepEqual(calls.map(([name]) => name), ["drum_window_open"]);
});

test("a command with several keys shows only its first key in the menu", async () => {
  // Cycle runs from L, Numpad / and /: joined, the hint read "L / Numpad / / /".
  const { get, responses } = fixture();
  responses.editor_shortcuts = { entries: [["cycle", "Cycle", ["L", "Numpad /", "/"]], ["undo", "Undo", []]] };
  const popup = get("editor-menu");
  get("menu-playback").click();
  await settle();
  const cycle = menuButtons(popup).find(button => button.textContent === "Cycle between L and R");
  assert.equal(cycle.children.find(child => child.type === "kbd")?.textContent, "L");
});

test("menu rows use the v3 design's icons, and every icon file is there", async () => {
  // The design canvas names these rows' Lucide icons; several used to borrow Redo's arrow.
  const icon = id => commandList.items.find(item => item.id === id).icon;
  const design = {
    importOsz: "PackageOpen", openFolder: "Folder", revert: "RotateCcw", export: "Package", folders: "Folder",
    undo: "Undo2", redo: "Redo2", cut: "Scissors", copy: "Copy", paste: "Clipboard", duplicate: "CopyPlus",
    delete: "Trash2", selectAll: "SquareDashedMousePointer", quantize: "AlignHorizontalJustifyCenter",
    repeat: "Repeat2", editRange: "SquareDashed", scaleByVolume: "ChartNoAxesColumn", protect: "Shield",
    returnOnStop: "UndoDot", follow: "LocateFixed", cycle: "Repeat", audioOutput: "Volume2",
    newDifficulty: "FilePlus", copyHitsounds: "Copy", drumDetection: "Drum", addBookmark: "BookmarkPlus",
    previousBookmark: "ArrowLeftToLine", nextBookmark: "ArrowRightToLine", addLane: "Plus", laneAdd: "Plus",
    songWaveform: "AudioWaveform", objectLines: "AlignHorizontalSpaceAround", usedOnly: "ListFilter",
    shortcuts: "Keyboard", volumePanel: "ChartNoAxesColumn", fit: "MoveHorizontal", audition: "Ear",
  };
  for (const [id, name] of Object.entries(design)) assert.equal(icon(id), name, id);
  assert.deepEqual(commandList.items.filter(item => item.icon === "Redo2").map(item => item.id), ["redo"]);
  const { readdir } = await import("node:fs/promises");
  const files = new Set(await readdir(new URL("../dist/studio-assets/icons/", import.meta.url)));
  for (const item of commandList.items.filter(item => item.icon)) {
    assert.ok(files.has(item.icon + ".svg"), item.id + " draws " + item.icon + ".svg");
  }
});

test("Cycle between L and R and Add numbered layer show their keys and still act as before", async () => {
  const { get, calls } = fixture();
  const popup = get("editor-menu");
  const hint = button => button.children.find(child => child.type === "kbd")?.textContent;
  get("menu-playback").click();
  await settle();
  const cycle = menuButtons(popup).find(button => button.textContent === "Cycle between L and R");
  assert.equal(hint(cycle), "Alt+C");
  cycle.click();
  assert.equal(popup.hidden, false, "a switch row keeps the menu open");
  assert.deepEqual(calls.at(-1), { kind: "cycle", on: true });

  const lanes = get("lanes"), name = get("second-name");
  name.dataset = { laneRow: "1" };
  name.closest = () => name;
  lanes.events.get("contextmenu")({ target: name, clientX: 10, clientY: 10, preventDefault() {} });
  await settle();
  const layer = menuButtons(popup).find(button => button.textContent === "Add numbered layer");
  assert.equal(hint(layer), "Ctrl+L");
  layer.click();
  await settle();
  assert.deepEqual(calls.slice(-2), [{ kind: "lane", row: 1, operation: "activate" }, { kind: "lane", row: 1, operation: "add_layer" }]);
});

test("the lane menu clones a numbered variant and alternates two or more selected hits with the next lane", async () => {
  const { get, calls, scene, view, context } = fixture();
  const popup = get("editor-menu"), lanes = get("lanes");
  const openOn = async (row) => {
    const name = get("lane-name-" + row);
    name.dataset = { laneRow: String(row) };
    name.closest = () => name;
    lanes.events.get("contextmenu")({ target: name, clientX: 10, clientY: 10, preventDefault() {} });
    await settle();
    return label => menuButtons(popup).find(button => button.textContent === label)
      ?? popup.children.find(button => button.textContent === label);
  };
  let row = await openOn(1);
  assert.equal(row("Clone as next numbered variant").disabled, false);
  assert.equal(row("Alternate selected hits with next lane").disabled, true, "the last lane has no next lane");
  row("Clone as next numbered variant").click();
  await settle();
  assert.deepEqual(calls.slice(-2), [{ kind: "lane", row: 1, operation: "activate" }, { kind: "clone_sample_variant", lane: 1 }]);

  context.closeEditorMenu?.();
  scene.lanes[0].canReplace = true;
  view.laneCount = 2;
  row = await openOn(0);
  assert.equal(row("Alternate selected hits with next lane").disabled, true, "one selected hit is not enough");
  scene.selection = { count: 2 };
  context.refreshEditorMenu();
  assert.equal(row("Alternate selected hits with next lane").disabled, false);
  row("Alternate selected hits with next lane").click();
  await settle();
  assert.deepEqual(calls.slice(-2), [{ kind: "lane", row: 0, operation: "activate" },
    { kind: "alternate_variants", from_lane: 0, to_lane: 1 }]);
});

test("File › Revert mapset to original… opens only with a first-open copy of the mapset", async () => {
  const { get, calls, view, context } = fixture();
  const popup = get("editor-menu");
  const revert = () => menuButtons(popup).find(button => button.textContent === "Revert mapset to original…")
    ?? popup.children.find(button => button.textContent === "Revert mapset to original…");
  let asked = 0;
  context.confirmRevertToOriginal = () => { asked += 1; };
  view.hasDocument = true;
  get("menu-file").click();
  assert.equal(revert().disabled, true, "no snapshot yet");
  view.hasOriginalSnapshot = true;
  view.hasDocument = false;
  context.refreshEditorMenu();
  assert.equal(revert().disabled, true, "no difficulty open");
  view.hasDocument = true;
  context.refreshEditorMenu();
  assert.equal(revert().disabled, false);
  revert().click();
  assert.equal(asked, 1);
  assert.equal(calls.some(call => Array.isArray(call) && call[0] === "revert_mapset_to_original"), false,
    "the menu row only asks; the dialog's Revert button invokes");

  const undoRevert = () => menuButtons(popup).find(button => button.textContent === "Undo revert")
    ?? popup.children.find(button => button.textContent === "Undo revert");
  let undone = 0;
  context.undoMapsetRevert = () => { undone += 1; };
  get("menu-file").click();
  assert.equal(undoRevert().disabled, true, "no pre-revert backup yet");
  view.canUndoRevert = true;
  context.refreshEditorMenu();
  assert.equal(undoRevert().disabled, false);
  undoRevert().click();
  assert.equal(undone, 1);
});

test("every rule, call, choice source and run kind the command list names exists in menus.js", () => {
  const { context } = fixture();
  const [rules, calls, sources] = ["menuRules", "menuCalls", "menuChoiceSources"].map(name => runInContext(name, context));
  const kinds = new Set([...code.matchAll(/kind === "(\w+)"/g)].map(match => match[1]));
  for (const item of commandList.items) {
    for (const rule of [item.enabled, item.checked].filter(Boolean)) assert.ok(rule in rules, item.id + ": rule " + rule);
    if (item.choices) assert.ok(item.choices in sources, item.id + ": choices " + item.choices);
    if (!item.run) continue;
    assert.ok(kinds.has(item.run[0]), item.id + ": run kind " + item.run[0]);
    if (item.run[0] === "call") assert.equal(typeof calls[item.run[1]], "function", item.id + ": call " + item.run[1]);
  }
});

test("a menu that cannot load its command list says so in the status line", () => {
  const { get, window } = fixture();
  window.dawsuUi.commands = undefined;
  get("menu-edit").click();
  assert.equal(get("editor-menu").hidden, undefined, "no menu opens");
  assert.equal(get("status").textContent, "The menus could not load. Restart dawsu! to try again.");
});

test("a refresh with nothing changed hands the rows over only once", async () => {
  const { get, window, context } = fixture();
  let handed = 0;
  const draw = window.dawsuUi.menu;
  window.dawsuUi.menu = model => { handed += 1; draw(model); };
  get("menu-edit").click();
  await settle();
  const after = handed;
  context.refreshEditorMenu();
  context.refreshEditorMenu();
  assert.equal(handed, after);
});

test("the editor keeps a failing menu refresh from stopping the rest of show()", async () => {
  const { readEditorSource } = await import("./editor-runtime.mjs");
  assert.match(readEditorSource(), /try \{ env\.window\.refreshEditorMenu\?\.\(\); \} catch \(error\) \{/);
});

test("the menu popup grows to fit a row with a long key hint instead of wrapping its label", () => {
  assert.match(css, /#editor-menu\{[^}]*width:max-content;min-width:min\(330px,calc\(100vw - 8px\)\);max-width:calc\(100vw - 8px\)/);
  assert.match(css, /\.menu-choice-group\{contain:inline-size;/, "choice buttons still wrap instead of widening the menu");
});

// v3: the lane toolbar has no lane names button; View › Lane names switches them.
test("View › Lane names switches the lane names", () => {
  const { get, calls, view } = fixture();
  view.panels = { laneNames: true };
  get("menu-view").click();
  const row = menuButtons(get("editor-menu")).find(item => item.textContent === "Lane names");
  assert.ok(row, "Lane names row");
  assert.equal(row.attributes["aria-checked"], "true");
  row.click();
  assert.deepEqual(calls.at(-1), { kind: "panel", panel: "lane_names", on: false });
});
