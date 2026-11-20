import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  osuTimestamp, readout, segmentsDisabled, selectionValue, transportActions,
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
  assert.deepEqual(segmentsDisabled(null, null), { playback: true, jump: true });
  assert.deepEqual(segmentsDisabled({ busy: false }, null), { playback: true, jump: true });
  assert.deepEqual(segmentsDisabled({ busy: false }, map), { playback: false, jump: false });
  assert.deepEqual(segmentsDisabled({ busy: true }, map), { playback: false, jump: true });
  assert.deepEqual(segmentsDisabled({ busy: false, fault: "The editor stopped" }, map), { playback: true, jump: true });
});

function shown(view, scene = map) {
  bridge.set(view, scene);
  return render(TransportBar).body;
}
const buttons = html => [...html.matchAll(/<button\b[^>]*>/g)].map(match => match[0]);
// The transport buttons, before the readout's SELECTION button.
const bar = html => buttons(html.slice(0, html.indexOf('class="readout')));
const label = tag => tag.match(/aria-label="([^"]+)"/)?.[1];

test("the bar is collapse tab, Go to start, Go to end, Cycle, Stop and Play, with no Record and no L or R", () => {
  const html = shown({ playing: false, cycle: true, busy: false, position: 0 });
  assert.deepEqual(bar(html).map(label),
    ["Hide Go to start and Go to end", "Go to start", "Go to end", "Cycle", "Stop", "Play"]);
  assert.doesNotMatch(html, /Record|Locator|>L<|>R</);
  assert.equal([...html.matchAll(/<svg/g)].length, 6, "every button has its icon; no copy icon without a selection");
  assert.doesNotMatch(html, /\sstyle=|\sid=|\(Space\)|\(End\)|\(\/\)/);
});

test("Stop is pressed when stopped, Play while playing, Cycle while cycling", () => {
  const pressed = html => bar(html).filter(tag => /aria-pressed="true"/.test(tag)).map(label);
  assert.deepEqual(pressed(shown({ playing: false, cycle: false, busy: false })), ["Stop"]);
  assert.deepEqual(pressed(shown({ playing: true, cycle: true, busy: false })), ["Cycle", "Play"]);
  assert.match(bar(shown({ playing: true, cycle: false, busy: false })).find(tag => label(tag) === "Play"), /class="[^"]*\bplaying\b/);
});

test("before a map every button but the collapse tab is disabled; a job leaves playback working", () => {
  const disabled = html => bar(html).filter(tag => /\sdisabled/.test(tag)).map(label);
  assert.deepEqual(disabled(shown(null, null)), ["Go to start", "Go to end", "Cycle", "Stop", "Play"]);
  assert.deepEqual(disabled(shown({ busy: true, playing: false, cycle: false })), ["Go to start", "Go to end"]);
  assert.deepEqual(disabled(shown({ busy: false, playing: false, cycle: false })), []);
});

test("the readout shows time, BPM, TIME and SELECTION, and SELECTION copies the timestamp", () => {
  let html = shown({ busy: false, position: 106052, musicalPosition: "38.2.240 · 190.0 BPM · 7/8 · 1/4" },
    { ...map, selection: { count: 2, start: 324324, object: "Circle" } });
  const text = html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");
  for (const part of ["01:46.052", "190.0 BPM", "7/8 TIME", "05:24:324 - SELECTION"]) assert.ok(text.includes(part), part);
  assert.doesNotMatch(text, /BAR|38\.2\.240/);
  const copy = buttons(html).find(tag => /title="Copy/.test(tag));
  assert.ok(copy, "SELECTION is a button");
  assert.doesNotMatch(copy, /\sdisabled/);

  html = shown({ busy: false, position: 0 });
  assert.match(html.replace(/<[^>]+>/g, " "), /No selection/);
  assert.match(buttons(html).find(tag => /title="Copy/.test(tag)), /\sdisabled/, "nothing to copy");
  html = shown(null, null);
  assert.match(html.replace(/<[^>]+>/g, " "), /00:00\.000[^]*–[^]*–[^]*No selection/);
});
