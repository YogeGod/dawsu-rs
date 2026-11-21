// Does dawsu! save what 3.1.4 would save for the same edits? One number, and the differences in words.
//
// node tools/compare-314.mjs --exe <dawsu-tauri.exe> [--old-app <dawsu-hitsounding-editor checkout>]
//   [--map <fixture .osu>] [--json <file>] [--keep] [--quick]
//
//   --exe      the built editor, as for tools/everyday-check.mjs
//   --old-app  a checkout of YogeGod/dawsu-hitsounding-editor (default: ../dawsu-hitsounding-editor next
//              to this repo). It is only read: a clone of it at the 3.1.4 release commit is made under
//              target/compare-314/old-app-3.1.4 and built there.
//   --map      as for the everyday check (its default fixture)
//   --json     also write the comparison as data to this file
//   --keep     keep the work folder (map copies, results) under target/compare-314/
//   --quick    pass --quick to the everyday check (only its editing and saving steps)
//
// What it does:
//   0. Copies the map's folder and adds a short silent WAV for each sound file its hits name but the
//      folder lacks (3.1.4 will not save without them; the fixtures carry no audio).
//   1. Runs the everyday check (the robot mapper) on a copy of the map and keeps its edited copy.
//   2. Replays the same edits in 3.1.4's own window on a second copy, without a screen and without
//      sound, and saves through 3.1.4's own Save (tools/compare-314/Program.cs).
//   3. Runs the lazer oracle over the original, the dawsu! copy and the 3.1.4 copy, so osu!lazer itself
//      reads all three saved maps.
//   4. Lists, for each copy, every sound osu!lazer would play: its time, the sound it resolves to and its
//      volume. A sound is the mapset file lazer would pick (compared by its audio) or lazer's own skin
//      sound by name. The score is the share of sound moments where both copies play the same sounds at
//      the same volumes.
//   5. Lines up what each app showed after each edit (where a hit ended up, what a typed volume became).
//
// Nothing plays and nothing outside target/ is written, except what 3.1.4 itself writes under
// %LOCALAPPDATA%\OsuHitsoundEditor, which the twin removes again. Windows only, like both apps.
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const RELEASE = "c81774eb212c530e928cd184f739a379cecccc0b"; // "Release dawsu! V3.1.4"
const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = {};
for (let i = 2; i < process.argv.length; i++) {
  const value = process.argv[i + 1];
  const flag = value === undefined || value.startsWith("--");
  args[process.argv[i].replace(/^--/, "")] = flag ? true : value;
  if (!flag) i++;
}
if (!args.exe) throw new Error("--exe is required: the built dawsu-tauri.exe");
const given = resolve(args.map ?? join(repo, "tests/fixtures/save-pack/synthetic/lf-no-final-newline.osu"));
if (!existsSync(given)) throw new Error(`no map at ${given}`);
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..*/, "");
const root = join(repo, "target", "compare-314");
const work = join(root, `${stamp}-${process.pid}`);
mkdirSync(work, { recursive: true });

// 0. The map, with a stand-in tone for each sound file it names but its folder lacks. 3.1.4 refuses to save
// a map whose hit names a missing file ("Save stopped: Missing clap-custom.wav."), and the fixtures
// carry no audio, so both apps get the same complete copy: each missing file becomes a short tone
// WAV under the name the map uses. The given map's folder is only read.
const source = join(work, "source");
cpSync(dirname(given), source, { recursive: true, dereference: true });
const original = join(source, basename(given));
const standIns = [];
{
  let section = "";
  const present = new Set(readdirSync(source).map(name => name.toLowerCase()));
  for (const line of readFileSync(original, "latin1").split(/\r?\n/)) {
    if (/^\[.*\]$/.test(line.trim())) { section = line.trim(); continue; }
    if (section !== "[HitObjects]") continue;
    const name = line.split(",").at(-1).split(":")[4]?.trim();
    if (!name || present.has(name.toLowerCase()) || /[\\/]/.test(name)) continue;
    writeFileSync(join(source, name), toneWav(name));
    present.add(name.toLowerCase());
    standIns.push(name);
  }
}
// A quiet 0.1 s tone whose pitch comes from the file name, so two stand-ins never sound the same and a
// mix made from them can be told apart from a mix of other layers. Nothing plays it; it is only read.
function toneWav(name) {
  const samples = 4410; // 0.1 s, 16-bit mono, 44.1 kHz
  const wav = Buffer.alloc(44 + samples * 2);
  const pitch = 200 + (createHash("sha256").update(name.toLowerCase()).digest().readUInt16LE(0) % 1800);
  for (let i = 0; i < samples; i++) wav.writeInt16LE(Math.round(3000 * Math.sin(2 * Math.PI * pitch * i / 44100)), 44 + i * 2);
  wav.write("RIFF", 0); wav.writeUInt32LE(36 + samples * 2, 4); wav.write("WAVEfmt ", 8);
  wav.writeUInt32LE(16, 16); wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22); wav.writeUInt32LE(44100, 24);
  wav.writeUInt32LE(88200, 28); wav.writeUInt16LE(2, 32); wav.writeUInt16LE(16, 34); wav.write("data", 36);
  wav.writeUInt32LE(samples * 2, 40);
  return wav;
}
if (standIns.length) console.log(`0/4 stand-in tones for sound files the map names but lacks: ${standIns.join(", ")}`);

function run(command, list, options = {}) {
  const result = spawnSync(command, list, { encoding: "utf8", maxBuffer: 64 << 20, ...options });
  if (result.error) throw new Error(`${command} could not start: ${result.error.message}`);
  return result;
}
function must(command, list, options) {
  const result = run(command, list, options);
  if (result.status !== 0) throw new Error(`${command} ${list.join(" ")} failed:\n${(result.stdout ?? "") + (result.stderr ?? "")}`.slice(0, 4000));
  return result;
}

// 1. The robot mapper, keeping its edited copy.
console.log("1/4 the everyday check on a copy of the map…");
const robotJson = join(work, "robot.json");
const robotRun = run(process.execPath, [join(repo, "tools/everyday-check.mjs"), "--exe", args.exe, "--map", original,
  "--json", robotJson, "--keep-profile", ...(args.quick ? ["--quick"] : [])], { stdio: ["ignore", "pipe", "inherit"] });
const robotLast = robotRun.stdout.trim().split("\n").at(-1);
console.log(`   ${robotLast}`);
if (!existsSync(robotJson)) throw new Error("the everyday check wrote no results");
const robot = JSON.parse(readFileSync(robotJson, "utf8"));
if (!robot.actions?.some(action => action.do === "save")) throw new Error("the everyday check did not reach Save, so there is nothing to compare");

// 2. 3.1.4, at its release commit, replaying the same edits.
console.log("2/4 the same edits in 3.1.4…");
let oldApp = resolve(args["old-app"] ?? join(repo, "..", "dawsu-hitsounding-editor"));
if (!existsSync(join(oldApp, "src/OsuHitsound.App/OsuHitsound.App.csproj"))) throw new Error(`no 3.1.4 checkout at ${oldApp}`);
// Always a separate clone under target/, so the build's bin/ and obj/ folders never land in the given
// checkout; cloning only reads it.
{
  const tree = join(root, "old-app-3.1.4");
  if (!existsSync(join(tree, ".git"))) {
    rmSync(tree, { recursive: true, force: true });
    must("git", ["clone", "--quiet", "--no-checkout", oldApp, tree]);
  }
  if (run("git", ["-C", tree, "rev-parse", "HEAD"]).stdout.trim() !== RELEASE) must("git", ["-C", tree, "checkout", "--quiet", "--detach", RELEASE]);
  const there = run("git", ["-C", tree, "rev-parse", "HEAD"]).stdout.trim();
  if (there !== RELEASE) throw new Error(`${tree} is at ${there}, not the 3.1.4 release ${RELEASE}`);
  oldApp = tree;
}
const twinBin = join(root, "twin-bin");
must("dotnet", ["build", join(repo, "tools/compare-314/Twin314.csproj"), "-c", "Release", "-v", "q", "-nologo",
  `-p:OldApp=${oldApp}`, "-o", twinBin]);
const copies = { original: join(work, "original"), dawsu: join(work, "dawsu-rs"), old: join(work, "v3-1-4") };
cpSync(dirname(original), copies.original, { recursive: true, dereference: true });
cpSync(dirname(robot.mapCopy), copies.dawsu, { recursive: true, dereference: true });
cpSync(dirname(original), copies.old, { recursive: true, dereference: true });
const twinJson = join(work, "twin.json");
const twinRun = run(join(twinBin, process.platform === "win32" ? "Snapshot.exe" : "Snapshot"),
  [join(copies.old, basename(original)), robotJson, twinJson]);
console.log(`   ${(twinRun.stdout || twinRun.stderr).trim().split("\n").at(-1)}`);
const twin = existsSync(twinJson) ? JSON.parse(readFileSync(twinJson, "utf8"))
  : { steps: [], saved: false, error: (twinRun.stderr || twinRun.stdout || `exit code ${twinRun.status}`).trim().slice(0, 600) };
if (twin.error || !twin.saved) {
  // No 3.1.4 save, so no score: comparing with its unedited copy would measure nothing.
  const why = twin.error ? `3.1.4 stopped: ${twin.error}` : `3.1.4 did not save${twin.steps?.at(-1)?.status ? `: "${twin.steps.at(-1).status}"` : ""}`;
  console.log(why);
  console.log("compare with 3.1.4: no score (3.1.4 did not save)");
  if (args.json) writeFileSync(resolve(args.json), JSON.stringify({
    summary: { standIns, robot: robotLast, robotFailed: (robot.steps ?? []).filter(step => step.status === "FAIL").map(step => `${step.number} ${step.name}: ${step.detail}`),
      twin: why, sounds: { same: 0, total: 0, edited: { same: 0, total: 0 } }, steps: { same: 0, total: 0 } },
    steps: [], moments: [], dialogs: twin.dialogs ?? [], work }, null, 2) + "\n");
  try { rmSync(robot.profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 }); } catch {}
  if (!args.keep) try { rmSync(work, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 }); } catch {}
  process.exit(1);
}

// 3. osu!lazer reads all three.
console.log("3/4 osu!lazer reads the three maps…");
const oracleOut = join(work, "oracle");
must("dotnet", ["run", "-c", "Release", "--project", join(repo, "tools/lazer-oracle"), "--", oracleOut,
  copies.original, copies.dawsu, copies.old]);
const oracle = name => JSON.parse(readFileSync(join(oracleOut, name, `${basename(original)}.json`), "utf8"));

// 4. What lazer would play, per copy.
console.log("4/4 comparing…");
// Which file or skin sound lazer plays for one sample, in a mapset folder. A beatmap sample is looked up
// in the mapset only when the sample may use beatmap samples (osu.Game/Skinning/LegacyBeatmapSkin.cs:92-97);
// the lookup tries each name and its last path piece, keeps only names ending in the custom index when
// there is one, then the bare name (osu.Game/Skinning/LegacySkin.cs:610-640). A name with no file falls
// through to the player's skin, which is the same for both copies, so it is named, not resolved.
function folderFiles(folder) {
  const files = new Map();
  for (const name of readdirSync(folder)) if (statSync(join(folder, name)).isFile()) files.set(name.toLowerCase(), name);
  return files;
}
function resolveSample(sample, folder, files) {
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
function heard(result, folder) {
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
function wavSamples(path) {
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
function sameSound(a, b) {
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
const byTime = list => {
  const map = new Map();
  for (const sound of list) map.set(sound.time, [...(map.get(sound.time) ?? []), sound]);
  return map;
};
// Pairs up the sounds at one moment; what is left over on either side is a difference.
// Same sound at the same volume pairs first, so two layers in a different order are not counted as two
// volume differences.
function matchMoment(left, right) {
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
const before = byTime(heard(oracle("original"), copies.original));
const ours = byTime(heard(oracle("dawsu-rs"), copies.dawsu));
const theirs = byTime(heard(oracle("v3-1-4"), copies.old));
const times = [...new Set([...ours.keys(), ...theirs.keys()])].sort((a, b) => a - b);
const describe = sound => `${sound.label} at ${sound.volume}%${sound.loop ? " (loop)" : ""}`;
const moments = times.map(time => {
  const m = matchMoment(ours.get(time) ?? [], theirs.get(time) ?? []);
  const edited = JSON.stringify(matchMoment(ours.get(time) ?? [], before.get(time) ?? [])) !== JSON.stringify({ onlyLeft: [], onlyRight: [], volume: [] })
    || JSON.stringify(matchMoment(theirs.get(time) ?? [], before.get(time) ?? [])) !== JSON.stringify({ onlyLeft: [], onlyRight: [], volume: [] });
  const same = !m.onlyLeft.length && !m.onlyRight.length && !m.volume.length;
  const notes = [
    ...m.onlyLeft.map(sound => `only dawsu! plays ${describe(sound)}`),
    ...m.onlyRight.map(sound => `only 3.1.4 plays ${describe(sound)}`),
    ...m.volume.map(([a, b]) => `${a.label}: dawsu! ${a.volume}%, 3.1.4 ${b.volume}%`),
  ];
  return { time, same, edited, notes };
});
const sameCount = moments.filter(moment => moment.same).length;
const editedMoments = moments.filter(moment => moment.edited);
const editedSame = editedMoments.filter(moment => moment.same).length;

// 5. What each app showed after each edit.
// Hit lists are compared as what changed since the map opened in that app, because the two apps
// may read the opened map differently (that shows on the "open" line once, not on every edit after it).
const opened = { dawsu: robot.actions.find(action => action.do === "open")?.seen, v314: twin.steps?.find(step => step.do === "open")?.seen };
const changes = (list, start) => Array.isArray(list) && Array.isArray(start)
  ? { added: list.filter(hit => !start.includes(hit)), removed: start.filter(hit => !list.includes(hit)) } : list;
const showChange = change => change && typeof change === "object" && !Array.isArray(change)
  ? [change.added.length ? `added ${change.added.join(", ")}` : "", change.removed.length ? `removed ${change.removed.join(", ")}` : ""].filter(Boolean).join("; ") || "no change"
  : null;
const show = value => value === null || value === undefined ? "nothing" : Array.isArray(value) ? `${value.length} hits` : String(value);
const steps = robot.actions.map((action, index) => {
  const other = twin.steps?.[index];
  const label = action.do === "open" ? "open the map (the hits each app reads from it)"
    : action.do === "place" ? `place a hit on ${action.lane} (dawsu! put it at ${action.seen} ms)`
    : action.do === "move" ? `move it by ${action.by} ms`
    : action.do === "volume" ? `type ${action.value}% as its volume`
    : action.do === "add-file" ? `add the sample ${action.file} to the mapset` : action.do;
  const relative = action.do !== "open";
  const ours = relative ? changes(action.seen, opened.dawsu) : action.seen;
  const theirs = relative ? changes(other?.seen, opened.v314) : other?.seen;
  const same = action.do === "add-file" || JSON.stringify(ours) === JSON.stringify(theirs);
  return { edit: label, same, dawsu: ours ?? null, v314: theirs ?? null, note: other?.status ?? other?.note ?? null,
    text: same ? null : action.do === "open"
      ? `dawsu! shows ${ours?.filter(hit => !theirs?.includes(hit)).join(", ") || "nothing more"}; 3.1.4 shows ${theirs?.filter(hit => !ours?.includes(hit)).join(", ") || "nothing more"}`
      : `dawsu! ${showChange(ours) ?? show(ours)}, 3.1.4 ${showChange(theirs) ?? show(theirs)}${other?.status ? ` (3.1.4 said: "${other.status}")` : ""}` };
});

const summary = {
  standIns,
  robot: robotLast,
  robotFailed: (robot.steps ?? []).filter(step => step.status === "FAIL").map(step => `${step.number} ${step.name}: ${step.detail}`),
  twin: twin.error ? `3.1.4 stopped: ${twin.error}` : `3.1.4 saved: ${twin.saved}`,
  sounds: { same: sameCount, total: moments.length, edited: { same: editedSame, total: editedMoments.length } },
  steps: { same: steps.filter(step => step.same).length, total: steps.length },
};
for (const step of steps) console.log(`${step.same ? "SAME" : "DIFF"} ${step.edit}${step.text ? " — " + step.text : ""}`);
for (const moment of moments.filter(moment => !moment.same)) console.log(`DIFF ${moment.time} ms — ${moment.notes.join("; ")}`);
if (twin.dialogs?.length) console.log(`3.1.4's Save showed: ${twin.dialogs.map(dialog => `"${dialog.title}" (chose ${dialog.chose})`).join(", ")}`);
console.log(summary.twin);
console.log(`edits: ${summary.steps.same} of ${summary.steps.total} ended the same in both apps`);
console.log(`sounds: ${editedSame} of ${editedMoments.length} edited moments play the same in both saved maps (${sameCount} of ${moments.length} moments in all)`);
// The headline is the edited moments (the unedited ones are the same in nearly every map), never
// rounded up to 100%.
const percent = editedMoments.length ? Math.floor(100 * editedSame / editedMoments.length) : 100;
console.log(`compare with 3.1.4: ${percent}% of edited sound moments the same (${editedSame} of ${editedMoments.length}; ${moments.length - sameCount} differences in all)`);
if (args.json) writeFileSync(resolve(args.json), JSON.stringify({ summary, steps, moments, dialogs: twin.dialogs ?? [], work }, null, 2) + "\n");

// The robot's own profile folder (its map copy) is no longer needed; the work folder goes unless --keep.
try { rmSync(robot.profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 }); } catch {}
if (!args.keep) try { rmSync(work, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 }); } catch {}
else console.log(`kept: ${relative(repo, work)}`);
if (twin.error || !twin.saved || sameCount !== moments.length) process.exitCode = 1;
