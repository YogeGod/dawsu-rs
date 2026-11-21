// Pure helpers for the More Snaps (#options-Grid) and Lanes (#options-Lanes) popovers.

export function clampSnapPart(value, fallback) {
  const numeric = typeof value === "string" ? (value.trim() === "" ? NaN : Number(value)) : Number(value);
  if (!Number.isFinite(numeric)) return fallback;
  return Math.min(64, Math.max(1, Math.trunc(numeric)));
}

export function snapPopoverAction(scene, patch = {}) {
  if (!scene) return null;
  return {
    kind: "snap",
    on: patch.on ?? scene.snap,
    mode: patch.mode ?? scene.snapMode,
    beats: clampSnapPart(patch.beats, scene.beats),
    divisor: clampSnapPart(patch.divisor, scene.divisor),
  };
}

export function tripletSnapAction(scene, triplets) {
  if (!scene) return null;
  return {
    kind: "snap",
    on: scene.snap,
    mode: scene.snapMode,
    beats: 1,
    divisor: triplets ? 6 : 4,
  };
}

export function stepLaneHeight(current, factor) {
  return Math.round(current * factor);
}
