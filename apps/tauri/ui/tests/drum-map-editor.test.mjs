import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { formatChokeGroup, formatMidiNoteLabel, groupDrumPresets } from "../src/lib/drumMapEditor.js";
import DrumMapEditorModal from "../src/parts/DrumMapEditorModal.svelte";

test("drumMapEditor formats MIDI notes, choke groups, and preset categories", () => {
  assert.equal(formatMidiNoteLabel(36), "C1 (36)");
  assert.equal(formatMidiNoteLabel(42), "F#1 (42)");
  assert.equal(formatMidiNoteLabel(null), "Unassigned");
  assert.equal(formatChokeGroup(1), "A");
  assert.equal(formatChokeGroup(null), "None");
  const grouped = groupDrumPresets([{ name: "Yoge Expanded Kit" }, { name: "GGD Matt Garstka" }]);
  assert.equal(grouped.free.length, 1);
  assert.equal(grouped.pro.length, 1);
});

test("DrumMapEditorModal renders Pad Test, MIDI Learn, and Choke Group selectors", () => {
  const html = render(DrumMapEditorModal, {
    props: {
      open: true,
      presets: [{ name: "Yoge Expanded Kit" }, { name: "GGD Matt Garstka" }],
      rows: [
        { id: "r1", name: "Closed Hat", midiNote: 42, chokeGroup: 1 },
        { id: "r2", name: "Open Hat", midiNote: 46, chokeGroup: 1 },
      ],
    },
  }).body;
  assert.match(html, /Closed Hat/);
  assert.match(html, /F#1 \(42\)/);
  assert.match(html, /▶ Test/);
  assert.match(html, /MIDI Learn/);
});
