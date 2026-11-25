// Pure Velocity Lollipop Lane (1-127) coordinate, tier threshold, and per-hit volume helpers.
// Matches `gain_to_midi_velocity` in `crates/dawsu-application/src/drum_vst.rs:332`.

export const VELOCITY_TIERS = Object.freeze({
  ghostMax: 71,
  midMax: 104,
});

/** Mirrors Rust `drum_vst::gain_to_midi_velocity(gain: f32) -> u8`. */
export function gainToMidiVelocity(gain) {
  const clamped = Math.max(0, Math.min(1, Number(gain) || 0));
  return Math.max(1, Math.min(127, Math.round(clamped * 127)));
}

/** Converts osu! hit volume (5..100%) to MIDI velocity (1..127) via `gainToMidiVelocity`. */
export function osuVolumeToMidiVelocity(volumePercent) {
  return gainToMidiVelocity((Number(volumePercent) || 0) / 100);
}

/** Converts MIDI velocity (1..127) back to osu! saved hit volume (5..100%). */
export function midiVelocityToOsuVolume(velocity) {
  const clamped = Math.max(1, Math.min(127, Math.round(Number(velocity) || 1)));
  return Math.max(5, Math.min(100, Math.round((clamped / 127) * 100)));
}

/** Classifies a MIDI velocity (1..127) into Ghost (<72), Mid (72..104), or Hard (105..127). */
export function velocityTier(velocity) {
  const v = Math.max(1, Math.min(127, Math.round(Number(velocity) || 1)));
  if (v <= VELOCITY_TIERS.ghostMax) return "ghost";
  if (v <= VELOCITY_TIERS.midMax) return "mid";
  return "hard";
}

/** Converts a pointer Y coordinate inside the controller lane to MIDI velocity (1..127). */
export function yToVelocity(y, laneHeight) {
  const h = Math.max(1, Number(laneHeight) || 1);
  const ratio = 1 - Math.max(0, Math.min(1, y / h));
  return Math.max(1, Math.min(127, Math.round(1 + ratio * 126)));
}

/**
 * Applies a velocity lollipop drag:
 * - Single-hit drag edits ONLY that hit's per-hit volume (leaving simultaneous hits untouched).
 * - Multi-select drag applies a relative delta preserving volume differences (DECISIONS.md:19).
 */
export function applyVelocityLollipopDrag(selectedHits, draggedIndex, nextVelocity) {
  if (!Array.isArray(selectedHits) || selectedHits.length === 0) return [];
  const nextVol = midiVelocityToOsuVolume(nextVelocity);
  if (selectedHits.length === 1) {
    return [{ ...selectedHits[0], volume: nextVol, scope: "per_hit" }];
  }
  const anchor = selectedHits[draggedIndex] ?? selectedHits[0];
  const delta = nextVol - (anchor.volume ?? 100);
  return selectedHits.map(hit => ({
    ...hit,
    volume: Math.max(5, Math.min(100, Math.round((hit.volume ?? 100) + delta))),
    scope: "per_hit",
  }));
}
