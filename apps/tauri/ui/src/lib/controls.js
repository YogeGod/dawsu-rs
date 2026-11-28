// One source for the timeline's wheel gestures and tool keys, so the wheel handler, the status-bar
// hints, the tool pill tooltips and the first-launch preset chooser cannot disagree.
import { DEFAULT_DAW_TOOL_KEYS, DEFAULT_OSU_TOOL_KEYS, MORE_TOOLS_KEY } from "./quasimode.js";

// What a wheel tick over the timeline does. The handler takes the first true one in the order
// rows, snap, zoom, pan, seek.
export function wheelIntent({ osu = false, ctrl = false, shift = false, alt = false, horizontal = false, overGrid = true } = {}) {
  const rows = ctrl && shift;
  const snap = !horizontal && (osu ? ctrl : alt && !ctrl);
  const zoom = !horizontal && (osu ? alt && !ctrl : ctrl);
  const pan = horizontal || (!osu && shift);
  const seek = osu && (shift || overGrid);
  return { rows, snap, zoom, pan, seek };
}

// The keys for each wheel gesture with a vertical wheel over the grid; null where the preset has none
// (a sideways wheel always pans).
export const WHEEL_HINTS = {
  DAW: { rows: "Ctrl+Shift+Wheel", snap: "Alt+Wheel", zoom: "Ctrl+Wheel", pan: "Shift+Wheel", seek: null },
  "osu!": { rows: "Ctrl+Shift+Wheel", snap: "Ctrl+Wheel", zoom: "Alt+Wheel", pan: null, seek: "Wheel" },
};

export const wheelHints = preset => WHEEL_HINTS[preset === "osu!" ? "osu!" : "DAW"];

export const toolTitle = (name, key) => (key ? `${name} (${key})` : name);

const LEADS = {
  "osu!": "osu! Editor: arrow keys move on snap, Space plays.",
  DAW: "DAW-like: the editor’s default timeline shortcuts.",
};
const WHEEL_VERBS = [["zoom", "zooms"], ["snap", "steps the snap"], ["pan", "pans"], ["seek", "seeks"]];

// One paragraph for the preset chooser: the tool keys and the wheel gestures.
export function presetSummary(preset) {
  const osu = preset === "osu!";
  const keys = { ...(osu ? DEFAULT_OSU_TOOL_KEYS : DEFAULT_DAW_TOOL_KEYS), [MORE_TOOLS_KEY]: "Erase" };
  const tools = Object.keys(keys).sort().map(key => `${key} ${keys[key]}`).join(", ");
  const hints = wheelHints(preset);
  const wheel = WHEEL_VERBS.filter(([intent]) => hints[intent]).map(([intent, verb]) => `${hints[intent]} ${verb}`).join(", ");
  return `${LEADS[osu ? "osu!" : "DAW"]} Tools: ${tools}. ${wheel}.`;
}
