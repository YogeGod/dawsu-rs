import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { autoMatchDrumPreset, formatHostLatency, groupDrumInstruments } from "../src/lib/drumVst3.js";
import DrumInstrumentBar from "../src/parts/DrumInstrumentBar.svelte";

test("groupDrumInstruments keeps zero-plugin built-in kits alongside scanned VST3 plugins", () => {
  const grouped = groupDrumInstruments([
    { uid: "ggd1", name: "Kontakt 7 GGD", vendor: "NI", path: "C:/ggd.vst3", isInstrument: true },
  ]);
  assert.equal(grouped.builtin.length, 2);
  assert.equal(grouped.vst3.length, 1);
  assert.equal(autoMatchDrumPreset("Kontakt 7 GGD"), "GGD Matt Garstka");
  assert.equal(autoMatchDrumPreset("MT Power Drum Kit"), "General MIDI");
  assert.equal(formatHostLatency(512, 48000), "10.7 ms");
});

test("DrumInstrumentBar renders built-in kits, scanned VST3 plugins, and fallback status", () => {
  const html = render(DrumInstrumentBar, {
    props: { open: true, sidecarAvailable: false, plugins: [] },
  }).body;
  assert.match(html, /dawsu! Stock Kit/);
  assert.match(html, /Custom Multi-Sampler/);
  assert.match(html, /Built-in mode/);
});
