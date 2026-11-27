import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { Bridge } from "../src/lib/bridge.svelte.js";
import { attachStripWheel, laneHandlers } from "../src/lib/laneHandlers.js";
import { folderDragType } from "../src/lib/lanes.js";

// The lane headers' event handlers, driven with fake events; each must send the action app.js expects.
function setup() {
  const sent = [];
  let ticks = 0;
  const on = laneHandlers((...args) => sent.push(args), async () => { ticks++; });
  return { on, sent, ticks: () => ticks };
}
const button = { closest: selector => (selector === "button" ? {} : null) };
const plain = { closest: () => null };

test("a click on the row outside its buttons selects the lane; a click on a button does not", () => {
  const { on, sent } = setup();
  const event = { target: plain, ctrlKey: true };
  on.rowClick(event, 4);
  on.rowClick({ target: button }, 4);
  assert.deepEqual(sent, [["select", 4, event]]);
});

test("name, M, S, layer chevron, + and − send their actions", () => {
  const { on, sent } = setup();
  const event = { shiftKey: true };
  on.select(event, 2);
  on.mix(2, "mute");
  on.mix(2, "solo");
  on.family(2);
  on.addLayer(2);
  on.remove(2);
  assert.deepEqual(sent, [["select", 2, event], ["mix", 2, "mute"], ["mix", 2, "solo"], ["family", 2], ["addLayer", 2], ["remove", 2]]);
});

test("the fold strip and the folder name both fold; folder M and S mix the set", () => {
  const { on, sent } = setup();
  on.fold("soft");
  on.foldName("soft");
  on.folderMix("soft", "mute");
  on.folderMix("soft", "solo");
  assert.deepEqual(sent, [["folder", "soft"], ["folder", "soft"], ["folderMix", "soft", "mute"], ["folderMix", "soft", "solo"]]);
});

test("Alt+Up and Alt+Down move the folder and give focus back to its strip after the move", async () => {
  const { on, sent, ticks } = setup();
  const focused = [];
  const strip = { focus: () => focused.push("strip") };
  const header = { querySelector: selector => (selector === ".fold" ? strip : null) };
  const key = (key, altKey = true) => {
    const event = { key, altKey, prevented: false, stopped: false, currentTarget: { closest: () => header },
      preventDefault() { this.prevented = true; }, stopPropagation() { this.stopped = true; } };
    return event;
  };
  const down = key("ArrowDown");
  await on.folderKey(down, "normal");
  const up = key("ArrowUp");
  await on.folderKey(up, "drum");
  const plainDown = key("ArrowDown", false);
  await on.folderKey(plainDown, "normal");
  await on.folderKey(key("Enter"), "normal");
  assert.deepEqual(sent, [["folderStep", "normal", true], ["folderStep", "drum", false]]);
  assert.ok(down.prevented && down.stopped && up.prevented);
  assert.equal(plainDown.prevented, false, "a plain arrow is left to the editor");
  assert.equal(ticks(), 2);
  assert.deepEqual(focused, ["strip", "strip"]);
});

test("a folder dropped on the lower half of another goes after it; other drags are ignored", () => {
  const { on, sent } = setup();
  const data = new Map();
  const transfer = { setData: (type, value) => data.set(type, value), getData: type => data.get(type) ?? "",
    get types() { return [...data.keys()]; }, effectAllowed: "" };
  on.dragStart({ dataTransfer: transfer }, "drum");
  assert.equal(transfer.effectAllowed, "move");
  let allowed = false;
  on.dragOver({ dataTransfer: transfer, preventDefault: () => (allowed = true) });
  assert.ok(allowed);
  const target = { getBoundingClientRect: () => ({ y: 100, height: 30 }) };
  on.drop({ dataTransfer: transfer, clientY: 120, currentTarget: target, preventDefault() {} }, "normal");
  on.drop({ dataTransfer: transfer, clientY: 105, currentTarget: target, preventDefault() {} }, "soft");
  const empty = { getData: () => "", types: [] };
  on.drop({ dataTransfer: empty, clientY: 120, currentTarget: target, preventDefault() { throw new Error("kept"); } }, "soft");
  allowed = false;
  on.dragOver({ dataTransfer: { types: ["Files"] }, preventDefault: () => (allowed = true) });
  assert.equal(allowed, false);
  assert.deepEqual(sent, [["folderMove", "drum", "normal", true], ["folderMove", "drum", "soft", false]]);
  assert.equal(folderDragType, "application/x-dawsu-sample-set");
});

test("Ctrl+scroll on the lane strip cycles colour and leaves plain, Shift, and Alt wheel untouched", () => {
  const { on, sent } = setup();
  const makeWheel = (more = {}) => ({
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    deltaY: 0,
    prevented: false,
    stopped: false,
    preventDefault() { this.prevented = true; },
    stopPropagation() { this.stopped = true; },
    ...more,
  });

  const down = makeWheel({ ctrlKey: true, deltaY: 100 });
  on.stripWheel(down, 2);
  assert.ok(down.prevented && down.stopped);

  const up = makeWheel({ ctrlKey: true, deltaY: -100 });
  on.stripWheel(up, 2);
  assert.ok(up.prevented && up.stopped);

  const metaDown = makeWheel({ metaKey: true, deltaY: 50 });
  on.stripWheel(metaDown, 2);
  assert.ok(metaDown.prevented && metaDown.stopped);

  const ignored = [
    makeWheel({ deltaY: 100 }),
    makeWheel({ ctrlKey: true, shiftKey: true, deltaY: 100 }),
    makeWheel({ ctrlKey: true, altKey: true, deltaY: 100 }),
    makeWheel({ ctrlKey: true, deltaY: 0 }),
  ];
  for (const ev of ignored) {
    on.stripWheel(ev, 2);
    assert.equal(ev.prevented, false);
    assert.equal(ev.stopped, false);
  }
  assert.deepEqual(sent, [["cycleColour", 2, 1], ["cycleColour", 2, -1], ["cycleColour", 2, 1]]);

  const listeners = [];
  const node = {
    addEventListener(type, fn, opts) { listeners.push({ type, fn, opts }); },
    removeEventListener(type, fn) {
      const idx = listeners.findIndex(item => item.type === type && item.fn === fn);
      if (idx >= 0) listeners.splice(idx, 1);
    },
  };
  let row = 4;
  const calls = [];
  const detach = attachStripWheel((event, r) => calls.push([event, r]), () => row);
  const cleanup = detach(node);
  assert.equal(listeners.length, 1);
  assert.equal(listeners[0].type, "wheel");
  assert.deepEqual(listeners[0].opts, { passive: false });
  listeners[0].fn(down);
  row = 7;
  listeners[0].fn(up);
  assert.deepEqual(calls, [[down, 4], [up, 7]]);
  cleanup();
  assert.equal(listeners.length, 0);
});

test("LaneHeaders wires every handler from laneHandlers", () => {
  const part = readFileSync(new URL("../src/parts/LaneHeaders.svelte", import.meta.url), "utf8");
  for (const name of ["rowClick", "select", "mix", "family", "addLayer", "remove", "fold", "foldName", "folderMix", "folderKey",
    "dragStart", "dragOver", "drop", "stripWheel"]) {
    assert.match(part, new RegExp("on\\." + name + "\\b"), name);
  }
  assert.doesNotMatch(part, /send\(/, "no click sends past the handlers");
});

test("new lane rows reach the page before lanes() returns, so app.js can scroll to them", () => {
  const bridge = new Bridge();
  const order = [];
  bridge.handOverLanes({ rows: [] }, () => order.push(bridge.lanes ? "flushed with rows" : "flushed empty"));
  assert.deepEqual(order, ["flushed with rows"]);
  const bridgeSource = readFileSync(new URL("../src/lib/bridge.svelte.js", import.meta.url), "utf8");
  const main = readFileSync(new URL("../src/main.js", import.meta.url), "utf8");
  assert.match(bridgeSource, /lanes:\s*headers => this\.handOverLanes\(headers\),/);
  assert.match(main, /ui:\s*bridge\.editorApi/);
});
