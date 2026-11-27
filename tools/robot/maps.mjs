// Picks a ranked map for a robot run and copies its mapset into the run's profile. The original is
// only ever read: it is hashed before and after so a run can prove it never changed.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { snapshot } from "./harness.mjs";
import { readOsu, soundMoments } from "./osu-sounds.mjs";

function candidatesIn(folder, depth = 0) {
  const found = [];
  for (const name of readdirSync(folder).sort()) {
    const path = join(folder, name);
    const stat = statSync(path);
    if (stat.isDirectory() && depth < 3) found.push(...candidatesIn(path, depth + 1));
    else if (/\.osu$/i.test(name)) found.push({ osu: path });
    else if (/\.osz$/i.test(name)) found.push({ osz: path });
  }
  return found;
}

/**
 * A difficulty worth a run: osu!standard, its song in the folder, at least 30 s and 60 sound
 * moments of objects, and (when siblings is set) at least one other osu!standard difficulty.
 */
export function usable(osuPath, { siblings = false } = {}) {
  const bytes = readFileSync(osuPath);
  const osu = readOsu(bytes);
  if (Number(osu.general.Mode ?? 0) !== 0) return false;
  const audio = osu.general.AudioFilename;
  if (!audio || !existsSync(join(dirname(osuPath), audio))) return false;
  const moments = soundMoments(bytes);
  if (moments.length < 60 || moments.at(-1).time - moments[0].time < 30000) return false;
  if (siblings) {
    const others = readdirSync(dirname(osuPath)).filter(name => /\.osu$/i.test(name) && name !== basename(osuPath));
    if (!others.some(name => Number(readOsu(readFileSync(join(dirname(osuPath), name))).general.Mode ?? 0) === 0)) return false;
  }
  return true;
}

/**
 * { original, source, copy, mapFolder, hashOriginal(), before, note } for --map or --dataset.
 * source is the original's folder, or the .osz it came from.
 */
export function pickAndCopy({ map, dataset, pick, seed, profile, siblings = false }) {
  let original = null;
  let source = null;
  let note = "";
  if (map) {
    original = resolve(map);
    source = dirname(original);
  } else {
    const all = candidatesIn(resolve(dataset));
    const order = all.map((_, i) => i);
    const start = pick !== undefined ? Number(pick) : 0;
    if (pick === undefined) {
      // A repeatable shuffle from the seed (mulberry32), so a failing map can be run again.
      const used = seed !== undefined ? Number(seed) : Date.now() % 100000;
      let t = used >>> 0;
      const random = () => { t += 0x6D2B79F5; let r = Math.imul(t ^ (t >>> 15), 1 | t); r ^= r + Math.imul(r ^ (r >>> 7), 61 | r); return ((r ^ (r >>> 14)) >>> 0) / 4294967296; };
      for (let i = order.length - 1; i > 0; i--) { const j = Math.floor(random() * (i + 1)); [order[i], order[j]] = [order[j], order[i]]; }
      note = ` (seed ${used}; rerun with --seed ${used})`;
    }
    for (const i of order.slice(start)) {
      const candidate = all[i];
      if (candidate.osu && usable(candidate.osu, { siblings })) { original = candidate.osu; source = dirname(candidate.osu); break; }
      if (candidate.osz) {
        // Unpacked into the profile, never next to the original (bsdtar reads zip on Windows 10+).
        const out = join(profile, "unpacked", basename(candidate.osz, ".osz"));
        mkdirSync(out, { recursive: true });
        if (spawnSync("tar", ["-xf", candidate.osz, "-C", out]).status !== 0) continue;
        const osu = readdirSync(out).filter(name => /\.osu$/i.test(name)).map(name => join(out, name)).find(path => usable(path, { siblings }));
        if (osu) { original = osu; source = candidate.osz; break; }
      }
    }
    if (!original) throw new Error(`no usable osu!standard difficulty under ${dataset}`);
  }
  const isOsz = /\.osz$/i.test(source);
  const hashOriginal = () => isOsz
    ? new Map([[basename(source), createHash("sha256").update(readFileSync(source)).digest("hex")]])
    : snapshot(source);
  const before = hashOriginal();
  const mapFolder = join(profile, "Songs", basename(dirname(original)));
  // dereference: a link in the map's folder is copied as the file it points at, so the editor can
  // never write through the copy into the original.
  cpSync(dirname(original), mapFolder, { recursive: true, dereference: true });
  return { original, source, isOsz, copy: join(mapFolder, basename(original)), mapFolder, hashOriginal, before, note };
}
