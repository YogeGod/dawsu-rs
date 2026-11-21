import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { folderShades, laneShades } from "../src/lib/colour.js";
import { folderChip, laneIcons } from "../src/lib/lanes.js";
import LaneHeaders from "../src/parts/LaneHeaders.svelte";

const folder = (more = {}) => ({ type: "folder", key: "folder:soft", set: "soft", name: "Soft set",
  count: 4, used: 2, collapsed: false, muted: false, solo: false, ...more });
const lane = (more = {}) => ({ type: "lane", key: "lane:0", row: 0, number: 5, name: "Clap", label: "Clap", display: "Clap",
  title: "soft-hitclap.wav · Select and audition", tag: "Soft", icon: "clap", colour: null, folder: "soft",
  active: false, selected: false, muted: false, solo: false, quiet: false, layer: false, family: null, addLayer: null, ...more });

function shown(rows, names = true, busy = false) {
  bridge.lanes = { names, busy, rows };
  return render(LaneHeaders).body;
}
const rowsOf = html => html.split(/(?=<div class="lane\b)/).slice(1);
const buttons = html => [...html.matchAll(/<button\b[^>]*>([^]*?)<\/button>/g)].map(match => ({
  tag: match[0].slice(0, match[0].indexOf(">") + 1), text: match[1].replace(/<[^>]+>/g, "").trim(),
}));

// app.js shows no folder without lanes, so there is no "Empty" chip.
test("the folder chip shows the count, or used/count when folded, with the full text in its title", () => {
  assert.deepEqual(folderChip({ count: 4, used: 2, collapsed: false }), { text: "4", title: "4 lanes" });
  assert.deepEqual(folderChip({ count: 4, used: 2, collapsed: true }), { text: "2/4", title: "4 lanes, 2 used" });
  assert.deepEqual(folderChip({ count: 1, used: 0, collapsed: true }), { text: "0/1", title: "1 lane, 0 used" });
});

test("before app.js hands over its lanes the column is empty", () => {
  bridge.lanes = null;
  assert.equal(rowsOf(render(LaneHeaders).body).length, 0);
});

test("a lane header is spine, number, icon tile, M, S, name and set tag, in Tab order", () => {
  const [row] = rowsOf(shown([lane()]));
  assert.match(row, /^<div class="lane\b[^"]*"[^>]*data-lane-row="0"/);
  assert.match(row, /<span class="spine[^>]*><\/span>\s*<span class="num[^>]*>5<\/span>/);
  assert.match(row, /<span class="tile[^>]*aria-hidden="true"[^>]*><svg viewBox="0 0 16 16"/);
  assert.ok(row.includes(laneIcons.clap));
  assert.deepEqual(buttons(row).map(button => button.text), ["M", "S", "Clap"]);
  const [mute, solo, name] = buttons(row).map(button => button.tag);
  assert.match(mute, /class="lane-mute[^"]*"/);
  assert.match(mute, /aria-label="Mute Clap"/);
  // M and S name the lane plainly, without the " · idx 2" lane info.
  const [info] = rowsOf(shown([lane({ label: "soft-hitclap2 · idx 2", display: "soft-hitclap2" })]));
  assert.match(buttons(info)[0].tag, /aria-label="Mute soft-hitclap2"/);
  assert.match(buttons(info)[1].tag, /aria-label="Solo soft-hitclap2"/);
  assert.match(buttons(info)[2].tag, /aria-label="Select and audition soft-hitclap2 · idx 2"/);
  assert.match(mute, /aria-pressed="false"/);
  assert.match(solo, /aria-label="Solo Clap"/);
  assert.match(name, /class="name[^"]*"/);
  assert.match(name, /data-lane-row="0"/);
  assert.match(name, /aria-haspopup="menu"/);
  assert.match(name, /aria-label="Select and audition Clap"/);
  assert.match(name, /title="soft-hitclap.wav · Select and audition"/);
  assert.match(row, /<span class="tag[^>]*>Soft<\/span>/);
});

test("a lane is grey until the mapper picks a colour; the picked colour shows on strip, tile and header", () => {
  const grey = laneShades(null);
  const [plain] = rowsOf(shown([lane()]));
  assert.ok(plain.includes("--lh-strip: " + grey.strip), plain);
  assert.ok(plain.includes("--lh-header: " + grey.header));
  const red = laneShades("#d04932");
  const [picked] = rowsOf(shown([lane({ colour: "#d04932" })]));
  for (const [name, value] of Object.entries(red)) assert.ok(picked.includes("--lh-" + name + ": " + value), name);
  assert.ok(picked.includes("--lh-spine: " + folderShades().strip), "the spine is the folder's grey");
  const [loose] = rowsOf(shown([lane({ folder: null, tag: "File", icon: "file" })]));
  assert.doesNotMatch(loose, /--lh-spine/, "a lane outside a folder has no spine colour");
});

test("state classes: active, selected, quiet (muted or not soloed), numbered layer", () => {
  const classes = row => row.match(/^<div class="([^"]*)"/)[1].split(/\s+/);
  const [state] = rowsOf(shown([lane({ active: true, selected: true, quiet: true, layer: true, muted: true })]));
  for (const name of ["lane", "active", "lane-selected", "quiet", "lane-layer"]) assert.ok(classes(state).includes(name), name);
  assert.match(buttons(state)[0].tag, /aria-pressed="true"/);
  assert.match(buttons(state)[2].tag, /aria-pressed="true"/, "the name shows the lane selection");
});

test("numbered layers get the family chevron; lanes that can take a layer get +", () => {
  const [row] = rowsOf(shown([lane({ family: { collapsed: true, label: "Expand soft-hitclap layers" },
    addLayer: "Add numbered soft-hitclap layer" })], true, true));
  assert.deepEqual(buttons(row).map(button => button.text), ["M", "S", "▸", "Clap", "+"]);
  const chevron = buttons(row)[2].tag;
  assert.match(chevron, /class="lane-disclosure[^"]*"/);
  assert.match(chevron, /aria-expanded="false"/);
  assert.match(chevron, /aria-label="Expand soft-hitclap layers"/);
  const add = buttons(row)[4].tag;
  assert.match(add, /aria-label="Add numbered soft-hitclap layer"/);
  assert.match(add, /\sdisabled/, "Add waits while a job runs");
});

test("with lane names hidden a lane keeps number, M, S and the select button", () => {
  const [row] = rowsOf(shown([lane({ addLayer: "Add numbered soft-hitclap layer" })], false));
  assert.doesNotMatch(row, /class="tile|class="tag/);
  assert.deepEqual(buttons(row).map(button => button.text), ["M", "S", "▶", "+"]);
  assert.match(row, /class="num[^>]*>5</);
});

test("a folder header is the fold strip, M, S, name and count chip", () => {
  const [row] = rowsOf(shown([folder({ collapsed: true, muted: true })]));
  assert.match(row, /^<div class="lane sample-folder\b[^"]*"[^>]*data-sample-set="soft"/);
  assert.match(row, /draggable="true"/);
  const [strip, mute, solo] = buttons(row);
  assert.match(strip.tag, /class="fold[^"]*"/);
  assert.match(strip.tag, /aria-expanded="false"/);
  assert.match(strip.tag, /aria-label="Soft set"/);
  assert.match(strip.tag, /title="[^"]*Alt\+Up or Alt\+Down/);
  assert.match(mute.tag, /aria-label="Mute Soft set"/);
  assert.match(mute.tag, /aria-pressed="true"/);
  assert.match(solo.tag, /aria-label="Solo Soft set"/);
  assert.match(row, /<span class="name[^>]*>Soft set<\/span>/);
  assert.match(row, /<span class="chip[^"]*" title="4 lanes, 2 used">2\/4<\/span>/);
  assert.ok(row.includes("--lh-header: " + folderShades().header));
  // With lane names hidden the 99 px column keeps strip, M and S, as before.
  const [compact] = rowsOf(shown([folder()], false));
  assert.doesNotMatch(compact, /class="chip|class="name/);
  assert.equal(buttons(compact).length, 3);
});

test("an unassigned drum row shows its name only, or a dash with names hidden", () => {
  const drum = { type: "drum", key: "drum:1", name: "Ghost", title: "Ghost · Unassigned" };
  const [row] = rowsOf(shown([drum]));
  assert.match(row, /<span class="name unassigned-drum[^"]*" title="Ghost · Unassigned">Ghost<\/span>/);
  assert.equal(buttons(row).length, 0);
  const [compact] = rowsOf(shown([drum], false));
  assert.match(compact, /title="Ghost · Unassigned">–<\/span>/);
});

// Yoge, 2026-11-21: a main lane folds its numbered layers like a folder, more quietly.
test("a main lane with layers is a family head with a fold arrow", () => {
  const head = more => rowsOf(shown([lane({ name: "Finish", family: { label: "Collapse normal-hitfinish layers", ...more } })]))[0];
  const open = head({ collapsed: false });
  assert.match(open, /^<div class="lane\b[^"]*\bfamily-head\b/);
  assert.match(buttons(open)[2].tag, /class="lane-disclosure[^"]*"[^>]*aria-expanded="true"/);
  assert.equal(buttons(open)[2].text, "▾");
  const folded = head({ collapsed: true, label: "Expand normal-hitfinish layers" });
  assert.match(buttons(folded)[2].tag, /aria-expanded="false"[^>]*aria-label="Expand normal-hitfinish layers"/);
  assert.equal(buttons(folded)[2].text, "▸");
});

test("a layer row is indented under its main lane by a guide", () => {
  const [layer, plain] = rowsOf(shown([lane({ layer: true, name: "Finish 2" }), lane({ key: "lane:1", row: 1 })]));
  assert.match(layer, /<span class="num[^>]*>5<\/span>\s*(<!--[^>]*-->)*<span class="layer-guide[^"]*" aria-hidden="true"><\/span>/);
  assert.doesNotMatch(plain, /layer-guide/);
});

test("− removes the lane: named for screen readers, after +, disabled while busy or for a protected lane", () => {
  const remove = { label: "Remove Finish 3", disabled: false };
  const [row] = rowsOf(shown([lane({ name: "Finish 3", addLayer: "Add numbered normal-hitfinish layer", remove })]));
  assert.deepEqual(buttons(row).map(button => button.text), ["M", "S", "Finish 3", "+", "−"]);
  const minus = buttons(row)[4].tag;
  assert.match(minus, /class="lane-remove[^"]*"/);
  assert.match(minus, /aria-label="Remove Finish 3"/);
  assert.doesNotMatch(minus, /\sdisabled/);
  assert.match(buttons(rowsOf(shown([lane({ remove })], true, true))[0]).at(-1).tag, /\sdisabled/);
  assert.match(buttons(rowsOf(shown([lane({ remove: { ...remove, disabled: true } })]))[0]).at(-1).tag, /\sdisabled/);
  assert.doesNotMatch(rowsOf(shown([lane({ remove: null })]))[0], /lane-remove/);
  // With lane names hidden it stays, beside +.
  assert.deepEqual(buttons(rowsOf(shown([lane({ addLayer: "Add", remove })], false))[0]).map(button => button.text), ["M", "S", "▶", "+", "−"]);
});
