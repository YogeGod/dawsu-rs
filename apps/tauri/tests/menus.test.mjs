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
  get("redo").disabled = true;
  get("speed").options = [
    { value: ".25", textContent: "0.25×" }, { value: ".5", textContent: "0.5×" },
    { value: ".75", textContent: "0.75×" }, { value: "1", textContent: "1×" },
    { value: "1.25", textContent: "1.25×" }, { value: "1.5", textContent: "1.5×" },
  ];
  const view = { speed: 1, playbackStart: "cursor", follow: false };
  const commands = { entries: [["undo", "Undo", ["Alt+U"]], ["save", "Save", ["Ctrl+S"]]] };
  const window = { innerWidth: 900, innerHeight: 640, addEventListener() {} };
  const context = createContext({
    $: get, document, window, view, scene: { selection: { count: 1 } }, drag: null, dialog: {},
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
  assert.deepEqual(calls.at(-1), { kind: "review_save" });
  assert.equal(popup.hidden, true);
});
