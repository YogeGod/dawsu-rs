// The Mixer's faders and strips (v3 §6). Song and master gains multiply the samples
// (dawsu-audio), so a gain's level in dB is 20·log10(gain).
import { laneGrey, mixIntoCanvas } from "./colour.js";

// v3's fader scale: dB at a place from the top (0) to the bottom (1) of the fader's travel.
export const faderStops = [[6, 0], [0, 0.14], [-5, 0.27], [-10, 0.4], [-20, 0.6], [-30, 0.75], [-40, 0.85], [-60, 1]];
export const scaleMarks = [["6", 0], ["0", 0.14], ["5", 0.27], ["10", 0.4], ["20", 0.6], ["30", 0.75], ["40", 0.85], ["∞", 1]];

const toDb = gain => (gain > 0 ? 20 * Math.log10(gain) : -Infinity);
const toGain = db => (db === -Infinity ? 0 : 10 ** (db / 20));
const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

// "−3.10", "0.00" (never "−0.00"), or "−∞" for silence.
export function dbText(gain) {
  const db = toDb(gain);
  if (db === -Infinity) return "−∞";
  const text = db.toFixed(2);
  return text === "-0.00" ? "0.00" : text.replace("-", "−");
}

export function faderPosition(gain) {
  const db = toDb(gain);
  if (db >= faderStops[0][0]) return 0;
  for (let k = 1; k < faderStops.length; k++) {
    const [db1, p1] = faderStops[k];
    const [db0, p0] = faderStops[k - 1];
    if (db >= db1) return p0 + (db0 - db) / (db0 - db1) * (p1 - p0);
  }
  return 1;
}

// The gain at a place on the fader, held to the editor's range for that gain.
export function positionGain(position, min, max) {
  if (position >= 1) return clamp(0, min, max);
  const p = Math.max(0, position);
  for (let k = 1; k < faderStops.length; k++) {
    const [db1, p1] = faderStops[k];
    const [db0, p0] = faderStops[k - 1];
    if (p <= p1) return clamp(toGain(db0 - (p - p0) / (p1 - p0) * (db0 - db1)), min, max);
  }
  return clamp(0, min, max);
}

// The places on the travel a gain's range reaches: above `top` and below `bottom` it can't go.
export const faderReach = (min, max) => ({ top: faderPosition(max), bottom: faderPosition(min) });

// Arrow keys move a fader by dB; below -60 dB it reaches silence.
export function stepGain(gain, db, min, max) {
  const next = toDb(gain) === -Infinity ? (db > 0 ? -60 : -Infinity) : toDb(gain) + db;
  return clamp(next < -60 ? 0 : toGain(next), min, max);
}

const faderSteps = { ArrowUp: 1, ArrowRight: 1, ArrowDown: -1, ArrowLeft: -1, PageUp: 6, PageDown: -6 };

// The gain a key moves a fader to, or null when the key belongs to the editor.
export function faderKey(event, gain, min, max) {
  if (event.ctrlKey || event.altKey || event.metaKey) return null;
  if (event.key in faderSteps) return stepGain(gain, faderSteps[event.key], min, max);
  if (event.key === "Home") return min;
  if (event.key === "End") return max;
  return null;
}

// One action in flight at a time, in order; a value that waits is replaced by a newer one.
// `drained` runs once the last one is answered.
export function faderSender(change, drained) {
  let busy = false;
  let queued = null;
  return async function send(value) {
    queued = value;
    if (busy) return;
    busy = true;
    try {
      while (queued !== null) {
        const next = queued;
        queued = null;
        await change(next);
      }
    } finally {
      busy = false;
      drained?.();
    }
  };
}

// After a mouse drag Home, End and the arrow keys go back to the editor (as the snap slider).
export const releaseFader = slot => slot?.blur?.();

// The editor takes both gains in one action: the moved fader's value and the other fader's last
// sent value while the editor has not answered it yet, else the editor's.
export function gainsAction(view, which, value, pending = {}) {
  return {
    kind: "gains",
    song: which === "song" ? value : pending.song ?? view?.song,
    main: which === "main" ? value : pending.main ?? view?.main,
  };
}

// A click on a strip selects its lane only outside its controls and its greyed-out parts.
export const stripClickSelects = target => !target?.closest?.("button, .fader, .box");

// The Song strip's colour: v3's teal (--editor-teal), as a hex for the strip shades.
export const songColour = "#4fd8d0";
// The Hitsounds bus strip's colour: warm amber to stand apart from Song and sample lanes.
export const hitsoundColour = "#f0a040";

// v3: fill 20 % into #181818 (selected 42 %), edge 30 % into the canvas, name bar the colour.
// A lane is grey unless the mapper picked a colour (ADR 0068).
export function stripShades(colour) {
  const strip = typeof colour === "string" && /^#[\da-f]{6}$/i.test(colour) ? colour.toLowerCase() : laneGrey;
  return {
    colour: strip, fill: mixIntoCanvas(strip, 20, "#181818"), selected: mixIntoCanvas(strip, 42, "#181818"),
    edge: mixIntoCanvas(strip, 30),
  };
}
