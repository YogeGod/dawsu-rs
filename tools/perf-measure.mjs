// Measure how long the dawsu! Tauri editor takes to open a map and how smoothly the playhead moves.
//
// node tools/perf-measure.mjs --exe <dawsu-tauri.exe> --map <difficulty.osu>
//   [--seek <ms>]       where playback starts (default 30000)
//   [--play <ms>]       how long playback is recorded (default 6000)
//   [--centre]          turn on the Playhead group's Centre, so the view scrolls every frame
//   [--runs <n>]        open and play the map this many times, each in a new editor (default 1)
//   [--json <file>]     also write the results as JSON
//
// The map's whole folder is copied into a temporary profile and the copy is opened, so the
// original mapset is never touched. Windows only (WebView2 remote debugging). Playwright Core is
// found as in tools/ui-screenshot.mjs.
//
// Load: from the open request until the editor is no longer busy and the map's lanes are on screen.
// Playhead: for each animation frame the page draws during playback, where the playhead line is.
// "Playhead moves per second" counts frames where the drawn line moved (the x app.js last drew
// the ruler's playhead line at); "longest still gap" is the longest time it stayed put while the song played. A smooth playhead moves every frame (about 60 per second).
import { spawn, spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
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
for (const key of ["exe", "map"]) if (!args[key]) throw new Error(`--${key} is required`);

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
const percentile = (values, p) => {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(p / 100 * sorted.length))];
};
const round = value => value === null || value === undefined ? null : Math.round(value * 10) / 10;

// Runs in the page once app.js has loaded: long tasks are recorded. (Tauri's invoke can't be wrapped
// without breaking the page, so app.js's update() and show() are timed instead, below.)
function instrument() {
  window.__perf = { calls: [], longTasks: [], updates: [], shows: [] };
  try {
    new PerformanceObserver(list => {
      for (const entry of list.getEntries()) window.__perf.longTasks.push({ start: entry.startTime, ms: entry.duration });
    }).observe({ type: "longtask", buffered: true });
  } catch {}
}

async function measureOnce(chromium, run) {
  const profile = resolve(mkdtempSync(join(tmpdir(), "dawsu-perf-")));
  const mapCopy = join(profile, "map");
  cpSync(dirname(resolve(args.map)), mapCopy, { recursive: true });
  const mapPath = join(mapCopy, basename(args.map));
  // A ready profile: no first-launch preset question, the DAW keys, Follow on as by default.
  mkdirSync(join(profile, "dawsu-rs"), { recursive: true });
  const preferences = { key_preset: "DAW", follow: true, return_on_stop: true };
  if (args.samples) preferences.default_samples = args.samples;
  writeFileSync(join(profile, "dawsu-rs", "preferences.json"), JSON.stringify(preferences));
  const port = await freePort();
  const child = spawn(resolve(args.exe), [], {
    env: { ...process.env, LOCALAPPDATA: profile, WEBVIEW2_USER_DATA_FOLDER: join(profile, "webview2"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
    stdio: "ignore",
  });
  let browser;
  try {
    for (let i = 0; i < 120 && !browser; i++) {
      await sleep(500);
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`).catch(() => null);
    }
    if (!browser) throw new Error("the editor's WebView2 never opened its debugging port");
    const page = browser.contexts()[0].pages()[0];
    await page.waitForFunction(() => typeof act === "function" && document.readyState === "complete", null, { timeout: 60000 });
    await page.evaluate(instrument);
    // Let the start screen settle; a first-launch question would show here.
    await sleep(1500);
    // app.js's update() (one editor round trip and its show()) and show() (the page's own work)
    // are global functions, so each call is timed by wrapping them.
    await page.evaluate(() => {
      window.__perf.updates = [];
      window.__perf.shows = [];
      window.__perf.paints = [];
      // Every line paint() draws goes through app.js's global line(); the ruler's playhead-coloured
      // one is the drawn playhead, so its x is kept, to the device pixel.
      window.__perf.playheadX = null;
      const drawLine = window.line;
      window.line = function (ctx, colour, x1, ...rest) {
        if (ctx?.canvas?.id === "ruler" && colour === slateTimeline.playhead) {
          window.__perf.playheadX = Math.round(x1 * (window.devicePixelRatio || 1));
        }
        return drawLine.call(this, ctx, colour, x1, ...rest);
      };
      for (const [name, list] of [["update", window.__perf.updates], ["show", window.__perf.shows], ["paint", window.__perf.paints]]) {
        const original = window[name];
        window[name] = function (...args) {
          const start = performance.now();
          const result = original.apply(this, args);
          const done = () => list.push({ start, ms: performance.now() - start, scene: !!(name === "show" ? args[0]?.scene : args[1] ?? true) });
          if (result?.then) result.then(done, done); else done();
          return result;
        };
      }
    });
    const firstLaunch = await page.evaluate(() => !!view.firstLaunch);
    if (firstLaunch) await page.evaluate(() => act({ kind: "key_preset", name: "DAW" }));

    // Load.
    const load = await page.evaluate(async path => {
      window.__perf.longTasks.length = 0;
      window.__perf.updates.length = 0;
      window.__perf.shows.length = 0;
      const start = performance.now();
      act({ kind: "open", path });
      const name = path.split(/[\\/]/).pop();
      let busySeen = null;
      while (performance.now() - start < 600000) {
        await new Promise(done => setTimeout(done, 5));
        if (view.busy && busySeen === null) busySeen = performance.now();
        if (!view.busy && scene?.path?.endsWith(name) && document.querySelectorAll("#lanes .lane").length > 0) break;
      }
      const ready = performance.now();
      // The first frame that paints the map.
      await new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done)));
      const painted = performance.now();
      return {
        readyMs: ready - start, paintedMs: painted - start,
        lanes: scene?.lanes?.length ?? 0, moments: scene?.moments?.length ?? 0,
        longTasks: window.__perf.longTasks.length,
        longTaskMs: window.__perf.longTasks.reduce((sum, task) => sum + task.ms, 0),
        status: view.status, error: view.error ?? null, warnings: view.warnings?.length ?? 0,
        updates: window.__perf.updates.length,
        slowestUpdateMs: Math.max(0, ...window.__perf.updates.map(u => u.ms)),
        shows: window.__perf.shows.length,
        fullShows: window.__perf.shows.filter(u => u.scene).length,
        slowestShowMs: Math.max(0, ...window.__perf.shows.map(u => u.ms)),
        showTotalMs: window.__perf.shows.reduce((sum, u) => sum + u.ms, 0),
      };
    }, mapPath);

    // Playhead.
    const playhead = await page.evaluate(async ({ seek, duration, centre }) => {
      if (centre) {
        const button = [...document.querySelectorAll("[data-ui-mount=playhead] button")].find(b => /centre/i.test(b.title + b.ariaLabel + b.textContent));
        if (button && button.ariaPressed !== "true") button.click();
      }
      await act({ kind: "seek", ms: seek });
      await new Promise(done => setTimeout(done, 500));
      act({ kind: "play" });
      // Wait for playback to start.
      const waitStart = performance.now();
      while (!view.playing && performance.now() - waitStart < 5000) await new Promise(done => setTimeout(done, 5));
      await new Promise(done => setTimeout(done, 300));
      window.__perf.longTasks.length = 0;
      window.__perf.updates.length = 0;
      window.__perf.shows.length = 0;
      window.__perf.paints.length = 0;
      const frames = [];
      const start = performance.now();
      // A hidden or covered window gets no animation frames; stop after the play time either way.
      await new Promise(done => {
        setTimeout(done, duration + 2000);
        const step = now => {
          // Where the playhead line was last drawn on the ruler (see the line() wrapper above).
          frames.push({ now, x: window.__perf.playheadX });
          if (now - start < duration) requestAnimationFrame(step);
          else done();
        };
        requestAnimationFrame(step);
      });
      await act({ kind: "stop" });
      return { frames, longTasks: window.__perf.longTasks, updates: [...window.__perf.updates], shows: [...window.__perf.shows], paints: [...window.__perf.paints] };
    }, { seek: Number(args.seek ?? 30000), duration: Number(args.play ?? 6000), centre: !!args.centre });

    const frames = playhead.frames;
    if (frames.length < 10) throw new Error(`only ${frames.length} animation frames: keep the editor's window visible while it measures`);
    const span = (frames.at(-1).now - frames[0].now) / 1000;
    // Device pixels of the drawn line; a frame without the line (off screen) counts as no move.
    const xs = frames.map(frame => frame.x);
    let moves = 0, longestStill = 0, lastMove = frames[0].now;
    const frameGaps = [], jumpPx = [];
    for (let i = 1; i < frames.length; i++) {
      frameGaps.push(frames[i].now - frames[i - 1].now);
      if (xs[i] !== null && xs[i - 1] !== null && xs[i] !== xs[i - 1]) {
        moves++;
        // A page turn while following draws the line far back: not a step of the line.
        if (xs[i] > xs[i - 1]) jumpPx.push(xs[i] - xs[i - 1]);
        longestStill = Math.max(longestStill, frames[i].now - lastMove);
        lastMove = frames[i].now;
      }
    }
    longestStill = Math.max(longestStill, frames.at(-1).now - lastMove);
    const result = {
      run,
      load: Object.fromEntries(Object.entries(load).map(([key, value]) => [key, typeof value === "number" ? round(value) : value])),
      playhead: {
        pageFramesPerSecond: round(frames.length / span),
        framesWithLine: frames.filter(frame => frame.x !== null).length,
        frames: frames.length,
        playheadMovesPerSecond: round(moves / span),
        longestStillGapMs: round(longestStill),
        medianJumpPx: round(percentile(jumpPx, 50)),
        biggestJumpPx: round(Math.max(0, ...jumpPx)),
        slowestFrameMs: round(Math.max(0, ...frameGaps)),
        framesOver33Ms: frameGaps.filter(gap => gap > 33.4).length,
        longTasks: playhead.longTasks.length,
        longTaskMs: round(playhead.longTasks.reduce((sum, task) => sum + task.ms, 0)),
        updatesPerSecond: round(playhead.updates.length / span),
        updateMedianMs: round(percentile(playhead.updates.map(u => u.ms), 50)),
        updateP95Ms: round(percentile(playhead.updates.map(u => u.ms), 95)),
        showMedianMs: round(percentile(playhead.shows.map(u => u.ms), 50)),
        showP95Ms: round(percentile(playhead.shows.map(u => u.ms), 95)),
        paintsPerSecond: round(playhead.paints.length / span),
        paintMedianMs: round(percentile(playhead.paints.map(u => u.ms), 50)),
        paintP95Ms: round(percentile(playhead.paints.map(u => u.ms), 95)),
      },
    };
    return result;
  } finally {
    await browser?.close().catch(() => {});
    child.kill();
    await new Promise(done => (child.exitCode !== null ? done() : child.once("exit", done)));
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
    } catch (error) {
      console.warn(`could not remove ${profile}: ${error.message}`);
    }
  }
}

const chromium = playwrightCore();
const results = [];
for (let run = 1; run <= Number(args.runs ?? 1); run++) {
  const result = await measureOnce(chromium, run);
  results.push(result);
  console.log(JSON.stringify(result, null, 2));
}
if (args.json) writeFileSync(resolve(args.json), JSON.stringify({ exe: resolve(args.exe), map: basename(args.map), results }, null, 2));
