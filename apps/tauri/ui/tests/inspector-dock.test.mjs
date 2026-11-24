import assert from "node:assert/strict";
import { test } from "node:test";
import {
  ORDER_STORAGE_KEY,
  applyFloatingGeometry,
  applySavedOrder,
  clampFloatingPosition,
  clearFloatingGeometry,
  computeDropIndex,
  moveHostToIndex,
  normalizeOrder,
  readSavedOrder,
  reorderIds,
  saveCurrentOrder,
  shouldDetach,
  writeSavedOrder,
} from "../src/lib/inspectorDock.js";

const DEFAULT_IDS = [
  "map-preview",
  "inspector-selection",
  "inspector-lane",
  "inspector-difficulty",
  "inspector-bookmarks",
  "inspector-history",
];

function memoryStorage(initial = {}) {
  const map = new Map(Object.entries(initial));
  return {
    getItem: key => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => { map.set(key, String(value)); },
  };
}

function mockSidebar(ids) {
  const sidebar = {
    children: [],
    appendChild(node) {
      const idx = this.children.indexOf(node);
      if (idx >= 0) this.children.splice(idx, 1);
      this.children.push(node);
      return node;
    },
    insertBefore(node, ref) {
      const idx = this.children.indexOf(node);
      if (idx >= 0) this.children.splice(idx, 1);
      if (!ref) {
        this.children.push(node);
        return node;
      }
      const refIdx = this.children.indexOf(ref);
      if (refIdx < 0) this.children.push(node);
      else this.children.splice(refIdx, 0, node);
      return node;
    },
  };
  for (const id of ids) {
    const mount = { dataset: { uiMount: id }, matches: sel => sel === "[data-ui-mount]" };
    const section = {
      id,
      matches: sel => sel === "section.inspector-part",
      querySelector: sel => (sel === "[data-ui-mount]" ? mount : null),
    };
    sidebar.children.push(section);
  }
  return sidebar;
}

test("normalizeOrder keeps saved order, drops unknown/duplicate ids, and appends new sections", () => {
  assert.deepEqual(
    normalizeOrder(["inspector-history", "inspector-lane", "ghost", "inspector-lane"], DEFAULT_IDS),
    [
      "inspector-history",
      "inspector-lane",
      "map-preview",
      "inspector-selection",
      "inspector-difficulty",
      "inspector-bookmarks",
    ],
  );
  assert.deepEqual(normalizeOrder([], DEFAULT_IDS), DEFAULT_IDS);
  assert.deepEqual(normalizeOrder(null, DEFAULT_IDS), DEFAULT_IDS);
});

test("reorderIds moves a section up or down and clamps target index", () => {
  assert.deepEqual(reorderIds(["a", "b", "c", "d"], "c", 0), ["c", "a", "b", "d"]);
  assert.deepEqual(reorderIds(["a", "b", "c", "d"], "a", 2), ["b", "c", "a", "d"]);
  assert.deepEqual(reorderIds(["a", "b", "c"], "a", 99), ["b", "c", "a"]);
  assert.deepEqual(reorderIds(["a", "b", "c"], "c", -5), ["c", "a", "b"]);
  assert.deepEqual(reorderIds(["a", "b"], "missing", 1), ["a", "b"]);
});

test("computeDropIndex picks the slot based on section vertical midpoints", () => {
  const rects = [
    { top: 0, bottom: 100 },   // mid 50
    { top: 100, bottom: 200 }, // mid 150
    { top: 200, bottom: 300 }, // mid 250
  ];
  assert.equal(computeDropIndex(rects, 20, 1), 0);
  assert.equal(computeDropIndex(rects, 120, 0), 1);
  assert.equal(computeDropIndex(rects, 210, 0), 2);
  assert.equal(computeDropIndex(rects, 400, 0), 2);
});

test("shouldDetach triggers when pointer moves horizontally past the Inspector sidebar", () => {
  const rect = { left: 1000, right: 1260, width: 260 };
  assert.equal(shouldDetach(1100, rect), false);
  assert.equal(shouldDetach(980, rect), false);
  assert.equal(shouldDetach(970, rect), true);
  assert.equal(shouldDetach(1290, rect), true);
  assert.equal(shouldDetach(500, null), false);
});

test("clampFloatingPosition keeps detached panels reachable inside the viewport", () => {
  assert.deepEqual(clampFloatingPosition(300, 140, 260, 300, 1280, 720), { left: 300, top: 140 });
  assert.deepEqual(clampFloatingPosition(-40, -20, 260, 300, 1280, 720), { left: 8, top: 8 });
  assert.deepEqual(clampFloatingPosition(1200, 710, 260, 300, 1280, 720), { left: 1012, top: 680 });
});

test("readSavedOrder and writeSavedOrder persist safely under dawsu.inspectorOrder", () => {
  const store = memoryStorage();
  writeSavedOrder(["inspector-lane", "inspector-selection"], store);
  assert.equal(store.getItem(ORDER_STORAGE_KEY), JSON.stringify(["inspector-lane", "inspector-selection"]));
  assert.deepEqual(readSavedOrder(store), ["inspector-lane", "inspector-selection"]);

  const badStore = memoryStorage({ [ORDER_STORAGE_KEY]: "{not-json" });
  assert.deepEqual(readSavedOrder(badStore), []);

  const brokenStore = { getItem() { throw new Error("denied"); }, setItem() { throw new Error("denied"); } };
  assert.deepEqual(readSavedOrder(brokenStore), []);
  assert.doesNotThrow(() => writeSavedOrder(["a"], brokenStore));
});

test("applySavedOrder, moveHostToIndex, and saveCurrentOrder reorder sidebar DOM hosts", () => {
  const sidebar = mockSidebar(DEFAULT_IDS);
  const store = memoryStorage({
    [ORDER_STORAGE_KEY]: JSON.stringify(["inspector-bookmarks", "inspector-selection"]),
  });

  assert.equal(applySavedOrder(sidebar, store), true);
  assert.deepEqual(sidebar.children.map(el => el.id), [
    "inspector-bookmarks",
    "inspector-selection",
    "map-preview",
    "inspector-lane",
    "inspector-difficulty",
    "inspector-history",
  ]);
  // Applying again when already in order is a no-op.
  assert.equal(applySavedOrder(sidebar, store), false);

  // Move History to the top and save.
  const historyHost = sidebar.children.find(el => el.id === "inspector-history");
  assert.equal(moveHostToIndex(sidebar, historyHost, 0), true);
  const saved = saveCurrentOrder(sidebar, store);
  assert.equal(saved[0], "inspector-history");
  assert.deepEqual(readSavedOrder(store), saved);
});

test("applyFloatingGeometry and clearFloatingGeometry toggle floating state cleanly", () => {
  const classes = new Set();
  const panel = {
    classList: {
      add: (...names) => names.forEach(n => classes.add(n)),
      remove: (...names) => names.forEach(n => classes.delete(n)),
    },
    style: {},
  };
  applyFloatingGeometry(panel, 220, 120, 280);
  assert.ok(classes.has("insp-floating"));
  assert.equal(panel.style.left, "220px");
  assert.equal(panel.style.top, "120px");
  assert.equal(panel.style.width, "280px");
  assert.ok(Number(panel.style.zIndex) >= 40);

  clearFloatingGeometry(panel);
  assert.ok(!classes.has("insp-floating"));
  assert.equal(panel.style.left, "");
  assert.equal(panel.style.top, "");
  assert.equal(panel.style.width, "");
  assert.equal(panel.style.zIndex, "");
});
