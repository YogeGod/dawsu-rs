// The long robot mapper: uses the real built editor on a copy of a ranked map and checks that each
// feature does what it should (tools/robot/features.mjs), not only that it can be clicked.
//
// node tools/feature-check.mjs --exe <dawsu-tauri.exe> (--map <ranked .osu> | --dataset <folder>)
//   [--pick <n>] [--seed <n>] [--only <regex>] [--skip <regex>] [--json <file>] [--keep-profile] [--allow-stale]
// node tools/feature-check.mjs --features [--markdown]   (the feature list and which steps cover it)
//
//   --map/--dataset/--pick/--seed  as tools/hitsound-30s.mjs; the mapset needs a second
//                  osu!standard difficulty (for Copy hitsounds)
//   --only/--skip  run only, or leave out, the steps whose name matches
//
// How a step judges: it acts only through the screen (clicks, keys, menu rows, dialogs, the Ctrl+K
// palette), then compares the result with an answer worked out on its own: the snap grid from the
// map's red lines, what osu!lazer plays from the saved file (tools/robot/osu-sounds.mjs), the files
// on disk. The editor's view is moved with a view action, never an edit action. Nothing plays:
// Audition is switched off first and playback is never started.
//
// This is not the merge gate (tools/everyday-check.mjs is); it is the run that tells whether the
// features still work as intended. Windows only in practice; see tools/robot/harness.mjs.
import { readdirSync, readFileSync, rmSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { FEATURES, markdown } from "./robot/features.mjs";
import { checkFreshness, createRobot, diffSnapshots, parseArgs, sleep, snapshot } from "./robot/harness.mjs";
import { pickAndCopy } from "./robot/maps.mjs";
import { controlPoints, readOsu, soundMoments } from "./robot/osu-sounds.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = parseArgs();

// The steps, in order: [name, covers, body]. Bodies are defined below, once the robot exists.
const STEPS = [];
const define = (name, covers, body) => STEPS.push({ name, covers, body });

// ---- Helpers that do not need the editor ----------------------------------------------------------

// The closest grid line to `time` at 1/divisor of a beat, from the red lines alone (lazer's
// GetClosestSnappedTime, ControlPointInfo.cs:169-186: within the red line's section, or the next red
// line when that is closer).
function closestGrid(lines, time, divisor) {
  const red = lines.timingAt(time);
  const step = red.beatLength / divisor;
  const snapped = red.time + Math.round((time - red.time) / step) * step;
  const next = lines.timing.find(point => point.time > red.time);
  return next && Math.abs(time - next.time) <= Math.abs(time - snapped) ? next.time : snapped;
}
// The sounds of a moment as "bank-name" families with the moment's hitnormal volume.
const families = samples => samples.filter(s => s.name !== "file").map(s => `${s.bank}-${s.name}`).sort();

if (args.features) {
  // Static list: steps' covers are known without running anything. tools/hitsound-30s.mjs's steps
  // are read from its source (step("name", ..., { covers: [...] })).
  defineSteps(null, null);
  const scenario = readFileSync(join(repo, "tools", "hitsound-30s.mjs"), "utf8");
  for (const match of scenario.matchAll(/await step\(`?"?([^"`]+)["`],(?:(?!await step\()[\s\S])*?covers: \[([^\]]*)\]/g)) {
    STEPS.push({ name: `hitsound-30s: ${match[1]}`, covers: [...match[2].matchAll(/"([FN]\d{3})"/g)].map(m => m[1]) });
  }
  if (args.markdown) process.stdout.write(markdown(STEPS));
  else for (const row of FEATURES) {
    const steps = STEPS.filter(step => step.covers.includes(row.id)).map(step => step.name);
    console.log(`${row.id} ${row.feature} — ${steps.length ? steps.join("; ") : row.manual ? "by hand: " + row.manual : "NOT CHECKED"}`);
  }
  process.exit(0);
}

if (!args.exe) throw new Error("--exe is required: the built dawsu-tauri.exe");
if (!args.map && !args.dataset) throw new Error("give --map <ranked .osu> or --dataset <folder of ranked mapsets>");
const exe = resolve(args.exe);
checkFreshness(repo, exe, Boolean(args["allow-stale"]));
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..*/, "");
const profile = resolve(join(repo, "target", "feature-check", `${stamp}-${process.pid}`));
const picked = pickAndCopy({ map: args.map, dataset: args.dataset, pick: args.pick, seed: args.seed, profile, siblings: true });
console.log(`map: ${picked.original}${picked.isOsz ? ` (from ${basename(picked.source)})` : ""}${picked.note}`);
const robot = createRobot({ exe, profile });
defineSteps(robot, picked);

const only = args.only ? new RegExp(args.only, "i") : null;
const skip = args.skip ? new RegExp(args.skip, "i") : null;
try {
  for (const { name, covers, body, stops, always } of STEPS) {
    if ((only && !only.test(name) && !always) || (skip && skip.test(name))) continue;
    await robot.step(name, body, { covers, stops, always });
  }
} finally {
  await robot.quit().catch(() => {});
}
const { failed, passed, total } = robot.finish(args.json ? resolve(args.json) : null);
const covered = new Set(robot.results.filter(r => r.status === "PASS").flatMap(r => r.covers));
console.log(`features checked and working: ${covered.size} of ${FEATURES.length}`);
console.log(`feature check: ${passed} of ${total} steps passed${failed ? `, ${failed} failed` : ""} (profile ${profile})`);
if (!args["keep-profile"] && !failed) rmSync(profile, { recursive: true, force: true });
process.exit(failed ? 1 : 0);

// ---- The steps ------------------------------------------------------------------------------------
function defineSteps(robot, picked) {
  // With no robot (--features) only names and covers are read.
  const r = robot ?? {};
  const { expect, until, read, hits, hitCount, namedHits, laneRow, showTime, showRow, at, fromMenu, openMenu, menuRow,
    palette, pickTool, setDivisor, dialogShown, dialogButton, answerDialogs, save } = r;
  const page = () => r.page();
  const p = picked ?? {};
  const mapFolder = p.mapFolder;
  const copy = p.copy;
  let copyBefore = null;
  let osuBefore = null;
  let lines = null;
  let silent = false;
  let scratch = null;        // a lane row and an empty time for throwaway hits
  let lastSaved = null;
  const created = [];        // .osu files the run itself created in the mapset copy
  const needsSilence = () => silent ? null : { skip: "Audition could not be switched off" };

  // Place one hit with the Draw tool on `row` at `time`; returns the time it landed at.
  async function draw(time, row) {
    await pickTool("Draw");
    await showTime(time, { zoom: Math.max(0.5, await read(() => window.scene.zoom)) });
    await showRow(row);
    const count = await hitCount();
    const landedOn = () => until(([count, row]) => {
      if (window.scene.moments.reduce((sum, m) => sum + m[3].length, 0) !== count + 1) return null;
      return window.scene.moments.filter(([, , , list]) => list.some(([r]) => r === row)).map(([t]) => t);
    }, [count, row], `no hit appeared at ${time} ms`, 4000);
    // One retry: a click right after a tool or view change can be swallowed.
    for (let attempt = 0; ; attempt++) {
      const point = await at(time, row);
      await page().mouse.click(point.x, point.y);
      try { return await landedOn(); } catch (error) {
        if (attempt >= 1 || await hitCount() !== count) throw error;
        await sleep(300);
      }
    }
  }
  // Click "Review output" on an output dialog, then wait until the job has really started (busy, or
  // the next dialog) before answering dialogs: right after the click `view.busy` is still false.
  async function reviewOutput(from) {
    await dialogButton("Review output").click();
    await until(from => window.view.busy || window.view.review
      || (document.getElementById("dialog").open && document.getElementById("dialog-title").textContent !== from),
      from, "the output job did not start", 10000).catch(() => {});
  }
  // Add lane (the row under the lane list): show a lane with no hits for throwaway hits. Returns its row or -1.
  async function addScratchLane() {
    const shownBefore = await read(() => Array.from(document.querySelectorAll("#lanes [data-lane-row]"), el => el.dataset.laneRow));
    await page().click("#add-lane");
    await dialogShown("Add lane");
    if (!(await dialogButton("Add").count())) { await page().keyboard.press("Escape"); return -1; }
    await dialogButton("Add").click();
    return until(([n, shown]) => {
      const rows = Array.from(document.querySelectorAll("#lanes [data-lane-row]"), el => el.dataset.laneRow);
      const fresh = rows.find(row => !shown.includes(row) && !window.scene.lanes[Number(row)]?.hitCount);
      return fresh !== undefined ? { row: Number(fresh) } : null;
    }, [0, shownBefore], "Add lane showed no new empty lane", 6000).then(found => found.row, () => -1);
  }
  async function clickHit(time, row) {
    await showTime(time);
    await showRow(row);
    const point = await at(time, row);
    await page().mouse.click(point.x, point.y);
  }
  async function selectOnly(time, row) {
    await pickTool("Select");
    await r.focusEditor();
    await page().keyboard.press("Escape");
    await clickHit(time, row);
    await until(([time, row]) => window.scene.selection?.count === 1 && window.scene.moments.some(([t, , , list]) =>
      Math.abs(t - time) < 1 && list.some(([r, s]) => r === row && s)), [time, row], `the hit at ${time} ms was not selected`, 4000);
  }

  STEPS.length = 0;

  define("Start with no map: the start screen and the key preset question", ["F070", "F075", "N002"], async () => {
    copyBefore = snapshot(mapFolder);
    osuBefore = readFileSync(copy);
    lines = controlPoints(readOsu(osuBefore));
    await r.launch(null);
    await until(() => !document.getElementById("home").hidden && !!document.getElementById("home-open")
      && !!document.getElementById("home-setup"), null, "the start screen did not show", 15000);
    const preset = page().getByRole("button", { name: "DAW-like" });
    if (await preset.count() && await preset.first().isVisible()) {
      await preset.first().click();
      await until(() => window.view.keyPreset === "DAW", null, "DAW-like did not set the key preset", 4000);
      await until(() => document.querySelector('#home-setup [data-step="shortcut-style"].done') !== null,
        null, "the Ready checklist did not mark Shortcuts done", 4000);
      return "asked once, DAW-like chosen, checklist updated";
    }
    return "no preset question (already answered)";
  });
  STEPS.at(-1).stops = true;

  define("Open the copied ranked map: every object's sounds show as lanes, as osu! plays them", ["F001", "F012", "F013"], async () => {
    await r.quit();
    await r.launch(copy);
    await until(() => window.scene?.lanes?.length > 0 && document.querySelectorAll("#lanes [data-lane-row]").length > 0,
      null, "the map did not open", 60000);
    const shown = await hits();
    const expected = soundMoments(osuBefore);
    const byTime = new Map();
    for (const hit of shown) {
      const key = Math.round(hit.time);
      if (!byTime.has(key)) byTime.set(key, new Set());
      if (hit.family) byTime.get(key).add(hit.family);
    }
    const problems = [];
    for (const moment of expected) {
      const want = new Set(families(moment.samples));
      if (!want.size) continue;
      const got = byTime.get(Math.round(moment.time)) ?? byTime.get(Math.round(moment.time) - 1) ?? byTime.get(Math.round(moment.time) + 1);
      const missing = [...want].filter(name => !got?.has(name));
      if (missing.length) problems.push(`${Math.round(moment.time)} ms (${moment.node}) misses ${missing.join("+")}`);
    }
    expect(!problems.length, `${problems.length} of ${expected.length} sound moments differ from osu!: ${problems.slice(0, 5).join("; ")}`);
    return `${expected.length} sound moments, ${shown.length} hits, all match`;
  });
  STEPS.at(-1).stops = true;

  define("Menus list their rows with the active preset's shortcuts; the Help menu is complete", ["F072", "N003"], async () => {
    const empty = [];
    for (const menu of ["File", "Edit", "Playback", "Map", "View", "Help"]) {
      await openMenu(menu);
      const rows = await read(() => Array.from(document.querySelectorAll("#editor-menu [role^=menuitem]"), row => row.textContent.trim()));
      if (rows.length < 3) empty.push(`${menu}: ${rows.length} rows`);
      if (menu === "Help") for (const label of ["Quick start tour", "Shortcuts", "Wiki", "FAQ", "Report a bug…", "Open crash log folder"]) {
        if (!rows.some(row => row.startsWith(label))) empty.push(`Help lacks ${label}`);
      }
      if (menu === "Edit") {
        // The keys arrive a moment after the menu opens (editor_shortcuts), as in the gate.
        let keys = "";
        for (let i = 0; i < 50 && !keys; i++) {
          keys = (await menuRow("Undo").locator(".menu-keys, kbd, [class*=key]").allTextContents().catch(() => [])).join("").trim();
          if (!keys) await sleep(100);
        }
        if (!/Ctrl\+Z|Control\+Z/i.test(keys)) empty.push(`Edit › Undo shows "${keys}", not Ctrl+Z (DAW-like)`);
      }
      await page().keyboard.press("Escape");
    }
    expect(!empty.length, empty.join("; "));
  });

  define("Switch Audition off so nothing plays", [], async () => {
    if (await read(() => window.view.audition)) await page().locator('button[aria-label="Audition"]').click({ timeout: 5000 });
    await until(() => window.view.audition === false, null, "Audition is still on");
    silent = true;
  });
  STEPS.at(-1).stops = true;

  define("Snap: Grid mode puts a click on the closest grid line at 1/4 and 1/3", ["F030", "F031", "F020"], async () => {
    if (!silent) return needsSilence();
    if (await page().locator("#show-default-skin-lanes").isVisible().catch(() => false)) await page().click("#show-default-skin-lanes");
    if (await read(() => window.scene.snapMode === "objects")) {
      await page().click("#snap-mode-toggle");
      await until(() => window.scene.snapMode === "grid", null, "the snap stayed on Objects");
    }
    // A lane the map does not use, so throwaway hits never touch the map's own sounds.
    // Only a lane with a header on screen can be clicked; slider slide/tick rows have none.
    let fallback = await read(() => {
      const shown = lane => !!document.querySelector(`#lanes [data-lane-row="${lane}"]`);
      const lanes = window.scene.lanes;
      const plain = (lane, row) => lane.hitCount === 0 && shown(row) && !/slider(slide|tick|whistle)/.test(lane.name);
      const whistle = lanes.findIndex((lane, row) => lane.name === "drum-hitwhistle" && (lane.index ?? 0) === 0 && plain(lane, row));
      return whistle >= 0 ? whistle : lanes.findIndex(plain);
    });
    if (fallback < 0) fallback = await addScratchLane();
    expect(fallback >= 0, "no empty lane with a header on screen for throwaway hits, and Add lane made none");
    const objects = soundMoments(osuBefore);
    // A spot away from every object (the widest gap in the middle half of the map), so a drawn hit
    // makes a moment of its own and takes the Insert volume (Composer::add joins a nearby moment).
    let mid = objects[Math.floor(objects.length / 2)].time;
    let widest = 0;
    for (let i = Math.floor(objects.length / 4); i < Math.floor(objects.length * 3 / 4); i++) {
      const gap = objects[i + 1].time - objects[i].time;
      if (gap > widest) { widest = gap; mid = closestGrid(lines, (objects[i].time + objects[i + 1].time) / 2, 4); }
    }
    expect(widest >= lines.timingAt(mid).beatLength, `no gap of a beat between objects (widest ${Math.round(widest)} ms)`);
    scratch = { row: fallback, time: mid };
    const results = [];
    for (const divisor of [4, 3]) {
      await setDivisor(divisor);
      const red = lines.timingAt(mid);
      // A third of the way between two grid lines: the closest line is the earlier one.
      const target = closestGrid(lines, mid, divisor) + red.beatLength / divisor / 3;
      const want = closestGrid(lines, target, divisor);
      const before = await hits();
      await draw(target, fallback);
      const after = await hits();
      const added = after.filter(h => h.row === fallback && !before.some(b => b.row === fallback && b.time === h.time));
      expect(added.length === 1, `1/${divisor}: ${added.length} new hits`);
      expect(Math.abs(added[0].time - want) <= 1, `1/${divisor}: a click at ${target.toFixed(1)} ms landed at ${added[0].time} ms, the closest grid line is ${want.toFixed(1)} ms`);
      results.push(`1/${divisor}: ${target.toFixed(1)} → ${added[0].time} ms`);
      await page().keyboard.press("Control+Z");
      await until(count => window.scene.moments.reduce((s, m) => s + m[3].length, 0) === count, before.length, "Undo did not remove the test hit");
    }
    await setDivisor(4);
    return results.join("; ");
  });

  define("Insert volume: a new hit takes it, and it never goes under 5%", ["F023", "F049"], async () => {
    if (!scratch) return { skip: "no scratch lane" };
    const field = page().locator('input[aria-label="Insert volume"]');
    await field.fill("37");
    await field.press("Tab");
    await until(() => window.scene.insertVolume === 37, null, "Insert volume did not become 37%");
    await r.focusEditor();
    const before = await hitCount();
    await draw(scratch.time, scratch.row);
    const hit = (await hits()).find(h => h.row === scratch.row && Math.abs(h.time - closestGrid(lines, scratch.time, 4)) <= 1);
    expect(hit?.volume === 37, `the new hit has ${hit?.volume}%, not 37%`);
    await field.fill("2");
    await field.press("Tab");
    await sleep(400);
    const insert = await read(() => window.scene.insertVolume);
    expect(insert >= 5, `typing 2% set Insert volume to ${insert}%`);
    await field.fill("100");
    await field.press("Tab");
    await r.focusEditor();
    await page().keyboard.press("Control+Z");
    await until(count => window.scene.moments.reduce((s, m) => s + m[3].length, 0) === count, before, "Undo did not remove the test hit");
    return `new hit 37%; typing 2% gives ${insert}%`;
  });

  define("Undo and Redo restore the exact hits, and History lists each edit", ["F026"], async () => {
    if (!scratch) return { skip: "no scratch lane" };
    const start = await namedHits();
    // scene.history holds the done steps then the undone ones; historyPosition counts the done ones.
    const position = await read(() => window.scene.historyPosition);
    await draw(scratch.time, scratch.row);
    const placed = await namedHits();
    const after = await read(() => ({ position: window.scene.historyPosition, length: window.scene.history.length }));
    expect(after.position === position + 1 && after.length === after.position, `History went from step ${position} to ${after.position} (${after.length} listed) after one edit`);
    await r.focusEditor();
    await page().keyboard.press("Control+Z");
    await until(n => window.scene.moments.reduce((s, m) => s + m[3].length, 0) === n, start.length, "Undo did not remove the hit");
    expect(JSON.stringify(await namedHits()) === JSON.stringify(start), "Undo did not give back the exact hits");
    await page().keyboard.press("Control+Shift+Z");
    await sleep(400);
    let redone = await namedHits();
    if (JSON.stringify(redone) !== JSON.stringify(placed)) {
      await page().keyboard.press("Control+Y");
      await sleep(400);
      redone = await namedHits();
    }
    expect(JSON.stringify(redone) === JSON.stringify(placed), "Redo did not give back the placed hit");
    await page().keyboard.press("Control+Z");
    await sleep(300);
  });

  define("Erase tool removes the clicked hit and nothing else", ["F020"], async () => {
    if (!scratch) return { skip: "no scratch lane" };
    const start = await namedHits();
    const landed = await draw(scratch.time, scratch.row);
    const time = landed.find(t => Math.abs(t - closestGrid(lines, scratch.time, 4)) <= 1);
    await pickTool("Erase");
    await clickHit(time, scratch.row);
    await until(count => window.scene.moments.reduce((s, m) => s + m[3].length, 0) === count, start.length, "Erase did not remove the hit");
    expect(JSON.stringify(await namedHits()) === JSON.stringify(start), "Erase changed other hits");
    await pickTool("Select");
  });

  define("Mute tool silences a hit in dawsu! only", ["F020", "F017"], async () => {
    if (!scratch) return { skip: "no scratch lane" };
    const startAt = await read(() => window.scene.historyPosition);
    const landed = await draw(scratch.time, scratch.row);
    const time = landed.find(t => Math.abs(t - closestGrid(lines, scratch.time, 4)) <= 1);
    const drawnAt = await read(() => window.scene.historyPosition);
    await pickTool("Mute");
    await clickHit(time, scratch.row);
    await until(([time, row]) => window.scene.moments.some(([t, , , list]) => t === time && list.some(([r, , muted]) => r === row && muted)),
      [time, scratch.row], "the Mute tool did not mark the hit muted");
    const mutedAt = await read(() => window.scene.historyPosition);
    await pickTool("Select");
    await r.focusEditor();
    // Undo back to where the step started, by History position, not by a count of presses: an
    // extra Undo would take away earlier steps (Show 12 default lanes, the scratch lane).
    for (let i = 0; i < 4 && await read(() => window.scene.historyPosition) > startAt; i++) {
      await page().keyboard.press("Control+Z");
      await sleep(250);
    }
    expect(await read(() => window.scene.historyPosition) === startAt, "Undo did not go back to where the step started");
    return mutedAt === drawnAt ? "muted; note: Mute adds no Undo step" : "muted, and Undo takes it back";
  });

  define("Copy, paste at the playhead, duplicate and select all", ["F021", "F038"], async () => {
    if (!scratch) return { skip: "no scratch lane" };
    const start = await hitCount();
    const landed = await draw(scratch.time, scratch.row);
    const time = landed.find(t => Math.abs(t - closestGrid(lines, scratch.time, 4)) <= 1);
    await selectOnly(time, scratch.row);
    await fromMenu("Edit", "Copy");
    await fromMenu("Playback", "Go to start");
    await until(() => window.view.position === 0, null, "Go to start did not put the playhead at 0");
    await fromMenu("Edit", "Paste at playhead");
    await until(count => window.scene.moments.reduce((s, m) => s + m[3].length, 0) === count + 2, start, "Paste did not add one hit");
    const pasted = (await hits()).filter(h => h.row === scratch.row && Math.abs(h.time - time) > 1);
    expect(pasted.length === 1 && pasted[0].time < 50, `the pasted hit is at ${pasted.map(h => h.time).join(", ")} ms, not at the playhead (0)`);
    await selectOnly(time, scratch.row);
    await fromMenu("Edit", "Duplicate");
    await until(count => window.scene.moments.reduce((s, m) => s + m[3].length, 0) === count + 3, start, "Duplicate did not add one hit");
    const dup = (await hits()).filter(h => h.row === scratch.row && h.time > time + 1);
    expect(dup.length === 1, `Duplicate put ${dup.length} hits after the original`);
    await fromMenu("Edit", "Select all");
    const selected = await until(() => window.scene.selection?.count > 0 && window.scene.selection.count, null, "Select all selected nothing");
    expect(selected === await hitCount(), `Select all selected ${selected} of ${await hitCount()} hits`);
    await page().keyboard.press("Escape");
    for (let i = 0; i < 3; i++) await page().keyboard.press("Control+Z");
    await until(count => window.scene.moments.reduce((s, m) => s + m[3].length, 0) === count, start, "Undo did not take the three hits away");
    return `pasted at ${pasted[0].time} ms, duplicated to ${dup[0].time} ms, ${selected} selected`;
  });

  define("Quantize moves an off-grid hit onto the grid", ["F022"], async () => {
    if (!scratch) return { skip: "no scratch lane" };
    const start = await hitCount();
    // Snap off for the placement, so the hit sits between grid lines.
    await page().keyboard.press("Escape");
    const snapOn = await read(() => window.scene.snap);
    if (snapOn) await read(() => window.act({ kind: "snap", on: false, mode: window.scene.snapMode, beats: window.scene.beats, divisor: window.scene.divisor }));
    const red = lines.timingAt(scratch.time);
    const target = closestGrid(lines, scratch.time, 4) + red.beatLength / 4 / 5;
    const landed = await draw(target, scratch.row);
    const off = landed.find(t => Math.abs(t - target) < red.beatLength / 8);
    if (snapOn) await read(() => window.act({ kind: "snap", on: true, mode: window.scene.snapMode, beats: window.scene.beats, divisor: window.scene.divisor }));
    expect(off !== undefined, `the hit did not land near ${target.toFixed(1)} ms`);
    await selectOnly(off, scratch.row);
    await fromMenu("Edit", "Quantize…");
    await dialogShown("Quantize");
    await dialogButton("Quantize").click();
    const want = closestGrid(lines, off, 4);
    await until(([row, want]) => window.scene.moments.some(([t, , , list]) => Math.abs(t - want) <= 1 && list.some(([r]) => r === row)),
      [scratch.row, want], `the hit at ${off} ms was not quantized to ${want.toFixed(1)} ms`);
    await page().keyboard.press("Control+Z");
    await page().keyboard.press("Control+Z");
    await until(count => window.scene.moments.reduce((s, m) => s + m[3].length, 0) === count, start, "Undo did not take the hit away");
    return `${off} → ${want.toFixed(1)} ms`;
  });

  define("Protect hitnormals: Delete leaves a hitnormal on an object", ["F024"], async () => {
    const target = (await hits()).find(h => /-hitnormal$/.test(h.family ?? ""));
    expect(target, "the map shows no hitnormal");
    await fromMenu("Edit", "Protect hitnormals from deletion");
    await until(() => window.scene.protect === true, null, "Protect did not turn on");
    await selectOnly(target.time, target.row);
    const count = await hitCount();
    await r.focusEditor();
    await page().keyboard.press("Delete");
    await sleep(600);
    expect(await hitCount() === count, "Delete removed a protected hitnormal");
    await fromMenu("Edit", "Protect hitnormals from deletion");
    await until(() => window.scene.protect === false, null, "Protect did not turn off");
  });

  define("Bookmarks: add at the playhead, then Next bookmark jumps to it", ["F034", "F038"], async () => {
    const objects = soundMoments(osuBefore);
    const time = Math.round(objects[Math.floor(objects.length / 3)].time);
    await read(time => window.act({ kind: "seek", ms: time }, false), time).catch(() => {});
    await sleep(300);
    const position = await read(() => window.view.position);
    expect(position > 0, "the playhead did not move from 0");
    const before = await read(() => window.scene.bookmarks.length);
    // Map › Add bookmark adds one at the playhead straight away, with no dialog and no label.
    await fromMenu("Map", "Add bookmark");
    await until(([n, at]) => window.scene.bookmarks.length === n + 1 && window.scene.bookmarks.some(b => Math.abs(b.time - at) <= 1),
      [before, position], "the bookmark was not added at the playhead");
    expect(!(await read(() => document.getElementById("dialog").open)), "Add bookmark opened a dialog");
    await fromMenu("Playback", "Go to start");
    await until(() => window.view.position === 0, null, "Go to start did not put the playhead at 0");
    for (let i = 0; i < 20; i++) {
      await fromMenu("Map", "Next bookmark");
      await sleep(250);
      if (Math.abs(await read(() => window.view.position) - position) <= 1) return `bookmark at ${position} ms`;
    }
    throw new Error(`Next bookmark never reached ${position} ms`);
  });

  define("Locators to selection: L and R take the selection's first and last time", ["F035"], async () => {
    const all = (await hits()).filter(h => /-hitnormal$/.test(h.family ?? ""));
    const a = all[5], b = all[12];
    expect(a && b, "too few hits");
    await selectOnly(a.time, a.row);
    await page().keyboard.down("Shift");
    await clickHit(b.time, b.row);
    await page().keyboard.up("Shift");
    await fromMenu("Playback", "Locators to selection");
    await until(([a, b]) => Math.abs(window.view.left - a) <= 1 && Math.abs(window.view.right - b) <= 1, [a.time, b.time],
      `L/R did not become ${a.time}/${b.time}`);
    await page().keyboard.press("Escape");
  });

  define("Used lanes only hides empty lanes and shows them again", ["F019"], async () => {
    // The lane list on screen (scene.laneOrder keeps every row; the filter hides headers).
    const visible = () => read(() => Array.from(document.querySelectorAll("#lanes [data-lane-row]"),
      el => window.scene.lanes[Number(el.dataset.laneRow)]).filter(Boolean));
    const all = await visible();
    if (!all.some(lane => !lane.hitCount)) return { skip: "every lane on screen already has hits, so there is nothing to hide" };
    let on = false;
    try {
      await fromMenu("View", "Used lanes only");
      on = true;
      const used = await until(n => {
        const shown = Array.from(document.querySelectorAll("#lanes [data-lane-row]"), el => window.scene.lanes[Number(el.dataset.laneRow)]).filter(Boolean);
        return shown.length !== n && shown;
      }, all.length, "Used lanes only changed nothing");
      const empty = used.filter(lane => !lane.hitCount);
      expect(!empty.length, `empty lanes still shown: ${empty.map(l => l.name).join(", ")}`);
      await fromMenu("View", "Used lanes only");
      on = false;
      await until(n => document.querySelectorAll("#lanes [data-lane-row]").length === n, all.length, "the hidden lanes did not come back");
      return `${all.length} → ${used.length} → ${all.length} lanes`;
    } finally {
      // Never leave the filter on: later steps need the empty lanes.
      if (on) await fromMenu("View", "Used lanes only").catch(() => {});
    }
  });

  define("Collapse sample layers folds the numbered layers without touching hits", ["F018"], async () => {
    const start = await namedHits();
    const rows = () => read(() => document.querySelectorAll("#lanes [data-lane-row]").length);
    // The families open now, so exactly those are unfolded again afterwards.
    const open = await read(() => Array.from(document.querySelectorAll('#lanes .lane-disclosure[aria-expanded="true"]'), b => b.getAttribute("aria-label")));
    const before = await rows();
    await fromMenu("View", "Collapse sample layers");
    await sleep(500);
    const folded = await rows();
    expect(JSON.stringify(await namedHits()) === JSON.stringify(start), "collapsing changed the hits");
    // The command only folds (nothing in View unfolds): unfold with each family's arrow, as a mapper would.
    // The arrow's label follows its state: "Collapse … layers" while open, "Expand … layers" folded.
    for (const label of open.map(text => text.replace(/^Collapse\b/, "Expand"))) {
      const arrow = page().locator(`#lanes .lane-disclosure[aria-expanded="false"][aria-label="${label.replace(/"/g, '\\"')}"]`).first();
      if (await arrow.count()) { await arrow.click(); await sleep(80); }
    }
    await until(n => document.querySelectorAll("#lanes [data-lane-row]").length === n, before, "the family arrows did not bring every layer back", 4000);
    expect(JSON.stringify(await namedHits()) === JSON.stringify(start), "unfolding changed the hits");
    if (!open.length) return "no numbered layers to fold";
    expect(folded < before, `Collapse sample layers hid nothing (${before} rows before and after)`);
    return `${before} → ${folded} → ${before} rows`;
  });

  define("Hitsounding mode turns on and off from the Map menu", ["F050"], async () => {
    const was = await read(() => window.scene.hs);
    await fromMenu("Map", "Hitsounding mode");
    await until(was => window.scene.hs === !was, was, "Hitsounding mode did not change");
    await fromMenu("Map", "Hitsounding mode");
    await until(was => window.scene.hs === was, was, "Hitsounding mode did not change back");
  });

  define("F1 help lists the shortcuts and mouse actions, and search filters them", ["F082"], async () => {
    await fromMenu("Help", "Shortcuts");
    await dialogShown("Shortcuts and mouse actions");
    const all = await read(() => document.querySelectorAll("#dialog-body p").length);
    await page().locator('#dialog-body input[type="search"]').fill("undo");
    await sleep(300);
    const some = await read(() => Array.from(document.querySelectorAll("#dialog-body p"), p => p.textContent));
    expect(all >= 30, `only ${all} entries`);
    expect(some.length > 0 && some.length < all && some.every(t => /undo/i.test(t)), `search for "undo" left ${some.length} of ${all}: ${some.slice(0, 3).join(" / ")}`);
    await dialogButton("Close").click();
    return `${all} entries, ${some.length} for "undo"`;
  });

  // A whistle on a plain circle and on a slider head, saved: the file must then play exactly what
  // the editor shows, and only those objects' lines may change.
  let edited = [];
  define("Add a whistle to a circle and to a slider head", ["F020", "F048"], async () => {
    if (!silent) return needsSilence();
    const objects = soundMoments(osuBefore);
    const circle = objects.find(m => m.node === "circle" && !m.samples.some(s => s.name === "hitwhistle") && m.time > 2000);
    const head = objects.find(m => m.node === "head" && !m.samples.some(s => s.name === "hitwhistle") && m.time > 2000);
    expect(circle && head, "the map has no plain circle or slider head");
    for (const moment of [circle, head]) {
      // One object has one addition bank: an existing addition (a clap, a finish) sets it, else the hitnormal's.
      const addition = moment.samples.find(s => s.name !== "hitnormal" && s.name !== "file");
      const bank = addition?.bank ?? moment.samples.find(s => s.name === "hitnormal")?.bank ?? "normal";
      // The object's own custom index when the mapset has that whistle file, else the skin's (index 0).
      const index = moment.samples[0].index ?? 0;
      const file = `${bank}-hitwhistle${index >= 2 ? index : ""}`;
      const useIndex = index >= 1 && existsSample(file) ? index : 0;
      const row = await laneRow(useIndex >= 2 ? file : `${bank}-hitwhistle`, useIndex);
      expect(row >= 0, `no ${bank}-hitwhistle lane`);
      await read(() => window.act({ kind: "snap", on: true, mode: "objects", beats: window.scene.beats, divisor: window.scene.divisor }));
      await draw(moment.time, row);
      edited.push(moment);
    }
    await read(() => window.act({ kind: "snap", on: true, mode: "grid", beats: window.scene.beats, divisor: window.scene.divisor }));
    return edited.map(m => `${m.node} at ${Math.round(m.time)} ms`).join(", ");
  });
  const existsSample = name => ["wav", "ogg", "mp3"].some(ext => readdirSync(mapFolder).some(file => file.toLowerCase() === `${name}.${ext}`));

  define("Save writes only the edited objects, and the file plays what the editor showed", ["F048", "F049", "F025"], async () => {
    if (!edited.length) return { skip: "nothing was edited" };
    const shown = await hits();
    await save();
    lastSaved = readFileSync(copy);
    // Outside [TimingPoints] and [HitObjects] nothing may change; in [HitObjects] only the edited
    // objects' lines (Save may add green lines for an index or volume a line cannot hold).
    const was = readOsu(osuBefore).sections;
    const now = readOsu(lastSaved).sections;
    const otherSections = Object.keys({ ...was, ...now }).filter(name => !["TimingPoints", "HitObjects"].includes(name)
      && JSON.stringify(was[name]) !== JSON.stringify(now[name]));
    expect(!otherSections.length, `sections changed: ${otherSections.join(", ")}`);
    const objectsBefore = was.HitObjects ?? [];
    const objectsAfter = now.HitObjects ?? [];
    expect(objectsBefore.length === objectsAfter.length, `object count ${objectsBefore.length} → ${objectsAfter.length}`);
    const editedObjects = new Set(edited.map(m => m.object));
    const strayObjects = objectsBefore.map((line, i) => [i, line, objectsAfter[i]]).filter(([i, a, b]) => a !== b && !editedObjects.has(i));
    expect(!strayObjects.length, `objects not edited changed: ${strayObjects.slice(0, 3).map(([, a, b]) => `${a} → ${b}`).join("; ")}`);
    const changed = objectsBefore.filter((line, i) => line !== objectsAfter[i]);
    const saved = soundMoments(lastSaved);
    const problems = [];
    for (const moment of edited) {
      const found = saved.find(s => Math.abs(s.time - moment.time) <= 1);
      if (!found?.samples.some(s => s.name === "hitwhistle")) problems.push(`${Math.round(moment.time)} ms (${moment.node}) plays no whistle`);
    }
    const low = saved.flatMap(m => m.samples.filter(s => s.volume < 5).map(() => Math.round(m.time)));
    // Every moment the editor showed on an object still plays its families.
    for (const moment of saved) {
      const lanes = new Set(shown.filter(h => Math.abs(h.time - moment.time) <= 1).map(h => h.family));
      const missing = families(moment.samples).filter(name => !lanes.has(name));
      if (missing.length && lanes.size) problems.push(`${Math.round(moment.time)} ms plays ${missing.join("+")} the editor did not show`);
    }
    expect(!problems.length, problems.slice(0, 5).join("; "));
    expect(!low.length, `volumes under 5% at ${low.slice(0, 5).join(", ")} ms`);
    return `${changed.length} lines changed`;
  });

  define("A second Save after another slider hitsound change works (issue #132)", ["N017", "F048"], async () => {
    const head = edited.find(m => m.node === "head");
    if (!head || !lastSaved) return { skip: "no saved slider edit" };
    const bank = head.samples.find(s => s.name === "hitnormal")?.bank ?? "normal";
    const row = await laneRow(`${bank}-hitclap`, 0);
    expect(row >= 0, `no ${bank}-hitclap lane`);
    await read(() => window.act({ kind: "snap", on: true, mode: "objects", beats: window.scene.beats, divisor: window.scene.divisor }));
    await draw(head.time, row);
    await read(() => window.act({ kind: "snap", on: true, mode: "grid", beats: window.scene.beats, divisor: window.scene.divisor }));
    await save();
    const saved = soundMoments(readFileSync(copy)).find(s => Math.abs(s.time - head.time) <= 1);
    expect(saved?.samples.some(s => s.name === "hitclap") && saved.samples.some(s => s.name === "hitwhistle"),
      `the slider head at ${Math.round(head.time)} ms plays ${saved?.samples.map(s => s.name).join("+")}`);
    lastSaved = readFileSync(copy);
  });

  define("Revert to saved throws away unsaved hits", ["F005"], async () => {
    if (!scratch || !lastSaved) return { skip: "nothing saved" };
    const saved = await namedHits();
    await draw(scratch.time, scratch.row);
    await fromMenu("File", "Revert to saved…");
    await dialogShown("Revert to saved difficulty?");
    await dialogButton("Revert").click();
    // Reloaded: the drawn hit is gone and nothing is unsaved.
    const after = await until(n => !window.view.dirty && !window.view.busy
      && window.scene.moments.reduce((s, m) => s + m[3].length, 0) !== n + 1 && window.scene.moments.reduce((s, m) => s + m[3].length, 0),
      saved.length, "Revert did not reload the file", 15000);
    expect(after === saved.length, `Revert shows ${after} hits, the saved difficulty had ${saved.length}`);
    // The reloaded file can show a mix Save wrote on lanes of its own (e.g. a numbered index), so the
    // lane names may differ from what was drawn; that is reported, not failed.
    const now = await namedHits();
    if (JSON.stringify(now) === JSON.stringify(saved)) return `${after} hits, as saved`;
    const lanes = [...new Set(now.filter(h => !saved.includes(h)).map(h => h.replace(/^\S+ ms /, "")))];
    return `${after} hits, as saved; reopened on other lanes: ${lanes.slice(0, 6).join(", ")}`;
  });

  define("Copy hitsounds: the other difficulty keeps its objects and plays the copied sounds", ["F055", "F008"], async () => {
    const target = readdirSync(mapFolder).filter(name => /\.osu$/i.test(name) && name !== basename(copy))
      .find(name => Number(readOsu(readFileSync(join(mapFolder, name))).general.Mode ?? 0) === 0);
    if (!target) return { skip: "the mapset has no other osu!standard difficulty" };
    const targetPath = join(mapFolder, target);
    const before = readFileSync(targetPath);
    await fromMenu("Map", "Copy hitsounds…");
    await dialogShown("Copy hitsounds");
    const name = readOsu(before).metadata.Version;
    const box = page().locator("#dialog-body label").filter({ hasText: `[${name}]` }).locator('input[type="checkbox"]').first();
    expect(await box.count(), `the Copy dialog does not list ${name}`);
    await box.check();
    await reviewOutput("Copy hitsounds");
    // "Editor message" is a refusal (for example sample-mix fixes needed first): report it, never OK it.
    try {
      await answerDialogs(() => !window.view.busy && !window.view.review, { timeout: 60000, refuse: /^Editor message$/ });
    } catch (error) {
      if (!error.refused) throw error;
      await page().keyboard.press("Escape");
      expect(readFileSync(targetPath).equals(before), "Copy refused but still changed the target");
      return { skip: `the editor refused: ${error.message.slice(0, 220)}` };
    }
    const after = readFileSync(targetPath);
    expect(!after.equals(before), "Copy finished but the target file did not change");
    const shape = bytes => (readOsu(bytes).sections.HitObjects ?? []).map(line => line.split(",").slice(0, 4).join(","));
    expect(JSON.stringify(shape(before)) === JSON.stringify(shape(after)), "Copy moved, added or removed objects in the target");
    const source = soundMoments(readFileSync(copy));
    const copied = soundMoments(after);
    const problems = [];
    let matched = 0;
    for (const moment of copied) {
      const from = source.find(s => Math.abs(s.time - moment.time) <= 5);
      if (!from) continue;
      matched++;
      const want = families(from.samples).filter(n => !/hitnormal/.test(n));
      const got = families(moment.samples).filter(n => !/hitnormal/.test(n));
      if (JSON.stringify(want) !== JSON.stringify(got)) problems.push(`${Math.round(moment.time)} ms: ${got.join("+") || "none"}, source ${want.join("+") || "none"}`);
    }
    expect(matched > 0, "no target object shares a time with the source");
    expect(!problems.length, `${problems.length} of ${matched} matched moments differ: ${problems.slice(0, 4).join("; ")}`);
    return `${matched} matched moments carry the source's additions`;
  });

  define("Generate hitsound difficulty: the new difficulty plays at the source's times", ["F053", "F054"], async () => {
    const before = new Set(readdirSync(mapFolder));
    await fromMenu("Map", "Generate hitsound difficulty…");
    await dialogShown("Generate hitsound difficulty");
    await reviewOutput("Generate hitsound difficulty");
    await answerDialogs(() => !window.view.busy && !window.view.review, { timeout: 90000 });
    const added = readdirSync(mapFolder).filter(name => !before.has(name));
    const osu = added.filter(name => /\.osu$/i.test(name));
    expect(osu.length === 1, `Generate added ${JSON.stringify(added)}`);
    created.push(...osu);
    const generated = soundMoments(readFileSync(join(mapFolder, osu[0])));
    const source = soundMoments(readFileSync(copy));
    const missing = source.filter(s => s.samples.length && !generated.some(g => Math.abs(g.time - s.time) <= 1));
    expect(!missing.length, `${missing.length} source moments have no generated sound, first at ${Math.round(missing[0]?.time)} ms`);
    const text = readFileSync(join(mapFolder, osu[0]), "latin1");
    const unused = added.filter(name => !/\.osu$/i.test(name) && !text.includes(name.replace(/\.(wav|ogg|mp3)$/i, ""))
      && !/^(normal|soft|drum)-hit(normal|whistle|finish|clap)\d+\./i.test(name));
    expect(!unused.length, `Generate wrote files its difficulty does not use: ${unused.join(", ")}`);
    return `${osu[0]}: ${generated.length} moments; ${added.length - 1} sample files`;
  });

  define("New hitsound difficulty from the Map menu: empty, named, timing kept", ["F004"], async () => {
    const before = new Set(readdirSync(mapFolder));
    await fromMenu("Map", "New hitsound difficulty…");
    await dialogShown("New difficulty");
    await page().locator("#dialog-body input").first().fill("Robot HS");
    await reviewOutput("New difficulty");
    await answerDialogs(() => !window.view.busy && !window.view.review, { timeout: 30000 });
    const added = readdirSync(mapFolder).filter(name => !before.has(name));
    expect(added.length === 1 && /\[Robot HS\]\.osu$/.test(added[0]), `New difficulty added ${JSON.stringify(added)}`);
    created.push(added[0]);
    const made = readOsu(readFileSync(join(mapFolder, added[0])));
    expect(!(made.sections.HitObjects ?? []).length, "the new difficulty has objects");
    expect(made.metadata.Version === "Robot HS" && made.metadata.BeatmapID === "0", `Version ${made.metadata.Version}, BeatmapID ${made.metadata.BeatmapID}`);
    // Timing comes from the difficulty as it is on disk now: an earlier Save may have added green lines.
    const source = readOsu(readFileSync(copy)).sections.TimingPoints;
    expect(JSON.stringify(made.sections.TimingPoints) === JSON.stringify(source), "timing lines differ from the open difficulty's");
  });

  define("Revert mapset to original puts every .osu back, and Undo revert brings the edits back", ["N012"], async () => {
    const existing = [...copyBefore.keys()].filter(name => /\.osu$/i.test(name));
    const edited = snapshot(mapFolder);
    await fromMenu("File", "Revert mapset to original…");
    await dialogShown("Revert mapset to original?");
    await dialogButton("Revert mapset").click();
    await until(() => !window.view.busy && !document.getElementById("dialog").open, null, "the revert did not finish", 30000);
    await sleep(500);
    const reverted = snapshot(mapFolder);
    const wrong = existing.filter(name => reverted.get(name) !== copyBefore.get(name));
    expect(!wrong.length, `not back to the original: ${wrong.join(", ")}`);
    await fromMenu("File", "Undo revert");
    await until(() => !window.view.busy, null, "Undo revert did not finish", 30000);
    await sleep(500);
    const undone = snapshot(mapFolder);
    const lost = existing.filter(name => undone.get(name) !== edited.get(name));
    expect(!lost.length, `Undo revert did not bring back: ${lost.join(", ")}`);
    return `${existing.length} difficulties reverted and restored`;
  });

  define("The mapset copy holds only files the maps use: no project files, backups or stray samples", ["F048", "F085", "F088"], async () => {
    await r.quit();
    const after = snapshot(mapFolder);
    const { added, removed, changed } = diffSnapshots(copyBefore, after);
    expect(!removed.length, `removed: ${removed.join(", ")}`);
    const osuTexts = [...after.keys()].filter(n => /\.osu$/i.test(n)).map(n => readFileSync(join(mapFolder, n), "latin1"));
    const indexes = new Set();
    for (const text of osuTexts) for (const m of soundMoments(Buffer.from(text, "latin1"))) for (const s of m.samples) indexes.add(s.index);
    const plays = name => /\.osu$/i.test(name) || osuTexts.some(t => t.includes(basename(name))) || (() => {
      const match = /^(normal|soft|drum)-(hit(normal|whistle|finish|clap)|slider(slide|whistle|tick))(\d*)\.(wav|ogg|mp3)$/i.exec(basename(name));
      return !!match && indexes.has(match[5] ? Number(match[5]) : 1);
    })();
    const stray = added.filter(name => !plays(name));
    expect(!stray.length, `files no difficulty plays: ${stray.join(", ")}`);
    const otherChanged = changed.filter(name => !/\.osu$/i.test(name));
    expect(!otherChanged.length, `sample or other files changed: ${otherChanged.join(", ")}`);
    return `added ${added.join(", ") || "nothing"}; changed ${changed.join(", ") || "nothing"}`;
  });
  STEPS.at(-1).always = true;

  define("The original ranked mapset is byte-identical", [], async () => {
    const { added, removed, changed } = diffSnapshots(p.before, p.hashOriginal());
    expect(!added.length && !removed.length && !changed.length, `added ${added.join(", ") || "none"}; removed ${removed.join(", ") || "none"}; changed ${changed.join(", ") || "none"}`);
  });
  STEPS.at(-1).always = true;

  define("No errors after the last step", [], async () => {});
  STEPS.at(-1).always = true;
}
