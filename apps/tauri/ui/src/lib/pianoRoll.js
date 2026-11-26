// Pure <canvas> Piano Roll coordinate, local drag preview, and CC64 sustain helpers (K2).

export const MIN_PITCH = 36; // C2
export const MAX_PITCH = 96; // C7
export const DEFAULT_LOW_PITCH = 60; // C4
export const DEFAULT_HIGH_PITCH = 84; // C6

const BLACK_KEYS = new Set([1, 3, 6, 8, 10]);

export function isBlackKey(pitch) {
  return BLACK_KEYS.has(((pitch % 12) + 12) % 12);
}

export function pitchToY(pitch, topPitch, rowHeight) {
  return (topPitch - pitch) * rowHeight;
}

export function yToPitch(y, topPitch, rowHeight) {
  const step = Math.floor(y / Math.max(1, rowHeight));
  return Math.max(MIN_PITCH, Math.min(MAX_PITCH, topPitch - step));
}

/**
 * Applies a local drag preview delta to a note without sending IPC until pointerup (M3).
 */
export function previewDraggedNote(note, deltaMs, deltaSemitones, mode = "move") {
  if (!note) return null;
  if (mode === "resize") {
    return { ...note, endMs: Math.max(note.startMs + 20, note.endMs + deltaMs) };
  }
  const dur = Math.max(20, note.endMs - note.startMs);
  const startMs = Math.max(0, note.startMs + deltaMs);
  const pitch = Math.max(0, Math.min(127, Math.round(note.pitch + deltaSemitones)));
  return { ...note, startMs, endMs: startMs + dur, pitch };
}

/** Hit-tests CC64 sustain spans at `timeMs`. */
export function sustainSpanAt(spans, timeMs) {
  if (!Array.isArray(spans)) return null;
  return spans.find(s => timeMs >= s.start_ms && timeMs <= s.end_ms) ?? null;
}

/** Evaluates a curved velocity ramp for live Shift+drag preview (mirrors Rust apply_velocity_ramp). */
export function sampleVelocityRamp(timeMs, startMs, endMs, startVel, endVel, curve = 0) {
  if (!Number.isFinite(timeMs) || !Number.isFinite(startMs) || !Number.isFinite(endMs)) return null;
  const clampVel = v => Math.min(127, Math.max(1, Math.round(Number(v) || 1)));
  const [t0, t1, v0, v1] = startMs <= endMs
    ? [startMs, endMs, clampVel(startVel), clampVel(endVel)]
    : [endMs, startMs, clampVel(endVel), clampVel(startVel)];
  if (timeMs < t0 || timeMs > t1) return null;
  const c = Number.isFinite(Number(curve)) ? Math.max(-4, Math.min(4, Number(curve))) : 0;
  const exponent = Math.pow(2, c);
  const span = t1 - t0;
  const t = Math.abs(span) < 1e-6 ? 1 : Math.min(1, Math.max(0, (timeMs - t0) / span));
  const shaped = Math.pow(t, exponent);
  return clampVel(v0 + (v1 - v0) * shaped);
}
