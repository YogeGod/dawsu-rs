import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { recentItems, recentMapName, recoveryState } from "../src/lib/startScreen.js";
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
  for (const id of ["home-resume", "home-open", "home-import", "home-folders", "recovery", "recovery-entries", "home-recent", "recent"]) {
    assert.match(html, new RegExp('id="' + id + '"'), id);
  }
  assert.doesNotMatch(html, /Open from osu!/);
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
