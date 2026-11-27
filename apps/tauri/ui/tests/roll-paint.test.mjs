import assert from "node:assert/strict";
import { test } from "node:test";
import {
  hitTest, maxScrollY, noteIndex, noteRect, paintRoll, paintVelocity, pointAt, rollWheelAction, visibleNotes,
  visibleRows,
} from "../src/lib/rollPaint.js";

test("the roll takes only the plain wheel and Ctrl+Shift+wheel; every other turn is the editor's", () => {
  const turn = keys => rollWheelAction({ ctrlKey: false, shiftKey: false, altKey: false, metaKey: false, deltaY: 120, ...keys });
  assert.equal(turn({}), "pitch");
  assert.equal(turn({ ctrlKey: true, shiftKey: true }), "rows");
  // Time zoom or snap step (Ctrl), snap step or time zoom (Alt), pan (Shift), and a sideways turn.
  assert.equal(turn({ ctrlKey: true }), null);
  assert.equal(turn({ altKey: true }), null);
  assert.equal(turn({ shiftKey: true }), null);
  assert.equal(turn({ deltaY: 0, deltaX: 120 }), null);
});

// A 2D context that records every rectangle filled, with the fill style at that moment.
function fakeContext() {
  const calls = [];
  return {
    calls,
    fillStyle: "", strokeStyle: "", globalAlpha: 1, font: "", textBaseline: "",
    save() {}, restore() {}, beginPath() {}, rect() {}, clip() {}, moveTo() {}, lineTo() {}, stroke() {},
    fillRect(x, y, w, h) { calls.push({ kind: "fill", style: this.fillStyle, x, y, w, h }); },
    strokeRect(x, y, w, h) { calls.push({ kind: "stroke", style: this.strokeStyle, x, y, w, h }); },
    fillText(text, x, y) { calls.push({ kind: "text", text, x, y }); },
  };
}

const pitches = Array.from({ length: 49 }, (_, i) => 84 - i); // C6 down to C2
const roll = { top: 0, height: 280, rowHeight: 14, scrollY: 0, pitches };
const note = (index, startMs, endMs, pitch = 72, velocity = 100) => ({ index, startMs, endMs, pitch, velocity, muted: false });
const noteFills = context2d => context2d.calls.filter(call => call.kind === "fill" && (call.style === "#0a84ff" || call.style === "#efc16a"));

test("only the notes inside the visible time window are painted", () => {
  // 10,000 notes one second apart; the window 100.0 s to 104.0 s holds notes 100..104 whole and
  // none of the rest, so with 200 ms notes exactly the starts 100, 101, 102, 103, 104 s show.
  const notes = Array.from({ length: 10000 }, (_, i) => note(i, i * 1000, i * 1000 + 200));
  const seen = visibleNotes(notes, 100000, 104000);
  assert.deepEqual(seen.map(n => n.index), [100, 101, 102, 103, 104]);

  const context2d = fakeContext();
  paintRoll(context2d, 800, { start: 100000, zoom: 0.2 }, roll, { notes, index: noteIndex(notes), selected: null });
  assert.equal(noteFills(context2d).length, 5);
});

test("a paint reads only the notes near the window, not the whole track", () => {
  // Every read of a note's start is counted; the index is found once, before the paints.
  let reads = 0;
  const notes = Array.from({ length: 10000 }, (_, i) => {
    const plain = note(i, i * 1000, i * 1000 + 200);
    return { get startMs() { reads++; return plain.startMs; }, endMs: plain.endMs, pitch: 72, velocity: 100, index: i, muted: false };
  });
  const index = noteIndex(notes);
  assert.deepEqual(index, { longest: 200, sorted: true });
  reads = 0;
  const view = { start: 100000, zoom: 0.2 };
  paintRoll(fakeContext(), 800, view, roll, { notes, index, selected: null });
  paintVelocity(fakeContext(), 800, 66, view, { notes, index, selected: null });
  hitTest(notes, 100, 170, view, roll, index);
  // A binary search over 10,000 notes plus the five in view, three times over: far below 10,000.
  assert.ok(reads < 200, `${reads} reads of a note's start`);
});

test("a long note whose head is off-screen is still painted", () => {
  const notes = [note(0, 0, 10000), note(1, 20000, 20100)];
  assert.deepEqual(visibleNotes(notes, 4000, 8000).map(n => n.index), [0]);
  const context2d = fakeContext();
  paintRoll(context2d, 800, { start: 4000, zoom: 0.2 }, roll, { notes, index: noteIndex(notes), selected: null });
  const [fill] = noteFills(context2d);
  assert.equal(noteFills(context2d).length, 1);
  assert.equal(fill.x, -800, "it starts 4 s left of the window");
  assert.equal(fill.w, 2000);
});

test("notes that are not in start order are still found", () => {
  const notes = [note(0, 9000, 9100), note(1, 5000, 5100), note(2, 100, 200)];
  assert.deepEqual(visibleNotes(notes, 4000, 8000).map(n => n.index), [1]);
});

test("a note sits on its pitch row and on the editor's time axis", () => {
  const view = { start: 1000, zoom: 0.5 };
  const scrolled = { ...roll, top: 60, scrollY: 28 };
  assert.deepEqual(noteRect(note(0, 1200, 1400, 80), view, scrolled), { x: 100, y: 60 + 4 * 14 - 28, w: 100, h: 13 });
  assert.equal(noteRect(note(0, 1200, 1400, 30), view, scrolled), null, "a pitch with no row has no box");
  assert.deepEqual(pointAt(100, 60 + 4 * 14 - 28 + 3, view, scrolled), { ms: 1200, pitch: 80 });
  assert.deepEqual(visibleRows(scrolled), [2, 21]);
  assert.equal(maxScrollY(scrolled), 49 * 14 - 280);
});

test("hit-testing finds the note under the pointer and its resize edge", () => {
  const notes = [note(0, 1000, 2000, 72), note(1, 3000, 3200, 72)];
  const view = { start: 0, zoom: 0.1 };
  const y = (84 - 72) * 14 + 5;
  assert.equal(hitTest(notes, 150, y, view, roll).note.index, 0);
  assert.equal(hitTest(notes, 150, y, view, roll).edge, false);
  assert.equal(hitTest(notes, 198, y, view, roll).edge, true, "the last 6 px resize");
  assert.equal(hitTest(notes, 250, y, view, roll), null, "between the notes");
  assert.equal(hitTest(notes, 150, y + 14, view, roll), null, "the row below");
});

test("the dragged note is drawn where it would land, and the selected one last", () => {
  const notes = [note(0, 1000, 1200, 72), note(1, 1100, 1300, 72)];
  const context2d = fakeContext();
  paintRoll(context2d, 800, { start: 0, zoom: 0.1 }, roll, {
    notes, index: noteIndex(notes), selected: 0, drag: { index: 0, startMs: 2000, endMs: 2200, pitch: 70 },
  });
  const fills = noteFills(context2d);
  assert.equal(fills.at(-1).style, "#efc16a");
  assert.equal(fills.at(-1).x, 200);
  assert.equal(fills.at(-1).y, (84 - 70) * 14);
});

test("the velocity lane draws one bar per visible note at the note's start", () => {
  const notes = [note(0, 1000, 1200, 72, 127), note(1, 2000, 2200, 72, 64), note(2, 90000, 90100, 72, 10)];
  const context2d = fakeContext();
  paintVelocity(context2d, 800, 66, { start: 0, zoom: 0.1 }, { notes, index: noteIndex(notes), selected: 1 });
  const bars = noteFills(context2d);
  assert.deepEqual(bars.map(bar => [bar.x, bar.h, bar.style]), [[100, 64, "#0a84ff"], [200, 32, "#efc16a"]]);

  // A bar being dragged shows the dragged value.
  const dragged = fakeContext();
  paintVelocity(dragged, 800, 66, { start: 0, zoom: 0.1 }, { notes, index: noteIndex(notes), selected: null, velocityDrag: { index: 1, velocity: 127 } });
  assert.equal(noteFills(dragged)[1].h, 64);
});
