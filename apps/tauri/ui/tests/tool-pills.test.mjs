import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { auditionAction, toolAction, toolPills, TOOLS } from "../src/lib/tools.js";
import AuditionPill from "../src/parts/AuditionPill.svelte";
import ToolPills from "../src/parts/ToolPills.svelte";

test("the toolbar shows Select, Draw and Erase, plus the current tool when it lives off the toolbar", () => {
  assert.deepEqual(TOOLS, ["Select", "Draw", "Erase", "Mute", "Grab", "Range", "Ramp"]);
  for (const tool of ["Select", "Draw", "Erase", null, undefined, "Unknown"]) {
    assert.deepEqual(toolPills(tool), ["Select", "Draw", "Erase"], String(tool));
  }
  for (const tool of ["Mute", "Grab", "Range", "Ramp"]) assert.deepEqual(toolPills(tool), ["Select", "Draw", "Erase", tool]);
});

test("pills send the editor's tool and audition actions", () => {
  assert.deepEqual(toolAction("Erase"), { kind: "tool", name: "Erase" });
  assert.deepEqual(auditionAction({ audition: true }), { kind: "audition", on: false });
  assert.deepEqual(auditionAction({ audition: false }), { kind: "audition", on: true });
});

function shown(part, view, scene = { path: "Hard.osu" }) {
  bridge.set(view, scene);
  return render(part).body;
}
const pressed = html => [...html.matchAll(/<button[^>]*aria-pressed="true"[^>]*>[^]*?<\/button>/g)]
  .map(match => match[0].replace(/<[^>]+>/g, "").trim());
const labels = html => [...html.matchAll(/<button[^>]*>([^]*?)<\/button>/g)].map(match => match[1].replace(/<[^>]+>/g, "").trim());

test("the current tool's pill is pressed; an off-toolbar tool gets a fourth pressed pill that opens the picker", () => {
  let html = shown(ToolPills, { tool: "Draw", busy: false });
  assert.deepEqual(labels(html), ["Select", "Draw", "Erase"]);
  assert.deepEqual(pressed(html), ["Draw"]);
  assert.doesNotMatch(html, /disabled/);

  html = shown(ToolPills, { tool: "Ramp", busy: false });
  // The fourth pill is icon only, so the toolbar still fits; its name is in the tooltip and aria-label.
  assert.deepEqual(labels(html), ["Select", "Draw", "Erase", ""]);
  const fourth = html.match(/<button[^>]*aria-haspopup="menu"[^>]*>/)?.[0] ?? "";
  assert.match(fourth, /aria-pressed="true"/);
  assert.match(fourth, /aria-label="Ramp"/);
  assert.match(fourth, /title="Ramp"/);
  assert.equal([...html.matchAll(/aria-haspopup/g)].length, 1, "only the off-toolbar pill opens the picker");
  assert.equal([...html.matchAll(/<svg/g)].length, 4, "every pill has its icon");
  assert.doesNotMatch(html, /\sstyle=|\sid=/);
});

test("before the first update, with no map, or while busy, the pills show but can't be pressed", () => {
  for (const [view, scene] of [[null, null], [{ tool: "Select", busy: false }, null], [{ tool: "Select", busy: true }, { path: "Hard.osu" }]]) {
    const html = shown(ToolPills, view, scene);
    assert.deepEqual(labels(html), ["Select", "Draw", "Erase"]);
    assert.equal([...html.matchAll(/disabled/g)].length, 3);
    const audition = shown(AuditionPill, view, scene);
    assert.match(audition, /disabled/);
  }
  assert.deepEqual(pressed(shown(ToolPills, null, null)), []);
  assert.match(shown(AuditionPill, null, null), /aria-pressed="false"/);
});

test("the Audition pill follows the editor's audition setting", () => {
  let html = shown(AuditionPill, { audition: true, busy: false });
  assert.deepEqual(labels(html), ["Audition"]);
  assert.match(html, /aria-pressed="true"/);
  assert.doesNotMatch(html, /disabled|\sstyle=/);
  html = shown(AuditionPill, { audition: false, busy: false });
  assert.match(html, /aria-pressed="false"/);
});
