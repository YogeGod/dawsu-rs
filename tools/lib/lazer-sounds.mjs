// What osu!lazer would play for a saved map, from the lazer oracle's JSON (tools/lazer-oracle), and how to
// tell two such lists apart. Used by tools/compare-314.mjs and tools/remap-check.mjs.
import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

// Which file or skin sound lazer plays for one sample, in a mapset folder. A beatmap sample is looked up
// in the mapset only when the sample may use beatmap samples (osu.Game/Skinning/LegacyBeatmapSkin.cs:92-97);
// the lookup tries each name and its last path piece, keeps only names ending in the custom index when
// there is one, then the bare name (osu.Game/Skinning/LegacySkin.cs:610-640). A name with no file falls
// through to the player's skin, which is the same for both copies, so it is named, not resolved.
export function folderFiles(folder) {
  const files = new Map();
  for (const name of readdirSync(folder)) if (statSync(join(folder, name)).isFile()) files.set(name.toLowerCase(), name);
  return files;
}
export function resolveSample(sample, folder, files) {
  if (sample.use_beatmap_samples) {
    let names = sample.lookup_names.flatMap(name => [name, name.split("/").at(-1)]);
    if (sample.suffix) names = names.filter(name => name.endsWith(sample.suffix));
    names.push(sample.name);
    for (const name of names) {
      for (const candidate of [name, `${name}.wav`, `${name}.ogg`, `${name}.mp3`]) {
        const file = files.get(candidate.toLowerCase());
        if (file) return { file: join(folder, file), label: file };
      }
    }
  }
  // The player's skin cannot use custom sample banks, so lazer drops the names that end in the custom
  // index there (LegacySkin.cs:621-624 with UseCustomSampleBanks false): "soft-hitnormal2" plays as
  // "soft-hitnormal".
  const skinNames = sample.lookup_names.filter(name => !name.includes(".") && !(sample.suffix && name.endsWith(sample.suffix)));
  const skin = skinNames[0] ?? sample.name;
  return { skin: skin.split("/").at(-1), label: `skin ${skin.split("/").at(-1)}` };
}
// Every sound lazer plays, as [time, label, volume, resolved]. Circles play at their start; a slider
// plays its head, repeats and ticks through its nested objects and its tail sounds at its end
// (osu.Game.Rulesets.Osu/Objects/Slider.cs:37, Drawables/DrawableSlider.cs:165-167); a spinner plays at
// its end. Looping sounds (slider slide and whistle, spinner spin) count once at the object's start.
export function heard(result, folder) {
  const files = folderFiles(folder);
  const out = [];
  const add = (time, sample, loop) => {
    const where = resolveSample(sample, folder, files);
    out.push({ time: Math.round(time), loop, volume: sample.volume, ...where });
  };
  const looping = name => /^(sliderslide|sliderwhistle|spinnerspin|spinnerbonus)$/.test(name);
  const walk = object => {
    const end = object.end_time ?? object.start_time;
    const own = object.type === "Spinner" ? end : object.start_time;
    if (object.type !== "Slider") for (const sample of object.samples ?? []) add(own, sample, false);
    for (const sample of object.auxiliary_samples ?? []) add(looping(sample.name) ? object.start_time : end, sample, looping(sample.name));
    for (const nested of object.nested ?? []) walk(nested);
  };
  for (const object of result.hit_objects) walk(object);
  return out;
}
// Two mapset files sound the same when their bytes match, or, for two WAVs, when their audio matches
// within a small tolerance (a mix two apps rendered with different headers or rounding).
const hashes = new Map();
const hashOf = path => hashes.get(path) ?? hashes.set(path, createHash("sha256").update(readFileSync(path)).digest("hex")).get(path);
export function wavSamples(path) {
  const bytes = readFileSync(path);
  if (bytes.toString("ascii", 0, 4) !== "RIFF" || bytes.toString("ascii", 8, 12) !== "WAVE") return null;
  let offset = 12, format = null;
  while (offset + 8 <= bytes.length) {
    const id = bytes.toString("ascii", offset, offset + 4), size = bytes.readUInt32LE(offset + 4);
    if (id === "fmt ") format = { code: bytes.readUInt16LE(offset + 8), channels: bytes.readUInt16LE(offset + 10), rate: bytes.readUInt32LE(offset + 12), bits: bytes.readUInt16LE(offset + 22) };
    if (id === "data" && format) {
      const data = bytes.subarray(offset + 8, offset + 8 + size);
      const count = Math.floor(data.length / (format.bits / 8));
      const values = new Float64Array(count);
      for (let i = 0; i < count; i++) {
        values[i] = format.code === 3 && format.bits === 32 ? data.readFloatLE(i * 4)
          : format.bits === 16 ? data.readInt16LE(i * 2) / 32768
          : format.bits === 24 ? data.readIntLE(i * 3, 3) / 8388608
          : format.bits === 8 ? (data[i] - 128) / 128 : NaN;
      }
      return { ...format, values };
    }
    offset += 8 + size + (size % 2);
  }
  return null;
}
export function sameSound(a, b) {
  if (a.skin || b.skin) return a.skin === b.skin;
  if (hashOf(a.file) === hashOf(b.file)) return true;
  const x = wavSamples(a.file), y = wavSamples(b.file);
  if (!x || !y || x.rate !== y.rate || x.channels !== y.channels) return false;
  const n = Math.max(x.values.length, y.values.length);
  if (Math.abs(x.values.length - y.values.length) > x.channels * Math.ceil(x.rate / 1000)) return false; // over 1 ms longer
  let diff = 0, power = 0;
  for (let i = 0; i < n; i++) {
    const p = x.values[i] ?? 0, q = y.values[i] ?? 0;
    diff += (p - q) ** 2;
    power += p * p;
  }
  return power === 0 ? diff === 0 : Math.sqrt(diff / power) < 0.01; // within 1% (-40 dB)
}
export const byTime = list => {
  const map = new Map();
  for (const sound of list) map.set(sound.time, [...(map.get(sound.time) ?? []), sound]);
  return map;
};
// Pairs up the sounds at one moment; what is left over on either side is a difference.
// Same sound at the same volume pairs first, so two layers in a different order are not counted as two
// volume differences.
export function matchMoment(left, right) {
  const rest = [...right];
  const unpaired = [];
  for (const sound of left) {
    const exact = rest.findIndex(other => other.loop === sound.loop && other.volume === sound.volume && sameSound(sound, other));
    if (exact < 0) unpaired.push(sound);
    else rest.splice(exact, 1);
  }
  const onlyLeft = [], volume = [];
  for (const sound of unpaired) {
    const same = rest.findIndex(other => other.loop === sound.loop && sameSound(sound, other));
    if (same < 0) { onlyLeft.push(sound); continue; }
    volume.push([sound, rest[same]]);
    rest.splice(same, 1);
  }
  return { onlyLeft, onlyRight: rest, volume };
}

export const describe = sound => `${sound.label} at ${sound.volume}%${sound.loop ? " (loop)" : ""}`;
