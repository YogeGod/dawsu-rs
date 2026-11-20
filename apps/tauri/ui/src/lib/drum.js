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

// drum_review's arguments: the editor's options with every change not yet answered (`pending`)
// and `change` applied. One lane for both drums is refused here and nothing is sent.
export function reviewRequest(view, pending, change) {
  const next = { ...pending, ...change };
  const options = { ...view.options, ...next };
  if (sameLane(options)) return { error: "Pick different lanes for kicks and snares.", pending };
  return { pending: next, request: { map: view.map, options } };
}

// The pending changes the editor's options don't hold yet.
export const settle = (pending, view) =>
  Object.fromEntries(Object.entries(pending).filter(([key, value]) => view?.options?.[key] !== value));

// The window's own state beside the editor's answer (`view`): the rows of the review it shows,
// the mapper's ticks, the picked row, the sources started for this map, and the chosen source.
export const initialState = view => take({ view: null, rows: null, overrides: {}, picked: null, started: [], source: null, stemPath: null }, view);

// Drum stem when the map has no song; otherwise what the editor analyses, or the song.
const sourceFor = view => view.source?.kind ?? (view.hasSong === false ? "stem" : "song");

// The state after an answer. Rows come only when the review changed; ticks stay for the hits the
// new review still has. Another map starts over.
export function take(state, next) {
  if (!next) return state;
  let { rows, overrides, picked, started, source, stemPath } = state;
  if (next.map !== state.view?.map) {
    rows = null;
    overrides = {};
    picked = null;
    started = [];
    source = sourceFor(next);
    stemPath = next.source?.kind === "stem" ? next.source.path : null;
  } else if (source === null) {
    source = sourceFor(next);
  }
  if (next.rows) {
    rows = next.rows;
    overrides = keepTicks(overrides, next.rows);
  } else if (next.state !== "ready") {
    rows = null;
  }
  return { ...state, view: next, rows, overrides, picked, started, source, stemPath };
}

// The source to start now, or null: the song as soon as the window opens, a stem once a file is
// picked. Each source starts by itself once per map; a refused start is forgotten (`forget`).
export function autoStart(state, busy) {
  const { view, source, stemPath, started } = state;
  if (busy || !view?.map || view.state !== "idle") return null;
  const key = source === "song" ? (view.hasSong ? "song" : null) : stemPath;
  return key && !started.includes(key) ? key : null;
}

export const forget = (state, key) => ({ ...state, started: state.started.filter(item => item !== key) });

export const startRequest = state => ({
  map: state.view.map,
  source: state.source === "song" ? { kind: "song" } : { kind: "stem", path: state.stemPath },
});

// drum_apply's arguments: the review the mapper saw and the hits checked in it; null without any.
export function applyRequest(state) {
  if (state.view?.state !== "ready" || !state.rows) return null;
  const checked = ticked(state.rows, state.overrides);
  return checked.length ? { map: state.view.map, review: state.view.review, ticked: checked } : null;
}

// When to poll next, or null once the window is closing.
export const nextPoll = (closed, view) => (closed ? null : pollDelay(view));

// Answers come back out of order; only one newer than the last taken is shown.
export const isFresh = (sequence, taken) => sequence > taken;

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
