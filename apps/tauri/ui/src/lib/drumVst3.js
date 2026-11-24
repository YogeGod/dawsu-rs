// Drum Map <-> VST3 drum instrument mapping, localStorage persistence (`dawsu.drumVst3`),
// and keysounds_action IPC routing for using VST3 drum sounds with a drum map.
import { keysoundsAdd, keysoundsAddNote, keysoundsDelete, keysoundsLoadVst3, keysoundsPreview, keysoundsView } from "./midi.js";

export const DRUM_VST3_KEY = "dawsu.drumVst3";
export const DRUM_MAP_TRACK_NAME = "Drum Map (VST3)";
export const DRUM_NOTE_MS = 150;

// Standard General MIDI (GM) drum note numbers (channel 10 / 0-indexed 9).
export const GM_DRUM_NOTES = Object.freeze({
  kick: 36,
  rim: 37,
  snare: 38,
  clap: 39,
  closedHiHat: 42,
  pedalHiHat: 44,
  lowTom: 45,
  openHiHat: 46,
  midTom: 47,
  crash: 49,
  highTom: 50,
  ride: 51,
  percussion: 39,
});

const DEFAULT_CONFIG = Object.freeze({
  enabled: false,
  pluginId: "",
  pluginName: "",
  pluginPath: "",
});

// Maps a drum map row (object with `{ name, midiNote }` or a plain string name) to a GM MIDI note.
// Explicit valid `midiNote` (0..127) on a row wins; otherwise matches standard drum map row names.
export function drumRowToMidiNote(rowOrName) {
  if (rowOrName && typeof rowOrName === "object") {
    if (Number.isInteger(rowOrName.midiNote) && rowOrName.midiNote >= 0 && rowOrName.midiNote <= 127) {
      return rowOrName.midiNote;
    }
    return drumRowToMidiNote(rowOrName.name ?? rowOrName.drum ?? "");
  }
  const text = String(rowOrName ?? "").trim().toLowerCase().replace(/[_/]+/g, " ");
  if (!text) return GM_DRUM_NOTES.percussion;

  // Order matters: check specific phrases before general ones.
  if (/\b(rim|rimshot|cross[\s-]*stick|side[\s-]*stick)\b/.test(text)) return GM_DRUM_NOTES.rim;
  if (/\bclap\b/.test(text)) return GM_DRUM_NOTES.clap;
  if (/\b(kick|bd|bass[\s-]*drum)\b/.test(text)) return GM_DRUM_NOTES.kick;
  if (/\b(snare|sd)\b/.test(text)) return GM_DRUM_NOTES.snare;
  if (/\bpedal\b.*\b(hat|hh)\b|\b(hat|hh)\b.*\bpedal\b/.test(text)) return GM_DRUM_NOTES.pedalHiHat;
  if (/\bopen\b.*\b(hat|hh)\b|\b(hat|hh)\b.*\bopen\b|\bohh\b/.test(text)) return GM_DRUM_NOTES.openHiHat;
  if (/\b(closed\b.*\b(hat|hh)|hi[\s-]*hat|hh|chh|hat)\b/.test(text)) return GM_DRUM_NOTES.closedHiHat;
  if (/\b(high|hi)\b.*\btom\b|\btom\b.*\b(high|hi|1)\b/.test(text)) return GM_DRUM_NOTES.highTom;
  if (/\b(low|floor)\b.*\btom\b|\btom\b.*\b(low|floor|3)\b/.test(text)) return GM_DRUM_NOTES.lowTom;
  if (/\b(mid\b.*\btom|tom)\b/.test(text)) return GM_DRUM_NOTES.midTom;
  if (/\b(crash|china|splash)\b/.test(text)) return GM_DRUM_NOTES.crash;
  if (/\b(ride|bell)\b/.test(text)) return GM_DRUM_NOTES.ride;
  return GM_DRUM_NOTES.percussion;
}

// Attaches resolved GM `midiNote` to each drum map row.
export function mapDrumRowsToMidi(rows) {
  return (rows ?? []).map((row, index) => ({
    index: row.index ?? index,
    name: row.name ?? "",
    laneKey: row.laneKey ?? null,
    assigned: Boolean(row.assigned),
    midiNote: drumRowToMidiNote(row),
  }));
}

// Reads the persisted Drum Map VST3 toggle and selected plugin from localStorage (`dawsu.drumVst3`).
export function readDrumVst3(storage) {
  try {
    const raw = storage?.getItem?.(DRUM_VST3_KEY);
    if (!raw) return { ...DEFAULT_CONFIG };
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object") return { ...DEFAULT_CONFIG };
    return {
      enabled: Boolean(parsed.enabled),
      pluginId: typeof parsed.pluginId === "string" ? parsed.pluginId : "",
      pluginName: typeof parsed.pluginName === "string" ? parsed.pluginName : "",
      pluginPath: typeof parsed.pluginPath === "string" ? parsed.pluginPath : "",
    };
  } catch {
    return { ...DEFAULT_CONFIG };
  }
}

// Writes the Drum Map VST3 toggle and selected plugin to localStorage (`dawsu.drumVst3`).
export function writeDrumVst3(storage, config) {
  const normalized = {
    enabled: Boolean(config?.enabled),
    pluginId: typeof config?.pluginId === "string" ? config.pluginId : "",
    pluginName: typeof config?.pluginName === "string" ? config.pluginName : "",
    pluginPath: typeof config?.pluginPath === "string" ? config.pluginPath : "",
  };
  try {
    storage?.setItem?.(DRUM_VST3_KEY, JSON.stringify(normalized));
  } catch {}
  return normalized;
}

// Normalizes scanned VST3 plugins (from either `vst3_scan` report or `keysounds_action` summary)
// into sorted instrument choices `{ id, name, vendor, path }`.
export function vst3InstrumentOptions(reportOrPlugins) {
  const list = Array.isArray(reportOrPlugins) ? reportOrPlugins : (reportOrPlugins?.plugins ?? []);
  return list
    .filter(plugin => Boolean(plugin?.is_instrument ?? plugin?.isInstrument))
    .map(plugin => ({
      id: String(plugin.uid ?? plugin.id ?? plugin.path ?? plugin.name ?? ""),
      name: String(plugin.name ?? "VST3 Instrument"),
      vendor: String(plugin.vendor ?? ""),
      path: String(plugin.path ?? ""),
    }))
    .filter(choice => choice.id !== "")
    .sort((a, b) => a.name.localeCompare(b.name));
}

// Converts an osu! hit volume (0..100) or detection strength (0..1) into a MIDI velocity (1..127).
export function volumeToVelocity(volume) {
  const value = Number(volume);
  if (!Number.isFinite(value)) return 100;
  const scaled = value <= 1 ? value * 127 : (value / 100) * 127;
  return Math.min(127, Math.max(1, Math.round(scaled)));
}

// Builds deduplicated MIDI drum hits `{ startMs, pitch, velocity }` from Drum Detect rows + overrides.
export function detectedRowsToDrumNotes(rows, overrides = {}, isCheckedFn = null) {
  const notes = [];
  const seen = new Set();
  for (const row of rows ?? []) {
    const checked = isCheckedFn ? isCheckedFn(row, overrides) : (overrides[row.id] ?? true);
    if (!checked) continue;
    const startMs = Math.max(0, Math.round(Number(row.time) || 0));
    const pitch = drumRowToMidiNote(row.drum ?? row.name);
    const key = `${startMs}:${pitch}`;
    if (seen.has(key)) continue;
    seen.add(key);
    notes.push({ startMs, pitch, velocity: volumeToVelocity(row.strength ?? 0.85) });
  }
  return notes.sort((a, b) => a.startMs - b.startMs || a.pitch - b.pitch);
}

// Builds deduplicated MIDI drum hits `{ startMs, pitch, velocity }` from the editor's active scene
// (`scene.moments` + `scene.drumRows` / `drumView.rows`).
export function sceneDrumHitsToNotes(scene, drumView) {
  const rowToPitch = new Map();
  for (const entry of scene?.drumRows ?? []) {
    if (typeof entry?.row !== "number" || !entry?.name) continue;
    // A combined label like "Kick / Kick 2" uses the primary drum piece name.
    const primaryName = String(entry.name).split("/")[0].trim();
    rowToPitch.set(entry.row, drumRowToMidiNote(primaryName));
  }
  if (!rowToPitch.size && drumView?.rows?.length && scene?.lanes?.length) {
    const laneKeyToRow = new Map();
    for (let i = 0; i < scene.lanes.length; i++) {
      const lane = scene.lanes[i];
      if (lane?.sourceKey) laneKeyToRow.set(lane.sourceKey, i);
      if (lane?.key) laneKeyToRow.set(lane.key, i);
    }
    for (const row of drumView.rows) {
      if (!row?.assigned || !row?.laneKey) continue;
      const sceneRow = laneKeyToRow.get(row.laneKey);
      if (sceneRow != null && !rowToPitch.has(sceneRow)) {
        rowToPitch.set(sceneRow, drumRowToMidiNote(row));
      }
    }
  }
  if (!rowToPitch.size) return [];

  const notes = [];
  const seen = new Set();
  for (const moment of scene?.moments ?? []) {
    const [time, volume, , hits] = moment;
    const startMs = Math.max(0, Math.round(Number(time) || 0));
    const velocity = volumeToVelocity(volume ?? 80);
    for (const hit of hits ?? []) {
      const laneRow = Array.isArray(hit) ? hit[0] : hit?.row;
      const pitch = rowToPitch.get(laneRow);
      if (pitch == null) continue;
      const key = `${startMs}:${pitch}`;
      if (seen.has(key)) continue;
      seen.add(key);
      notes.push({ startMs, pitch, velocity });
    }
  }
  return notes.sort((a, b) => a.startMs - b.startMs || a.pitch - b.pitch);
}

// Creates or updates the Drum Map VST3 keysound track via `keysounds_action` IPC, loads the
// chosen VST3 instrument when available, and populates mapped GM MIDI drum hits.
export async function syncDrumVst3Track(invoke, { plugin = null, hits = [], trackName = DRUM_MAP_TRACK_NAME } = {}) {
  if (typeof invoke !== "function") return null;
  let summary = await invoke("keysounds_action", { payload: keysoundsView() });
  const existing = (summary?.tracks ?? []).find(track => track.name === trackName) ?? null;

  let trackId = existing?.id ?? null;
  if (existing && hits.length > 0) {
    summary = await invoke("keysounds_action", { payload: keysoundsDelete(existing.id) });
    trackId = null;
  }

  if (trackId == null) {
    const before = new Set((summary?.tracks ?? []).map(track => track.id));
    summary = await invoke("keysounds_action", { payload: keysoundsAdd(trackName, plugin?.name || null) });
    trackId = (summary?.tracks ?? []).find(track => !before.has(track.id))?.id
      ?? (summary?.tracks ?? []).find(track => track.name === trackName)?.id
      ?? null;
  }

  if (trackId != null && plugin?.path && summary?.sidecarAvailable) {
    const loadPayload = keysoundsLoadVst3(trackId, {
      id: plugin.id || plugin.pluginId || null,
      name: plugin.name || plugin.pluginName || "VST3 Instrument",
      path: plugin.path || plugin.pluginPath,
    });
    if (loadPayload) {
      summary = await invoke("keysounds_action", { payload: loadPayload });
    }
  }

  let addedNotes = 0;
  if (trackId != null) {
    for (const hit of hits) {
      summary = await invoke("keysounds_action", {
        payload: keysoundsAddNote(trackId, hit.startMs, hit.pitch, hit.velocity ?? 100, DRUM_NOTE_MS),
      });
      addedNotes++;
    }
  }

  return {
    trackId,
    addedNotes,
    pluginName: plugin?.name || plugin?.pluginName || "",
    summary,
  };
}

// Auditions a single drum map row through the active VST3 preview path.
export async function previewDrumVst3Hit(invoke, rowOrName, velocity = 100) {
  if (typeof invoke !== "function") return null;
  const pitch = drumRowToMidiNote(rowOrName);
  await invoke("keysounds_action", { payload: keysoundsPreview(0, pitch, velocity, 200) });
  return pitch;
}
