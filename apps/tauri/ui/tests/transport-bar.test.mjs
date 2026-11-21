import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  clickGate, copyText, osuTimestamp, readout, segmentsDisabled, selectionValue, transportActions,
} from "../src/lib/transport.js";
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

test("the bar is collapse tab, Go to start, Go to end, Cycle, Stop, Play and a greyed Record, with no L or R", () => {
  const html = shown({ playing: false, cycle: true, busy: false, position: 0, duration: 5000 });
  assert.deepEqual(bar(html).map(label),
    ["Go to start and Go to end", "Go to start", "Go to end", "Cycle", "Stop", "Play", "Record"]);
  // Recording has no code yet (Yoge, 2026-11-21): Record shows greyed out and does nothing.
  const record = bar(html).at(-1);
  assert.match(record, /\sdisabled/);
  assert.doesNotMatch(record, /aria-pressed/);
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
  assert.deepEqual(disabled(shown(null, null)), ["Go to start", "Go to end", "Cycle", "Stop", "Play", "Record"]);
  assert.deepEqual(disabled(shown({ busy: true, playing: false, cycle: false, duration: 5000 })), ["Go to start", "Go to end", "Record"]);
  assert.deepEqual(disabled(shown({ busy: false, playing: false, cycle: false, duration: 5000 })), ["Record"]);
  assert.deepEqual(disabled(shown({ busy: false, playing: false, cycle: false })), ["Go to end", "Record"]);
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
