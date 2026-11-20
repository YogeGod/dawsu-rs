import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import {
  applyRequest, autoStart, canCheck, checkedByDefault, checkLine, forget, initialState, isChecked, isFresh, landsText,
  laneLabel, listenFrom, nextPoll, pollDelay, progressText, reviewRequest, sameLane, settle, startRequest, stripMarks, take,
  ticked, keepTicks, timeText,
} from "../src/lib/drum.js";
import DrumDetectWindow from "../src/parts/DrumDetectWindow.svelte";

const lanes = [
  { key: "kick", name: "drum-hitnormal", hidden: false, number: 1 },
  { key: "kick2", name: "drum-hitnormal", hidden: false, number: 2 },
  { key: "clap", name: "Soft Clap", hidden: true, number: null },
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
  // Lanes read with their header number, so two lanes with one sample name stay apart.
  assert.deepEqual(lanes.map(laneLabel), ["drum-hitnormal 1", "drum-hitnormal 2", "Soft Clap (hidden)"]);
  assert.equal(progressText({ done: 25, total: 100 }), "Detecting drum hits… 25%");
});

test("the same lane for kicks and snares is refused before anything is sent", () => {
  assert.equal(sameLane({ kick: "clap", snare: "clap" }), true);
  assert.equal(sameLane({ kick: null, snare: null }), false);
  assert.equal(sameLane(options), false);
});

test("options go to the editor with the map the window shows, and quick changes add up", () => {
  const first = reviewRequest(ready, {}, { sensitivity: 62 });
  assert.deepEqual(first.request, { map: ready.map, options: { ...options, sensitivity: 62 } });
  // A second change before the editor answered the first keeps the first.
  const second = reviewRequest(ready, first.pending, { objectMs: 12 });
  assert.deepEqual(second.request.options, { ...options, sensitivity: 62, objectMs: 12 });
  // Once the editor holds a change it is no longer pending.
  assert.deepEqual(settle(second.pending, { ...ready, options: { ...options, sensitivity: 62 } }), { objectMs: 12 });
  assert.deepEqual(settle({}, ready), {});
});

test("one lane for kicks and snares is refused before anything is sent, and nothing stays pending", () => {
  const refused = reviewRequest(ready, { objectMs: 12 }, { snare: "kick" });
  assert.equal(refused.error, "Pick different lanes for kicks and snares.");
  assert.equal(refused.request, undefined);
  assert.deepEqual(refused.pending, { objectMs: 12 });
});

test("a map without a song starts on Drum stem; the song is analysed as soon as the window opens", () => {
  assert.equal(initialState(null).source, null);
  const noSong = take(initialState(null), { ...ready, state: "idle", hasSong: false, source: null, rows: undefined });
  assert.equal(noSong.source, "stem");
  assert.equal(autoStart(noSong, false), null, "a stem waits for a file");
  const song = take(initialState(null), { ...ready, state: "idle", source: null, rows: undefined });
  assert.equal(song.source, "song");
  assert.equal(autoStart(song, false), "song");
  assert.deepEqual(startRequest(song), { map: ready.map, source: { kind: "song" } });
  assert.equal(autoStart(song, true), null, "not while a command is out");
  // Once per source and map: a cancel or a failure doesn't start it again by itself...
  const once = { ...song, started: ["song"] };
  assert.equal(autoStart(once, false), null);
  // ...but a refused start is forgotten, so Detect (or the next poll) can try again.
  assert.equal(autoStart(forget(once, "song"), false), "song");
  const stem = { ...noSong, stemPath: "C:/stems/drums.wav" };
  assert.equal(autoStart(stem, false), "C:/stems/drums.wav");
  assert.deepEqual(startRequest(stem).source, { kind: "stem", path: "C:/stems/drums.wav" });
});

test("a new review keeps the ticks of the hits it still has; another map starts over", () => {
  let state = take(initialState(null), ready);
  assert.deepEqual(state.rows, rows);
  state = { ...state, overrides: { k10: false, k30: true }, picked: "k30", started: ["song"] };
  // A poll for the same review sends no rows: the window keeps its own.
  state = take(state, { ...ready, rows: undefined });
  assert.deepEqual(state.rows, rows);
  state = take(state, { ...ready, review: "78", rows: rows.filter(row => row.id !== "k30") });
  assert.deepEqual(state.overrides, { k10: false });
  state = take(state, { ...ready, state: "running", rows: undefined });
  assert.equal(state.rows, null, "a new run clears the list");
  state = take(state, { ...ready, map: "C:/Songs/other.osu" });
  assert.deepEqual([state.overrides, state.picked, state.started], [{}, null, []]);
});

test("Apply sends the review the mapper saw and the checked hits, and nothing without them", () => {
  const state = { ...take(initialState(null), ready), overrides: { k30: true } };
  assert.deepEqual(applyRequest(state), { map: ready.map, review: "77", ticked: ["k10", "s20", "k30", "k50"] });
  assert.equal(applyRequest({ ...state, overrides: { k10: false, s20: false, k30: false, k50: false } }), null);
  assert.equal(applyRequest(take(initialState(null), { ...ready, state: "running", rows: undefined })), null);
});

test("the window stops polling once closed, and drops answers older than the last it took", () => {
  assert.equal(nextPoll(true, ready), null);
  assert.equal(nextPoll(false, ready), 250);
  assert.equal(isFresh(3, 2), true);
  assert.equal(isFresh(2, 3), false);
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
  assert.match(body, /title="Relative to the strongest hit"[^>]*>Confidence</);
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

test("v3's note on where detection runs is shown", () => {
  assert.match(render(DrumDetectWindow, { props: { initialView: ready } }).body,
    /Runs on your PC with signal processing\. No audio is uploaded\. Works best on a drum stem\./);
});

test("an idle window with nothing detected offers Detect", () => {
  const idle = { ...ready, state: "idle", rows: undefined, review: undefined };
  assert.match(render(DrumDetectWindow, { props: { initialView: idle } }).body, />Detect<\/button>/);
  assert.doesNotMatch(render(DrumDetectWindow, { props: { initialView: ready } }).body, />Detect<\/button>/);
});

test("rows can be reached with the keyboard", () => {
  assert.match(render(DrumDetectWindow, { props: { initialView: ready } }).body, /class="row[^"]*"[^>]*tabindex="0"|tabindex="0"[^>]*class="row/);
});

test("a stem source shows its file and offset", () => {
  const stem = { ...ready, source: { kind: "stem", path: "C:\\stems\\echoes-drums.wav", name: "echoes-drums.wav" }, options: { ...options, stemOffset: 120 } };
  const { body } = render(DrumDetectWindow, { props: { initialView: stem } });
  assert.match(body, /echoes-drums\.wav/);
  assert.match(body, />Change<\/button>/);
  assert.match(body, /value="120"/);
  assert.match(body, /Stem starts at/);
});
