// The robot mapper's shared harness: start the real built dawsu! editor on a copied map, act through
// real input, report one PASS, FAIL or SKIP line per step, and read what changed on disk.
//
// tools/everyday-check.mjs (the merge gate) keeps its own copy of these helpers on purpose, so this
// longer robot can change without touching the gate. The launch, step and wait logic below is the
// gate's, unchanged, so a step behaves the same in both.
//
// Windows only in practice (WebView2 remote debugging), written to stay portable.
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { join, relative, resolve } from "node:path";

export const sleep = ms => new Promise(done => setTimeout(done, ms));

/**
 * A path as one comparable string: the editor canonicalises paths (Windows gives \\?\E:\... and
 * follows the target/ junction), so both sides go through the real path, without the \\?\ prefix.
 */
export function samePath(a, b) {
  const norm = path => {
    const plain = String(path).replace(/^\\\\\?\\/, "");
    let real = resolve(plain);
    try { real = realpathSync.native(real); } catch {}
    return real.replace(/^\\\\\?\\/, "").toLowerCase();
  };
  return norm(a) === norm(b);
}

/** argv as { flag: value | true }. */
export function parseArgs(argv = process.argv.slice(2)) {
  const args = {};
  for (let i = 0; i < argv.length; i++) {
    const value = argv[i + 1];
    const flag = value === undefined || value.startsWith("--");
    args[argv[i].replace(/^--/, "")] = flag ? true : value;
    if (!flag) i++;
  }
  return args;
}

export function playwrightCore() {
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
    for (const entry of readdirSync(gDir)) candidates.push(join(gDir, entry, "node_modules", "playwright-core"));
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

/** Every file under a folder, by its path inside it, with its bytes' hash. */
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

/** What changed between two snapshots. */
export function diffSnapshots(before, after) {
  return {
    added: [...after.keys()].filter(name => !before.has(name)),
    removed: [...before.keys()].filter(name => !after.has(name)),
    changed: [...after.keys()].filter(name => before.has(name) && before.get(name) !== after.get(name)),
  };
}

/**
 * One robot: an editor process, its pages, the errors they report and the step results.
 * profile: the run's own folder (LOCALAPPDATA, WebView2 profile, shots).
 */
export function createRobot({ exe, profile, shots = join(profile, "shots"), chromium = playwrightCore() }) {
  mkdirSync(shots, { recursive: true });
  let app = null;
  let current = null;
  const errors = [];
  const results = [];
  let errorsChecked = 0;
  let stopReason = null;

  async function launch(map, env = {}) {
    const port = await freePort();
    const child = spawn(exe, map ? [map] : [], {
      env: { ...process.env, LOCALAPPDATA: profile, WEBVIEW2_USER_DATA_FOLDER: join(profile, "webview2"),
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`, ...env },
      stdio: "ignore",
    });
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
    context.setDefaultTimeout(5000);
    const listen = page => {
      const where = () => page.url().split("/").pop() || "page";
      page.on("console", message => {
        if (message.type() === "error") errors.push({ step: current, text: `${where()}: console error: ${message.text()}` });
      });
      const origin = error => (error.stack ?? "").split("\n").find(line => /^\s+at /.test(line))?.trim().replace(/https?:\/\/[^/]+\//g, "") ?? "";
      page.on("pageerror", error => errors.push({ step: current, text: `${where()}: page error: ${error.message}${origin(error) ? ` (${origin(error)})` : ""}` }));
    };
    context.on("page", listen);
    await context.addInitScript(() => {
      try { localStorage.setItem("dawsu.quickStartDone", "1"); } catch {}
      window.addEventListener("unhandledrejection", event =>
        console.error("unhandled promise rejection: " + (event.reason?.stack ?? event.reason)));
    });
    const editorPage = () => context.pages().find(open => !open.isClosed() && /^https?:\/\/tauri\.localhost\//.test(open.url()));
    let page = null;
    for (let i = 0; i < 240 && !page; i++) {
      page = editorPage() ?? null;
      if (!page) await sleep(250);
    }
    if (!page) throw new Error(`the editor never loaded its page: ${JSON.stringify(context.pages().map(open => open.url()))}`);
    app.page = page;
    for (const open of context.pages()) listen(open);
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

  async function tidy() {
    for (let i = 0; i < 3; i++) {
      const open = await read(() =>
        document.getElementById("dialog")?.open
        || Array.from(document.querySelectorAll('[role="dialog"]')).some(el => !el.hidden && getComputedStyle(el).display !== "none")
        || !document.getElementById("editor-menu")?.hidden).catch(() => false);
      if (!open) return;
      await page().keyboard.press("Escape");
      await sleep(250);
    }
  }

  /**
   * One step. body returns nothing, a note string, or { skip: reason }. covers: feature ids it
   * checks (tools/robot/features.mjs). stops: when it throws, the rest of the run is skipped.
   */
  async function step(name, body, { covers = [], always = false, stops = false } = {}) {
    const number = String(results.length + 1).padStart(2, "0");
    if (stopReason && !always) {
      results.push({ number, name, covers, status: "SKIP", detail: stopReason });
      console.log(`SKIP ${number} ${name} — ${stopReason}`);
      return results.at(-1);
    }
    current = number;
    const result = { number, name, covers, status: "PASS", detail: "" };
    let threw = false;
    try {
      const note = await body();
      if (typeof note === "string") result.detail = note;
      if (note?.skip) { result.status = "SKIP"; result.detail = note.skip; }
    } catch (error) {
      threw = true;
      result.status = "FAIL";
      result.detail = error.message;
    }
    await sleep(150);
    const raised = errors.slice(errorsChecked).filter(error => error.step === number || error.step === null);
    errorsChecked = errors.length;
    if (raised.length) {
      result.status = "FAIL";
      const counts = new Map();
      for (const { text } of raised) counts.set(text, (counts.get(text) ?? 0) + 1);
      result.errors = [...counts].map(([text, count]) => count > 1 ? `${text} (×${count})` : text);
      result.detail = [result.detail, ...result.errors].filter(Boolean).join(" | ");
    }
    result.detail = String(result.detail).replace(/\s*\n\s*/g, " ");
    if (result.status === "FAIL" && app?.context) {
      const path = join(shots, `${number}-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/-$/, "").slice(0, 80)}.png`);
      const target = app.context.pages().filter(open => !open.isClosed()).at(-1) ?? app.page;
      if (await target.screenshot({ path }).then(() => true, () => false)) result.screenshot = path;
    }
    results.push(result);
    if (threw && stops) stopReason = `step ${number} failed, so the steps that need it cannot run`;
    console.log(`${result.status} ${number} ${name}${result.detail ? " — " + result.detail : ""}${result.screenshot ? " (" + result.screenshot + ")" : ""}`);
    current = null;
    if (result.status === "FAIL" && app?.page) await tidy().catch(() => {});
    return result;
  }

  const expect = (ok, message) => { if (!ok) throw new Error(message); };

  // The scene, read through the page's read-only globals.
  const hits = () => read(() => window.scene.moments.flatMap(([time, volume, , list]) =>
    list.map(([row, selected, muted, hitVolume]) => ({ time, row, selected: !!selected, muted: !!muted, volume: hitVolume ?? volume,
      lane: window.scene.lanes[row]?.name, family: window.scene.lanes[row]?.family ?? null, index: window.scene.lanes[row]?.index ?? null }))));
  // Click an empty spot of the menu bar row, so keys go to the editor and not to a focused field
  // (the editor ignores shortcuts typed into inputs); the gate does the same.
  const focusEditor = async () => { await page().mouse.click(400, 10); await sleep(100); };
  const hitCount = () => read(() => window.scene.moments.reduce((sum, moment) => sum + moment[3].length, 0));
  const namedHits = () => read(() => window.scene.moments.flatMap(([time, , , list]) =>
    list.map(([row]) => `${time} ms ${window.scene.lanes[row]?.name ?? "row " + row}#${window.scene.lanes[row]?.index ?? "f"}`)).sort());
  // A lane's row by name and, when given, custom index (the skin lane and the numberless file share a name).
  const laneRow = (name, index) => read(([name, index]) => window.scene.lanes.findIndex(lane =>
    lane.name === name && (index === undefined || index === null || lane.index === index)), [name, index ?? null]);

  /** Show `time` on the timeline with `zoom` px per ms (a view change, not an edit). */
  async function showTime(time, { zoom, lead = 200 } = {}) {
    await read(([time, zoom, lead]) => window.act({ kind: "view", start: Math.max(0, time - lead),
      zoom: zoom ?? window.scene.zoom, row_height: window.scene.rowHeight }), [time, zoom ?? null, lead]);
    await until(time => {
      const grid = document.getElementById("grid").getBoundingClientRect();
      const x = (time - window.scene.viewStart) * window.scene.zoom;
      return x > 0 && x < grid.width - 20;
    }, time, `the timeline did not scroll to ${time} ms`, 4000);
  }
  /** Scroll the lane list so a row's header is on screen. */
  async function showRow(row) {
    await read(row => document.querySelector(`#lanes [data-lane-row="${row}"]`)?.scrollIntoView({ block: "center" }), row);
    await sleep(120);
  }
  /** Where a time and a lane are on the timeline canvas, in page pixels. */
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
  /** Run a Ctrl+K command palette entry by its label. */
  async function palette(label) {
    await page().mouse.move(5, 5);
    await page().keyboard.press("Control+K");
    const input = page().locator(".cmdp-input");
    await input.waitFor({ state: "visible", timeout: 4000 });
    await input.fill(label);
    await sleep(150);
    const row = page().locator('[aria-label="Command palette"] [role="option"]').filter({ hasText: label }).first();
    if (await row.count()) await row.click();
    else await page().keyboard.press("Enter");
    await until(() => !document.querySelector('[aria-label="Command palette"]'), null, `the palette stayed open after "${label}"`, 4000);
  }
  async function pickTool(name) {
    const pill = page().locator('[aria-label="Editing tools"]').getByRole("button", { name, exact: true });
    if (["Select", "Draw", "Erase"].includes(name)) await pill.click({ timeout: 5000 });
    // The other four have no pill of their own; the gate picks them the same way.
    else await read(tool => window.act({ kind: "tool", name: tool }, false), name);
    await until(name => window.view.tool === name, name, `the tool stayed ${await read(() => window.view.tool)}, not ${name}`, 4000);
  }
  /** Set the snap to 1/n with the snap slider's stops (the slider's own clicks). */
  async function setDivisor(n) {
    if (await read(n => window.scene.divisor === n && window.scene.beats === 1, n)) return;
    const stops = [1, 2, 3, 4, 6, 8, 12, 16];
    const k = stops.indexOf(n);
    expect(k >= 0, `1/${n} is not a snap stop`);
    const tick = await read(k => document.querySelectorAll('[role="slider"][aria-label="Snap"] .tick')[k]?.getBoundingClientRect().toJSON(), k);
    expect(tick, "the snap slider has no stops");
    await page().mouse.click(tick.x + tick.width / 2, tick.y + tick.height / 2);
    await until(n => window.scene.divisor === n && window.scene.beats === 1, n, `the snap did not become 1/${n}`, 4000);
  }
  const dialogTitle = () => read(() => document.getElementById("dialog").open ? document.getElementById("dialog-title").textContent : null);
  async function dialogShown(title) {
    await until(title => document.getElementById("dialog").open &&
      document.getElementById("dialog-title").textContent.startsWith(title), title,
      `no "${title}" dialog opened (open: ${JSON.stringify(await dialogTitle())})`, 8000);
  }
  const dialogButton = text => page().locator("#dialog-actions button", { hasText: text }).first();
  /**
   * Answer the dialogs an output action shows (save review, output review, problems) with their
   * go-ahead button until none is open or `done` holds. Returns the titles seen. Any other dialog fails.
   */
  async function answerDialogs(done, { timeout = 30000, accept = /^(Write files|Save|Done|Continue to Save|Apply|OK|Revert)$/ } = {}) {
    const seen = [];
    const end = Date.now() + timeout;
    while (Date.now() < end) {
      if (await read(done).catch(() => false) && !(await dialogTitle())) return seen;
      const title = await dialogTitle();
      if (title) {
        const buttons = await read(() => Array.from(document.querySelectorAll("#dialog-actions button"), b => b.textContent.trim()));
        const go = buttons.find(text => accept.test(text));
        if (!go) {
          const body = (await read(() => document.getElementById("dialog-body").innerText)).slice(0, 200);
          throw new Error(`"${title}" offered ${JSON.stringify(buttons)}: ${body}`);
        }
        seen.push(title);
        await dialogButton(go).click();
      }
      await sleep(250);
    }
    throw new Error(`the action did not finish (dialogs seen: ${seen.join(", ") || "none"})`);
  }

  async function save() {
    await page().click("#save");
    const seen = await answerDialogs(() => !window.view.dirty && !window.view.busy);
    expect(!(await read(() => window.view.dirty)), "the editor still shows unsaved changes after Save");
    return seen;
  }

  function finish(jsonPath) {
    const failed = results.filter(result => result.status === "FAIL").length;
    const passed = results.filter(result => result.status === "PASS").length;
    if (jsonPath) writeFileSync(jsonPath, JSON.stringify({ results }, null, 2));
    return { failed, passed, total: results.length };
  }

  return {
    launch, quit, step, expect, until, read, page: () => page(), app: () => app, results, errors, tidy,
    hits, hitCount, namedHits, focusEditor, laneRow, showTime, showRow, at, openMenu, fromMenu, menuRow, palette, pickTool,
    setDivisor, dialogTitle, dialogShown, dialogButton, answerDialogs, save, finish,
  };
}

/**
 * Refuse a binary older than the latest code commit or the UI build stamp (as the gate does), unless
 * allowStale. Prints the provenance lines either way.
 */
export function checkFreshness(repo, exe, allowStale = false) {
  const CODE_PATHS = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates", "apps/tauri/Cargo.toml", "apps/tauri/build.rs",
    "apps/tauri/tauri.conf.json", "apps/tauri/capabilities", "apps/tauri/icons", "apps/tauri/src", "apps/tauri/ui/src",
    "apps/tauri/ui/package.json", "apps/tauri/ui/package-lock.json", "apps/tauri/ui/vite.config.js"];
  const log = spawnSync("git", ["log", "-1", "--format=%h %ct", "--", ...CODE_PATHS], { cwd: repo, encoding: "utf8" });
  const [short, epoch] = (log.stdout ?? "").trim().split(" ");
  const codeMs = Number(epoch) * 1000;
  const exeMs = statSync(exe).mtimeMs;
  const stamp = join(repo, "apps", "tauri", "dist", "ui", "build-stamp.json");
  const reasons = [];
  if (Number.isFinite(codeMs) && exeMs < codeMs) reasons.push(`binary is older than code commit ${short}`);
  if (!existsSync(stamp)) reasons.push("apps/tauri/dist/ui/build-stamp.json is missing");
  else if (exeMs < statSync(stamp).mtimeMs) reasons.push("binary is older than apps/tauri/dist/ui/build-stamp.json");
  console.log(`provenance exe: ${exe} (mtime ${new Date(exeMs).toISOString()}), code commit ${short ?? "unknown"}`);
  if (reasons.length) {
    console.log(`provenance freshness: STALE — ${reasons.join("; ")}${allowStale ? " (continuing: --allow-stale)" : ""}`);
    if (!allowStale) {
      console.error("Rebuild first (npm run build in apps/tauri/ui, then cargo build -p dawsu-tauri --locked), or pass --allow-stale.");
      process.exit(1);
    }
  } else console.log("provenance freshness: FRESH");
}
