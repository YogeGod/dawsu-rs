// Screenshot the dawsu! Tauri editor on a copy of a map, for before/after checks.
//
// node tools/ui-screenshot.mjs --exe <dawsu-tauri.exe> --map <difficulty.osu> --out <shot.png>
//   [--profile <folder>]  isolated LOCALAPPDATA and WebView2 profile (default: a new temp folder)
//   [--eval "<js>"]       run in the page before the shot, e.g. act({ kind: "view", start: 23300,
//                         zoom: scene.zoom, row_height: scene.rowHeight })
//   [--wait <ms>]         extra settle time before the shot (default 2500)
//
// The map's whole folder is copied into the profile first and the copy is opened, so the
// original mapset is never touched. Each run picks a free debugging port, so several runs
// can capture at once. Windows only (WebView2 remote debugging). Playwright Core is found in
// DAWSU_PLAYWRIGHT_CORE or the npm cache; nothing is installed.
import { spawn, spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";

const args = {};
for (let i = 2; i < process.argv.length; i += 2) args[process.argv[i].replace(/^--/, "")] = process.argv[i + 1];
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
  const loaded = await page.waitForFunction(() => document.querySelectorAll("#lanes .lane").length > 0,
    null, { timeout: 30000 }).then(() => true, () => false);
  if (!loaded) console.log("no lanes appeared; page text:", (await page.evaluate(() => document.body.innerText)).slice(0, 400));
  await sleep(Number(args.wait ?? 2500));
  if (args.eval) { await page.evaluate(args.eval); await sleep(1000); }
  await page.screenshot({ path: resolve(args.out) });
  console.log(`saved ${resolve(args.out)} (profile ${profile})`);
} finally {
  await browser?.close().catch(() => {});
  child.kill();
}
