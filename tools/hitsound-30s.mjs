// The robot mapper hitsounds 30 seconds of a ranked map the way a person would, in the real editor,
// then checks what osu! will play from the saved file.
//
// node tools/hitsound-30s.mjs --exe <dawsu-tauri.exe> (--map <ranked .osu> | --dataset <folder>)
//   [--pick <n>] [--seed <n>] [--oracle] [--json <file>] [--keep-profile] [--allow-stale]
//
//   --map         one osu!standard difficulty of a ranked mapset; its whole folder is copied first
//   --dataset     a folder of ranked mapsets (folders or .osz files, e.g. E:\osu-lazer\exports\dataset);
//                 one osu!standard difficulty with at least 30 s of objects is picked from it
//   --pick/--seed which one: --pick n takes the n-th candidate by name, --seed n a repeatable random one
//                 (the run prints the seed it used)
//   --oracle      also run tools/lazer-oracle (needs dotnet) on the saved difficulty and compare
//                 osu!lazer's own answer with the robot's
//   --json        also write the results as data
//
// What it does, all through the editor's own screen (clicks, keys, the Ctrl+K palette):
//   1. opens the copied gameplay difficulty and switches Audition off, so nothing plays;
//   2. creates an empty [Hitsounds] difficulty from it (Ctrl+K › Create [Hitsounds] difficulty);
//   3. shows the 12 default lanes, picks Grid snap, and places every planned hit with the Draw tool
//      at the gameplay difficulty's object times: hitnormals everywhere, claps on the backbeat,
//      finishes on phrase starts, the kiai start and spinner ends, a few whistles on slider accents,
//      Soft outside kiai, Normal with Drum additions inside, volumes 40-90% set with Insert volume
//      (tools/robot/hitsound-plan.mjs has the rules);
//   4. saves through the save review;
// then checks that:
//   - every placed hit landed on its grid line and lane (snap works as intended);
//   - the saved file plays exactly the plan, by osu!lazer's rules (tools/robot/osu-sounds.mjs, itself
//     checked against lazer's own results), every volume at 5% or more;
//   - the mapset copy gained only the new .osu (and any sample file it really plays), every other
//     file is byte-identical, and the original mapset was never touched;
//   - reopening the saved difficulty shows the same hits.
//
// Only copies are opened: the map's folder is copied into this run's profile under target/, and the
// original is hashed before and after. Windows only in practice; see tools/robot/harness.mjs.
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkFreshness, createRobot, diffSnapshots, parseArgs, samePath, sleep, snapshot } from "./robot/harness.mjs";
import { planHitsounds } from "./robot/hitsound-plan.mjs";
import { pickAndCopy } from "./robot/maps.mjs";
import { describe, playedFile, readOsu, soundMoments } from "./robot/osu-sounds.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = parseArgs();
if (!args.exe) throw new Error("--exe is required: the built dawsu-tauri.exe");
const exe = resolve(args.exe);
if (!existsSync(exe)) throw new Error(`no editor at ${exe}`);
if (!args.map && !args.dataset) throw new Error("give --map <ranked .osu> or --dataset <folder of ranked mapsets>");
checkFreshness(repo, exe, Boolean(args["allow-stale"]));

const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..*/, "");
const profile = resolve(join(repo, "target", "robot-hitsound", `${stamp}-${process.pid}`));
mkdirSync(profile, { recursive: true });

// ---- Pick the map and copy its folder -------------------------------------------------------------
const picked = pickAndCopy({ map: args.map, dataset: args.dataset, pick: args.pick, seed: args.seed, profile });
const { original, source: originalFolder, isOsz: originalIsOsz, hashOriginal, before: originalBefore, mapFolder } = picked;
const reference = picked.copy;
const seedNote = picked.note;
const copyBefore = snapshot(mapFolder);
const referenceBytes = readFileSync(reference);
const mapFiles = new Set(readdirSync(mapFolder).map(name => name.toLowerCase()));
let plan = planHitsounds(referenceBytes, mapFiles);
console.log(`map: ${relative(process.cwd(), original)}${originalIsOsz ? ` (from ${basename(originalFolder)})` : ""}${seedNote}`);
console.log(`window: ${Math.round(plan.window.start)}–${Math.round(plan.window.end)} ms${plan.window.kiaiStart !== null ? `, kiai from ${plan.window.kiaiStart} ms` : ""}; ` +
  `${plan.moments.length} sound moments planned, ${plan.skipped.length} off the grid skipped`);

// ---- The run ---------------------------------------------------------------------------------------
const robot = createRobot({ exe, profile });
const { step, expect, until, read, page, hits, hitCount, laneRow, showTime, showRow, at, palette, pickTool, setDivisor, save, quit } = robot;
let hsPath = null;
let rows = new Map();     // plan lane "soft-hitclap#1" → scene row
let placedOff = [];       // hits that did not land where planned
let savedHits = null;

const laneKey = layer => `${layer.lane}#${layer.index}`;
async function sceneLanes() {
  return read(() => window.scene.lanes.map((lane, row) => ({ row, name: lane.name, index: lane.index ?? null })));
}

try {
  await step("Open the copied ranked map and see its objects", async () => {
    await robot.launch(reference);
    await robot.answerKeyPreset();
    await until(() => window.scene?.lanes?.length > 0 && !window.view?.songLoading, null, "the map did not open", 60000);
    const path = await read(() => window.scene.path);
    expect(samePath(path, reference), `the editor opened ${path}, not the copy`);
    const points = await read(() => window.scene.gameplay?.points?.length ?? 0);
    return `${await hitCount()} hits, ${points} object points`;
  }, { stops: true });

  await step("Switch Audition off so nothing plays", async () => {
    if (await read(() => window.view.audition)) await page().locator('button[aria-label="Audition"]').click({ timeout: 5000 });
    await until(() => window.view.audition === false, null, "Audition is still on");
  }, { stops: true });

  await step("Create an empty [Hitsounds] difficulty from it (Ctrl+K)", async () => {
    const before = new Set(readdirSync(mapFolder));
    await palette("Create [Hitsounds] difficulty");
    // The editor reports canonical paths, so compare on this side, not in the page.
    const end = Date.now() + 30000;
    while (Date.now() < end) {
      const now = await read(() => !window.view.busy && window.scene?.path).catch(() => null);
      if (now && !samePath(now, reference)) { hsPath = now.replace(/^\\\\\?\\/, ""); break; }
      await sleep(200);
    }
    expect(hsPath, "the editor did not open a new difficulty");
    expect(samePath(dirname(hsPath), mapFolder), `the new difficulty is not in the mapset copy: ${hsPath}`);
    const added = readdirSync(mapFolder).filter(name => !before.has(name));
    expect(hsPath && added.length === 1 && added[0] === basename(hsPath), `the mapset gained ${JSON.stringify(added)}`);
    expect(await read(() => window.scene.hs === true), "the new difficulty is not in Hitsounding mode");
    expect(await hitCount() === 0, `the new difficulty already has ${await hitCount()} hits`);
    const hs = readOsu(readFileSync(hsPath));
    const ref = readOsu(referenceBytes);
    expect(!(hs.sections.HitObjects ?? []).length, "the new difficulty has hit objects");
    expect(/hitsounds/i.test(hs.metadata.Version ?? ""), `its Version is ${hs.metadata.Version}`);
    expect(hs.metadata.BeatmapID === "0", `its BeatmapID is ${hs.metadata.BeatmapID}, not 0`);
    expect(JSON.stringify(hs.sections.TimingPoints) === JSON.stringify(ref.sections.TimingPoints), "its timing lines differ from the map's");
    expect(hs.general.AudioFilename === ref.general.AudioFilename, "its song differs from the map's");
    return `${basename(hsPath)}: no objects, Version ${hs.metadata.Version}, BeatmapID 0, timing copied`;
  }, { stops: true, covers: ["F004", "F050", "N013"] });

  await step("Show the 12 default lanes and find every lane the plan uses", async () => {
    if (await page().locator("#show-default-skin-lanes").isVisible().catch(() => false)) await page().click("#show-default-skin-lanes");
    else await palette("Show 12 default lanes");
    await until(() => window.scene.laneOrder.length >= 12, null, "the 12 default lanes did not show");
    let lanes = await sceneLanes();
    // A section that wants the map's own files (index 1) uses the skin when the editor shows no such lane.
    const missing = section => plan.moments.filter(m => (m.kiai ? "kiai" : "calm") === section)
      .some(m => m.layers.some(l => !lanes.some(lane => lane.name === l.lane && lane.index === l.index)));
    const notes = [];
    const forced = {};
    for (const section of ["calm", "kiai"]) {
      if (plan.index[section] === 1 && missing(section)) {
        notes.push(`${section}: no index-1 lanes shown, using the skin`);
        forced[section] = 0;
      }
    }
    if (Object.keys(forced).length) plan = planHitsounds(referenceBytes, mapFiles, { indexes: { ...plan.index, ...forced } });
    lanes = await sceneLanes();
    rows = new Map();
    for (const layer of plan.moments.flatMap(m => m.layers)) {
      const key = laneKey(layer);
      if (rows.has(key)) continue;
      const lane = lanes.find(l => l.name === layer.lane && l.index === layer.index) ?? (layer.index === 0 ? lanes.find(l => l.name === layer.lane && l.index === null) : null);
      expect(lane, `no ${layer.lane} lane with index ${layer.index}: ${JSON.stringify(lanes.map(l => `${l.name}#${l.index}`))}`);
      rows.set(key, lane.row);
    }
    return `${[...rows.keys()].join(", ")}${notes.length ? "; " + notes.join("; ") : ""}`;
  }, { stops: true, covers: ["F012", "F015"] });

  await step("Snap to the beat grid", async () => {
    if (await read(() => window.scene.snapMode === "objects")) {
      await page().click("#snap-mode-toggle");
      await until(() => window.scene.snapMode === "grid", null, "the snap stayed on Objects");
    }
    if (!(await read(() => window.scene.snap))) throw new Error("snap is off");
    await pickTool("Draw");
  }, { stops: true, covers: ["F030"] });

  // Placing, in three 10-second parts so the report shows progress.
  const parts = [0, 1, 2].map(k => plan.moments.filter(m => m.time >= plan.window.start + k * 10000 && m.time < plan.window.start + (k + 1) * 10000));
  let insert = null;
  for (const [k, moments] of parts.entries()) {
    await step(`Hitsound seconds ${k * 10}–${k * 10 + 10} (${moments.length} moments)`, async () => {
      if (!moments.length) return { skip: "no objects in these 10 seconds" };
      const zoom = Math.max(0.6, await read(() => window.scene.zoom));
      let placed = 0;
      for (const moment of moments) {
        if (insert !== moment.volume) {
          const field = page().locator('input[aria-label="Insert volume"]');
          await field.fill(String(moment.volume));
          await field.press("Tab");
          await until(v => window.scene.insertVolume === v, moment.volume, `Insert volume did not become ${moment.volume}%`, 4000);
          insert = moment.volume;
          await robot.focusEditor();
        }
        await setDivisor(moment.divisor);
        await showTime(moment.exact, { zoom });
        for (const layer of moment.layers) {
          const row = rows.get(laneKey(layer));
          await showRow(row);
          const count = await hitCount();
          const point = await at(moment.exact, row);
          await page().mouse.click(point.x, point.y);
          const landed = await until(([count, row]) => {
            const now = window.scene.moments.reduce((sum, m) => sum + m[3].length, 0);
            if (now !== count + 1) return null;
            return window.scene.moments.filter(([, , , list]) => list.some(([r]) => r === row)).map(([t, v, , list]) =>
              [t, list.find(([r]) => r === row)[3] ?? v]);
          }, [count, row], `no hit appeared on ${layer.lane} at ${moment.time} ms`, 4000);
          const here = landed.find(([t]) => Math.abs(t - moment.exact) <= 1);
          if (!here) placedOff.push(`${layer.lane} planned ${moment.time} ms`);
          else if (here[1] !== moment.volume) placedOff.push(`${layer.lane} at ${moment.time} ms has ${here[1]}%, not ${moment.volume}%`);
          placed++;
        }
      }
      expect(!placedOff.length, `${placedOff.length} hits off plan: ${placedOff.slice(0, 6).join("; ")}`);
      return `${placed} hits placed`;
    }, { covers: ["F020", "F023", "F030", "F031"] });
  }

  await step("The editor shows exactly the planned hits, every volume at 5% or more", async () => {
    const now = await hits();
    const want = plan.moments.flatMap(m => m.layers.map(l => ({ time: m.time, exact: m.exact, lane: l.lane, row: rows.get(laneKey(l)), volume: m.volume })));
    const missing = want.filter(w => !now.some(h => h.row === w.row && Math.abs(h.time - w.exact) <= 1));
    const extra = now.filter(h => !want.some(w => h.row === w.row && Math.abs(h.time - w.exact) <= 1));
    const low = now.filter(h => h.volume < 5);
    expect(!missing.length && !extra.length, `missing ${missing.slice(0, 5).map(w => `${w.lane}@${w.time}`).join(", ") || "none"}; ` +
      `extra ${extra.slice(0, 5).map(h => `${h.lane}@${h.time}`).join(", ") || "none"}`);
    expect(!low.length, `volumes under 5%: ${low.slice(0, 5).map(h => `${h.lane}@${h.time}=${h.volume}`).join(", ")}`);
    return `${now.length} hits on ${plan.moments.length} moments`;
  }, { covers: ["F049"] });

  await step("Save, through the save review", async () => {
    const seen = await save();
    savedHits = await hits();
    return seen.join("; ") || "no review shown";
  }, { stops: true, covers: ["F048", "F050"] });

  await step("The saved [Hitsounds] difficulty plays the plan, by osu!lazer's rules", async () => {
    const saved = soundMoments(readFileSync(hsPath));
    const files = new Set(readdirSync(mapFolder).map(name => name.toLowerCase()));
    const problems = [];
    for (const moment of plan.moments) {
      const found = saved.filter(s => Math.abs(s.time - moment.exact) <= 1);
      if (found.length !== 1) { problems.push(`${moment.time} ms: ${found.length} sound moments in the file`); continue; }
      // Judged by the sound osu! looks up (the map's file, or the skin's) with its volume, so an
      // index 1 with no file counts the same as index 0, as it sounds in osu!.
      const got = found[0].samples.map(s => `${playedFile(s, files)}@${s.volume}`).sort();
      const want = moment.layers.map(l => `${playedFile({ name: l.sound, bank: l.bank, index: l.index }, files)}@${moment.volume}`).sort();
      if (JSON.stringify(got) !== JSON.stringify(want)) problems.push(`${moment.time} ms (${moment.why}): plays ${got.join(" ")}, planned ${want.join(" ")}`);
    }
    const extra = saved.filter(s => !plan.moments.some(m => Math.abs(s.time - m.exact) <= 1));
    if (extra.length) problems.push(`${extra.length} unplanned sound moments, first at ${extra[0].time} ms`);
    expect(!problems.length, `${problems.length} problems: ${problems.slice(0, 6).join("; ")}`);
    return `${saved.length} sound moments match`;
  }, { covers: ["F048", "F050", "F051"] });

  await step("Every saved volume is 5% or more", async () => {
    const bytes = readFileSync(hsPath);
    const low = soundMoments(bytes).flatMap(m => m.samples.filter(s => s.volume < 5).map(s => `${m.time} ms ${describe(s)}`));
    // Lines the ranked map already had are the mapper's, not the editor's; only new ones count.
    const old = new Set(readOsu(referenceBytes).sections.TimingPoints ?? []);
    const lines = (readOsu(bytes).sections.TimingPoints ?? []).filter(line => !old.has(line) && Number(line.split(",")[5]) < 5);
    expect(!low.length && !lines.length, `under 5%: ${[...low, ...lines].slice(0, 6).join("; ")}`);
  }, { covers: ["F049"] });

  if (args.oracle) {
    await step("osu!lazer itself (the lazer oracle) agrees with the robot", async () => {
      const input = join(profile, "oracle-in");
      const output = join(profile, "oracle-out");
      mkdirSync(input, { recursive: true });
      cpSync(hsPath, join(input, basename(hsPath)));
      const run = spawnSync("dotnet", ["run", "-c", "Release", "--project", join(repo, "tools", "lazer-oracle"), "--", output, input],
        { cwd: repo, encoding: "utf8", shell: process.platform === "win32" });
      if (run.status !== 0) return { skip: `the oracle did not run: ${(run.stderr || run.stdout || "").trim().split("\n").at(-1)}` };
      const result = JSON.parse(readFileSync(join(output, basename(input), `${basename(hsPath)}.json`), "utf8"));
      const files = new Set(readdirSync(mapFolder).map(name => name.toLowerCase()));
      const theirs = result.hit_objects.map(o => ({ time: o.start_time, samples: o.samples.map(s => s.kind === "file"
        ? `map:${s.filename}@${s.volume}` : `${playedFile({ name: s.name, bank: s.bank, index: s.custom_sample_bank }, files)}@${s.volume}`).sort() }));
      const mine = soundMoments(readFileSync(hsPath)).map(m => ({ time: m.time, samples: m.samples.map(s => `${playedFile(s, files)}@${s.volume}`).sort() }));
      expect(theirs.length === mine.length, `lazer has ${theirs.length} objects, the robot ${mine.length}`);
      const differ = mine.filter((m, i) => Math.abs(m.time - theirs[i].time) > 0.5 || JSON.stringify(m.samples) !== JSON.stringify(theirs[i].samples));
      expect(!differ.length, `${differ.length} moments differ, first at ${differ[0]?.time} ms`);
      return `${theirs.length} objects agree`;
    }, { covers: ["F048"] });
  }

  await step("The mapset copy gained only the new difficulty; nothing else changed", async () => {
    await quit();
    const after = snapshot(mapFolder);
    const { added, removed, changed } = diffSnapshots(copyBefore, after);
    expect(!removed.length, `removed: ${removed.join(", ")}`);
    expect(!changed.length, `changed: ${changed.join(", ")}`);
    // Every planned layer is a plain sound a circle's line can hold, so Save needs no new sample file.
    const stray = added.filter(name => !hsPath || name !== basename(hsPath));
    expect(!stray.length, `new files besides the difficulty: ${stray.join(", ")}`);
    return hsPath ? `only ${basename(hsPath)} added` : "no difficulty was made; nothing else added";
  }, { always: true, covers: ["F048"] });

  await step("The original ranked mapset is byte-identical", async () => {
    const after = hashOriginal();
    const { added, removed, changed } = diffSnapshots(originalBefore, after);
    expect(!added.length && !removed.length && !changed.length, `added ${added.join(", ") || "none"}; removed ${removed.join(", ") || "none"}; changed ${changed.join(", ") || "none"}`);
    return `${after.size} files unchanged`;
  }, { always: true });

  await step("Reopening the saved difficulty shows the same hits", async () => {
    expect(savedHits && hsPath, "Save did not finish");
    await robot.launch(hsPath);
    await robot.answerKeyPreset(5000);
    await until(() => window.scene?.lanes?.length > 0, null, "the saved difficulty did not open", 60000);
    await sleep(500);
    // Save writes each circle at whole milliseconds, so a hit matches within 1 ms on the same lane.
    const now = await robot.hits();
    const same = (a, b) => a.lane === b.lane && a.index === b.index && Math.abs(a.time - b.time) <= 1;
    const missing = savedHits.filter(hit => !now.some(h => same(h, hit))).map(h => `${h.lane}@${h.time}`);
    const extra = now.filter(hit => !savedHits.some(h => same(h, hit))).map(h => `${h.lane}@${h.time}`);
    expect(!missing.length && !extra.length, `missing ${missing.slice(0, 5).join(", ") || "none"}; extra ${extra.slice(0, 5).join(", ") || "none"}`);
    return `${now.length} hits`;
  }, { covers: ["F050", "F085"] });

  await step("No errors after the last step", async () => {}, { always: true });
} finally {
  await quit().catch(() => {});
}

const { failed, passed, total } = robot.finish(args.json ? resolve(args.json) : null);
writeFileSync(join(profile, "plan.json"), JSON.stringify(plan, null, 2));
console.log(`hitsound 30s: ${passed} of ${total} steps passed${failed ? `, ${failed} failed` : ""} (profile ${profile})`);
if (!args["keep-profile"] && !failed) rmSync(profile, { recursive: true, force: true });
process.exit(failed ? 1 : 0);
