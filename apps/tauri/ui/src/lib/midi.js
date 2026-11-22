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
