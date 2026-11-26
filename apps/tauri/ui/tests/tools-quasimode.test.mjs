import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_DAW_TOOL_KEYS,
  DEFAULT_OSU_TOOL_KEYS,
  QUASIMODE_HOLD_MS,
  buildToolKeyMap,
  createQuasimodeController,
} from "../src/lib/quasimode.js";

test("buildToolKeyMap mirrors DAW and osu! presets and respects custom shortcuts without leaving stale defaults", () => {
  assert.deepEqual(buildToolKeyMap(null, "DAW"), DEFAULT_DAW_TOOL_KEYS);
  assert.deepEqual(buildToolKeyMap(null, "osu!"), DEFAULT_OSU_TOOL_KEYS);

  const custom = buildToolKeyMap({
    entries: [
      ["toolSelect", "Select", ["Q"]],
      ["toolDraw", "Draw", ["W"]],
      ["toolErase", "Erase", ["E"]],
      ["toolRange", "Range", ["R"]],
      ["toolRamp", "Ramp", ["Ctrl+R"]], // chorded shortcut should not bind single-key quasimode
    ],
  });
  assert.deepEqual(custom, { q: "Select", w: "Draw", e: "Erase", r: "Range" });
});

test("tapping '4' under holdMs opens More Tools without switching tool, while holding '4' activates Erase quasimode", () => {
  let tool = "Draw";
  let nowMs = 1000;
  let timers = [];
  let pickerOpens = 0;
  const setCalls = [];

  const q = createQuasimodeController({
    holdMs: QUASIMODE_HOLD_MS,
    getTool: () => tool,
    setTool: (next) => {
      tool = next;
      setCalls.push(next);
    },
    openMoreTools: () => {
      pickerOpens += 1;
    },
    setTimeoutFn: (fn, ms) => {
      const id = timers.length + 1;
      timers.push({ id, fn, ms, cancelled: false });
      return id;
    },
    clearTimeoutFn: (id) => {
      const t = timers.find(item => item.id === id);
      if (t) t.cancelled = true;
    },
    now: () => nowMs,
  });

  // Short tap < 180 ms opens More Tools picker.
  assert.equal(q.handleKeyDown({ key: "4" }), true);
  assert.equal(tool, "Draw");
  nowMs += 60;
  assert.equal(q.handleKeyUp({ key: "4" }), true);
  assert.equal(pickerOpens, 1);
  assert.equal(tool, "Draw");

  // Hold >= 180 ms fires hold timer -> Erase, then keyup restores prior tool (Draw).
  timers = [];
  assert.equal(q.handleKeyDown({ key: "4" }), true);
  nowMs += QUASIMODE_HOLD_MS;
  for (const t of timers) if (!t.cancelled) t.fn();
  assert.equal(tool, "Erase");
  assert.equal(q.handleKeyUp({ key: "4" }), true);
  assert.equal(tool, "Draw");
  assert.equal(pickerOpens, 1);
  assert.deepEqual(setCalls, ["Erase", "Draw"]);
});

test("pressing '4' and starting a pointer stroke immediately activates Erase and defers restore until pointerup", () => {
  let tool = "Select";
  let dragging = false;
  let nowMs = 0;
  let pickerOpens = 0;

  const q = createQuasimodeController({
    holdMs: 180,
    getTool: () => tool,
    setTool: (next) => { tool = next; },
    isDragging: () => dragging,
    openMoreTools: () => { pickerOpens += 1; },
    setTimeoutFn: () => 1,
    clearTimeoutFn: () => {},
    now: () => nowMs,
  });

  assert.equal(q.handleKeyDown({ key: "4" }), true);
  nowMs = 30;
  dragging = true;
  assert.equal(q.notifyStrokeStart(), true);
  assert.equal(tool, "Erase", "dual key enters Erase synchronously on pointer stroke");

  // Release '4' before pointerup: tool remains Erase until stroke finishes.
  assert.equal(q.handleKeyUp({ key: "4" }), true);
  assert.equal(tool, "Erase");
  assert.equal(pickerOpens, 0);

  dragging = false;
  assert.equal(q.notifyStrokeEnd(), true);
  assert.equal(tool, "Select");
});

test("quick-tap of '3' keeps Draw selected permanently, while holding '3' reverts to Select on keyup", () => {
  let tool = "Select";
  let nowMs = 0;
  const q = createQuasimodeController({
    holdMs: 180,
    getTool: () => tool,
    setTool: (next) => { tool = next; },
    setTimeoutFn: () => 1,
    clearTimeoutFn: () => {},
    now: () => nowMs,
  });

  // Tap < 180 ms -> stays Draw.
  assert.equal(q.handleKeyDown({ key: "3" }), true);
  assert.equal(tool, "Draw");
  nowMs += 50;
  assert.equal(q.handleKeyUp({ key: "3" }), true);
  assert.equal(tool, "Draw");

  // Switch back cleanly by holding '1' (Select) for >= 180 ms -> returns to Draw on keyup.
  assert.equal(q.handleKeyDown({ key: "1" }), true);
  assert.equal(tool, "Select");
  nowMs += 200;
  assert.equal(q.handleKeyUp({ key: "1" }), true);
  assert.equal(tool, "Draw");

  // Active pointer drag suppresses tool switch keydowns, and cancel() restores held quasimodes.
  let dragging = true;
  const guarded = createQuasimodeController({
    holdMs: 180,
    getTool: () => tool,
    setTool: (next) => { tool = next; },
    isDragging: () => dragging,
    setTimeoutFn: () => 1,
    clearTimeoutFn: () => {},
    now: () => nowMs,
  });
  assert.equal(guarded.handleKeyDown({ key: "1" }), false);
  assert.equal(tool, "Draw");
  dragging = false;
  assert.equal(guarded.handleKeyDown({ key: "2" }), true);
  assert.equal(tool, "Grab");
  guarded.notifyStrokeStart();
  assert.equal(guarded.cancel(true), true);
  assert.equal(tool, "Draw");
});
