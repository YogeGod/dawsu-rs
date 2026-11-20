// Plain logic for the drum detection window, tested under node without a webview.

const OBJECTS = { circle: "Circle", sliderHead: "Slider head", sliderRepeat: "Slider repeat", sliderEnd: "Slider end", spinnerEnd: "Spinner end" };
const LANDS = { ...OBJECTS, grid: "Grid line", flagged: "Too far from the grid", duplicate: "Already there", noLane: "No lane picked" };

export const landsText = lands => LANDS[lands] ?? lands;

// Hits on an object or a grid line start checked; flagged hits start unchecked.
export const checkedByDefault = row => row.lands in OBJECTS || row.lands === "grid";
// A hit already there, or one whose drum has no lane, can't be added.
export const canCheck = row => row.lands !== "duplicate" && row.lands !== "noLane";

// `overrides` holds the mapper's own ticks by row id; every other row keeps its default.
export const isChecked = (row, overrides) =>
  canCheck(row) && (Object.prototype.hasOwnProperty.call(overrides, row.id) ? overrides[row.id] : checkedByDefault(row));

export const ticked = (rows, overrides) => (rows ?? []).filter(row => isChecked(row, overrides)).map(row => row.id);

export function checkLine(rows, overrides) {
  const total = (rows ?? []).filter(canCheck).length;
  return `${ticked(rows, overrides).length} of ${total} ${total === 1 ? "hit" : "hits"} checked`;
}

// After a re-plan, ticks stay for the hits the new review still has (ids are the drum and its frame).
export function keepTicks(overrides, rows) {
  const ids = new Set((rows ?? []).map(row => row.id));
  return Object.fromEntries(Object.entries(overrides).filter(([id]) => ids.has(id)));
}

const pad = (value, width) => String(value).padStart(width, "0");

// mm:ss.mmm, the editor's clock.
export function timeText(ms) {
  const whole = Math.max(0, Math.floor(ms));
  return `${pad(Math.floor(whole / 60000), 2)}:${pad(Math.floor(whole / 1000) % 60, 2)}.${pad(whole % 1000, 3)}`;
}

// A lane reads as its lane header does: its name and number. Hidden lanes have no header number.
export const laneLabel = lane =>
  lane.number == null ? `${lane.name}${lane.hidden ? " (hidden)" : ""}` : `${lane.name} ${lane.number}`;

export const progressText = view =>
  `Detecting drum hits… ${Math.floor((view.done * 100) / Math.max(1, view.total))}%`;

export const sameLane = options => options.kick != null && options.kick === options.snare;

// drum_review's arguments: the window's options with `change` applied, for the map it shows.
export const optionsRequest = (view, change) => ({ map: view.map, options: { ...view.options, ...change } });

export const pollDelay = view => (view?.state === "running" || view?.listening ? 100 : 250);

// Listen starts a second before the picked hit, so the hit is heard in context.
export function listenFrom(rows, picked) {
  const row = (rows ?? []).find(candidate => candidate.id === picked);
  return row ? Math.max(0, row.time - 1000) : 0;
}

// The strip's diamonds: x from 0 to 1 across the song.
export function stripMarks(rows, overrides, duration) {
  if (!(duration > 0)) return [];
  return (rows ?? []).map(row => ({ x: row.time / duration, drum: row.drum, on: isChecked(row, overrides) }));
}

// Keeps the WebView from opening a file dropped on the window.
export function blockDrops(target) {
  const stop = event => event.preventDefault();
  target.addEventListener("dragover", stop);
  target.addEventListener("drop", stop);
  return () => {
    target.removeEventListener("dragover", stop);
    target.removeEventListener("drop", stop);
  };
}
