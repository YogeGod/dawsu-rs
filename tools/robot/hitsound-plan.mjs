// How a human hitsounder would hitsound 30 seconds of a map: the plan the robot mapper follows in
// tools/hitsound-30s.mjs, and the answer it checks the saved [Hitsounds] difficulty against.
//
// The rules are the common ones from the hitsounding guides the project already uses (Lasse, Mentai
// and Net0, see hitsound-pack-census notes) kept simple and predictable:
//   - a hitnormal on every sound moment of the gameplay difficulty (circles, slider heads, repeats,
//     tails, spinner ends), because osu! always plays one there anyway (ConvertHitObjectParser.cs:547-553);
//   - a clap on the backbeat (beats 2 and 4 of a 4/4 bar, every second beat in other meters);
//   - a finish on the first downbeat of every four bars, on the first moment of a kiai section,
//     and on a spinner's end;
//   - a whistle on slider heads that land on beat 3 outside kiai (a light melodic accent);
//   - outside kiai the Soft set throughout; inside kiai a Normal hitnormal with Drum additions,
//     so the chorus sounds harder (one normal set and one addition set per moment, which a circle's
//     line can hold: ConvertHitObjectParser.cs:197-238);
//   - volumes by weight: downbeat 80%, other beats 70%, half beats 55%, the rest 40%, kiai +10%,
//     never under 5% (the project rule; lazer's floor is DrawableHitObject.cs:186);
//   - the custom index the map's green line gives at that moment, so no new green line is needed.
import { controlPoints, readOsu, soundMoments } from "./osu-sounds.mjs";

export const DIVISORS = [1, 2, 3, 4, 6, 8, 12, 16];
const TOLERANCE_MS = 2;

/** The beat position of `time` under its red line: { divisor, beat, onBeat, barBeat, bar } or null when off every grid. */
export function beatPosition(lines, time) {
  const red = lines.timingAt(time);
  const beats = (time - red.time) / red.beatLength;
  for (const divisor of DIVISORS) {
    const ticks = Math.round(beats * divisor);
    const snapped = red.time + ticks * red.beatLength / divisor;
    if (Math.abs(snapped - time) <= TOLERANCE_MS) {
      const beat = ticks / divisor;
      const whole = Math.floor(beat + 1e-9);
      const onBeat = ticks % divisor === 0;
      const meter = red.meter ?? 4;
      return { divisor, beat, onBeat, half: !onBeat && (ticks * 2) % divisor === 0, barBeat: ((whole % meter) + meter) % meter,
        bar: Math.floor(whole / meter), redTime: red.time, snapped };
    }
  }
  return null;
}

/** The 30 seconds to hitsound: from 10 s before the first kiai when there is one, else from the first object. */
export function chooseWindow(moments, lines, length = 30000) {
  const first = moments[0]?.time ?? 0;
  const kiai = lines.effects.find(point => point.kiai && point.time > first);
  const start = kiai ? Math.max(first, kiai.time - 10000) : first;
  return { start, end: start + length, kiaiStart: kiai?.time ?? null };
}


/**
 * The plan for a gameplay difficulty's bytes and the mapset's file names (lower case):
 * indexes: { calm, kiai } forces a section's custom index.
 * { window, moments: [{ time, volume, divisor, kiai, why, layers: [{ lane, bank, sound, index }] }], skipped }.
 */
export function planHitsounds(referenceBytes, mapFiles, { length = 30000, indexes = {} } = {}) {
  const osu = readOsu(referenceBytes);
  const lines = controlPoints(osu);
  const all = soundMoments(referenceBytes);
  const window = chooseWindow(all, lines, length);
  const sets = {
    calm: { normal: "soft", additions: "soft", whistle: true },
    kiai: { normal: "normal", additions: "drum", whistle: false },
  };
  const lanesOf = set => [`${set.normal}-hitnormal`, `${set.additions}-hitclap`, `${set.additions}-hitfinish`,
    ...(set.whistle ? [`${set.additions}-hitwhistle`] : [])];
  // The custom index each moment plays: the green line's in force there, so the circle's own line
  // holds it and Save needs no new green line (index 0 on a circle means "the green line's index",
  // so a skin sound under a green line with index 1 or 2 would need a line of its own).
  // indexes: { calm, kiai } forces a section's index instead.
  const lineIndex = time => lines.sampleAt(time + 5).index ?? 0;
  const index = { calm: indexes.calm ?? null, kiai: indexes.kiai ?? null };

  const byTime = new Map();
  const skipped = [];
  let kiaiOpened = false;
  let firstBar = null;
  for (const moment of all) {
    if (moment.time < window.start || moment.time >= window.end) continue;
    const where = beatPosition(lines, moment.time);
    if (!where) { skipped.push({ time: moment.time, reason: "not on a snap grid line (1/1 to 1/16)" }); continue; }
    // The grid line itself: a click there snaps onto it, and Save writes it as whole milliseconds.
    const time = Math.round(where.snapped);
    if (byTime.has(time)) {
      if (moment.node === "spinner") byTime.get(time).spinner = true;
      continue;
    }
    const kiai = lines.kiaiAt(moment.time + 1);
    firstBar ??= where.bar;
    byTime.set(time, { time, exact: where.snapped, where, kiai, node: moment.node, spinner: moment.node === "spinner" });
  }

  const moments = [];
  for (const entry of [...byTime.values()].sort((a, b) => a.time - b.time)) {
    const { where, kiai } = entry;
    const section = kiai ? "kiai" : "calm";
    const set = sets[section];
    const why = [];
    const at = index[section] ?? lineIndex(entry.exact);
    // Lanes of index 2 and up carry the number in their name (soft-hitnormal2); index 0 and 1 share it.
    const layer = (sound, bank) => ({ lane: `${bank}-${sound}${at >= 2 ? at : ""}`, bank, sound, index: at });
    const layers = [layer("hitnormal", set.normal)];
    const meter = 4;
    const backbeat = where.onBeat && (where.barBeat % 2 === 1);
    const phraseStart = where.onBeat && where.barBeat === 0 && ((where.bar - firstBar) % 4 === 0);
    const kiaiStart = kiai && !kiaiOpened;
    if (kiai) kiaiOpened = true;
    if (phraseStart || kiaiStart || entry.spinner) {
      layers.push(layer("hitfinish", set.additions));
      why.push(entry.spinner ? "spinner end" : kiaiStart ? "kiai starts" : "phrase start");
    } else if (backbeat) {
      layers.push(layer("hitclap", set.additions));
      why.push("backbeat");
    } else if (set.whistle && entry.node === "head" && where.onBeat && where.barBeat === 2 % meter) {
      layers.push(layer("hitwhistle", set.additions));
      why.push("slider accent");
    }
    let volume = where.onBeat ? (where.barBeat === 0 ? 80 : 70) : where.half ? 55 : 40;
    if (entry.spinner) volume = 70;
    if (kiai) volume = Math.min(100, volume + 10);
    volume = Math.max(5, volume);
    moments.push({ time: entry.time, exact: entry.exact, volume, divisor: where.divisor, kiai, why: why.join(", ") || "hitnormal", layers });
  }
  return { window, index, moments, skipped };
}
