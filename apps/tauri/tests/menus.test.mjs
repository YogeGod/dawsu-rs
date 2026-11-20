import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const code = await readFile(new URL("../dist/menus.js", import.meta.url), "utf8");
const html = await readFile(new URL("../dist/index.html", import.meta.url), "utf8");
const css = await readFile(new URL("../dist/style.css", import.meta.url), "utf8");
const catalogue = (await Promise.all([
  "../../../crates/dawsu-application/src/commands.rs",
  "../../../crates/dawsu-editor/src/editing_commands.rs",
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
    };
  }
  function get(id) {
    if (!elements.has(id)) elements.set(id, Object.assign(element("div"), { id }));
    return elements.get(id);
  }
  get("editor-menus").append(...["file", "edit", "playback", "map", "view"].map(name => get("menu-" + name)));
  get("editor-menu").offsetWidth = 330;
  get("editor-menu").offsetHeight = 220;
  get("redo").disabled = true;
  const speedButtons = [.25, .5, .75, 1, 1.25, 1.5].map(value => ({
    dataset: { playbackSpeed: String(value) }, textContent: (value * 100) + "%",
  }));
  const view = { speed: 1, playbackStart: "cursor", follow: false, activeLane: 0 };
  const scene = { path: "Test.osu", selection: {count:1}, lanes: [
    {sourceKey:"first", canDelete:false}, {sourceKey:"second", canDelete:true, canReplace:true, canAddLayer:true, colourKey:"second"},
  ] };
  const commands = { entries: [["undo", "Undo", ["Alt+U"]], ["save", "Save", ["Ctrl+S"]]] };
  const window = { innerWidth: 900, innerHeight: 640, addEventListener() {} };
  const context = createContext({
    $: get, document, window, view, scene, speedButtons, drag: null, dialog: {},
    lanePositions: new Map([[0,0],[1,1]]),
    adjacentLaneForMove: () => null,
    run(task) { return task(); },
    async update(action) { calls.push(JSON.parse(JSON.stringify(action))); view.activeLane = action.row; },
    toolIcons: { Select: "MousePointer2", Draw: "Drumstick", Erase: "Eraser", Mute: "VolumeX",
      Grab: "Hand", Range: "Scan", Ramp: "TrendingUp" },
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
  let deleted=false;
  get("delete-lane").addEventListener("click",()=>{deleted=true;});
  menu.children.find(button=>button.textContent==="Delete lane…").click();
  await new Promise(setImmediate);
  assert.deepEqual(calls,[{kind:"lane",row:1,operation:"activate"}]);
  assert.equal(deleted,true);
});

test("right-click opens the seven timeline tools without invoking an edit", () => {
  const { document, get, calls, key, view } = fixture();
  const popup = get("editor-menu"), grid = get("grid");
  view.tool = "Erase";
  const click = { clientX: 895, clientY: 637, preventDefault() { this.defaultPrevented = true; } };
  assert.equal(typeof grid.events.get("contextmenu"), "function", "grid registers a tool picker");
  grid.events.get("contextmenu")(click);
  assert.equal(click.defaultPrevented, true);
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

  grid.events.get("contextmenu")(click);
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
  assert.deepEqual([...menus.matchAll(/id="menu-(\w+)"/g)].map(match => match[1]),
    ["file", "edit", "playback", "map", "view"]);
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
    "Open map…", "Import .osz…", "Open map folder", "—",
    "Save", "Revert to saved…", "Export…", "—",
    "Start screen", "Folders…",
  ]);
  assert.deepEqual(open("edit"), [
    "Undo", "Redo", "—",
    "Cut", "Copy", "Paste at playhead", "Duplicate", "Delete", "Select all", "—",
    "Quantize…", "Repeat…", "Edit range…", "Scale by volume…", "Place sound set…", "Save selection as sound set…",
    "Sound set placement…", "—",
    "Protect hitnormals from deletion", "Audition when placing hits",
  ]);
  assert.deepEqual(open("playback"), [
    "Play / pause", "Stop", "Go to start", "Go to end", "—",
    "Start playing from: Cursor | Left locator | Selection | Selection or left locator", "—",
    "Return on stop", "Follow playhead", "Cycle between L and R", "—",
    "Speed · pitch stays the same: 25 | 50 | 75 | 100 | 125 | 150", "—",
    "Mute song", "Audio output…",
  ]);
  assert.deepEqual(open("map"), [
    "Difficulty: Hard | Insane", "—",
    "New hitsound difficulty…", "—",
    "Generate hitsound difficulty…", "Copy hitsounds…", "Drum hit detection…",
    "Generate export style: Per-object", "—",
    "Add bookmark", "Previous bookmark", "Next bookmark", "—",
    "Add lane…", "Delete active lane…",
  ]);
  assert.deepEqual(open("view"), [
    "Browser", "Inspector", "Volume panel", "Lane names", "—",
    "Song waveform", "Object lines", "Used lanes only", "Collapse sample layers", "—",
    "Zoom in", "Zoom out", "Fit difficulty", "—",
    "Time signatures…", "Open drum map…", "Edit layout…", "View settings…", "—",
    "Shortcuts",
  ]);
  for (const name of ["file", "edit", "playback", "map", "view"]) {
    get("menu-" + name).click();
    for (const button of menuButtons(popup)) assert.notEqual(button.attributes["aria-haspopup"], "menu");
  }
});

test("menus only click ribbon controls that exist in the window", () => {
  const ids = new Set([...code.matchAll(/(?:click|\$)\("([\w-]+)"\)/g)].map(match => match[1]));
  for (const id of ["folders", "audio", "open", "go-home", "used-only", "ruler-waveform"]) assert.ok(ids.has(id), id);
  for (const id of ids) assert.match(html, new RegExp('id="' + id + '"'), id);
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

  const { get, scene } = fixture();
  const popup = get("editor-menu");
  get("menu-edit").click();
  assert.equal(popup.attributes["aria-labelledby"], "menu-edit");
  const plain = menuButtons(popup).find(button => button.textContent === "Protect hitnormals from deletion");
  const icon = menuButtons(popup).find(button => button.textContent === "Select all");
  assert.match(plain.className, /\bmenu-item-plain\b/);
  assert.doesNotMatch(icon.className, /\bmenu-item-plain\b/);

  const grid = get("grid");
  grid.events.get("contextmenu")({ clientX: 10, clientY: 10, preventDefault() {} });
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
  const known = new Set([...catalogue.matchAll(/(?:CommandDefinition::new|\bdef)\(\s*"(\w+)"/g)].map(match => match[1]));
  for (const id of ["undo", "save", "importOsz", "addBookmark", "start", "cycle"]) assert.ok(known.has(id), "catalogue reader finds " + id);
  assert.ok(!known.has("notACommand"));
  const used = [
    ...code.matchAll(/menuCommand\(.*?,\s*(?:null|"\w+"),\s*"(\w+)"/g),
    ...code.matchAll(/shortcut: "(\w+)"\s*[,}]/g),
  ].map(match => match[1]);
  assert.ok(used.length > 25, "menus.js commands are found");
  for (const id of used) assert.ok(known.has(id), id + " is not a catalogue command");
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
