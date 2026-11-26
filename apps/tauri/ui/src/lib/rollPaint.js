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

const colours = {
  lane: "#1c1c1e",
  laneBlack: "#151517",
  laneOutOfScale: "#101012",
  laneEdge: "rgba(255,255,255,.03)",
  octaveEdge: "rgba(255,255,255,.1)",
  objectLine: "rgba(239,193,106,.28)",
  sustain: "rgba(56,189,248,.12)",
  ghost: "rgba(148,163,184,.22)",
  note: "#0a84ff",
  noteSelected: "#efc16a",
  noteMuted: "#6e6e75",
  noteEdge: "rgba(0,0,0,.45)",
  noteSelectedEdge: "#fff3d6",
  noteName: "#08080a",
  velocityLane: "#0e0e10",
  velocityGuide: "rgba(255,255,255,.06)",
  ramp: "#efc16a",
};

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const pitchName = pitch => `${NAMES[pitch % 12]}${Math.floor(pitch / 12) - 1}`;
const isBlack = pitch => NAMES[pitch % 12].length > 1;

// The longest note of a track, found once per summary; visibleNotes() needs it.
export function longestNoteMs(notes) {
  let longest = 0;
  for (const note of notes ?? []) longest = Math.max(longest, note.endMs - note.startMs);
  return longest;
}

function sortedByStart(notes) {
  for (let i = 1; i < notes.length; i++) if (notes[i].startMs < notes[i - 1].startMs) return false;
  return true;
}

// The notes that overlap [startMs, endMs]. Notes arrive sorted by start, so a binary search finds
// the first candidate; it starts `longest` ms early so a long note whose head is off-screen stays.
// Notes out of order (never from Rust, but cheap to survive) are scanned whole.
export function visibleNotes(notes, startMs, endMs, longest = longestNoteMs(notes)) {
  if (!notes?.length) return [];
  if (!sortedByStart(notes)) return notes.filter(note => note.endMs >= startMs && note.startMs <= endMs);
  let low = 0;
  let high = notes.length;
  const from = startMs - longest;
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
export function hitTest(notes, x, y, view, roll, longest = longestNoteMs(notes)) {
  const reach = NOTE_MIN_PX / view.zoom;
  const near = visibleNotes(notes, view.start + x / view.zoom - reach, view.start + x / view.zoom + reach, longest);
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
// model: { notes, longest, ghosts, objectTimes, sustainSpans, selected, drag, scaleMask, foldToScale }
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
    context2d.globalAlpha = 0.45 + 0.55 * note.velocity / 127;
    context2d.fillStyle = note.muted ? colours.noteMuted : selected ? colours.noteSelected : colours.note;
    context2d.fillRect(box.x, box.y, box.w, box.h);
    context2d.globalAlpha = 1;
    context2d.strokeStyle = selected ? colours.noteSelectedEdge : colours.noteEdge;
    context2d.strokeRect(box.x + 0.5, box.y + 0.5, box.w - 1, box.h - 1);
    if (rowHeight >= 11 && box.w >= 22) {
      context2d.fillStyle = colours.noteName;
      context2d.fillText(pitchName(dragged.pitch), box.x + 3, box.y + box.h / 2 + 0.5);
    }
  };
  // The dragged note may start outside the window it is dragged into, so it is always a candidate.
  for (const note of visibleNotes(model.notes, view.start, endMs, model.longest)) {
    if (model.selected === note.index || model.drag?.index === note.index) selectedNote = note;
    else drawNote(note);
  }
  if (!selectedNote && model.drag) selectedNote = (model.notes ?? []).find(note => note.index === model.drag.index) ?? null;
  // The selected note is drawn last, over its neighbours.
  if (selectedNote) drawNote(selectedNote);
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
// model: { notes, longest, selected, velocityDrag: { index, velocity }, ramp }.
export function paintVelocity(context2d, width, height, view, model) {
  context2d.fillStyle = colours.velocityLane;
  context2d.fillRect(0, 0, width, height);
  context2d.fillStyle = colours.velocityGuide;
  for (const fraction of [0, 0.5, 1]) context2d.fillRect(0, Math.round((height - 1) * fraction), width, 1);
  const endMs = view.start + width / view.zoom;
  for (const note of visibleNotes(model.notes, view.start, endMs, model.longest)) {
    const velocity = shownVelocity(note, model);
    const barHeight = Math.max(1, Math.round((height - 2) * velocity / 127));
    context2d.fillStyle = note.muted ? colours.noteMuted : model.selected === note.index ? colours.noteSelected : colours.note;
    context2d.fillRect(Math.round((note.startMs - view.start) * view.zoom), height - barHeight, VELOCITY_BAR_PX, barHeight);
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
