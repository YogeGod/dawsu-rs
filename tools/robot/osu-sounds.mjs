// What a .osu file plays, worked out the way osu!lazer does it, independently of dawsu!'s own Rust.
//
// The robot mapper uses this to check a saved difficulty by its sound, not only by its bytes: every
// circle, slider head, repeat and tail, and spinner end becomes one "sound moment" with the samples
// lazer would play there (bank, name, custom index or file, volume).
//
// Every rule cites osu!lazer at ppy/osu@131fe72 (the project's pinned truth):
//   LegacyBeatmapDecoder.cs   osu.Game/Beatmaps/Formats/LegacyBeatmapDecoder.cs
//   LegacyDecoder.cs          osu.Game/Beatmaps/Formats/LegacyDecoder.cs
//   ConvertHitObjectParser.cs osu.Game/Rulesets/Objects/Legacy/ConvertHitObjectParser.cs
//   LegacyRulesetExtensions   osu.Game/Rulesets/Objects/Legacy/LegacyRulesetExtensions.cs
//   Slider.cs                 osu.Game.Rulesets.Osu/Objects/Slider.cs
//   HitSampleInfo.cs          osu.Game/Audio/HitSampleInfo.cs
//
// Scope: osu!standard (Mode 0), the only mode dawsu! opens. Plain JS, no dependencies, so it runs
// anywhere Node runs. tools/robot/osu-sounds.test.mjs checks it against the committed lazer oracle
// results in tests/fixtures/lazer-oracle/.

const BANKS = ["none", "normal", "soft", "drum"]; // LegacySampleBank: None 0, Normal 1, Soft 2, Drum 3
const LENIENCY = 5; // LegacyBeatmapDecoder.cs:38 CONTROL_POINT_LENIENCY
const MINIMUM_VOLUME = 5; // DrawableHitObject.cs:186 MINIMUM_SAMPLE_VOLUME: quieter samples still play at 5%

/** The sections of a .osu: { name: [lines] }, key/value sections also as maps. */
export function readOsu(input) {
  // A file's bytes are read as UTF-8, bad bytes as U+FFFD, as lazer's LineBufferedReader does; a lone
  // CR also ends a line.
  const text = typeof input === "string" ? input : new TextDecoder("utf-8").decode(input);
  const sections = {};
  let section = "";
  const lines = text.replace(/^\uFEFF/, "").split(/\r\n|\r|\n/);
  const formatVersion = Number(/osu file format v(\d+)/.exec(lines[0] ?? "")?.[1] ?? 14);
  for (const raw of lines) {
    let line = raw.trim();
    if (!line || line.startsWith("//")) continue;
    // LegacyDecoder.cs:52-56, 101-107: a "//" after the first character starts a comment, except in [Metadata].
    if (section !== "Metadata" && line.indexOf("//") > 0) line = line.slice(0, line.indexOf("//")).trimEnd();
    const header = /^\[(.+)\]$/.exec(line);
    if (header) { section = header[1]; sections[section] ??= []; continue; }
    if (section) (sections[section] ??= []).push(line);
  }
  const pairs = name => Object.fromEntries((sections[name] ?? []).map(line => {
    const at = line.indexOf(":");
    return at < 0 ? [line, ""] : [line.slice(0, at).trim(), line.slice(at + 1).trim()];
  }));
  return { formatVersion, sections, general: pairs("General"), metadata: pairs("Metadata"),
    difficulty: pairs("Difficulty"), editor: pairs("Editor") };
}

/** Timing lines as lazer keeps them: red lines (timing) and the sample/difficulty state at each line. */
export function controlPoints(osu) {
  // LegacyBeatmapDecoder.cs:75 an early format (v<5) moves every time 24 ms later.
  const offset = osu.formatVersion < 5 ? 24 : 0;
  // LegacyBeatmapDecoder.cs:265-270 [General] SampleSet and SampleVolume are the defaults.
  const defaultBank = (osu.general.SampleSet ?? "Normal").toLowerCase();
  const defaultVolume = Number(osu.general.SampleVolume ?? 100);
  const byTime = new Map();
  for (const line of osu.sections.TimingPoints ?? []) {
    const split = line.split(",");
    if (split.length < 2) continue;
    const time = Number(split[0].trim()) + offset;
    const beatLength = Number(split[1].trim());
    // LegacyBeatmapDecoder.cs:497-527
    let bank = split.length >= 4 ? BANKS[Number(split[3])] ?? "normal" : defaultBank;
    if (bank === "none") bank = "normal"; // :526
    const index = split.length >= 5 ? Number(split[4]) : 0;
    const volume = split.length >= 6 ? Number(split[5]) : defaultVolume;
    const timingChange = split.length >= 7 ? split[6][0] === "1" : true;
    const kiai = split.length >= 8 ? (Number(split[7]) & 1) === 1 : false;
    // :494 a green line's slider velocity; NaN beat lengths count as 1.
    const sv = beatLength < 0 ? 100 / -beatLength : 1;
    const entry = byTime.get(time) ?? { time, timing: null, sample: null, sv: null, kiai: null };
    // LegacyBeatmapDecoder.cs addControlPoint/flushPendingPoints: at one time, a green line's
    // sample, difficulty and effect state override the red line's; the last of each kind wins.
    if (timingChange) {
      // LegacyBeatmapDecoder.cs:576-603: red lines go to the front of the pending list and the list is
      // read from the back, so of two red lines at one time the first one's state is kept.
      if (!entry.timing) {
        // TimingControlPoint.cs:65-68 a red line's beat length is held between 6 and 60000 ms.
        // LegacyBeatmapDecoder.cs:496-498 the meter (beats per bar) is the third field; missing or 0 is 4.
        const meter = split.length >= 3 && Number(split[2]) > 0 ? Number(split[2]) : 4;
        entry.timing = { time, beatLength: Math.min(60000, Math.max(6, beatLength)), meter };
        entry.sample ??= { bank, index, volume, fromRed: true };
        if (entry.sv === null) { entry.sv = sv; entry.svFromRed = true; }
        if (entry.kiai === null) { entry.kiai = kiai; entry.kiaiFromRed = true; }
      }
    } else {
      entry.sample = { bank, index, volume, fromRed: false };
      entry.sv = sv; entry.svFromRed = false;
      entry.kiai = kiai; entry.kiaiFromRed = false;
    }
    byTime.set(time, entry);
  }
  const points = [...byTime.values()].sort((a, b) => a.time - b.time);
  const timing = points.filter(p => p.timing).map(p => p.timing);
  const samples = points.filter(p => p.sample).map(p => ({ time: p.time, ...p.sample }));
  const difficulty = points.filter(p => p.sv !== null).map(p => ({ time: p.time, sv: p.sv }));
  const effects = points.filter(p => p.kiai !== null).map(p => ({ time: p.time, kiai: p.kiai }));
  // ControlPointInfo.BinarySearchWithFallback: the last point at or before the time, else the fallback.
  const at = (list, time, fallback) => {
    let found = null;
    for (const p of list) { if (p.time <= time) found = p; else break; }
    return found ?? fallback;
  };
  return {
    offset, timing, samples, difficulty, effects,
    // LegacyControlPointInfo.cs:26 the first sample point is the fallback; with none, lazer's default.
    sampleAt: time => at(samples, time, samples[0] ?? { bank: "normal", index: 0, volume: 100 }),
    timingAt: time => at(timing, time, timing[0] ?? { time: 0, beatLength: 1000 }),
    svAt: time => at(difficulty, time, { sv: 1 }).sv,
    kiaiAt: time => at(effects, time, { kiai: false }).kiai,
  };
}

// ConvertHitObjectParser.cs:197-238 readCustomSampleBanks.
function readBanks(text, info, banksOnly = false) {
  if (!text) return info;
  const split = text.split(":");
  let bank = BANKS[Number(split[0])] ?? "normal";
  let add = BANKS[Number(split[1])] ?? "normal";
  if (bank === "none") bank = null;
  if (add === "none") add = null;
  info.normal = bank;
  info.additions = add || bank;
  if (banksOnly) return info;
  if (split.length > 2) info.index = Number(split[2]);
  if (split.length > 3) info.volume = Math.max(0, Number(split[3]));
  info.file = split.length > 4 && split[4] ? split[4] : null;
  return info;
}

// ConvertHitObjectParser.cs:543-569 convertSoundType: a hitnormal always plays (layered when the
// Normal bit is off), the custom file replaces it, then finish, whistle, clap in that order.
function samplesFor(type, info) {
  const list = [];
  const sample = (name, bank) => ({ name, bank, index: info.index ?? 0, volume: info.volume ?? 0, bankSpecified: !!bank });
  if (!info.file) list.push({ ...sample("hitnormal", info.normal), layered: type !== 0 && !(type & 1) });
  else list.push({ name: "file", file: info.file, volume: info.volume ?? 0 });
  if (type & 4) list.push(sample("hitfinish", info.additions));
  if (type & 2) list.push(sample("hitwhistle", info.additions));
  if (type & 8) list.push(sample("hitclap", info.additions));
  return list;
}

// LegacyDecoder.cs:192-203 LegacySampleControlPoint.ApplyTo: the object's own index, volume and bank
// win when set; otherwise the timing line's.
function applyLine(sample, line) {
  if (sample.name === "file") return { ...sample, volume: sample.volume > 0 ? sample.volume : line.volume };
  return {
    name: sample.name,
    bank: sample.bankSpecified ? sample.bank : line.bank,
    index: sample.index > 0 ? sample.index : line.index,
    volume: sample.volume > 0 ? sample.volume : line.volume,
    layered: !!sample.layered,
  };
}

/**
 * Every sound moment in the difficulty, in time order:
 * { time, object: index, node: "circle"|"head"|"repeat"|"tail"|"spinner", samples: [...] }.
 * A sample is { name, bank, index, volume, layered } or { name: "file", file, volume }.
 */
export function soundMoments(input) {
  const osu = readOsu(input);
  const lines = controlPoints(osu);
  const sliderMultiplier = Math.min(3.6, Math.max(0.4, Number(osu.difficulty.SliderMultiplier ?? 1.4))); // LegacyBeatmapDecoder.cs:130
  const moments = [];
  (osu.sections.HitObjects ?? []).forEach((line, object) => {
    const split = line.split(",");
    if (split.length < 5) return;
    const start = Number(split[2]) + lines.offset;
    const type = Number(split[3]);
    const sound = Number(split[4]);
    const info = { normal: null, additions: null, index: 0, volume: 0, file: null };
    if (type & 1) {
      if (split.length > 5) readBanks(split[5], info);
      // LegacyBeatmapDecoder.cs:186-189 a circle's line is the one at its end time + 5 ms.
      const at = lines.sampleAt(start + LENIENCY);
      moments.push({ time: start, object, node: "circle", samples: samplesFor(sound, info).map(s => applyLine(s, at)) });
    } else if (type & 2) {
      const repeats = Math.max(0, Number(split[6]) - 1); // :95 stable counted the first span
      let length = split.length > 7 ? Math.max(0, Number(split[7])) : 0;
      if (split.length > 10) readBanks(split[10], info, true); // :105-106 banks only for a slider
      const nodes = repeats + 2;
      const nodeInfo = Array.from({ length: nodes }, () => ({ ...info }));
      if (split.length > 9 && split[9].length) split[9].split("|").slice(0, nodes).forEach((s, i) => readBanks(s, nodeInfo[i]));
      const nodeSound = Array(nodes).fill(sound);
      if (split.length > 8 && split[8].length) split[8].split("|").slice(0, nodes).forEach((s, i) => { nodeSound[i] = Number.parseInt(s, 10) || 0; });
      // Slider.cs:162 velocity from the precision-adjusted beat length (LegacyRulesetExtensions.cs:17-39).
      const timing = lines.timingAt(start);
      const sv = lines.svAt(start);
      const svBeat = -100 / sv;
      const multiplier = svBeat < 0 ? Math.min(1000, Math.max(10, Math.fround(-svBeat))) / 100 : 1;
      const velocity = 100 * sliderMultiplier / (timing.beatLength * multiplier);
      if (!length) length = pathLength(split[5], Number(split[0]), Number(split[1]));
      const span = length / velocity;
      for (let i = 0; i < nodes; i++) {
        // LegacyBeatmapDecoder.cs:176-181 node i's line is at its own time + 5 ms.
        const time = start + i * span;
        const at = lines.sampleAt(time + LENIENCY);
        moments.push({ time, object, node: i === 0 ? "head" : i === nodes - 1 ? "tail" : "repeat",
          samples: samplesFor(nodeSound[i], nodeInfo[i]).map(s => applyLine(s, at)) });
      }
    } else if (type & 8) {
      const end = Math.max(start, Number(split[5]) + lines.offset);
      if (split.length > 6) readBanks(split[6], info);
      const at = lines.sampleAt(end + LENIENCY);
      moments.push({ time: end, object, node: "spinner", samples: samplesFor(sound, info).map(s => applyLine(s, at)) });
    }
  });
  return moments.sort((a, b) => a.time - b.time || a.object - b.object);
}

// A path's length when the line gives none: straight segments between the points (good enough for
// the rare map without a length; the oracle test only uses maps that give one).
function pathLength(path, x, y) {
  const points = [[x, y], ...path.split("|").slice(1).map(p => p.split(":").map(Number))];
  let length = 0;
  for (let i = 1; i < points.length; i++) length += Math.hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]);
  return length;
}

/**
 * The file osu!lazer looks up first for a sample, given the mapset's files (lower-cased names).
 * HitSampleInfo.cs:89-99 LookupNames and LegacyBeatmapSkin.cs:94: index 0 is the skin only; index 1
 * is the numberless beatmap file; 2+ adds the number. A beatmap file is .wav, .ogg or .mp3.
 * Returns "map:<file>" or "skin:<bank>-<name>".
 */
export function playedFile(sample, mapFiles) {
  if (sample.name === "file") return `map:${sample.file}`;
  const base = `${sample.bank}-${sample.name}${sample.index >= 2 ? sample.index : ""}`;
  if (sample.index >= 1) {
    for (const ext of ["wav", "ogg", "mp3"]) if (mapFiles.has(`${base}.${ext}`.toLowerCase())) return `map:${base}.${ext}`;
  }
  return `skin:${sample.bank}-${sample.name}`;
}

/** One line per sample, as "soft-hitclap:1@60", for comparing two difficulties or a plan. */
export function describe(sample) {
  if (sample.name === "file") return `file:${sample.file}@${Math.max(MINIMUM_VOLUME, sample.volume)}`;
  return `${sample.bank}-${sample.name}:${sample.index}@${sample.volume}`;
}

export { MINIMUM_VOLUME, LENIENCY };
