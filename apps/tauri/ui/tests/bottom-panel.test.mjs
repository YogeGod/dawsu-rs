import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import { faderPosition, gainsAction, positionGain, stepGain, stripShades } from "../src/lib/mixer.js";
import { laneGrey, mixIntoCanvas } from "../src/lib/colour.js";
import BottomPanel from "../src/parts/BottomPanel.svelte";
import Mixer from "../src/parts/Mixer.svelte";

const text = html => html.replace(/<!--[^]*?-->/g, "").replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
const tags = (html, name) => [...html.matchAll(new RegExp(`<${name}\\b[^>]*>`, "g"))].map(match => match[0]);
const label = tag => tag.match(/aria-label="([^"]+)"/)?.[1];
const pressed = tag => /aria-pressed="true"/.test(tag);
const disabled = tag => /\sdisabled/.test(tag);

const view = (more = {}) => ({ busy: false, song: 0.7, main: 0.9, songMuted: false, songSolo: false,
  outputDevice: "Speakers (Realtek)", panels: { controller: true }, ...more });

function panel(v, tab = "volume", scene = { path: "a.osu" }) {
  bridge.set(v, scene);
  bridge.bottomTab = tab;
  return render(BottomPanel).body;
}

// Gains multiply the samples, so a gain sits on the dB scale at 20·log10(gain).
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

test("a fader sends both gains, the other one as the editor shows it", () => {
  assert.deepEqual(gainsAction(view(), "song", 0.5), { kind: "gains", song: 0.5, main: 0.9 });
  assert.deepEqual(gainsAction(view(), "main", 0.4), { kind: "gains", song: 0.7, main: 0.4 });
});

test("a strip is grey until the mapper picks a lane colour", () => {
  const grey = stripShades(null);
  assert.equal(grey.colour, laneGrey);
  assert.equal(grey.fill, mixIntoCanvas(laneGrey, 20, "#181818"));
  assert.equal(grey.selected, mixIntoCanvas(laneGrey, 42, "#181818"));
  assert.equal(grey.edge, mixIntoCanvas(laneGrey, 30));
  assert.equal(stripShades("#D05050").colour, "#d05050");
});

test("folded, the panel is one strip that opens it; no V key, since V jumps to the last hit", () => {
  const html = panel(view({ panels: { controller: false } }));
  const [button, ...rest] = tags(html, "button");
  assert.equal(rest.length, 0);
  assert.equal(label(button), "Open Volume and Mixer");
  assert.equal(text(html), "Volume Mixer Folded · drag up to open");
  assert.doesNotMatch(html, /press V/);
  assert.equal(disabled(tags(panel(view({ panels: { controller: false } }), "volume", null), "button")[0]), true);
});

test("open, the Volume tab offers Per object; Volume curve is greyed out", () => {
  const html = panel(view());
  const shownButtons = [...html.matchAll(/(<button\b[^>]*>)([^]*?)<\/button>/g)].map(([, tag, inner]) => [text(inner), pressed(tag), disabled(tag)]);
  assert.deepEqual(shownButtons,
    [["Volume", true, false], ["Mixer", false, false], ["Per object", true, false], ["Volume curve", false, true]]);
  assert.match(text(html), /^Volume Mixer Per object Volume curve$/);
  const mixer = panel(view(), "mixer");
  assert.deepEqual(tags(mixer, "button").map(pressed), [false, true]);
  assert.equal(text(mixer), "Volume Mixer");
  assert.doesNotMatch(html + mixer, /\sstyle=|\sid=/);
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

test("the Mixer is the Song strip, one strip per used lane, then the master output", () => {
  const html = mixer();
  const [song, normal, clap] = strips(html);
  assert.equal(strips(html).length, 3);
  assert.match(song, /aria-label="Song"/);
  assert.match(normal, /aria-label="Normal, lane 1"/);
  assert.match(clap, /aria-label="Clap, lane 4"/);
  // v3 8b footer: the lane icon, the set name and the lane number; the name bar shows the sound.
  assert.match(text(normal), /Soft 1 Normal$/);
  assert.match(text(clap), /Soft 4 Clap$/);
  assert.match(html, /<section class="master[^"]*" aria-label="Master output">/);
  assert.match(text(html.slice(masterAt(html))), /^Master output Speakers \(Realtek\)/);
});

test("M and S work on every strip and show the editor's state", () => {
  const html = mixer(view({ songMuted: true }));
  const [song, normal, clap] = strips(html).map(part => tags(part, "button").filter(tag => /Mute|Solo/.test(label(tag) ?? "")));
  assert.deepEqual(song.map(tag => [label(tag), pressed(tag), disabled(tag)]), [["Mute song", true, false], ["Solo song", false, false]]);
  assert.deepEqual(normal.map(tag => [label(tag), pressed(tag)]), [["Mute Normal", false], ["Solo Normal", false]]);
  assert.deepEqual(clap.map(tag => [label(tag), pressed(tag)]), [["Mute Clap", true], ["Solo Clap", false]]);
});

test("only the Song and Master faders work; nothing shows a level the editor does not have", () => {
  const html = mixer();
  const sliders = tags(html, "div").filter(tag => /role="slider"/.test(tag));
  assert.deepEqual(sliders.map(label), ["Song level", "Master level"]);
  assert.match(sliders[0], /aria-valuemin="0" aria-valuemax="100" aria-valuenow="70"/);
  assert.match(sliders[1], /aria-valuemin="20" aria-valuemax="100" aria-valuenow="90"/);
  const [song, normal] = strips(html);
  // Lane faders have no cap: there is no per-lane level yet.
  assert.equal((song.match(/class="cap/g) ?? []).length, 1);
  assert.equal((normal.match(/class="cap/g) ?? []).length, 0);
  // Pan and meters are greyed out and empty on every strip.
  for (const part of strips(html)) {
    assert.deepEqual(tags(part, "button").filter(tag => label(tag) === "Pan").map(disabled), [true]);
    assert.doesNotMatch(part, /class="meter[^"]*"[^>]*>\s*<div\b[^>]*class="fill/);
  }
  // No level or peak readout has a backend yet, Song and Master included: all greyed out and empty.
  const boxes = [...html.matchAll(/<span class="box (level|peak)([^"]*)"[^>]*>([^<]*)<\/span>/g)];
  assert.equal(boxes.length, 2 * 3 + 2);
  for (const [box, , classes, inside] of boxes) {
    assert.match(classes, /\boff\b/, box);
    assert.equal(inside, "", box);
  }
  assert.doesNotMatch(text(html), /dB|kHz|\bms\b|Default device|Reset levels/);
  // style: directives render as attributes only on the server; ui-bundle.test.mjs checks the sources.
  assert.doesNotMatch(html, /\sid=/);
});

test("before a map is open, or while a job runs, the Mixer takes no input", () => {
  for (const html of [mixer(view({ busy: true })), (bridge.set(null, null), bridge.mixer = null, render(Mixer).body)]) {
    for (const tag of tags(html, "button")) assert.equal(disabled(tag), true, tag);
    for (const tag of tags(html, "div").filter(tag => /role="slider"/.test(tag))) assert.match(tag, /aria-disabled="true"/);
  }
});
