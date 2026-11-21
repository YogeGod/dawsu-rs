import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  audioLabel, cannotSaveLabel, hitLaneCounts, nameOf, PLAYBACK_SPEEDS, problemTooltip, selectionLabel,
  splitFileName, statusText, ZOOM_MIN, zoomSliderValue,
} from "../src/lib/statusbar.js";
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

function shown(view, scene = map, notice = "") {
  bridge.set(view, scene);
  bridge.statusNotice = notice;
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
  assert.doesNotMatch(html, /\sstyle=|data-playback-speed|data-doc|data-map|data-action/);
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
});
