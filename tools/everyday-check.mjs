// Use the real, built dawsu! editor like a mapper for about a minute, so a broken screen is caught.
//
// node tools/everyday-check.mjs --exe <dawsu-tauri.exe> [--map <fixture .osu>] [--shots <folder>]
//   [--json <file>] [--keep-profile]
//
//   --exe           the built editor (cargo build -p dawsu-tauri --locked: target/debug/dawsu-tauri.exe)
//   --map           a difficulty whose whole folder is copied first; the copy is opened and edited,
//                   the original is only read (default: tests/fixtures/save-pack/synthetic/
//                   lf-no-final-newline.osu, a standard map with circles, a slider and a spinner;
//                   no fixture has its audio file, so the song is silent). Any map works: the
//                   edits are planned from the map itself (see planEdits): a hit on an empty beat
//                   away from every object, a whistle on a plain circle, and a whistle on a circle
//                   that names its own sound file when the map has one (else that step is skipped).
//   --shots         where a failing step's screenshot goes (default: <profile>/shots)
//   --json          also write the results as data to this file
//   --keep-profile  keep the profile folder (map copy, settings, WebView2 profile) after the run
//   --quick         only the steps that edit and save: skips the menus, the seven tools, the panels,
//                   the windows and the zoom step (for runs over many maps)
//
// The --json file also lists the edits the run made, in order, with what the editor showed after
// each ("actions"), and where the edited copy is ("mapCopy"). tools/compare-314.mjs replays those
// edits in dawsu! 3.1.4 and compares the two saved maps. It also lists every sound the editor would
// play for the map as opened and for the saved copy reopened ("sounds": the app's playback_list, worked
// out without an audio device), which tools/compare-314.mjs checks against what osu!lazer plays.
//
// The JS tests run against a hand-written page (apps/tauri/tests/fake-dom.mjs); this runs the real
// one. Every step acts through real input: mouse clicks and drags, key presses, buttons and menu
// rows. It only reads the page's `scene` and `view`, its text, the playback list and the files on
// disk to check the result; it never calls the page's act(). Each console error, page error and unhandled promise
// rejection in any of the editor's windows fails the step it happened in. A failing step saves a
// screenshot and the run goes on. An error between steps counts against the next step, and one
// after the last step fails a final step. The exit code is 1 when a step failed.
//
// The editor is stopped by ending its process, not by closing its window; the reopen steps start it
// again on the saved copy.
//
// Nothing plays: Audition is switched off before the first hit is placed and playback is never
// started. Every window the check opens is closed again.
//
// Windows only in practice (WebView2 remote debugging), written to stay portable. Playwright Core
// is found in DAWSU_PLAYWRIGHT_CORE or the npm cache, as tools/ui-screenshot.mjs does; nothing is
// installed.
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = {};
for (let i = 2; i < process.argv.length; i++) {
  const value = process.argv[i + 1];
  const flag = value === undefined || value.startsWith("--");
  args[process.argv[i].replace(/^--/, "")] = flag ? true : value;
  if (!flag) i++;
}
if (!args.exe) throw new Error("--exe is required: the built dawsu-tauri.exe");
const exe = resolve(args.exe);
if (!existsSync(exe)) throw new Error(`no editor at ${exe}`);
for (const key of ["map", "shots", "json"]) if (args[key] === true) throw new Error(`--${key} needs a value`);
const original = resolve(args.map ?? join(repo, "tests/fixtures/save-pack/synthetic/lf-no-final-newline.osu"));
if (!existsSync(original)) throw new Error(`no map at ${original}`);
const started = Date.now();

function playwrightCore() {
  const candidates = [];
  if (process.env.DAWSU_PLAYWRIGHT_CORE) candidates.push(process.env.DAWSU_PLAYWRIGHT_CORE);
  const npm = spawnSync("npm config get cache", { encoding: "utf8", shell: true });
  const npx = join(npm.stdout?.trim() ?? "", "_npx");
  if (npm.status === 0 && existsSync(npx)) {
    for (const entry of readdirSync(npx)) candidates.push(join(npx, entry, "node_modules", "playwright-core"));
  }
  const found = candidates.find(path => existsSync(join(path, "package.json")));
  if (!found) throw new Error("Playwright Core not found: set DAWSU_PLAYWRIGHT_CORE to its folder");
  return createRequire(join(found, "package.json"))(found).chromium;
}

const freePort = () => new Promise((done, fail) => {
  const server = createServer().listen(0, "127.0.0.1", () => {
    const { port } = server.address();
    server.close(() => done(port));
  }).on("error", fail);
});
const sleep = ms => new Promise(done => setTimeout(done, ms));

// Every file under a folder, by its path inside it, with its bytes' hash.
function snapshot(folder) {
  const files = new Map();
  const walk = dir => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name);
      if (statSync(path).isDirectory()) walk(path);
      else files.set(relative(folder, path).replaceAll("\\", "/"), createHash("sha256").update(readFileSync(path)).digest("hex"));
    }
  };
  walk(folder);
  return files;
}

// Found before any folder is made, so a missing Playwright leaves nothing behind.
const chromium = playwrightCore();

// The profile, map copy and shots stay next to the build (target/ is ignored and on the build drive).
const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..*/, "");
const profile = resolve(join(repo, "target", "everyday-check", `${stamp}-${process.pid}`));
const shots = resolve(args.shots ?? join(profile, "shots"));
const mapFolder = join(profile, "map");
const mapCopy = join(mapFolder, basename(original));
mkdirSync(profile, { recursive: true });
mkdirSync(shots, { recursive: true });
const originalBefore = snapshot(dirname(original));
// dereference: a link in the map's folder is copied as the file it points at, so the editor can
// never write through the copy into the original.
cpSync(dirname(original), mapFolder, { recursive: true, dereference: true });
// No fixture folder holds a spare sample, so Add lane would have nothing to offer. The Add lane step
// puts this short silent WAV (16-bit mono, 44.1 kHz, 0.1 s) in the copy first, as a mapper drops a
// new sample into the open mapset. The original folder is never written.
const laneSample = "everyday-check-lane.wav";
function writeLaneSample() {
  const samples = 4410;
  const wav = Buffer.alloc(44 + samples * 2);
  wav.write("RIFF", 0);
  wav.writeUInt32LE(36 + samples * 2, 4);
  wav.write("WAVEfmt ", 8);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(44100, 24);
  wav.writeUInt32LE(88200, 28);
  wav.writeUInt16LE(2, 32);
  wav.writeUInt16LE(16, 34);
  wav.write("data", 36);
  wav.writeUInt32LE(samples * 2, 40);
  writeFileSync(join(mapFolder, laneSample), wav);
}
const copyBefore = snapshot(mapFolder);
const osuBefore = readFileSync(mapCopy);

// Where the edits go, read from the map: its first red line's beat, its objects, and from those an
// empty beat (two beats clear of every object, for the placed hit), a plain circle (no whistle, no
// sound file of its own, alone at its time) and a circle that names its own sound file (optional).
function planEdits(text) {
  let section = "", red = null;
  const objects = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (/^\[.*\]$/.test(line)) { section = line; continue; }
    if (!line || line.startsWith("//")) continue;
    const fields = line.split(",");
    if (section === "[TimingPoints]" && fields.length >= 2 && !red) {
      const beat = Number(fields[1]);
      if (beat > 0 && (fields.length < 7 || fields[6].trim() === "1")) red = { offset: Number(fields[0]), beat };
    }
    if (section === "[HitObjects]" && fields.length >= 5) {
      const type = Number(fields[3]), time = Math.round(Number(fields[2]));
      // A spinner's end is field 6; a mania hold's is the first part of field 6.
      const end = type & 8 ? Number(fields[5]) : type & 128 ? Number(fields[5]?.split(":")[0]) : time;
      const sample = type & 1 && fields.length >= 6 ? fields[5].split(":") : [];
      objects.push({ time, end: Number.isFinite(end) ? end : time, circle: !!(type & 1), whistle: !!(Number(fields[4]) & 2), file: sample[4]?.trim() || "" });
    }
  }
  const beat = red?.beat ?? 500, offset = red?.offset ?? 0;
  const alone = object => objects.filter(other => other.time === object.time).length === 1;
  const plain = objects.find(o => o.circle && !o.whistle && !o.file && alone(o));
  const custom = objects.find(o => o.circle && !o.whistle && o.file && alone(o) && o !== plain);
  const clear = time => objects.every(o => (o.end < time - 2 * beat || o.time > time + 2 * beat));
  const first = objects.length ? Math.min(...objects.map(o => o.time)) : offset;
  const last = objects.length ? Math.max(...objects.map(o => o.end)) : offset;
  let empty = null;
  for (let k = Math.ceil((first - offset) / beat); offset + k * beat <= last && empty === null; k++) {
    if (clear(offset + k * beat)) empty = Math.round(offset + k * beat);
  }
  empty ??= Math.round(offset + Math.ceil((last - offset) / beat + 4) * beat);
  // Clicked this far after a circle, the 1/4 snap still lands on it.
  const nudge = Math.max(1, Math.min(40, Math.floor(beat / 4 * 0.3)));
  return { beat, empty, plain: plain?.time ?? null, custom: custom?.time ?? null, customFile: custom?.file ?? null, nudge };
}
const plan = planEdits(osuBefore.toString("latin1"));

// One running editor: its process, its pages and the errors they report.
let app = null;
let current = null;
const errors = [];

async function launch(map) {
  const port = await freePort();
  const child = spawn(exe, map ? [map] : [], {
    // WebView2 finds its profile without LOCALAPPDATA; name it, or a running dawsu! shares its browser.
    env: { ...process.env, LOCALAPPDATA: profile, WEBVIEW2_USER_DATA_FOLDER: join(profile, "webview2"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
    stdio: "ignore",
  });
  // Known at once, so a start that fails half-way is still shot and closed.
  app = { child, browser: null, context: null, page: null };
  let browser = null;
  for (let i = 0; i < 120 && !browser && child.exitCode === null; i++) {
    await sleep(500);
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`).catch(() => null);
  }
  if (!browser) {
    throw new Error(child.exitCode === null ? "the editor's WebView2 never opened its debugging port"
      : `the editor exited at start with code ${child.exitCode}`);
  }
  app.browser = browser;
  const context = browser.contexts()[0];
  app.context = context;
  // A broken screen should fail a step in seconds, not wait out Playwright's 30 s per click.
  context.setDefaultTimeout(5000);
  const listen = page => {
    const where = () => page.url().split("/").pop() || "page";
    page.on("console", message => {
      if (message.type() === "error") errors.push({ step: current, text: `${where()}: console error: ${message.text()}` });
    });
    // Where a page error was thrown: its first stack line, with the page's own address left out.
    const origin = error => (error.stack ?? "").split("\n").find(line => /^\s+at /.test(line))?.trim().replace(/https?:\/\/[^/]+\//g, "") ?? "";
    page.on("pageerror", error => errors.push({ step: current, text: `${where()}: page error: ${error.message}${origin(error) ? ` (${origin(error)})` : ""}` }));
  };
  context.on("page", listen);
  await context.addInitScript(() => window.addEventListener("unhandledrejection", event =>
    console.error("unhandled promise rejection: " + (event.reason?.stack ?? event.reason))));
  let page = null;
  for (let i = 0; i < 40 && !page; i++) {
    page = context.pages()[0] ?? null;
    if (!page) await sleep(250);
  }
  if (!page) throw new Error("the editor opened no window");
  app.page = page;
  for (const open of context.pages()) listen(open);
  // Listen from the very start: load the page again with the listeners in place.
  await page.reload();
  await page.waitForFunction(() => typeof window.scene !== "undefined" && document.readyState === "complete", null, { timeout: 30000 });
  return app;
}

async function quit() {
  if (!app) return;
  const { browser, child } = app;
  app = null;
  await browser?.close().catch(() => {});
  child.kill();
  await new Promise(done => (child.exitCode !== null ? done() : child.once("exit", done)));
}

// The steps and their results.
const results = [];
// The edits, in order, for tools/compare-314.mjs: what was done and what the editor showed after.
const actions = [];
const did = action => actions.push({ step: current, ...action });
// What the editor would play, read from the app without sound (null on a build without playback_list).
const sounds = { opened: null, reopened: null };
const playbackList = () => read(() => window.__TAURI__.core.invoke("playback_list")).catch(() => null);
let errorsChecked = 0;
// When the map never opened, the steps that need it are skipped rather than failed one by one.
let noMap = null;
const opensMap = new Set(["Open the map copy and see lanes and a drawn timeline"]);
const quickSkips = /^(Open the .* menu|Pick each of the seven tools|Fold and show|Open and close|Zoom and scroll)/;
async function step(name, body, always = false) {
  const number = String(results.length + 1).padStart(2, "0");
  if (args.quick && quickSkips.test(name)) {
    results.push({ number, name, status: "SKIP", detail: "quick run" });
    console.log(`SKIP ${number} ${name} — quick run`);
    return;
  }
  if (noMap && !always) {
    results.push({ number, name, status: "SKIP", detail: noMap });
    console.log(`SKIP ${number} ${name} — ${noMap}`);
    return;
  }
  current = number;
  const result = { number, name, status: "PASS", detail: "" };
  let threw = false;
  try {
    const note = await body();
    if (typeof note === "string") result.detail = note;
    if (note?.skip) {
      result.status = "SKIP";
      result.detail = note.skip;
    }
  } catch (error) {
    threw = true;
    result.status = "FAIL";
    result.detail = error.message;
  }
  await sleep(150);
  // This step's errors, and any that came between the last step and this one.
  const raised = errors.slice(errorsChecked).filter(error => error.step === number || error.step === null);
  errorsChecked = errors.length;
  if (raised.length) {
    result.status = "FAIL";
    // The same error many times reads as one line with a count.
    const counts = new Map();
    for (const { text } of raised) counts.set(text, (counts.get(text) ?? 0) + 1);
    result.errors = [...counts].map(([text, count]) => count > 1 ? `${text} (×${count})` : text);
    result.detail = [result.detail, ...result.errors].filter(Boolean).join(" | ");
  }
  result.detail = result.detail.replace(/\s*\n\s*/g, " ");
  if (result.status === "FAIL" && app?.context) {
    const path = join(shots, `${number}-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/-$/, "")}.png`);
    const target = app.context.pages().filter(page => !page.isClosed()).at(-1) ?? app.page;
    if (await target.screenshot({ path }).then(() => true, () => false)) result.screenshot = path;
  }
  results.push(result);
  // Only a map that never showed stops the rest; errors while it opened fail this step alone.
  if (threw && opensMap.has(name)) noMap = `the map did not open in step ${number}`;
  console.log(`${result.status} ${number} ${name}${result.detail ? " — " + result.detail : ""}${result.screenshot ? " (" + result.screenshot + ")" : ""}`);
  current = null;
  // A dialog or menu a failed step left open would block the next one.
  if (result.status === "FAIL" && app?.page) await tidy().catch(() => {});
}

const page = () => app.page;
const read = (fn, arg) => page().evaluate(fn, arg);
async function until(fn, arg, what, timeout = 8000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    last = await read(fn, arg).catch(error => ({ error: error.message }));
    if (last && !last.error) return last;
    await sleep(50);
  }
  throw new Error(`${what} (last: ${JSON.stringify(last)?.slice(0, 160)})`);
}
const expect = (ok, message) => {
  if (!ok) throw new Error(message);
};

// Close any open dialog and menu.
async function tidy() {
  for (let i = 0; i < 3; i++) {
    const open = await read(() => document.getElementById("dialog")?.open || !document.getElementById("editor-menu")?.hidden).catch(() => false);
    if (!open) return;
    await page().keyboard.press("Escape");
    await sleep(250);
  }
}

// Every hit in the open difficulty: [time, lane row, selected, volume].
const hits = () => read(() => window.scene.moments.flatMap(([time, volume, , list]) =>
  list.map(([row, selected, , hitVolume]) => [time, row, !!selected, hitVolume ?? volume])));
const scene = fn => read(fn);
// The same by lane name, as "3500 ms normal-hitwhistle": rows renumber when lanes come and go.
const namedHits = () => read(() => window.scene.moments.flatMap(([time, , , list]) =>
  list.map(([row]) => `${Math.round(time)} ms ${window.scene.lanes[row]?.name ?? "row " + row}`)).sort());

// Where a time and a lane are on the timeline canvas, in page pixels.
async function at(time, row) {
  const point = await read(([time, row]) => {
    const grid = document.getElementById("grid").getBoundingClientRect();
    const lane = document.querySelector(`#lanes [data-lane-row="${row}"]`)?.getBoundingClientRect();
    if (!lane) return null;
    return { x: grid.x + (time - window.scene.viewStart) * window.scene.zoom, y: lane.y + lane.height / 2, grid };
  }, [time, row]);
  if (!point) throw new Error(`lane row ${row} has no header on screen`);
  expect(point.x > point.grid.x && point.x < point.grid.x + point.grid.width, `time ${time} is outside the visible timeline`);
  expect(point.y > point.grid.y && point.y < point.grid.y + point.grid.height, `lane row ${row} is outside the visible timeline`);
  return point;
}

// Brings a time and a lane row into view as a mapper would: Fit all, zoom in until one pixel is at
// most 5 ms, click the overview strip at that time, and turn the wheel over the lanes until the row
// shows. Short maps already show everything after Fit all.
async function reveal(time, row) {
  await page().click("#ruler-fit");
  await until(() => window.scene.viewStart === 0, null, "Fit all did not go back to the start");
  for (let i = 0; i < 16; i++) {
    const zoom = await read(() => window.scene.zoom);
    if (zoom >= 0.2 || await read(() => document.getElementById("ruler-zoom-in").disabled)) break;
    await page().click("#ruler-zoom-in");
    await until(zoom => window.scene.zoom > zoom, zoom, "Zoom in did not zoom in");
  }
  const visible = () => read(time => {
    const width = document.getElementById("grid").clientWidth / window.scene.zoom;
    return time > window.scene.viewStart + width * 0.05 && time < window.scene.viewStart + width * 0.95;
  }, time);
  if (!await visible()) {
    const box = await read(() => document.getElementById("overview").getBoundingClientRect().toJSON());
    const duration = await read(() => window.view.duration);
    expect(duration > 0, "the editor reports no song length");
    // The overview ignores a click while the editor is busy, so wait for it and try up to three times.
    const shown = time => {
      const width = document.getElementById("grid").clientWidth / window.scene.zoom;
      return time >= window.scene.viewStart && time <= window.scene.viewStart + width;
    };
    let last = null;
    for (let attempt = 0; attempt < 3 && !(await read(shown, time)); attempt++) {
      await until(() => !window.view.busy, null, "the editor stayed busy", 15000);
      await page().mouse.click(box.x + box.width * Math.min(1, time / duration), box.y + box.height / 2);
      last = await until(shown, time, "", 3000).catch(() => null);
      if (last) break;
    }
    if (!(await read(shown, time))) {
      const now = await read(() => ({ viewStart: window.scene.viewStart, zoom: window.scene.zoom, busy: window.view.busy, width: document.getElementById("grid").clientWidth }));
      throw new Error(`clicking the overview did not bring ${time} ms into view (overview ${Math.round(box.width)}×${Math.round(box.height)} px at ${Math.round(box.x)},${Math.round(box.y)}; song ${Math.round(duration)} ms; view ${JSON.stringify(now)})`);
    }
  }
  const grid = await read(() => document.getElementById("grid").getBoundingClientRect().toJSON());
  await page().mouse.move(grid.x + grid.width / 2, grid.y + grid.height / 2);
  for (let turn = 0; turn < 40; turn++) {
    const where = await read(row => {
      const lane = document.querySelector(`#lanes [data-lane-row="${row}"]`)?.getBoundingClientRect();
      const grid = document.getElementById("grid").getBoundingClientRect();
      if (!lane) return 0;
      return lane.y < grid.y ? -1 : lane.y + lane.height > grid.y + grid.height ? 1 : 0;
    }, row);
    if (!where) break;
    await page().mouse.wheel(0, where * 120);
    await sleep(80);
  }
}

// A row of the open popup menu, by its text.
const menuRow = text => page().locator("#editor-menu button").filter({ hasText: text }).first();
async function openMenu(name) {
  await page().click(`#menu-${name.toLowerCase()}`);
  await until(() => !document.getElementById("editor-menu").hidden &&
    document.querySelectorAll("#editor-menu button").length > 0, null, `the ${name} menu did not open`, 4000);
}
async function fromMenu(menu, row) {
  await openMenu(menu);
  const button = menuRow(row);
  expect(await button.count(), `${menu} has no "${row}" row`);
  expect(await button.isEnabled(), `${menu} › ${row} is disabled`);
  await button.click();
}
async function pickTool(name) {
  // Select, Draw and Erase have pills (a fourth pill shows a picked tool by its icon only).
  const pill = page().locator('[aria-label="Editing tools"]').getByRole("button", { name, exact: true });
  if (["Select", "Draw", "Erase"].includes(name)) await pill.click({ timeout: 5000 });
  else {
    // Mute, Grab, Range and Ramp are in the right-click picker on the timeline.
    const box = await read(() => document.getElementById("grid").getBoundingClientRect().toJSON());
    await page().mouse.click(box.x + box.width - 20, box.y + box.height - 10, { button: "right" });
    await until(() => !document.getElementById("editor-menu").hidden, null, "the right-click tool picker did not open", 4000);
    await menuRow(name).click();
  }
  await until(name => window.view.tool === name, name, `the tool stayed ${await read(() => window.view.tool)}, not ${name}`, 4000);
}
const dialogTitle = () => read(() => document.getElementById("dialog").open ? document.getElementById("dialog-title").textContent : null);
async function dialogShown(title) {
  await until(title => document.getElementById("dialog").open &&
    document.getElementById("dialog-title").textContent.startsWith(title), title,
    `no "${title}" dialog opened (open: ${JSON.stringify(await dialogTitle())})`, 6000);
}
const dialogButton = text => page().locator("#dialog-actions button", { hasText: text }).first();
async function dialogClosed() {
  await until(() => !document.getElementById("dialog").open, null, "the dialog did not close", 4000);
}
// Another editor window, found by the page it loads.
async function windowOpened(name) {
  const end = Date.now() + 15000;
  while (Date.now() < end) {
    const found = app.context.pages().find(candidate => !candidate.isClosed() && candidate.url().endsWith("/" + name));
    if (found) {
      await found.waitForSelector("[data-ui-ready]", { timeout: Math.max(1000, end - Date.now()) });
      return found;
    }
    await sleep(200);
  }
  throw new Error(`no ${name} window opened`);
}
async function windowClosed(window, name) {
  const end = Date.now() + 8000;
  while (Date.now() < end && !window.isClosed()) await sleep(200);
  expect(window.isClosed(), `the ${name} window did not close`);
}
// The keys a menu row shows for its command, as Playwright names them ("Ctrl+Shift+Z" → "Control+Shift+Z").
const keyNames = { Ctrl: "Control", Del: "Delete", Esc: "Escape", Cmd: "Meta" };
const playwrightKey = keys => keys.split("+").map(key => keyNames[key] ?? key).join("+");
async function menuKeys(menu, row) {
  await openMenu(menu);
  const button = menuRow(row);
  let keys = "";
  // The keys arrive a moment after the menu opens (editor_shortcuts).
  for (let i = 0; i < 50 && !keys; i++) {
    keys = (await button.locator(".menu-keys, kbd, [class*=key]").allTextContents().catch(() => [])).join("").trim();
    if (!keys) await sleep(100);
  }
  await page().keyboard.press("Escape");
  return keys;
}

// Steps that place, select or move hits run only once Audition is off, so nothing plays.
let silent = false;
const needsSilence = () => silent ? null : { skip: "Audition could not be switched off, so no hit is placed" };
let laneRow = null;
let hitsBeforeEdits = 0;
// The hits by name just before and just after the Delete step, for Undo and Redo to match.
let withPlaced = [];
let withoutPlaced = [];
let placed = null;
const keys = {};
let savedHits = null;
let mixFiles = [];

try {
  await step("Start with no map and see the start screen", async () => {
    await launch(null);
    await until(() => !document.getElementById("home").hidden && document.getElementById("workspace").hidden &&
      !!document.getElementById("home-open"), null, "the start screen with Open map… did not show", 15000);
    // A fresh profile asks for a key preset once: answer it as a mapper would.
    const preset = page().getByRole("button", { name: "DAW-like" });
    if (await preset.count() && await preset.first().isVisible()) {
      await preset.first().click();
      await until(() => window.view.keyPreset === "DAW", null, "choosing DAW-like did not set the key preset", 4000);
    }
    const text = await read(() => document.getElementById("home").innerText);
    expect(text.includes("Open a map"), `the start screen reads: ${text.slice(0, 80)}`);
  });
  await quit();

  await step("Open the map copy and see lanes and a drawn timeline", async () => {
    await launch(mapCopy);
    await until(() => document.querySelectorAll("#lanes [data-lane-row]").length > 0 && window.scene?.lanes?.length > 0,
      null, "no lanes appeared", 60000);
    await sleep(800);
    const drawn = await read(() => {
      const grid = document.getElementById("grid");
      const pixels = grid.getContext("2d").getImageData(0, 0, grid.width, grid.height).data;
      const colours = new Set();
      for (let i = 0; i < pixels.length && colours.size < 4; i += 4 * 97) colours.add(pixels.slice(i, i + 4).join(","));
      return { width: grid.width, height: grid.height, colours: colours.size };
    });
    expect(drawn.width > 0 && drawn.height > 0, "the timeline canvas has no size");
    expect(drawn.colours > 1, "the timeline canvas is blank");
    hitsBeforeEdits = (await hits()).length;
    expect(hitsBeforeEdits > 0, "the map's hits are not shown");
    did({ do: "open", seen: await namedHits() });
    sounds.opened = await playbackList();
    return `${await read(() => window.scene.lanes.length)} lanes, ${hitsBeforeEdits} hits`;
  });

  for (const menu of ["File", "Edit", "Playback", "Map", "View"]) {
    await step(`Open the ${menu} menu and see its items`, async () => {
      await openMenu(menu);
      const rows = await read(() => Array.from(document.querySelectorAll("#editor-menu [role^=menuitem]"), row => row.textContent.trim()));
      expect(rows.length >= 3, `the ${menu} menu lists ${rows.length} items`);
      await page().keyboard.press("Escape");
      await until(() => document.getElementById("editor-menu").hidden, null, `Escape did not close the ${menu} menu`, 3000);
      return `${rows.length} items`;
    });
  }

  await step("Switch Audition off so nothing plays", async () => {
    if (await read(() => window.view.audition)) {
      await page().locator('button[aria-label="Audition"]').click({ timeout: 5000 });
    }
    await until(() => window.view.audition === false, null, "Audition is still on");
    silent = true;
  });

  await step("Pick each of the seven tools", async () => {
    for (const tool of ["Draw", "Erase", "Mute", "Grab", "Range", "Ramp", "Select"]) await pickTool(tool);
  });

  await step("Change the snap with the snap slider", async () => {
    const before = await scene(() => window.scene.beats + "/" + window.scene.divisor);
    const box = await read(() => document.querySelector('[role="slider"][aria-label="Snap"]').getBoundingClientRect().toJSON());
    // The left end is the coarsest stop.
    await page().mouse.click(box.x + 3, box.y + box.height / 2);
    await until(before => window.scene.beats + "/" + window.scene.divisor !== before, before, `the snap stayed ${before}`);
    const coarse = await scene(() => window.scene.beats + "/" + window.scene.divisor);
    await page().locator('[role="slider"][aria-label="Snap"]').focus();
    for (let i = 0; i < 2; i++) await page().keyboard.press("ArrowRight");
    await until(coarse => window.scene.beats + "/" + window.scene.divisor !== coarse, coarse, `the arrow keys left the snap at ${coarse}`);
    // Back to 1/4, the map's grid, by clicking that stop.
    const target = await read(() => {
      const ticks = Array.from(document.querySelectorAll('[role="slider"][aria-label="Snap"] .tick'));
      return ticks.map(tick => tick.getBoundingClientRect().toJSON());
    });
    for (const tick of target) {
      if (await scene(() => window.scene.divisor === 4 && window.scene.beats === 1)) break;
      await page().mouse.click(tick.x + tick.width / 2, tick.y + tick.height / 2);
      await sleep(200);
    }
    await until(() => window.scene.divisor === 4 && window.scene.beats === 1, null, "the slider never reached 1/4");
    return `${before} → ${coarse} → 1/4`;
  });

  await step("Place a hit with a click", async () => {
    if (!silent) return needsSilence();
    // An empty spot on the first lane, on the beat grid, away from the map's objects.
    laneRow = await read(() => window.scene.lanes.findIndex(lane => lane.name === "normal-hitnormal"));
    expect(laneRow >= 0, "no normal-hitnormal lane");
    await pickTool("Draw");
    const before = await hits();
    await reveal(plan.empty, laneRow);
    const point = await at(plan.empty, laneRow);
    await page().mouse.click(point.x, point.y);
    const after = await until(count => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0) === count + 1 &&
      window.scene.moments.flatMap(([time, , , list]) => list.map(([row]) => [time, row])), before.length, "the hit count did not go up by one");
    const added = after.filter(([time, row]) => !before.some(([t, r]) => t === time && r === row));
    expect(added.length === 1, `expected one new hit, found ${JSON.stringify(added)}`);
    placed = added[0];
    expect(placed[1] === laneRow, `the hit landed on lane row ${placed[1]}, not ${laneRow}`);
    did({ do: "place", lane: "normal-hitnormal", at: plan.empty, seen: placed[0] });
    return `hit at ${placed[0]} ms on ${await read(row => window.scene.lanes[row].name, laneRow)}`;
  });

  await step("Select the hit", async () => {
    if (!silent) return needsSilence();
    await pickTool("Select");
    await page().mouse.click(400, 10);
    const point = await at(placed[0], placed[1]);
    await page().mouse.click(point.x, point.y);
    await until(([time, row]) => window.scene.selection?.count === 1 && window.scene.moments.some(([t, , , list]) =>
      t === time && list.some(([r, selected]) => r === row && selected)), placed, "the hit was not selected");
  });

  await step("Move the hit with a drag", async () => {
    if (!silent) return needsSilence();
    const point = await at(placed[0], placed[1]);
    const zoom = await read(() => window.scene.zoom);
    const mouse = page().mouse;
    // At a hand's pace: rest on the hit, press, slide a quarter second later in eight moves, let go.
    await mouse.move(point.x, point.y);
    await sleep(300);
    // What the editor was doing as the button went down, for the report if the hit stays put.
    const pressed = await read(() => ({ busy: !!window.view.busy, selected: window.scene.selection?.count ?? 0, tool: window.view.tool }));
    await mouse.down();
    await sleep(100);
    for (let i = 1; i <= 8; i++) {
      await mouse.move(point.x + 250 * zoom * i / 8, point.y);
      await sleep(60);
    }
    await sleep(100);
    await mouse.up();
    const moved = await until(([time, row]) => {
      const here = window.scene.moments.flatMap(([t, , , list]) => list.filter(([r, s]) => r === row && s).map(() => t));
      return here.length === 1 && here[0] !== time ? here[0] : null;
    }, placed, `the selected hit did not move (at the press: ${JSON.stringify(pressed)})`);
    const count = (await hits()).length;
    expect(count === hitsBeforeEdits + 1, `the drag changed the hit count to ${count}`);
    did({ do: "move", from: placed[0], by: moved - placed[0], seen: moved });
    placed = [moved, placed[1]];
    return `now at ${moved} ms`;
  });

  await step("Change the hit's volume and keep every volume at 5% or more", async () => {
    if (!silent) return needsSilence();
    const amount = page().locator("#inspector-amount");
    expect(await amount.isVisible(), "the Inspector shows no volume field for the selection");
    await page().selectOption("#inspector-change", "absolute");
    await amount.fill("60");
    await page().locator('form[aria-label="Change volume"] button[type="submit"]').click();
    await until(([time, row]) => window.scene.moments.some(([t, volume, , list]) =>
      t === time && list.some(([r, , , hitVolume]) => r === row && (hitVolume ?? volume) === 60)), placed, "the hit's volume did not become 60");
    did({ do: "volume", value: 60, seen: 60 });
    // A mapper types 2: the hit must not end up under 5%.
    await amount.fill("2");
    await page().locator('form[aria-label="Change volume"] button[type="submit"]').click();
    // Wait for the editor to answer: the volume leaves 60, or (if it refuses 2) the editor is idle again.
    const answered = Date.now() + 5000;
    while (Date.now() < answered) {
      await sleep(150);
      const now = await read(([time, row]) => window.scene.moments.flatMap(([t, volume, , list]) =>
        t === time ? list.filter(([r]) => r === row).map(([, , , hitVolume]) => hitVolume ?? volume) : [])[0], placed);
      if (now !== 60) break;
    }
    await until(() => !window.view.busy, null, "the editor stayed busy after the volume change");
    const typed = await read(([time, row]) => window.scene.moments.flatMap(([t, volume, , list]) =>
      t === time ? list.filter(([r]) => r === row).map(([, , , hitVolume]) => hitVolume ?? volume) : [])[0], placed);
    did({ do: "volume", value: 2, seen: typed });
    expect(typed >= 5, `typing 2% and Apply set the hit's volume to ${typed}%`);
    const low = await read(() => {
      const found = [];
      for (const [time, volume, , list] of window.scene.moments) {
        if (volume < 5) found.push(`${time} ms at ${volume}%`);
        for (const [row, , , hitVolume] of list) if (hitVolume != null && hitVolume < 5) found.push(`${time} ms lane ${row} at ${hitVolume}%`);
      }
      if (window.scene.insertVolume < 5) found.push(`insert volume ${window.scene.insertVolume}%`);
      for (const key of ["main", "song"]) if (window.view[key] < 0.05) found.push(`${key} level ${window.view[key]}`);
      return found;
    });
    expect(!low.length, `volumes under 5%: ${low.join(", ")}`);
  });

  await step("Delete the hit with the Delete key", async () => {
    if (!silent) return needsSilence();
    withPlaced = await namedHits();
    await page().mouse.move(300, 5);
    await page().keyboard.press("Delete");
    await until(count => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0) === count,
      hitsBeforeEdits, "the hit count did not go back down");
    withoutPlaced = await namedHits();
    did({ do: "delete", seen: withoutPlaced });
    const gone = withPlaced.filter(name => !withoutPlaced.includes(name));
    expect(withoutPlaced.every(name => withPlaced.includes(name)) && gone.length <= 1,
      `Delete changed other hits: before ${JSON.stringify(withPlaced)}, after ${JSON.stringify(withoutPlaced)}`);
  });

  await step("Undo brings the hit back", async () => {
    if (!silent) return needsSilence();
    keys.undo = await menuKeys("Edit", "Undo");
    expect(keys.undo, "the Edit menu shows no key for Undo");
    await page().keyboard.press(playwrightKey(keys.undo));
    await until(count => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0) === count + 1,
      hitsBeforeEdits, `${keys.undo} did not bring the hit back`);
    const back = await namedHits();
    did({ do: "undo", seen: back });
    expect(JSON.stringify(back) === JSON.stringify(withPlaced), `Undo brought back different hits: ${JSON.stringify(back)}`);
    return keys.undo;
  });

  await step("Redo removes it again", async () => {
    if (!silent) return needsSilence();
    keys.redo = await menuKeys("Edit", "Redo");
    expect(keys.redo, "the Edit menu shows no key for Redo");
    await page().keyboard.press(playwrightKey(keys.redo));
    await until(count => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0) === count,
      hitsBeforeEdits, `${keys.redo} did not remove the hit again`);
    const again = await namedHits();
    did({ do: "redo", seen: again });
    expect(JSON.stringify(again) === JSON.stringify(withoutPlaced), `Redo left different hits: ${JSON.stringify(again)}`);
    return keys.redo;
  });

  await step("Add a lane", async () => {
    const before = await read(() => window.scene.lanes.length);
    writeLaneSample();
    did({ do: "add-file", file: laneSample });
    await page().getByRole("button", { name: "Add lane" }).last().click();
    await dialogShown("Add lane");
    const choices = await read(() => Array.from(document.querySelectorAll("#dialog-body option"), option => option.textContent));
    if (!choices.length) {
      const text = await read(() => document.getElementById("dialog-body").innerText);
      await dialogButton("Done").click();
      throw new Error(`Add lane offers nothing although the copy holds ${laneSample}: ${text.trim().slice(0, 90)}`);
    }
    const choice = choices.find(name => name.includes("everyday-check-lane")) ?? choices[0];
    await page().selectOption("#dialog-body select", { label: choice });
    await dialogButton("Add").click();
    await dialogClosed();
    await until(before => window.scene.lanes.length === before + 1, before, "the lane count did not go up by one");
    return `added ${choice} (${choices.length} offered)`;
  });

  for (const [name, id, part] of [["Browser", "view-browser", "#sample-browser"], ["Inspector", "view-inspector", '#editor-inspector section[aria-label="Selection"]'],
    ["Volume and Mixer", "view-controller", "#controller"]]) {
    await step(`Fold and show the ${name} panel`, async () => {
      // Shown: the switch is pressed and the panel's content has room; folded leaves a rail or a strip.
      const shown = () => read(([id, part]) => {
        const box = document.querySelector(part)?.getBoundingClientRect();
        const room = !!box && box.width > 120 && box.height > 60;
        return document.getElementById(id).getAttribute("aria-pressed") === "true" && room;
      }, [id, part]);
      const first = await shown();
      for (const want of [!first, first]) {
        await page().click(`#${id}`);
        const end = Date.now() + 4000;
        while (Date.now() < end && await shown() !== want) await sleep(100);
        expect(await shown() === want, `the ${name} switch did not ${want ? "show" : "fold"} the panel`);
      }
      return first ? "shown, folded, shown" : "folded, shown, folded";
    });
  }

  await step("Open and close Preferences (View settings…)", async () => {
    await fromMenu("View", "View settings…");
    await dialogShown("View");
    await dialogButton("Done").click();
    await dialogClosed();
  });

  await step("Open and close Folders…", async () => {
    await fromMenu("File", "Folders…");
    await dialogShown("Folders");
    await page().keyboard.press("Escape");
    await dialogClosed();
  });

  await step("Open and close the shortcut editor", async () => {
    await fromMenu("View", "Shortcuts");
    await dialogShown("Shortcuts and mouse actions");
    await dialogButton("Customise shortcuts").click();
    await dialogShown("Customise shortcuts");
    const rows = await read(() => document.getElementById("dialog-body").innerText.split("\n")
      .filter(line => /Unassigned|Clear/.test(line) || /^(Ctrl|Shift|Alt)$/.test(line.trim())).length);
    const listed = await read(() => document.getElementById("dialog-body").innerText.includes("Open difficulty or mapset"));
    expect(listed && rows > 5, `the shortcut editor lists no commands (${rows} key rows)`);
    await dialogButton("Close").click();
    await dialogClosed();
    return `${rows} rows`;
  });

  await step("Open and close the Export window", async () => {
    await page().click("#export-entry");
    const window = await windowOpened("export.html");
    await window.getByRole("button", { name: "Cancel" }).click();
    await windowClosed(window, "Export");
  });

  await step("Open and close the drum window", async () => {
    await fromMenu("Map", "Drum hit detection…");
    const window = await windowOpened("drum.html");
    await window.getByRole("button", { name: "Cancel" }).click();
    await windowClosed(window, "drum");
  });

  await step("Open and close the map preview window", async () => {
    // The preview sits in the Inspector: show it first if a step before folded it.
    if (await read(() => document.getElementById("view-inspector").getAttribute("aria-pressed") !== "true")) await page().click("#view-inspector");
    await fromMenu("View", "Map preview");
    const popOut = page().getByRole("button", { name: "Open in its own window" });
    await popOut.waitFor({ timeout: 5000 });
    await popOut.click();
    const window = await windowOpened("preview.html");
    await page().getByRole("button", { name: "Show it here" }).click();
    await windowClosed(window, "map preview");
    await fromMenu("View", "Map preview");
    expect(await read(() => typeof window.dawsuUi?.mapPreviewOn === "function"), "the page no longer has dawsuUi.mapPreviewOn to ask");
    await until(() => !window.dawsuUi.mapPreviewOn(), null, "View › Map preview did not switch the preview off", 4000);
  });

  await step("Open and close the node graph window", async () => {
    await fromMenu("Map", "Node graph…");
    const window = await windowOpened("node-graph.html");
    await window.locator("button", { hasText: /^Close$/ }).first().click();
    await windowClosed(window, "node graph");
  });

  await step("Zoom and scroll the timeline", async () => {
    const view = () => read(() => ({ zoom: window.scene.zoom, start: window.scene.viewStart, top: document.getElementById("scroll").scrollTop }));
    const before = await view();
    await page().click("#ruler-zoom-in");
    await until(zoom => window.scene.zoom > zoom, before.zoom, "Zoom in did not zoom in");
    const box = await read(() => document.getElementById("grid").getBoundingClientRect().toJSON());
    await page().mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    // The editor drops wheel turns while it is still answering the last view change, so turn the
    // wheel again, as a mapper would, up to four times.
    async function wheel(key, deltaY, done, arg, what) {
      for (let turn = 0; turn < 4; turn++) {
        if (key) await page().keyboard.down(key);
        await page().mouse.wheel(0, deltaY);
        if (key) await page().keyboard.up(key);
        if (await until(done, arg, what, 1500).then(() => true, () => false)) return;
      }
      throw new Error(`${what} after four turns`);
    }
    const zoomed = await view();
    await wheel("Control", -240, zoom => window.scene.zoom > zoom, zoomed.zoom, "Ctrl+wheel did not zoom in");
    const wheeled = await view();
    await wheel("Shift", 240, start => window.scene.viewStart > start, wheeled.start, "Shift+wheel did not scroll the timeline later");
    await page().mouse.wheel(0, -2000);
    await until(() => document.getElementById("scroll").scrollTop === 0, null, "the wheel did not scroll the lanes to the top");
    await page().mouse.wheel(0, 200);
    await until(() => document.getElementById("scroll").scrollTop > 0, null, "the wheel did not scroll the lanes down");
    await page().mouse.wheel(0, -2000);
    await page().click("#ruler-fit");
    await until(zoom => window.scene.zoom < zoom && window.scene.viewStart === 0, wheeled.zoom, "Fit all did not show the whole song");
    await until(() => document.getElementById("scroll").scrollTop === 0, null, "the lanes did not scroll back to the top");
  });

  // Two whistles to save: one on a plain circle, which Save writes into its line, and one on the
  // circle with a custom clap file, which Save can only write as a mix of new sample files.
  for (const [time, kind] of [[plan.plain, "the plain circle"], [plan.custom, `the circle with its own sound file (${plan.customFile})`]]) {
    await step(`Add a whistle to ${kind}${time === null ? "" : ` at ${time} ms`}`, async () => {
      if (!silent) return needsSilence();
      if (time === null) return { skip: "the map has no such circle" };
      // The whistle a mapper adds: the circle's own sample set and custom index, read from its hitnormal.
      // Lanes are told apart by index too: the skin's sound (index 0) and the map's file without a
      // number (index 1) share a name such as "soft-hitwhistle", and a whistle on the skin lane where the
      // map uses the file makes Save copy the skin sound to a new index.
      const whistle = await read(time => {
        const lanes = window.scene.lanes;
        const own = window.scene.moments.find(([t]) => t === time)?.[3].map(([row]) => lanes[row]) ?? [];
        const normal = own.find(lane => /-hitnormal$/.test(lane?.family ?? ""));
        const set = normal?.family.replace(/-hitnormal$/, "") ?? "normal", index = normal?.index ?? 0;
        const exact = lanes.findIndex(lane => lane.family === `${set}-hitwhistle` && lane.index === index);
        return exact >= 0 ? exact : lanes.findIndex(lane => lane.family === "normal-hitwhistle" && lane.index === 0);
      }, time);
      expect(whistle >= 0, "no whistle lane for the circle's sample set");
      const whistleName = await read(row => window.scene.lanes[row].name, whistle);
      await pickTool("Draw");
      await reveal(time, whistle);
      const before = (await hits()).length;
      // A little after the circle, inside its first quarter beat: the snap puts the hit on the circle.
      const point = await at(time + plan.nudge, whistle);
      await page().mouse.click(point.x, point.y);
      await until(([count, row, time]) => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0) === count + 1 &&
        window.scene.moments.some(([t, , , list]) => t === time && list.some(([r]) => r === row)), [before, whistle, time],
        `no whistle appeared on the circle at ${time} ms`);
      did({ do: "place", lane: whistleName, at: time + plan.nudge, seen: time });
      await pickTool("Select");
    });
  }

  await step("Save, through the save review", async () => {
    expect(await read(() => window.view.dirty), "the editor does not show unsaved changes");
    await page().click("#save");
    let reviewed = [];
    const end = Date.now() + 15000;
    while (Date.now() < end && await read(() => window.view.dirty || document.getElementById("dialog").open)) {
      const title = await dialogTitle();
      if (title?.startsWith("Review Save")) {
        reviewed = await read(() => Array.from(document.querySelectorAll("#dialog-body p"), p => p.textContent));
        await dialogButton(/^(Save|Done)$/).click();
      } else if (title?.startsWith("Save hitsounds")) {
        reviewed.push(title);
        await dialogButton(/^(Save|Continue to Save)$/).click();
      } else if (title) {
        throw new Error(`Save showed "${title}": ${(await read(() => document.getElementById("dialog-body").innerText)).slice(0, 120)}`);
      }
      await sleep(250);
    }
    expect(!(await read(() => window.view.dirty)), "the editor still shows unsaved changes");
    savedHits = await namedHits();
    did({ do: "save", seen: savedHits });
    return reviewed.join("; ");
  });

  await step("The original fixture folder is byte-identical", async () => {
    await quit();
    const after = snapshot(dirname(original));
    const changed = [...new Set([...originalBefore.keys(), ...after.keys()])].filter(name => originalBefore.get(name) !== after.get(name));
    expect(!changed.length, `changed: ${changed.join(", ")}`);
    return `${after.size} files`;
  }, true);

  await step("The copy's .osu changed only on the edited lines", async () => {
    const before = osuBefore.toString("latin1").split("\n");
    const after = readFileSync(mapCopy).toString("latin1").split("\n");
    expect(before.length === after.length, `the line count went from ${before.length} to ${after.length}`);
    const changed = before.map((line, index) => [index, line, after[index]]).filter(([, line, now]) => line !== now);
    expect(changed.length, "nothing in the .osu changed");
    const objects = before.indexOf("[HitObjects]") === -1 ? before.findIndex(line => line.trim() === "[HitObjects]") : before.indexOf("[HitObjects]");
    const edited = [plan.plain, plan.custom].filter(time => time !== null);
    const stray = changed.filter(([index, line]) => index <= objects || !edited.includes(Math.round(Number(line.split(",")[2]))));
    expect(!stray.length, `lines other than the circles at ${edited.join(" and ")} ms changed: ${stray.map(([index, line, now]) =>
      `line ${index + 1}: ${JSON.stringify(line)} → ${JSON.stringify(now)}`).join("; ")}`);
    return changed.map(([index, line, now]) => `line ${index + 1}: ${line.trim()} → ${now.trim()}`).join("; ");
  });

  await step("Save left no stray files in the mapset copy", async () => {
    const after = snapshot(mapFolder);
    // The Add lane step's own sample is not Save's.
    const added = [...after.keys()].filter(name => !copyBefore.has(name) && name !== laneSample);
    const removed = [...copyBefore.keys()].filter(name => !after.has(name));
    const changed = [...after.keys()].filter(name => copyBefore.has(name) && copyBefore.get(name) !== after.get(name));
    // A new sample file is one Save meant to write only when the saved difficulty plays it: its name
    // is written in the .osu, or it is a set-sound-index file whose custom index a hit object or
    // timing line uses.
    const osu = readFileSync(mapCopy, "latin1");
    const indexes = new Set();
    let section = "";
    for (const line of osu.split(/\r?\n/)) {
      if (/^\[.*\]$/.test(line.trim())) section = line.trim();
      const fields = line.split(",");
      if (section === "[TimingPoints]" && fields.length > 4) indexes.add(Number(fields[4]));
      if (section === "[HitObjects]" && fields.length > 5) {
        const sample = fields[fields.length - 1].split(":");
        if (sample.length >= 4) indexes.add(Number(sample[2]));
      }
    }
    const plays = name => osu.includes(name) || (() => {
      const match = /^(normal|soft|drum)-(hit(normal|whistle|finish|clap)|slider(slide|whistle|tick))(\d+)\.(wav|ogg|mp3)$/i.exec(name);
      return !!match && indexes.has(Number(match[5]));
    })();
    const unused = added.filter(name => !plays(name));
    mixFiles = added.filter(plays);
    expect(!unused.length && !removed.length, `unused new files: ${unused.join(", ") || "none"}; removed: ${removed.join(", ") || "none"}`);
    expect(changed.length === 1 && changed[0] === basename(mapCopy), `changed: ${changed.join(", ")}`);
    return added.length ? `${basename(mapCopy)} changed; new files the saved difficulty plays: ${added.join(", ")}`
      : `only ${basename(mapCopy)} changed`;
  });

  let reopened = null;
  const compare = (keep, what) => {
    expect(savedHits && reopened, "Save or the reopen did not finish, so there is nothing to compare");
    const saved = savedHits.filter(keep);
    const now = reopened.filter(keep);
    const missing = saved.filter(hit => !now.includes(hit));
    const extra = now.filter(hit => !saved.includes(hit));
    expect(!missing.length && !extra.length, `${what}: missing ${missing.join(", ") || "none"}; extra ${extra.join(", ") || "none"}`);
    return `${now.length} hits`;
  };
  const mixed = hit => plan.custom !== null && hit.startsWith(`${plan.custom} ms `);
  await step("Reopening the saved copy shows the same hits", async () => {
    expect(savedHits, "Save did not finish, so there is nothing to compare");
    await quit();
    await launch(mapCopy);
    await until(() => document.querySelectorAll("#lanes [data-lane-row]").length > 0 && window.scene?.lanes?.length > 0,
      null, "no lanes appeared after reopening", 60000);
    reopened = await namedHits();
    sounds.reopened = await playbackList();
    return compare(hit => !mixed(hit), "away from the mixed circle");
  });
  // Save writes a sound the circle's line cannot hold as a mix of new sample files, and the editor
  // project keeps no record of the lanes it was drawn on: reopened, the circle plays those files
  // (as crates/dawsu-application/src/lib.rs hs_save_mix_reopens_without_duplicate_project_hits expects).
  await step("Reopening shows the mixed circle as the sample files Save wrote", async () => {
    if (plan.custom === null) return { skip: "the map has no circle with its own sound file" };
    expect(reopened, "the reopen did not finish, so there is nothing to compare");
    const now = reopened.filter(mixed).map(hit => hit.slice(`${plan.custom} ms `.length));
    expect(now.length, `the circle at ${plan.custom} ms has no hits after reopening`);
    const written = mixFiles.map(name => name.replace(/\.(wav|ogg|mp3)$/i, ""));
    const other = now.filter(name => !written.includes(name));
    expect(written.length && !other.length, `at ${plan.custom} ms: ${now.join(", ")}; Save wrote: ${mixFiles.join(", ") || "nothing"}`);
    return now.join(", ");
  });
  await step("No errors after the last step", async () => {
    await sleep(500);
  }, true);
} finally {
  await quit();
  const passed = results.filter(result => result.status === "PASS").length;
  const failed = results.filter(result => result.status === "FAIL").length;
  const seconds = Math.round((Date.now() - started) / 1000);
  console.log(`run time: ${seconds} s`);
  console.log(`everyday check: ${passed} of ${results.length} steps passed`);
  if (args.json) {
    writeFileSync(resolve(args.json), JSON.stringify({ exe, map: original, profile, mapCopy, plan, passed, total: results.length, seconds, steps: results, actions, sounds }, null, 2) + "\n");
  }
  if (failed || !results.length) process.exitCode = 1;
  if (!args["keep-profile"]) {
    for (const name of readdirSync(profile)) {
      const folder = join(profile, name);
      if (folder === shots) continue;
      try {
        rmSync(folder, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
      } catch (error) {
        console.warn(`could not remove ${folder}: ${error.message}`);
      }
    }
    // The shots stay when a step failed; an empty profile folder goes.
    try {
      if (!args.shots && !readdirSync(shots).length) rmSync(shots, { recursive: true, force: true });
      if (!readdirSync(profile).length) rmSync(profile, { recursive: true, force: true });
    } catch {}
  }
}
