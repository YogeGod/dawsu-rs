import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  dbText, faderKey, faderPosition, faderReach, faderSender, gainsAction, positionGain, releaseFader, songColour,
  stepGain, stripClickSelects, stripShades,
} from "../src/lib/mixer.js";
import { laneGrey, mixIntoCanvas } from "../src/lib/colour.js";
import BottomPanel from "../src/parts/BottomPanel.svelte";
import Mixer from "../src/parts/Mixer.svelte";

const text = html => html.replace(/<!--[^]*?-->/g, "").replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
const tags = (html, name) => [...html.matchAll(new RegExp(`<${name}\\b[^>]*>`, "g"))].map(match => match[0]);
const label = tag => tag.match(/aria-label="([^"]+)"/)?.[1];
const pressed = tag => /aria-pressed="true"/.test(tag);
const disabled = tag => /\sdisabled/.test(tag);
const buttonsOf = html => [...html.matchAll(/(<button\b[^>]*>)([^]*?)<\/button>/g)]
  .map(([, tag, inner]) => [text(inner), pressed(tag), disabled(tag)]);

const view = (more = {}) => ({ busy: false, song: 0.7, main: 0.9, songMuted: false, songSolo: false,
  outputDevice: "Speakers (Realtek)", panels: { controller: true }, ...more });

function panel(v, tab = "volume", scene = { path: "a.osu" }) {
  bridge.set(v, scene);
  bridge.bottomTab = tab;
  return render(BottomPanel).body;
}

// Gains multiply the samples, so a gain's level in dB is 20·log10(gain).
test("the Song and Master readouts are the real gain in dB, to two decimals", () => {
  assert.equal(dbText(1), "0.00");
  assert.equal(dbText(0.99999), "0.00", "never −0.00");
  assert.equal(dbText(0.7), "−3.10");
  assert.equal(dbText(0.2), "−13.98");
  assert.equal(dbText(0), "−∞");
});

test("the fader follows v3's dB scale and stops at the editor's limits", () => {
  assert.equal(faderPosition(1), 0.14);
  assert.equal(faderPosition(0), 1);
  assert.ok(Math.abs(faderPosition(10 ** (-10 / 20)) - 0.4) < 1e-9);
  assert.ok(Math.abs(positionGain(0.4, 0, 1) - 10 ** (-10 / 20)) < 1e-9);
  // Song gain is 0 to 1 and master gain 0.2 to 1 (main.rs), so the top 6 dB is never reached.
  assert.equal(positionGain(0, 0, 1), 1);
  assert.equal(positionGain(1, 0, 1), 0);
  assert.equal(positionGain(1, 0.2, 1), 0.2);
  assert.ok(Math.abs(stepGain(0.5, 1, 0, 1) - 0.5 * 10 ** (1 / 20)) < 1e-9);
  assert.equal(stepGain(0.99, 1, 0, 1), 1);
  assert.equal(stepGain(0.001, -1, 0, 1), 0, "below -60 dB the song fader reaches silence");
});

test("the part of the travel a gain can't reach is marked: above 0 dB, and below -14 dB for the master", () => {
  assert.deepEqual(faderReach(0, 1), { top: 0.14, bottom: 1 });
  const master = faderReach(0.2, 1);
  assert.equal(master.top, 0.14);
  assert.ok(Math.abs(master.bottom - faderPosition(0.2)) < 1e-12 && master.bottom < 0.5);
});

test("arrow, Page, Home and End move a fader; other keys and modifiers go to the editor", () => {
  const key = (name, more = {}) => faderKey({ key: name, ...more }, 0.5, 0.2, 1);
  assert.equal(key("ArrowUp"), stepGain(0.5, 1, 0.2, 1));
  assert.equal(key("ArrowLeft"), stepGain(0.5, -1, 0.2, 1));
  assert.equal(key("PageDown"), stepGain(0.5, -6, 0.2, 1));
  assert.equal(key("Home"), 0.2);
  assert.equal(key("End"), 1);
  assert.equal(key("ArrowUp", { ctrlKey: true }), null);
  assert.equal(key("a"), null);
});

test("a fader sends one gains action at a time, in order, and the newest value wins", async () => {
  const sent = [];
  const waits = [];
  let drained = 0;
  const send = faderSender(value => new Promise(resolve => { sent.push(value); waits.push(resolve); }), () => drained++);
  send(0.1);
  send(0.2);
  send(0.3);
  assert.deepEqual(sent, [0.1]);
  waits.shift()();
  await new Promise(resolve => setTimeout(resolve));
  assert.deepEqual(sent, [0.1, 0.3], "0.2 was replaced before it went out");
  assert.equal(drained, 0);
  waits.shift()();
  await new Promise(resolve => setTimeout(resolve));
  assert.equal(drained, 1);
});

test("after a mouse drag the fader gives the keys back to the editor", () => {
  let blurred = 0;
  releaseFader({ blur: () => blurred++ });
  assert.equal(blurred, 1);
});

test("a fader sends both gains: its own value and the other fader's last one", () => {
  assert.deepEqual(gainsAction(view(), "song", 0.5), { kind: "gains", song: 0.5, main: 0.9 });
  assert.deepEqual(gainsAction(view(), "main", 0.4), { kind: "gains", song: 0.7, main: 0.4 });
  // The master was just moved and the editor has not answered yet: the song fader keeps it.
  assert.deepEqual(gainsAction(view(), "song", 0.5, { main: 0.3 }), { kind: "gains", song: 0.5, main: 0.3 });
});

test("a strip is grey until the mapper picks a lane colour; the Song strip is teal", () => {
  const grey = stripShades(null);
  assert.equal(grey.colour, laneGrey);
  assert.equal(grey.fill, mixIntoCanvas(laneGrey, 20, "#181818"));
  assert.equal(grey.selected, mixIntoCanvas(laneGrey, 42, "#181818"));
  assert.equal(grey.edge, mixIntoCanvas(laneGrey, 30));
  assert.equal(stripShades("#D05050").colour, "#d05050");
  const tokens = readFileSync(new URL("../src/tokens.css", import.meta.url), "utf8");
  assert.equal(songColour, tokens.match(/--dark-teal:\s*(#[\da-f]{6})/i)[1]);
});

test("a click on a strip's greyed-out or own controls never selects its lane", () => {
  const at = selector => ({ closest: query => query.split(",").map(part => part.trim()).includes(selector) ? {} : null });
  for (const inside of ["button", ".fader", ".box"]) assert.equal(stripClickSelects(at(inside)), false, inside);
  assert.equal(stripClickSelects(at(".label")), true);
});

test("folded, the panel is one strip that opens it; no V key, since V jumps to the last hit", () => {
  const html = panel(view({ panels: { controller: false } }));
  const [button, ...rest] = tags(html, "button");
  assert.equal(rest.length, 0);
  assert.equal(label(button), "Volume and Mixer");
  assert.match(button, /aria-expanded="false"/);
  assert.equal(text(html), "Volume Mixer Drag up to open");
  assert.doesNotMatch(html, /press V/);
  assert.equal(disabled(tags(panel(view({ panels: { controller: false } }), "volume", null), "button")[0]), true);
});

test("open, the panel shows only its Volume and Mixer tabs, with no buttons that can never be pressed", () => {
  const html = panel(view());
  assert.deepEqual(buttonsOf(html), [["Volume", true, false], ["Mixer", false, false]]);
  const mixer = panel(view(), "mixer");
  assert.deepEqual(buttonsOf(mixer), [["Volume", false, false], ["Mixer", true, false]]);
  assert.doesNotMatch(html + mixer, /Volume curve|Reset levels|Per object/);
  assert.doesNotMatch(html + mixer, /\sstyle=|\sid=/);
});

test("the tab and mode labels shorten with an ellipsis instead of overlapping", () => {
  const source = readFileSync(new URL("../src/parts/BottomPanel.svelte", import.meta.url), "utf8");
  const rule = source.match(/\.seg button \{([^}]*)\}/)[1];
  assert.match(rule, /overflow: hidden;/);
  assert.match(rule, /text-overflow: ellipsis;/);
});

const strip = (more = {}) => ({ key: "lane:0", row: 0, number: 1, name: "Normal", set: "Soft", icon: "normal",
  colour: null, selected: false, muted: false, solo: false, quiet: false, ...more });

function mixer(v = view(), strips = [strip(), strip({ key: "lane:3", row: 3, number: 4, name: "Clap", icon: "clap", muted: true, colour: "#d05050" })]) {
  bridge.set(v, { path: "a.osu" });
  bridge.mixer = strips;
  return render(Mixer).body;
}
const masterAt = html => html.search(/<section class="master\b/);
const strips = html => html.slice(0, masterAt(html)).split(/(?=<div class="strip\b)/).slice(1);

test("the Mixer is the Song bus, Hitsounds bus, one strip per used lane, then the master output", () => {
  const html = mixer();
  const [song, hitsounds, normal, clap] = strips(html);
  assert.equal(strips(html).length, 4);
  assert.match(song, /aria-label="Song"/);
  assert.match(song, new RegExp("--strip-colour: " + songColour));
  assert.match(hitsounds, /aria-label="Hitsounds"/);
  assert.match(normal, /aria-label="Normal, lane 1"/);
  assert.match(clap, /aria-label="Clap, lane 4"/);
  // v3 8b footer: the lane icon, the set name and the lane number; the name bar shows the sound.
  assert.match(text(normal), /Soft 1 Normal$/);
  assert.match(text(clap), /Soft 4 Clap$/);
  assert.match(html, /<section class="master[^"]*" aria-label="Master output">/);
  assert.match(text(html.slice(masterAt(html))), /^Master output Speakers \(Realtek\)/);
});

test("M and S work on every strip and name the lane's set", () => {
  const html = mixer(view({ songMuted: true }));
  const [song, hitsounds, normal, clap] = strips(html).map(part => tags(part, "button").filter(tag => /Mute|Solo/.test(label(tag) ?? "")));
  assert.deepEqual(song.map(tag => [label(tag), pressed(tag), disabled(tag)]), [["Mute song", true, false], ["Solo song", false, false]]);
  assert.deepEqual(hitsounds.map(tag => [label(tag), pressed(tag)]), [["Mute all hitsounds", false], ["Solo all hitsounds", false]]);
  assert.deepEqual(normal.map(tag => [label(tag), pressed(tag)]), [["Mute Normal (Soft)", false], ["Solo Normal (Soft)", false]]);
  assert.deepEqual(clap.map(tag => [label(tag), pressed(tag)]), [["Mute Clap (Soft)", true], ["Solo Clap (Soft)", false]]);
});

test("Song, Hitsounds bus, every lane strip, and Master faders work and show dB levels", () => {
  const html = mixer();
  const sliders = tags(html, "div").filter(tag => /role="slider"/.test(tag));
  assert.deepEqual(sliders.map(label), ["Song volume", "Hitsounds bus volume", "Normal volume", "Clap volume", "Master volume"]);
  assert.match(sliders[0], /aria-valuemin="0" aria-valuemax="100" aria-valuenow="70" aria-valuetext="−3.10 dB"/);
  assert.match(sliders[4], /aria-valuemin="20" aria-valuemax="100" aria-valuenow="90" aria-valuetext="−0.92 dB"/);
  const [song, hitsounds, normal] = strips(html);
  assert.equal((song.match(/class="cap/g) ?? []).length, 1);
  assert.equal((hitsounds.match(/class="cap/g) ?? []).length, 1);
  assert.equal((normal.match(/class="cap/g) ?? []).length, 1);
  assert.equal((song.match(/class="reach top/g) ?? []).length, 1);
  assert.equal((song.match(/class="reach bottom/g) ?? []).length, 0, "the song reaches silence");
  const master = html.slice(masterAt(html));
  assert.equal((master.match(/class="reach (top|bottom)/g) ?? []).length, 2);
  // Pan is greyed out, empty and not a control; the meters are empty.
  for (const part of strips(html)) {
    assert.match(part, /<div class="box pan off[^"]*" aria-hidden="true"><\/div>/);
    assert.equal(tags(part, "button").filter(tag => label(tag) === "Pan").length, 0);
    assert.doesNotMatch(part, /class="meter[^"]*"[^>]*>\s*<div\b[^>]*class="fill/);
  }
  const boxes = [...html.matchAll(/<span class="box (level|peak)([^"]*)"[^>]*>([^<]*)<\/span>/g)]
    .map(([, kind, classes, inside]) => [kind, /\boff\b/.test(classes), inside]);
  assert.deepEqual(boxes, [
    ["level", false, "−3.10"], ["peak", true, ""],
    ["level", false, "0.00"], ["peak", true, ""],
    ["level", false, "0.00"], ["peak", true, ""],
    ["level", false, "0.00"], ["peak", true, ""],
    ["level", false, "−0.92"], ["peak", true, ""],
  ]);
  // The master's peak bars are there, greyed out when no peaks are present yet.
  assert.match(master, /<div class="peak-bars off[^"]*" aria-hidden="true">[\s\S]*?<span class="peak-fill/);
  assert.doesNotMatch(text(html), /dBFS|kHz|\bms\b|Default device/);
  // style: directives render as attributes only on the server; ui-bundle.test.mjs checks the sources.
  assert.doesNotMatch(html, /\sid=/);
});

test("before a map is open, or while a job runs, the Mixer takes no input", () => {
  for (const html of [mixer(view({ busy: true })), (bridge.set(null, null), bridge.mixer = null, render(Mixer).body)]) {
    for (const tag of tags(html, "button")) assert.equal(disabled(tag), true, tag);
    for (const tag of tags(html, "div").filter(tag => /role="slider"/.test(tag))) assert.match(tag, /aria-disabled="true"/);
  }
});
