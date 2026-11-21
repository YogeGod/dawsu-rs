// The robot remaps a ranked map's hitsounds from scratch in the real dawsu!, then osu!lazer checks the
// result against the original, and every place the editor got in the way is listed.
//
// node tools/remap-check.mjs --exe <dawsu-tauri.exe> --map <ranked .osu> [--seconds 45] [--json <file>] [--keep]
//
//   --exe      the built editor, as for tools/everyday-check.mjs
//   --map      a difficulty with many samples; its whole folder is copied first and only read
//   --seconds  how much of the map to remap, from its first object (default 45; 0 = all of it)
//   --json     also write the results as data to this file
//   --keep     keep the work folder under target/remap-check/
//
// What it does, as fast as the editor answers (no fixed waits between hits):
//   1. Copies the mapset twice: "original" stays as it is; in "work" the difficulty's objects inside the
//      section lose their hitsounds (additions, custom sets and indexes, volumes, sound files) and keep
//      everything else. Timing and green lines are left alone.
//   2. Opens the original copy in dawsu! and reads every hit and volume it shows in the section: the goal.
//      Then starts the editor once more on the stripped copy and reads what is left.
//   3. Switches Audition off, picks the Objects snap and remaps like a mapper: adds a lane through
//      Add lane for each sound file the goal needs, draws each missing hit, erases each extra one, and
//      sets each moment's volume in the Inspector. Each place the editor does not do what the click
//      asked (the hit lands elsewhere, does not appear, will not erase, the volume does not take, a lane
//      cannot be added, an error in the console) is written down with its time and lane.
//   4. Saves through the save review, then has the lazer oracle read the original and the remapped copy
//      and lists every sound moment in the section that plays differently.
//
// Nothing plays: Audition is off before the first click. Windows only in practice, like everyday-check.
import { spawn, spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { byTime, describe, heard, matchMoment } from "./lib/lazer-sounds.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = {};
for (let i = 2; i < process.argv.length; i++) {
  const value = process.argv[i + 1];
  const flag = value === undefined || value.startsWith("--");
  args[process.argv[i].replace(/^--/, "")] = flag ? true : value;
  if (!flag) i++;
}
if (!args.exe || !args.map) throw new Error("--exe and --map are required");
const exe = resolve(args.exe);
const given = resolve(args.map);
if (!existsSync(exe)) throw new Error(`no editor at ${exe}`);
if (!existsSync(given)) throw new Error(`no map at ${given}`);
const seconds = args.seconds === undefined ? 45 : Number(args.seconds);
const started = Date.now();
const sleep = ms => new Promise(done => setTimeout(done, ms));

function playwrightCore() {
  const candidates = [];
  if (process.env.DAWSU_PLAYWRIGHT_CORE) candidates.push(process.env.DAWSU_PLAYWRIGHT_CORE);
  const npm = spawnSync("npm config get cache", { encoding: "utf8", shell: true });
  const npx = join(npm.stdout?.trim() ?? "", "_npx");
  if (npm.status === 0 && existsSync(npx)) for (const entry of readdirSync(npx)) candidates.push(join(npx, entry, "node_modules", "playwright-core"));
  const found = candidates.find(path => existsSync(join(path, "package.json")));
  if (!found) throw new Error("Playwright Core not found: set DAWSU_PLAYWRIGHT_CORE to its folder");
  return createRequire(join(found, "package.json"))(found).chromium;
}
const chromium = playwrightCore();

// 1. Copies, and the stripped difficulty.
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..*/, "");
const work = join(repo, "target", "remap-check", `${stamp}-${process.pid}`);
const profile = join(work, "profile");
const folders = { original: join(work, "original"), remapped: join(work, "remapped") };
mkdirSync(profile, { recursive: true });
for (const folder of Object.values(folders)) cpSync(dirname(given), folder, { recursive: true, dereference: true });
const name = basename(given);
const originalMap = join(folders.original, name);
const workMap = join(folders.remapped, name);
const text = readFileSync(originalMap, "latin1");
const newline = text.includes("\r\n") ? "\r\n" : "\n";
let firstTime = null;
{
  let section = "";
  const lines = text.split(/\r?\n/);
  for (const line of lines) {
    if (/^\[.*\]$/.test(line.trim())) section = line.trim();
    else if (section === "[HitObjects]" && line.split(",").length >= 5) { firstTime = Number(line.split(",")[2]); break; }
  }
}
if (firstTime === null) throw new Error("the map has no hit objects");
const end = seconds > 0 ? firstTime + seconds * 1000 : Infinity;
const inSection = time => time >= firstTime && time <= end;
// Clears one object's hitsounds: additions, its sample sets and index, volume and sound file.
function strip(line) {
  const f = line.split(",");
  if (f.length < 5 || !inSection(Number(f[2]))) return line;
  const type = Number(f[3]);
  f[4] = "0";
  const plain = "0:0:0:0:";
  if (type & 2) {
    // Slider: edge sounds and edge sets per node, then the object's own sample.
    if (f.length > 8) f[8] = f[8].split("|").map(() => "0").join("|");
    if (f.length > 9) f[9] = f[9].split("|").map(() => "0:0").join("|");
    if (f.length > 10) f[10] = plain;
  } else if (type & 128) {
    if (f.length > 5) f[5] = `${f[5].split(":")[0]}:${plain}`;
  } else if (type & 8) {
    if (f.length > 6) f[6] = plain;
  } else if (f.length > 5) f[5] = plain;
  return f.join(",");
}
{
  let section = "";
  const out = text.split(/\r?\n/).map(line => {
    if (/^\[.*\]$/.test(line.trim())) { section = line.trim(); return line; }
    return section === "[HitObjects]" ? strip(line) : line;
  });
  writeFileSync(workMap, Buffer.from(out.join(newline), "latin1"));
}

// One editor: its process and page, and every console error it reports.
let app = null;
const errors = [];
const freePort = () => new Promise((done, fail) => {
  const server = createServer().listen(0, "127.0.0.1", () => { const { port } = server.address(); server.close(() => done(port)); }).on("error", fail);
});
async function launch(map) {
  const port = await freePort();
  const child = spawn(exe, [map], {
    env: { ...process.env, LOCALAPPDATA: profile, WEBVIEW2_USER_DATA_FOLDER: join(profile, "webview2"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
    stdio: "ignore",
  });
  app = { child, browser: null, page: null };
  for (let i = 0; i < 120 && !app.browser && child.exitCode === null; i++) {
    await sleep(250);
    app.browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`).catch(() => null);
  }
  if (!app.browser) throw new Error("the editor's WebView2 never opened its debugging port");
  const context = app.browser.contexts()[0];
  context.setDefaultTimeout(5000);
  for (let i = 0; i < 40 && !app.page; i++) { app.page = context.pages()[0] ?? null; if (!app.page) await sleep(250); }
  if (!app.page) throw new Error("the editor opened no window");
  app.page.on("console", message => { if (message.type() === "error") errors.push({ at: Date.now(), text: message.text() }); });
  app.page.on("pageerror", error => errors.push({ at: Date.now(), text: `page error: ${error.message}` }));
  await app.page.waitForFunction(() => typeof window.scene !== "undefined" && document.readyState === "complete", null, { timeout: 30000 });
}
async function quit() {
  if (!app) return;
  const { browser, child } = app;
  app = null;
  await browser?.close().catch(() => {});
  child.kill();
  await new Promise(done => (child.exitCode !== null ? done() : child.once("exit", done)));
}
const page = () => app.page;
const read = (fn, arg) => page().evaluate(fn, arg);
async function until(fn, arg, what, timeout = 6000) {
  const stop = Date.now() + timeout;
  while (Date.now() < stop) {
    const value = await read(fn, arg).catch(() => null);
    if (value) return value;
    await sleep(40);
  }
  throw new Error(what);
}
const expect = (ok, message) => { if (!ok) throw new Error(message); };

// The section's hits as "time|lane" with their volume, read from the open difficulty.
const sectionHits = () => read(([from, to]) => {
  const out = {};
  for (const [time, volume, , list] of window.scene.moments) {
    if (time < from || time > to) continue;
    for (const [row, , , hitVolume] of list) out[`${Math.round(time)}|${window.scene.lanes[row]?.name ?? "row " + row}`] = hitVolume ?? volume;
  }
  return out;
}, [firstTime, Number.isFinite(end) ? end : 1e12]);
const laneRow = laneName => read(laneName => window.scene.lanes.findIndex(lane => lane.name === laneName), laneName);
const hitCount = () => read(() => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0));

async function pickTool(tool) {
  const pill = page().locator('[aria-label="Editing tools"]').getByRole("button", { name: tool, exact: true });
  await pill.click({ timeout: 5000 });
  await until(tool => window.view.tool === tool, tool, `the tool did not become ${tool}`, 3000);
}
// Brings a time and a lane row into view, but only when they are not already: Fit all and zoom happen
// once, then the overview strip and the lane wheel move the view.
let zoomed = false;
async function reveal(time, row) {
  const state = () => read(([time, row]) => {
    const grid = document.getElementById("grid").getBoundingClientRect();
    const width = grid.width / window.scene.zoom;
    const lane = document.querySelector(`#lanes [data-lane-row="${row}"]`)?.getBoundingClientRect();
    return { inTime: time > window.scene.viewStart + width * 0.05 && time < window.scene.viewStart + width * 0.95,
      laneSide: !lane ? 0 : lane.y < grid.y ? -1 : lane.y + lane.height > grid.y + grid.height ? 1 : 0 };
  }, [time, row]);
  if (!zoomed) {
    for (let i = 0; i < 16; i++) {
      const zoom = await read(() => window.scene.zoom);
      if (zoom >= 0.2 || await read(() => document.getElementById("ruler-zoom-in").disabled)) break;
      await page().click("#ruler-zoom-in");
      await until(zoom => window.scene.zoom > zoom, zoom, "Zoom in did not zoom in");
    }
    zoomed = true;
  }
  let now = await state();
  if (!now.inTime) {
    const box = await read(() => document.getElementById("overview").getBoundingClientRect().toJSON());
    const duration = await read(() => window.view.duration);
    await page().mouse.click(box.x + box.width * Math.min(1, time / duration), box.y + box.height / 2);
    await until(time => {
      const width = document.getElementById("grid").clientWidth / window.scene.zoom;
      return time > window.scene.viewStart && time < window.scene.viewStart + width;
    }, time, `the overview did not bring ${time} ms into view`);
    now = await state();
  }
  if (now.laneSide) {
    const grid = await read(() => document.getElementById("grid").getBoundingClientRect().toJSON());
    await page().mouse.move(grid.x + grid.width / 2, grid.y + grid.height / 2);
    for (let turn = 0; turn < 60 && now.laneSide; turn++) {
      await page().mouse.wheel(0, now.laneSide * 120);
      await sleep(30);
      now = await state();
    }
  }
}
async function pointAt(time, row) {
  const point = await read(([time, row]) => {
    const grid = document.getElementById("grid").getBoundingClientRect();
    const lane = document.querySelector(`#lanes [data-lane-row="${row}"]`)?.getBoundingClientRect();
    return lane ? { x: grid.x + (time - window.scene.viewStart) * window.scene.zoom, y: lane.y + lane.height / 2 } : null;
  }, [time, row]);
  expect(point, `lane row ${row} has no header on screen`);
  return point;
}
const dialogTitle = () => read(() => document.getElementById("dialog").open ? document.getElementById("dialog-title").textContent : null);
const dialogButton = label => page().locator("#dialog-actions button", { hasText: label }).first();

const friction = [];
const note = (kind, detail) => { friction.push({ kind, detail }); };
const counts = { added: 0, erased: 0, volumes: 0, lanes: 0 };
let goal = null, stripped = null, final = null, saveReview = [];

try {
  // 2. The goal, then the stripped copy.
  await launch(originalMap);
  await until(() => window.scene?.lanes?.length > 0 && window.scene.moments.length > 0, null, "the original copy showed no hits", 60000);
  goal = await sectionHits();
  await quit();
  await launch(workMap);
  await until(() => window.scene?.lanes?.length > 0, null, "the stripped copy showed no lanes", 60000);
  stripped = await sectionHits();
  // A fresh profile asks for a key preset once.
  const preset = page().getByRole("button", { name: "DAW-like" });
  if (await preset.count() && await preset.first().isVisible()) await preset.first().click();

  // 3. Silence first, then the Objects snap.
  if (await read(() => window.view.audition)) await page().locator('button[aria-label="Audition"]').click();
  await until(() => window.view.audition === false, null, "Audition stayed on, so the robot will not click hits");
  await page().click("#grid-options");
  await page().selectOption("#snap-mode", "objects");
  await until(() => window.scene.snapMode === "objects" && window.scene.snap, null, "the Objects snap did not turn on").catch(async error => {
    note("snap", error.message);
  });
  await page().keyboard.press("Escape");

  const goalKeys = Object.keys(goal), strippedKeys = Object.keys(stripped);
  const toAdd = goalKeys.filter(key => !(key in stripped)).map(key => key.split("|")).map(([t, lane]) => [Number(t), lane]);
  const toErase = strippedKeys.filter(key => !(key in goal)).map(key => key.split("|")).map(([t, lane]) => [Number(t), lane]);

  // Lanes the goal uses that the stripped copy lacks: added through Add lane, as a mapper would.
  const lanes = new Set(await read(() => window.scene.lanes.map(lane => lane.name)));
  for (const lane of [...new Set(toAdd.map(([, lane]) => lane))].filter(lane => !lanes.has(lane))) {
    try {
      await page().getByRole("button", { name: "Add lane" }).last().click();
      await until(() => document.getElementById("dialog").open, null, "Add lane opened no dialog", 4000);
      const choices = await read(() => Array.from(document.querySelectorAll("#dialog-body option"), option => option.textContent));
      const bare = value => value.replace(/\.(wav|ogg|mp3)$/i, "").toLowerCase();
      const choice = choices.find(text => bare(text) === bare(lane));
      if (!choice) {
        await dialogButton(/^(Done|Cancel)$/).click().catch(() => page().keyboard.press("Escape"));
        note("lane", `Add lane does not offer ${lane} (it offers ${choices.length}: ${choices.slice(0, 6).join(", ")}${choices.length > 6 ? "…" : ""})`);
        continue;
      }
      await page().selectOption("#dialog-body select", { label: choice });
      await dialogButton("Add").click();
      await until(() => !document.getElementById("dialog").open, null, "the Add lane dialog did not close", 4000);
      counts.lanes++;
    } catch (error) {
      note("lane", `adding ${lane}: ${error.message}`);
      await page().keyboard.press("Escape").catch(() => {});
    }
  }

  // Draw the missing hits in time order, then erase the extra ones.
  for (const [tool, list, kind] of [["Draw", toAdd, "draw"], ["Erase", toErase, "erase"]]) {
    if (!list.length) continue;
    await pickTool(tool);
    list.sort((a, b) => a[0] - b[0]);
    for (const [time, lane] of list) {
      const row = await laneRow(lane);
      if (row < 0) { note(kind, `${time} ms ${lane}: no such lane to ${kind} on`); continue; }
      try {
        await reveal(time, row);
        const before = await hitCount();
        const point = await pointAt(time, row);
        await page().mouse.click(point.x, point.y);
        const changed = await until(([before, sign]) => {
          const count = window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0);
          return count === before + sign ? count : null;
        }, [before, kind === "draw" ? 1 : -1], "no change", 2500).catch(() => null);
        const there = await read(([time, row]) => window.scene.moments.some(([t, , , list]) => Math.round(t) === time && list.some(([r]) => r === row)), [time, row]);
        if (kind === "draw") {
          if (there) counts.added++;
          else if (changed) {
            const landed = await read(([row, time]) => window.scene.moments.filter(([, , , list]) => list.some(([r]) => r === row))
              .map(([t]) => Math.round(t)).sort((a, b) => Math.abs(a - time) - Math.abs(b - time))[0], [row, time]);
            note("draw", `${time} ms ${lane}: the click placed the hit at ${landed} ms`);
          } else note("draw", `${time} ms ${lane}: the click added no hit`);
        } else if (!there && changed) counts.erased++;
        else note("erase", `${time} ms ${lane}: the click did not erase it`);
      } catch (error) {
        note(kind, `${time} ms ${lane}: ${error.message}`);
      }
    }
  }

  // Volumes: one Inspector change per moment whose volume differs from the goal.
  const now = await sectionHits();
  const wanted = new Map();
  for (const [key, volume] of Object.entries(goal)) {
    if (key in now && now[key] !== volume) {
      const [t, lane] = key.split("|");
      if (!wanted.has(t)) wanted.set(t, { lane, volume });
    }
  }
  if (wanted.size) await pickTool("Select");
  for (const [t, { lane, volume }] of wanted) {
    const time = Number(t);
    try {
      const row = await laneRow(lane);
      await reveal(time, row);
      const point = await pointAt(time, row);
      await page().mouse.click(point.x, point.y);
      await until(() => window.scene.selection?.count >= 1, null, "the click selected nothing", 2500);
      await page().selectOption("#inspector-change", "absolute");
      await page().locator("#inspector-amount").fill(String(volume));
      await page().locator('form[aria-label="Change volume"] button[type="submit"]').click();
      const got = await until(([time, row, volume]) => {
        const hit = window.scene.moments.find(([t]) => Math.round(t) === time);
        const one = hit?.[3].find(([r]) => r === row);
        const value = one ? (one[3] ?? hit[1]) : null;
        return value === volume ? value : null;
      }, [time, row, volume], "no", 2500).catch(() => null);
      if (got === null) {
        const value = await read(([time, row]) => {
          const hit = window.scene.moments.find(([t]) => Math.round(t) === time);
          const one = hit?.[3].find(([r]) => r === row);
          return one ? (one[3] ?? hit[1]) : null;
        }, [time, row]);
        note("volume", `${time} ms: asked for ${volume}%, the hit shows ${value}%`);
      } else counts.volumes++;
    } catch (error) {
      note("volume", `${time} ms: ${error.message}`);
    }
  }
  final = await sectionHits();
  const missing = Object.keys(goal).filter(key => !(key in final));
  const extra = Object.keys(final).filter(key => !(key in goal));
  const offVolume = Object.keys(goal).filter(key => key in final && final[key] !== goal[key]);
  if (missing.length) note("result", `${missing.length} goal hits are still missing, e.g. ${missing.slice(0, 5).join(", ")}`);
  if (extra.length) note("result", `${extra.length} hits are there that the original does not have, e.g. ${extra.slice(0, 5).join(", ")}`);
  if (offVolume.length) note("result", `${offVolume.length} hits have another volume than the original, e.g. ${offVolume.slice(0, 5).map(key => `${key} ${final[key]}% (want ${goal[key]}%)`).join(", ")}`);

  // 4. Save, through the save review.
  await page().click("#save");
  const stop = Date.now() + 30000;
  while (Date.now() < stop && await read(() => window.view.dirty || document.getElementById("dialog").open)) {
    const title = await dialogTitle();
    if (title?.startsWith("Review Save")) {
      saveReview = await read(() => Array.from(document.querySelectorAll("#dialog-body p"), p => p.textContent));
      await dialogButton(/^(Save|Done)$/).click();
    } else if (title?.startsWith("Save hitsounds")) {
      saveReview.push(title);
      await dialogButton(/^(Save|Continue to Save)$/).click();
    } else if (title) {
      note("save", `Save showed "${title}": ${(await read(() => document.getElementById("dialog-body").innerText)).slice(0, 160)}`);
      await page().keyboard.press("Escape");
      break;
    }
    await sleep(100);
  }
  if (await read(() => window.view.dirty)) note("save", "the editor still shows unsaved changes after Save");
} catch (error) {
  note("stopped", error.message);
} finally {
  await quit();
}
for (const { text } of errors) note("console", text);

// 5. What lazer plays in the section, original against remapped.
let moments = [];
const oracleOut = join(work, "oracle");
const oracle = spawnSync("dotnet", ["run", "-c", "Release", "--project", join(repo, "tools/lazer-oracle"), "--", oracleOut, folders.original, folders.remapped],
  { encoding: "utf8", maxBuffer: 64 << 20 });
if (oracle.status !== 0) note("oracle", (oracle.stdout + oracle.stderr).trim().split("\n").slice(-3).join(" "));
else {
  const load = folder => JSON.parse(readFileSync(join(oracleOut, basename(folders[folder]), `${name}.json`), "utf8"));
  const sounds = folder => byTime(heard(load(folder), folders[folder]).filter(sound => inSection(sound.time)));
  const want = sounds("original"), got = sounds("remapped");
  moments = [...new Set([...want.keys(), ...got.keys()])].sort((a, b) => a - b).map(time => {
    const m = matchMoment(got.get(time) ?? [], want.get(time) ?? []);
    const notes = [...m.onlyLeft.map(s => `only the remap plays ${describe(s)}`), ...m.onlyRight.map(s => `the remap lacks ${describe(s)}`),
      ...m.volume.map(([a, b]) => `${a.label}: remap ${a.volume}%, original ${b.volume}%`)];
    return { time, same: !notes.length, notes };
  });
}

const same = moments.filter(moment => moment.same).length;
const kinds = {};
for (const { kind } of friction) kinds[kind] = (kinds[kind] ?? 0) + 1;
const secondsTaken = Math.round((Date.now() - started) / 1000);
console.log(`map: ${relative(repo, given)}; section ${firstTime}–${Number.isFinite(end) ? end : "end"} ms`);
console.log(`goal: ${goal ? Object.keys(goal).length : "?"} hits; stripped copy kept ${stripped ? Object.keys(stripped).length : "?"}`);
console.log(`robot: ${counts.lanes} lanes added, ${counts.added} hits drawn, ${counts.erased} erased, ${counts.volumes} volumes set`);
for (const item of friction) console.log(`GOT IN THE WAY (${item.kind}) ${item.detail}`);
for (const moment of moments.filter(moment => !moment.same).slice(0, 30)) console.log(`DIFF ${moment.time} ms — ${moment.notes.join("; ")}`);
if (moments.filter(moment => !moment.same).length > 30) console.log(`… and ${moments.filter(moment => !moment.same).length - 30} more differences`);
if (saveReview.length) console.log(`save review: ${saveReview.join(" | ").slice(0, 300)}`);
console.log(`run time: ${secondsTaken} s`);
console.log(`remap check: ${same} of ${moments.length} sound moments play as in the original; ${friction.length} places the editor got in the way (${Object.entries(kinds).map(([k, n]) => `${k} ${n}`).join(", ") || "none"})`);
if (args.json) writeFileSync(resolve(args.json), JSON.stringify({ exe, map: given, section: [firstTime, Number.isFinite(end) ? end : null],
  counts, friction, moments, saveReview, seconds: secondsTaken, work }, null, 2) + "\n");
if (!args.keep) try { rmSync(work, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 }); } catch {}
else console.log(`kept: ${relative(repo, work)}`);
if (friction.length || same !== moments.length) process.exitCode = 1;
