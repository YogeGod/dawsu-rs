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
    invoke: async () => commands,
    openSiblingDifficulty(path) { calls.push({ sibling: path }); },
  });
  runInContext(code, context);
  function key(key, handler = listeners.get("keydown")) {
    const event = { key, preventDefault() { this.defaultPrevented = true; } };
    handler(event);
    return !!event.defaultPrevented;
  }
  return { document, get, view, scene, window, calls, listeners, key };
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
    "Save", "Revert to saved…", "—",
    "Start screen", "Folders…",
  ]);
  assert.deepEqual(open("edit"), [
    "Undo", "Redo", "—",
    "Cut", "Copy", "Paste at playhead", "Duplicate", "Delete", "Select all", "—",
    "Quantize…", "Repeat…", "Edit range…", "Scale by volume…", "—",
    "Protect hitnormals from deletion", "Audition when placing hits",
  ]);
  assert.deepEqual(open("playback"), [
    "Play / pause", "Stop", "Go to start", "—",
    "Start playing from: Cursor | Left locator | Selection | Selection or left locator", "—",
    "Return on stop", "Follow playhead", "Cycle between L and R", "—",
    "Speed · pitch stays the same: 25 | 50 | 75 | 100 | 125 | 150", "—",
    "Mute song", "Audio output…",
  ]);
  assert.deepEqual(open("map"), [
    "Difficulty: Hard | Insane", "—",
    "New hitsound difficulty…", "—",
    "Generate hitsound difficulty…", "Copy hitsounds…",
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
  assert.deepEqual(calls, [
    { kind: "protect", on: true }, { kind: "audition", on: false }, { kind: "cycle", on: true }, { id: "start" },
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
