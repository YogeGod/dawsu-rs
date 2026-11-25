import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { CHORD_STRATEGIES, formatChordPitches } from "../src/lib/chordResolver.js";
import ChordResolverPopover from "../src/parts/ChordResolverPopover.svelte";

test("chordResolver exposes FlattenPolyphonic and TopNoteOnly only (no storyboard)", () => {
  assert.deepEqual(CHORD_STRATEGIES.map(s => s.id), ["flatten_polyphonic", "top_note_only"]);
  assert.equal(formatChordPitches([67, 60, 64]), "C4 + E4 + G4");
});

test("ChordResolverPopover renders strategies and explains why storyboard samples are not used", () => {
  const html = render(ChordResolverPopover, {
    props: {
      open: true,
      conflict: { pitches: [60, 64, 67] },
      totalConflicts: 12,
    },
  }).body;
  assert.match(html, /Chord: C4 \+ E4 \+ G4/);
  assert.match(html, /Flatten Chord to 1 Polyphonic Sample/);
  assert.match(html, /Top Note Only/);
  assert.match(html, /Apply to All 12 Chords/);
});
