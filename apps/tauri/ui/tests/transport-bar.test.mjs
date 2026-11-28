import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { Bridge, bridge } from "../src/lib/bridge.svelte.js";
import {
  clickGate, copyText, osuTimestamp, parseTime, readout, segmentsDisabled, selectionValue, transportActions,
} from "../src/lib/transport.js";
import { samePlayhead, sameUiView } from "../src/lib/view.js";
import TransportBar from "../src/parts/TransportBar.svelte";

const map = { path: "Hard.osu", selection: null };

test("the readout shows the playhead time, tempo and time signature, with dashes before a map", () => {
  const live = { position: 106052, musicalPosition: "38.2.240 · 190.0 BPM · 7/8 · 1/4" };
  assert.deepEqual(readout(live, map), { time: "01:46.052", bpm: "190.0", meter: "7/8" });
  assert.deepEqual(readout({ position: 3725004 }, map), { time: "62:05.004", bpm: "–", meter: "–" });
  for (const [view, scene] of [[null, null], [live, null], [{ position: NaN }, map]]) {
    assert.deepEqual(readout(view, scene), { time: "00:00.000", bpm: "–", meter: "–" });
  }
});

test("SELECTION is the osu! timestamp of the selection's start, milliseconds floored", () => {
  assert.equal(osuTimestamp(324324.9), "05:24:324 -");
  assert.equal(osuTimestamp(0), "00:00:000 -");
  assert.equal(osuTimestamp(3725004), "62:05:004 -");
  assert.equal(osuTimestamp(-3), "00:00:000 -");
  assert.equal(selectionValue({ ...map, selection: { count: 4, start: 324324, object: "Circle" } }), "05:24:324 -");
  assert.equal(selectionValue(map), null);
  assert.equal(selectionValue(null), null);
});

test("the buttons send the editor's own playback actions", () => {
  const send = transportActions({ cycle: true, duration: 201500 });
  assert.deepEqual(send.start, { kind: "start" });
  assert.deepEqual(send.end, { kind: "seek", ms: 201500 });
  assert.deepEqual(send.cycle, { kind: "cycle", on: false });
  assert.deepEqual(send.stop, { kind: "stop" });
  assert.deepEqual(send.play, { kind: "play" });
  assert.deepEqual(transportActions({ cycle: false, duration: 1 }).cycle, { kind: "cycle", on: true });
});

test("Play, Stop and Cycle need only a map; Go to start and end also wait for a running job", () => {
  const ready = { busy: false, duration: 5000 };
  assert.deepEqual(segmentsDisabled(null, null), { playback: true, jump: true, end: true });
  assert.deepEqual(segmentsDisabled(ready, null), { playback: true, jump: true, end: true });
  assert.deepEqual(segmentsDisabled(ready, map), { playback: false, jump: false, end: false });
  assert.deepEqual(segmentsDisabled({ ...ready, busy: true }, map), { playback: false, jump: true, end: true });
  assert.deepEqual(segmentsDisabled({ ...ready, fault: "The editor stopped" }, map), { playback: true, jump: true, end: true });
  // Go to end needs the song's length.
  assert.deepEqual(segmentsDisabled({ busy: false }, map), { playback: false, jump: false, end: true });
});

test("Play and Cycle take one click per view, so a second click before the editor answers does nothing", () => {
  const gate = clickGate();
  const first = { playing: false };
  assert.equal(gate(first), true);
  assert.equal(gate(first), false, "same view: the first click is still on its way");
  assert.equal(gate({ playing: true }), true, "the next view opens the gate again");
});

test("a copy reports copied, or failed when the clipboard refuses", async () => {
  const written = [];
  assert.equal(await copyText("05:24:324 -", { writeText: async text => written.push(text) }), "copied");
  assert.deepEqual(written, ["05:24:324 - "]);
  assert.equal(await copyText("05:24:324 -", { writeText: async () => { throw new Error("NotAllowedError"); } }), "failed");
  assert.equal(await copyText("05:24:324 -", undefined), "failed");
});

function shown(view, scene = map) {
  bridge.set(view, scene);
  return render(TransportBar).body;
}
const buttons = html => [...html.matchAll(/<button\b[^>]*>/g)].map(match => match[0]);
// The transport buttons, before the readout's SELECTION button.
const bar = html => buttons(html.slice(0, html.indexOf('class="readout')));
const label = tag => tag.match(/aria-label="([^"]+)"/)?.[1];

test("the bar is collapse tab, Go to start, Go to end, Cycle, Stop, Play and MIDI Record, with no L or R", () => {
  const html = shown({ playing: false, cycle: true, busy: false, position: 0, duration: 5000 });
  assert.deepEqual(bar(html).map(label),
    ["Go to start and Go to end", "Go to start", "Go to end", "Cycle", "Stop", "Play", "MIDI Record"]);
  // MIDI Record is an arm toggle: unarmed until midi_record_toggle answers armed.
  const record = bar(html).at(-1);
  assert.doesNotMatch(record, /\sdisabled/);
  assert.match(record, /aria-pressed="false"/);
  assert.match(record, /title="Arm MIDI recording"/);
  assert.match(bar(html)[0], /aria-expanded="true"/, "the tab's state is in aria-expanded, its name stays");
  assert.match(html, /title="Cycle between L and R"/);
  assert.doesNotMatch(html, /Locator|>L<|>R</);
  assert.equal([...html.matchAll(/<svg/g)].length, 7, "every button has its icon; no copy icon without a selection");
  assert.doesNotMatch(html, /\sstyle=|\sid=|\(Space\)|\(End\)|\(\/\)/);
});

test("Stop lights when stopped, Play is pressed while playing, Cycle while cycling", () => {
  const pressed = html => bar(html).filter(tag => /aria-pressed="true"/.test(tag)).map(label);
  // Stop is a button, not a toggle: it lights while stopped but never announces "pressed".
  let html = shown({ playing: false, cycle: false, busy: false });
  assert.deepEqual(pressed(html), []);
  const stop = bar(html).find(tag => label(tag) === "Stop");
  assert.match(stop, /class="[^"]*\bon\b/);
  assert.doesNotMatch(stop, /aria-pressed/);
  assert.doesNotMatch(bar(shown({ playing: true, cycle: false, busy: false })).find(tag => label(tag) === "Stop"), /class="[^"]*\bon\b/);
  assert.deepEqual(pressed(shown({ playing: true, cycle: true, busy: false })), ["Cycle", "Play"]);
  assert.match(bar(shown({ playing: true, cycle: false, busy: false })).find(tag => label(tag) === "Play"), /class="[^"]*\bplaying\b/);
});

test("before a map every button but the collapse tab is disabled; a job leaves playback working", () => {
  const disabled = html => bar(html).filter(tag => /\sdisabled/.test(tag)).map(label);
  assert.deepEqual(disabled(shown(null, null)), ["Go to start", "Go to end", "Cycle", "Stop", "Play", "MIDI Record"]);
  assert.deepEqual(disabled(shown({ busy: true, playing: false, cycle: false, duration: 5000 })), ["Go to start", "Go to end"]);
  assert.deepEqual(disabled(shown({ busy: false, playing: false, cycle: false, duration: 5000 })), []);
  assert.deepEqual(disabled(shown({ busy: false, playing: false, cycle: false })), ["Go to end"]);
});

test("the readout shows time, BPM, TIME and SELECTION, and SELECTION copies the timestamp", () => {
  let html = shown({ busy: false, position: 106052, musicalPosition: "38.2.240 · 190.0 BPM · 7/8 · 1/4" },
    { ...map, selection: { count: 2, start: 324324, object: "Circle" } });
  const text = html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");
  for (const part of ["01:46.052", "190.0 BPM", "7/8 TIME", "05:24:324 - SELECTION"]) assert.ok(text.includes(part), part);
  assert.doesNotMatch(text, /BAR|38\.2\.240/);
  const copy = buttons(html).find(tag => /title="Copy/.test(tag));
  assert.ok(copy, "SELECTION is a button");
  assert.match(copy, /aria-label="Copy selection time"/);
  assert.doesNotMatch(html, /<span[^>]*aria-label/, "no label on a span without a role");
  assert.doesNotMatch(copy, /\sdisabled/);

  html = shown({ busy: false, position: 0 });
  assert.match(html.replace(/<[^>]+>/g, " "), /No selection/);
  assert.match(buttons(html).find(tag => /title="Copy/.test(tag)), /\sdisabled/, "nothing to copy");
  html = shown(null, null);
  assert.match(html.replace(/<[^>]+>/g, " "), /00:00\.000[^]*–[^]*–[^]*No selection/);
});

const baseView = (overrides = {}) => ({
  busy: false,
  fault: null,
  playing: false,
  cycle: false,
  duration: 200000,
  speed: 1,
  audition: false,
  tool: "Select",
  follow: false,
  returnOnStop: false,
  panels: { browser: true, inspector: true, controller: true, laneNames: true },
  song: 1,
  main: 1,
  outputDevice: "Speakers",
  songMuted: false,
  songSolo: false,
  status: "Ready",
  showHome: false,
  hasDocument: true,
  objectLines: false,
  warnings: [],
  outputProblems: [],
  recent: [],
  recovery: null,
  position: 1000,
  musicalPosition: "1.1.0 · 120.0 BPM · 4/4 · 1/4",
  ...overrides,
});

test("sameUiView and samePlayhead compare only the fields each subscriber needs", () => {
  const a = baseView();
  assert.equal(sameUiView(a, a), true);
  assert.equal(sameUiView(a, null), false);
  assert.equal(sameUiView(null, a), false);
  assert.equal(sameUiView(null, null), true);
  assert.equal(sameUiView(a, baseView({ position: 2500, musicalPosition: "2.3.0 · 180.0 BPM · 3/4 · 1/4" })), true);
  assert.equal(sameUiView(baseView({ duration: NaN, song: undefined, main: undefined }),
    baseView({ duration: NaN, song: undefined, main: undefined })), true);
  assert.equal(sameUiView(baseView({ panels: { controller: true } }), baseView({ panels: {} })), true);
  assert.equal(sameUiView(baseView({ panels: { controller: false } }), baseView({ panels: { controller: true } })), false);
  // Preferences and the drum lane column read these, so a change to one reaches them.
  assert.equal(sameUiView(a, baseView({ hsMode: true })), false);
  assert.equal(sameUiView(a, baseView({ songsFolder: "E:/osu!/Songs" })), false);
  assert.equal(sameUiView(a, baseView({ hitsoundLibrary: "E:/samples" })), false);
  assert.equal(sameUiView(a, baseView({ offset: 12 })), false);
  assert.equal(sameUiView(a, baseView({ playbackStart: "start" })), false);
  assert.equal(sameUiView(a, baseView({ classicBrowser: true })), false);
  const drum = (more) => baseView({ drumView: { enabled: true, name: "Kit", autoDetect: true, hideUnassigned: true, rows: [], ...more } });
  assert.equal(sameUiView(drum(), drum()), true);
  assert.equal(sameUiView(drum(), drum({ hideUnassigned: false })), false);
  assert.equal(sameUiView(drum(), drum({ autoDetect: false })), false);
  assert.equal(sameUiView(drum(), drum({ name: "Other kit" })), false);
  assert.equal(sameUiView(drum(), drum({ rows: [{ index: 0, name: "Kick", laneKey: "a", assigned: true }] })), false);

  assert.equal(samePlayhead(a, a), true);
  assert.equal(samePlayhead(a, null), false);
  assert.equal(samePlayhead(null, a), false);
  assert.equal(samePlayhead(a, baseView({ tool: "Draw", cycle: true, busy: true })), true);
  assert.equal(samePlayhead(a, baseView({ position: 1085 })), false);
  assert.equal(samePlayhead(a, baseView({ musicalPosition: "1.2.0 · 120.0 BPM · 4/4 · 1/4" })), false);
  assert.equal(samePlayhead(baseView({ position: NaN }), baseView({ position: NaN })), true);
  assert.equal(samePlayhead(baseView({ playing: true }), baseView({ playing: true })), false);
  assert.equal(samePlayhead(baseView({ playing: true }), baseView({ playing: false })), false);
});

test("Bridge.set isolates playback ticks and paused seeks from bridge.view", () => {
  const local = new Bridge();
  const playing1 = baseView({ playing: true, position: 1000 });
  const playing2 = baseView({ playing: true, position: 1085, musicalPosition: "1.2.0 · 180.0 BPM · 3/4 · 1/4" });
  local.set(playing1, map);
  assert.equal(local.view, playing1);
  assert.equal(local.playhead, playing1);
  assert.equal(local.latestView, playing1);
  assert.equal(local.scene, map);

  local.set(playing2, map);
  assert.equal(local.view, playing1, "playback tick keeps bridge.view reference identical");
  assert.equal(local.scene, map);
  assert.equal(local.playhead, playing2);
  assert.equal(local.latestView, playing2);

  const paused1 = baseView({ playing: false, position: 2000 });
  local.set(paused1, map);
  assert.equal(local.view, paused1);
  assert.equal(local.playhead, paused1);

  const pausedIdle = baseView({ playing: false, position: 2000 });
  local.set(pausedIdle, map);
  assert.equal(local.view, paused1, "idle paused poll keeps bridge.view");
  assert.equal(local.playhead, paused1, "idle paused poll keeps bridge.playhead");
  assert.equal(local.latestView, pausedIdle);

  const pausedSeek = baseView({ playing: false, position: 5000, musicalPosition: "4.1.0 · 140.0 BPM · 4/4 · 1/4" });
  local.set(pausedSeek, map);
  assert.equal(local.view, paused1, "paused seek leaves bridge.view unchanged");
  assert.equal(local.playhead, pausedSeek, "paused seek updates bridge.playhead");

  const uiChanges = [
    { busy: true },
    { fault: "Stopped" },
    { playing: true },
    { cycle: true },
    { duration: 150000 },
    { speed: 1.5 },
    { audition: true },
    { tool: "Draw" },
    { follow: true },
    { returnOnStop: true },
    { panels: { browser: true, inspector: true, controller: false, laneNames: true } },
    { panels: { browser: false, inspector: true, controller: true, laneNames: true } },
    { panels: { browser: true, inspector: false, controller: true, laneNames: true } },
    { panels: { browser: true, inspector: true, controller: true, laneNames: false } },
    { song: 0.5 },
    { main: 0.7 },
    { outputDevice: "Headphones" },
    { songMuted: true },
    { songSolo: true },
    { status: "Saved" },
    { showHome: true },
    { hasDocument: false },
    { objectLines: true },
    { warnings: ["Clip"] },
    { outputProblems: [{ time: 100, message: "Gap", target: "a.osu", timeline: "a.osu" }] },
    { recent: ["b.osu"] },
    { recovery: { error: null, offers: [{ id: "1", created_ms: 10, mapset: "S", difficulty: "Hard" }] } },
  ];
  for (const patch of uiChanges) {
    const baseline = baseView({ playing: false, position: 5000 });
    local.set(baseline, map);
    const changed = baseView({ playing: false, position: 5000, ...patch });
    local.set(changed, map);
    assert.equal(local.view, changed, "UI field change updates bridge.view: " + JSON.stringify(patch));
  }

  // Fresh equivalent arrays on 85 ms playback ticks must not replace bridge.view.
  const tick1 = baseView({ playing: true, warnings: [], outputProblems: [], recent: ["a.osu"], recovery: { error: null, offers: [] } });
  const tick2 = baseView({ playing: true, position: 1085, warnings: [], outputProblems: [], recent: ["a.osu"], recovery: { error: null, offers: [] } });
  local.set(tick1, map);
  local.set(tick2, map);
  assert.equal(local.view, tick1, "fresh equal arrays keep bridge.view reference identical");

  local.set(null, null);
  assert.equal(local.view, null);
  assert.equal(local.playhead, null);
  assert.equal(local.latestView, null);
  assert.equal(local.scene, null);
  assert.equal(local.snapGroup, null);
});

test("TransportBar SSR readout updates across a playhead-only tick while bridge.view stays identical", () => {
  const first = baseView({ playing: true, position: 1000, musicalPosition: "1.1.0 · 120.0 BPM · 4/4 · 1/4" });
  const second = baseView({ playing: true, position: 106052, musicalPosition: "38.2.240 · 190.0 BPM · 7/8 · 1/4" });
  bridge.set(first, map);
  assert.equal(bridge.view, first);
  const html1 = render(TransportBar).body.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");
  assert.ok(html1.includes("00:01.000"));
  assert.ok(html1.includes("120.0 BPM"));
  assert.ok(html1.includes("4/4 TIME"));

  bridge.set(second, map);
  assert.equal(bridge.view, first);
  assert.equal(bridge.playhead, second);
  const html2 = render(TransportBar).body.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");
  assert.ok(html2.includes("01:46.052"));
  assert.ok(html2.includes("190.0 BPM"));
  assert.ok(html2.includes("7/8 TIME"));
});

test("only TransportBar and MapPreview read bridge.playhead across all Svelte parts", () => {
  const partsDir = new URL("../src/parts/", import.meta.url);
  const readers = readdirSync(partsDir)
    .filter(name => name.endsWith(".svelte"))
    .filter(name => readFileSync(new URL(name, partsDir), "utf8").includes("bridge.playhead"))
    .sort();
  assert.deepEqual(readers, ["MapPreview.svelte", "Mixer.svelte", "TransportBar.svelte"]);
  const transportSource = readFileSync(new URL("TransportBar.svelte", partsDir), "utf8");
  assert.match(transportSource, /readout\(\s*bridge\.playhead\s*,\s*scene\s*\)/);
});

test("MIDI Record's pressed state is view.tapRecordingArmed, also right after a toggle answer", () => {
  const record = view => bar(shown({ busy: false, playing: false, cycle: false, ...view })).at(-1);
  for (const armed of [true, false]) {
    assert.match(record({ tapRecordingArmed: armed }), new RegExp(`aria-pressed="${armed}"`));
  }
  assert.match(record({ tapRecordingArmed: true }), /title="Stop MIDI recording and add the notes"/);
  // No local flag can disagree with the view: the toggle answer only decides whether to refresh hits.
  const source = readFileSync(new URL("../src/parts/TransportBar.svelte", import.meta.url), "utf8");
  assert.doesNotMatch(source, /let armed\b|\barmed = /);
  assert.match(source, /const recordArmed = \$derived\(!!view\?\.tapRecordingArmed\);/);
  assert.match(source, /await act\(null, !status\?\.armed && !!status\?\.committedNotes\);/);
});

test("a typed time goes to that moment: the readout's own form, osu!'s timestamp, or minutes and seconds", () => {
  assert.equal(parseTime("01:54.385"), 114385);
  assert.equal(parseTime("01:54:385 (1,2,3) - "), 114385, "a pasted osu! timestamp, combo numbers and all");
  assert.equal(parseTime(" 1:54 "), 114000);
  assert.equal(parseTime("114.5"), 114500, "seconds");
  assert.equal(parseTime("62:05.004"), 3725004);
  for (const bad of ["", "abc", "1:75", "-3", "01:54:38x", null]) assert.equal(parseTime(bad), null, String(bad));
});

test("the readout's time is a button that offers to type a time to go to", () => {
  const html = shown({ busy: false, position: 106052, musicalPosition: "38.2.240 · 190.0 BPM · 7/8 · 1/4" }, map);
  const time = buttons(html).find(tag => /class="time\b/.test(tag));
  assert.ok(time, "the time is a button");
  assert.match(time, /aria-label="Go to time"/);
  assert.match(time, /title="Click and type a time, such as 01:54:385, to go there"/);
  assert.doesNotMatch(time, /\sdisabled/);
  assert.match(buttons(shown(null, null)).find(tag => /class="time\b/.test(tag)), /\sdisabled/, "no map, no seeking");
});

test("Go to time pages the timeline to the new time after seeking", () => {
  const source = readFileSync(new URL("../src/parts/TransportBar.svelte", import.meta.url), "utf8");
  assert.match(source, /act\(\{ kind: "seek", ms \}\);\s*(\/\/[^\n]*\n\s*)?bridge\.callAction\("revealTime", ms\);/);
});
