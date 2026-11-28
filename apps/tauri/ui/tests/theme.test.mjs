import assert from "node:assert/strict";
import { test } from "node:test";
import { COLOR_ROWS, SCHEMES, applyColorConfig, inkFor, resolvePalette } from "../src/lib/theme.js";
import { registerTimelineTheme } from "../src/lib/registry.js";

// A document whose root style records every custom property applyColorConfig sets.
function fakeDocument() {
  const props = new Map();
  return { props, documentElement: { style: { setProperty: (name, value) => props.set(name, value) } } };
}

test("applyColorConfig leaves --editor-warning alone for every built-in scheme; tl.cycle follows the scheme", () => {
  assert.deepEqual(SCHEMES.map(s => s.id), ["obsidian", "grey", "dark", "blue-grey", "teal", "slate"]);
  for (const scheme of SCHEMES) {
    const doc = fakeDocument();
    const themeTarget = {};
    registerTimelineTheme(themeTarget);
    applyColorConfig({ scheme: scheme.id, rows: {} }, doc);
    registerTimelineTheme(null);
    assert.ok(doc.props.has("--editor-text"), `${scheme.id} themes the chrome`);
    assert.ok(!doc.props.has("--editor-warning"), `${scheme.id} must not overwrite --editor-warning`);
    assert.equal(themeTarget.cycle, scheme.palette.cycle, `${scheme.id} timeline cycle`);
  }
});

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

// WCAG 2 relative luminance and contrast ratio, worked independently of theme.js.
function luminance(hex) {
  const [r, g, b] = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map(c => c <= .03928 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4);
  return .2126 * r + .7152 * g + .0722 * b;
}
function contrast(first, second) {
  const [light, dark] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (light + .05) / (dark + .05);
}

test("inkFor picks the light or dark ink with the higher WCAG contrast", () => {
  assert.equal(inkFor("#000000"), "#f5f5f7");
  assert.equal(inkFor("#0e0e10"), "#f5f5f7");
  assert.equal(inkFor("#383838"), "#f5f5f7");
  assert.equal(inkFor("#ffffff"), "#141416");
  assert.equal(inkFor("#f2f2f2"), "#141416");
  // Mid grey: #8c8c8c reads 3.1:1 with light ink but 5.5:1 with dark ink.
  assert.equal(inkFor("#8c8c8c"), "#141416");
  // 3-, 4-, and 8-digit hex strings, whitespace, and malformed/empty fallbacks.
  assert.equal(inkFor("#000"), "#f5f5f7");
  assert.equal(inkFor("#fff"), "#141416");
  assert.equal(inkFor("#0f0"), "#141416");
  assert.equal(inkFor("#0f0f"), "#141416");
  assert.equal(inkFor("#000f"), "#f5f5f7");
  assert.equal(inkFor("#ffff"), "#141416");
  assert.equal(inkFor("#ffffff80"), "#141416");
  assert.equal(inkFor("#00000080"), "#f5f5f7");
  assert.equal(inkFor(" #ffffff "), "#141416");
  for (const bad of ["", null, undefined, "#12345z", "rgb(255,255,255)"]) {
    assert.equal(inkFor(bad), "#f5f5f7", `fallback for ${String(bad)}`);
  }
});

test("built-in dark schemes keep their hand-tuned text, secondary and muted inks", () => {
  for (const scheme of SCHEMES) {
    const palette = resolvePalette({ scheme: scheme.id, rows: {} });
    for (const key of ["text", "secondary", "muted"]) {
      assert.equal(palette[key], scheme.palette[key], `${scheme.id} ${key}`);
    }
  }
});

test("text on panel, ruler bar on ruler and playhead on canvas read at 4.5:1 or better", () => {
  const grey = b => ({ h: 0, s: 0, b });
  const configs = [
    ...SCHEMES.map(scheme => [scheme.id, { scheme: scheme.id, rows: {} }]),
    ["bright", { scheme: "obsidian", rows: { scheme: grey(0.95) } }],
    ["mid-bright", { scheme: "obsidian", rows: { scheme: grey(0.58) } }],
    ["bright with dark grid", { scheme: "obsidian", rows: { scheme: grey(0.95), gridBg: grey(0.08) } }],
    ["dark with bright ruler", { scheme: "obsidian", rows: { rulerBg: grey(0.95) } }],
  ];
  for (const [name, config] of configs) {
    const palette = resolvePalette(config);
    const themeTarget = {};
    registerTimelineTheme(themeTarget);
    applyColorConfig(config, fakeDocument());
    registerTimelineTheme(null);
    assert.equal(themeTarget.rulerBar, palette.rulerBar, `${name} timeline rulerBar`);
    assert.equal(themeTarget.playhead, palette.playhead, `${name} timeline playhead`);
    for (const [ink, surface] of [["text", "panel"], ["rulerBar", "ruler"], ["playhead", "canvas"]]) {
      const ratio = contrast(palette[ink], palette[surface]);
      assert.ok(ratio >= 4.5, `${name}: ${ink} ${palette[ink]} on ${surface} ${palette[surface]} is ${ratio.toFixed(2)}:1`);
    }
  }
});

test("Obsidian's ruler bar and playhead stay crisp #f5f5f7", () => {
  const palette = resolvePalette({ scheme: "obsidian", rows: {} });
  assert.equal(palette.rulerBar, "#f5f5f7");
  assert.equal(palette.playhead, "#f5f5f7");
});
