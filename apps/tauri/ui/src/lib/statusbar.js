// Pure helpers for the editor's bottom status bar.

export const PLAYBACK_SPEEDS = [0.25, 0.5, 0.75, 1, 1.25, 1.5];
export const ZOOM_MIN = -3.301;
export const ZOOM_MAX = 0.477;
export const ZOOM_STEP = 0.01;

export function nameOf(path) {
  const parts = String(path).split(/[\\/]/);
  return parts[parts.length - 1] ?? "";
}

// The file name is cut in the middle: the head shrinks with an ellipsis, the tail ("[Hard].osu") stays.
export function splitFileName(path) {
  if (!path) return { head: "", tail: "" };
  const name = nameOf(path);
  const match = /\[[^\]]*\]\.osu$/i.exec(name);
  const tail = match ? match[0] : name.slice(-Math.min(name.length, 12));
  return { head: name.slice(0, name.length - tail.length), tail };
}

export function cannotSaveLabel(count) {
  return count + (count === 1 ? " moment" : " moments") + " can't be saved";
}

export function problemTooltip(problems) {
  const messages = (problems ?? []).map(problem => problem.message).filter(Boolean);
  if (!messages.length) return "";
  const lines = messages.slice(0, 10);
  if (messages.length > 10) lines.push("and " + (messages.length - 10) + " more");
  return lines.join("\n");
}

export function selectionLabel(count) {
  return count + (count === 1 ? " hit selected" : " hits selected");
}

export function hitLaneCounts(scene) {
  const hits = (scene?.moments ?? []).reduce((sum, moment) => sum + (moment[3]?.length ?? 0), 0);
  // Shown lanes only: a removed (hidden) lane keeps its model row but no header.
  const lanes = scene?.laneOrder?.length ?? scene?.lanes?.length ?? 0;
  return hits + (hits === 1 ? " hit · " : " hits · ") + lanes + (lanes === 1 ? " lane" : " lanes");
}

export function statusText(notice, view) {
  return notice || view?.fault || view?.status || (view?.busy ? "Working…" : "");
}

export function audioLabel(device) {
  const name = device || "";
  return {
    text: name || "Audio output",
    ariaLabel: name ? "Audio output: " + name : "Audio output",
  };
}

export function zoomSliderValue(zoom) {
  return Number.isFinite(zoom) && zoom > 0 ? Math.log10(zoom) : 0;
}
