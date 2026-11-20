import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { playheadActions } from "../src/lib/playhead.js";
import PlayheadGroup from "../src/parts/PlayheadGroup.svelte";

const buttons = html => [...html.matchAll(/<button\b[^>]*>/g)].map(match => match[0]);
const label = tag => tag.match(/aria-label="([^"]+)"/)?.[1];

function shown(view, scene = { path: "Hard.osu" }, centre = false) {
  bridge.set(view, scene);
  bridge.centre = centre;
  return render(PlayheadGroup).body;
}

test("Follow and Return send the editor's own actions; Centre is not a backend setting", () => {
  const send = playheadActions({ follow: true, returnOnStop: false });
  assert.deepEqual(send.follow, { kind: "follow", on: false });
  assert.deepEqual(send.return, { kind: "return_on_stop", on: true });
  assert.equal(send.centre, undefined);
});

test("the group is the Playhead label and the Follow, Centre and Return toggles", () => {
  const html = shown({ follow: true, returnOnStop: false, busy: false });
  assert.deepEqual(buttons(html).map(label), ["Follow", "Centre", "Return"]);
  assert.match(html, />Playhead</);
  assert.equal([...html.matchAll(/<svg/g)].length, 3);
  const pressed = page => buttons(page).filter(tag => /aria-pressed="true"/.test(tag)).map(label);
  assert.deepEqual(pressed(html), ["Follow"]);
  assert.deepEqual(pressed(shown({ follow: true, returnOnStop: true, busy: false }, undefined, true)), ["Follow", "Centre", "Return"]);
  assert.doesNotMatch(html, /\sstyle=|\sid=|data-doc/);
});

test("the toggles wait while there is no map, a job runs or the editor faulted", () => {
  const disabled = page => buttons(page).filter(tag => /\sdisabled/.test(tag)).map(label);
  assert.deepEqual(disabled(shown(null, null)), ["Follow", "Centre", "Return"]);
  assert.deepEqual(disabled(shown({ busy: true })), ["Follow", "Centre", "Return"]);
  assert.deepEqual(disabled(shown({ busy: false, fault: "The editor stopped" })), ["Follow", "Centre", "Return"]);
  assert.deepEqual(disabled(shown({ busy: false, follow: true })), []);
  // Centre only acts while following, so it waits for Follow.
  assert.deepEqual(disabled(shown({ busy: false, follow: false })), ["Centre"]);
});
