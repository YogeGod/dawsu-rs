// Where the keysound roll puts a note, from the editor's own Snap (plan revision 5, §9).
// Pure over the scene the editor paints from: { snap, snapMode, beats, divisor, sections, moments }.
//
// A plain click floors to the snap step and lasts one step. With Snap off it stays where clicked
// and lasts one beat at that time's tempo; in Objects mode it starts on the nearest hit object and
// lasts one beat. Move and resize snap to the nearest step or object. Every result is whole ms.
import { snapTimeInScene } from "../editor/helpers.js";

// Rust's resize_note keeps a note at least this long.
const NOTE_FLOOR_MS = 20;
const FALLBACK_BEAT_MS = 500;
const whole = ms => Math.max(0, Math.round(ms));

// The tempo section in force at `ms`: [origin, beat length].
function sectionAt(scene, ms) {
  const sections = scene?.sections ?? [];
  let found = null;
  for (const section of sections) {
    if (section[0] <= ms || !found) found = section;
    if (section[0] > ms) break;
  }
  return found && found[1] > 0 ? found : [0, FALLBACK_BEAT_MS];
}

const beatAt = (scene, ms) => sectionAt(scene, ms)[1];

// The grid step in force at `ms`, as snapToActiveGridInScene measures it.
function stepAt(scene, ms) {
  const [origin, beat] = sectionAt(scene, ms);
  return { origin, step: (beat * Math.max(1, scene?.beats || 1)) / Math.max(1, scene?.divisor || 4) };
}

// A clicked note: { startMs, lengthMs }.
export function placeNote(scene, ms) {
  const at = Math.max(0, ms);
  if (!scene?.snap) return { startMs: whole(at), lengthMs: whole(beatAt(scene, at)) };
  if (scene.snapMode === "objects") {
    const startMs = whole(snapTimeInScene(scene, at));
    return { startMs, lengthMs: whole(beatAt(scene, startMs)) };
  }
  const { origin, step } = stepAt(scene, at);
  const start = origin + Math.floor((at - origin) / step + 1e-6) * step;
  return { startMs: whole(start), lengthMs: whole(start + step) - whole(start) };
}

// Where a moved start or a resized end lands.
export const snapNearest = (scene, ms) => whole(snapTimeInScene(scene, ms));

// The snap noteDragPayload takes for move and resize: nearest step, and a note at least one step
// long (Rust's 20 ms with Snap off).
export function noteSnap(scene) {
  return {
    nearest: ms => snapNearest(scene, ms),
    minLength: ms => (scene?.snap ? Math.max(NOTE_FLOOR_MS, whole(stepAt(scene, ms).step)) : NOTE_FLOOR_MS),
  };
}

// The end of a note drawn from `placed` and released at `ms`: the snapped release point, never
// shorter than the click's own length. With Snap off a drag sets any length down to 20 ms, and a
// press with no drag keeps the click length.
export function drawEnd(scene, placed, ms, dragged = true) {
  const clickEnd = placed.startMs + placed.lengthMs;
  if (!scene?.snap) return dragged ? Math.max(placed.startMs + NOTE_FLOOR_MS, whole(ms)) : clickEnd;
  return Math.max(clickEnd, snapNearest(scene, ms));
}
