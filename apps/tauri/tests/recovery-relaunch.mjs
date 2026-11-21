import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { copyFile, mkdir, mkdtemp, readFile, readdir, realpath, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { basename, dirname, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

// Run with an executable built with the recovery-smoke feature and Playwright Core
// available to Node. DAWSU_PLAYWRIGHT_CORE can point to an installed package.
assert.equal(process.platform, "win32", "This check uses the Windows WebView2 runtime.");
assert.ok(process.argv[2], "Pass the path to the recovery-smoke dawsu-tauri.exe.");
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.DAWSU_PLAYWRIGHT_CORE || "playwright-core");
const executable = await realpath(process.argv[2]);
const tempRoot = await realpath(tmpdir());
const prefix = "dawsu-recovery-relaunch-";
const profile = await mkdtemp(join(tempRoot, prefix));
const appData = join(profile, "app-data");
const recoveryDir = join(appData, "recovery");
const map = join(profile, "mapset", "copy.osu");
const localhost = "127.0.0.1";
const launches = [];

async function within(label, action, timeout = 8_000) {
  let timer;
  try {
    return await Promise.race([
      action(),
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(label + " timed out")), timeout);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

async function freePort() {
  const server = createServer();
  server.listen(0, localhost);
  await once(server, "listening");
  const { port } = server.address();
  server.close();
  await once(server, "close");
  return port;
}

async function launch(args) {
  const port = await freePort();
  const endpoint = `http://${localhost}:${port}`;
  const child = spawn(executable, args, {
    windowsHide: true,
    stdio: ["ignore", "ignore", "pipe"],
    env: {
      ...process.env,
      LOCALAPPDATA: join(profile, "local"),
      DAWSU_RECOVERY_SMOKE_DATA: appData,
      WEBVIEW2_USER_DATA_FOLDER: join(profile, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:
        `--remote-debugging-port=${port} --remote-debugging-address=${localhost}`,
    },
  });
  const run = { child, stderr: "" };
  launches.push(run);
  child.stderr.on("data", (data) => { run.stderr = (run.stderr + data).slice(-4000); });
  run.exited = new Promise((resolve) => {
    child.once("exit", (code, signal) => resolve({ code, signal }));
    child.once("error", (error) => resolve({ error }));
  });
  await within("Starting dawsu!", () => once(child, "spawn"));
  const deadline = Date.now() + 15_000;
  let ready = false;
  while (Date.now() < deadline && !ready) {
    assert.equal(child.exitCode, null, `dawsu! exited before WebView2 started. ${run.stderr}`);
    ready = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(500) })
      .then((response) => response.ok, () => false);
    if (!ready) await delay(125);
  }
  assert.ok(ready, "WebView2 did not expose its loopback test endpoint.");
  run.browser = await chromium.connectOverCDP(endpoint, { timeout: 8_000 });
  const context = run.browser.contexts()[0];
  assert.ok(context, "WebView2 did not return a browser context.");
  const page = context.pages()[0] ?? await context.waitForEvent("page", { timeout: 8_000 });
  page.setDefaultTimeout(8_000);
  await page.waitForFunction(() => window.__TAURI__?.webviewWindow.getCurrentWebviewWindow().label === "main");
  return { run, page };
}

async function state(page, action = null) {
  return within("Reading the Rust editor state", () => page.evaluate((action) => (
    window.__TAURI__.core.invoke("editor_view", { scene: true, action })
  ), action));
}

async function finishClose(run, close) {
  const closed = within("Closing dawsu!", close).catch((error) => {
    if (!/target .*closed/i.test(String(error))) throw error;
  });
  const [exit] = await Promise.all([within("Waiting for dawsu! to exit", () => run.exited), closed]);
  assert.deepEqual(exit, { code: 0, signal: null }, run.stderr);
}

const mapHash = async () => createHash("sha256").update(await readFile(map)).digest("hex");
const hitsOf = (scene) => scene.moments.map(([time, volume, anchored, hits]) => [
  time, volume, anchored, hits.map(([row]) => row),
]);
const pendingIds = async () => (await readdir(recoveryDir, { withFileTypes: true }))
  .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
  .map((entry) => entry.name);
const title = "Project persistence test [Hard]";
const editedVolume = 63;

async function dismissAudioError(page, current) {
  if (!current.error) return;
  assert.match(current.error, /audio|device|output/i);
  await page.locator("#dialog").getByRole("button", { name: "OK", exact: true }).click();
  await page.locator("#dialog").waitFor({ state: "hidden" });
}

try {
  await mkdir(dirname(map), { recursive: true });
  await mkdir(join(profile, "local"));
  await copyFile(new URL("../../../crates/dawsu-application/tests/project-fixture.osu", import.meta.url), map);
  const sourceHash = await mapHash();
  console.log("Using a disposable Windows app data and WebView2 profile at " + profile);

  const first = await launch([map]);
  // The open file shows in the status bar; unsaved edits show on Save (data-dirty).
  await first.page.locator("#file-name").filter({ hasText: title }).waitFor();
  const before = await state(first.page);
  await dismissAudioError(first.page, before);
  assert.equal(before.scene.moments[0][1], 100);
  assert.equal(before.dirty, false);
  await first.page.locator("#status").click();
  await first.page.keyboard.press("Control+a");
  await first.page.locator("#selected").filter({ hasText: /\(\d+\)/ }).waitFor();
  await first.page.getByLabel("Set volume", { exact: true }).fill(String(editedVolume));
  await first.page.locator("#set-volume").click();
  await first.page.locator('#save[data-dirty="true"]').waitFor();
  const edited = await state(first.page);
  assert.equal(edited.scene.moments[0][1], editedVolume);
  assert.equal(edited.dirty, true);
  assert.equal(await mapHash(), sourceHash);
  console.log(`Process ${first.run.child.pid} changed volume from 100 to ${editedVolume} with unsaved edits.`);

  const fault = await state(first.page, { kind: "recovery_smoke_panic" });
  assert.match(fault.fault, /Saved a recovery copy/);
  const message = first.page.locator("#dialog");
  await message.getByRole("heading", { name: "Editor stopped" }).waitFor();
  assert.match(await message.textContent(), /Project persistence test \[Hard\]/);
  const ids = await pendingIds();
  assert.equal(ids.length, 1);
  const [id] = ids;
  assert.match(id, /^Project persistence test - Hard - \d+/);
  await finishClose(first.run, () => message.getByRole("button", { name: "Close dawsu!", exact: true }).click());
  assert.equal(await mapHash(), sourceHash);
  console.log(`Process ${first.run.child.pid} saved ${id} and exited with code 0 through the fault dialog.`);

  const second = await launch([]);
  assert.notEqual(second.run.child.pid, first.run.child.pid);
  const home = second.page.getByRole("region", { name: "Recovery copies" });
  await home.waitFor();
  const start = await state(second.page);
  await dismissAudioError(second.page, start);
  assert.equal(start.hasDocument, false);
  assert.equal(await second.page.locator("#home-open").isEnabled(), true);
  assert.match(await home.locator("time").textContent(), /\d/);
  await home.getByRole("button", { name: "Restore " + title, exact: true }).click();
  await second.page.locator('#save[data-dirty="true"]').waitFor();
  const restored = await state(second.page);
  assert.equal(restored.dirty, true);
  assert.deepEqual(hitsOf(restored.scene), hitsOf(edited.scene));
  assert.deepEqual(restored.scene.lanes, edited.scene.lanes);
  assert.equal(await mapHash(), sourceHash);
  assert.deepEqual(await pendingIds(), [id]);
  console.log(`Process ${second.run.child.pid} restored volume ${editedVolume} as unsaved without changing the map SHA-256 ${sourceHash}.`);

  await second.page.getByRole("button", { name: "Save", exact: true }).click();
  await second.page.locator("#status").filter({ hasText: "Saved with backup" }).waitFor();
  const saved = await state(second.page);
  assert.equal(saved.dirty, false);
  assert.notEqual(await mapHash(), sourceHash);
  assert.deepEqual(await pendingIds(), []);
  assert.deepEqual(await readdir(join(recoveryDir, "archive")), [id]);
  await finishClose(second.run, () => second.page.evaluate(() => (
    window.__TAURI__.core.invoke("close_editor", { discard: false })
  )));
  console.log("Explicit Save archived the offer and the second process exited with code 0.");
} finally {
  try {
    for (const run of launches.reverse()) {
      if (run.child.exitCode === null && run.child.signalCode === null && run.child.pid) {
        if (run.child.kill()) await within("Stopping the test process", () => run.exited);
      }
      if (run.browser?.isConnected()) {
        await within("Closing WebView2 automation", () => run.browser.close());
      }
    }
  } finally {
    const resolved = await realpath(profile);
    assert.equal(dirname(resolved), tempRoot);
    assert.equal(basename(resolved), basename(profile));
    assert.ok(basename(resolved).startsWith(prefix));
    await rm(resolved, { recursive: true, maxRetries: 5, retryDelay: 200 });
  }
}
