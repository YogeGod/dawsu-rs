// Live keyboard lane tapping (F028) and timeline tap-recording (F029) helpers.
import { recordToast } from "./midi.js";

export const NUMPAD_TAP_CODES = [
  "Numpad1", "Numpad2", "Numpad3",
  "Numpad4", "Numpad5", "Numpad6",
  "Numpad7", "Numpad8", "Numpad9",
];

export const HOME_ROW_TAP_CODES = [
  "KeyA", "KeyS", "KeyD",
  "KeyF", "KeyG", "KeyH",
  "KeyJ", "KeyK", "KeyL",
];

// Maps a keyboard event to visible lane slot 0..8, or null if the key is not a live tap.
// Numpad 1–9 always play lanes 1–9 live; home row A–L plays & records lanes 1–9 when tap recording is armed.
export function tapLaneSlotForKey({ code, ctrl = false, shift = false, alt = false } = {}, tapRecordingArmed = false) {
  if (ctrl || shift || alt || !code) return null;
  const numpadIndex = NUMPAD_TAP_CODES.indexOf(code);
  if (numpadIndex !== -1) return numpadIndex;
  if (tapRecordingArmed) {
    const homeIndex = HOME_ROW_TAP_CODES.indexOf(code);
    if (homeIndex !== -1) return homeIndex;
  }
  return null;
}

// Resolves visible lane slot `0..8` to the backend model row index from `state.laneRows`.
export function resolveVisibleTapRow(laneRows, slotIndex) {
  if (!Array.isArray(laneRows) || !Number.isInteger(slotIndex) || slotIndex < 0) return null;
  const visibleRows = laneRows.filter((row) => Number.isInteger(row) && row >= 0);
  return visibleRows[slotIndex] ?? null;
}

// Enriches TransportBar's MIDI/Tap Record toast when no hardware MIDI port is connected.
export function tapAwareRecordToast(status) {
  if (!status) return null;
  if (status.armed && !status.portName) {
    return {
      tone: "info",
      title: "Tap recording armed",
      body: "Press Space to play and tap A–L or Numpad 1–9. Click Record again to commit as one Undo step.",
    };
  }
  return recordToast(status);
}

// Plain-English toast for File › Reload in osu! and Save auto-reload.
export function osuReloadToast(result) {
  if (!result) return null;
  if (result.status === "reloaded_stable") {
    return { tone: "success", title: "Reloaded in osu!", body: result.message };
  }
  if (result.status === "lazer_external_edit") {
    return { tone: "info", title: "osu!lazer external edit", body: result.message };
  }
  return { tone: "warning", title: "osu! was not reloaded", body: result.message };
}

// The same warning when the reload_osu_editor call itself was rejected.
export function osuReloadFailedToast(error) {
  return { tone: "warning", title: "osu! was not reloaded", body: String(error?.message ?? error) };
}
