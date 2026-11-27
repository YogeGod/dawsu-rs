import assert from "node:assert/strict";
import { test } from "node:test";
import { COLOR_ROWS, applyColorConfig, resolvePalette } from "../src/lib/theme.js";
import { registerTimelineTheme } from "../src/lib/registry.js";

test("gridLines row is independent of gridBg and supports hue selection (#103)", () => {
  const gridLinesRow = COLOR_ROWS.find(row => row.id === "gridLines");
  const gridCycleRow = COLOR_ROWS.find(row => row.id === "gridCycle");
  assert.ok(gridLinesRow, "gridLines row exists");
  assert.equal(gridLinesRow.requires, undefined, "gridLines must not depend on gridBg");
  assert.equal(gridLinesRow.hasHue, true, "gridLines supports Hue, Saturation, Brightness");
  assert.equal(gridCycleRow?.requires, undefined, "gridCycle must not depend on gridBg");
});

test("resolvePalette preserves Obsidian grid defaults and applies standalone gridLines colors (#103)", () => {
  const defaults = resolvePalette({ scheme: "obsidian", rows: {} });
  assert.equal(defaults.gridSub, "rgba(255,255,255,.04)");
  assert.equal(defaults.gridBeat, "rgba(255,255,255,.07)");
  assert.equal(defaults.gridBar, "rgba(255,255,255,.26)");

  const redLines = resolvePalette({
    scheme: "obsidian",
    rows: { gridLines: { h: 0, s: 1, b: 1 } },
  });
  assert.equal(redLines.gridBar, "#ff0000");
  assert.equal(redLines.barLine, "#ff0000");
  assert.notEqual(redLines.gridBeat, defaults.gridBeat);
  assert.notEqual(redLines.gridSub, defaults.gridSub);
  assert.equal(redLines.border, "#2b2b30", "standalone gridLines change leaves chrome border untouched");

  const themeTarget = {};
  registerTimelineTheme(themeTarget);
  applyColorConfig({ scheme: "obsidian", rows: { gridLines: { h: 120, s: 1, b: 1 } } }, null);
  assert.equal(themeTarget.gridBar, "#00ff00");
  assert.equal(themeTarget.barLine, "#00ff00");
  assert.ok(themeTarget.gridBeat && themeTarget.gridSub);
  registerTimelineTheme(null);
});
