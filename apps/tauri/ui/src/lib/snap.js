// The v3 snap slider's stops, as 1/x of a beat. Every other snap stays in the Snap ▾ menu.
export const STOPS = [1, 2, 3, 4, 6, 8, 12, 16];

const finer = ["ArrowRight", "ArrowUp"];
const coarser = ["ArrowLeft", "ArrowDown"];
export const sliderKeys = [...finer, ...coarser, "Home", "End"];

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
  if (coarser.includes(key)) return Math.max(0, STOPS.findLastIndex(stop => 1 / stop > length));
  return null;
}

export const snapAction = (scene, index) =>
  ({ kind: "snap", on: scene.snap, mode: scene.snapMode, beats: 1, divisor: STOPS[index] });
