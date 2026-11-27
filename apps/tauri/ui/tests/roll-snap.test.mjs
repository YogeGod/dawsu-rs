import assert from "node:assert/strict";
import { test } from "node:test";
import { noteDragPayload } from "../src/lib/midi.js";
import { drawEnd, noteSnap, placeNote, snapNearest } from "../src/lib/rollSnap.js";

// The editor's scene as the roll reads it: 120 BPM from 0, Snap 1/4 (a 125 ms step).
const grid = { snap: true, snapMode: "grid", beats: 1, divisor: 4, sections: [[0, 500]], moments: [] };
const off = { ...grid, snap: false };
// Two hit objects, at 1000 ms and 2150 ms; moments are [time, ..., hasObject].
const objects = { ...grid, snapMode: "objects", moments: [[1000, 0, true], [1800, 0, false], [2150, 0, true]] };

test("a plain click floors to the snap step and is one step long; Snap off keeps the time and lasts a beat", () => {
  // Plan task 5.1: Snap 1/4 at 120 BPM, a click at 190 ms places at 125 ms.
  assert.deepEqual(placeNote(grid, 190), { startMs: 125, lengthMs: 125 });
  assert.deepEqual(placeNote(grid, 250), { startMs: 250, lengthMs: 125 }, "on a step stays on it");
  assert.deepEqual(placeNote(off, 190), { startMs: 190, lengthMs: 500 }, "Snap off: one beat at that tempo");
  assert.deepEqual(placeNote(off, 190.6), { startMs: 191, lengthMs: 500 }, "whole ms");
  // A second red line at 2150 ms with a 400 ms beat: its 1/4 step is 100 ms from 2150.
  const two = { ...grid, sections: [[0, 500], [2150, 400]] };
  assert.deepEqual(placeNote(two, 2330), { startMs: 2250, lengthMs: 100 }, "steps run 2150, 2250, 2350");
  assert.deepEqual(placeNote({ ...off, sections: [[0, 500], [2150, 400]] }, 2330), { startMs: 2330, lengthMs: 400 });
});

test("in Objects snap a placed note starts on the nearest object and lasts a beat", () => {
  assert.deepEqual(placeNote(objects, 1300), { startMs: 1000, lengthMs: 500 });
  assert.deepEqual(placeNote(objects, 1900), { startMs: 2150, lengthMs: 500 }, "1800 has no object");
  assert.deepEqual(placeNote({ ...objects, moments: [] }, 1300), { startMs: 1300, lengthMs: 500 }, "no objects: where clicked");
});

test("move and resize snap to the nearest step, matching Rust's snap_notes_to_grid vector", () => {
  // Plan task 5.1: a drag released at 190 ms lands at 250 ms; Snap off leaves it at 190 ms.
  assert.equal(snapNearest(grid, 190), 250);
  assert.equal(snapNearest(off, 190), 190);
  // keysound_session.rs snap_notes_to_grid_follows_each_tempo_section_and_keeps_lengths.
  const two = { ...grid, sections: [[0, 500], [2150, 400]] };
  assert.equal(snapNearest(two, 1040), 1000);
  assert.equal(snapNearest(two, 2330), 2350);
  assert.equal(snapNearest({ ...objects, sections: two.sections }, 2330), 2150);

  const note = { index: 2, startMs: 1000, endMs: 1125, pitch: 60, velocity: 90 };
  const zoom = 0.2;
  const drag = (scene, mode, dxMs) => noteDragPayload(5, note, mode, dxMs * zoom, 0, undefined, null, zoom, 14, noteSnap(scene));
  assert.deepEqual(drag(grid, "move", -810), { kind: "move_note", trackId: 5, noteIndex: 2, startMs: 250, pitch: 60 });
  assert.deepEqual(drag(off, "move", -810), { kind: "move_note", trackId: 5, noteIndex: 2, startMs: 190, pitch: 60 });
  assert.deepEqual(drag(grid, "resize", 140), { kind: "resize_note", trackId: 5, noteIndex: 2, endMs: 1250 });
  assert.equal(drag(grid, "resize", -500), null, "shorter than a step keeps one step: no change");
  assert.deepEqual(drag(off, "resize", -500), { kind: "resize_note", trackId: 5, noteIndex: 2, endMs: 1020 },
    "Snap off: down to Rust's 20 ms floor");
  assert.equal(drag(grid, "move", 20), null, "still on the same step: nothing sent");
});

test("a Draw drag stretches the note to the snapped release point, never under its click length", () => {
  const placed = placeNote(grid, 625);
  assert.equal(drawEnd(grid, placed, 1110), 1125);
  assert.equal(drawEnd(grid, placed, 640), 750, "a short drag keeps one step");
  assert.equal(drawEnd(grid, placed, 300), 750, "a drag to the left keeps one step");
  const free = placeNote(off, 625);
  assert.equal(drawEnd(off, free, 700, true), 700, "Snap off: a drag sets any length");
  assert.equal(drawEnd(off, free, 630, true), 645, "…down to 20 ms");
  assert.equal(drawEnd(off, free, 700, false), 1125, "no drag: the click length");
});

test("Draw and Erase keep their meaning in the roll; every other tool acts as Select", async () => {
  const { rollTool } = await import("../src/lib/rollPaint.js");
  const { keysoundsDeleteNotes } = await import("../src/lib/midi.js");
  assert.equal(rollTool("Draw"), "draw");
  assert.equal(rollTool("Erase"), "erase");
  for (const tool of ["Select", "Mute", "Grab", "Range", "Ramp", undefined]) assert.equal(rollTool(tool), "select");
  assert.deepEqual(keysoundsDeleteNotes(4, [3, 1]), { kind: "delete_notes", trackId: 4, noteIndices: [3, 1] });
});

test("after an Erase the selection follows its note to its new index, or clears when it was erased", async () => {
  const { selectionAfterDelete } = await import("../src/lib/keysounds.js");
  // Notes 0-4, note 3 selected; erasing 0 and 1 moves it to index 1, so Delete still deletes it.
  assert.equal(selectionAfterDelete(3, [1, 0]), 1);
  assert.equal(selectionAfterDelete(3, [4]), 3, "a later note leaves the index alone");
  assert.equal(selectionAfterDelete(3, [3, 0]), null, "the selected note was erased");
  assert.equal(selectionAfterDelete(null, [0]), null);
});

test("the empty-track hint names the key bound to Draw in the current preset", async () => {
  const { emptyRollHint } = await import("../src/lib/keysounds.js");
  const { buildToolKeyMap, toolKeyFor } = await import("../src/lib/quasimode.js");
  assert.equal(toolKeyFor(buildToolKeyMap(null, "DAW"), "Draw"), "3");
  assert.equal(toolKeyFor(buildToolKeyMap(null, "osu!"), "Draw"), "2");
  assert.equal(toolKeyFor({}, "Draw"), null);
  assert.equal(emptyRollHint("Lead", "2"), "No notes on Lead yet. Double-click in the roll, or pick Draw (2), to place one.");
  assert.equal(emptyRollHint("", null), "No notes on this track yet. Double-click in the roll, or pick Draw, to place one.");
});

test("the hint names the Draw key only when the Keysounds key gate lets it through (plain 1-4)", async () => {
  const { emptyRollHint } = await import("../src/lib/keysounds.js");
  assert.equal(emptyRollHint("Lead", "3"), "No notes on Lead yet. Double-click in the roll, or pick Draw (3), to place one.");
  assert.equal(emptyRollHint("Lead", "d"), "No notes on Lead yet. Double-click in the roll, or pick Draw, to place one.",
    "a letter bound to Draw is blocked in Keysounds mode, so the hint does not offer it");
});
