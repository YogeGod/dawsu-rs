// The robot mapper's shared parts: start the real, built dawsu! editor on a copy of a map, act on
// it through real input, and record each step as PASS, FAIL or SKIP.
//
// tools/everyday-check.mjs (the one-minute merge gate) and tools/every-input.mjs (the long run that
// presses every input) are built on this. Nothing here calls the page's act(): the steps act through
// the mouse, the keyboard and the windows' own controls, and only read the page to check the result.
//
// Windows only in practice (WebView2 remote debugging), written to stay portable. Playwright Core is
// found in DAWSU_PLAYWRIGHT_CORE or the npm cache, as tools/ui-screenshot.mjs does; nothing is
// installed.
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { join, relative, resolve } from "node:path";

export function parseArgs(argv = process.argv) {
  const args = {};
  for (let i = 2; i < argv.length; i++) {
    const value = argv[i + 1];
    const flag = value === undefined || value.startsWith("--");
    args[argv[i].replace(/^--/, "")] = flag ? true : value;
    if (!flag) i++;
  }
  return args;
}

// Refuse a binary older than HEAD or the UI build stamp, unless --allow-stale.
export function checkProvenance(repo, exe, allowStale, tool = "the robot mapper") {
  const headShortRes = spawnSync("git", ["rev-parse", "--short", "HEAD"], { cwd: repo, encoding: "utf8" });
  const headShort = headShortRes.status === 0 && headShortRes.stdout.trim() ? headShortRes.stdout.trim() : "unknown";
  const headTimeRes = spawnSync("git", ["log", "-1", "--format=%ct"], { cwd: repo, encoding: "utf8" });
  const headEpochSec = headTimeRes.status === 0 ? Number.parseInt(headTimeRes.stdout.trim(), 10) : Number.NaN;
  const headMtimeMs = Number.isFinite(headEpochSec) ? headEpochSec * 1000 : null;
  const headIso = headMtimeMs !== null ? new Date(headMtimeMs).toISOString() : "unknown";

  const exeMtimeMs = statSync(exe).mtimeMs;
  const exeMtimeIso = new Date(exeMtimeMs).toISOString();

  const buildStampPath = join(repo, "apps", "tauri", "dist", "ui", "build-stamp.json");
  const hasBuildStamp = existsSync(buildStampPath);
  const stampMtimeMs = hasBuildStamp ? statSync(buildStampPath).mtimeMs : null;
  const stampIso = stampMtimeMs !== null ? new Date(stampMtimeMs).toISOString() : "missing";

  const staleReasons = [];
  if (headMtimeMs !== null && exeMtimeMs < headMtimeMs) {
    staleReasons.push(`binary is older than latest commit ${headShort} (${headIso})`);
  }
  if (!hasBuildStamp) {
    staleReasons.push("apps/tauri/dist/ui/build-stamp.json is missing");
  } else if (exeMtimeMs < stampMtimeMs) {
    staleReasons.push(`binary is older than apps/tauri/dist/ui/build-stamp.json (${stampIso})`);
  }
  const isStale = staleReasons.length > 0;

  console.log(`provenance repo: ${repo} (HEAD ${headShort})`);
  console.log(`provenance exe:  ${exe} (mtime ${exeMtimeIso})`);
  if (!isStale) {
    console.log(`provenance freshness: FRESH (newer than HEAD ${headShort} [${headIso}] and build-stamp.json [${stampIso}])`);
  } else {
    const verdict = allowStale ? "STALE (continuing because --allow-stale was passed)" : "STALE";
    console.log(`provenance freshness: ${verdict} — ${staleReasons.join("; ")}`);
    if (!allowStale) {
      console.error(
        `Refusing to run ${tool} on a stale build: ${staleReasons.join("; ")}.\n` +
        `Rebuild the UI (npm run build in apps/tauri/ui) and the app binary (cargo build -p dawsu-tauri --locked) first, ` +
        `or pass --allow-stale to bypass this check.`
      );
      process.exit(1);
    }
  }
  return { headShort };
}

export function playwrightCore() {
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
export const sleep = ms => new Promise(done => setTimeout(done, ms));

// Every file under a folder, by its path inside it, with its bytes' hash.
export function snapshot(folder) {
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

// A short silent WAV (16-bit mono, 44.1 kHz, 0.1 s), as a mapper drops a new sample into a mapset.
export function silentWav(path, seconds = 0.1) {
  const samples = Math.round(44100 * seconds);
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
  writeFileSync(path, wav);
}

// The keys a menu row or the shortcut list shows, as Playwright names them ("Ctrl+Shift+Z" → "Control+Shift+Z").
const keyNames = { Ctrl: "Control", Del: "Delete", Esc: "Escape", Cmd: "Meta", Space: " ", Return: "Enter",
  PgUp: "PageUp", PgDn: "PageDown", Ins: "Insert", Up: "ArrowUp", Down: "ArrowDown", Left: "ArrowLeft", Right: "ArrowRight" };
export const playwrightKey = keys => keys.split("+").map(key => keyNames[key] ?? key).join("+");

// One robot: a profile folder, one running editor at a time and the steps' results.
//   repo        the dawsu-rs checkout (the profile goes under target/<name>/)
//   exe         the built editor
//   name        the profile folder's name under target/ ("everyday-check", "every-input")
//   shots       where a failing step's screenshot goes (default <profile>/shots)
//   opensMap    the step names that open the map: when one throws, the steps after it are skipped
export function createRobot({ repo, exe, name, shots: shotsArg, opensMap = [] }) {
  const chromium = playwrightCore();
  const stamp = new Date().toISOString().replace(/[-:]/g, "").replace(/\..*/, "");
  const profile = resolve(join(repo, "target", name, `${stamp}-${process.pid}`));
  const shots = resolve(shotsArg ?? join(profile, "shots"));
  mkdirSync(profile, { recursive: true });
  mkdirSync(shots, { recursive: true });

  let app = null;
  let current = null;
  const errors = [];
  const results = [];
  let errorsChecked = 0;
  let noMap = null;
  const opens = new Set(opensMap);

  async function launch(map, extraArgs = []) {
    const port = await freePort();
    // TEMP and TMP point into the profile too: the editor looks there for osu!lazer's "Edit
    // externally" folders, which are real maps.
    const temp = join(profile, "temp");
    mkdirSync(temp, { recursive: true });
    const child = spawn(exe, [...(map ? [map] : []), ...extraArgs], {
      // WebView2 finds its profile without LOCALAPPDATA; name it, or a running dawsu! shares its browser.
      env: { ...process.env, LOCALAPPDATA: profile, TEMP: temp, TMP: temp, WEBVIEW2_USER_DATA_FOLDER: join(profile, "webview2"),
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
      const open = await read(() => document.getElementById("dialog")?.open || !document.getElementById("editor-menu")?.hidden).catch(() => false);
      if (!open) return;
      await page().keyboard.press("Escape");
      await sleep(250);
    }
  }

  // step(name, body, always) or step(name, body, { always, covers }). `covers` names the inventory
  // entries (tools/robot-inputs.mjs) the step exercises; it is kept in the results.
  async function step(name, body, options = false) {
    const { always = false, covers = [] } = typeof options === "object" ? options : { always: options };
    const number = String(results.length + 1).padStart(2, "0");
    if (noMap && !always) {
      results.push({ number, name, status: "SKIP", detail: noMap, covers });
      console.log(`SKIP ${number} ${name} — ${noMap}`);
      return;
    }
    current = number;
    const result = { number, name, status: "PASS", detail: "", covers };
    let threw = false;
    try {
      const note = await body(result);
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
      const path = join(shots, `${number}-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/-$/, "").slice(0, 80)}.png`);
      const target = app.context.pages().filter(page => !page.isClosed()).at(-1) ?? app.page;
      if (await target.screenshot({ path }).then(() => true, () => false)) result.screenshot = path;
    }
    results.push(result);
    // Only a map that never showed stops the rest; errors while it opened fail this step alone.
    if (threw && opens.has(name)) noMap = `the map did not open in step ${number}`;
    console.log(`${result.status} ${number} ${name}${result.detail ? " — " + result.detail : ""}${result.screenshot ? " (" + result.screenshot + ")" : ""}`);
    current = null;
    // A dialog or menu a failed step left open would block the next one.
    if (result.status === "FAIL" && app?.page) await tidy().catch(() => {});
  }

  // Every hit in the open difficulty: [time, lane row, selected, volume].
  const hits = () => read(() => window.scene.moments.flatMap(([time, volume, , list]) =>
    list.map(([row, selected, , hitVolume]) => [time, row, !!selected, hitVolume ?? volume])));
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
    // Select, Draw and Erase have pills; the other tools are in the More tools menu by the pills.
    const pill = page().locator('[aria-label="Editing tools"]').getByRole("button", { name, exact: true });
    if (["Select", "Draw", "Erase"].includes(name)) await pill.click({ timeout: 5000 });
    else {
      const more = page().locator("[data-more-tools]").first();
      if (await more.count()) {
        await more.click();
        await until(() => !document.getElementById("editor-menu").hidden, null, "the More tools menu did not open", 3000);
        await menuRow(name).click();
      } else {
        throw new Error(`no pill and no More tools button for ${name}`);
      }
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
  async function windowOpened(name, timeout = 15000) {
    const end = Date.now() + timeout;
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

  // Print the totals, write --json, tidy the profile. Returns the number of failed steps.
  async function finish({ started, json, keepProfile, extra = {}, label = "robot mapper" }) {
    await quit();
    const passed = results.filter(result => result.status === "PASS").length;
    const failed = results.filter(result => result.status === "FAIL").length;
    const seconds = Math.round((Date.now() - started) / 1000);
    console.log(`run time: ${seconds} s`);
    console.log(`${label}: ${passed} of ${results.length} steps passed`);
    if (json) {
      writeFileSync(resolve(json), JSON.stringify({ exe, profile, passed, total: results.length, seconds, ...extra, steps: results }, null, 2) + "\n");
    }
    if (!keepProfile) {
      for (const entry of readdirSync(profile)) {
        const folder = join(profile, entry);
        if (folder === shots) continue;
        try {
          rmSync(folder, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
        } catch (error) {
          console.warn(`could not remove ${folder}: ${error.message}`);
        }
      }
      // The shots stay when a step failed; an empty profile folder goes.
      try {
        if (!shotsArg && !readdirSync(shots).length) rmSync(shots, { recursive: true, force: true });
        if (!readdirSync(profile).length) rmSync(profile, { recursive: true, force: true });
      } catch {}
    }
    return failed || !results.length ? 1 : 0;
  }

  return {
    profile, shots, results, errors,
    get app() { return app; },
    get noMap() { return noMap; },
    launch, quit, step, page, read, until, expect, tidy, hits, namedHits, at,
    menuRow, openMenu, fromMenu, pickTool, dialogTitle, dialogShown, dialogButton, dialogClosed,
    windowOpened, windowClosed, menuKeys, finish,
  };
}
