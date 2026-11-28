import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  PRACTICE_TRIED_KEY,
  SETUP_CHECKLIST_DISMISSED_KEY,
  dismissSetupChecklist,
  isAudioOutputReady,
  isHitsoundVersion,
  isPracticeTried,
  isSetupChecklistDismissed,
  markPracticeTried,
  recentItems,
  recentMapName,
  recoveryState,
  setupChecklistState,
  songsSourceTooltip,
} from "../src/lib/startScreen.js";
import { sameUiView } from "../src/lib/view.js";
import { detectImageMime } from "../src/lib/mapsetBackground.js";
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

  const enriched = recentItems({
    recent: [mapPath],
    recentMeta: [{ path: mapPath, background: "data:image/png;base64,AA==", isHs: true }],
  });
  assert.deepEqual(enriched, [
    {
      path: mapPath,
      name: "yuikonnu - Chikyuu Saigo no Kokuhaku wo",
      difficulty: "[Kokuhaku in 2019]",
      backgroundUrl: "data:image/png;base64,AA==",
      isHs: true,
    },
  ]);

  assert.equal(isHitsoundVersion("Hitsounds"), true);
  assert.equal(isHitsoundVersion("HS"), true);
  assert.equal(isHitsoundVersion("Normal HS"), true);
  assert.equal(isHitsoundVersion("Insane"), false);
  assert.equal(songsSourceTooltip("osu!.db"), "Indexed from osu! database");
  assert.equal(songsSourceTooltip("cache"), "Loaded from local Songs cache");
  assert.equal(songsSourceTooltip("folder-scan"), "Scanned from Songs folder");
  assert.equal(songsSourceTooltip("none"), "");

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
  for (const id of ["home-resume", "home-open", "home-create-hs", "home-open-osu", "home-import", "home-folders", "home-practice", "recovery", "recovery-entries", "home-recent", "recent"]) {
    assert.match(html, new RegExp('id="' + id + '"'), id);
  }
  assert.match(html, /Create Hitsound diff…/);
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
  assert.match(html, /id="home-practice"[^>]*disabled/);
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

test("Songs folder grid uses max-content rows and never collapses 50+ cards inside max-height", async () => {
  const { readFileSync } = await import("node:fs");
  const svelte = readFileSync(new URL("../src/parts/StartScreen.svelte", import.meta.url), "utf8");
  const intake = readFileSync(new URL("../src/parts/MapsetIntakeCard.svelte", import.meta.url), "utf8");
  assert.match(svelte, /:global\(#home\)\s*\.home-panel\.has-right-intake\s*\{[^}]*justify-items:\s*stretch;/);
  assert.match(svelte, /\.home-right-intake\s*\{[^}]*width:\s*100%;[^}]*min-width:\s*0;/);
  assert.match(svelte, /\.home-songs\s*\{[^}]*display:\s*flex;\s*flex-direction:\s*column;/);
  assert.match(svelte, /\.home-songs-list\s*\{[^}]*grid-auto-rows:\s*max-content;[^}]*align-content:\s*start;/);
  assert.doesNotMatch(svelte, /\.home-song-set\s*\{[^}]*overflow:\s*hidden/);
  assert.match(svelte, /\.home-panel\.has-right-intake\s*\.home-main\s*\.home-actions/);
  assert.match(svelte, /\.home-panel\.has-right-intake\s*\.home-setup-list/);
  assert.match(svelte, /home-song-backdrop/);
  assert.match(svelte, /home-song-bg/);
  assert.match(intake, /\.intake-path\s*\{[^}]*white-space:\s*normal;/);
});

test("detectImageMime recognizes PNG, JPEG, WebP, GIF, and BMP magic headers", () => {
  assert.equal(detectImageMime(new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])), "image/png");
  assert.equal(detectImageMime(new Uint8Array([0xff, 0xd8, 0xff, 0xe0])), "image/jpeg");
  assert.equal(detectImageMime(new Uint8Array([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50])), "image/webp");
  assert.equal(detectImageMime(new Uint8Array([0x47, 0x49, 0x46, 0x38, 0x39, 0x61])), "image/gif");
  assert.equal(detectImageMime(new Uint8Array([0x42, 0x4d, 0, 0])), "image/bmp");
  assert.equal(detectImageMime(new Uint8Array([1, 2, 3, 4])), "");
  assert.equal(detectImageMime(new Uint8Array([])), "");
});

test("setupChecklistState tracks 4-step live progress, practice completion, audio check, and dismissal", () => {
  const store = new Map();
  const storage = {
    getItem: key => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, String(value)),
  };

  assert.equal(isSetupChecklistDismissed(storage), false);
  assert.equal(isPracticeTried(storage), false);
  assert.equal(isAudioOutputReady(""), false);
  assert.equal(isAudioOutputReady("Audio unavailable"), false);
  assert.equal(isAudioOutputReady("AUDIO UNAVAILABLE"), false);
  assert.equal(isAudioOutputReady("Speakers (Realtek)"), true);

  const empty = setupChecklistState(
    { outputDevice: "Audio unavailable", songsFolder: "", keyPreset: null, firstLaunch: true, recent: [] },
    false
  );
  assert.equal(empty.visible, true);
  assert.equal(empty.allDone, false);
  assert.equal(empty.completedCount, 0);
  assert.equal(empty.totalCount, 4);
  assert.deepEqual(empty.steps.map(s => [s.id, s.done, s.stateLabel, s.action]), [
    ["audio", false, "Set output", "audio"],
    ["songs-folder", false, "Choose", "folders"],
    ["shortcut-style", false, "Choose", "shortcuts"],
    ["first-map", false, "Try practice", "tryPractice"],
  ]);

  const partial = setupChecklistState(
    { outputDevice: "Speakers (Realtek)", songsFolder: "C:\\osu!\\Songs", keyPreset: "DAW", firstLaunch: false, recent: [] },
    false
  );
  assert.equal(partial.visible, true);
  assert.equal(partial.completedCount, 3);
  assert.deepEqual(partial.steps.map(s => [s.id, s.done, s.stateLabel]), [
    ["audio", true, "Ready"],
    ["songs-folder", true, "Set"],
    ["shortcut-style", true, "DAW"],
    ["first-map", false, "Try practice"],
  ]);

  markPracticeTried(storage);
  assert.equal(store.get(PRACTICE_TRIED_KEY), "1");
  assert.equal(isPracticeTried(storage), true);
  const completedByPractice = setupChecklistState(
    { outputDevice: "Speakers (Realtek)", songsFolder: "C:\\osu!\\Songs", keyPreset: "osu!", firstLaunch: false, recent: [] },
    false,
    { practiceTried: isPracticeTried(storage) }
  );
  assert.equal(completedByPractice.allDone, true);
  assert.equal(completedByPractice.visible, false);

  const complete = setupChecklistState(
    { outputDevice: "Speakers (Realtek)", songsFolder: "C:\\osu!\\Songs", keyPreset: "osu!", firstLaunch: false, recent: [mapPath] },
    false
  );
  assert.equal(complete.allDone, true);
  assert.equal(complete.visible, false);

  dismissSetupChecklist(storage);
  assert.equal(store.get(SETUP_CHECKLIST_DISMISSED_KEY), "1");
  assert.equal(isSetupChecklistDismissed(storage), true);
  assert.equal(setupChecklistState({ outputDevice: "", songsFolder: "", keyPreset: null, recent: [] }, true).visible, false);
});

test("sameUiView detects keyPreset, firstLaunch, and isPracticeMapset transitions", () => {
  const base = { busy: false, songsFolder: "", hitsoundLibrary: "", outputDevice: "", keyPreset: "", firstLaunch: true, isPracticeMapset: false, recent: [], warnings: [], outputProblems: [] };
  assert.equal(sameUiView(base, { ...base }), true);
  assert.equal(sameUiView(base, { ...base, keyPreset: "DAW" }), false);
  assert.equal(sameUiView(base, { ...base, firstLaunch: false }), false);
  assert.equal(sameUiView(base, { ...base, isPracticeMapset: true }), false);
});

test("StartScreen renders compact Ready checklist when incomplete and hides when dismissed or complete", () => {
  const previousStorage = globalThis.localStorage;
  const store = new Map();
  globalThis.localStorage = {
    getItem: key => store.get(key) ?? null,
    setItem: (key, value) => store.set(key, String(value)),
  };

  try {
    // Incomplete: visible with compact 4-pill Ready strip and disables first-map once done.
    bridge.set({
      busy: false,
      hasDocument: false,
      showHome: true,
      outputDevice: "Speakers (Realtek)",
      songsFolder: "C:\\osu!\\Songs",
      keyPreset: null,
      firstLaunch: true,
      recent: [],
    }, null);
    let html = render(StartScreen).body;
    assert.match(html, /id="home-setup"/);
    assert.doesNotMatch(html, /id="home-setup"[^>]*hidden/);
    assert.match(html, /Ready/);
    assert.match(html, /2 of 4 ready/);
    assert.match(html, /data-step="audio"[^>]*>[\s\S]*?Ready/);
    assert.match(html, /data-step="songs-folder"[^>]*>[\s\S]*?Set/);
    assert.match(html, /data-step="shortcut-style"[^>]*>[\s\S]*?Choose/);
    assert.match(html, /data-step="first-map"[^>]*>[\s\S]*?Try practice/);
    assert.match(html, /id="home-setup-dismiss"/);
    assert.doesNotMatch(html, /\sstyle=/);

    // Practice tried via localStorage flips first-map to done + disabled when checklist stays open.
    store.set(PRACTICE_TRIED_KEY, "1");
    html = render(StartScreen).body;
    assert.match(html, /3 of 4 ready/);
    assert.match(html, /data-step="first-map"[^>]*disabled/);

    // All 4 complete: automatically hidden.
    bridge.set({
      busy: false,
      hasDocument: false,
      showHome: true,
      outputDevice: "Speakers (Realtek)",
      songsFolder: "C:\\osu!\\Songs",
      keyPreset: "DAW",
      firstLaunch: false,
      recent: [],
    }, null);
    html = render(StartScreen).body;
    assert.match(html, /id="home-setup"[^>]*hidden/);

    // Dismissed in localStorage: stays hidden even when incomplete.
    store.clear();
    store.set(SETUP_CHECKLIST_DISMISSED_KEY, "1");
    bridge.set({
      busy: false,
      hasDocument: false,
      showHome: true,
      outputDevice: "Audio unavailable",
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

test("StartScreen recovery cards group Restore and Discard for each copy", () => {
  bridge.set({
    busy: false,
    hasDocument: false,
    showHome: true,
    recent: [],
    recovery: {
      error: "",
      offers: [
        { id: "rec-1", mapset: "Mapset", difficulty: "Insane", created_ms: 1700000000000 },
        { id: "rec-2", mapset: "Other", difficulty: "Hard", created_ms: 1700000001000 },
      ],
    },
  }, null);
  const html = render(StartScreen).body;
  const entries = [...html.matchAll(/<div class="recovery-entry">([\s\S]*?)<\/div><\/div>/g)].map(match => match[1]);
  assert.equal(entries.length, 2);
  assert.match(entries[0], /aria-label="Restore Mapset \[Insane\]"/);
  assert.match(entries[0], /aria-label="Discard Mapset \[Insane\]"/);
  assert.match(entries[1], /aria-label="Discard Other \[Hard\]"/);
  assert.match(entries[0], /title="Moves this copy to the recovery archive folder"[^>]*>Discard</);
  assert.doesNotMatch(html, /\sstyle=/);
});

test("sameUiView detects an autosave interval change and treats a missing one as 30 s", () => {
  const base = { busy: false, songsFolder: "", hitsoundLibrary: "", outputDevice: "", recent: [], warnings: [], outputProblems: [] };
  assert.equal(sameUiView({ ...base, autosaveIntervalSecs: 30 }, base), true);
  assert.equal(sameUiView({ ...base, autosaveIntervalSecs: 30 }, { ...base, autosaveIntervalSecs: 15 }), false);
  assert.equal(sameUiView({ ...base, autosaveIntervalSecs: 0 }, base), false);
});

test("a Songs card's difficulty chip wraps a long name instead of cutting off what tells two copies apart", async () => {
  // "... (copy)" and "... (copy) (1)" both showed as "Echoes Through Eternal Time (co...".
  const { readFileSync } = await import("node:fs");
  const css = readFileSync(new URL("../src/parts/StartScreen.svelte", import.meta.url), "utf8").split("<style>")[1];
  const rule = css.match(/\.home-song-diff > span:first-child\s*\{([^}]*)\}/)?.[1] ?? "";
  assert.doesNotMatch(rule, /nowrap|ellipsis/);
  assert.match(rule, /overflow-wrap:\s*anywhere;/);
});

test("a Songs card's mapset title and mapper wrap instead of cutting off the song name", async () => {
  // "Arch Echo - SUPER SUDDEN DEATH (Yoge God) (26)" showed as "Arch Echo - SUPER SUDDEN D...".
  const { readFileSync } = await import("node:fs");
  const css = readFileSync(new URL("../src/parts/StartScreen.svelte", import.meta.url), "utf8").split("<style>")[1];
  for (const selector of [".home-song-meta strong", ".home-song-creator"]) {
    const rule = css.match(new RegExp(selector.replace(/[.]/g, "\\.") + String.raw`\s*\{([^}]*)\}`))?.[1] ?? "";
    assert.ok(rule, `${selector} rule found`);
    assert.doesNotMatch(rule, /nowrap|ellipsis/, selector);
    assert.match(rule, /overflow-wrap:\s*anywhere;/, selector);
  }
});
