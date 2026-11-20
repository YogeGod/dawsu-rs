// Screenshot the dawsu! Tauri editor on a copy of a map, for before/after checks.
//
// node tools/ui-screenshot.mjs --exe <dawsu-tauri.exe> --map <difficulty.osu> --out <shot.png>
//   [--profile <folder>]  isolated LOCALAPPDATA and WebView2 profile (default: a new temp folder)
//   [--eval "<js>"]       run in the page before the shot, e.g. act({ kind: "view", start: 23300,
//                         zoom: scene.zoom, row_height: scene.rowHeight })
//   [--wait <ms>]         extra settle time before the shot (default 2500)
//   [--preset <name>]     key preset chosen at the first-launch question (default DAW)
//   [--timeout <ms>]      how long to wait for the map's lanes (default 60000)
//   [--csp]               reload with a listener from the start and fail on any content security
//                         policy violation ("Refused to ...") in any of the editor's windows
//   [--width <px>]        lay the editor's page out at this width (and --height <px>) for the shot
//   [--window <name>]     shoot another window (export, drum) instead: --eval should open it, for
//                         example `document.getElementById("export-entry").click()`
//   [--eval-window "<js>"] run in that window before its shot
//
// The value --eval returns is printed. The map's whole folder is copied into the profile first
// and the copy is opened, so the original mapset is never touched. Each run picks a free
// debugging port, so several runs can capture at once. Windows only (WebView2 remote debugging).
// Playwright Core is found in DAWSU_PLAYWRIGHT_CORE or the npm cache; nothing is installed.
import { spawn, spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";

const args = {};
for (let i = 2; i < process.argv.length; i++) {
  const value = process.argv[i + 1];
  const flag = value === undefined || value.startsWith("--");
  args[process.argv[i].replace(/^--/, "")] = flag ? true : value;
  if (!flag) i++;
}
for (const key of ["exe", "map", "out"]) if (!args[key]) throw new Error(`--${key} is required`);

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

const profile = resolve(args.profile ?? mkdtempSync(join(tmpdir(), "dawsu-shot-")));
const mapCopy = join(profile, "map");
cpSync(dirname(resolve(args.map)), mapCopy, { recursive: true });
const port = await freePort();
const chromium = playwrightCore();
const child = spawn(resolve(args.exe), [join(mapCopy, basename(args.map))], {
  env: { ...process.env, LOCALAPPDATA: profile,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
  stdio: "ignore",
});
const sleep = ms => new Promise(done => setTimeout(done, ms));
let browser;
try {
  for (let i = 0; i < 120 && !browser; i++) {
    await sleep(500);
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`).catch(() => null);
  }
  if (!browser) throw new Error("the editor's WebView2 never opened its debugging port");
  const page = browser.contexts()[0].pages()[0];
  const lanesShown = timeout => page.waitForFunction(() => document.querySelectorAll("#lanes .lane").length > 0,
    null, { timeout }).then(() => true, () => false);
  // A fresh profile first asks for a key preset and holds the map until one is chosen.
  let loaded = await lanesShown(8000);
  if (!loaded) {
    await page.evaluate(name => act({ kind: "key_preset", name }), args.preset ?? "DAW").catch(() => {});
    loaded = await lanesShown(Number(args.timeout ?? 60000));
  }
  const violations = [];
  if (args.csp && loaded) {
    const listen = target => target.on("console", message => {
      if (message.text().includes("Refused to")) violations.push(message.text());
    });
    listen(page);
    // Windows opened later (Export) get the same listener and init script.
    page.context().on("page", listen);
    await page.context().addInitScript(() => document.addEventListener("securitypolicyviolation", event =>
      console.error(`Refused to ${event.effectiveDirective}: ${event.blockedURI} at ${event.sourceFile}:${event.lineNumber}`)));
    await page.reload();
    loaded = await lanesShown(Number(args.timeout ?? 60000));
  }
  if (!loaded) console.log("no lanes appeared; page text:", (await page.evaluate(() => document.body.innerText)).slice(0, 400));
  if (args.width) await page.setViewportSize({ width: Number(args.width), height: Number(args.height ?? 700) });
  await sleep(Number(args.wait ?? 2500));
  if (args.eval) {
    const result = await page.evaluate(args.eval);
    if (result !== undefined) console.log("eval:", JSON.stringify(result));
    await sleep(1000);
  }
  let shot = page;
  if (args.window) {
    // Each Tauri window is its own WebView2 page; find it by the page it loads.
    const name = args.window + ".html";
    shot = null;
    for (let i = 0; i < 60 && !shot; i++) {
      shot = page.context().pages().find(candidate => candidate.url().endsWith("/" + name)) ?? null;
      if (!shot) await sleep(250);
    }
    if (!shot) throw new Error(`no ${name} window opened; does --eval open it?`);
    await shot.waitForSelector("[data-ui-ready]", { timeout: 15000 });
    await sleep(Number(args.wait ?? 2500));
    if (args["eval-window"]) {
      const result = await shot.evaluate(args["eval-window"]);
      if (result !== undefined) console.log("eval-window:", JSON.stringify(result));
      await sleep(1000);
    }
  }
  await shot.screenshot({ path: resolve(args.out) });
  console.log(`saved ${resolve(args.out)} (profile ${profile})`);
  if (args.csp) {
    console.log(`content security policy violations: ${violations.length}`);
    for (const text of violations) console.log("  " + text);
    if (violations.length || !loaded) process.exitCode = 1;
  }
} finally {
  await browser?.close().catch(() => {});
  child.kill();
}
