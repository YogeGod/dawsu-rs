// Phase 5's pure helpers: MIDI import and export, MIDI recording, the MIDI input port kept in
// localStorage, the VST3 scan and the Keysounds panel. Rust does the work (main.rs's midi_*,
// vst3_scan and keysounds_action commands); these only shape what the parts send and show.

// The MIDI input port MIDI Record arms, picked in Preferences › MIDI Input & VST3. This computer only.
export const PORT_KEY = "dawsu.midiInputPort";

// The picked port's index, or null for "the first port" (Rust's default).
export function readPort(storage) {
  try {
    const raw = storage?.getItem?.(PORT_KEY);
    if (raw == null || raw === "") return null;
    const index = Number(raw);
    return Number.isInteger(index) && index >= 0 ? index : null;
  } catch {
    return null;
  }
}

export function writePort(storage, index) {
  try {
    if (index == null || index === "") storage?.removeItem?.(PORT_KEY);
    else storage?.setItem?.(PORT_KEY, String(index));
  } catch {}
}

// apply_midi_import's arguments from the modal's choices.
export function importArgs(info, tracks, { useBeatmapTempo = true, offsetMs = 0, snap = true, followVolume = false } = {}) {
  const offset = Number(offsetMs);
  return {
    path: info.path,
    tracks: [...tracks].sort((a, b) => a - b),
    useBeatmapTempo: !!useBeatmapTempo,
    offsetMs: Number.isFinite(offset) ? offset : 0,
    snap: !!snap,
    followVolume: !!followVolume,
  };
}

const plural = (count, word) => `${count} ${word}${count === 1 ? "" : "s"}`;

export function importToast(outcome) {
  const added = outcome?.added ?? 0;
  const skipped = outcome?.skipped ?? 0;
  return {
    tone: "success",
    title: "MIDI imported",
    body: `Added ${plural(added, "hit")}${skipped ? ` (${skipped} skipped)` : ""}. Press Ctrl+Z to undo.`,
  };
}

// midi_record_toggle's arguments: the picked port and the editor's snap as 1/divisor of a beat
// (a snap of several beats, such as 2/1, has no beat divisor and sends none).
export function recordArgs(portIndex, scene) {
  if (scene?.snap === false) {
    return {
      portIndex: portIndex ?? null,
      snapDivisor: null,
    };
  }
  const divisor = Number(scene?.divisor);
  const beats = Number(scene?.beats ?? 1);
  return {
    portIndex: portIndex ?? null,
    snapDivisor: beats === 1 && Number.isInteger(divisor) && divisor > 0 ? divisor : null,
  };
}

// The toast after a toggle: armed names the port; disarmed says what was recorded.
export function recordToast(status) {
  if (!status) return null;
  if (status.armed) {
    return {
      tone: "info",
      title: "MIDI recording armed",
      body: status.portName ? `Listening on ${status.portName}. Press Space to play and record.` : "No MIDI input port found.",
    };
  }
  const committed = status.committedNotes ?? 0;
  if (!committed) return { tone: "info", title: "MIDI recording stopped", body: "No notes were recorded." };
  return { tone: "success", title: "MIDI recorded", body: `Added ${plural(committed, "hit")}. Press Ctrl+Z to undo.` };
}

// vst3_scan's report in one line (the report's fields are snake_case).
export function scanSummary(report) {
  const plugins = report?.plugins ?? [];
  const skipped = report?.skipped ?? [];
  if (!plugins.length) return NO_PLUGINS + (skipped.length ? ` ${plural(skipped.length, "file")} could not be read.` : "");
  const instruments = plugins.filter(plugin => plugin.is_instrument ?? plugin.isInstrument).length;
  return `Found ${plural(plugins.length, "plug-in")} (${plural(instruments, "instrument")})` +
    (skipped.length ? `, ${skipped.length} skipped.` : ".");
}

export const NO_PLUGINS = "No VST3 plug-ins found. Install one in the system VST3 folder, then scan again.";

// The scanned plug-ins a keysound track may use: instruments only, by name.
export function instrumentChoices(plugins) {
  return (plugins ?? []).filter(plugin => plugin.isInstrument ?? plugin.is_instrument)
    .map(plugin => ({ id: plugin.id ?? plugin.uid, name: plugin.name, vendor: plugin.vendor ?? "" }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

// keysounds_action payloads.
export const keysoundsView = () => ({ kind: "view" });
export const keysoundsScan = () => ({ kind: "scan_vst3" });
export function keysoundsAdd(name, plugin) {
  const payload = { kind: "add_track", name: String(name ?? "").trim() || "Keysound Track" };
  if (plugin) payload.vst3Plugin = plugin;
  return payload;
}
export const keysoundsDelete = trackId => ({ kind: "delete_track", trackId });
// Rust trims the name too and rejects an empty one; an empty one is not sent.
export function keysoundsRenameTrack(trackId, name) {
  const trimmed = String(name ?? "").trim();
  return trackId == null || !trimmed ? null : { kind: "rename_track", trackId, name: trimmed };
}
// Snaps every note start with the editor's own snap setting (Rust reads it).
export const keysoundsSnapToGrid = trackId => ({ kind: "snap_notes_to_grid", trackId });

// Piano roll payloads. Rust reads pitch, velocity, noteIndex and semitones as whole numbers (a
// fraction falls back to its default), so they are rounded here.
const whole = (value, low, high) => Math.min(high, Math.max(low, Math.round(Number(value) || 0)));
const ms = value => Math.max(0, Math.round(Number(value) || 0));
export const KEYSOUND_NOTE_MS = 250;
export const QUANTIZE_MS = 125;
export const keysoundsAddNote = (trackId, startMs, pitch, velocity = 100, lengthMs = KEYSOUND_NOTE_MS) =>
  ({ kind: "add_note", trackId, startMs: ms(startMs), endMs: ms(startMs) + ms(lengthMs), pitch: whole(pitch, 0, 127), velocity: whole(velocity, 1, 127) });
export const keysoundsMoveNote = (trackId, noteIndex, startMs, pitch) =>
  ({ kind: "move_note", trackId, noteIndex, startMs: ms(startMs), pitch: whole(pitch, 0, 127) });
export const keysoundsResizeNote = (trackId, noteIndex, endMs) => ({ kind: "resize_note", trackId, noteIndex, endMs: ms(endMs) });
export const keysoundsDeleteNote = (trackId, noteIndex) => ({ kind: "delete_note", trackId, noteIndex });
export const keysoundsDeleteNotes = (trackId, noteIndices) => ({ kind: "delete_notes", trackId, noteIndices });
export const keysoundsSetVelocity = (trackId, noteIndex, velocity) =>
  ({ kind: "set_velocity", trackId, noteIndex, velocity: whole(velocity, 1, 127) });
export const keysoundsQuantize = (trackId, stepMs = QUANTIZE_MS) => ({ kind: "quantize_notes", trackId, stepMs });
export const keysoundsTranspose = (trackId, semitones) => ({ kind: "transpose_notes", trackId, semitones: whole(semitones, -127, 127) });
export const keysoundsPreview = (trackId, pitch, velocity = 100, durationMs = 200) =>
  ({ kind: "preview_note", trackId, pitch: whole(pitch, 0, 127), velocity: whole(velocity, 1, 127), durationMs: ms(durationMs) });
export const keysoundsGenerate = () => ({ kind: "generate_keysounds" });
export const keysoundsToggleMute = trackId => ({ kind: "toggle_mute", trackId });
export const keysoundsToggleSolo = trackId => ({ kind: "toggle_solo", trackId });
export const keysoundsSyncToObjects = (trackId, maxShiftMs = 50) => ({ kind: "sync_to_objects", trackId, maxShiftMs: ms(maxShiftMs) });
export const keysoundsTrimOverlaps = trackId => ({ kind: "trim_overlaps", trackId });
export const keysoundsStrum = (trackId, spreadMs = 20, direction = "up", chordToleranceMs = 15) => ({
  kind: "strum_notes",
  trackId,
  spreadMs: Math.max(1, Math.round(Number(spreadMs) || 20)),
  direction: direction === "down" || direction === "alternate" ? direction : "up",
  chordToleranceMs: Math.max(0, Math.round(Number(chordToleranceMs) || 15)),
});
export const keysoundsHumanise = (trackId, timingMs = 10, velocityRange = 12, seed = null) => {
  const payload = {
    kind: "humanise_notes",
    trackId,
    timingMs: Math.max(0, Math.round(Number(timingMs) || 0)),
    velocityRange: whole(velocityRange, 0, 64),
  };
  if (seed != null && Number.isFinite(Number(seed))) payload.seed = Math.max(0, Math.round(Number(seed)));
  return payload;
};
export const keysoundsVelocityRamp = (trackId, startMs, endMs, startVelocity, endVelocity, curve = 0) => ({
  kind: "velocity_ramp",
  trackId,
  startMs: ms(startMs),
  endMs: ms(endMs),
  startVelocity: whole(startVelocity, 1, 127),
  endVelocity: whole(endVelocity, 1, 127),
  curve: Number.isFinite(Number(curve)) ? Math.max(-4, Math.min(4, Number(curve))) : 0,
});
// load_vst3 takes the scanned plug-in's file and id (summary.vst3Plugins), or null when none is picked.
export function keysoundsLoadVst3(trackId, plugin) {
  if (trackId == null || !plugin?.path) return null;
  return { kind: "load_vst3", trackId, pluginPath: plugin.path, pluginUid: plugin.id ?? null, pluginName: plugin.name ?? "VST3 Instrument" };
}

export function generateToast(generated) {
  if (!generated) return null;
  const unsynced = generated.unsyncedCount ?? 0;
  return {
    tone: "success",
    title: "[Keysounds] generated",
    body: `Wrote ${generated.fileName} with ${plural(generated.syncedCount ?? 0, "synced note")}` +
      (unsynced ? ` (${unsynced} not on an object).` : "."),
  };
}

// The roll's geometry: C2 (36) at the bottom to C6 (84) at the top, one row per semitone.
export const ROLL_LOW = 36;
export const ROLL_HIGH = 84;
export const ROW_PX = 14;
export const PX_PER_MS = 0.16;
const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
export const SCALE_ROOTS = NAMES;
export const SCALE_DEFS = [
  { id: "chromatic", label: "Chromatic", intervals: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] },
  { id: "major", label: "Major", intervals: [0, 2, 4, 5, 7, 9, 11] },
  { id: "natural_minor", label: "Minor", intervals: [0, 2, 3, 5, 7, 8, 10] },
  { id: "harmonic_minor", label: "Harmonic Minor", intervals: [0, 2, 3, 5, 7, 8, 11] },
  { id: "melodic_minor", label: "Melodic Minor", intervals: [0, 2, 3, 5, 7, 9, 11] },
  { id: "dorian", label: "Dorian", intervals: [0, 2, 3, 5, 7, 9, 10] },
  { id: "mixolydian", label: "Mixolydian", intervals: [0, 2, 4, 5, 7, 9, 10] },
  { id: "pentatonic_major", label: "Pentatonic Major", intervals: [0, 2, 4, 7, 9] },
  { id: "pentatonic_minor", label: "Pentatonic Minor", intervals: [0, 3, 5, 7, 10] },
  { id: "blues", label: "Blues", intervals: [0, 3, 5, 6, 7, 10] },
];
export function scalePitchMask(root = 0, scaleId = "chromatic") {
  const def = SCALE_DEFS.find(s => s.id === scaleId) ?? SCALE_DEFS[0];
  const r = ((Math.round(Number(root) || 0) % 12) + 12) % 12;
  let mask = 0;
  for (const interval of def.intervals) mask |= 1 << ((r + interval) % 12);
  return mask;
}
export function isPitchInScale(pitch, root = 0, scaleId = "chromatic") {
  const pc = ((pitch % 12) + 12) % 12;
  return ((scalePitchMask(root, scaleId) >> pc) & 1) === 1;
}
export function visibleRollPitches(root = 0, scaleId = "chromatic", fold = false) {
  const base = rollPitches();
  if (!fold || scaleId === "chromatic") return base;
  const mask = scalePitchMask(root, scaleId);
  return base.filter(p => ((mask >> (((p % 12) + 12) % 12)) & 1) === 1);
}
export const pitchName = pitch => `${NAMES[pitch % 12]}${Math.floor(pitch / 12) - 1}`;
export const isBlackKey = pitch => NAMES[pitch % 12].length > 1;
export const rollPitches = () => Array.from({ length: ROLL_HIGH - ROLL_LOW + 1 }, (_, i) => ROLL_HIGH - i);
export function pitchRow(pitch, visiblePitches = null) {
  if (Array.isArray(visiblePitches) && visiblePitches.length) {
    const idx = visiblePitches.indexOf(pitch);
    return idx >= 0 ? idx * ROW_PX : -1;
  }
  return (ROLL_HIGH - pitch) * ROW_PX;
}
export function pitchAtY(y, visiblePitches = null) {
  if (Array.isArray(visiblePitches) && visiblePitches.length) {
    const idx = whole(Math.floor(y / ROW_PX), 0, visiblePitches.length - 1);
    return visiblePitches[idx];
  }
  return whole(ROLL_HIGH - Math.floor(y / ROW_PX), ROLL_LOW, ROLL_HIGH);
}
// The time under x, snapped down to the grid step.
export const msAtX = (x, stepMs = QUANTIZE_MS) => Math.max(0, Math.floor(x / PX_PER_MS / stepMs) * stepMs);
// The roll's width in ms: past the last note's end by two seconds, at least eight seconds.
export const rollSpanMs = notes => Math.max(8000, ...(notes ?? []).map(note => note.endMs + 2000));
// Velocity from a point in the velocity lane (top = 127).
export const velocityAtY = (y, height) => whole(127 * (1 - y / Math.max(1, height)), 1, 127);

function nearestPitchIndex(visiblePitches, pitch) {
  const exact = visiblePitches.indexOf(pitch);
  if (exact >= 0) return exact;
  let best = 0;
  let bestDist = Infinity;
  for (let i = 0; i < visiblePitches.length; i++) {
    const dist = Math.abs(visiblePitches[i] - pitch);
    if (dist < bestDist) {
      bestDist = dist;
      best = i;
    }
  }
  return best;
}

// What a finished note drag sends: a move snaps start and pitch to the grid, a resize the end.
// Null when the note did not change (a plain click selects it).
// `pxPerMs` and `rowPx` are the roll's scale: the editor's zoom and the roll's row height.
// `snap` (rollSnap.js noteSnap) replaces the fixed `stepMs` grid with the editor's own Snap.
export function noteDragPayload(trackId, note, mode, dx, dy, stepMs = QUANTIZE_MS, visiblePitches = null,
  pxPerMs = PX_PER_MS, rowPx = ROW_PX, snap = null) {
  const nearest = snap ? snap.nearest : ms => Math.round(ms / stepMs) * stepMs;
  if (mode === "resize") {
    if (!dx) return null;
    const shortest = snap ? snap.minLength(note.startMs) : stepMs;
    const endMs = Math.max(note.startMs + shortest, nearest(note.endMs + dx / pxPerMs));
    return endMs === note.endMs ? null : keysoundsResizeNote(trackId, note.index, endMs);
  }
  if (!dx && !dy) return null;
  const startMs = Math.max(0, nearest(note.startMs + dx / pxPerMs));
  let pitch;
  if (Array.isArray(visiblePitches) && visiblePitches.length) {
    const curIdx = nearestPitchIndex(visiblePitches, note.pitch);
    const nextIdx = whole(curIdx + Math.round(dy / rowPx), 0, visiblePitches.length - 1);
    pitch = visiblePitches[nextIdx];
  } else {
    pitch = whole(note.pitch - Math.round(dy / rowPx), ROLL_LOW, ROLL_HIGH);
  }
  if (startMs === note.startMs && pitch === note.pitch) return null;
  return keysoundsMoveNote(trackId, note.index, startMs, pitch);
}

// The note's index after Rust re-sorted the track: the one at this start and pitch.
export function findNoteIndex(notes, startMs, pitch) {
  const found = (notes ?? []).find(note => note.pitch === pitch && Math.abs(note.startMs - startMs) < 1);
  return found ? found.index : null;
}

// The Rust host sidecar's state in one line.
export function sidecarStatus(summary) {
  if (!summary) return "";
  const loaded = summary.loadedPlugins?.length ?? 0;
  if (!summary.sidecarAvailable) return "VST3 host not found: notes preview with the built-in tone.";
  return loaded ? `VST3 host ready · ${plural(loaded, "instrument")} loaded.` : "VST3 host ready · no instrument loaded.";
}
