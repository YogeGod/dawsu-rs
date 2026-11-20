// The v3 snap slider's stops, as 1/x of a beat. Every other snap stays in the Snap ▾ menu.
export const STOPS = [1, 2, 3, 4, 6, 8, 12, 16];

const finer = ["ArrowRight", "ArrowUp"];
const coarser = ["ArrowLeft", "ArrowDown"];
const sliderKeys = [...finer, ...coarser, "Home", "End"];

// The slider takes Arrow, Home and End without modifiers; every other key goes to the editor.
export const ownsKey = event => sliderKeys.includes(event.key) && !event.ctrlKey && !event.altKey && !event.metaKey;

// After a mouse drag or click Left and Right seek again; only keyboard use (Tab) keeps focus.
export const keepsFocus = input => input === "keyboard";

// The snap the slider stands for: the stop being chosen, or else the editor's snap.
export const currentSnap = (local, scene) => local !== null ? [1, STOPS[local]] : [scene.beats, scene.divisor];

// A move sends a snap change only when the target stop differs from that snap.
export const changesSnap = ([beats, divisor], target) => divisor !== beats * STOPS[target];

// Stops compare by snap length on a log scale, so 1/5 sits nearer 1/6 than 1/4.
export function nearestStop(beats, divisor) {
  const distance = stop => Math.abs(Math.log(beats * stop / divisor));
  return STOPS.reduce((best, stop, index) => distance(stop) < distance(STOPS[best]) ? index : best, 0);
}

export const onStop = (beats, divisor) => STOPS.some(stop => divisor === beats * stop);

export const stopAt = fraction => Math.min(STOPS.length - 1, Math.max(0, Math.round(fraction * (STOPS.length - 1))));

// Like the wheel in app.js: the next stop strictly finer or coarser than the current snap.
export function stepStop(beats, divisor, key) {
  const length = beats / divisor;
  if (key === "Home") return 0;
  if (key === "End") return STOPS.length - 1;
  if (finer.includes(key)) {
    const index = STOPS.findIndex(stop => 1 / stop < length);
    return index < 0 ? STOPS.length - 1 : index;
  }
  if (coarser.includes(key)) {
    // A reverse loop: Safari before 15.4 lacks findLastIndex.
    for (let index = STOPS.length - 1; index >= 0; index--) if (1 / STOPS[index] > length) return index;
    return 0;
  }
  return null;
}

export const snapAction = (scene, index) =>
  ({ kind: "snap", on: scene.snap, mode: scene.snapMode, beats: 1, divisor: STOPS[index] });
