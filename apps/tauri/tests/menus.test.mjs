import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const code = await readFile(new URL("../dist/menus.js", import.meta.url), "utf8");

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
  get("editor-menus").append(...["map", "edit", "view", "playback"].map(name => get("menu-" + name)));
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
  });
  runInContext(code, context);
  function key(key, handler = listeners.get("keydown")) {
    const event = { key, preventDefault() { this.defaultPrevented = true; } };
    handler(event);
    return !!event.defaultPrevented;
  }
  return { document, get, view, window, calls, listeners, key };
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

test("Playback keeps its segmented choices and switches open, and Map keeps Generate per-object", () => {
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
  const follow = popup.children.find(child => child.textContent === "Follow playback");
  assert.equal(follow.attributes.role, "menuitemcheckbox");
  follow.click();
  assert.equal(follow.attributes["aria-checked"], "true");
  assert.equal(choices("Speed").length, 6);
  choices("Speed").at(-1).click();
  assert.deepEqual(calls, [
    { kind: "playback_start", value: "left" }, { kind: "follow", on: true }, { kind: "speed", value: 1.5 },
  ]);

  get("menu-map").click();
  const style = choices("Generate export style")[0];
  assert.equal(style.textContent, "Per-object");
  assert.equal(style.attributes["aria-checked"], "true");
  assert.equal(style.disabled, true);
  popup.children.find(child => child.textContent === "Save").click();
  assert.deepEqual(calls.at(-1), { id: "save" });
  assert.equal(popup.hidden, true);
});
