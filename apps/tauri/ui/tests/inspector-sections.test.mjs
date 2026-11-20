import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { laneGrey, tintName } from "../src/lib/colour.js";
import { inspectorHandlers, laneTint } from "../src/lib/inspector.js";
import InspectorSections from "../src/parts/InspectorSections.svelte";

const model = (more = {}) => ({
  map: true, busy: false, doc: true, selection: null, scope: "all", hint: "Changes all sounds at the selected times.",
  range: false, insertVolume: 100, protect: false, audition: true, hs: false, lane: null, ...more,
});
const selection = (more = {}) => ({
  count: "2 hits", lanes: " · Soft Clap", start: "01:45.263", volume: "60–80%", object: "Circle", sliderBody: null, ...more,
});
const lane = (more = {}) => ({ title: "Soft Clap", sample: "soft-hitclap.wav", colour: null, canColour: true, canReplace: true, ...more });

function shown(inspector) {
  bridge.inspector = inspector;
  return render(InspectorSections).body;
}
const buttons = html => [...html.matchAll(/<button\b[^>]*>([^]*?)<\/button>/g)].map(match => ({
  tag: match[0].slice(0, match[0].indexOf(">") + 1), text: match[1].replace(/<[^>]+>/g, "").trim(),
}));
const button = (html, text) => buttons(html).find(item => item.text === text || item.text.startsWith(text + " "));
const disabled = tag => /\sdisabled/.test(tag);

test("tint names follow v3's hue bands; greys have none", () => {
  const names = ["#d04932", "#d78628", "#deab27", "#c9bc30", "#a6be32", "#44ba4b", "#21af91", "#289dc9",
    "#388ada", "#5776d4", "#945ac5", "#b751bb", "#cc50a2"].map(tintName);
  assert.equal(new Set(names).size > 8, true, names.join());
  assert.equal(tintName("#cc50a2"), "Pink");
  assert.equal(tintName("#388ada"), "Blue");
  assert.equal(tintName("#23b76c"), "Green");
  assert.equal(tintName("#44ba4b"), "Lime");
  assert.equal(tintName(laneGrey), "Grey");
  assert.equal(tintName("#808080"), "Grey");
  assert.deepEqual(laneTint(lane()), { swatch: laneGrey, name: "None" });
  assert.deepEqual(laneTint(lane({ colour: "#CC50A2" })), { swatch: "#cc50a2", name: "Pink" });
});

test("before app.js hands over the Inspector the sections are empty and wait", () => {
  const html = shown(null);
  assert.match(html, />Selection</);
  assert.match(html, /Nothing selected/);
  for (const text of ["Generate…", "Merge settings…", "Scale by volume…"]) assert.ok(disabled(button(html, text).tag), text);
});

test("three folding sections in v3's order, each header a button with a chevron", () => {
  const html = shown(model());
  const heads = [...html.matchAll(/<button type="button" class="head[^"]*"[^>]*aria-expanded="true"[^>]*>([^]*?)<\/button>/g)]
    .map(match => match[1].replace(/<[^>]+>/g, "").trim());
  assert.deepEqual(heads, ["Selection", "Lane", "Difficulty"]);
  assert.equal([...html.matchAll(/<svg/g)].length >= 3, true);
  assert.doesNotMatch(html, /\sstyle="/);
  assert.doesNotMatch(html, /data-(action|command|volume|doc|map)\b/);
});

test("Selection shows its rows and every control the old card had", () => {
  const html = shown(model({ selection: selection(), range: true }));
  assert.match(html, /2 hits<span class="dim[^"]*">\s*· Soft Clap<\/span>/);
  assert.match(html, /<input[^>]*aria-label="Start"[^>]*value="01:45.263"/);
  assert.match(html, />Volume<\/span>\s*<output[^>]*>60–80%<\/output>/);
  assert.match(html, />On object<\/span>\s*<output[^>]*>Circle<\/output>/);
  for (const text of ["Move", "Apply", "−5", "+5", "Quantize…", "Repeat…", "Duplicate", "Delete", "Edit range…",
    "Scale by volume…"]) assert.ok(button(html, text), text);
  assert.match(html, /<option value="absolute"[^>]*>Set all<\/option>\s*<option value="relative"[^>]*>Relative<\/option>/);
  assert.match(html, /<option value="all"[^>]*>All lanes<\/option>\s*<option value="selected"[^>]*>Selected lanes<\/option>/);
  assert.match(html, /Changes all sounds at the selected times\./);
  assert.match(html, /aria-label="Insert volume"[^>]*value="100"/);
  for (const label of ["Protect normals", "Audition"]) assert.match(html, new RegExp(label + "[^]*?type=\"checkbox\""));
  assert.match(html, /aria-label="Audition"[^>]*checked/);
  assert.doesNotMatch(html, /Slider body sounds/);
});

test("nothing selected shows the line and keeps Insert volume, Protect normals and Audition", () => {
  const html = shown(model());
  assert.match(html, /Nothing selected/);
  assert.equal(button(html, "Quantize…"), undefined);
  assert.ok(disabled(button(html, "Edit range…").tag), "Edit range… waits for a range");
  assert.ok(!disabled(button(html, "Scale by volume…").tag));
  assert.match(html, /aria-label="Insert volume"/);
});

test("the slider body fields show the selected slider's sounds", () => {
  const body = { key: "k", set: "soft", whistle: true, whistleSet: "auto", volume: 60 };
  const html = shown(model({ selection: selection({ sliderBody: body }) }));
  assert.match(html, /Slider body sounds/);
  assert.match(html, /<option value="soft"[^>]*selected/);
  assert.match(html, /aria-label="Body and head volume"[^>]*value="60"/);
  assert.match(html, /aria-label="Whistle on slide"[^>]*checked/);
  const none = shown(model({ selection: selection({ sliderBody: { ...body, set: "" } }) }));
  assert.match(none, /<select[^>]*aria-label="Slide and tick sample set"[^>]*disabled/);
});

test("a busy editor disables the selection controls; Insert, the switches, New and Copy stay as before", () => {
  const html = shown(model({ busy: true, doc: false, selection: selection(), lane: lane() }));
  for (const text of ["Move", "Apply", "−5", "Quantize…", "Delete", "Scale by volume…", "Generate…",
    "Merge settings…", "Replace sample…"]) assert.ok(disabled(button(html, text).tag), text);
  assert.ok(disabled(buttons(html).find(item => /id="lane-colour"/.test(item.tag)).tag));
  for (const text of ["New…", "Copy…"]) assert.ok(!disabled(button(html, text).tag), text);
  assert.doesNotMatch(html, /aria-label="Insert volume"[^>]*disabled/);
});

test("Lane shows the name, the file and the tint: None in grey until the mapper picks a colour", () => {
  const unpicked = shown(model({ lane: lane() }));
  assert.match(unpicked, /<strong[^>]*>Soft Clap<\/strong>/);
  assert.match(unpicked, />soft-hitclap\.wav</);
  const tint = buttons(unpicked).find(item => /id="lane-colour"/.test(item.tag));
  assert.equal(tint.text, "None");
  assert.match(tint.tag, /aria-label="Lane tint: None"/);
  assert.match(tint.tag, /aria-haspopup="dialog"/);
  assert.match(unpicked, new RegExp(`--swatch: ${laneGrey}`));
  assert.ok(!disabled(button(unpicked, "Replace sample…").tag));

  const picked = shown(model({ lane: lane({ colour: "#cc50a2", canReplace: false }) }));
  assert.equal(buttons(picked).find(item => /id="lane-colour"/.test(item.tag)).text, "Pink");
  assert.match(picked, /--swatch: #cc50a2/);
  assert.ok(disabled(button(picked, "Replace sample…").tag));

  const noLane = shown(model());
  assert.ok(disabled(buttons(noLane).find(item => /id="lane-colour"/.test(item.tag)).tag));
  assert.match(buttons(noLane).find(item => /id="lane-colour"/.test(item.tag)).tag, /aria-label="Lane tint"/);
});

test("Difficulty keeps Hitsounding mode and the four buttons, with the ids the menus click", () => {
  const html = shown(model({ hs: true }));
  assert.match(html, /aria-label="Hitsounding mode"[^>]*checked/);
  for (const [id, text] of [["generate", "Generate…"], ["new", "New…"], ["copy", "Copy…"], ["merge-settings", "Merge settings…"],
    ["replace-sample", "Replace sample…"]]) {
    assert.match(button(html, text).tag, new RegExp(`id="${id}"`), text);
  }
});

test("each control sends the action app.js's inspectorAction runs", () => {
  const sent = [];
  let blurred = 0;
  const on = inspectorHandlers((...args) => { sent.push(args); return args[0] === "moveStart"; });
  const prevent = { preventDefault() { this.prevented = true; } };
  const field = { value: "1330.5", blur() { blurred++; } };
  on.moveStart(prevent, field);
  assert.equal(prevent.prevented, true);
  assert.equal(blurred, 1, "the field lets go once the hits move");
  on.volume({ preventDefault() {} }, "relative", -15);
  on.volume({ preventDefault() {} }, "absolute", null);
  on.nudge(-5);
  on.scope("selected");
  on.command("quantizeDialog");
  on.sliderBody({ preventDefault() {} }, { set: "drum", whistleSet: "normal", whistle: true, volume: 80 });
  on.insert({ currentTarget: { valueAsNumber: 70 } });
  on.toggle("protect", { currentTarget: { checked: true } });
  on.open("laneColour");
  assert.deepEqual(sent, [
    ["moveStart", "1330.5"], ["volume", -15, true], ["volume", NaN, false], ["nudge", -5], ["scope", "selected"],
    ["command", "quantizeDialog"], ["sliderBody", { set: "drum", whistleSet: "normal", whistle: true, volume: 80 }],
    ["insert", 70], ["toggle", "protect", true], ["laneColour"],
  ]);
});

test("folded sections keep their buttons in the page, so the menus can still click them by id", () => {
  const html = shown(model({ lane: lane() }));
  assert.equal([...html.matchAll(/<div hidden=/g)].length, 0, "open sections are not hidden");
  // The source never drops a body when it folds.
  const source = readFileSync(new URL("../src/parts/InspectorSections.svelte", import.meta.url), "utf8");
  assert.doesNotMatch(source, /\{#if open\./);
  assert.equal([...source.matchAll(/<div hidden=\{!open\.(selection|lane|difficulty)\}>/g)].length, 3);
});
