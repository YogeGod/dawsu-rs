// Musical Typing: computer keyboard as a live MIDI keyboard for the Keysounding Piano Roll.
// Ported from dawsu v3.1.4 KeysoundEditor.Typing.cs.

export const MIN_BASE_PITCH = 24; // C1
export const MAX_BASE_PITCH = 96; // C7
export const DEFAULT_BASE_PITCH = 60; // C4
export const DEFAULT_VELOCITY = 100;
export const VELOCITY_STEP = 10;

// Maps KeyboardEvent.code to semitone offset above basePitch.
// Home row (A..') plays white keys; top row (W, E, T, Y, U, O, P) plays black keys.
export const KEY_OFFSETS = Object.freeze({
  KeyA: 0,       // C
  KeyW: 1,       // C#
  KeyS: 2,       // D
  KeyE: 3,       // D#
  KeyD: 4,       // E
  KeyF: 5,       // F
  KeyT: 6,       // F#
  KeyG: 7,       // G
  KeyY: 8,       // G#
  KeyH: 9,       // A
  KeyU: 10,      // A#
  KeyJ: 11,      // B
  KeyK: 12,      // C+1
  KeyO: 13,      // C#+1
  KeyL: 14,      // D+1
  KeyP: 15,      // D#+1
  Semicolon: 16, // E+1
  Quote: 17,     // F+1
});

export function createMusicalTypingState() {
  return {
    enabled: false,
    basePitch: DEFAULT_BASE_PITCH,
    velocity: DEFAULT_VELOCITY,
  };
}

export function clampBasePitch(pitch) {
  const parsed = Number(pitch);
  const n = Number.isFinite(parsed) ? Math.round(parsed) : DEFAULT_BASE_PITCH;
  return Math.max(MIN_BASE_PITCH, Math.min(MAX_BASE_PITCH, n));
}

export function clampVelocity(velocity) {
  const parsed = Number(velocity);
  const n = Number.isFinite(parsed) ? Math.round(parsed) : DEFAULT_VELOCITY;
  return Math.max(1, Math.min(127, n));
}

export function pitchForKeyCode(code, basePitch = DEFAULT_BASE_PITCH) {
  if (!Object.prototype.hasOwnProperty.call(KEY_OFFSETS, code)) return null;
  const pitch = clampBasePitch(basePitch) + KEY_OFFSETS[code];
  return Math.max(0, Math.min(127, pitch));
}

export function octaveLabel(basePitch = DEFAULT_BASE_PITCH) {
  const octave = Math.floor(clampBasePitch(basePitch) / 12) - 1;
  return `C${octave}`;
}

export function handleMusicalTypingKeyDown(state, heldCodes, event) {
  if (!state?.enabled || !event) return { action: null, state };
  if (event.ctrlKey || event.metaKey || event.altKey) return { action: null, state };

  const code = event.code || "";
  if (event.key === "Escape" || code === "Escape") {
    heldCodes?.clear?.();
    return { action: "off", state: { ...state, enabled: false } };
  }

  if (code === "KeyZ") {
    const basePitch = clampBasePitch(state.basePitch - 12);
    return { action: "octave", basePitch, state: { ...state, basePitch } };
  }
  if (code === "KeyX") {
    const basePitch = clampBasePitch(state.basePitch + 12);
    return { action: "octave", basePitch, state: { ...state, basePitch } };
  }
  if (code === "KeyC") {
    const velocity = clampVelocity(state.velocity - VELOCITY_STEP);
    return { action: "velocity", velocity, state: { ...state, velocity } };
  }
  if (code === "KeyV") {
    const velocity = clampVelocity(state.velocity + VELOCITY_STEP);
    return { action: "velocity", velocity, state: { ...state, velocity } };
  }

  const pitch = pitchForKeyCode(code, state.basePitch);
  if (pitch == null) return { action: null, state };

  if (event.repeat || heldCodes?.has?.(code)) {
    return { action: "repeat", pitch, velocity: state.velocity, state };
  }
  heldCodes?.add?.(code);
  return { action: "note", pitch, velocity: state.velocity, state };
}

export function handleMusicalTypingKeyUp(heldCodes, code) {
  if (!heldCodes || !code) return false;
  return heldCodes.delete(code);
}
