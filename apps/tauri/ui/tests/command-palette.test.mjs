import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { items, keyOnly, menus } from "../src/lib/commands.js";
import {
  MAX_RECENT,
  RECENT_STORAGE_KEY,
  buildEntries,
  filterAndRank,
  firstEnabledIndex,
  nextEnabledIndex,
  readRecent,
  recordRecent,
  scoreMatch,
} from "../src/lib/commandPalette.js";
import CommandPalette from "../src/parts/CommandPalette.svelte";

const fakeStorage = (initial = {}) => {
  const data = new Map(Object.entries(initial));
  return {
    getItem: key => (data.has(key) ? data.get(key) : null),
    setItem: (key, value) => data.set(key, String(value)),
    data,
  };
};

const sampleShortcuts = {
  preset: "DAW",
  entries: [
    ["save", "Save", ["Ctrl+S"]],
    ["undo", "Undo", ["Ctrl+Z"]],
    ["redo", "Redo", ["Ctrl+Y", "Ctrl+Shift+Z"]],
    ["quantizeDialog", "Quantize…", ["Ctrl+Q"]],
    ["play", "Play / pause", ["Space"]],
    ["cycle", "Cycle between L and R", ["L", "Numpad /", "/"]],
    ["zoomIn", "Zoom in", ["Ctrl+="]],
    ["volumeUp", "Volume up", ["Alt+Up"]],
    ["deselect", "Clear selection", ["Escape"]],
  ],
};

const openMapScene = {
  path: "Test.osu",
  selection: { count: 2 },
  range: { start: 0, end: 1000 },
  placement: null,
  history: ["Draw hit", "Move 2 hits"],
  historyPosition: 2,
  lanes: [{ sourceKey: "kick", canDelete: false }],
};

const openMapView = {
  hasDocument: true,
  busy: false,
  fault: null,
  activeLane: 0,
};

test("buildEntries pulls menu commands and keyOnly shortcuts with active bindings and enabled state", () => {
  const entries = buildEntries({
    items,
    menus,
    keyOnly,
    shortcuts: sampleShortcuts,
    scene: openMapScene,
    view: openMapView,
  });

  const save = entries.find(e => e.id === "save");
  assert.ok(save);
  assert.equal(save.group, "File");
  assert.equal(save.shortcutText, "Ctrl+S");
  assert.equal(save.enabled, true);

  const undo = entries.find(e => e.id === "undo");
  assert.ok(undo);
  assert.equal(undo.label, "Undo · Move 2 hits");
  assert.equal(undo.shortcutText, "Ctrl+Z");
  assert.equal(undo.enabled, true);

  // keyOnly command with a catalogue label appears under Shortcuts.
  const volUp = entries.find(e => e.id === "cmd:volumeUp");
  assert.ok(volUp);
  assert.equal(volUp.label, "Volume up");
  assert.equal(volUp.group, "Shortcuts");
  assert.equal(volUp.shortcutText, "Alt+Up");
  assert.equal(volUp.enabled, true);

  // On the Start screen (no scene), map-only commands are disabled while idle commands stay enabled.
  const startEntries = buildEntries({
    items,
    menus,
    keyOnly,
    shortcuts: sampleShortcuts,
    scene: null,
    view: { hasDocument: false, busy: false },
  });
  assert.equal(startEntries.find(e => e.id === "openMap").enabled, true);
  assert.equal(startEntries.find(e => e.id === "preferences").enabled, true);
  assert.equal(startEntries.find(e => e.id === "save").enabled, false);
  assert.equal(startEntries.find(e => e.id === "cmd:volumeUp").enabled, false);
  assert.equal(startEntries.find(e => e.id === "quickStart").enabled, false);
  for (const id of ["shortcuts", "helpWiki", "helpFaq", "helpBugReport", "openCrashLogs"]) {
    const entry = startEntries.find(e => e.id === id);
    assert.ok(entry, id);
    assert.equal(entry.group, "Help");
    assert.equal(entry.enabled, true, id);
  }
  assert.equal(entries.find(e => e.id === "quickStart").group, "Help");
  assert.equal(entries.find(e => e.id === "quickStart").enabled, true);
});

test("fuzzy search matches labels, groups, and shortcuts in ranked order", () => {
  const entries = buildEntries({
    items,
    menus,
    keyOnly,
    shortcuts: sampleShortcuts,
    scene: openMapScene,
    view: openMapView,
  });

  // Exact / prefix label match ranks first.
  const saveMatches = filterAndRank(entries, "save");
  assert.equal(saveMatches[0].id, "save");
  assert.ok(scoreMatch(saveMatches[0], "save") > scoreMatch(saveMatches[1], "save"));

  // Subsequence fuzzy match works on labels.
  const quantMatches = filterAndRank(entries, "qnt");
  assert.ok(quantMatches.some(e => e.id === "quantize"));

  // Group name search surfaces commands from that group.
  const playbackMatches = filterAndRank(entries, "playback");
  assert.ok(playbackMatches.length >= 5);
  assert.ok(playbackMatches.every(e => e.group === "Playback" || e.label.toLowerCase().includes("playback")));

  // Multi-word query across group + label narrows accurately.
  const mapBookmark = filterAndRank(entries, "map bookmark");
  assert.ok(mapBookmark.length >= 3);
  assert.ok(mapBookmark.every(e => e.group === "Map" && e.label.toLowerCase().includes("bookmark")));

  // Shortcut search matches active bindings (including secondary bindings like Ctrl+Shift+Z).
  const shortcutMatches = filterAndRank(entries, "Ctrl+Q");
  assert.equal(shortcutMatches[0].id, "quantize");

  const secondaryShortcut = filterAndRank(entries, "Ctrl+Shift+Z");
  assert.ok(secondaryShortcut.some(e => e.id === "redo"));
});

test("empty query puts recent commands first in recency order and persists via storage", () => {
  const storage = fakeStorage();
  assert.deepEqual(readRecent(storage), []);

  let recent = recordRecent(storage, "zoomIn");
  recent = recordRecent(storage, "quantize", recent);
  recent = recordRecent(storage, "save", recent);
  recent = recordRecent(storage, "quantize", recent);
  assert.deepEqual(recent, ["quantize", "save", "zoomIn"]);
  assert.deepEqual(readRecent(storage), ["quantize", "save", "zoomIn"]);
  assert.equal(storage.data.get(RECENT_STORAGE_KEY), JSON.stringify(["quantize", "save", "zoomIn"]));

  // Capped at MAX_RECENT.
  for (let i = 0; i < MAX_RECENT + 4; i++) recent = recordRecent(storage, `item-${i}`, recent);
  assert.equal(recent.length, MAX_RECENT);

  const entries = buildEntries({
    items,
    menus,
    keyOnly,
    shortcuts: sampleShortcuts,
    scene: openMapScene,
    view: openMapView,
  });
  const ordered = filterAndRank(entries, "", ["quantize", "zoomIn", "save"]);
  assert.deepEqual(ordered.slice(0, 3).map(e => e.id), ["quantize", "zoomIn", "save"]);
  assert.equal(ordered[0].recent, true);
  assert.equal(ordered[1].recent, true);
  assert.equal(ordered[2].recent, true);
  assert.equal(ordered[3].recent, false);
});

test("keyboard navigation helpers skip disabled entries and wrap cleanly", () => {
  const list = [
    { id: "a", enabled: false },
    { id: "b", enabled: true },
    { id: "c", enabled: false },
    { id: "d", enabled: true },
  ];
  assert.equal(firstEnabledIndex(list), 1);
  assert.equal(nextEnabledIndex(list, 1, 1), 3);
  assert.equal(nextEnabledIndex(list, 3, 1), 1);
  assert.equal(nextEnabledIndex(list, 1, -1), 3);
  assert.equal(nextEnabledIndex([{ id: "x", enabled: false }], 0, 1), -1);
});

test("CommandPalette.svelte hides when closed and renders entries with active shortcut keycaps when open", () => {
  bridge.commandPaletteOpen = false;
  bridge.commandPaletteShortcuts = null;
  bridge.set(openMapView, openMapScene);

  const closedHtml = render(CommandPalette).body;
  assert.equal(closedHtml.includes("cmdp-modal"), false);

  bridge.commandPaletteShortcuts = sampleShortcuts;
  bridge.commandPaletteOpen = true;
  const openHtml = render(CommandPalette).body;

  assert.match(openHtml, /role="dialog"/);
  assert.match(openHtml, /aria-label="Command palette"/);
  assert.match(openHtml, /Type a command or shortcut/);
  assert.match(openHtml, /<kbd class="cmdp-key[^"]*">Ctrl\+S<\/kbd>/);
  assert.match(openHtml, /<kbd class="cmdp-key[^"]*">Ctrl\+Q<\/kbd>/);
  assert.match(openHtml, /<kbd class="cmdp-key[^"]*">Alt\+Up<\/kbd>/);

  // Clean up shared bridge state for other tests.
  bridge.commandPaletteOpen = false;
  bridge.commandPaletteShortcuts = null;
});
