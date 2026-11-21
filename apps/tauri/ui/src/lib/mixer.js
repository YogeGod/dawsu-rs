// The Mixer's faders and strips (v3 §6). Song and master gains multiply the samples
// (dawsu-audio), so a gain sits on v3's dB scale at 20·log10(gain).
import { laneGrey, mixIntoCanvas } from "./colour.js";

// v3's fader scale: dB at a place from the top (0) to the bottom (1) of the fader's travel.
export const faderStops = [[6, 0], [0, 0.14], [-5, 0.27], [-10, 0.4], [-20, 0.6], [-30, 0.75], [-40, 0.85], [-60, 1]];
export const scaleMarks = [["6", 0], ["0", 0.14], ["5", 0.27], ["10", 0.4], ["20", 0.6], ["30", 0.75], ["40", 0.85], ["∞", 1]];

const toDb = gain => (gain > 0 ? 20 * Math.log10(gain) : -Infinity);
const toGain = db => (db === -Infinity ? 0 : 10 ** (db / 20));
const clamp = (value, min, max) => Math.min(max, Math.max(min, value));

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

// Arrow keys move a fader by dB; below -60 dB it reaches silence.
export function stepGain(gain, db, min, max) {
  const next = toDb(gain) === -Infinity ? (db > 0 ? -60 : -Infinity) : toDb(gain) + db;
  return clamp(next < -60 ? 0 : toGain(next), min, max);
}

// The editor takes both gains in one action.
export function gainsAction(view, which, value) {
  return { kind: "gains", song: which === "song" ? value : view?.song, main: which === "main" ? value : view?.main };
}

// v3: fill 20 % into #181818 (selected 42 %), edge 30 % into the canvas, name bar the colour.
// A lane is grey unless the mapper picked a colour (ADR 0068).
export function stripShades(colour) {
  const strip = typeof colour === "string" && /^#[\da-f]{6}$/i.test(colour) ? colour.toLowerCase() : laneGrey;
  return {
    colour: strip, fill: mixIntoCanvas(strip, 20, "#181818"), selected: mixIntoCanvas(strip, 42, "#181818"),
    edge: mixIntoCanvas(strip, 30),
  };
}
