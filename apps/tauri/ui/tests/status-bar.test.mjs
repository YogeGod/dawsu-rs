import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { hasPriorityStatus, hintsFor, hintZoneFromTarget } from "../src/lib/hints.js";
import {
  audioLabel, cannotSaveLabel, hitLaneCounts, nameOf, PLAYBACK_SPEEDS, problemTooltip, selectionLabel,
  splitFileName, statusText, ZOOM_MIN, zoomSliderValue,
} from "../src/lib/statusbar.js";
import { TOOLS } from "../src/lib/tools.js";
import StatusBar from "../src/parts/StatusBar.svelte";

const map = {
  path: "C:/Songs/Set/Artist - Song (Mapper) [Hard].osu",
  lanes: [{ key: "a" }, { key: "b" }],
  laneOrder: [0, 1],
  moments: [[100, 100, true, [[0, false], [1, false]]], [200, 100, true, [[0, false]]]],
  selection: { count: 0 },
  zoom: 1,
  rowHeight: 30,
};

test("statusbar helpers format file names, problems, counts, status and audio labels", () => {
  assert.deepEqual(PLAYBACK_SPEEDS, [0.25, 0.5, 0.75, 1, 1.25, 1.5]);
  assert.equal(nameOf("C:\\Songs\\Map\\Song [Hard].osu"), "Song [Hard].osu");
  assert.deepEqual(splitFileName("C:/Songs/Artist - Song (Me) [Hard].osu"), {
    head: "Artist - Song (Me) ",
    tail: "[Hard].osu",
  });
  assert.deepEqual(splitFileName(""), { head: "", tail: "" });
  assert.equal(cannotSaveLabel(1), "1 moment can't be saved");
  assert.equal(cannotSaveLabel(3), "3 moments can't be saved");
  const thirteen = Array.from({ length: 13 }, (_, i) => ({ time: i, message: "Moment " + i }));
  assert.equal(problemTooltip(thirteen), [
    "Moment 0", "Moment 1", "Moment 2", "Moment 3", "Moment 4",
    "Moment 5", "Moment 6", "Moment 7", "Moment 8", "Moment 9",
    "and 3 more",
  ].join("\n"));
  assert.equal(selectionLabel(1), "1 hit selected");
  assert.equal(selectionLabel(4), "4 hits selected");
  assert.equal(hitLaneCounts(map), "3 hits · 2 lanes");
  assert.equal(hitLaneCounts({ ...map, laneOrder: [0], moments: [[100, 80, true, [[0, false]]]] }), "1 hit · 1 lane");
  assert.equal(statusText("Part of the editor did not load", { status: "Ready", busy: true }), "Part of the editor did not load");
  assert.equal(statusText("", { fault: "Stopped", status: "Ready" }), "Stopped");
  assert.equal(statusText("", { busy: true }), "Working…");
  assert.deepEqual(audioLabel("Speakers"), { text: "Speakers", ariaLabel: "Audio output: Speakers" });
  assert.deepEqual(audioLabel(""), { text: "Audio output", ariaLabel: "Audio output" });
  assert.equal(zoomSliderValue(1), 0);
  assert.equal(zoomSliderValue(0.1), -1);
});

function shown(view, scene = map, notice = "", hint = null) {
  bridge.set(view, scene);
  bridge.statusNotice = notice;
  bridge.setHint(hint);
  return render(StatusBar).body;
}

test("StatusBar renders controls in v3 order with speed buttons, zoom, file name, counts and audio", () => {
  const html = shown({
    busy: false,
    playing: true,
    speed: 0.75,
    status: "Ready",
    warnings: ["Clipping"],
    outputDevice: "Speakers (Realtek)",
    outputProblems: [{ time: 1, message: "Too loud" }, { time: 2, message: "Missing" }],
  }, { ...map, selection: { count: 4 } });

  const order = [
    'id="playback-rates"', 'id="play-state"', 'id="selection-count"', 'id="status"', 'id="problem-summary"',
    'id="next-output-problem"', 'id="warnings"', 'id="cancel-job"', 'id="file-name"', 'id="hit-lane-counts"',
    'id="status-view"', 'id="audio"',
  ].map(marker => html.indexOf(marker));
  assert.ok(order.every(index => index >= 0), String(order));
  assert.deepEqual(order, [...order].sort((a, b) => a - b));
  assert.ok(html.indexOf('id="file-name-head"') < html.indexOf('id="file-name-tail"'));
  assert.match(html, /min="-3\.301"[^>]*id="zoom"/);
  assert.equal(ZOOM_MIN, -3.301);
  assert.match(html, /aria-label="Playback speed 75%"[^>]*aria-pressed="true"/);
  assert.match(html, /id="play-state"[^>]*data-playing="true"[^>]*>\s*Playing\s*</);
  assert.match(html, /id="selection-count"[^>]*>\s*4 hits selected\s*</);
  assert.match(html, /id="problem-summary"[^>]*title="Too loud\nMissing"/);
  assert.match(html, /id="problem-summary-text">2 moments can't be saved</);
  assert.match(html, /id="hit-lane-counts"[^>]*>3 hits · 2 lanes</);
  assert.match(html, /id="audio"[^>]*aria-label="Audio output: Speakers \(Realtek\)"[^>]*>Speakers \(Realtek\)</);
  // Tests and tools find each speed button by its value.
  assert.deepEqual([...html.matchAll(/data-playback-speed="([^"]+)"/g)].map(match => match[1]),
    ["0.25", "0.5", "0.75", "1", "1.25", "1.5"]);
  assert.match(html, /<button[^>]*data-playback-speed="0.75"[^>]*aria-pressed="true"|<button[^>]*aria-pressed="true"[^>]*data-playback-speed="0.75"/);
  assert.doesNotMatch(html, /\sstyle=|data-doc|data-map|data-action/);
});

test("StatusBar hides map controls when no map is open and disables jobs cleanly", () => {
  const empty = shown({ busy: true, status: "Opening", outputDevice: "" }, null, "Part of the editor did not load");
  assert.match(empty, /id="playback-rates"[^>]*hidden/);
  assert.match(empty, /id="play-state"[^>]*hidden/);
  assert.match(empty, /id="status-view"[^>]*hidden/);
  assert.match(empty, /id="hit-lane-counts"[^>]*hidden/);
  assert.match(empty, /id="file-name"[^>]*hidden/);
  assert.doesNotMatch(empty, /id="cancel-job"[^>]*hidden/);
  assert.match(empty, /id="status"[^>]*>Part of the editor did not load</);
  assert.match(empty, /id="audio"[^>]*>Audio output</);
  bridge.statusNotice = "";
  bridge.setHint(null);
});

test("hintsFor returns tool-aware timeline hints, panel hints, and empty for unknown zones", () => {
  for (const tool of TOOLS) {
    const items = hintsFor("timeline", tool);
    assert.ok(items.length >= 2, `timeline hints for ${tool}`);
    for (const item of items) {
      assert.ok(item.keys.length > 0 && item.action.length > 0);
    }
  }
  assert.ok(hintsFor("timeline", "Draw").some(item => item.keys === "Click" && item.action === "Place hit"));
  assert.ok(hintsFor("timeline", "Select").some(item => item.keys === "Drag" && item.action === "Box select"));
  assert.ok(hintsFor("timeline", "Ramp").some(item => item.keys === "Drag" && item.action === "Ramp volume"));
  assert.ok(hintsFor("ruler", "Select").some(item => item.keys === "Ctrl+Click" && item.action === "L locator"));
  assert.ok(hintsFor("volume", "Select").some(item => item.keys === "Shift+Drag" && item.action === "Ramp"));
  assert.ok(hintsFor("lanes", "Select").some(item => item.keys === "M" && item.action === "Mute"));
  assert.ok(hintsFor("mixer", "Select").some(item => item.keys === "Drag fader"));
  assert.ok(hintsFor("browser", "Select").length >= 2);
  assert.ok(hintsFor("inspector", "Select").length >= 2);
  assert.ok(hintsFor("overview", "Select").length >= 2);
  assert.deepEqual(hintsFor("unknown", "Select"), []);
  assert.deepEqual(hintsFor(null, "Select"), []);
  assert.deepEqual(hintsFor("", "Draw"), []);

  assert.equal(hintZoneFromTarget({ closest: () => ({ dataset: { hintZone: "ruler" } }) }), "ruler");
  assert.equal(hintZoneFromTarget({ closest: () => null }), null);
  assert.equal(hintZoneFromTarget(null), null);

  assert.equal(hasPriorityStatus("", { busy: false, fault: null, status: "Ready" }), false);
  assert.equal(hasPriorityStatus("Error", { busy: false }), true);
  assert.equal(hasPriorityStatus("", { busy: true }), true);
  assert.equal(hasPriorityStatus("", { fault: "Stopped" }), true);
});

test("StatusBar renders kbd hover hints when bridge.hint is set and falls back cleanly", () => {
  const withHint = shown({ busy: false, tool: "Draw", status: "Ready" }, map, "", "timeline");
  assert.match(withHint, /id="hint-bar"/);
  assert.match(withHint, /<kbd>Click<\/kbd><span>Place hit<\/span>/);
  assert.match(withHint, /<kbd>Right-click<\/kbd><span>Delete<\/span>/);
  assert.match(withHint, /id="status"[^>]*hidden/);

  const cleared = shown({ busy: false, tool: "Draw", status: "Ready" }, map, "", null);
  assert.doesNotMatch(cleared, /id="hint-bar"/);
  assert.doesNotMatch(cleared, /id="status"[^>]*hidden/);
  assert.match(cleared, /id="status"[^>]*>Ready</);

  // Active status notice or busy job keeps normal status visible instead of flickering to hints.
  const busy = shown({ busy: true, tool: "Draw", status: "Saving" }, map, "", "timeline");
  assert.doesNotMatch(busy, /id="hint-bar"/);
  assert.doesNotMatch(busy, /id="status"[^>]*hidden/);

  const notice = shown({ busy: false, tool: "Draw", status: "Ready" }, map, "Audio stopped", "ruler");
  assert.doesNotMatch(notice, /id="hint-bar"/);
  assert.match(notice, /id="status"[^>]*>Audio stopped</);

  bridge.statusNotice = "";
  bridge.setHint(null);
});
