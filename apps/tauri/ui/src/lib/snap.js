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

// osu!lazer's snap groups (BeatDivisorPresetCollection.cs:21-39 at 131fe72a15581049401a588a13684938d4d0e0c7).
export const COMMON = { name: "common", presets: [1, 2, 4, 8, 16] };
export const TRIPLETS = { name: "triplets", presets: [1, 3, 6, 12] };

// A divisor in neither group gets a group of its own whole divisors: 1/5 gives {1, 5}.
function customGroup(divisor) {
  const presets = [];
  for (let candidate = 1; candidate <= divisor; candidate++) if (divisor % candidate === 0) presets.push(candidate);
  return { name: "custom", presets };
}

// The 1/x a snap stands for, or null for a snap like 4/1 or 3/4.
const wholeDivisor = (beats, divisor) => divisor % beats === 0 ? divisor / beats : null;

// Like lazer's SetArbitraryDivisor: keep the group that has the snap, else Common, else Triplets, else
// the snap's own group. A snap that is no 1/x keeps the group (or starts in Common).
export function nextGroup(group, beats, divisor) {
  const whole = wholeDivisor(beats, divisor);
  if (whole === null) return group ?? COMMON;
  if (group?.presets.includes(whole)) return group;
  if (COMMON.presets.includes(whole)) return COMMON;
  if (TRIPLETS.presets.includes(whole)) return TRIPLETS;
  return customGroup(whole);
}

// The group after the editor shows `scene`: none without a map, worked out afresh when a map opens.
export function followGroup(group, shown, scene) {
  if (!scene) return null;
  return nextGroup(shown?.path === scene.path ? group : null, scene.beats, scene.divisor);
}

// Lazer's SelectNext/SelectPrevious: the next finer or coarser 1/x in the group, or null at either end.
export function stepInGroup(group, beats, divisor, finer) {
  const length = beats / divisor;
  const presets = finer ? group.presets : [...group.presets].reverse();
  return presets.find(preset => finer ? 1 / preset < length : 1 / preset > length) ?? null;
}

// BindableBeatDivisor.cs:104-140, hexes from OsuColour.cs; the timeline's snapColours in app.js match.
const DIVISOR_COLOURS = {
  1: "#ffffff", 2: "#ed1121", 3: "#8866ee", 4: "#66ccff", 5: "#b3d944", 6: "#eeaa00",
  7: "#b3d944", 8: "#ffcc22", 9: "#b3d944", 12: "#cc6600", 16: "#6644cc",
};
export const divisorColour = divisor => DIVISOR_COLOURS[divisor] ?? "#ff0000";

// A snap takes the colour of its divisor in lowest terms: 2/4 is red like 1/2, 4/1 white like 1/1.
export function snapColour(beats, divisor) {
  let [a, b] = [beats, divisor];
  while (b) [a, b] = [b, a % b];
  return divisorColour(divisor / a);
}
