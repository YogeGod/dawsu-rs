// The keysound piano roll on canvas: which notes are in view, where each one goes, and the drawing.
// Pure functions over a 2D context and plain data, so a fake context can test them.
//
// `view` is the editor's own time axis: { start, zoom } with x = (ms - start) * zoom.
// `roll` is the roll's part of the canvas and its pitch axis:
//   { top, height, rowHeight, scrollY, pitches } with `pitches` the visible rows, highest first.
import { sampleVelocityRamp } from "./pianoRoll.js";

export const ROLL_ROW_MIN = 8;
export const ROLL_ROW_MAX = 28;
// A note's right edge, this wide, resizes it.
export const NOTE_EDGE_PX = 6;
const NOTE_MIN_PX = 4;
const VELOCITY_BAR_PX = 3;
// A hollow or selected note's velocity bar is wider, so its outline shows.
const VELOCITY_BAR_WIDE_PX = 5;
// A wheel turn during a Shift+drag ramp bends it this much, up to ±RAMP_BEND_MAX.
export const RAMP_BEND_STEP = 0.25;
const RAMP_BEND_MAX = 2;
// A chord conflict's ▼ on the ruler: half its width and its height, in px.
const CHORD_MARK_HALF_PX = 4;
const CHORD_MARK_HEIGHT_PX = 6;
// Conflicts closer than this on the ruler share one ▼.
const CHORD_MARK_MERGE_PX = 8;

const colours = {
  lane: "#1c1c1e",
  laneBlack: "#151517",
  laneOutOfScale: "#101012",
  laneEdge: "rgba(255,255,255,.03)",
  octaveEdge: "rgba(255,255,255,.1)",
  objectLine: "rgba(239,193,106,.28)",
  sustain: "rgba(56,189,248,.12)",
  ghost: "rgba(148,163,184,.22)",
  noteMuted: "#6e6e75",
  noteEdge: "rgba(0,0,0,.45)",
  noteSelectedEdge: "#ffffff",
  noteName: "#08080a",
  // --editor-warning in tokens.css.
  chordMark: "#ffd60a",
  landing: "rgba(255,255,255,.85)",
  erasing: "rgba(255,69,58,.55)",
  erasingEdge: "#ff453a",
  chordMarkEdge: "#08080a",
  velocityLane: "#0e0e10",
  velocityGuide: "rgba(255,255,255,.06)",
  ramp: "#efc16a",
};

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const pitchName = pitch => `${NAMES[pitch % 12]}${Math.floor(pitch / 12) - 1}`;
const isBlack = pitch => NAMES[pitch % 12].length > 1;

// A note's colour from its velocity: blue (215°) at 1 through purple to red (360°) at 127, linear
// (plan revision 4, Q1 A). Notes and velocity bars share it.
export function velocityColour(velocity) {
  const clamped = Math.min(127, Math.max(1, Number(velocity) || 1));
  return `hsl(${215 + 145 * (clamped - 1) / 126}, 85%, 62%)`;
}

// What Generate does with a note, as the roll draws it: "grey" when the matcher never considers it
// (muted note, or a muted or not-soloed track: synced null), "hollow" when it is considered and left
// out (synced false), else "filled".
function noteState(note) {
  if (note.muted || note.synced === null) return "grey";
  return note.synced === false ? "hollow" : "filled";
}

// A Shift+drag ramp's curve from the keys held: Alt eases in (+1), Ctrl eases out (−1), neither
// keeps `fallback` (0 at the press, so plain Shift+drag is linear).
export const rampCurve = (event, fallback = 0) => (event.altKey ? 1 : event.ctrlKey ? -1 : fallback);

// A pointer move during a ramp: only pressing Alt or Ctrl changes the shape, so a wheel bend made
// while a key is held survives the moves after it. `keys` is the shape the keys gave last.
export function rampMove(ramp, event) {
  const keys = rampCurve(event);
  return { curve: keys !== ramp.keys && keys !== 0 ? keys : ramp.curve, keys };
}

// A wheel turn while a ramp is drawn bends it one step, down for a turn towards the user. A turn
// with no step (0) leaves it alone.
export const rampBend = (curve, delta) => !delta ? curve
  : Math.max(-RAMP_BEND_MAX, Math.min(RAMP_BEND_MAX, curve + (delta > 0 ? RAMP_BEND_STEP : -RAMP_BEND_STEP)));

// What visibleNotes() needs to know about a track's notes, found once per summary and not once per
// paint: the longest note, and whether the notes are in start order.
export function noteIndex(notes) {
  let longest = 0;
  let sorted = true;
  for (let i = 0; i < (notes?.length ?? 0); i++) {
    longest = Math.max(longest, notes[i].endMs - notes[i].startMs);
    if (i && notes[i].startMs < notes[i - 1].startMs) sorted = false;
  }
  return { longest, sorted };
}

// The notes that overlap [startMs, endMs]. Notes arrive sorted by start, so a binary search finds
// the first candidate; it starts `longest` ms early so a long note whose head is off-screen stays.
// Notes out of order (never from Rust, but cheap to survive) are scanned whole.
export function visibleNotes(notes, startMs, endMs, index = noteIndex(notes)) {
  if (!notes?.length) return [];
  if (!index.sorted) return notes.filter(note => note.endMs >= startMs && note.startMs <= endMs);
  let low = 0;
  let high = notes.length;
  const from = startMs - index.longest;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (notes[middle].startMs < from) low = middle + 1;
    else high = middle;
  }
  const seen = [];
  for (let i = low; i < notes.length && notes[i].startMs <= endMs; i++) {
    if (notes[i].endMs >= startMs) seen.push(notes[i]);
  }
  return seen;
}

// What the editor's tool does in the roll: Draw and Erase keep their meaning; every other tool
// (Select, Mute, Grab, Range, Ramp) moves and resizes as Select does (plan 3.5).
export const rollTool = tool => (tool === "Draw" ? "draw" : tool === "Erase" ? "erase" : "select");

// Whether a pointer move left the Draw landing note or the Erase set as it was, so hovering
// repaints the lanes only when a preview changes.
export const sameLanding = (a, b) =>
  a === b || (!!a && !!b && a.startMs === b.startMs && a.endMs === b.endMs && a.pitch === b.pitch);
export const sameIndices = (a, b) => a.length === b.length && a.every((index, at) => index === b[at]);

// What a wheel turn over the roll does: "pitch" scrolls the rows, "rows" sets the row height, and
// null leaves the turn to the editor's own handler (time zoom, snap step, pan), which must still
// see Ctrl+wheel, Alt+wheel, Shift+wheel and a sideways turn.
export function rollWheelAction({ ctrlKey, shiftKey, altKey, metaKey, deltaY }) {
  if (!deltaY) return null;
  if (ctrlKey && shiftKey && !altKey) return "rows";
  return ctrlKey || shiftKey || altKey || metaKey ? null : "pitch";
}

// The rows in view: [first, last] indexes into roll.pitches.
export function visibleRows(roll) {
  const first = Math.max(0, Math.floor(roll.scrollY / roll.rowHeight));
  const last = Math.min(roll.pitches.length - 1, Math.ceil((roll.scrollY + roll.height) / roll.rowHeight) - 1);
  return [first, last];
}

// The furthest the roll scrolls down: its last row at the bottom edge.
export const maxScrollY = roll => Math.max(0, roll.pitches.length * roll.rowHeight - roll.height);

// A note's box on the canvas, or null when its pitch has no row (folded away).
export function noteRect(note, view, roll) {
  const row = roll.pitches.indexOf(note.pitch);
  if (row < 0) return null;
  return {
    x: (note.startMs - view.start) * view.zoom,
    y: roll.top + row * roll.rowHeight - roll.scrollY,
    w: Math.max(NOTE_MIN_PX, (note.endMs - note.startMs) * view.zoom),
    h: roll.rowHeight - 1,
  };
}

// The time and pitch under a canvas point.
export function pointAt(x, y, view, roll) {
  const row = Math.floor((y - roll.top + roll.scrollY) / roll.rowHeight);
  const clamped = Math.min(roll.pitches.length - 1, Math.max(0, row));
  return { ms: Math.max(0, view.start + x / view.zoom), pitch: roll.pitches[clamped] };
}

// The note under a canvas point, the last drawn first; `edge` when the point is on its resize edge.
export function hitTest(notes, x, y, view, roll, index = noteIndex(notes)) {
  const reach = NOTE_MIN_PX / view.zoom;
  const near = visibleNotes(notes, view.start + x / view.zoom - reach, view.start + x / view.zoom + reach, index);
  for (let i = near.length - 1; i >= 0; i--) {
    const box = noteRect(near[i], view, roll);
    if (!box || x < box.x || x > box.x + box.w || y < box.y || y > box.y + box.h) continue;
    return { note: near[i], edge: x >= box.x + box.w - Math.min(NOTE_EDGE_PX, box.w / 2) };
  }
  return null;
}

function inScale(pitch, mask) {
  return ((mask >> (((pitch % 12) + 12) % 12)) & 1) === 1;
}

// The whole roll inside its part of the canvas: rows, the map's grid (drawGrid, the editor's own),
// object lines, other tracks' notes as ghosts, then this track's notes.
// model: { notes, index, ghosts, objectTimes, sustainSpans, selected, drag, scaleMask, foldToScale }
// with `drag` the dragged note as it would land: { index, startMs, endMs, pitch }.
export function paintRoll(context2d, width, view, roll, model, drawGrid = null) {
  const { top, height, rowHeight } = roll;
  const endMs = view.start + width / view.zoom;
  context2d.save();
  context2d.beginPath();
  context2d.rect(0, top, width, height);
  context2d.clip();

  context2d.fillStyle = colours.lane;
  context2d.fillRect(0, top, width, height);
  const [firstRow, lastRow] = visibleRows(roll);
  for (let row = firstRow; row <= lastRow; row++) {
    const pitch = roll.pitches[row];
    const y = top + row * rowHeight - roll.scrollY;
    const outside = !model.foldToScale && !inScale(pitch, model.scaleMask ?? 0xfff);
    if (outside || isBlack(pitch)) {
      context2d.fillStyle = outside ? colours.laneOutOfScale : colours.laneBlack;
      context2d.fillRect(0, y, width, rowHeight);
    }
    context2d.fillStyle = pitch % 12 === 0 ? colours.octaveEdge : colours.laneEdge;
    context2d.fillRect(0, y + rowHeight - 1, width, 1);
  }

  context2d.fillStyle = colours.sustain;
  for (const span of model.sustainSpans ?? []) {
    const [from, to] = [span.start_ms, span.end_ms];
    if (!(to >= view.start && from <= endMs)) continue;
    context2d.fillRect((from - view.start) * view.zoom, top, Math.max(1, (to - from) * view.zoom), height);
  }

  drawGrid?.(context2d, top, top + height);

  context2d.fillStyle = colours.objectLine;
  for (const time of model.objectTimes ?? []) {
    if (time < view.start || time > endMs) continue;
    context2d.fillRect(Math.round((time - view.start) * view.zoom), top, 1, height);
  }

  context2d.fillStyle = colours.ghost;
  for (const ghost of model.ghosts ?? []) {
    if (ghost.endMs < view.start || ghost.startMs > endMs) continue;
    const box = noteRect(ghost, view, roll);
    if (box && box.y + box.h >= top && box.y <= top + height) context2d.fillRect(box.x, box.y, box.w, box.h);
  }

  context2d.font = "9px Inter, system-ui, sans-serif";
  context2d.textBaseline = "middle";
  let selectedNote = null;
  const drawNote = note => {
    const dragged = model.drag?.index === note.index ? { ...note, ...model.drag } : note;
    const box = noteRect(dragged, view, roll);
    if (!box || box.y + box.h < top || box.y > top + height) return;
    const selected = model.selected === note.index;
    const state = noteState(note);
    const tone = velocityColour(dragged.velocity);
    if (state !== "hollow") {
      context2d.fillStyle = state === "grey" ? colours.noteMuted : tone;
      context2d.fillRect(box.x, box.y, box.w, box.h);
    }
    context2d.strokeStyle = selected ? colours.noteSelectedEdge : state === "hollow" ? tone : colours.noteEdge;
    context2d.strokeRect(box.x + 0.5, box.y + 0.5, box.w - 1, box.h - 1);
    if (rowHeight >= 11 && box.w >= 22) {
      context2d.fillStyle = state === "hollow" ? tone : colours.noteName;
      context2d.fillText(pitchName(dragged.pitch), box.x + 3, box.y + box.h / 2 + 0.5);
    }
  };
  // The dragged note may start outside the window it is dragged into, so it is always a candidate.
  for (const note of visibleNotes(model.notes, view.start, endMs, model.index)) {
    if (model.selected === note.index || model.drag?.index === note.index) selectedNote = note;
    else drawNote(note);
  }
  if (!selectedNote && model.drag) selectedNote = (model.notes ?? []).find(note => note.index === model.drag.index) ?? null;
  // The selected note is drawn last, over its neighbours.
  if (selectedNote) drawNote(selectedNote);
  // Erase: the notes a release would delete, tinted red.
  if (model.erasing?.length) {
    const erasing = new Set(model.erasing);
    for (const note of visibleNotes(model.notes, view.start, endMs, model.index)) {
      if (!erasing.has(note.index)) continue;
      const box = noteRect(note, view, roll);
      if (!box) continue;
      context2d.fillStyle = colours.erasing;
      context2d.fillRect(box.x, box.y, box.w, box.h);
      context2d.strokeStyle = colours.erasingEdge;
      context2d.strokeRect(box.x + 0.5, box.y + 0.5, box.w - 1, box.h - 1);
    }
  }
  // Draw: where a click (or the drag so far) puts a note, as a dashed outline with its name.
  const landing = model.landing ? noteRect(model.landing, view, roll) : null;
  if (landing) {
    context2d.setLineDash([3, 2]);
    context2d.strokeStyle = colours.landing;
    context2d.strokeRect(landing.x + 0.5, landing.y + 0.5, landing.w - 1, landing.h - 1);
    context2d.setLineDash([]);
    if (rowHeight >= 11 && landing.w >= 22) {
      context2d.fillStyle = colours.landing;
      context2d.fillText(pitchName(model.landing.pitch), landing.x + 3, landing.y + landing.h / 2 + 0.5);
    }
  }
  context2d.restore();
}

// The velocity a note shows in the lane: the ramp or bar being dragged wins over its own.
export function shownVelocity(note, model) {
  const ramp = model.ramp;
  if (ramp) {
    const preview = sampleVelocityRamp(note.startMs, ramp.startMs, ramp.endMs, ramp.startVel, ramp.endVel, ramp.curve);
    if (preview != null) return preview;
  }
  return model.velocityDrag?.index === note.index ? model.velocityDrag.velocity : note.velocity;
}

// The velocity lane: one bar per note in view, at the note's start, on the editor's time axis.
// model: { notes, index, selected, velocityDrag: { index, velocity }, ramp }.
export function paintVelocity(context2d, width, height, view, model) {
  context2d.fillStyle = colours.velocityLane;
  context2d.fillRect(0, 0, width, height);
  context2d.fillStyle = colours.velocityGuide;
  for (const fraction of [0, 0.5, 1]) context2d.fillRect(0, Math.round((height - 1) * fraction), width, 1);
  const endMs = view.start + width / view.zoom;
  for (const note of visibleNotes(model.notes, view.start, endMs, model.index)) {
    const velocity = shownVelocity(note, model);
    const barHeight = Math.max(1, Math.round((height - 2) * velocity / 127));
    const state = noteState(note);
    const selected = model.selected === note.index;
    const x = Math.round((note.startMs - view.start) * view.zoom);
    const y = height - barHeight;
    const width = state === "hollow" || selected ? VELOCITY_BAR_WIDE_PX : VELOCITY_BAR_PX;
    const tone = state === "grey" ? colours.noteMuted : velocityColour(velocity);
    // A bar mirrors its note: hollow is an outline, and the selected bar wears the white ring.
    if (state === "hollow") {
      context2d.strokeStyle = tone;
      context2d.strokeRect(x + 0.5, y + 0.5, width - 1, barHeight - 1);
    } else {
      context2d.fillStyle = tone;
      context2d.fillRect(x, y, width, barHeight);
    }
    if (selected) {
      context2d.strokeStyle = colours.noteSelectedEdge;
      context2d.strokeRect(x - 0.5, y - 0.5, width + 1, barHeight + 1);
    }
  }
  const ramp = model.ramp;
  if (ramp) {
    context2d.strokeStyle = colours.ramp;
    context2d.beginPath();
    const steps = 24;
    for (let i = 0; i <= steps; i++) {
      const ms = ramp.startMs + (ramp.endMs - ramp.startMs) * i / steps;
      const velocity = sampleVelocityRamp(ms, ramp.startMs, ramp.endMs, ramp.startVel, ramp.endVel, ramp.curve) ?? ramp.startVel;
      const x = (ms - view.start) * view.zoom;
      const y = (height - 2) * (1 - velocity / 127) + 1;
      if (i) context2d.lineTo(x, y);
      else context2d.moveTo(x, y);
    }
    context2d.stroke();
  }
}

// The chord conflicts in view as ▼ marks on the ruler, left to right: { x, conflicts }. Every
// track's conflicts are marked, the same set the Inspector counts; conflicts within a few px share
// a mark, so chords on one object in two tracks draw once.
export function chordMarks(conflicts, start, zoom, width) {
  const seen = (conflicts ?? [])
    .map(conflict => ({ conflict, x: (conflict.timeMs - start) * zoom }))
    .filter(({ x }) => x >= 0 && x <= width)
    .sort((first, second) => first.x - second.x);
  const marks = [];
  for (const { conflict, x } of seen) {
    const last = marks[marks.length - 1];
    if (last && x - last.x < CHORD_MARK_MERGE_PX) last.conflicts.push(conflict);
    else marks.push({ x, conflicts: [conflict] });
  }
  return marks;
}

// The ruler's tooltip over a chord mark: its chords, and where to resolve them.
export function chordMarkTitle(mark) {
  const chords = mark.conflicts.map(conflict => "Chord " + conflict.pitches.map(pitchName).join(" "));
  return [...chords, "Resolve chords from Generate in the Inspector."].join("\n");
}

// Each mark as a warning-yellow ▼ resting on the ruler's bottom edge, `bottom`, pointing at its object.
export function drawChordMarks(context2d, marks, bottom) {
  for (const { x } of marks) {
    const centre = Math.round(x) + 0.5;
    context2d.beginPath();
    context2d.moveTo(centre - CHORD_MARK_HALF_PX, bottom - 1 - CHORD_MARK_HEIGHT_PX);
    context2d.lineTo(centre + CHORD_MARK_HALF_PX, bottom - 1 - CHORD_MARK_HEIGHT_PX);
    context2d.lineTo(centre, bottom - 1);
    context2d.closePath();
    context2d.fillStyle = colours.chordMark;
    context2d.fill();
    context2d.strokeStyle = colours.chordMarkEdge;
    context2d.stroke();
  }
}
