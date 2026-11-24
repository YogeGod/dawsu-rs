import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  SETUP_CHECKLIST_DISMISSED_KEY,
  dismissSetupChecklist,
  isSetupChecklistDismissed,
  recentItems,
  recentMapName,
  recoveryState,
  setupChecklistState,
} from "../src/lib/startScreen.js";
import StartScreen from "../src/parts/StartScreen.svelte";

const mapPath = "C:\\Songs\\1 yuikonnu - Chikyuu\\yuikonnu - Chikyuu Saigo no Kokuhaku wo (RealCuT) [Kokuhaku in 2019].osu";

test("startScreen helpers format recent map names and gate recovery offers", () => {
  assert.deepEqual(recentMapName(mapPath), ["yuikonnu - Chikyuu Saigo no Kokuhaku wo", "[Kokuhaku in 2019]"]);
  assert.deepEqual(recentMapName("C:/maps/recent.osu"), ["recent", ""]);
  assert.deepEqual(recentMapName("C:/maps/Some Pack.OSZ"), ["Some Pack", ""]);

  const items = recentItems({ recent: [mapPath, "C:/maps/recent.osu"] });
  assert.deepEqual(items, [
    { path: mapPath, name: "yuikonnu - Chikyuu Saigo no Kokuhaku wo", difficulty: "[Kokuhaku in 2019]" },
    { path: "C:/maps/recent.osu", name: "recent", difficulty: "" },
  ]);

  const recovery = { error: "", offers: [{ id: "rec-1", mapset: "Song", difficulty: "Hard", created_ms: 1700000000000 }] };
  assert.equal(recoveryState({ hasDocument: true, recovery }, null).visible, false);
  assert.equal(recoveryState({ hasDocument: false, recovery }, { path: "a.osu" }).visible, false);
  const shown = recoveryState({ hasDocument: false, recovery }, null);
  assert.equal(shown.visible, true);
  assert.equal(shown.offers.length, 1);
  assert.equal(shown.offers[0].name, "Song [Hard]");
  assert.equal(recoveryState({ hasDocument: false, recovery: { error: "Unreadable", offers: [] } }, null).visible, true);
});

test("StartScreen renders actions, drop zone, resume, recent maps and recovery copies", () => {
  bridge.set({
    busy: false,
    hasDocument: true,
    showHome: true,
    recent: [mapPath, "C:/maps/recent.osu"],
    recovery: {
      error: "Check disk",
      offers: [{ id: "rec-1", mapset: "Mapset", difficulty: "Insane", created_ms: 1700000000000 }],
    },
  }, null);
  let html = render(StartScreen).body;
  for (const id of ["home-resume", "home-open", "home-open-osu", "home-import", "home-folders", "recovery", "recovery-entries", "home-recent", "recent"]) {
    assert.match(html, new RegExp('id="' + id + '"'), id);
  }
  assert.match(html, /Open from osu!/);
  assert.match(html, /class="home-drop"[^>]*>[\s\S]*?\.osu[\s\S]*?\.osz/);
  assert.doesNotMatch(html, /id="home-resume"[^>]*hidden/);
  assert.doesNotMatch(html, /id="home-recent"[^>]*hidden/);
  assert.match(html, /<strong>yuikonnu - Chikyuu Saigo no Kokuhaku wo<\/strong>\s*<span>\[Kokuhaku in 2019\]<\/span>/);
  assert.match(html, /id="recovery"[^>]*hidden/, "recovery stays hidden when hasDocument is true");

  bridge.set({
    busy: true,
    hasDocument: false,
    showHome: true,
    recent: [],
    recovery: {
      error: "Check disk",
      offers: [{ id: "rec-1", mapset: "Mapset", difficulty: "Insane", created_ms: 1700000000000 }],
    },
  }, null);
  html = render(StartScreen).body;
  assert.match(html, /id="home-resume"[^>]*hidden/);
  assert.match(html, /id="home-recent"[^>]*hidden/);
  assert.doesNotMatch(html, /id="recovery"[^>]*hidden/);
  assert.match(html, /Check disk/);
  assert.match(html, /aria-label="Restore Mapset \[Insane\]"[^>]*disabled/);
  assert.match(html, /id="home-open"[^>]*disabled/);
  assert.match(html, /id="home-import"[^>]*disabled/);
  assert.match(html, /id="home-folders"[^>]*disabled/);
  assert.doesNotMatch(html, /\sstyle=/);
});

test("StartScreen shows Songs folder search when songsFolder is configured", () => {
  bridge.set({
    busy: false,
    hasDocument: false,
    showHome: true,
    recent: [],
    songsFolder: "",
  }, null);
  let html = render(StartScreen).body;
  assert.match(html, /id="home-songs"[^>]*hidden/);

  bridge.set({
    busy: false,
    hasDocument: false,
    showHome: true,
    recent: [],
    songsFolder: "C:\\osu!\\Songs",
  }, null);
  html = render(StartScreen).body;
  assert.doesNotMatch(html, /id="home-songs"[^>]*hidden/);
  assert.match(html, /id="home-songs-search"/);
  assert.match(html, /id="home-songs-refresh"/);
  assert.match(html, /id="home-songs-list"/);
  assert.doesNotMatch(html, /\sstyle=/);
});

test("setupChecklistState tracks live progress, completion, and dismissal", () => {
  const store = new Map();
  const storage = {
    getItem: key => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, String(value)),
  };

  assert.equal(isSetupChecklistDismissed(storage), false);

  const empty = setupChecklistState({ songsFolder: "", keyPreset: null, firstLaunch: true, recent: [] }, false);
  assert.equal(empty.visible, true);
  assert.equal(empty.allDone, false);
  assert.equal(empty.completedCount, 0);
  assert.deepEqual(empty.steps.map(s => [s.id, s.done]), [
    ["songs-folder", false],
    ["shortcut-style", false],
    ["first-map", false],
  ]);

  const partial = setupChecklistState({ songsFolder: "C:\\osu!\\Songs", keyPreset: "DAW", firstLaunch: false, recent: [] }, false);
  assert.equal(partial.visible, true);
  assert.equal(partial.completedCount, 2);
  assert.deepEqual(partial.steps.map(s => [s.id, s.done]), [
    ["songs-folder", true],
    ["shortcut-style", true],
    ["first-map", false],
  ]);

  const complete = setupChecklistState({ songsFolder: "C:\\osu!\\Songs", keyPreset: "osu!", firstLaunch: false, recent: [mapPath] }, false);
  assert.equal(complete.allDone, true);
  assert.equal(complete.visible, false);

  dismissSetupChecklist(storage);
  assert.equal(store.get(SETUP_CHECKLIST_DISMISSED_KEY), "1");
  assert.equal(isSetupChecklistDismissed(storage), true);
  assert.equal(setupChecklistState({ songsFolder: "", keyPreset: null, recent: [] }, true).visible, false);
});

test("StartScreen renders Get set up checklist when incomplete and hides when dismissed or complete", () => {
  const previousStorage = globalThis.localStorage;
  const store = new Map();
  globalThis.localStorage = {
    getItem: key => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, String(value)),
  };

  try {
    // Incomplete: visible with action buttons for remaining steps and Done for completed ones.
    bridge.set({
      busy: false,
      hasDocument: false,
      showHome: true,
      songsFolder: "C:\\osu!\\Songs",
      keyPreset: null,
      firstLaunch: true,
      recent: [],
    }, null);
    let html = render(StartScreen).body;
    assert.match(html, /id="home-setup"/);
    assert.doesNotMatch(html, /id="home-setup"[^>]*hidden/);
    assert.match(html, /Get set up/);
    assert.match(html, /1 of 3 done/);
    assert.match(html, /data-step="songs-folder"[^>]*>[\s\S]*?Done/);
    assert.match(html, /data-step="shortcut-style"[^>]*>[\s\S]*?Open Shortcuts/);
    assert.match(html, /data-step="first-map"[^>]*>[\s\S]*?Open map/);
    assert.match(html, /id="home-setup-dismiss"/);
    assert.doesNotMatch(html, /\sstyle=/);

    // All 3 complete: automatically hidden.
    bridge.set({
      busy: false,
      hasDocument: false,
      showHome: true,
      songsFolder: "C:\\osu!\\Songs",
      keyPreset: "DAW",
      firstLaunch: false,
      recent: [mapPath],
    }, null);
    html = render(StartScreen).body;
    assert.match(html, /id="home-setup"[^>]*hidden/);

    // Dismissed in localStorage: stays hidden even when incomplete.
    store.set(SETUP_CHECKLIST_DISMISSED_KEY, "1");
    bridge.set({
      busy: false,
      hasDocument: false,
      showHome: true,
      songsFolder: "",
      keyPreset: null,
      firstLaunch: true,
      recent: [],
    }, null);
    html = render(StartScreen).body;
    assert.match(html, /id="home-setup"[^>]*hidden/);
  } finally {
    if (previousStorage === undefined) delete globalThis.localStorage;
    else globalThis.localStorage = previousStorage;
  }
});
