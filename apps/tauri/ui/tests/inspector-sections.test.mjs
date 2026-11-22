import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { flushSync } from "svelte";
import { render } from "svelte/server";
import { Bridge, bridge } from "../src/lib/bridge.svelte.js";
import { laneGrey, tintName } from "../src/lib/colour.js";
import {
  emptyInspector, fieldHold, inspectorHandlers, laneTint, shownValue, sliderBodyFields, startingAmount, syncChecked,
  volumeLabel,
} from "../src/lib/inspector.js";
import InspectorDifficulty from "../src/parts/InspectorDifficulty.svelte";
import InspectorLane from "../src/parts/InspectorLane.svelte";
import InspectorSelection from "../src/parts/InspectorSelection.svelte";

const model = (more = {}) => ({ ...emptyInspector, doc: true, hint: "Changes all sounds at the selected times.", ...more });
const selection = (more = {}) => ({
  summary: "2 hits selected · Clap (Soft)", start: "01:45.263", volume: "60–80%", object: "Circle", sliderBody: null, ...more,
});
const lane = (more = {}) => ({ title: "Clap · Soft set", sample: "soft-hitclap.wav", colour: null, canColour: true,
  canReplace: true, ...more });

function shown(inspector) {
  bridge.inspector = inspector;
  return [InspectorSelection, InspectorLane, InspectorDifficulty].map(part => render(part).body).join("\n");
}
const buttons = html => [...html.matchAll(/<button\b[^>]*>([^]*?)<\/button>/g)].map(match => ({
  tag: match[0].slice(0, match[0].indexOf(">") + 1), text: match[1].replace(/<[^>]+>/g, "").trim(),
}));
const button = (html, text) => buttons(html).find(item => item.text === text);
const byId = (html, id) => buttons(html).find(item => new RegExp(`\\sid="${id}"`).test(item.tag));
const disabled = tag => /\sdisabled/.test(tag);
const source = name => readFileSync(new URL(`../src/parts/${name}.svelte`, import.meta.url), "utf8");

test("tint names follow v3's hue bands; greys have none", () => {
  assert.equal(tintName("#cc50a2"), "Pink");
  assert.equal(tintName("#388ada"), "Blue");
  assert.equal(tintName("#23b76c"), "Green");
  assert.equal(tintName("#44ba4b"), "Lime");
  assert.equal(tintName(laneGrey), "Grey");
  assert.equal(tintName("#808080"), "Grey");
  assert.deepEqual(laneTint(lane()), { swatch: laneGrey, name: "None" });
  assert.deepEqual(laneTint(lane({ colour: "#CC50A2" })), { swatch: "#cc50a2", name: "Pink" });
});

test("the colour dialog opens on the same grey the Inspector shows", async () => {
  const { laneGrey: editorGrey } = await import("../src/editor/helpers.js");
  assert.equal(editorGrey, laneGrey);
});

test("a field being typed in keeps its text, and goes back to the editor's value on blur unless it sent", () => {
  assert.equal(shownValue(null, "01:00.000"), "01:00.000");
  assert.equal(shownValue("00:59.000", "01:00.000"), "00:59.000");
  const hold = fieldHold();
  const field = { value: "typed" };
  hold.release(field, 100);
  assert.equal(field.value, "100", "blur without sending shows the editor's value again");
  field.value = "1330";
  hold.sent();
  hold.release(field, "01:00.000");
  assert.equal(field.value, "1330", "a sent value stays until the editor's update replaces it");
  hold.release(field, "01:00.000");
  assert.equal(field.value, "01:00.000", "the next blur resets again");
});

test("slider body fields seed from the selected body; switches show the editor's value", () => {
  assert.deepEqual(sliderBodyFields(null), { set: "", whistle: false, whistleSet: "auto", volume: 100 });
  assert.deepEqual(sliderBodyFields({ key: "k", set: "soft", whistle: 1, whistleSet: "normal", volume: 60 }),
    { set: "soft", whistle: true, whistleSet: "normal", volume: 60 });
  const box = { checked: true };
  syncChecked(box, false);
  assert.equal(box.checked, false);
  assert.deepEqual([startingAmount("absolute"), startingAmount("relative")], [100, 0]);
  assert.deepEqual([volumeLabel("absolute"), volumeLabel("relative")], ["Set to", "Change by"]);
});

test("the bridge hands the Inspector over at once and parts render before it", () => {
  const local = new Bridge();
  let flushed = 0;
  local.handOverInspector({ busy: false }, () => flushed++);
  assert.deepEqual([local.inspector, flushed], [{ busy: false }, 1]);
  bridge.handOverInspector(null, flushSync);
  const html = shown(null);
  assert.match(html, /Nothing selected/);
  for (const text of ["Generate…", "Merge settings…", "Scale by volume…"]) assert.ok(disabled(button(html, text).tag), text);
});

test("each section is a fold header with a chevron that controls its body; folding keeps the body", () => {
  const html = shown(model({ lane: lane() }));
  const heads = [...html.matchAll(/<button type="button" class="insp-head[^"]*" aria-expanded="true" aria-controls="([\w-]+)"[^>]*>([^]*?)<\/button>/g)]
    .map(match => [match[2].replace(/<[^>]+>/g, "").trim(), match[1]]);
  assert.deepEqual(heads, [["Selection", "inspector-selection-body"], ["Lane", "inspector-lane-body"],
    ["Difficulty", "inspector-difficulty-body"]]);
  for (const [, id] of heads) assert.match(html, new RegExp(`<div id="${id}"`));
  // Only the swatch's custom property, which the browser sets through the CSSOM (allowed by the CSP).
  assert.doesNotMatch(html, /\sstyle="(?!--swatch: #[\da-f]{6};")/);
  assert.doesNotMatch(html, /data-(action|command|volume|doc|map)\b/);
  for (const name of ["InspectorSelection", "InspectorLane", "InspectorDifficulty"]) {
    // Folded bodies stay in the page, so the menus can still click Lane colour, Generate and the rest by id.
    assert.doesNotMatch(source(name), /\{#if open\b/, name);
    assert.match(source(name), /hidden=\{!open\}/, name);
  }
});

test("Selection shows v3's summary and rows, with every control the old card had and visible labels", () => {
  const html = shown(model({ selection: selection(), range: true, audition: true }));
  assert.match(html, />2 hits selected · Clap \(Soft\)</);
  assert.match(html, /<label for="inspector-start"[^>]*>Start<\/label>\s*<input id="inspector-start"[^>]*value="01:45.263"/);
  assert.match(html, />Volume<\/span>\s*<output[^>]*>60–80%<\/output>/);
  assert.match(html, />On object<\/span>\s*<output[^>]*>Circle<\/output>/);
  for (const [id, text] of [["inspector-change", "Change"], ["inspector-amount", "Set to"], ["inspector-scope", "Applies to"]]) {
    assert.match(html, new RegExp(`<label for="${id}"[^>]*>${text}</label>`), text);
    assert.match(html, new RegExp(`id="${id}"`), id);
  }
  for (const text of ["Move", "Apply", "−5", "+5", "Quantize…", "Repeat…", "Duplicate", "Delete", "Edit range…",
    "Scale by volume…"]) assert.ok(button(html, text), text);
  assert.match(html, /<option value="absolute"[^>]*>Set all<\/option>\s*<option value="relative"[^>]*>Relative<\/option>/);
  assert.match(html, /<option value="all"[^>]*>All lanes<\/option>\s*<option value="selected"[^>]*>Selected lanes<\/option>/);
  assert.match(html, /Changes all sounds at the selected times\./);
  assert.match(html, /aria-label="Insert volume"[^>]*value="100"/);
  assert.match(html, /aria-label="Audition"[^>]*checked/);
  assert.doesNotMatch(html, /Slider body sounds/);
});

test("nothing selected shows the line and keeps Insert volume, Protect normals and Audition", () => {
  const html = shown(model());
  assert.match(html, />Nothing selected</);
  assert.equal(button(html, "Quantize…"), undefined);
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

// Each control's gate, as the old markup had it: [button, model that enables it, change that disables it].
const gates = [
  ["Move", { selection: selection() }, { busy: true }],
  ["Apply", { selection: selection() }, { busy: true }],
  ["−5", { selection: selection() }, { busy: true }],
  ["Quantize…", { selection: selection() }, { busy: true }],
  ["Delete", { selection: selection() }, { busy: true }],
  ["Edit range…", { range: true }, { range: false }],
  ["Edit range…", { range: true }, { busy: true }],
  ["Scale by volume…", {}, { doc: false }],
  ["#lane-colour", { lane: lane() }, { doc: false }],
  ["#lane-colour", { lane: lane() }, { lane: lane({ canColour: false }) }],
  ["#lane-colour", { lane: lane() }, { lane: null }],
  ["Replace sample…", { lane: lane() }, { doc: false }],
  ["Replace sample…", { lane: lane() }, { lane: lane({ canReplace: false }) }],
  ["Generate…", {}, { doc: false }],
  ["Merge settings…", {}, { doc: false }],
];

test("every button waits exactly where the old markup made it wait", () => {
  const find = (html, name) => (name.startsWith("#") ? byId(html, name.slice(1)) : button(html, name));
  for (const [name, on, off] of gates) {
    assert.ok(!disabled(find(shown(model(on)), name).tag), name + " enabled");
    assert.ok(disabled(find(shown(model({ ...on, ...off })), name).tag), name + " disabled by " + JSON.stringify(off));
  }
  // New… and Copy… never waited; Insert volume and the switches neither.
  const busy = shown(model({ busy: true, doc: false }));
  for (const text of ["New…", "Copy…"]) assert.ok(!disabled(button(busy, text).tag), text);
  assert.doesNotMatch(busy, /aria-label="(Insert volume|Protect normals|Audition|Hitsounding mode)"[^>]*disabled/);
});

test("Lane shows v3's title, the file and Lane colour: None in grey until the mapper picks one", () => {
  const unpicked = shown(model({ lane: lane() }));
  assert.match(unpicked, /<strong[^>]*>Clap · Soft set<\/strong>/);
  assert.match(unpicked, />soft-hitclap\.wav</);
  const colour = byId(unpicked, "lane-colour");
  assert.equal(colour.text, "None");
  assert.match(colour.tag, /aria-label="Lane colour: None"/);
  assert.match(colour.tag, /aria-haspopup="dialog"/);
  assert.match(unpicked, />Lane colour<\/span>/);
  assert.doesNotMatch(unpicked, /tint/i);
  assert.match(unpicked, new RegExp(`--swatch: ${laneGrey}`));
  const picked = shown(model({ lane: lane({ colour: "#cc50a2" }) }));
  assert.equal(byId(picked, "lane-colour").text, "Pink");
  assert.match(picked, /--swatch: #cc50a2/);
  assert.match(byId(shown(model()), "lane-colour").tag, /aria-label="Lane colour"/);
});

test("Difficulty keeps the four buttons with the ids the menus click; Hitsounding mode moved to Map and Preferences", () => {
  const html = shown(model({ hs: true }));
  assert.doesNotMatch(html, /aria-label="Hitsounding mode"/);
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
  const hold = fieldHold();
  const field = { value: "1330.5", blur() { blurred++; hold.release(field, "01:00.000"); } };
  assert.equal(on.moveStart(prevent, field, hold), true);
  assert.equal(prevent.prevented, true);
  assert.equal(blurred, 1, "the field lets go once the hits move");
  assert.equal(field.value, "1330.5", "and keeps the sent time until the editor's update");
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
