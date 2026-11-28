import assert from "node:assert/strict";
import { test } from "node:test";
import {
  RAMP_BEND_STEP, chordMarkTitle, chordMarks, drawChordMarks, hitTest, maxScrollY, noteIndex, noteRect, paintRoll,
  paintVelocity, pointAt, rampBend, rampCurve, rampMove, rollWheelAction, velocityColour, visibleNotes, visibleRows,
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
    fillRect(x, y, w, h) { calls.push({ kind: "fill", style: this.fillStyle, alpha: this.globalAlpha, x, y, w, h }); },
    strokeRect(x, y, w, h) { calls.push({ kind: "stroke", style: this.strokeStyle, x, y, w, h }); },
    fillText(text, x, y) { calls.push({ kind: "text", style: this.fillStyle, text, x, y }); },
    fill() { calls.push({ kind: "path", style: this.fillStyle }); },
    setLineDash(dash) { calls.push({ kind: "dash", dash }); },
    closePath() {},
  };
}

const pitches = Array.from({ length: 49 }, (_, i) => 84 - i); // C6 down to C2
const roll = { top: 0, height: 280, rowHeight: 14, scrollY: 0, pitches };
const note = (index, startMs, endMs, pitch = 72, velocity = 100) => ({ index, startMs, endMs, pitch, velocity, muted: false });
const noteFills = context2d => context2d.calls.filter(call => call.kind === "fill" && call.style.startsWith("hsl("));

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
  assert.equal(fills.at(-1).x, 200);
  assert.equal(fills.at(-1).y, (84 - 70) * 14);
});

test("the velocity lane draws one bar per visible note at the note's start", () => {
  const notes = [note(0, 1000, 1200, 72, 127), note(1, 2000, 2200, 72, 64), note(2, 90000, 90100, 72, 10)];
  const context2d = fakeContext();
  paintVelocity(context2d, 800, 66, { start: 0, zoom: 0.1 }, { notes, index: noteIndex(notes), selected: 1 });
  const bars = noteFills(context2d);
  assert.deepEqual(bars.map(bar => [bar.x, bar.h, bar.style]), [[100, 64, velocityColour(127)], [200, 32, velocityColour(64)]]);

  // A bar being dragged shows the dragged value.
  const dragged = fakeContext();
  paintVelocity(dragged, 800, 66, { start: 0, zoom: 0.1 }, { notes, index: noteIndex(notes), selected: null, velocityDrag: { index: 1, velocity: 127 } });
  assert.equal(noteFills(dragged)[1].h, 64);
});

const hue = colour => Number(colour.match(/^hsl\((\d+(?:\.\d+)?), 85%, 62%\)$/)[1]);

test("velocity colour runs linearly from blue at 1 to red at 127", () => {
  assert.equal(hue(velocityColour(1)), 215);
  assert.equal(hue(velocityColour(127)), 360);
  assert.equal(hue(velocityColour(64)), 215 + 145 * 63 / 126);
  assert.equal(velocityColour(0), velocityColour(1), "clamped below");
  assert.equal(velocityColour(200), velocityColour(127), "clamped above");
});

// The calls that paint one note, found by its box's x.
const at = (context2d, x) => context2d.calls.filter(call => call.x === x || call.x === x + 0.5 || call.x === x + 3);

test("a note's look: filled by velocity, hollow when Generate leaves it out, grey when muted, white ring when selected", () => {
  const notes = [
    { ...note(0, 1000, 1500, 72, 100), synced: true },
    { ...note(1, 2000, 2500, 72, 40), synced: false },
    { ...note(2, 3000, 3500, 72, 90), synced: null },
    { ...note(3, 4000, 4500, 72, 90), synced: true, muted: true },
    { ...note(4, 5000, 5500, 72, 70), synced: true },
  ];
  const context2d = fakeContext();
  paintRoll(context2d, 800, { start: 0, zoom: 0.1 }, roll, { notes, index: noteIndex(notes), selected: 4 });

  const filled = at(context2d, 100);
  assert.deepEqual(filled.find(call => call.kind === "fill"), { kind: "fill", style: velocityColour(100), alpha: 1, x: 100, y: 168, w: 50, h: 13 },
    "a solid fill: hue alone shows velocity");

  const hollow = at(context2d, 200);
  assert.equal(hollow.filter(call => call.kind === "fill").length, 0, "no fill");
  assert.equal(hollow.find(call => call.kind === "stroke").style, velocityColour(40));
  assert.equal(hollow.find(call => call.kind === "text").style, velocityColour(40), "the name in the velocity colour");

  assert.equal(at(context2d, 300).find(call => call.kind === "fill").style, "#6e6e75", "synced null is grey");
  assert.equal(at(context2d, 400).find(call => call.kind === "fill").style, "#6e6e75", "muted is grey");

  const selected = at(context2d, 500);
  assert.equal(selected.find(call => call.kind === "fill").style, velocityColour(70), "fill unchanged");
  assert.equal(selected.find(call => call.kind === "stroke").style, "#ffffff");
});

test("other tracks' notes are grey ghosts", () => {
  const context2d = fakeContext();
  paintRoll(context2d, 800, { start: 0, zoom: 0.1 }, roll, {
    notes: [], index: noteIndex([]), selected: null, ghosts: [note(9, 1000, 1500, 72, 127)],
  });
  const ghost = context2d.calls.find(call => call.kind === "fill" && call.x === 100);
  assert.equal(ghost.style, "rgba(148,163,184,.22)");
});

test("velocity bars mirror their notes: hollow is an outline, grey stays grey, selected is wide with a white ring", () => {
  const notes = [
    { ...note(0, 1000, 1200, 72, 127), synced: true },
    { ...note(1, 2000, 2200, 72, 64), synced: false },
    { ...note(2, 3000, 3200, 72, 64), synced: null },
    { ...note(3, 4000, 4200, 72, 64), synced: true },
  ];
  const context2d = fakeContext();
  paintVelocity(context2d, 800, 66, { start: 0, zoom: 0.1 }, { notes, index: noteIndex(notes), selected: 3 });
  const plain = at(context2d, 100);
  assert.deepEqual(plain.map(call => [call.kind, call.style, call.w]), [["fill", velocityColour(127), 3]]);
  const hollow = at(context2d, 200);
  assert.deepEqual(hollow.map(call => [call.kind, call.style, call.w]), [["stroke", velocityColour(64), 4]]);
  assert.equal(at(context2d, 300)[0].style, "#6e6e75");
  const selected = context2d.calls.filter(call => call.x >= 399 && call.x <= 401);
  assert.deepEqual(selected.map(call => [call.kind, call.style, call.w]),
    [["fill", velocityColour(64), 5], ["stroke", "#ffffff", 6]]);
});

test("a Shift+drag ramp is linear; Alt eases in, Ctrl eases out, and the wheel bends it in steps up to ±2", () => {
  const press = keys => rampCurve({ altKey: false, ctrlKey: false, ...keys });
  assert.equal(press({}), 0);
  assert.equal(press({ altKey: true }), 1);
  assert.equal(press({ ctrlKey: true }), -1);
  // Holding the key during the drag keeps the shape; letting go keeps what the wheel made.
  assert.equal(rampCurve({ altKey: false, ctrlKey: false }, 0.75), 0.75);
  assert.equal(RAMP_BEND_STEP, 0.25);
  assert.equal(rampBend(0, 120), 0.25);
  assert.equal(rampBend(0, -120), -0.25);
  assert.equal(rampBend(2, 120), 2);
  assert.equal(rampBend(-2, -120), -2);
  // A turn with no vertical step (Shift+wheel can arrive as deltaX only) leaves the curve alone;
  // the caller passes `deltaY || deltaX`.
  assert.equal(rampBend(0.5, 0), 0.5);
});

test("chord marks: one ▼ per conflict time in view, conflicts at one time share it", () => {
  const conflicts = [
    { timeMs: 1000, trackId: 1, pitches: [60, 64, 67] },
    { timeMs: 1000, trackId: 2, pitches: [48, 55] },
    { timeMs: 3000, trackId: 1, pitches: [62, 65] },
    { timeMs: 99000, trackId: 1, pitches: [60, 64] },
  ];
  const marks = chordMarks(conflicts, 0, 0.1, 800);
  assert.deepEqual(marks.map(mark => [mark.x, mark.conflicts.length]), [[100, 2], [300, 1]]);
  assert.equal(chordMarkTitle(marks[0]), "Chord C4 E4 G4\nChord C3 G3\nResolve chords from Generate in the Inspector.");
  assert.deepEqual(chordMarks(null, 0, 0.1, 800), []);

  const context2d = fakeContext();
  drawChordMarks(context2d, marks, 40);
  assert.deepEqual(context2d.calls.filter(call => call.kind === "path").map(call => call.style), ["#ffd60a", "#ffd60a"]);
});

test("an empty track shows one hint line in the roll, and only while it has no notes", async () => {
  const { render } = await import("svelte/server");
  const { default: KeysoundRollSurface } = await import("../src/parts/keysounds/KeysoundRollSurface.svelte");
  const hint = props => render(KeysoundRollSurface, { props: { pitches, trackId: 4, trackName: "Lead", ...props } }).body
    .replace(/<!--[^]*?-->/g, "").match(/<p class="ks-empty[^"]*"[^>]*>([^<]*)<\/p>/)?.[1] ?? null;
  assert.equal(hint({ notes: [] }), "No notes on Lead yet. Double-click in the roll, or pick Draw, to place one.", "no editor here, so no key to name");
  assert.equal(hint({ notes: [note(0, 0, 100)] }), null);
  assert.equal(hint({ notes: [], loading: true }), null);
  assert.equal(hint({ notes: [], trackId: null }), null);
});

test("the empty-track hint tracks bridge.shortcutsRev so Draw updates after refreshShortcuts", async () => {
  const { readFileSync } = await import("node:fs");
  const { render } = await import("svelte/server");
  const { bridge } = await import("../src/lib/bridge.svelte.js");
  const { registerAction, actions } = await import("../src/lib/registry.js");
  const { default: KeysoundRollSurface } = await import("../src/parts/keysounds/KeysoundRollSurface.svelte");
  const surface = readFileSync(new URL("../src/parts/keysounds/KeysoundRollSurface.svelte", import.meta.url), "utf8");
  assert.match(surface, /const drawKey = \$derived\(\([^)]*bridge\.shortcutsRev[^)]*toolKey/);
  let key = "3";
  registerAction("toolKey", tool => (tool === "Draw" ? key : null));
  try {
    const hint = () => render(KeysoundRollSurface, { props: { pitches, trackId: 4, trackName: "Lead", notes: [] } }).body
      .replace(/<!--[^]*?-->/g, "").match(/<p class="ks-empty[^"]*"[^>]*>([^<]*)<\/p>/)?.[1] ?? null;
    assert.match(hint(), /pick Draw \(3\)/);
    key = "2";
    bridge.shortcutsRev++;
    assert.match(hint(), /pick Draw \(2\)/);
  } finally {
    delete actions.toolKey;
  }
});

test("a wheel bend survives pointer moves while Alt or Ctrl stays held; pressing a key mid-drag sets its shape", () => {
  const keys = (altKey, ctrlKey) => ({ altKey, ctrlKey });
  // Shift+Alt press: ease in, then the wheel bends it to 1.25.
  let ramp = { curve: rampCurve(keys(true, false)), keys: 1 };
  ramp = { ...ramp, curve: rampBend(ramp.curve, 120) };
  assert.deepEqual(rampMove(ramp, keys(true, false)), { curve: 1.25, keys: 1 }, "Alt still held: the bend stays");
  assert.deepEqual(rampMove(ramp, keys(false, false)), { curve: 1.25, keys: 0 }, "Alt let go: the bend stays");
  assert.deepEqual(rampMove({ curve: 1.25, keys: 0 }, keys(false, true)), { curve: -1, keys: -1 }, "Ctrl pressed: ease out");
  assert.deepEqual(rampMove({ curve: 0.5, keys: 0 }, keys(false, false)), { curve: 0.5, keys: 0 }, "no key: the bend stays");
});

test("the Draw tool's landing note is a dashed outline with its name; Erase tints the notes it would delete red", () => {
  const notes = [note(0, 1000, 1500, 72), note(1, 2000, 2500, 72), note(2, 3000, 3500, 72)];
  const context2d = fakeContext();
  paintRoll(context2d, 800, { start: 0, zoom: 0.1 }, roll, {
    notes, index: noteIndex(notes), selected: null,
    landing: { startMs: 4000, endMs: 4500, pitch: 70 }, erasing: [0, 2],
  });
  const dashed = context2d.calls.findIndex(call => call.kind === "dash" && call.dash.length);
  assert.ok(dashed >= 0, "a dashed line is set");
  const outline = context2d.calls.slice(dashed).find(call => call.kind === "stroke");
  assert.deepEqual([outline.x, outline.y, outline.w, outline.h], [400.5, (84 - 70) * 14 + 0.5, 49, 12]);
  assert.ok(context2d.calls.slice(dashed).some(call => call.kind === "text" && call.text === "A#4"), "the landing note is named");
  const red = context2d.calls.filter(call => call.kind === "fill" && call.style === "rgba(255,69,58,.55)").map(call => call.x);
  assert.deepEqual(red, [100, 300], "only the notes Erase would delete");
});

test("a pointer move that leaves the Draw or Erase preview as it was keeps it, so hovering repaints nothing", async () => {
  const { sameIndices, sameLanding } = await import("../src/lib/rollPaint.js");
  const { readFileSync } = await import("node:fs");
  const at = { startMs: 125, endMs: 250, pitch: 60 };
  assert.equal(sameLanding(null, null), true);
  assert.equal(sameLanding(at, { ...at }), true, "the same snapped note in a new object");
  assert.equal(sameLanding(at, { ...at, endMs: 375 }), false);
  assert.equal(sameLanding(at, null), false);
  assert.equal(sameIndices([], []), true);
  assert.equal(sameIndices([2], [2]), true);
  assert.equal(sameIndices([2], [3]), false);
  assert.equal(sameIndices([], [3]), false);
  // The surface sets its previews only through that check on a move.
  const surface = readFileSync(new URL("../src/parts/keysounds/KeysoundRollSurface.svelte", import.meta.url), "utf8");
  const onMove = surface.slice(surface.indexOf("function hover"), surface.indexOf("async function onUp"));
  assert.doesNotMatch(onMove, /\blanding = |\berasing = \[\]/);
  assert.match(onMove, /showPreview\(/);
  // Alt pressed or released, or a tool key, updates the preview with the pointer still (plan Q1, Q3).
  assert.match(surface, /<svelte:window onkeydown=\{onAltKey\} onkeyup=\{onAltKey\} \/>/);
  assert.match(onMove, /event\.key === "Alt" && pointer && !drag\) hover\(pointer\.x, pointer\.y, event\.type === "keydown"\)/);
  assert.match(onMove, /bridge\.view\?\.tool;\s*untrack\(\(\) => \{ if \(pointer && !drag\) hover\(/);
});

test("an Erase drag hits every note along the pointer's path, not only where moves were reported", async () => {
  const { notesAlong } = await import("../src/lib/rollPaint.js");
  // Two short notes on C5 at x 100-110 and 300-310; one reported move jumps from x 50 to x 400.
  const notes = [note(0, 1000, 1100, 72), note(1, 3000, 3100, 72), note(2, 5000, 5100, 72)];
  const view = { start: 0, zoom: 0.1 };
  const y = (84 - 72) * 14 + 5;
  assert.deepEqual(notesAlong(notes, { x: 50, y }, { x: 400, y }, view, roll), [0, 1]);
  assert.deepEqual(notesAlong(notes, { x: 50, y: y + 14 }, { x: 400, y: y + 14 }, view, roll), [], "the row below");
  assert.deepEqual(notesAlong(notes, { x: 105, y }, { x: 105, y }, view, roll), [0], "a press without a move");
});
