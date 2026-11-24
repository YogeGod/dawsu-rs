import assert from "node:assert/strict";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";
import { dockingRules } from "./docking-rules.mjs";
import { bootEditor } from "./editor-runtime.mjs";

async function editor() {
  const elements = new Map();
  function element() {
    const ctx2d = new Proxy({}, {
      get(target, name) {
        if (name in target) return target[name];
        return () => {};
      },
    });
    return {
      getContext: () => ctx2d,
      clientWidth: 300,
      clientHeight: 80,
      scrollTop: 0,
      open: false,
      hidden: false,
      disabled: false,
      value: "",
      textContent: "",
      title: "",
      dataset: {},
      style: { setProperty() {}, removeProperty() {} },
      children: [],
      attributes: new Map(),
      listeners: {},
      showModal() { this.open = true; },
      close() { this.open = false; },
      append() {},
      prepend() {},
      replaceChildren() {},
      setAttribute(k, v) { this.attributes.set(k, String(v)); },
      getAttribute(k) { return this.attributes.get(k) ?? null; },
      removeAttribute(k) { this.attributes.delete(k); },
      classList: { toggle() {}, add() {}, remove() {}, contains: () => false },
      addEventListener(name, listener) { (this.listeners[name] ??= []).push(listener); },
      getBoundingClientRect: () => ({ x: 0, y: 0, left: 0, top: 0, width: 300, height: 80 }),
      querySelector: () => null,
      querySelectorAll: () => [],
    };
  }
  const get = (id) => {
    if (!elements.has(id)) elements.set(id, element());
    return elements.get(id);
  };

  let now = 0;
  let tick = null;
  let nextView = null;
  const invoked = [];
  const uiUpdates = [];
  const menuRefreshes = [];
  const toasts = [];
  const scene = {
    path: "fixture.osu",
    previewKey: 1,
    viewStart: 0,
    zoom: 0.1,
    rowHeight: 30,
    moments: [],
    waveform: [],
    lanes: [],
    difficulties: [],
    sections: [[0, 500, 4]],
    gameplay: { points: [], spans: [] },
    bookmarks: [],
    history: [],
    historyPosition: 0,
  };

  const context = createContext({
    document: {
      readyState: "complete",
      getElementById: get,
      createElement: element,
      querySelectorAll: () => [],
      addEventListener() {},
    },
    window: {
      devicePixelRatio: 1,
      editorDocking: dockingRules,
      dawsuUi: {
        update(view, sc) { uiUpdates.push({ view, sc }); },
      },
      refreshEditorMenu() { menuRefreshes.push(now); },
      showToast(toast) { toasts.push(toast); },
      __TAURI__: {
        core: {
          invoke: async (name, args) => {
            invoked.push({ name, args });
            if (name === "editor_view") return { ...nextView };
            return undefined;
          },
        },
        event: { listen() {} },
      },
    },
    performance: { now: () => now },
    ResizeObserver: class { observe() {} },
    Option: class { constructor(text, value) { this.text = text; this.value = value; } },
    requestAnimationFrame() { return 1; },
    cancelAnimationFrame() {},
    setInterval(callback) { tick = callback; },
    console,
  });

  const baseView = {
    hasDocument: true,
    showHome: false,
    busy: false,
    playing: false,
    dirty: false,
    fault: null,
    status: "Ready",
    error: null,
    warnings: [],
    position: 1000,
    musicalPosition: "1.1.0 · 120.0 BPM · 4/4 · 1/4",
    duration: 60000,
    speed: 1,
    left: 0,
    right: 0,
    cycle: false,
    activeLane: 0,
    tool: "Select",
    follow: false,
    offset: 0,
    songMuted: false,
    songSolo: false,
    hitsoundsMuted: false,
    hitsoundsSolo: false,
    song: 1,
    main: 1,
    hitsounds: 1,
    objectLines: false,
    hsMode: false,
    outputProblems: [],
    drumVst: null,
    soundSetPrompts: [],
    panels: { browser: true, inspector: true, controller: true, laneNames: true },
    recent: ["fixture.osu"],
    recovery: null,
  };

  nextView = { ...baseView, scene };
  await bootEditor(context, { autoStart: true });
  get("volume-controller-panel").hidden = true;
  get("grid").clientWidth = 300;
  invoked.length = 0;

  const counts = {
    renderFrame: 0,
    renderActiveLane: 0,
    renderPanelLayout: 0,
    renderSelection: 0,
    refreshLaneToolbar: 0,
    renderDialogs: 0,
    paintSoon: 0,
    followBookmarkPick: 0,
    uiUpdate: 0,
    menuRefresh: 0,
  };
  runInContext(`
    for (const name of ["renderFrame", "renderActiveLane", "renderPanelLayout", "renderSelection", "refreshLaneToolbar", "renderDialogs", "paintSoon", "followBookmarkPick"]) {
      const orig = globalThis[name];
      globalThis[name] = (...args) => {
        globalThis.__counts[name]++;
        return orig(...args);
      };
    }
  `, context);
  context.__counts = counts;

  const resetCounts = () => {
    for (const key of Object.keys(counts)) counts[key] = 0;
    uiUpdates.length = 0;
    menuRefreshes.length = 0;
    toasts.length = 0;
    invoked.length = 0;
  };

  return {
    baseView,
    counts,
    uiUpdates,
    menuRefreshes,
    toasts,
    invoked,
    get,
    resetCounts,
    async pollAt(at, patch = {}) {
      now = at;
      nextView = {
        ...baseView,
        warnings: [...baseView.warnings],
        outputProblems: [...baseView.outputProblems],
        soundSetPrompts: [...baseView.soundSetPrompts],
        panels: { ...baseView.panels },
        recent: [...baseView.recent],
        ...patch,
      };
      await tick();
      counts.uiUpdate = uiUpdates.length;
      counts.menuRefresh = menuRefreshes.length;
    },
    evaluate: (code) => runInContext(code, context),
  };
}

test("an unchanged 1-second idle poll skips redundant DOM, menu, dialog, and canvas work", async () => {
  const ed = await editor();
  ed.resetCounts();

  await ed.pollAt(1000);

  assert.equal(ed.invoked.length, 1);
  assert.equal(ed.invoked[0].args.action, null);
  assert.equal(ed.invoked[0].args.scene, false);

  assert.equal(ed.counts.followBookmarkPick, 1, "still updates bookmark pick state");
  assert.equal(ed.counts.renderFrame, 0);
  assert.equal(ed.counts.renderActiveLane, 0);
  assert.equal(ed.counts.renderPanelLayout, 0);
  assert.equal(ed.counts.renderSelection, 0);
  assert.equal(ed.counts.refreshLaneToolbar, 0);
  assert.equal(ed.counts.renderDialogs, 0);
  assert.equal(ed.counts.paintSoon, 0);
  assert.equal(ed.counts.menuRefresh, 0);

  // If a transient status notice was set by an earlier failed call, the next unchanged idle poll clears it.
  ed.evaluate('setStatusNotice("Transient error")');
  assert.equal(ed.get("status").textContent, "Transient error");
  ed.resetCounts();
  await ed.pollAt(2000);
  assert.equal(ed.counts.renderSelection, 0);
  assert.equal(ed.counts.paintSoon, 0);
  assert.equal(ed.get("status").textContent, "Ready");
});

test("idle poll takes the full refresh path when status, error, warnings, position, or problem/view fields change", async () => {
  const ed = await editor();

  // 1. Status change (e.g. finished background job / save)
  ed.resetCounts();
  await ed.pollAt(1000, { status: "Saved 3 files" });
  assert.equal(ed.counts.renderFrame, 1);
  assert.equal(ed.counts.renderPanelLayout, 1);
  assert.equal(ed.counts.renderSelection, 1);
  assert.equal(ed.counts.refreshLaneToolbar, 1);
  assert.equal(ed.counts.renderDialogs, 1);
  assert.equal(ed.counts.paintSoon, 1);
  assert.equal(ed.counts.menuRefresh, 1);
  assert.equal(ed.counts.uiUpdate, 1);
  assert.equal(ed.get("status").textContent, "Saved 3 files");
  assert.equal(ed.toasts.length, 1);

  // Subsequent unchanged poll at the new status skips again
  ed.baseView.status = "Saved 3 files";
  ed.resetCounts();
  await ed.pollAt(2000);
  assert.equal(ed.counts.renderSelection, 0);
  assert.equal(ed.counts.paintSoon, 0);
  assert.equal(ed.counts.menuRefresh, 0);

  // 2. Error change opens modal via full path
  ed.resetCounts();
  await ed.pollAt(3000, { error: "Could not write file" });
  assert.equal(ed.counts.renderDialogs, 1);
  assert.equal(ed.get("dialog-title").textContent, "Editor message");
  assert.equal(ed.evaluate("modalKind"), "error");

  // 3. Warnings content change takes full path
  ed.resetCounts();
  await ed.pollAt(4000, { warnings: ["Missing sample"] });
  assert.equal(ed.counts.renderSelection, 1);
  assert.equal(ed.counts.uiUpdate, 1);

  ed.baseView.warnings = ["Missing sample"];
  ed.resetCounts();
  await ed.pollAt(5000, { warnings: ["Different sample"] });
  assert.equal(ed.counts.renderSelection, 1);
  assert.equal(ed.counts.uiUpdate, 1);

  // 4. Position change takes full path and repaints
  ed.resetCounts();
  await ed.pollAt(6000, { position: 2500 });
  assert.equal(ed.counts.paintSoon, 1);
  assert.equal(ed.counts.uiUpdate, 1);

  // 5. Output problems content change (same length) takes full path
  ed.baseView.outputProblems = [{ time: 100, message: "Old", target: "a.osu", timeline: "a.osu" }];
  await ed.pollAt(7000);
  ed.resetCounts();
  await ed.pollAt(8000, { outputProblems: [{ time: 100, message: "New", target: "a.osu", timeline: "a.osu" }] });
  assert.equal(ed.counts.renderSelection, 1);
  assert.equal(ed.counts.paintSoon, 1);

  // 6. objectLines, hsMode, drumVst, soundSetPrompts, and bookmarkStep changes take full path
  let nextAt = 9000;
  for (const patch of [
    { objectLines: true },
    { hsMode: true },
    { drumVst: { loaded: true, enabled: true, pluginName: "EZdrummer", mappedLanes: 4 } },
    { soundSetPrompts: [{ setId: "k", name: "Kick", placements: 2, editedKeepFiles: 0, open: false }] },
    { bookmarkStep: 500 },
  ]) {
    ed.resetCounts();
    await ed.pollAt(nextAt, patch);
    nextAt += 1000;
    assert.equal(ed.counts.renderDialogs, 1, "expected full path for " + JSON.stringify(patch));
    assert.equal(ed.counts.menuRefresh, 1, "expected menu refresh for " + JSON.stringify(patch));
  }
});
