import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import {
  canCheck, checkedByDefault, checkLine, isChecked, landsText, laneLabel, listenFrom, optionsRequest, pollDelay,
  progressText, sameLane, stripMarks, ticked, keepTicks, timeText,
} from "../src/lib/drum.js";
import DrumDetectWindow from "../src/parts/DrumDetectWindow.svelte";

const lanes = [
  { key: "kick", name: "Drum Normal", hidden: false },
  { key: "clap", name: "Soft Clap", hidden: true },
];
const rows = [
  { id: "k10", drum: "kick", time: 500, strength: 1, lands: "circle" },
  { id: "s20", drum: "snare", time: 1000, strength: 0.61, lands: "grid" },
  { id: "k30", drum: "kick", time: 1520, strength: 0.4, lands: "flagged" },
  { id: "s40", drum: "snare", time: 2000, strength: 0.7, lands: "duplicate" },
  { id: "k50", drum: "kick", time: 2500, strength: 0.5, lands: "sliderEnd" },
];
const options = { kick: "kick", snare: "clap", sensitivity: 50, objectMs: 30, grid: null, stemOffset: 0 };
const ready = {
  map: "C:\\Songs\\set\\map.osu", title: "Artist - Song", hasSong: true, duration: 4000, state: "ready",
  error: null, source: { kind: "song" }, options, editorGrid: 4, grids: [1, 2, 3, 4, 6, 8, 12, 16], maxObjectMs: 100,
  lanes, listening: false, review: "77", rows, unhides: ["Soft Clap"],
  counts: { added: 3, onObjects: 2, loose: 1, flagged: 1, duplicates: 1 },
};

test("hits on objects and grid lines start checked; flagged ones start unchecked; duplicates can't be checked", () => {
  assert.deepEqual(rows.map(checkedByDefault), [true, true, false, false, true]);
  assert.deepEqual(rows.map(canCheck), [true, true, true, false, true]);
  assert.equal(canCheck({ lands: "noLane" }), false);
  const overrides = { k30: true, k10: false, s40: true };
  assert.deepEqual(rows.map(row => isChecked(row, overrides)), [false, true, true, false, true]);
  assert.deepEqual(ticked(rows, overrides), ["s20", "k30", "k50"]);
  assert.equal(checkLine(rows, {}), "3 of 4 hits checked");
  assert.equal(checkLine([rows[0]], {}), "1 of 1 hit checked");
});

test("a new review keeps the mapper's ticks for the hits it still has", () => {
  assert.deepEqual(keepTicks({ k10: false, gone: true }, rows), { k10: false });
  assert.deepEqual(keepTicks({}, null), {});
});

test("rows read as osu! mappers say them", () => {
  assert.equal(timeText(319353), "05:19.353");
  assert.equal(timeText(500.4), "00:00.500");
  assert.deepEqual(["circle", "sliderHead", "sliderRepeat", "sliderEnd", "spinnerEnd", "grid", "flagged", "duplicate", "noLane"]
    .map(landsText), ["Circle", "Slider head", "Slider repeat", "Slider end", "Spinner end", "Grid line",
    "Too far from the grid", "Already there", "No lane picked"]);
  assert.equal(laneLabel(lanes[1]), "Soft Clap (hidden)");
  assert.equal(progressText({ done: 25, total: 100 }), "Detecting drum hits… 25%");
});

test("the same lane for kicks and snares is refused before anything is sent", () => {
  assert.equal(sameLane({ kick: "clap", snare: "clap" }), true);
  assert.equal(sameLane({ kick: null, snare: null }), false);
  assert.equal(sameLane(options), false);
});

test("options go to the editor with the map the window shows", () => {
  assert.deepEqual(optionsRequest(ready, { sensitivity: 62 }), { map: ready.map, options: { ...options, sensitivity: 62 } });
});

test("the window polls faster while it detects or listens", () => {
  assert.equal(pollDelay(null), 250);
  assert.equal(pollDelay(ready), 250);
  assert.equal(pollDelay({ state: "running" }), 100);
  assert.equal(pollDelay({ ...ready, listening: true }), 100);
});

test("Listen starts a second before the picked hit, or at the start", () => {
  assert.equal(listenFrom(rows, "s20"), 0);
  assert.equal(listenFrom(rows, "k50"), 1500);
  assert.equal(listenFrom(rows, null), 0);
  assert.equal(listenFrom(rows, "gone"), 0);
});

test("the strip puts kicks low and snares high across the song, checked ones solid", () => {
  const marks = stripMarks(rows, {}, 4000);
  assert.deepEqual(marks[0], { x: 0.125, drum: "kick", on: true });
  assert.deepEqual(marks[2], { x: 0.38, drum: "kick", on: false });
  assert.equal(marks.length, 5);
  assert.deepEqual(stripMarks(rows, {}, 0), []);
});

test("before the editor answers the window shows a quiet empty state with Apply off", () => {
  const { body } = render(DrumDetectWindow);
  assert.match(body, /Loading…/);
  assert.match(body, /<button[^>]*class="primary[^"]*"[^>]*disabled[^>]*>Apply<\/button>/);
});

test("with no map open the window says so", () => {
  const { body } = render(DrumDetectWindow, { props: { initialView: { ...ready, map: null, state: "idle", rows: undefined } } });
  assert.match(body, /Open a difficulty in the editor first\./);
});

test("with no lane picked the window asks for one", () => {
  const none = { ...ready, options: { ...options, kick: null, snare: null } };
  assert.match(render(DrumDetectWindow, { props: { initialView: none } }).body, /Pick a lane for kicks or snares to add hits\./);
  assert.doesNotMatch(render(DrumDetectWindow, { props: { initialView: ready } }).body, /Pick a lane/);
});

test("the review lists each hit with its tick, time, sound, strength and where it lands", () => {
  const { body } = render(DrumDetectWindow, { props: { initialView: ready } });
  assert.match(body, />Song<\/button>/);
  assert.match(body, />Drum stem<\/button>/);
  assert.match(body, /Kicks/);
  assert.match(body, /Snares/);
  assert.match(body, /<option value="clap"[^>]*>Soft Clap \(hidden\)<\/option>/);
  assert.match(body, /Sensitivity/);
  assert.match(body, /Snap to objects within/);
  assert.match(body, /Beat grid/);
  assert.match(body, /<canvas/);
  assert.match(body, />Strength</);
  assert.doesNotMatch(body, /Confidence/, "the score is relative strength, not a probability");
  assert.match(body, /00:01\.520/);
  assert.match(body, /Too far from the grid/);
  assert.match(body, /class="row flagged/);
  assert.match(body, /aria-label="Add the kick at 00:01\.520" disabled|disabled[^>]*aria-label="Add the snare at 00:02\.000"/);
  assert.match(body, /3 of 4 hits checked/);
  assert.match(body, /Applying shows the hidden lane Soft Clap\./);
  assert.match(body, />Listen<\/button>/);
  assert.match(body, /Song on the left, cues on the right/);
  assert.match(body, /<button[^>]*class="primary[^"]*"[^>]*>Apply<\/button>/);
  assert.doesNotMatch(body, /<button[^>]*class="primary[^"]*"[^>]*disabled/);
});

test("while detecting the list shows the progress and Apply waits", () => {
  const running = { ...ready, state: "running", done: 3, total: 4, rows: undefined, review: undefined };
  const { body } = render(DrumDetectWindow, { props: { initialView: running } });
  assert.match(body, /Detecting drum hits… 75%/);
  assert.match(body, /<button[^>]*class="primary[^"]*"[^>]*disabled[^>]*>Apply<\/button>/);
});

test("a stem source shows its file and offset", () => {
  const stem = { ...ready, source: { kind: "stem", path: "C:\\stems\\echoes-drums.wav", name: "echoes-drums.wav" }, options: { ...options, stemOffset: 120 } };
  const { body } = render(DrumDetectWindow, { props: { initialView: stem } });
  assert.match(body, /echoes-drums\.wav/);
  assert.match(body, />Change<\/button>/);
  assert.match(body, /value="120"/);
});
