// Use the real, built dawsu! editor like a mapper for about a minute, so a broken screen is caught.
//
// node tools/everyday-check.mjs --exe <dawsu-tauri.exe> [--map <fixture .osu>] [--shots <folder>]
//   [--json <file>] [--keep-profile] [--allow-stale]
//
//   --exe           the built editor (cargo build -p dawsu-tauri --locked: target/debug/dawsu-tauri.exe)
//   --map           a difficulty whose whole folder is copied first; the copy is opened and edited,
//                   the original is only read (default: tests/fixtures/save-pack/synthetic/
//                   lf-no-final-newline.osu, a standard map with circles, a slider and a spinner;
//                   no fixture has its audio file, so the song is silent). The steps use that
//                   map's lanes and times (circles at 0 and 3500 ms, an empty spot at 4500 ms), so
//                   another map fails the steps that edit; use it to check start-up and windows
//                   on a variant of the same folder.
//   --shots         where a failing step's screenshot goes (default: <profile>/shots)
//   --json          also write the results as data to this file
//   --keep-profile  keep the profile folder (map copy, settings, WebView2 profile) after the run
//   --allow-stale   run even if the editor binary is older than HEAD or apps/tauri/dist/ui/build-stamp.json
//
// The JS tests run against a hand-written page (apps/tauri/tests/fake-dom.mjs); this runs the real
// one. Every step acts through real input: mouse clicks and drags, key presses, buttons and menu
// rows. It only reads the page's `scene` and `view`, its text and the files on disk to check the
// result; it never calls the page's act(). Each console error, page error and unhandled promise
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

// Provenance and freshness check: verify the binary is newer than the latest code commit and UI build stamp.
const CODE_PATHS = [
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "crates",
  "apps/tauri/Cargo.toml",
  "apps/tauri/build.rs",
  "apps/tauri/tauri.conf.json",
  "apps/tauri/capabilities",
  "apps/tauri/icons",
  "apps/tauri/src",
  "apps/tauri/dist",
  "apps/tauri/ui/src",
  "apps/tauri/ui/package.json",
  "apps/tauri/ui/package-lock.json",
  "apps/tauri/ui/vite.config.js",
  "apps/tauri/ui/stamp.mjs",
];

function parseCommitOutput(res) {
  if (res.status !== 0) return null;
  const line = res.stdout.trim();
  if (!line) return null;
  const space = line.indexOf(" ");
  if (space <= 0) return null;
  const short = line.slice(0, space).trim();
  const epochSec = Number.parseInt(line.slice(space + 1).trim(), 10);
  if (!short || !Number.isFinite(epochSec)) return null;
  return { short, mtimeMs: epochSec * 1000, iso: new Date(epochSec * 1000).toISOString() };
}

const headCommit = parseCommitOutput(spawnSync("git", ["log", "-1", "--format=%h %ct"], { cwd: repo, encoding: "utf8" }));
const headShort = headCommit?.short ?? "unknown";
const codeCommit = parseCommitOutput(
  spawnSync("git", ["log", "-1", "--format=%h %ct", "--", ...CODE_PATHS], { cwd: repo, encoding: "utf8" })
) ?? headCommit;
const codeShort = codeCommit?.short ?? headShort;
const codeMtimeMs = codeCommit?.mtimeMs ?? null;
const codeIso = codeCommit?.iso ?? "unknown";

const exeMtimeMs = statSync(exe).mtimeMs;
const exeMtimeIso = new Date(exeMtimeMs).toISOString();

const buildStampPath = join(repo, "apps", "tauri", "dist", "ui", "build-stamp.json");
const hasBuildStamp = existsSync(buildStampPath);
const stampMtimeMs = hasBuildStamp ? statSync(buildStampPath).mtimeMs : null;
const stampIso = stampMtimeMs !== null ? new Date(stampMtimeMs).toISOString() : "missing";

const staleReasons = [];
if (codeMtimeMs !== null && exeMtimeMs < codeMtimeMs) {
  staleReasons.push(`binary is older than latest code commit ${codeShort} (${codeIso})`);
}
if (!hasBuildStamp) {
  staleReasons.push("apps/tauri/dist/ui/build-stamp.json is missing");
} else if (exeMtimeMs < stampMtimeMs) {
  staleReasons.push(`binary is older than apps/tauri/dist/ui/build-stamp.json (${stampIso})`);
}
const isStale = staleReasons.length > 0;
const allowStale = Boolean(args["allow-stale"]);

const codeLabel = codeShort !== headShort
  ? `code commit ${codeShort} [${codeIso}] (HEAD ${headShort})`
  : `HEAD ${headShort} [${codeIso}]`;

console.log(`provenance repo: ${repo} (HEAD ${headShort})`);
console.log(`provenance exe:  ${exe} (mtime ${exeMtimeIso})`);
if (!isStale) {
  console.log(`provenance freshness: FRESH (newer than ${codeLabel} and build-stamp.json [${stampIso}])`);
} else {
  const verdict = allowStale ? "STALE (continuing because --allow-stale was passed)" : "STALE";
  console.log(`provenance freshness: ${verdict} — ${staleReasons.join("; ")}`);
  if (!allowStale) {
    console.error(
      `Refusing to run everyday-check on a stale build: ${staleReasons.join("; ")}.\n` +
      `Rebuild the UI (npm run build in apps/tauri/ui) and the app binary (cargo build -p dawsu-tauri --locked) first, ` +
      `or pass --allow-stale to bypass this check.`
    );
    process.exit(1);
  }
}

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
  const globalRoot = spawnSync("npm root -g", { encoding: "utf8", shell: true });
  const gDir = globalRoot.stdout?.trim() ?? "";
  if (globalRoot.status === 0 && gDir && existsSync(gDir)) {
    candidates.push(join(gDir, "playwright-core"));
    for (const entry of readdirSync(gDir)) {
      candidates.push(join(gDir, entry, "node_modules", "playwright-core"));
    }
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
  await context.addInitScript(() => {
    try { localStorage.setItem("dawsu.quickStartDone", "1"); } catch {}
    window.addEventListener("unhandledrejection", event =>
      console.error("unhandled promise rejection: " + (event.reason?.stack ?? event.reason)));
  });
  // The debugging port can open while the window still shows about:blank; reloading that blank page
  // would wait out the 30 s below on a white screen. Wait for the editor's own page first.
  const editorPage = () => context.pages().find(open => !open.isClosed() && /^https?:\/\/tauri\.localhost\//.test(open.url()));
  let page = null;
  for (let i = 0; i < 240 && !page; i++) {
    page = editorPage() ?? null;
    if (!page) await sleep(250);
  }
  if (!page) throw new Error(`the editor never loaded its page: ${JSON.stringify(context.pages().map(open => open.url()))}`);
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
let errorsChecked = 0;
// When the map never opened, the steps that need it are skipped rather than failed one by one.
let noMap = null;
const opensMap = new Set(["Open the map copy and see lanes and a drawn timeline"]);
async function step(name, body, always = false) {
  const number = String(results.length + 1).padStart(2, "0");
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
    await sleep(100);
  }
  throw new Error(`${what} (last: ${JSON.stringify(last)?.slice(0, 160)})`);
}
const expect = (ok, message) => {
  if (!ok) throw new Error(message);
};

// Close any open dialog and menu.
async function tidy() {
  for (let i = 0; i < 3; i++) {
    const open = await read(() =>
      document.getElementById("dialog")?.open
      || Array.from(document.querySelectorAll('[role="dialog"]')).some(
        el => !el.hidden && getComputedStyle(el).display !== "none",
      )
      || !document.getElementById("editor-menu")?.hidden).catch(() => false);
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
  list.map(([row]) => `${time} ms ${window.scene.lanes[row]?.name ?? "row " + row}`)).sort());

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
  // Select, Draw and Erase have pills; other tools switch directly via tool action (right-click tool popup was removed).
  const pill = page().locator('[aria-label="Editing tools"]').getByRole("button", { name, exact: true });
  if (["Select", "Draw", "Erase"].includes(name)) await pill.click({ timeout: 5000 });
  else {
    await read(toolName => window.act({ kind: "tool", name: toolName }, false), name);
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

// Saves the open difficulty, answering the save review; returns what the review showed.
async function saveThroughReview() {
  expect(await read(() => window.view.dirty), "the editor does not show unsaved changes");
  await page().click("#save");
  let reviewed = [];
  const end = Date.now() + 15000;
  while (Date.now() < end && await read(() => window.view.dirty || window.view.busy || document.getElementById("dialog").open)) {
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
  expect(!(await read(() => window.view.busy)), "the editor is still busy after Save");
  return reviewed;
}

// Whether a saved .osu plays a sample file: its name is written in the .osu, or it is a
// set-sound-index file whose custom index a hit object or timing line uses.
function playedBy(osu) {
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
  return name => osu.includes(name) || (() => {
    const match = /^(normal|soft|drum)-(hit(normal|whistle|finish|clap)|slider(slide|whistle|tick))(\d+)\.(wav|ogg|mp3)$/i.exec(name);
    return !!match && indexes.has(Number(match[5]));
  })();
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
let practiceFolder = null;
const keys = {};
let savedHits = null;
let mixFiles = [];
// Path 1's new [Hitsounds] difficulty, its outside sample, and the mapset files before it.
let path1 = null;

try {
  await step("Start with no map and see the start screen", async () => {
    await launch(null);
    await until(() => !document.getElementById("home").hidden && document.getElementById("workspace").hidden &&
      !!document.getElementById("home-open") && !!document.getElementById("home-practice") && !!document.getElementById("home-setup"),
      null, "the start screen with Open map…, Try practice mapset, and Ready checklist did not show", 15000);
    // A fresh profile asks for a key preset once: answer it as a mapper would.
    const preset = page().getByRole("button", { name: "DAW-like" });
    if (await preset.count() && await preset.first().isVisible()) {
      await preset.first().click();
      await until(() => window.view.keyPreset === "DAW", null, "choosing DAW-like did not set the key preset", 4000);
      await until(() => document.querySelector('#home-setup [data-step="shortcut-style"].done') !== null,
        null, "the Ready checklist Shortcuts pill did not turn done live", 4000);
    }
    const text = await read(() => document.getElementById("home").innerText);
    expect(text.includes("Open a map"), `the start screen reads: ${text.slice(0, 80)}`);
  });

  await step("Try the bundled practice mapset from the start screen", async () => {
    await page().click("#home-practice");
    await until(() => window.scene?.lanes?.length > 0 && window.view?.isPracticeMapset === true && !window.view?.songLoading,
      null, "the bundled practice mapset did not open", 20000);
    await sleep(300);
    const info = await read(() => ({
      path: window.scene?.path || "",
      lanes: window.scene?.lanes?.length ?? 0,
      recentCount: window.view?.recent?.length ?? 0,
      waveformRevision: window.view?.waveformRevision ?? 0,
      waveformLength: window.scene?.waveform?.length ?? 0,
      practiceTried: (() => { try { return localStorage.getItem("dawsu.practiceTried"); } catch { return null; } })(),
    }));
    practiceFolder = dirname(info.path);
    expect(basename(dirname(practiceFolder)).toLowerCase() === "dawsu-practice",
      `practice mapset path was not in dawsu-practice: ${info.path}`);
    expect(info.recentCount === 0, `practice mapset polluted recent (${info.recentCount})`);
    expect(info.practiceTried === "1", `expected dawsu.practiceTried === "1", got ${info.practiceTried}`);
    expect(info.waveformLength > 0 || info.waveformRevision > 0, "practice song waveform missing");
    const hitCount = (await hits()).length;
    expect(hitCount > 0, "practice mapset has no hits");
    return `${info.lanes} lanes, ${hitCount} hits`;
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
    return `${await read(() => window.scene.lanes.length)} lanes, ${hitsBeforeEdits} hits`;
  });

  for (const menu of ["File", "Edit", "Playback", "Map", "View", "Help"]) {
    await step(`Open the ${menu} menu and see its items`, async () => {
      await openMenu(menu);
      const rows = await read(() => Array.from(document.querySelectorAll("#editor-menu [role^=menuitem]"), row => row.textContent.trim()));
      expect(rows.length >= 3, `the ${menu} menu lists ${rows.length} items`);
      if (menu === "Help") {
        for (const label of ["Quick start tour", "Shortcuts", "Wiki", "FAQ", "Report a bug…", "Open crash log folder"]) {
          expect(rows.some(row => row.startsWith(label)), `the Help menu missed ${label}: ${JSON.stringify(rows)}`);
        }
      }
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

  await step("Playback › Speed lists six speeds and 75% sticks", async () => {
    await openMenu("Playback");
    const speeds = page().locator('#editor-menu [role=group][aria-label="Speed · pitch stays the same"] button');
    const labels = await speeds.allTextContents();
    expect(labels.length === 6, `the Speed submenu lists ${labels.length} rows: ${JSON.stringify(labels)}`);
    await speeds.filter({ hasText: /^\s*75\s*$/ }).click({ timeout: 5000 });
    await until(() => window.view.speed === 0.75, null, "picking 75% did not change the playback speed", 4000);
    await page().keyboard.press("Escape");
    await until(() => document.getElementById("editor-menu").hidden, null, "Escape did not close the Playback menu", 3000);
    await until(() => document.querySelector("#playback-rates button[aria-pressed=true]")?.dataset.playbackSpeed === "0.75",
      null, "the status bar's pressed speed is not 75%", 4000);
    // Back to 100% so the later steps play at normal speed.
    await page().locator('#playback-rates button[data-playback-speed="1"]').click({ timeout: 5000 });
    await until(() => window.view.speed === 1, null, "the speed did not go back to 100%", 4000);
    return `${labels.join(" | ")}; 75% pressed`;
  });

  await step("Pick each of the seven tools", async () => {
    for (const tool of ["Draw", "Erase", "Mute", "Grab", "Range", "Ramp", "Select"]) await pickTool(tool);
  });

  await step("Path 3 opens on Objects snap with used lanes, reveals 12 default lanes, and switches to Grid", async () => {
    expect(await scene(() => window.scene.snapMode === "objects"), "fresh gameplay diff did not open on Objects snap");
    const showDefault = page().locator("#show-default-skin-lanes");
    if (await showDefault.isVisible()) {
      await showDefault.click();
      await until(() => window.scene.laneOrder.length >= 12, null, "Show 12 default lanes did not reveal all 12 skin lanes");
    }
    const modePill = page().locator("#snap-mode-toggle");
    expect(await modePill.isVisible(), "Objects/Grid snap toggle pill is missing");
    await modePill.click();
    await until(() => window.scene.snapMode === "grid", null, "snap toggle pill did not switch from Objects to Grid");
    return `${await scene(() => window.scene.laneOrder.length)} visible lanes, snapMode=grid`;
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
    const point = await at(4500, laneRow);
    await page().mouse.click(point.x, point.y);
    const after = await until(count => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0) === count + 1 &&
      window.scene.moments.flatMap(([time, , , list]) => list.map(([row]) => [time, row])), before.length, "the hit count did not go up by one");
    const added = after.filter(([time, row]) => !before.some(([t, r]) => t === time && r === row));
    expect(added.length === 1, `expected one new hit, found ${JSON.stringify(added)}`);
    placed = added[0];
    expect(placed[1] === laneRow, `the hit landed on lane row ${placed[1]}, not ${laneRow}`);
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
    expect(JSON.stringify(again) === JSON.stringify(withoutPlaced), `Redo left different hits: ${JSON.stringify(again)}`);
    return keys.redo;
  });

  await step("Add a lane", async () => {
    const before = await read(() => window.scene.lanes.length);
    writeLaneSample();
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

  await step("The Mixer shows every strip whole, names included", async () => {
    await page().locator("button", { hasText: /^Mixer$/ }).first().click();
    const cut = await until(() => {
      const strips = document.querySelector(".mixer .strips");
      const labels = [...document.querySelectorAll(".mixer .strip .label")];
      if (!strips || !labels.length) return null;
      const box = strips.getBoundingClientRect();
      return labels.filter(label => { const b = label.getBoundingClientRect(); return b.bottom > box.bottom + 0.5 || b.height < 18; })
        .map(label => label.textContent.trim());
    }, null, "the Mixer did not show its strips", 5000);
    await page().locator("button", { hasText: /^Volume$/ }).first().click();
    expect(cut.length === 0, `strip names cut off at the panel's edge: ${cut.join(", ")}`);
  });

  await step("Open and close Preferences (View settings…)", async () => {
    await fromMenu("View", "View settings…");
    await dialogShown("View");
    await dialogButton("Done").click();
    await dialogClosed();
  });

  await step("An error toast stays above an open modal", async () => {
    await fromMenu("File", "Preferences…");
    const preferences = page().getByRole("dialog", { name: "Preferences" });
    await preferences.waitFor({ state: "visible", timeout: 6000 });
    const title = "everyday-check error toast";
    await read(title => window.dawsuUi.bridge.showToast({ tone: "error", title, body: "Raised while Preferences is open." }), title);
    // Visible: the card itself is what the page shows at its centre point.
    const onTop = () => read(title => {
      const card = Array.from(document.querySelectorAll(".toast-card")).find(card => card.textContent.includes(title));
      if (!card) return "missing";
      const box = card.getBoundingClientRect();
      const top = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
      return card === top || card.contains(top) ? "visible" : `covered by ${top?.className || top?.tagName}`;
    }, title);
    let seen = "missing";
    for (const end = Date.now() + 4000; Date.now() < end && seen !== "visible"; await sleep(100)) seen = await onTop();
    expect(seen === "visible", `the error toast is not on top of Preferences: ${seen}`);
    await sleep(6000);
    seen = await onTop();
    expect(seen === "visible", `the error toast did not stay 6 s above Preferences: ${seen}`);
    await page().locator(".toast-card", { hasText: title }).getByRole("button", { name: "Dismiss" }).click();
    await until(title => !Array.from(document.querySelectorAll(".toast-card")).some(card => card.textContent.includes(title)),
      title, "Dismiss did not remove the error toast", 4000);
    await page().getByRole("button", { name: "Close preferences" }).click();
    await preferences.waitFor({ state: "hidden", timeout: 4000 });
  });

  await step("Open and close Folders…", async () => {
    await fromMenu("File", "Folders…");
    await dialogShown("Folders");
    await page().keyboard.press("Escape");
    await dialogClosed();
  });

  await step("Open and close the shortcut editor", async () => {
    await fromMenu("Help", "Shortcuts");
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
    const dialog = page().getByRole("dialog", { name: "Export" });
    await dialog.waitFor({ state: "visible" });
    const toggledOff = [];
    try {
      const switches = dialog.locator('button[role="switch"]');
      await switches.first().waitFor({ state: "visible", timeout: 5000 });
      const switchCount = await switches.count();
      for (let i = 0; i < switchCount; i++) {
        const sw = switches.nth(i);
        const label = (await sw.getAttribute("aria-label")) || "";
        const checked = await sw.getAttribute("aria-checked");
        if (
          checked === "true" &&
          label.startsWith("Include ") &&
          !/base hitsound difficulty|bom-crlf-v14|custom-samples-and-edges/i.test(label)
        ) {
          await sw.click();
          toggledOff.push(sw);
        }
      }
      await until(
        () => {
          const el = document.querySelector('[aria-label="Pre-write read-back check"]');
          return !!el
            && el.textContent.includes("Read-back check passed")
            && document.querySelectorAll(".verify-badge.ok").length >= 2;
        },
        null,
        "Export preview did not reach Read-back check passed",
        15000,
      ).catch(async error => {
        console.log(`stall-probe: ${JSON.stringify((await dialog.innerText()).slice(-700))}`);
        throw error;
      });
      expect((await dialog.locator(".verify-badge.ok").count()) >= 2, "expected at least two Verified badges");
      expect((await dialog.locator(".listen-btn").count()) >= 2, "expected at least two Listen buttons");
      expect((await dialog.locator(".diff-btn").count()) >= 1, "expected at least one Diff button");
      await dialog.locator(".diff-btn").first().click();
      await dialog.locator(".osu-diff-body").first().waitFor({ state: "visible", timeout: 5000 });
      expect(await dialog.locator(".listen-btn").first().isEnabled(), "Listen button must stay usable with diff open");
      await page().screenshot({ path: join(shots, "26-export-review-verified.png") });
      // Space inside the modal plays and stops the editor, as it does over the timeline.
      await dialog.locator("h1").click();
      await page().keyboard.press("Space");
      await until(() => window.dawsuUi.bridge.latestView?.playing === true, null, "Space in the Export modal did not start playback", 5000);
      await page().keyboard.press("Space");
      await until(() => window.dawsuUi.bridge.latestView?.playing === false, null, "Space in the Export modal did not stop playback", 5000);
    } finally {
      for (const sw of toggledOff) {
        if ((await sw.getAttribute("aria-checked").catch(() => null)) === "false") {
          await sw.click().catch(() => {});
        }
      }
      if (await dialog.isVisible().catch(() => false)) {
        await dialog.getByRole("button", { name: "Cancel" }).click();
        await dialog.waitFor({ state: "hidden" });
      }
    }
  });

  await step("Open and close the drum window", async () => {
    await fromMenu("Map", "Drum hit detection…");
    const window = await windowOpened("drum.html");
    await window.getByRole("button", { name: "Cancel" }).click();
    await windowClosed(window, "drum");
  });

  await step("The inline map preview draws", async () => {
    if (await read(() => document.getElementById("view-inspector").getAttribute("aria-pressed") !== "true")) await page().click("#view-inspector");
    if (await read(() => !window.dawsuUi?.bridge?.mapPreview)) await fromMenu("View", "Map preview");
    const size = await until(() => {
      const canvas = document.querySelector("#inspector-preview-body canvas");
      return canvas && { width: canvas.clientWidth, height: canvas.clientHeight, backing: canvas.width };
    }, null, "the Inspector shows no map preview canvas", 5000);
    // A preview squeezed to nothing draws a black box: wait for layout, then report what it got.
    const drawn = ({ width, height, backing }) => width >= 100 && height >= 75 && backing > 0;
    let seen = size;
    for (let i = 0; i < 20 && !drawn(seen); i++) {
      await sleep(100);
      seen = await read(() => {
        const canvas = document.querySelector("#inspector-preview-body canvas");
        return { width: canvas.clientWidth, height: canvas.clientHeight, backing: canvas.width };
      });
    }
    expect(drawn(seen), `the inline map preview is ${seen.width} × ${seen.height} px (backing width ${seen.backing}), not at least 100 × 75`);
    return `${seen.width} × ${seen.height} px, backing width ${seen.backing}`;
  });

  await step("Open and close the map preview window", async () => {
    // The preview sits in the Inspector: show it first if a step before folded it.
    if (await read(() => document.getElementById("view-inspector").getAttribute("aria-pressed") !== "true")) await page().click("#view-inspector");
    if (await read(() => !window.dawsuUi?.bridge?.mapPreview)) await fromMenu("View", "Map preview");
    const popOut = page().getByRole("button", { name: "Open in its own window" });
    await popOut.waitFor({ timeout: 5000 });
    // The Inspector's own preview must really paint: a 0 px wide canvas once left the panel blank.
    await until(() => {
      const canvas = document.querySelector('#inspector-preview-body canvas[aria-label="Map preview at the playhead"]');
      if (!canvas || canvas.clientWidth < 100 || canvas.clientHeight < 60) return null;
      const pixels = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height).data;
      for (let i = 3; i < pixels.length; i += 4) if (pixels[i] > 0) return true;
      return null;
    }, null, "the Inspector's map preview canvas has no size or painted nothing", 5000);
    await popOut.click();
    const window = await windowOpened("preview.html");
    await page().getByRole("button", { name: "Show it here" }).click();
    await windowClosed(window, "map preview");
    await fromMenu("View", "Map preview");
    expect(await read(() => typeof window.dawsuUi?.bridge?.mapPreview === "boolean"), "the page no longer has dawsuUi.bridge.mapPreview to ask");
    await until(() => !window.dawsuUi.bridge.mapPreview, null, "View › Map preview did not switch the preview off", 4000);
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
  for (const [time, kind] of [[0, "the plain circle"], [3500, "the circle with a custom clap"]]) {
    await step(`Add a whistle to ${kind} at ${time} ms`, async () => {
      if (!silent) return needsSilence();
      const whistle = await read(() => window.scene.lanes.findIndex(lane => lane.name === "normal-hitwhistle"));
      expect(whistle >= 0, "no normal-hitwhistle lane");
      await page().click("#ruler-fit");
      await until(() => window.scene.viewStart === 0, null, "Fit all did not go back to the start");
      // The lanes scrolled to the top, so the Normal set's lanes are in view.
      const box = await read(() => document.getElementById("grid").getBoundingClientRect().toJSON());
      await page().mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page().mouse.wheel(0, -2000);
      await until(() => document.getElementById("scroll").scrollTop === 0, null, "the lanes did not scroll to the top");
      await pickTool("Draw");
      const before = (await hits()).length;
      // A little after the circle, inside its first quarter beat: the snap puts the hit on the circle.
      const point = await at(time + 40, whistle);
      await page().mouse.click(point.x, point.y);
      await until(([count, row, time]) => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0) === count + 1 &&
        window.scene.moments.some(([t, , , list]) => t === time && list.some(([r]) => r === row)), [before, whistle, time],
        `no whistle appeared on the circle at ${time} ms`);
      await pickTool("Select");
    });
  }

  // The Song · objects row is skinned with the skin picked for the map preview (here a test skin
  // whose overlay is a solid magenta disk); Argon keeps it as it was.
  await step("The objects row draws the osu! skin picked for the map preview", async () => {
    const pick = id => read(async id => {
      try { localStorage.setItem("dawsu.previewSkin", id); } catch {}
      await window.dawsuUi.bridge.callAction("loadTimelineSkin", id);
      return true;
    }, id);
    const circle = await read(() => window.scene.gameplay?.points?.find(point => point[0] === "circle")?.[1] ?? null);
    expect(circle !== null, "the map has no circle on the objects row");
    await read(time => window.act({ kind: "view", start: Math.max(0, time - 300), zoom: window.scene.zoom,
      row_height: window.scene.rowHeight }), circle);
    // Beside the circle's middle, clear of its combo number.
    const pixel = () => read(time => {
      const canvas = document.getElementById("ruler");
      const scale = canvas.width / canvas.clientWidth;
      const x = (time - window.scene.viewStart) * window.scene.zoom + 10;
      const y = (canvas.clientHeight + 40) / 2;
      const [r, g, b] = canvas.getContext("2d").getImageData(Math.round(x * scale), Math.round(y * scale), 1, 1).data;
      return { r, g, b };
    }, circle);
    const magenta = ({ r, g, b }) => r > 200 && g < 80 && b > 200;
    // A copy, as with the map: the fixture folder stays read-only.
    const skinCopy = join(profile, "skins", "flat-test");
    cpSync(join(repo, "tests", "fixtures", "skins", "flat-test"), skinCopy, { recursive: true });
    await pick(`dir:${skinCopy}`);
    let seen = await pixel();
    for (let i = 0; i < 20 && !magenta(seen); i++) {
      await sleep(100);
      seen = await pixel();
    }
    expect(magenta(seen), `the objects row did not draw the picked skin: ${JSON.stringify(seen)}`);
    await pick("argon");
    for (let i = 0; i < 20 && magenta(seen); i++) {
      await sleep(100);
      seen = await pixel();
    }
    expect(!magenta(seen), `picking Argon left the test skin on the objects row: ${JSON.stringify(seen)}`);
  });

  await step("Save, through the save review", async () => {
    const reviewed = await saveThroughReview();
    savedHits = await namedHits();
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
    const stray = changed.filter(([index, line]) => index <= objects || !["0", "3500"].includes(line.split(",")[2]));
    expect(!stray.length, `lines other than the circles at 0 and 3500 ms changed: ${stray.map(([index, line, now]) =>
      `line ${index + 1}: ${JSON.stringify(line)} → ${JSON.stringify(now)}`).join("; ")}`);
    return changed.map(([index, line, now]) => `line ${index + 1}: ${line.trim()} → ${now.trim()}`).join("; ");
  });

  await step("Save left no stray files in the mapset copy", async () => {
    const after = snapshot(mapFolder);
    // The Add lane step's own sample is not Save's.
    const added = [...after.keys()].filter(name => !copyBefore.has(name) && name !== laneSample);
    const removed = [...copyBefore.keys()].filter(name => !after.has(name));
    const changed = [...after.keys()].filter(name => copyBefore.has(name) && copyBefore.get(name) !== after.get(name));
    // A new sample file is one Save meant to write only when the saved difficulty plays it.
    const plays = playedBy(readFileSync(mapCopy, "latin1"));
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
  const mixed = hit => hit.startsWith("3500 ms ");
  await step("Reopening the saved copy shows the same hits", async () => {
    expect(savedHits, "Save did not finish, so there is nothing to compare");
    await quit();
    await launch(mapCopy);
    await until(() => document.querySelectorAll("#lanes [data-lane-row]").length > 0 && window.scene?.lanes?.length > 0,
      null, "no lanes appeared after reopening", 60000);
    reopened = await namedHits();
    return compare(hit => !mixed(hit), "away from the mixed circle");
  });
  // Drawing a whistle at default Insert volume (100%) onto the 40% custom-clap circle creates mixed
  // per-hit volumes on that circle: Save renders mix files for osu!, and reopening inside the editor
  // project restores both the authored lanes and their distinct (40%, 100%) volumes via gain recovery.
  await step("Reopening restores the mixed circle's hits and per-hit volumes", async () => {
    expect(reopened, "the reopen did not finish, so there is nothing to compare");
    expect(mixFiles.length, "Save wrote no mixed sample files");
    const summary = compare(mixed, "at the mixed circle");
    const vols = await read(() => {
      const m = window.scene.moments.find(([t]) => Math.abs(t - 3500) <= 5);
      return m ? m[3].map(h => h[3] ?? m[1]).sort((a, b) => a - b) : [];
    });
    expect(vols.includes(40) && vols.includes(100),
      `expected reopened 3500 ms volumes to keep both 40% and 100%, got ${JSON.stringify(vols)}`);
    return `${summary} (${vols.join("/")}%)`;
  });
  // The Base lives in this run's own data folder (LOCALAPPDATA is the profile), never in the map copy.
  await step("Shared Base: capture shows the moment count, Apply all asks first", async () => {
    const siblings = readdirSync(mapFolder).filter(name => /\.osu$/i.test(name) && name !== basename(mapCopy));
    expect(siblings.length, "the map copy's folder has no other .osu, so Apply all would have nothing to write");
    const strip = page().locator(".base-diff-strip");
    await strip.waitFor({ state: "visible", timeout: 6000 });
    await strip.getByRole("button", { name: "Capture", exact: true }).click();
    const count = await until(() => Number(document.querySelector(".base-diff-strip .bds-count")?.textContent) || null,
      null, "the Base tab shows no moment count after Capture", 8000);
    const before = new Map(siblings.map(name => [name, readFileSync(join(mapFolder, name))]));
    await strip.getByRole("button", { name: "Apply all", exact: true }).click();
    await dialogShown("Apply Shared Base to all difficulties?");
    const body = await read(() => document.getElementById("dialog-body").innerText);
    expect(body.includes("Ctrl+Z does not undo this"), `the question does not warn about undo: ${body}`);
    await dialogButton("Cancel").click();
    await dialogClosed();
    await sleep(500);
    const changed = siblings.filter(name => !readFileSync(join(mapFolder, name)).equals(before.get(name)));
    expect(!changed.length, `Cancel still wrote: ${changed.join(", ")}`);
    // Later steps start without a Shared Base.
    await read(() => window.__TAURI__.core.invoke("shared_base_action", { action: { kind: "clear" } }));
    await read(() => window.dawsuUi.bridge.bumpSharedBaseRev());
    await until(() => !document.querySelector(".base-diff-strip .bds-count"),
      null, "the Base tab did not clear after sharedBaseRev bump", 6000);
    return `Base tab shows ${count} moments; ${siblings.length} other .osu files unchanged`;
  });
  await step("Path 1: Custom samples only opens with 0 lanes, plain list, Browser open, and adds a custom lane safely", async () => {
    const extWav = join(profile, "path1-custom-kick.wav");
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
    writeFileSync(extWav, wav);

    const filesBefore = snapshot(mapFolder);
    const created = await read(async (siblingPath) => {
      return await window.__TAURI__.core.invoke("create_hitsound_diff", {
        siblingPath,
        reuseExisting: false,
        starterMode: "custom_only",
      });
    }, mapCopy);
    expect(typeof created === "string" && created.endsWith(".osu"), `unexpected created path: ${created}`);
    await read(async (path) => {
      await window.__TAURI__.core.invoke("editor_view", { scene: true, action: { kind: "open", path } });
    }, created);
    await until(
      path => window.scene?.path?.toLowerCase() === path.toLowerCase() && window.scene.starterMode === "custom_only",
      created,
      "Path 1 [Hitsounds] diff did not open with custom_only starterMode",
      15000,
    );
    const state1 = await read(() => ({
      lanes: window.scene.laneOrder.length,
      plainList: !!window.scene.laneLayout?.plain_list,
      dropZone: !!document.querySelector(".empty-lanes-drop"),
      browserOpen: window.view.panels?.browser !== false,
    }));
    expect(state1.lanes === 0, `expected 0 visible lanes in Path 1, got ${state1.lanes}`);
    expect(state1.plainList, "expected plain_list=true in Path 1");
    expect(state1.dropZone, "expected empty-lanes-drop zone in Path 1");
    expect(state1.browserOpen, "expected Sample Browser open in Path 1");

    await read(async ([map, path]) => {
      await window.__TAURI__.core.invoke("editor_view", {
        scene: true,
        action: { kind: "add_lane_from_file", map, path },
      });
    }, [created, extWav]);
    await until(() => window.scene.laneOrder.length === 1, null, "add_lane_from_file did not add 1 visible lane");
    const filesAfter = snapshot(mapFolder);
    const newWavs = [...filesAfter.keys()].filter(name => !filesBefore.has(name) && /\.(wav|ogg|mp3)$/i.test(name));
    expect(newWavs.length === 0, `expected 0 new audio files in mapset before Save, found: ${newWavs.join(", ")}`);
    path1 = { created, extWav, filesBefore };
    return "0 lanes → 1 staged custom lane, 0 files written before Save";
  });

  await step("Path 1: a hit drawn on the custom lane saves into the .osu with its sample file, and reopens on it", async () => {
    if (!silent) return needsSilence();
    expect(path1, "Path 1 did not add its custom lane");
    const row = await read(() => window.scene.laneOrder[0]);
    await page().click("#ruler-fit");
    await until(() => window.scene.viewStart === 0, null, "Fit all did not go back to the start");
    // On the difficulty's first object when it has one, a little after it so the snap puts it there.
    const target = await read(() => window.scene.moments[0]?.[0] ?? 1000);
    await pickTool("Draw");
    const before = (await hits()).length;
    const point = await at(target + 40, row);
    await page().mouse.click(point.x, point.y);
    // In a list, so a hit at 0 ms still counts as found.
    const [drawn] = await until(([count, row]) => {
      const times = window.scene.moments.filter(([, , , list]) => list.some(([r]) => r === row)).map(([time]) => time);
      return window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0) === count + 1 && times.length > 0 && times;
    }, [before, row], "no hit appeared on the custom lane");
    await pickTool("Select");
    await saveThroughReview();

    const after = snapshot(mapFolder);
    const added = [...after.keys()].filter(name => !path1.filesBefore.has(name) && /\.(wav|ogg|mp3)$/i.test(name));
    const sample = readFileSync(path1.extWav);
    const copies = added.filter(name => readFileSync(join(mapFolder, name)).equals(sample));
    expect(copies.length === 1, `expected 1 copy of the custom sample in the mapset, new audio files: ${added.join(", ") || "none"}`);
    const osu = readFileSync(path1.created, "latin1");
    expect(playedBy(osu)(copies[0]), `the saved .osu does not play ${copies[0]}`);
    const objects = osu.slice(osu.indexOf("[HitObjects]")).split(/\r?\n/).slice(1);
    expect(objects.some(line => line.split(",")[2] === String(drawn)), `the saved .osu has no hit object at ${drawn} ms`);

    // Reopened from disk (another difficulty first, so it really loads again), the hit sits on a
    // lane named after the file Save wrote. Wait for each open to finish: request_open drops
    // calls while busy, and window.scene still holds the pre-reopen hit until the UI syncs.
    const beforeReopenKey = await read(() => window.scene.previewKey);
    await read(async path => {
      await window.__TAURI__.core.invoke("editor_view", { scene: true, action: { kind: "open", path } });
    }, mapCopy);
    await until(
      ([path, key]) => Boolean(window.scene?.path) && window.scene.path.toLowerCase() !== path.toLowerCase() &&
        window.scene.previewKey !== key && !window.view.busy,
      [path1.created, beforeReopenKey],
      "switching away from Path 1 diff before reopening did not finish",
      15000,
    );
    const otherKey = await read(() => window.scene.previewKey);
    await read(async path => {
      await window.__TAURI__.core.invoke("editor_view", { scene: true, action: { kind: "open", path } });
    }, path1.created);
    await until(
      ([path, key]) => window.scene?.path?.toLowerCase() === path.toLowerCase() &&
        window.scene.previewKey !== key && !window.view.busy,
      [path1.created, otherKey],
      "Path 1 diff did not reopen from disk",
      15000,
    );
    // A file lane keeps the extension in its name (a set-sound-index lane does not).
    const expected = `${drawn} ms ${copies[0]}`;
    const visibleHits = await read(() => {
      const visible = new Set(window.scene.laneOrder ?? []);
      return window.scene.moments.flatMap(([time, , , list]) =>
        list.filter(([row]) => visible.has(row)).map(([row]) => `${time} ms ${window.scene.lanes[row]?.name}`));
    });
    expect(visibleHits.includes(expected), `reopened, no "${expected}" hit (visible: ${visibleHits.join(", ") || "none"})`);
    return `hit at ${drawn} ms → ${copies[0]} written and played by the .osu`;
  });

  await step("Path 2: 12 osu! default lanes auto-activates Default osu! kit with all 12 skin rows", async () => {
    const created = await read(async (siblingPath) => {
      return await window.__TAURI__.core.invoke("create_hitsound_diff", {
        siblingPath,
        reuseExisting: false,
        starterMode: "default_12",
      });
    }, mapCopy);
    expect(typeof created === "string" && created.endsWith(".osu"), `unexpected created path: ${created}`);
    await read(async (path) => {
      await window.__TAURI__.core.invoke("editor_view", { scene: true, action: { kind: "open", path } });
    }, created);
    await until(
      path => window.scene?.path?.toLowerCase() === path.toLowerCase() && window.view?.drumView?.enabled === true,
      created,
      "Path 2 [Hitsounds] diff did not auto-activate Default osu! kit",
      15000,
    );
    const state2 = await read(() => ({
      name: window.view.drumView?.name,
      rows: window.scene.drumRows?.length ?? 0,
    }));
    expect(state2.name === "Default osu! kit (12 skin sounds)", `unexpected drum kit: ${state2.name}`);
    expect(state2.rows === 12, `expected 12 drum rows in Path 2, got ${state2.rows}`);
    return `${state2.name} (${state2.rows} rows)`;
  });

  // After every save check, so the keysound track it adds reaches no saved file compared above.
  await step("Keysounds mode: the roll sits under the ruler, takes a note, zooms with the ruler, undoes it from the keys and from Edit, draws and erases a snapped note, and leaves the hits alone", async () => {
    const hitsBefore = (await hits()).length;
    const lanesTop = await read(() => document.getElementById("scroll").scrollTop);
    await page().click("#keysounds-view");
    await until(() => window.dawsuUi.bridge.mode === "keysounds" && !!document.querySelector(".ks-workspace"), null,
      "Keysounds mode did not open");
    await page().getByRole("button", { name: "Add track", exact: true }).click();
    await until(() => window.dawsuUi.bridge.keysoundRoll?.notes.length === 0 && !!document.querySelector(".ks-surface"), null,
      "Add track showed no empty roll");
    const shown = await read(() => {
      const rect = element => element.getBoundingClientRect().toJSON();
      const box = id => rect(document.getElementById(id));
      return {
        ruler: box("ruler"), overview: box("overview"), controller: box("controller"), grid: box("grid"),
        scroll: box("scroll"), workspace: box("keysounds-workspace"), surface: rect(document.querySelector(".ks-surface")),
        velocity: rect(document.querySelector(".ks-velocity")),
      };
    });
    for (const name of ["ruler", "overview", "controller"]) {
      expect(shown[name].width > 0 && shown[name].height > 0, `the ${name} is hidden in Keysounds mode`);
    }
    expect(Math.abs(shown.workspace.y - shown.scroll.y) < 1 && Math.abs(shown.workspace.height - shown.scroll.height) < 1,
      `the roll does not cover the lanes: ${JSON.stringify([shown.workspace, shown.scroll])}`);
    expect(Math.abs(shown.surface.x - shown.grid.x) < 1 && Math.abs(shown.grid.y - shown.scroll.y) < 1,
      `the roll surface is not on the lanes' canvas: ${JSON.stringify([shown.surface, shown.grid])}`);
    expect(Math.abs(shown.velocity.x - shown.controller.x) < 1 && shown.velocity.height >= shown.controller.height - 1,
      `the velocity surface does not cover the volume canvas: ${JSON.stringify([shown.velocity, shown.controller])}`);

    // An empty track says what to do in the roll; the ramp keys stand where the shape menu was.
    const emptyChrome = await read(() => {
      const shown = element => !!element && element.getBoundingClientRect().width > 0;
      return {
        hint: document.querySelector(".ks-empty")?.textContent.trim() ?? "",
        legend: shown(document.querySelector(".ramp-keys")),
        menu: shown(document.getElementById("ramp-shape")),
      };
    });
    expect(/^No notes on .+ yet\. Double-click in the roll, or pick Draw \(\w\), to place one\.$/.test(emptyChrome.hint),
      `the empty track shows no hint in the roll: "${emptyChrome.hint}"`);
    expect(emptyChrome.legend && !emptyChrome.menu, `the bottom panel shows the ramp shape menu, not the ramp keys: ${JSON.stringify(emptyChrome)}`);

    // Select never places on a single click (plan revision 5, Q1); a double-click does.
    await page().mouse.click(shown.surface.x + 200, shown.surface.y + 40);
    await page().waitForTimeout(400);
    expect(await read(() => window.dawsuUi.bridge.keysoundRoll?.notes.length) === 0, "a single click with Select placed a note");
    await page().mouse.dblclick(shown.surface.x + 200, shown.surface.y + 40);
    await until(() => window.dawsuUi.bridge.keysoundRoll?.notes.length === 1, null, "a double-click on the roll placed no note");
    await until(() => !document.querySelector(".ks-empty"), null, "the empty-track hint stays after a note is placed");
    // The colour on the canvas where the note should be, from its time on the editor's own axis.
    const noteIsDrawn = () => {
      const roll = window.dawsuUi.bridge.keysoundRoll;
      const scene = window.dawsuUi.bridge.scene;
      const note = roll?.notes[0];
      const canvas = document.getElementById("grid");
      if (!note || !canvas.clientWidth) return false;
      const scale = canvas.width / canvas.clientWidth;
      // On the note's top edge, near its right end, clear of the note name.
      const x = ((note.endMs - scene.viewStart) * scene.zoom - 3) * scale;
      const y = (roll.top + roll.pitches.indexOf(note.pitch) * roll.rowHeight - roll.scrollY + 0.5) * scale;
      const [red, green, blue] = canvas.getContext("2d").getImageData(Math.round(x), Math.floor(y), 1, 1).data;
      // The selected note's white ring, not a dark lane.
      return red > 200 && green > 200 && blue > 200;
    };
    await until(noteIsDrawn, null, "the placed note is not drawn at its time on the lanes' canvas");

    // The plain wheel scrolls the roll's pitch rows and nothing else.
    const before = await read(() => ({
      scrollY: window.dawsuUi.bridge.keysoundRoll.scrollY, start: window.dawsuUi.bridge.scene.viewStart,
      position: window.dawsuUi.bridge.currentPositionMs(),
    }));
    await page().mouse.move(shown.surface.x + 300, shown.surface.y + 60);
    await page().mouse.wheel(0, before.scrollY > 0 ? -28 : 28);
    await until(y => window.dawsuUi.bridge.keysoundRoll.scrollY !== y, before.scrollY, "the wheel did not scroll the roll's pitch rows");
    const after = await read(() => ({
      start: window.dawsuUi.bridge.scene.viewStart, position: window.dawsuUi.bridge.currentPositionMs(),
      lanesTop: document.getElementById("scroll").scrollTop,
    }));
    expect(after.start === before.start && after.position === before.position && after.lanesTop === lanesTop,
      `the plain wheel over the roll also moved the editor: ${JSON.stringify([before, after])}`);
    await until(noteIsDrawn, null, "the note is not drawn at its row after the pitch scroll");

    // Ctrl+wheel over the roll is still the editor's time zoom, and leaves the roll's rows alone.
    const rows = await read(() => ({ zoom: window.dawsuUi.bridge.scene.zoom, rowHeight: window.dawsuUi.bridge.keysoundRoll.rowHeight }));
    for (let turn = 0; turn < 4; turn++) {
      await page().keyboard.down("Control");
      await page().mouse.wheel(0, -240);
      await page().keyboard.up("Control");
      if (await until(zoom => window.dawsuUi.bridge.scene.zoom > zoom, rows.zoom, "", 1500).then(() => true, () => false)) break;
    }
    const zoomed = await read(() => ({ zoom: window.dawsuUi.bridge.scene.zoom, rowHeight: window.dawsuUi.bridge.keysoundRoll.rowHeight }));
    expect(zoomed.zoom > rows.zoom, "Ctrl+wheel over the roll did not zoom the time axis");
    expect(zoomed.rowHeight === rows.rowHeight, "Ctrl+wheel over the roll changed the roll's row height");
    await until(noteIsDrawn, null, "the note is not drawn at its time after Ctrl+wheel");

    // Zooming the time axis from the ruler moves the note with the ruler.
    await page().click("#ruler-zoom-in");
    await until(zoom => window.dawsuUi.bridge.scene.zoom > zoom, await read(() => window.dawsuUi.bridge.scene.zoom) - 1e-9,
      "Zoom in did not zoom in").catch(() => {});
    await until(noteIsDrawn, null, "the note is not drawn at its time after a zoom");
    await page().click("#ruler-fit");
    await until(noteIsDrawn, null, "the note is not drawn at its time after Fit all");

    await page().keyboard.press("Control+z");
    await until(() => window.dawsuUi.bridge.keysoundRoll?.notes.length === 0, null, "Ctrl+Z did not undo the note");

    // The mode's chrome: its track in the lane column, its section in the Inspector with Generate
    // pinned at the Inspector's bottom edge, and its three pills in the Audition well's place.
    const chrome = await read(() => {
      const rect = element => element?.getBoundingClientRect().toJSON() ?? null;
      return {
        tracks: document.querySelectorAll(".ks-track").length,
        active: !!document.querySelector(".ks-track.active"),
        pills: Array.from(document.querySelectorAll('[data-ui-mount="audition"] button'), button => button.getAttribute("aria-label")),
        section: !!document.getElementById("inspector-keysounds-body"),
        laneHidden: document.querySelector('[data-ui-mount="inspector-lane"] > .insp')?.hidden === true,
        foot: rect(document.querySelector(".insp-ks-foot")), inspector: rect(document.getElementById("editor-inspector")),
      };
    });
    expect(chrome.tracks === 1 && chrome.active, `the track list does not show the one active track: ${JSON.stringify(chrome)}`);
    expect(chrome.pills.join() === "Ghosts,Object lines,Typing", `the toolbar pills in Keysounds mode are ${chrome.pills.join()}`);
    expect(chrome.section && chrome.laneHidden, "the Inspector does not show the Keysounds section in place of Selection and Lane");
    if (chrome.inspector?.width > 0 && chrome.inspector.height > 0) {
      expect(chrome.foot && Math.abs(chrome.foot.bottom - chrome.inspector.bottom) < 2,
        `Generate is not pinned to the Inspector's bottom edge: ${JSON.stringify([chrome.foot, chrome.inspector])}`);
    }

    // A double-click on the track's name renames it in place.
    await page().locator(".ks-track-main").first().dblclick();
    await page().locator(".ks-track-rename").fill("Lead");
    await page().keyboard.press("Enter");
    await until(() => document.querySelector(".ks-track-name")?.textContent === "Lead", null,
      "a double-click on the track name did not rename the track");
    // A right-click opens the row's menu at the pointer.
    const row = await read(() => document.querySelector(".ks-track").getBoundingClientRect().toJSON());
    await page().mouse.click(row.x + 60, row.y + 20, { button: "right" });
    await until(() => !!document.querySelector(".ks-track-menu"), null, "a right-click on a track opened no menu");
    const menuAt = await read(() => document.querySelector(".ks-track-menu").getBoundingClientRect().toJSON());
    expect(Math.abs(menuAt.x - (row.x + 60)) < 3 && Math.abs(menuAt.y - (row.y + 20)) < 3,
      `the track menu is not at the pointer: ${JSON.stringify([menuAt, row])}`);
    await page().keyboard.press("Escape");
    await until(() => !document.querySelector(".ks-track-menu"), null, "Escape did not close the track menu");
    // The menu sits on the page itself, outside the list: its rows still act on the track.
    await page().mouse.click(row.x + 60, row.y + 20, { button: "right" });
    await page().locator(".ks-track-menu button").filter({ hasText: "Rename" }).click();
    await page().locator(".ks-track-rename").fill("Lead 2");
    await page().keyboard.press("Enter");
    await until(() => document.querySelector(".ks-track-name")?.textContent === "Lead 2" && !document.querySelector(".ks-track-menu"),
      null, "Rename in the track menu did not rename the track");

    // The menus drop over the roll: no File row may be covered by it (Save keysound project is one).
    await openMenu("File");
    const covered = await read(() => Array.from(document.querySelectorAll("#editor-menu button")).filter(button => {
      const box = button.getBoundingClientRect();
      const top = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
      return top !== button && !button.contains(top);
    }).map(button => button.textContent.trim()));
    await page().keyboard.press("Escape");
    expect(!covered.length, `the roll covers File menu rows in Keysounds mode: ${covered.join(", ")}`);

    // A short drag on the Keysounds section's head must not reorder the Inspector's sections.
    if (chrome.inspector?.width > 0 && chrome.inspector.height > 0) {
      const sections = () => read(() => Array.from(document.querySelectorAll("#editor-inspector > section.inspector-part"),
        section => section.getAttribute("aria-label")).join());
      const order = await sections();
      const head = await read(() => document.querySelector(".insp-ks .insp-head").getBoundingClientRect().toJSON());
      await page().mouse.move(head.x + 40, head.y + head.height / 2);
      await page().mouse.down();
      for (let dy = 3; dy <= 15; dy += 3) await page().mouse.move(head.x + 40, head.y + head.height / 2 + dy);
      const dragged = await sections();
      await page().mouse.up();
      expect(dragged === order && await sections() === order,
        `a short drag on the Keysounds section head reordered the Inspector: ${order} → ${dragged}`);
    }

    // Edit › Undo follows the keysound history here: it takes the note back and leaves the hits alone.
    await page().mouse.dblclick(shown.surface.x + 200, shown.surface.y + 40);
    await until(() => window.dawsuUi.bridge.keysoundRoll?.notes.length === 1, null, "a second double-click on the roll placed no note");
    await fromMenu("Edit", "Undo");
    await until(() => window.dawsuUi.bridge.keysoundRoll?.notes.length === 0, null, "Edit › Undo did not undo the keysound note");
    expect((await hits()).length === hitsBefore, "Edit › Undo in Keysounds mode changed the hits");

    // Draw, picked by its own key in this preset, places a note on the snap step, one step long;
    // Erase then removes it (plan task 5.3).
    const drawKey = await read(() => window.dawsuUi.bridge.callAction("toolKey", "Draw"));
    expect(!!drawKey, "no key picks the Draw tool");
    await page().mouse.move(shown.surface.x + 230, shown.surface.y + 70);
    await page().keyboard.press(drawKey);
    await until(() => window.view.tool === "Draw" && window.dawsuUi.bridge.view?.tool === "Draw", null,
      `the ${drawKey} key did not pick Draw in Keysounds mode`);
    await page().mouse.click(shown.surface.x + 230, shown.surface.y + 70);
    await until(() => window.dawsuUi.bridge.keysoundRoll?.notes.length === 1, null, "a Draw click placed no note");
    const drawn = await read(() => {
      const scene = window.scene;
      const note = window.dawsuUi.bridge.keysoundRoll.notes[0];
      const [origin, beat] = scene.sections.filter(section => section[0] <= note.startMs).pop() ?? scene.sections[0];
      return { note, snap: scene.snap, mode: scene.snapMode, origin, beat,
        step: beat * Math.max(1, scene.beats || 1) / Math.max(1, scene.divisor || 4) };
    });
    if (drawn.snap && drawn.mode !== "objects") {
      const offset = (drawn.note.startMs - drawn.origin) / drawn.step;
      expect(Math.abs(offset - Math.round(offset)) * drawn.step <= 1 &&
        Math.abs(drawn.note.endMs - drawn.note.startMs - drawn.step) <= 1,
        `the Draw click did not land on a ${drawn.step} ms snap step: ${JSON.stringify(drawn)}`);
    } else {
      expect(Math.abs(drawn.note.endMs - drawn.note.startMs - drawn.beat) <= 1,
        `the Draw click with Snap off or on objects is not one beat long: ${JSON.stringify(drawn)}`);
    }
    await pickTool("Erase");
    const middle = await read(() => {
      const roll = window.dawsuUi.bridge.keysoundRoll;
      const note = roll.notes[0];
      return { x: (note.startMs - window.scene.viewStart) * window.scene.zoom + 3,
        y: roll.top + roll.pitches.indexOf(note.pitch) * roll.rowHeight - roll.scrollY + roll.rowHeight / 2 };
    });
    await page().mouse.click(shown.grid.x + middle.x, shown.grid.y + middle.y);
    await until(() => window.dawsuUi.bridge.keysoundRoll?.notes.length === 0, null, "an Erase click did not remove the drawn note");
    await pickTool("Select");
    expect((await hits()).length === hitsBefore, "Draw and Erase in Keysounds mode changed the hits");

    await page().click("#keysounds-view");
    await until(() => window.dawsuUi.bridge.mode !== "keysounds" && !document.querySelector(".ks-workspace"), null,
      "the mode switch did not leave Keysounds mode");
    expect((await hits()).length === hitsBefore, "Keysounds mode changed the hits");
    expect(await read(() => document.getElementById("scroll").scrollTop) === lanesTop, "the lanes' scroll position changed");
    return chrome.inspector?.width > 0 && chrome.inspector.height > 0
      ? "Generate pinned at the Inspector's bottom edge" : "Inspector not shown, footer not measured";
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
    writeFileSync(resolve(args.json), JSON.stringify({ exe, map: original, profile, passed, total: results.length, seconds, steps: results }, null, 2) + "\n");
  }
  if (failed || !results.length) process.exitCode = 1;
  if (practiceFolder && basename(dirname(practiceFolder)).toLowerCase() === "dawsu-practice") {
    const practiceRoot = dirname(practiceFolder);
    const sessionName = basename(practiceFolder);
    for (const dir of [practiceFolder, join(practiceRoot, "projects", sessionName), join(practiceRoot, "backups", sessionName)]) {
      try {
        rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 150 });
      } catch {}
    }
  }
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
