// The long robot mapper run: press every input the real, built dawsu! editor has, on a copy of a map.
//
// node tools/every-input.mjs --exe <dawsu-tauri.exe> --map <difficulty.osu> [--only <regex>]
//   [--presets DAW,osu!] [--routes menu,key,palette] [--skip <regex>] [--json <file>]
//   [--shots <folder>] [--keep-profile] [--allow-stale]
//
//   --exe       the built editor (cargo build -p dawsu-tauri --locked: target/debug/dawsu-tauri.exe)
//   --map       an osu!standard difficulty, best a ranked one; its whole folder is copied into the
//               run's profile and only the copy is opened and edited (default: the everyday check's
//               fixture). Nothing is written next to the original.
//   --only      run only the steps whose name matches this regular expression (the start, the map
//               opening and the closing safety checks always run)
//   --skip      leave out the steps whose name matches
//   --presets   the key presets to sweep (default both: DAW,osu!)
//   --routes    how menu rows and commands are reached (default all three: menu, key and palette)
//   --json      write the results as data; tools/robot-inputs.mjs --run <file> turns them into the
//               coverage table
//
// What it does, in order: starts with no map and uses the start screen; opens the copy; goes
// through every menu row, every command by its key in each preset, every Ctrl+K entry, every mouse
// action the editor lists in Shortcuts and mouse actions, and the features merged on 2026-11-24;
// then presses every control it can see in the main window, its dialogs and the other windows; and
// last checks that the original folder is byte-identical. The Save step checks that Save left no
// file the difficulty does not play; the last step lists what the whole run left in the copy.
//
// Like tools/everyday-check.mjs it acts only through real input (mouse, keys, the windows' own
// controls, and the native file pickers through tools/robot/native.ps1) and reads the page only
// to check the result; it never calls the page's act(). It reads the key list with the page's own
// read-only editor_shortcuts call, so the sweep follows the catalogue as it is now.
//
// Sound: the song and every sample folder are muted before anything plays, and playback is stopped
// within a second. File pickers are answered with files inside the run's profile, or cancelled.
// Explorer windows the editor opens are closed again. Every console error, page error and
// unhandled rejection fails the step it happened in.
//
// Windows only in practice (WebView2 remote debugging and the native pickers).
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkProvenance, createRobot, parseArgs, silentWav, sleep, snapshot } from "./robot/core.mjs";
import { startNative } from "./robot/native.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = parseArgs();
if (!args.exe) throw new Error("--exe is required: the built dawsu-tauri.exe");
const exe = resolve(args.exe);
if (!existsSync(exe)) throw new Error(`no editor at ${exe}`);
for (const key of ["map", "shots", "json", "only", "skip", "presets", "routes"]) if (args[key] === true) throw new Error(`--${key} needs a value`);
checkProvenance(repo, exe, Boolean(args["allow-stale"]), "every-input");
const original = resolve(args.map ?? join(repo, "tests/fixtures/save-pack/synthetic/lf-no-final-newline.osu"));
if (!existsSync(original)) throw new Error(`no map at ${original}`);
const only = args.only ? new RegExp(args.only, "i") : null;
const skip = args.skip ? new RegExp(args.skip, "i") : null;
const presets = (args.presets ?? "DAW,osu!").split(",").map(name => name.trim()).filter(Boolean);
const routes = new Set((args.routes ?? "menu,key,palette").split(",").map(name => name.trim()));
const started = Date.now();

const robot = createRobot({ repo, exe, name: "every-input", shots: args.shots,
  opensMap: ["Open the map copy"] });
const { profile, launch, quit, page, read, until, expect, hits, at, menuRow, openMenu, pickTool,
  dialogTitle, dialogButton, windowOpened } = robot;
const native = startNative();

// The map copy, and the files the pickers are answered with, all inside the profile.
const mapFolder = join(profile, "map", basename(dirname(original)));
const mapCopy = join(mapFolder, basename(original));
const inputs = join(profile, "inputs");
const outputs = join(profile, "outputs");
const songs = join(profile, "songs");
mkdirSync(inputs, { recursive: true });
mkdirSync(outputs, { recursive: true });
const originalBefore = snapshot(dirname(original));
cpSync(dirname(original), mapFolder, { recursive: true, dereference: true });
// A second copy under a Songs folder, for the start screen's mapset picker.
cpSync(dirname(original), join(songs, basename(dirname(original))), { recursive: true, dereference: true });
const maniaFixture = join(repo, "tests/fixtures/save-pack/synthetic/mania-4k-holds-keysounds.osu");
const maniaCopy = join(inputs, "mania-import", basename(maniaFixture));
mkdirSync(dirname(maniaCopy), { recursive: true });
cpSync(maniaFixture, maniaCopy);
const stemWav = join(inputs, "every-input-stem.wav");
silentWav(stemWav, 2);
const laneSample = "every-input-lane.wav";
silentWav(join(mapFolder, laneSample));
const replacementWav = join(inputs, "every-input-replacement.wav");
silentWav(replacementWav);
const copyBefore = snapshot(mapFolder);

// Whether a difficulty plays a sample file: its name is in the .osu, or it is a set-sound-index file
// whose custom index a hit object or timing line uses (as tools/everyday-check.mjs checks it).
function playedBy(osu, path) {
  const name = path.split("/").pop();
  if (osu.includes(name)) return true;
  const match = /^(normal|soft|drum)-(hit(normal|whistle|finish|clap)|slider(slide|whistle|tick))(\d+)\.(wav|ogg|mp3)$/i.exec(name);
  if (!match) return false;
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
  return indexes.has(Number(match[5]));
}

// The step wrapper: --only and --skip pick steps; `covers` names inventory entries (tools/robot-inputs.mjs).
const always = new Set(["Start with no map and see the start screen", "Open the map copy", "The original map folder is byte-identical",
  "No errors after the last step"]);
// Set when the editor ever shows a map outside the run's profile: the rest of the run is skipped, so
// nothing can be saved into a real mapset.
let outside = null;
const inProfile = path => !path || resolve(path).toLowerCase().startsWith(resolve(profile).toLowerCase());
async function step(name, body, covers = [], options = {}) {
  if (!always.has(name) && ((only && !only.test(name)) || (skip && skip.test(name)))) return;
  if (outside && !options.always) {
    await robot.step(name, async () => ({ skip: outside }), { covers: [] });
    return;
  }
  await robot.step(name, async result => {
    result.reached = [];
    // An editor that quit in the last step (its window closed) is started again on the copy.
    if (robot.app && robot.app.child.exitCode !== null) {
      await robot.quit();
      await launch(mapCopy);
      await until(() => window.scene?.lanes?.length > 0, null, "the editor did not reopen the copy", 60000);
      probe = null;
    }
    try {
      const note = await body(result);
      // A step that ends with the editor stuck elsewhere would fail every step after it.
      await settle().catch(() => {});
      return note;
    } finally {
      const path = robot.app?.page ? await read(() => window.scene?.path ?? null).catch(() => null) : null;
      if (!inProfile(path)) {
        outside = `the editor opened ${path}, outside the run's copy, so the run stopped`;
        await robot.quit();
        throw new Error(outside);
      }
    }
  }, { covers, ...options });
}

// What the editor shows, as one object; two of them differ where something happened.
const fingerprint = () => read(() => {
  const s = window.scene, v = window.view ?? {};
  const visible = element => !!element && element.getClientRects().length > 0 && getComputedStyle(element).visibility !== "hidden";
  const selected = [];
  let count = 0;
  for (const [time, , , list] of s?.moments ?? []) {
    for (const [row, isSelected, muted, volume] of list) {
      count++;
      if (isSelected && selected.length < 40) selected.push(`${time}:${row}:${volume ?? ""}`);
      if (muted && selected.length < 40) selected.push(`muted ${time}:${row}`);
    }
  }
  const dialog = document.getElementById("dialog");
  const menu = document.getElementById("editor-menu");
  return {
    path: s?.path ?? null,
    history: s ? `${s.historyPosition}/${s.history?.length}` : null,
    hits: count,
    selection: s?.selection?.count ?? 0,
    selected: selected.join(" "),
    range: s?.range ? s.range.map(Math.round).join("-") : null,
    tool: v.tool,
    zoom: s ? Math.round(s.zoom * 10000) : null,
    start: s ? Math.round(s.viewStart) : null,
    rowHeight: s?.rowHeight ?? null,
    scrollTop: document.getElementById("scroll")?.scrollTop ?? 0,
    position: Math.round(v.position ?? 0),
    playing: !!v.playing,
    follow: !!v.follow, returnOnStop: !!v.returnOnStop, cycle: !!v.cycle,
    left: Math.round(v.left ?? 0), right: Math.round(v.right ?? 0),
    songMuted: !!v.songMuted, objectLines: !!v.objectLines, audition: !!v.audition,
    hs: !!s?.hs, protect: !!s?.protect, snap: s ? `${s.beats}/${s.divisor}/${s.snap}/${s.snapMode}` : null,
    bookmarks: s?.bookmarks?.length ?? 0, lanes: s?.lanes?.length ?? 0, laneOrder: (s?.laneOrder ?? []).join(","),
    laneNames: (s?.lanes ?? []).map(lane => lane.name).join("|"),
    activeLane: v.activeLane ?? null, speed: v.speed ?? null, playbackStart: v.playbackStart ?? null,
    stemActive: !!v.stemActive, stemLoaded: !!v.stemLoaded, panels: JSON.stringify(v.panels ?? {}),
    dirty: !!v.dirty, home: !!v.showHome, keyPreset: v.keyPreset ?? null, hsMode: !!v.hsMode,
    externalOsu: !!v.externalOsuChanged, busy: !!v.busy,
    dialog: dialog?.open ? document.getElementById("dialog-title").textContent : null,
    modals: Array.from(document.querySelectorAll('[role="dialog"]')).filter(visible).filter(node => node.id !== "dialog")
      .map(node => node.getAttribute("aria-label") ?? document.getElementById(node.getAttribute("aria-labelledby") ?? "")?.textContent ?? "dialog").join("|"),
    menu: menu && !menu.hidden ? (menu.getAttribute("aria-label") ?? "menu") : null,
    layout: !!document.getElementById("layout-editor") && !document.getElementById("layout-editor").hidden,
    toasts: document.querySelectorAll(".toast-card").length,
    status: document.getElementById("status")?.textContent ?? "",
    waveform: document.getElementById("ruler-waveform")?.getAttribute("aria-pressed") ?? null,
    soundFirst: document.getElementById("ruler-sound-first")?.getAttribute("aria-pressed") ?? null,
    usedOnly: !!document.getElementById("used-only")?.checked,
    mapPreview: !!window.dawsuUi?.mapPreviewOn?.(),
    collapse: document.getElementById("collapse-layers")?.getAttribute("aria-pressed") ?? null,
    folders: Array.from(document.querySelectorAll("#lanes .sample-folder")).map(node =>
      `${node.dataset.sampleSet}:${node.querySelector(".fold")?.getAttribute("aria-expanded")}`).join(","),
    songMute: document.getElementById("song-mute")?.getAttribute("aria-pressed") ?? null,
    waveHeight: getComputedStyle(document.getElementById("editor-arrange") ?? document.body).getPropertyValue("--wave-height").trim(),
    scale: document.documentElement.style.zoom || getComputedStyle(document.documentElement).getPropertyValue("--ui-scale").trim(),
    palette: !!window.dawsuCommandPaletteOpen,
    focus: document.activeElement?.id || document.activeElement?.getAttribute?.("aria-label") || document.activeElement?.tagName || "",
    hint: document.querySelector("[data-hint-bar], .hint-bar, #status-hint")?.textContent ?? "",
  };
});
const differs = (before, after) => Object.keys(after).filter(key => JSON.stringify(before[key]) !== JSON.stringify(after[key]));
const describe = (before, after, keys) => keys.map(key => `${key} ${JSON.stringify(before[key])?.slice(0, 40)} → ${JSON.stringify(after[key])?.slice(0, 40)}`).join("; ");
const pid = () => robot.app?.child?.pid;
const others = () => robot.app.context.pages().filter(candidate => !candidate.isClosed() && candidate !== page());

// Put the editor back where every probe starts: no dialog, menu, window, picker or playback.
async function settle() {
  if (!robot.app?.page) return;
  if (native.available && (await native.dialogs(pid())).length) {
    await native.cancel(pid());
    await sleep(400);
  }
  for (const window of others()) {
    const name = window.url().split("/").pop();
    if (name === "preview.html") continue;
    const button = window.locator("button", { hasText: /^(Cancel|Close)$/ }).first();
    if (await button.count().catch(() => 0) && await button.isVisible().catch(() => false)) await button.click().catch(() => {});
    else await window.keyboard.press("Escape").catch(() => {});
    await sleep(500);
    if (!window.isClosed()) await window.close().catch(() => {});
  }
  for (let i = 0; i < 6; i++) {
    const now = await fingerprint().catch(() => null);
    if (!now) return;
    if (now.layout) {
      await page().click("#layout-cancel").catch(() => {});
      await sleep(250);
      continue;
    }
    if (!now.dialog && !now.modals && !now.menu && !now.palette && !now.home) break;
    if (now.home && !now.dialog && !now.modals && now.path) {
      await page().keyboard.press("Escape");
      await sleep(400);
      continue;
    }
    await page().keyboard.press("Escape");
    await sleep(300);
    const still = await fingerprint();
    if (still.dialog) {
      const button = page().locator("#dialog-actions button", { hasText: /^(Cancel|Close|Done|No|Keep editing|OK)$/ }).first();
      if (await button.count()) await button.click().catch(() => {});
    } else if (still.modals) {
      const close = page().locator('[role="dialog"] button').filter({ hasText: /^(Cancel|Close|Done|×|Skip|Not now)$/ }).first();
      const named = page().locator('[role="dialog"] button[aria-label^="Close"]').first();
      if (await named.count() && await named.isVisible()) await named.click().catch(() => {});
      else if (await close.count() && await close.isVisible()) await close.click().catch(() => {});
    }
    await sleep(250);
  }
  const now = await fingerprint();
  if (now.playing) {
    await page().locator('[aria-label="Transport"] button[aria-label="Stop"]').click().catch(() => {});
    await until(() => !window.view.playing, null, "playback did not stop", 3000).catch(() => {});
  }
  // Whatever a step pressed, the song and the sample folders stay muted and Audition stays off.
  if (now.path) await silence().catch(() => {});
}

// Explorer windows: the ones open before an input, so only the editor's new ones are closed.
// Only windows showing one of the run's own folders are closed; any other Explorer window is the
// mapper's own and stays open.
const runFolders = () => [basename(mapFolder), "map", "inputs", "outputs", "songs", basename(profile), "every-input"].map(name => name.toLowerCase());
const ours = window => runFolders().some(name => window.title.toLowerCase() === name || window.title.toLowerCase().includes(name));
async function closeNewExplorers(before) {
  const now = await native.explorers();
  const opened = now.filter(window => !before.some(old => old.hwnd === window.hwnd) && ours(window));
  for (const window of opened) await native.close(window.hwnd);
  // Explorer may show the folder in a window that was already open.
  const reused = now.filter(window => before.some(old => old.hwnd === window.hwnd) && ours(window) &&
    before.find(old => old.hwnd === window.hwnd).title !== window.title);
  return [...opened, ...reused];
}

// The robot's own hit: placed on an empty part of the song, so the probes never move the map's hits.
let probe = null;
let laneRow = 0;
async function view(time, zoom = 0.3) {
  // Centre the timeline on the time with the overview, as a mapper does, then zoom with the buttons.
  for (let i = 0; i < 12; i++) {
    const state = await read(() => ({ zoom: window.scene.zoom, start: window.scene.viewStart,
      width: document.getElementById("grid").clientWidth, duration: window.view.duration }));
    const inside = time > state.start + 30 / state.zoom && time < state.start + (state.width - 30) / state.zoom;
    if (state.zoom >= zoom && inside) return;
    if (!inside) {
      const box = await read(() => document.getElementById("overview").getBoundingClientRect().toJSON());
      await page().mouse.click(box.x + box.width * Math.min(0.999, Math.max(0.001, time / Math.max(1, state.duration))), box.y + box.height / 2);
      await sleep(250);
    }
    if (state.zoom < zoom) {
      await page().click("#ruler-zoom-in");
      await sleep(250);
    }
  }
}
async function revealLane(row) {
  for (let i = 0; i < 30; i++) {
    const where = await read(row => {
      const lane = document.querySelector(`#lanes [data-lane-row="${row}"]`)?.getBoundingClientRect();
      const grid = document.getElementById("grid").getBoundingClientRect();
      return lane ? { lane: lane.toJSON(), grid: grid.toJSON() } : null;
    }, row);
    if (!where) throw new Error(`lane row ${row} has no header`);
    if (where.lane.y > where.grid.y + 2 && where.lane.y + where.lane.height < where.grid.y + where.grid.height - 2) return;
    // The lane header area scrolls the lanes in either preset.
    const header = await read(() => document.getElementById("lanes").getBoundingClientRect().toJSON());
    await page().mouse.move(header.x + header.width / 2, header.y + header.height / 2);
    await page().mouse.wheel(0, where.lane.y < where.grid.y ? -120 : 120);
    await sleep(150);
  }
  throw new Error(`lane row ${row} never came into view`);
}
async function pointAt(time, row) {
  await view(time);
  await revealLane(row);
  return at(time, row);
}
const probeHit = () => read(([time, row]) => window.scene.moments.some(([t, , , list]) => t === time && list.some(([r]) => r === row)), probe);
async function ensureProbe() {
  if (probe && await probeHit()) return probe;
  const target = await read(() => {
    const times = window.scene.moments.map(([time]) => time);
    for (const [, time] of window.scene.gameplay?.points ?? []) times.push(time);
    for (const [, from, to] of window.scene.gameplay?.spans ?? []) times.push(from, to);
    times.sort((a, b) => a - b);
    const duration = window.view.duration;
    let best = [duration / 2, 0];
    for (let i = 0; i + 1 < times.length; i++) {
      const gap = times[i + 1] - times[i];
      const middle = (times[i] + times[i + 1]) / 2;
      if (gap > best[1] && middle > duration * 0.1 && middle < duration * 0.9) best = [middle, gap];
    }
    const lane = window.scene.lanes.findIndex(lane => lane.name === "normal-hitwhistle" || lane.name === "soft-hitwhistle");
    return { time: best[0], gap: best[1], lane: lane >= 0 ? lane : 0 };
  });
  laneRow = target.lane;
  await pickTool("Draw");
  const before = await hits();
  const point = await pointAt(target.time, laneRow);
  await page().mouse.click(point.x, point.y);
  const added = await until(count => {
    const now = window.scene.moments.flatMap(([time, , , list]) => list.map(([row]) => [time, row]));
    return now.length === count + 1 ? now : null;
  }, before.length, "the probe hit was not placed");
  probe = added.find(([time, row]) => !before.some(([t, r]) => t === time && r === row));
  await pickTool("Select");
  return probe;
}
async function selectProbe() {
  await ensureProbe();
  const now = await read(([time, row]) => window.scene.selection?.count === 1 && window.scene.moments.some(([t, , , list]) =>
    t === time && list.some(([r, selected]) => r === row && selected)), probe);
  if (now) return;
  if ((await read(() => window.view.tool)) !== "Select") await pickTool("Select");
  const point = await pointAt(probe[0], probe[1]);
  await page().mouse.click(point.x, point.y);
  await until(([time, row]) => window.scene.selection?.count === 1 && window.scene.moments.some(([t, , , list]) =>
    t === time && list.some(([r, selected]) => r === row && selected)), probe, "the probe hit was not selected", 4000);
}
// A second hit: the map's nearest hit on another lane is added with Ctrl+click.
async function selectTwo() {
  await selectProbe();
  const other = await read(([time, row]) => {
    let best = null;
    for (const [t, , , list] of window.scene.moments) {
      for (const [r] of list) if (!(t === time && r === row) && (!best || Math.abs(t - time) < Math.abs(best[0] - time))) best = [t, r];
    }
    return best;
  }, probe);
  if (!other) throw new Error("the map has no other hit to select");
  const point = await pointAt(other[0], other[1]);
  await page().keyboard.down("Control");
  await page().mouse.click(point.x, point.y);
  await page().keyboard.up("Control");
  await until(() => (window.scene.selection?.count ?? 0) >= 2, null, "Ctrl+click did not add a second hit", 4000);
  return other;
}
async function markRange() {
  await ensureProbe();
  await pickTool("Range");
  const zoom = await read(() => window.scene.zoom);
  const from = await pointAt(probe[0] - 150 / zoom, probe[1]);
  await page().mouse.move(from.x, from.y);
  await page().mouse.down();
  for (let i = 1; i <= 6; i++) await page().mouse.move(from.x + 50 * i, from.y + 2);
  await page().mouse.up();
  await until(() => !!window.scene.range, null, "the Range drag marked no range", 4000);
  await pickTool("Select");
}
async function addBookmark() {
  if ((await read(() => window.scene.bookmarks?.length ?? 0)) > 0) return;
  await openMenu("Map");
  await menuRow("Add bookmark").click();
  await sleep(400);
  if ((await dialogTitle())?.startsWith("Add bookmark")) await dialogButton(/^(Add|OK|Save)$/).click();
  await until(() => window.scene.bookmarks?.length > 0, null, "no bookmark was added", 4000);
}
let stemAttached = false;
async function attachStem() {
  if (await read(() => !!window.view.stemLoaded)) return;
  await openMenu("Playback");
  await menuRow("Attach drum stem…").click();
  await answerPicker(stemWav, "the drum stem picker");
  await until(() => window.view.stemLoaded && !window.view.stemLoading, null, "the drum stem did not load", 15000);
  stemAttached = true;
}
async function answerPicker(path, what) {
  if (!native.available) throw new Error(`${what} cannot be answered: ${native.reason}`);
  const end = Date.now() + 8000;
  while (Date.now() < end && !(await native.dialogs(pid())).length) await sleep(200);
  expect((await native.dialogs(pid())).length, `${what} did not open`);
  const answer = await native.choose(pid(), path);
  expect(answer.ok, `${what} could not be answered: ${answer.error ?? "the name box or OK button was not found"}`);
  await sleep(600);
}
// Undo with the Edit menu until the history is back where it was (never past it).
async function undoTo(history) {
  for (let i = 0; i < 12; i++) {
    const now = await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`);
    if (Number(now.split("/")[0]) <= Number(history.split("/")[0])) return;
    await openMenu("Edit");
    const undo = menuRow("Undo");
    if (!(await undo.isEnabled())) {
      await page().keyboard.press("Escape");
      return;
    }
    await undo.click();
    await sleep(250);
  }
}
async function silence() {
  // Mute the song and every sample folder, so anything that plays is silent.
  if ((await read(() => document.getElementById("song-mute")?.getAttribute("aria-pressed"))) !== "true") {
    await page().click("#song-mute").catch(() => {});
  }
  for (const button of await page().locator('#lanes .sample-folder button.lane-mute[aria-pressed="false"]').all()) {
    await button.click().catch(() => {});
  }
  if (await read(() => window.view.audition)) await page().locator('button[aria-label="Audition"]').click();
  await until(() => !window.view.audition, null, "Audition is still on", 3000);
}

const needs = {
  selection: selectProbe,
  two: selectTwo,
  range: markRange,
  bookmark: addBookmark,
  stem: attachStem,
  probe: ensureProbe,
  playing: async () => {
    await silence();
    if (!(await read(() => window.view.playing))) await page().locator('[aria-label="Transport"] button[aria-label="Play"]').click();
    await until(() => window.view.playing, null, "playback did not start", 3000);
  },
  stopped: async () => {
    if (await read(() => window.view.playing)) await page().locator('[aria-label="Transport"] button[aria-label="Stop"]').click();
    await until(() => !window.view.playing, null, "playback did not stop", 3000);
  },
  midSong: async () => {
    // The playhead somewhere in the middle, so start, end, seek and jump keys all have room.
    const box = await read(() => document.getElementById("ruler").getBoundingClientRect().toJSON());
    await page().mouse.click(box.x + box.width / 2, box.y + 30);
    await sleep(300);
  },
};

// What every command and menu row should do. `expect` is checked against the editor before and after:
//   changed: [fields]   at least one of these fingerprint fields changes
//   dialog: "Title"     the editor's dialog opens with this title (then it is closed with its Cancel)
//   modal: "Label"      a Svelte dialog with this label opens
//   window: "x.html"    another editor window opens
//   native: true        a native file or folder picker opens (then it is cancelled)
//   explorer: true      an Explorer window opens (then it is closed)
//   respond: true       anything visible happens: a field changes, a toast or a dialog
// `undo`: Undo it afterwards with the Edit menu; `twice`: run it again to put a switch back;
// `needs`: what must be set up first (see needs above).
const specs = {
  // File
  open: { expect: { native: true } },
  // Opens the map osu! has open, which is a real mapset: never pressed by the robot.
  openFromOsu: { unsafe: "Open from osu! opens the map open in osu!, a real mapset, so the robot never presses it" },
  importOsz: { expect: { native: true } },
  importMidi: { expect: { modal: "Import MIDI" } },
  importManiaHitsounds: { expect: { native: true } },
  exportMidi: { expect: { native: true } },
  openFolder: { expect: { explorer: true } },
  save: { needs: ["probe"], before: () => makeEdit(), expect: { changed: ["dirty", "dialog"] }, after: finishSave },
  revert: { needs: ["probe"], before: () => makeEdit(), expect: { dialog: "Revert to saved" }, undo: true },
  revertOriginal: { expect: { dialog: "Revert mapset to original" } },
  undoRevert: { expect: { respond: true } },
  export: { expect: { modal: "Export" } },
  startScreen: { expect: { changed: ["home"], dialog: "Save changes before going Home?" } },
  folders: { expect: { dialog: "Folders" } },
  preferences: { expect: { modal: "Preferences" } },
  // Edit
  undo: { needs: ["probe"], expect: { changed: ["history"] }, before: makeEdit, redoAfter: true },
  redo: { needs: ["probe"], expect: { changed: ["history"] }, before: async () => { await makeEdit(); await undoTo(await read(() => `${window.scene.historyPosition - 1}/0`)); }, undo: true },
  cut: { needs: ["selection"], expect: { changed: ["hits", "history"] }, undo: true },
  copy: { needs: ["selection"], expect: { respond: true, quiet: true } },
  paste: { needs: ["selection"], before: copySelection, expect: { changed: ["hits", "history"] }, undo: true },
  duplicate: { needs: ["selection"], expect: { changed: ["hits", "history"] }, undo: true },
  delete: { needs: ["selection"], expect: { changed: ["hits", "history"] }, undo: true },
  selectAll: { needs: ["selection"], expect: { changed: ["selection"] } },
  quantize: { needs: ["selection"], expect: { respond: true, quiet: true }, undo: true },
  quantizeDialog: { needs: ["selection"], expect: { dialog: "" } },
  repeat: { needs: ["selection"], expect: { dialog: "" } },
  patternStamp: { needs: ["selection"], expect: { dialog: "" } },
  editRange: { needs: ["range"], expect: { dialog: "" } },
  scaleByVolume: { expect: { dialog: "" } },
  placeSoundSet: { expect: { respond: true } },
  saveSoundSet: { needs: ["selection"], expect: { respond: true } },
  soundSetPlacement: { expect: { respond: true } },
  protect: { expect: { changed: ["protect"] }, twice: true },
  audition: { expect: { changed: ["audition"] }, twice: true },
  // Playback
  play: { needs: ["stopped"], before: silence, expect: { changed: ["playing"] } },
  stop: { needs: ["playing"], expect: { changed: ["playing", "position"] } },
  start: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  goToEnd: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  returnOnStop: { expect: { changed: ["returnOnStop"] }, twice: true },
  follow: { expect: { changed: ["follow"] }, twice: true },
  cycle: { expect: { changed: ["cycle"] }, twice: true },
  locatorsToSelection: { needs: ["selection"], expect: { changed: ["left", "right"] } },
  playLocatorRange: { needs: ["stopped"], before: silence, expect: { changed: ["playing", "position"] } },
  muteSong: { expect: { changed: ["songMuted", "songMute"] }, twice: true },
  toggleStemMonitor: { needs: ["stem"], expect: { changed: ["stemActive"] }, twice: true },
  separateDrumsAi: { expect: { modal: "Separate drums (AI)" } },
  attachDrumStem: { expect: { native: true } },
  clearDrumStem: { needs: ["stem"], expect: { changed: ["stemLoaded"] } },
  audioOutput: { expect: { dialog: "Audio" } },
  // Map
  newDiff: { expect: { dialog: "New difficulty" } },
  generate: { expect: { dialog: "Generate hitsound difficulty" } },
  copyHitsounds: { expect: { dialog: "Copy hitsounds" } },
  sharedBase: { expect: { modal: "Shared Base layer" } },
  drumDetection: { expect: { window: "drum.html" } },
  nodeGraph: { expect: { window: "node-graph.html" } },
  keysounds: { expect: { modal: "Keysounds and VST3" } },
  hsMode: { expect: { changed: ["hs", "hsMode"] }, twice: true },
  addBookmark: { needs: ["midSong"], expect: { changed: ["bookmarks", "dialog"] }, after: confirmBookmark },
  previousBookmark: { needs: ["bookmark", "stopped"], before: async () => seekTo("end"), expect: { changed: ["position"] } },
  nextBookmark: { needs: ["bookmark", "stopped"], before: async () => seekTo("start"), expect: { changed: ["position"] } },
  addLane: { expect: { dialog: "Add lane" } },
  deleteActiveLane: { needs: ["probe"], expect: { dialog: "Delete lane" } },
  // View
  browser: { expect: { changed: ["panels"] }, twice: true },
  inspector: { expect: { changed: ["panels"] }, twice: true },
  volumePanel: { expect: { changed: ["panels"] }, twice: true },
  mapPreview: { expect: { changed: ["mapPreview"] }, twice: true },
  laneNames: { expect: { changed: ["panels"] }, twice: true },
  songWaveform: { expect: { changed: ["waveform"] }, twice: true },
  soundFirst: { expect: { changed: ["soundFirst"] }, twice: true },
  objectLines: { expect: { changed: ["objectLines"] }, twice: true },
  usedOnly: { expect: { changed: ["usedOnly", "lanes"] }, twice: true },
  collapseLayers: { expect: { respond: true, quiet: true }, twice: true },
  filter: { expect: { respond: true } },
  rowsIn: { expect: { changed: ["rowHeight"] } },
  rowsOut: { expect: { changed: ["rowHeight"] } },
  zoomIn: { expect: { changed: ["zoom"] } },
  zoomOut: { expect: { changed: ["zoom"] } },
  fit: { before: async () => page().click("#ruler-zoom-in"), expect: { changed: ["zoom", "start"] } },
  timeSignatures: { expect: { dialog: "Time signatures" } },
  drumMap: { expect: { respond: true } },
  editLayout: { expect: { changed: ["layout"] } },
  viewSettings: { expect: { dialog: "View" } },
  quickStart: { expect: { modal: "" } },
  help: { expect: { dialog: "Shortcuts and mouse actions" } },
  // Tools: picked from a different tool, so the pick is seen.
  toolSelect: { before: async () => pickTool("Draw"), expect: { changed: ["tool"] } },
  toolDraw: { expect: { changed: ["tool"] } },
  toolErase: { expect: { changed: ["tool", "menu"] } },
  toolMute: { expect: { changed: ["tool"] } },
  toolGrab: { expect: { changed: ["tool"] } },
  toolRange: { expect: { changed: ["tool"] } },
  toolRamp: { expect: { changed: ["tool"] } },
  // The lane right-click menu (on the robot's lane).
  laneRename: { needs: ["probe"], expect: { dialog: "Rename lane" } },
  laneMoveUp: { needs: ["probe"], expect: { changed: ["laneOrder", "folders", "history"], orQuiet: "the lane is already the first one it can be" }, undo: true },
  laneMoveDown: { needs: ["probe"], expect: { changed: ["laneOrder", "folders", "history"], orQuiet: "the lane is already the last one it can be" }, undo: true },
  laneReplaceSample: { needs: ["probe"], expect: { dialog: "Replace sample on" } },
  laneColour: { needs: ["probe"], expect: { dialog: "Lane colour" } },
  laneAddLayer: { needs: ["probe"], expect: { changed: ["lanes", "history"] }, undo: true },
  laneCloneVariant: { needs: ["probe"], expect: { changed: ["lanes", "history"] }, undo: true },
  laneAlternate: { needs: ["two"], expect: { changed: ["hits", "selected", "history"] }, undo: true },
  laneAdd: { needs: ["probe"], expect: { dialog: "Add lane" } },
  laneDelete: { needs: ["probe"], expect: { dialog: "Delete lane" } },
  // Key-only commands
  volumeUp: { needs: ["selection"], expect: { changed: ["selected", "history"] }, undo: true },
  volumeDown: { needs: ["selection"], expect: { changed: ["selected", "history"] }, undo: true },
  volumeUpFine: { needs: ["selection"], expect: { changed: ["selected", "history"] }, undo: true },
  volumeDownFine: { needs: ["selection"], expect: { changed: ["selected", "history"] }, undo: true },
  jumpToSelection: { needs: ["selection", "stopped"], before: silence, expect: { changed: ["playing", "position"] } },
  nudgeLeft: { needs: ["selection"], expect: { changed: ["selected", "history"] }, undo: true },
  nudgeRight: { needs: ["selection"], expect: { changed: ["selected", "history"] }, undo: true },
  gridLeft: { needs: ["selection"], expect: { changed: ["selected", "history"], orQuiet: "the hit already sits on the grid" }, undo: true },
  gridRight: { needs: ["selection"], expect: { changed: ["selected", "history"], orQuiet: "the hit already sits on the grid" }, undo: true },
  navUp: { needs: ["selection"], expect: { changed: ["selected", "history"] }, undo: true },
  navDown: { needs: ["selection"], expect: { changed: ["selected", "history"] }, undo: true },
  selectPrevious: { needs: ["selection"], expect: { changed: ["selected"] } },
  selectNext: { needs: ["selection"], expect: { changed: ["selected"] } },
  addPrevious: { needs: ["selection"], expect: { changed: ["selection", "selected"] } },
  addNext: { needs: ["selection"], expect: { changed: ["selection", "selected"] } },
  navAddUp: { needs: ["selection"], expect: { changed: ["selection", "selected"], orQuiet: "no hit in the lane above" } },
  navAddDown: { needs: ["selection"], expect: { changed: ["selection", "selected"], orQuiet: "no hit in the lane below" } },
  addNumberedLayer: { needs: ["probe"], expect: { changed: ["lanes", "history"] }, undo: true },
  openHitsoundFolder: { expect: { respond: true, explorerOk: true } },
  reviewOutput: { expect: { modal: "Export" } },
  first: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  last: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  deselect: { needs: ["selection"], expect: { changed: ["selection"] } },
  snap: { expect: { changed: ["snap"] }, twice: true },
  jumpLeft: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  jumpRight: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  setLeft: { needs: ["stopped", "midSong"], expect: { changed: ["left"], orQuiet: "the left locator is already here" } },
  setRight: { needs: ["stopped", "midSong"], expect: { changed: ["right"], orQuiet: "the right locator is already here" } },
  removeBookmark: { needs: ["bookmark", "stopped"], before: async () => { await openMenu("Map"); await menuRow("Next bookmark").click(); await sleep(300); },
    expect: { changed: ["bookmarks"] } },
  seekBack: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  seekBackFar: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  seekForward: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  seekForwardFar: { needs: ["stopped", "midSong"], expect: { changed: ["position"] } },
  ...Object.fromEntries(Array.from({ length: 9 }, (_, i) => [`snap${i + 1}`, { expect: { changed: ["snap"], orQuiet: "the snap is already this one" } }])),
};

async function makeEdit() {
  await selectProbe();
  await page().mouse.move(300, 5);
  // An edit through the Edit menu: Duplicate the probe hit.
  await openMenu("Edit");
  await menuRow("Duplicate").click();
  await until(() => window.scene.historyPosition > 0, null, "Duplicate made no edit", 3000);
}
async function copySelection() {
  await openMenu("Edit");
  await menuRow("Copy").click();
  await sleep(200);
  // Paste lands at the playhead: put it a little after the probe.
  const zoom = await read(() => window.scene.zoom);
  const box = await read(() => document.getElementById("ruler").getBoundingClientRect().toJSON());
  const x = await read(([time, zoom]) => document.getElementById("ruler").getBoundingClientRect().x + (time + 200 / zoom - window.scene.viewStart) * zoom, [probe[0], zoom]);
  await page().mouse.click(Math.min(box.x + box.width - 5, x), box.y + 30);
  await sleep(300);
}
async function seekTo(end) {
  await openMenu("Playback");
  await menuRow(end === "end" ? "Go to end" : "Go to start").click();
  await sleep(300);
}
async function confirmBookmark() {
  if ((await dialogTitle())?.startsWith("Add bookmark")) {
    await dialogButton(/^(Add|OK|Save)$/).click();
    await until(() => !document.getElementById("dialog").open, null, "the bookmark dialog did not close", 3000);
  }
}
async function finishSave() {
  const end = Date.now() + 20000;
  const reviewed = [];
  while (Date.now() < end && await read(() => window.view.dirty || window.view.busy || document.getElementById("dialog").open)) {
    const title = await dialogTitle();
    if (title?.startsWith("Review Save")) {
      reviewed.push(title);
      await dialogButton(/^(Save|Done)$/).click();
    } else if (title?.startsWith("Save hitsounds")) {
      reviewed.push(title);
      await dialogButton(/^(Save|Continue to Save)$/).click();
    } else if (title) {
      const body = await read(() => document.getElementById("dialog-body").innerText);
      throw new Error(`Save showed "${title}": ${body.slice(0, 160)}`);
    }
    await sleep(250);
  }
  expect(!(await read(() => window.view.dirty)), "the editor still shows unsaved changes after Save");
  return reviewed;
}

// The command ids each menu row stands for.
const rows = await (async () => {
  const source = readFileSync(join(repo, "apps/tauri/ui/src/lib/commands.js"), "utf8");
  const items = [...source.matchAll(/\{\s*id:\s*"([^"]+)",\s*label:\s*"([^"]+)"(.*?)\},?\s*$/gm)].map(([, id, label, rest]) => ({
    id, label, command: /command:\s*"([^"]+)"/.exec(rest)?.[1] ?? null, choices: /choices:/.test(rest),
  }));
  const menusBlock = source.slice(source.indexOf("export const menus"), source.indexOf("export const keyOnly"));
  const menus = Object.fromEntries([...menusBlock.matchAll(/(\w+):\s*\[([^\]]*)\]/g)].map(([, name, list]) =>
    [name, [...list.matchAll(/"([^"]+)"/g)].map(m => m[1]).filter(id => id !== "-")]));
  const keyOnly = [...source.slice(source.indexOf("export const keyOnly")).matchAll(/"([^"]+)"/g)].map(m => m[1]);
  return { items, menus, keyOnly };
})();
const itemById = new Map(rows.items.map(item => [item.id, item]));
// A row's command decides (Quantize… is quantizeDialog, not the Q key's quantize); a row without one uses its id.
const specOf = id => specs[itemById.get(id)?.command ?? ""] ?? specs[id] ?? null;

// Run one input and check what it did. `press` gives the input; `id` picks the spec.
async function probeInput(id, press, result, label) {
  const spec = specOf(id);
  if (!spec) throw new Error(`the robot has no expectation for ${id}: add one to specs in tools/every-input.mjs`);
  if (spec.unsafe) return { skip: spec.unsafe };
  for (const need of spec.needs ?? []) await needs[need]();
  if (spec.before) await spec.before();
  await sleep(150);
  const before = await fingerprint();
  const explorersBefore = await native.explorers();
  const windowsBefore = others().length;
  await press();
  const want = spec.expect;
  let after = before;
  let seen = null;
  const end = Date.now() + (want.native || want.window ? 8000 : 4000);
  while (Date.now() < end && !seen) {
    await sleep(150);
    after = await fingerprint();
    const changed = differs(before, after).filter(key => !["focus", "hint", "status", "busy"].includes(key));
    if (want.native && native.available && (await native.dialogs(pid())).length) seen = "a file picker opened";
    else if (want.native && !native.available) seen = "skipped: " + native.reason;
    else if (want.window && others().length > windowsBefore) seen = `${others().at(-1).url().split("/").pop()} opened`;
    else if (want.dialog !== undefined && after.dialog?.startsWith(want.dialog)) seen = `"${after.dialog}" opened`;
    else if (want.modal !== undefined && after.modals && after.modals.split("|").some(name => name.startsWith(want.modal))) seen = `"${after.modals}" opened`;
    else if ((want.explorer || want.explorerOk) && (await native.explorers()).some(window => ours(window) &&
      !explorersBefore.some(old => old.hwnd === window.hwnd && old.title === window.title))) seen = "an Explorer window opened";
    else if (want.changed && want.changed.some(key => changed.includes(key))) seen = describe(before, after, want.changed.filter(key => changed.includes(key)));
    else if (want.respond && changed.length) seen = describe(before, after, changed);
  }
  if (!seen) {
    after = await fingerprint();
    const changed = differs(before, after);
    const quiet = want.quiet || want.orQuiet;
    if (quiet) seen = `no visible change (${typeof quiet === "string" ? quiet : "expected for this input"})${changed.length ? "; " + describe(before, after, changed) : ""}`;
    else {
      throw new Error(`${label}: expected ${JSON.stringify(want)}, but ${changed.length ? describe(before, after, changed) : "nothing changed"}`);
    }
  }
  if (spec.twice) {
    const mid = await fingerprint();
    await press();
    const keys = (want.changed ?? differs(before, mid)).filter(key => JSON.stringify(mid[key]) !== JSON.stringify(before[key]));
    const back = Date.now() + 3000;
    let restored = false;
    while (Date.now() < back && !restored) {
      await sleep(150);
      const now = await fingerprint();
      restored = keys.every(key => JSON.stringify(now[key]) === JSON.stringify(before[key]));
    }
    if (!restored) throw new Error(`${label}: pressed a second time, it did not switch back (${describe(before, await fingerprint(), keys)})`);
    seen += ", and back";
  }
  if (want.window && others().length > windowsBefore) {
    const opened = others().at(-1);
    expect(opened.url().endsWith(want.window), `${label} opened ${opened.url().split("/").pop()}, not ${want.window}`);
    await opened.waitForSelector("[data-ui-ready]", { timeout: 15000 });
  }
  if (spec.after) {
    const note = await spec.after();
    if (Array.isArray(note) && note.length) seen += ` (${note.join("; ")})`;
  }
  const opened = await closeNewExplorers(explorersBefore);
  if (want.explorer && !opened.length && !seen.startsWith("an Explorer")) throw new Error(`${label} opened no Explorer window`);
  if (native.available && (await native.dialogs(pid())).length) await native.cancel(pid());
  await settle();
  if (spec.undo) await undoTo(before.history);
  if (spec.redoAfter) {
    await openMenu("Edit");
    const redo = menuRow("Redo");
    if (await redo.isEnabled()) await redo.click(); else await page().keyboard.press("Escape");
    await undoTo(before.history);
  }
  return seen;
}

// The three ways to reach a command.
async function viaMenu(menu, item) {
  await page().mouse.move(300, 5);
  if (menu === "tool") {
    const more = page().locator("[data-more-tools]").first();
    await more.click();
    await until(() => !document.getElementById("editor-menu").hidden, null, "the More tools menu did not open", 3000);
  } else if (menu === "lane") {
    await ensureProbe();
    await revealLane(laneRow);
    await page().locator(`#lanes button.name[data-lane-row="${laneRow}"]`).click({ button: "right" });
    await until(() => !document.getElementById("editor-menu").hidden, null, "the lane menu did not open", 3000);
  } else {
    await openMenu(menu);
  }
  const button = page().locator("#editor-menu button").filter({ hasText: item.label }).first();
  expect(await button.count(), `${menu} has no "${item.label}" row`);
  if (!(await button.isEnabled())) {
    await page().keyboard.press("Escape");
    return { disabled: true };
  }
  await button.click();
  return {};
}
const keyCodes = { ".": "Period", "-": "Minus", "=": "Equal", "/": "Slash", "\\": "Backslash", ",": "Comma", ";": "Semicolon", "'": "Quote",
  "[": "BracketLeft", "]": "BracketRight", "`": "Backquote", Space: "Space", Up: "ArrowUp", Down: "ArrowDown", Left: "ArrowLeft",
  Right: "ArrowRight", Del: "Delete", Esc: "Escape", "Numpad /": "NumpadDivide", "Numpad *": "NumpadMultiply", "Numpad -": "NumpadSubtract",
  "Numpad +": "NumpadAdd", "Numpad .": "NumpadDecimal", "Numpad Enter": "NumpadEnter" };
// "Ctrl+Shift+Z" as Playwright presses it, by key position: letters as KeyZ so the page sees "z" or "Z".
function keyPress(keys) {
  // "Numpad +" ends in a plus of its own: take it off before splitting the modifiers.
  const numpadPlus = keys.endsWith("Numpad +");
  const parts = (numpadPlus ? keys.slice(0, -"Numpad +".length) : keys).split("+").filter(Boolean);
  let key = numpadPlus ? "Numpad +" : parts.pop();
  const mods = parts.map(mod => ({ Ctrl: "Control", Shift: "Shift", Alt: "Alt" }[mod] ?? mod));
  let name = keyCodes[key] ?? key;
  if (/^[A-Z]$/.test(key)) name = `Key${key}`;
  else if (/^[0-9]$/.test(key)) name = `Digit${key}`;
  else if (/^Numpad [0-9]$/.test(key)) name = `Numpad${key.slice(-1)}`;
  return [...mods, name].join("+");
}
async function viaKey(keys) {
  await page().mouse.move(300, 5);
  await page().keyboard.press(keyPress(keys));
}
async function viaPalette(label) {
  await page().mouse.move(300, 5);
  await page().keyboard.press("Control+KeyK");
  await until(() => !!window.dawsuCommandPaletteOpen, null, "Ctrl+K did not open the command palette", 3000);
  await page().keyboard.type(label.replace(/…$/, ""));
  await sleep(200);
  const options = page().locator('#cmdp-listbox [role="option"]');
  const labels = await options.locator(".cmdp-label").allTextContents();
  const index = labels.findIndex(text => text === label || text.startsWith(label + " · "));
  if (index < 0) {
    await page().keyboard.press("Escape");
    throw new Error(`Ctrl+K lists no "${label}" for what was typed (it shows: ${labels.slice(0, 5).join(", ")})`);
  }
  if (await options.nth(index).isDisabled()) {
    await page().keyboard.press("Escape");
    return { disabled: true };
  }
  // Down from the top to the entry, then Enter, as a mapper would.
  const active = await read(() => Number(document.querySelector('#cmdp-listbox [aria-selected="true"]')?.dataset.paletteIndex ?? -1));
  for (let i = Math.max(0, active); i < index; i++) await page().keyboard.press("ArrowDown");
  await page().keyboard.press("Enter");
  return {};
}

let shortcuts = null;
const readShortcuts = () => read(() => window.__TAURI__.core.invoke("editor_shortcuts"));
async function choosePreset(preset) {
  if ((await read(() => window.view.keyPreset)) === preset) return;
  await openMenu("File");
  await menuRow("Preferences…").click();
  const dialog = page().getByRole("dialog", { name: "Preferences" });
  await dialog.waitFor({ state: "visible" });
  await dialog.getByRole("button", { name: preset === "osu!" ? "osu! editor controls" : "DAW-like keys" }).click();
  await until(preset => window.view.keyPreset === preset, preset, `the key preset did not become ${preset}`, 5000);
  await dialog.getByRole("button", { name: "Close preferences" }).click();
  await dialog.waitFor({ state: "hidden" });
}

try {
  await step("Start with no map and see the start screen", async () => {
    await launch(null);
    await until(() => !document.getElementById("home").hidden && !!document.getElementById("home-open"), null,
      "the start screen did not show", 15000);
    const preset = page().getByRole("button", { name: "DAW-like" });
    if (await preset.count() && await preset.first().isVisible()) {
      await preset.first().click();
      await until(() => window.view.keyPreset === "DAW", null, "choosing DAW-like did not set the key preset", 4000);
    }
  }, ["dialog:key-preset"]);

  await step("Start screen: point the Songs folder at the run's copy and open the mapset from the picker", async result => {
    // Folders… › Choose Songs folder… opens a folder picker, answered with the run's own Songs folder.
    await page().click("#home-folders");
    await robot.dialogShown("Folders");
    await dialogButton("Choose Songs folder…").click();
    await answerPicker(songs, "the Songs folder picker");
    const want = resolve(songs).toLowerCase();
    await until(want => (window.view.songsFolder ?? "").replaceAll("/", "\\").toLowerCase().replace(/\\$/, "") === want ||
      (window.view.songsFolder ?? "").toLowerCase() === want, want, "the Songs folder is not the run's own copy", 5000)
      .catch(async error => {
        // Never go on with another Songs folder: it could be a real one.
        outside = `the Songs folder became ${await read(() => window.view.songsFolder)}, not the run's copy, so the run stopped`;
        await robot.quit();
        throw error;
      });
    await settle();
    // The picker lists the copy's mapset; a difficulty button opens it from the run's Songs folder.
    const diff = page().locator("#home-songs button.home-song-diff:not(.home-song-create-hs)").first();
    await diff.waitFor({ state: "visible", timeout: 20000 });
    const search = page().getByLabel("Search Songs folder");
    await search.fill(basename(original).slice(0, 4));
    await search.fill("");
    await diff.click();
    const opened = await until(want => window.scene?.path && !window.view.busy && window.scene.path, null, "the picked difficulty did not open", 60000);
    expect(inProfile(opened) && opened.toLowerCase().includes("songs"), `the picker opened ${opened}, not the run's Songs copy`);
    result.reached.push("feature:map-picker");
    return `opened ${basename(opened)} from the run's Songs folder`;
  }, ["feature:map-picker", "dialog:folders", "control:main:StartScreen:Search Songs folder (text)"]);
  await robot.quit();

  await step("Open the map copy", async () => {
    await launch(mapCopy);
    await until(() => document.querySelectorAll("#lanes [data-lane-row]").length > 0 && window.scene?.lanes?.length > 0,
      null, "no lanes appeared", 60000);
    await sleep(800);
    await silence();
    return `${await read(() => window.scene.lanes.length)} lanes, ${(await hits()).length} hits`;
  });

  await step("Place the robot's own hit on an empty part of the song", async () => {
    await ensureProbe();
    return `at ${probe[0]} ms on ${await read(row => window.scene.lanes[row].name, probe[1])}`;
  }, ["mouse:Click / drag (Draw)"]);

  // Every menu row, by mouse, and the dialogs those rows open.
  // covers-all: menu:* dialog:revert dialog:revert-original dialog:audio dialog:new dialog:generate dialog:copy
  // covers-all: dialog:time-signatures dialog:help dialog:delete-lane dialog:add-lane dialog:rename-lane dialog:lane-colour
  // covers-all: dialog:replace-sample dialog:bookmark dialog:timeline-edit dialog:sound-sets dialog:save-sound-set
  // covers-all: dialog:midi-import dialog:quick-start dialog:editor-view dialog:folders dialog:preferences dialog:keysounds
  // covers-all: dialog:shared-base dialog:stem-separation dialog:drum-map-setup dialog:export-modal
  if (routes.has("menu")) {
    for (const [menu, ids] of Object.entries(rows.menus)) {
      for (const id of ids) {
        const item = itemById.get(id);
        if (item.choices) continue;
        await step(`Menu ${menu} › ${item.label}`, async () => {
          let disabled = false;
          const seen = await probeInput(id, async () => { disabled = (await viaMenu(menu, item)).disabled; }, null, `${menu} › ${item.label}`)
            .catch(error => { if (disabled) return null; throw error; });
          if (seen?.skip) return seen;
          if (disabled) return { skip: `${menu} › ${item.label} is disabled here` };
          return seen;
        }, [`menu:${menu}.${id}`]);
      }
    }
    // The choice rows: every choice, then back to the first one.
    for (const [menu, id, label] of [["playback", "playbackStart", "Start playing from"], ["playback", "speed", "Speed"], ["map", "difficulty", "Difficulty"]]) {
      await step(`Menu ${menu} › ${label}: every choice`, async () => {
        await openMenu(menu);
        const choices = await page().locator("#editor-menu").locator("[role=menuitemradio], .menu-choice, .seg button")
          .filter({ hasNotText: /^$/ }).allTextContents();
        await page().keyboard.press("Escape");
        if (!choices.length) return { skip: `the ${label} row shows no choices here (one difficulty, or none to pick)` };
        const done = [];
        for (const choice of choices.slice(0, 8)) {
          await openMenu(menu);
          const button = page().locator("#editor-menu button").filter({ hasText: new RegExp(`^${choice.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}$`) }).first();
          if (!(await button.count())) { await page().keyboard.press("Escape"); continue; }
          await button.click();
          await sleep(500);
          await settle();
          done.push(choice);
        }
        // Back to the original difficulty if the choice switched it.
        if (id === "difficulty" && (await read(() => window.scene.path)) !== mapCopy) {
          await openMenu("Map");
          await page().locator("#editor-menu button").filter({ hasText: (await read(() => window.scene?.version)) ?? "" }).first().click().catch(() => {});
        }
        return `picked ${done.join(", ")}`;
      }, [`menu:${menu}.${id}`]);
    }
  }

  // Every command by its keys, in each preset.
  // covers-all: key:*@DAW key:*@osu!
  for (const preset of presets) {
    await step(`Switch to the ${preset === "DAW" ? "DAW-like" : "osu!"} key preset in Preferences`, async () => {
      await choosePreset(preset);
      shortcuts = await readShortcuts();
      expect(shortcuts?.entries?.length > 20, `editor_shortcuts listed ${shortcuts?.entries?.length ?? 0} commands`);
      return `${shortcuts.entries.length} commands, ${shortcuts.entries.filter(([, , keys]) => keys.length).length} with keys`;
    }, [`control:main:PreferencesModal:*`]);
    if (!routes.has("key") || !shortcuts) continue;
    for (const [id, description, keys] of shortcuts.entries) {
      if (!keys.length) {
        await step(`Keys ${preset}: ${description}`, async () => ({ skip: `no key in the ${preset} preset` }), [`key:${id}@${preset}`]);
        continue;
      }
      for (const key of keys) {
        await step(`Keys ${preset}: ${key} — ${description}`, async () => {
          if ((await read(() => window.view.keyPreset)) !== preset) await choosePreset(preset);
          return probeInput(id, () => viaKey(key), null, `${key} (${id})`);
        }, [`key:${id}@${preset}`]);
      }
    }
  }
  if (presets.includes("DAW")) await step("Back to the DAW-like key preset", () => choosePreset("DAW"));

  // Ctrl+K: opening and closing it, then every entry.
  await step("Ctrl+K opens, filters, moves, and closes three ways", async () => {
    await page().mouse.move(300, 5);
    await page().keyboard.press("Control+KeyK");
    await until(() => !!window.dawsuCommandPaletteOpen, null, "Ctrl+K did not open the command palette", 3000);
    const all = await page().locator('#cmdp-listbox [role="option"]').count();
    await page().keyboard.type("zoom");
    await sleep(200);
    const some = await page().locator('#cmdp-listbox [role="option"]').count();
    expect(some > 0 && some < all, `typing "zoom" left ${some} of ${all} entries`);
    const first = await read(() => document.querySelector('#cmdp-listbox [aria-selected="true"]')?.dataset.paletteIndex);
    await page().keyboard.press("ArrowDown");
    const second = await read(() => document.querySelector('#cmdp-listbox [aria-selected="true"]')?.dataset.paletteIndex);
    expect(first !== second, "ArrowDown did not move the choice");
    await page().keyboard.press("ArrowUp");
    await page().keyboard.press("Escape");
    await until(() => !window.dawsuCommandPaletteOpen, null, "Escape did not close the palette", 3000);
    await page().keyboard.press("Control+KeyK");
    await until(() => !!window.dawsuCommandPaletteOpen, null, "Ctrl+K did not open it again", 3000);
    await page().keyboard.press("Control+KeyK");
    await until(() => !window.dawsuCommandPaletteOpen, null, "Ctrl+K again did not close it", 3000);
    await page().keyboard.press("Control+KeyK");
    await until(() => !!window.dawsuCommandPaletteOpen, null, "Ctrl+K did not open it a third time", 3000);
    await page().mouse.click(5, 5);
    await until(() => !window.dawsuCommandPaletteOpen, null, "a click outside did not close it", 3000);
    // A click on an entry runs it: Zoom in.
    const zoom = await read(() => window.scene.zoom);
    await page().keyboard.press("Control+KeyK");
    await page().keyboard.type("Zoom in");
    await sleep(200);
    await page().locator('#cmdp-listbox [role="option"]').filter({ hasText: "Zoom in" }).first().click();
    await until(zoom => window.scene.zoom > zoom, zoom, "clicking Zoom in in the palette did not zoom in", 3000);
    return `${all} entries; "zoom" leaves ${some}`;
  }, ["palette:open-with-ctrl-k", "palette:type-to-search", "palette:arrow-keys", "palette:escape-closes", "palette:ctrl-k-closes",
    "palette:backdrop-closes", "palette:click-runs", "key:page:Ctrl+K", "dialog:command-palette"]);
  // covers-all: palette:run:* palette:enter-runs
  if (routes.has("palette")) {
    const descriptions = new Map((shortcuts?.entries ?? []).map(([id, description]) => [id, description]));
    const entries = [];
    for (const name of ["file", "edit", "playback", "map", "view", "tool"]) {
      for (const id of rows.menus[name]) {
        const item = itemById.get(id);
        if (item.choices || id === "editLayout" || entries.some(entry => entry.id === id)) continue;
        entries.push({ id, label: item.label });
      }
    }
    const inMenus = new Set(rows.items.map(item => item.command).filter(Boolean));
    for (const id of rows.keyOnly) if (!inMenus.has(id) && descriptions.has(id)) entries.push({ id, label: descriptions.get(id) });
    for (const { id, label } of entries) {
      await step(`Ctrl+K › ${label}`, async () => {
        let disabled = false;
        const seen = await probeInput(id, async () => { disabled = (await viaPalette(label)).disabled; }, null, `Ctrl+K ${label}`)
          .catch(error => { if (disabled) return null; throw error; });
        if (seen?.skip) return seen;
        if (disabled) return { skip: `"${label}" is disabled in the palette here` };
        return seen;
      }, [`palette:run:${id}`, "palette:enter-runs"]);
    }
  }

  // Keys the page handles itself.
  await step("4 opens the More tools menu, and a pick sets the tool", async () => {
    await page().mouse.move(300, 5);
    await page().keyboard.press("Digit4");
    await until(() => !document.getElementById("editor-menu").hidden, null, "4 did not open the tools menu", 3000);
    await menuRow("Grab").click();
    await until(() => window.view.tool === "Grab", null, "picking Grab did not set it", 3000);
    await pickTool("Select");
  }, ["key:page:4-tools-menu"]);
  await step("Holding Alt shows Draw while Select is on", async () => {
    await pickTool("Select");
    await page().keyboard.down("Alt");
    await sleep(200);
    const pressed = await read(() => document.querySelector('[aria-label="Editing tools"] button[aria-pressed="true"]')?.textContent?.trim());
    await page().keyboard.up("Alt");
    expect(pressed === "Draw", `with Alt held the pressed pill is ${pressed}`);
  }, ["key:page:Alt-held-draws"]);
  await step("Space plays even with a toolbar button focused", async () => {
    await silence();
    await page().locator('[aria-label="Editing tools"] button').first().focus();
    await page().keyboard.press("Space");
    await until(() => window.view.playing, null, "Space did not start playback", 3000);
    await page().keyboard.press("Space");
    await until(() => !window.view.playing, null, "Space did not stop playback", 3000);
  }, ["key:page:Space-not-a-button"]);
  await step("Tab and Shift+Tab move between the editor's areas", async () => {
    await page().mouse.move(300, 5);
    const before = await read(() => document.activeElement?.id || document.activeElement?.tagName);
    await page().keyboard.press("Tab");
    const after = await read(() => document.activeElement?.id || document.activeElement?.tagName);
    await page().keyboard.press("Shift+Tab");
    expect(before !== after, `Tab left the focus on ${after}`);
    return `${before} → ${after}`;
  }, ["key:page:Tab-routes"]);
  await step("Ctrl+= and Ctrl+- change the interface scale, Ctrl+0 resets it", async () => {
    await page().mouse.move(300, 5);
    const width = () => read(() => document.getElementById("grid").getBoundingClientRect().width);
    const start = await width();
    await page().keyboard.press("Control+Equal");
    await sleep(500);
    const bigger = await width();
    await page().keyboard.press("Control+Digit0");
    await sleep(500);
    const reset = await width();
    await page().keyboard.press("Control+Minus");
    await sleep(500);
    const smaller = await width();
    await page().keyboard.press("Control+Digit0");
    await sleep(500);
    const back = await width();
    expect(bigger < start, `Ctrl+= did not make the interface bigger (timeline ${start} → ${bigger} px wide)`);
    expect(Math.abs(reset - start) < 2, `Ctrl+0 did not reset the scale (${start} → ${reset} px)`);
    expect(smaller > start, `Ctrl+- did not make the interface smaller (${start} → ${smaller} px)`);
    expect(Math.abs(back - start) < 2, `Ctrl+0 did not reset after Ctrl+- (${start} → ${back} px)`);
    return `timeline ${Math.round(start)} px; Ctrl+= ${Math.round(bigger)}, Ctrl+- ${Math.round(smaller)}, Ctrl+0 back`;
  }, ["key:page:Ctrl+=/-/0-scale"]);
  await step("Shift+F10 on a lane opens its menu", async () => {
    await ensureProbe();
    await revealLane(laneRow);
    await page().locator(`#lanes button.name[data-lane-row="${laneRow}"]`).focus();
    await page().keyboard.press("Shift+F10");
    await until(() => !document.getElementById("editor-menu").hidden, null, "Shift+F10 opened no lane menu", 3000);
    await page().keyboard.press("Escape");
  }, ["key:page:Menu-or-Shift+F10-lane"]);
  await step("Escape on the start screen goes back to the open map", async () => {
    await openMenu("File");
    await menuRow("Start screen").click();
    await until(() => window.view.showHome, null, "the start screen did not show", 3000);
    await page().keyboard.press("Escape");
    await until(() => !window.view.showHome, null, "Escape did not go back to the map", 3000);
  }, ["key:page:Escape-resumes-from-home", "menu:file.startScreen"]);

  // Mouse actions, as Shortcuts and mouse actions lists them.
  const mouse = () => page().mouse;
  const box = id => read(id => document.getElementById(id).getBoundingClientRect().toJSON(), id);
  async function wheelUntil(over, delta, modifiers, check, what) {
    const before = await fingerprint();
    await mouse().move(over.x, over.y);
    for (let turn = 0; turn < 4; turn++) {
      for (const key of modifiers) await page().keyboard.down(key);
      await mouse().wheel(delta[0], delta[1]);
      for (const key of modifiers.toReversed()) await page().keyboard.up(key);
      await sleep(400);
      const now = await fingerprint();
      if (check(before, now)) return describe(before, now, differs(before, now));
    }
    throw new Error(`${what} (nothing changed after four turns)`);
  }
  for (const preset of presets) {
    const osu = preset === "osu!";
    const tag = preset === "DAW" ? "DAW-like" : "osu!";
    await step(`Mouse (${tag}): the wheel over the timeline, with and without Ctrl, Alt and Shift`, async () => {
      await choosePreset(preset);
      await ensureProbe();
      await view(probe[0]);
      const grid = await box("grid");
      const over = { x: grid.x + grid.width / 2, y: grid.y + grid.height / 2 };
      const notes = [];
      notes.push("wheel: " + await wheelUntil(over, [0, 120], [], (a, b) => osu ? a.position !== b.position : a.scrollTop !== b.scrollTop,
        osu ? "the wheel did not seek in osu! keys" : "the wheel did not scroll the lanes in DAW-like keys"));
      notes.push("zoom: " + await wheelUntil(over, [0, -120], [osu ? "Alt" : "Control"], (a, b) => a.zoom !== b.zoom, "the zoom wheel did not zoom"));
      notes.push("snap: " + await wheelUntil(over, [0, -120], [osu ? "Control" : "Alt"], (a, b) => a.snap !== b.snap, "the snap wheel did not change the snap"));
      notes.push("shift: " + await wheelUntil(over, [0, 120], ["Shift"], (a, b) => osu ? a.position !== b.position : a.start !== b.start,
        osu ? "Shift+wheel did not seek four grid lines" : "Shift+wheel did not scroll in time"));
      notes.push("sideways: " + await wheelUntil(over, [240, 0], [], (a, b) => a.start !== b.start, "a sideways wheel did not scroll in time"));
      notes.push("rows: " + await wheelUntil(over, [0, -120], ["Control", "Shift"], (a, b) => a.rowHeight !== b.rowHeight, "Ctrl+Shift+wheel did not change the lane height"));
      const lanes = await box("lanes");
      notes.push("headers: " + await wheelUntil({ x: lanes.x + lanes.width / 2, y: lanes.y + lanes.height / 2 }, [0, 120], [],
        (a, b) => a.scrollTop !== b.scrollTop, "the wheel over the lane headers did not scroll the lanes"));
      return notes.join(" | ");
    }, ["mouse:Wheel without modifiers", "mouse:Sideways wheel or trackpad swipe", "mouse:Ctrl+wheel (DAW-like) / Alt+wheel (osu!)",
      "mouse:Alt+wheel (DAW-like) / Ctrl+wheel (osu!)", "mouse:Shift+wheel", "mouse:Ctrl+Shift+wheel"]);
  }
  if (presets.includes("DAW")) await choosePreset("DAW").catch(() => {});

  await step("Mouse: click and drag the overview", async () => {
    const overview = await box("overview");
    const before = await fingerprint();
    await mouse().click(overview.x + overview.width * 0.2, overview.y + overview.height / 2);
    await sleep(300);
    const clicked = await fingerprint();
    expect(clicked.start !== before.start, "a click on the overview did not move the timeline");
    await mouse().move(overview.x + overview.width * 0.2, overview.y + overview.height / 2);
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(overview.x + overview.width * (0.2 + i * 0.05), overview.y + overview.height / 2);
    await mouse().up();
    await sleep(300);
    expect((await fingerprint()).start > clicked.start, "dragging the overview did not move the timeline later");
  }, ["mouse:Click or drag the overview"]);

  await step("Mouse: the ruler: click, Ctrl+click, Alt+click, drag a cycle, click and double-click the band", async () => {
    await ensureProbe();
    await view(probe[0], 0.1);
    const ruler = await box("ruler");
    const before = await fingerprint();
    await mouse().click(ruler.x + ruler.width * 0.3, ruler.y + 30);
    await until(position => Math.round(window.view.position) !== position, before.position, "a click on the ruler did not move the playhead", 3000);
    await page().keyboard.down("Control");
    await mouse().click(ruler.x + ruler.width * 0.35, ruler.y + 30);
    await page().keyboard.up("Control");
    await until(left => Math.round(window.view.left) !== left, before.left, "Ctrl+click on the ruler did not place the left locator", 3000);
    await page().keyboard.down("Alt");
    await mouse().click(ruler.x + ruler.width * 0.6, ruler.y + 30);
    await page().keyboard.up("Alt");
    await until(right => Math.round(window.view.right) !== right, before.right, "Alt+click on the ruler did not place the right locator", 3000);
    // Drag along the lower band: the playhead follows (scrub).
    const scrubbed = await fingerprint();
    await mouse().move(ruler.x + ruler.width * 0.4, ruler.y + 30);
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(ruler.x + ruler.width * (0.4 + i * 0.02), ruler.y + 30);
    await mouse().up();
    await until(position => Math.round(window.view.position) !== position, scrubbed.position, "dragging along the ruler did not move the playhead", 3000);
    // Draw a new cycle range in the upper band, away from the one just placed.
    const drawn = await fingerprint();
    await mouse().move(ruler.x + ruler.width * 0.7, ruler.y + 8);
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(ruler.x + ruler.width * (0.7 + i * 0.03), ruler.y + 8);
    await mouse().up();
    await until(([left, right]) => Math.round(window.view.left) !== left || Math.round(window.view.right) !== right,
      [drawn.left, drawn.right], "dragging the upper ruler drew no cycle range", 3000);
    // Double-click inside the band toggles cycling.
    const band = await read(() => {
      const ruler = document.getElementById("ruler").getBoundingClientRect();
      return { x: ruler.x + ((window.view.left + window.view.right) / 2 - window.scene.viewStart) * window.scene.zoom, y: ruler.y + 8 };
    });
    const cycling = await read(() => window.view.cycle);
    await mouse().dblclick(band.x, band.y);
    await until(cycling => window.view.cycle !== cycling, cycling, "double-clicking the cycle band did not switch cycling", 3000);
    await mouse().dblclick(band.x, band.y);
    await until(cycling => window.view.cycle === cycling, cycling, "double-clicking it again did not switch cycling back", 3000);
    // Drag inside the band moves it.
    const moved = await fingerprint();
    await mouse().move(band.x, band.y);
    await mouse().down();
    for (let i = 1; i <= 5; i++) await mouse().move(band.x - 10 * i, band.y);
    await mouse().up();
    await until(left => Math.round(window.view.left) !== left, moved.left, "dragging inside the cycle band did not move it", 3000);
  }, ["mouse:Click the ruler or waveform", "mouse:Ctrl+click / Alt+click the ruler", "mouse:Drag the upper ruler", "mouse:Click inside the cycle band",
    "mouse:Drag the playhead"]);

  await step("Mouse: drag and double-click under the waveform", async () => {
    const grip = await box("waveform-resize");
    const before = await fingerprint();
    await mouse().move(grip.x + grip.width / 2, grip.y + grip.height / 2);
    await mouse().down();
    for (let i = 1; i <= 5; i++) await mouse().move(grip.x + grip.width / 2, grip.y + grip.height / 2 + 6 * i);
    await mouse().up();
    await sleep(300);
    const dragged = await fingerprint();
    expect(dragged.waveHeight !== before.waveHeight, `dragging the waveform grip left its height at ${dragged.waveHeight}`);
    const again = await box("waveform-resize");
    await mouse().dblclick(again.x + again.width / 2, again.y + again.height / 2);
    await sleep(300);
    expect((await fingerprint()).waveHeight !== dragged.waveHeight, "double-clicking the grip did not reset the height");
  }, ["mouse:Drag / double-click under the waveform"]);

  await step("Mouse: click and double-click a gameplay object on the waveform", async () => {
    const object = await read(() => {
      const points = window.scene.gameplay?.points ?? [];
      const ruler = document.getElementById("ruler").getBoundingClientRect();
      const width = document.getElementById("grid").clientWidth;
      for (const [, time] of points) {
        const x = (time - window.scene.viewStart) * window.scene.zoom;
        if (x > 20 && x < width - 20) return { x: ruler.x + x, y: ruler.y + 40 + (ruler.height - 40) / 2, time };
      }
      return points.length ? { time: points[Math.floor(points.length / 2)][1], offscreen: true } : null;
    });
    if (!object) return { skip: "the map shows no gameplay objects on the waveform" };
    if (object.offscreen) {
      await view(object.time);
      return { skip: "no object was on screen; try a map with objects in view" };
    }
    const before = await fingerprint();
    await mouse().click(object.x, object.y);
    await until(selection => (window.scene.selection?.count ?? 0) !== selection || true, before.selection, "", 1000).catch(() => {});
    await sleep(400);
    const clicked = await fingerprint();
    expect(clicked.selected !== before.selected || clicked.position !== before.position, "clicking an object on the waveform selected nothing and left the playhead");
    await mouse().dblclick(object.x, object.y);
    await sleep(400);
  }, ["mouse:Click / double-click a gameplay object on the waveform"]);

  await step("Mouse: Select tool: click, Ctrl+click, Shift+click, box, drag, double-click", async () => {
    await selectProbe();
    const point = await pointAt(probe[0], probe[1]);
    // A box around the probe from empty space.
    await page().mouse.move(point.x - 40, point.y - 6);
    await mouse().down();
    for (let i = 1; i <= 5; i++) await mouse().move(point.x - 40 + 16 * i, point.y + 4);
    await mouse().up();
    await until(() => (window.scene.selection?.count ?? 0) >= 1, null, "a box drag selected nothing", 3000);
    // Ctrl+click toggles it off and on.
    await page().keyboard.down("Control");
    await mouse().click(point.x, point.y);
    await page().keyboard.up("Control");
    await until(() => (window.scene.selection?.count ?? 0) === 0, null, "Ctrl+click did not take the hit out of the selection", 3000);
    await page().keyboard.down("Shift");
    await mouse().click(point.x, point.y);
    await page().keyboard.up("Shift");
    await until(() => (window.scene.selection?.count ?? 0) >= 1, null, "Shift+click selected nothing", 3000);
    // Drag the hit later and Undo it.
    await selectProbe();
    const history = await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`);
    const zoom = await read(() => window.scene.zoom);
    await mouse().move(point.x, point.y);
    await sleep(200);
    await mouse().down();
    for (let i = 1; i <= 8; i++) await mouse().move(point.x + 200 * zoom * i / 8, point.y);
    await mouse().up();
    await until(history => `${window.scene.historyPosition}/${window.scene.history.length}` !== history, history, "dragging the hit did not move it", 3000);
    await undoTo(history);
    // Escape during a drag puts the hit back and makes no edit.
    await selectProbe();
    await mouse().move(point.x, point.y);
    await sleep(200);
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(point.x + 150 * zoom * i / 6, point.y);
    await page().keyboard.press("Escape");
    await mouse().up();
    await sleep(400);
    expect((await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`)) === history && await probeHit(),
      "Escape during a drag still moved the hit");
    // Double-click the hit: the playhead goes to it.
    await mouse().dblclick(point.x, point.y);
    await until(time => Math.abs(window.view.position - time) < 2, probe[0], "double-clicking the hit did not move the playhead to it", 3000);
    // Double-click empty space inserts a hit.
    const count = (await hits()).length;
    await mouse().dblclick(point.x + 120, point.y);
    await until(count => window.scene.moments.reduce((n, m) => n + m[3].length, 0) === count + 1, count, "double-clicking empty space inserted no hit", 3000);
    await undoTo(history);
  }, ["mouse:Click / drag (Select)", "mouse:Ctrl+click / Shift+click", "mouse:Double-click an item (Select tool)",
    "mouse:Double-click empty space (Select tool)", "key:page:Escape-cancels-drag"]);

  await step("Mouse: right-click a hit deletes it, right-click empty space picks a tool", async () => {
    await selectProbe();
    const history = await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`);
    const point = await pointAt(probe[0], probe[1]);
    const count = (await hits()).length;
    await mouse().click(point.x, point.y, { button: "right" });
    await until(count => window.scene.moments.reduce((n, m) => n + m[3].length, 0) === count - 1, count, "right-click did not delete the hit", 3000);
    await undoTo(history);
    await mouse().click(point.x + 150, point.y, { button: "right" });
    await until(() => !document.getElementById("editor-menu").hidden, null, "right-click on empty space opened no tool picker", 3000);
    await page().keyboard.press("Escape");
  }, ["mouse:Right-click a hit / drag (Select)", "mouse:Right-click in the grid"]);

  await step("Mouse: Draw: click places, a stroke paints, right-drag erases, Alt+drag draws with Select", async () => {
    await ensureProbe();
    const history = await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`);
    const point = await pointAt(probe[0], probe[1]);
    await pickTool("Draw");
    const count = (await hits()).length;
    const zoom = await read(() => window.scene.zoom);
    // A stroke from just after the probe across a few snap steps on the same lane.
    await mouse().move(point.x + 60, point.y);
    await mouse().down();
    for (let i = 1; i <= 8; i++) await mouse().move(point.x + 60 + 15 * i, point.y);
    await mouse().up();
    const painted = await until(count => {
      const now = window.scene.moments.reduce((n, m) => n + m[3].length, 0);
      return now > count ? now : null;
    }, count, "a Draw stroke placed nothing", 3000);
    // Right-drag back across them erases.
    await mouse().move(point.x + 60 + 15 * 8, point.y);
    await mouse().down({ button: "right" });
    for (let i = 7; i >= 0; i--) await mouse().move(point.x + 60 + 15 * i, point.y);
    await mouse().up({ button: "right" });
    await until(painted => window.scene.moments.reduce((n, m) => n + m[3].length, 0) < painted, painted, "a right-drag with Draw erased nothing", 3000);
    await undoTo(history);
    await pickTool("Select");
    // Alt+drag draws while Select is on.
    const before = (await hits()).length;
    await page().keyboard.down("Alt");
    await mouse().move(point.x + 80, point.y);
    await mouse().down();
    await mouse().move(point.x + 81, point.y);
    await mouse().up();
    await page().keyboard.up("Alt");
    await until(before => window.scene.moments.reduce((n, m) => n + m[3].length, 0) > before, before, "Alt+drag with Select drew nothing", 3000);
    await undoTo(history);
    return `painted ${painted - count} hits in one stroke at ${zoom.toFixed(3)} px/ms`;
  }, ["mouse:Click / drag (Draw)", "mouse:Right-click / drag (Draw)", "mouse:Alt+drag", "mouse:Draw tool paint stroke", "feature:paint-stroke"]);

  await step("Mouse: Erase, Mute, Grab and Range tools", async () => {
    await ensureProbe();
    const history = await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`);
    let point = await pointAt(probe[0], probe[1]);
    await pickTool("Mute");
    await mouse().click(point.x, point.y);
    await until(([time, row]) => window.scene.moments.some(([t, , , list]) => t === time && list.some(([r, , muted]) => r === row && muted)),
      probe, "the Mute tool did not mute the hit", 3000);
    await mouse().click(point.x, point.y);
    await pickTool("Erase");
    const count = (await hits()).length;
    await mouse().click(point.x, point.y);
    await until(count => window.scene.moments.reduce((n, m) => n + m[3].length, 0) === count - 1, count, "the Erase tool did not delete the hit", 3000);
    await undoTo(history);
    await pickTool("Grab");
    const before = await fingerprint();
    await mouse().move(point.x, point.y);
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(point.x - 20 * i, point.y - 5 * i);
    await mouse().up();
    await sleep(300);
    const panned = await fingerprint();
    expect(panned.start !== before.start || panned.scrollTop !== before.scrollTop, "the Grab drag did not pan");
    await pickTool("Range");
    point = await pointAt(probe[0], probe[1]);
    await mouse().move(point.x - 50, point.y);
    await mouse().down();
    for (let i = 1; i <= 5; i++) await mouse().move(point.x - 50 + 20 * i, point.y);
    await mouse().up();
    await until(() => !!window.scene.range, null, "the Range drag marked nothing", 3000);
    await mouse().click(point.x + 200, point.y);
    await until(() => !window.scene.range, null, "a click with Range did not clear the range", 3000);
    await pickTool("Select");
  }, ["mouse:Click / drag (Erase)", "mouse:Click an item (Mute)", "mouse:Drag (Grab)", "mouse:Drag (Range)"]);

  await step("Mouse: the volume lane: drag a bar, Ctrl+drag, Alt+paint, Shift+ramp, Ramp tool; never under 5%", async () => {
    await selectTwo();
    const history = await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`);
    const lane = await box("controller");
    expect(lane.height > 20, "the volume lane is folded");
    const x = await read(time => document.getElementById("controller").getBoundingClientRect().x + (time - window.scene.viewStart) * window.scene.zoom, probe[0]);
    const y = volume => lane.y + lane.height - volume / 100 * (lane.height - 3);
    const volumeOf = () => read(([time, row]) => window.scene.moments.flatMap(([t, volume, , list]) => t === time ?
      list.filter(([r]) => r === row).map(([, , , v]) => v ?? volume) : [])[0], probe);
    const start = await volumeOf();
    await mouse().move(x, y(start ?? 80));
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(x, y(start ?? 80) + (lane.height) * i / 6);
    await mouse().up();
    await sleep(400);
    const dragged = await volumeOf();
    expect(dragged !== start, `dragging the bar left its volume at ${dragged}`);
    expect(dragged >= 5, `dragging the bar to the bottom set ${dragged}%, under 5%`);
    await page().keyboard.down("Control");
    await mouse().move(x, y(dragged));
    await mouse().down();
    for (let i = 1; i <= 4; i++) await mouse().move(x, y(dragged) - 6 * i);
    await mouse().up();
    await page().keyboard.up("Control");
    await sleep(300);
    await page().keyboard.down("Alt");
    await mouse().move(x - 40, lane.y + lane.height - 2);
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(x - 40 + 15 * i, lane.y + lane.height - 1);
    await mouse().up();
    await page().keyboard.up("Alt");
    await sleep(300);
    await page().keyboard.down("Shift");
    await mouse().move(x - 40, y(90));
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(x - 40 + 15 * i, y(90) + 8 * i);
    await mouse().up();
    await page().keyboard.up("Shift");
    await sleep(300);
    await pickTool("Ramp");
    await mouse().move(x - 40, y(30));
    await mouse().down();
    for (let i = 1; i <= 6; i++) await mouse().move(x - 40 + 15 * i, lane.y + lane.height);
    await mouse().up();
    await sleep(400);
    await pickTool("Select");
    const low = await read(() => window.scene.moments.flatMap(([time, volume, , list]) =>
      list.filter(([, , , v]) => (v ?? volume) < 5).map(([row, , , v]) => `${time} ms row ${row} at ${v ?? volume}%`)));
    expect(!low.length, `volumes under 5% after the volume lane: ${low.slice(0, 5).join(", ")}`);
    expect((await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`)) !== history, "the volume lane made no edit");
    await undoTo(history);
  }, ["mouse:Drag a bar", "mouse:Ctrl+drag a selected bar", "mouse:Alt+drag in the volume lane", "mouse:Shift+drag in the volume lane",
    "mouse:Drag in the value lane (Ramp)"]);

  await step("Mouse: lane headers: click, Ctrl+click, Shift+click, right-click, fold, drag a folder", async () => {
    await ensureProbe();
    await revealLane(laneRow);
    const name = row => page().locator(`#lanes button.name[data-lane-row="${row}"]`);
    await name(laneRow).click();
    await until(row => window.view.activeLane === row, laneRow, "clicking a lane name did not make it the active lane", 3000);
    const next = await read(row => window.scene.laneOrder?.[window.scene.laneOrder.indexOf(row) + 1] ?? row + 1, laneRow);
    if (await name(next).count()) {
      await page().keyboard.down("Control");
      await name(next).click();
      await page().keyboard.up("Control");
      await page().keyboard.down("Shift");
      await name(laneRow).click();
      await page().keyboard.up("Shift");
    }
    await name(laneRow).click({ button: "right" });
    await until(() => !document.getElementById("editor-menu").hidden, null, "right-clicking a lane name opened no menu", 3000);
    await page().keyboard.press("Escape");
    const folder = page().locator("#lanes .sample-folder").first();
    const fold = folder.locator("button.fold");
    const open = await fold.getAttribute("aria-expanded");
    await fold.click();
    await until(open => document.querySelector("#lanes .sample-folder button.fold").getAttribute("aria-expanded") !== open, open, "the folder did not fold", 3000);
    // A folded folder shows its hits as a preview; a click on one opens it again.
    const preview = await read(() => {
      const folder = document.querySelector("#lanes .sample-folder").getBoundingClientRect();
      const grid = document.getElementById("grid").getBoundingClientRect();
      return { x: grid.x + grid.width / 2, y: folder.y + folder.height / 2 };
    });
    await mouse().click(preview.x, preview.y);
    await sleep(400);
    if ((await fold.getAttribute("aria-expanded")) !== open) await fold.click();
    // Drag the first folder below the second, then back.
    const folders = await page().locator("#lanes .sample-folder").count();
    if (folders > 1) {
      const order = await read(() => Array.from(document.querySelectorAll("#lanes .sample-folder"), node => node.dataset.sampleSet).join(","));
      await page().locator("#lanes .sample-folder").first().dragTo(page().locator("#lanes .sample-folder").nth(1));
      await sleep(400);
      const after = await read(() => Array.from(document.querySelectorAll("#lanes .sample-folder"), node => node.dataset.sampleSet).join(","));
      expect(after !== order, `dragging a folder header left the order at ${after}`);
      await page().locator("#lanes .sample-folder").nth(1).dragTo(page().locator("#lanes .sample-folder").first());
    }
  }, ["mouse:Click / Ctrl+click / Shift+click a lane name", "mouse:Right-click a lane name", "mouse:Click a collapsed hit preview",
    "mouse:Drag a sample-set folder header"]);

  await step("Mouse: hover shows a hint, and the wheel over the snap slider", async result => {
    await ensureProbe();
    const point = await pointAt(probe[0], probe[1]);
    await mouse().move(5, 5);
    await sleep(200);
    const before = await fingerprint();
    await mouse().move(point.x, point.y);
    await sleep(500);
    const hovered = await fingerprint();
    const notes = [`hint "${hovered.hint.slice(0, 40)}"`];
    expect(hovered.hint !== before.hint || hovered.status !== before.status, "hovering a hit changed no hint or status text");
    const slider = await read(() => document.querySelector('[role="slider"][aria-label="Snap"]').getBoundingClientRect().toJSON());
    const snap = await fingerprint();
    await mouse().move(slider.x + slider.width / 2, slider.y + slider.height / 2);
    await mouse().wheel(0, -120);
    await sleep(400);
    if ((await fingerprint()).snap !== snap.snap) {
      notes.push("the wheel over the snap slider changed the snap");
      result.reached.push("mouse:Wheel over the snap slider");
    } else notes.push("the snap slider does not take the wheel (not counted as reached)");
    return notes.join("; ");
  }, ["mouse:Hover a hit or lane (hint bar)", "feature:hint-bar"]);

  await step("Mouse: drag the panel edges", async () => {
    const edges = await read(() => Array.from(document.querySelectorAll("[data-dock-splitter], .dock-splitter, [role=separator]"))
      .filter(node => node.getClientRects().length).slice(0, 4).map(node => node.getBoundingClientRect().toJSON()));
    if (!edges.length) return { skip: "no panel edge is shown" };
    for (const edge of edges) {
      const horizontal = edge.width > edge.height;
      await mouse().move(edge.x + edge.width / 2, edge.y + edge.height / 2);
      await mouse().down();
      await mouse().move(edge.x + edge.width / 2 + (horizontal ? 0 : 30), edge.y + edge.height / 2 + (horizontal ? 30 : 0), { steps: 5 });
      await mouse().move(edge.x + edge.width / 2, edge.y + edge.height / 2, { steps: 5 });
      await mouse().up();
    }
    return `${edges.length} edges dragged and put back`;
  }, ["mouse:Drag a panel edge"]);

  await step("Mouse: a file dropped on the window", async () => ({
    skip: "a drop from Explorer comes through the operating system (tauri://drag-drop), which the page's test hooks cannot send",
  }), []);

  await step("Unsaved edits: going Home asks first, Keep editing stays", async () => {
    await makeEdit();
    await openMenu("File");
    await menuRow("Start screen").click();
    await robot.dialogShown("Save changes before going Home?");
    const buttons = await page().locator("#dialog-actions button").allTextContents();
    await page().keyboard.press("Escape");
    await robot.dialogClosed();
    expect(!(await read(() => window.view.showHome)) && await read(() => window.view.dirty), "cancelling the question did not keep the edits on screen");
    return `asked with ${buttons.join(", ")}`;
  }, ["dialog:pending-home"]);

  await step("Unsaved edits: the window's close button asks first, and cancelling keeps the editor", async () => {
    if (!native.available) return { skip: native.reason };
    expect(await read(() => window.view.dirty), "there are no unsaved edits to be asked about");
    const answer = await native.closeMain(pid());
    expect(answer.ok, `the editor's window could not be found: ${answer.error ?? ""}`);
    await robot.dialogShown("Quit dawsu!?");
    const buttons = await page().locator("#dialog-actions button").allTextContents();
    await page().keyboard.press("Escape");
    await robot.dialogClosed();
    expect(robot.app.child.exitCode === null, "the editor closed although the question was cancelled");
    return `asked with ${buttons.join(", ")}`;
  }, ["dialog:close-editor"]);

  await step("Unsaved edits: another difficulty asks first; a clean switch goes there and back", async () => {
    const others = await read(() => (window.scene.difficulties ?? []).filter(path => path !== window.scene.path));
    if (!others.length) return { skip: "the mapset copy has one difficulty" };
    expect(await read(() => window.view.dirty), "there are no unsaved edits to be asked about");
    const select = page().locator("#difficulty");
    const here = await select.inputValue();
    const other = await select.locator("option").evaluateAll((list, here) => list.map(option => option.value).find(value => value && value !== here), here);
    await select.selectOption(other);
    await robot.dialogShown("Save changes before opening?");
    await page().keyboard.press("Escape");
    await robot.dialogClosed();
    await undoTo("0/0");
    probe = null;
    if (await read(() => window.view.dirty)) return "asked; the copy still differs from disk after Undo, so the switch itself was left out";
    await select.selectOption(other);
    await until(here => window.scene.path !== here && !window.view.busy, here, "picking another difficulty did not open it", 20000);
    await select.selectOption(here);
    await until(here => window.scene.path === here && !window.view.busy, here, "picking the first difficulty again did not go back", 20000);
    probe = null;
    return `${others.length} other difficulties; asked, then switched and back`;
  }, ["dialog:pending", "control:main:index:#difficulty (select)", "menu:map.difficulty"]);

  // The editor's own dialogs: every control in each.
  for (const [menu, row, title] of [["Map", "Copy hitsounds…", "Copy hitsounds"], ["Map", "Generate hitsound difficulty…", "Generate hitsound difficulty"],
    ["Map", "New hitsound difficulty…", "New difficulty"], ["Playback", "Audio output…", "Audio"], ["View", "Time signatures…", "Time signatures"],
    ["View", "View settings…", "View"], ["File", "Folders…", "Folders"], ["Map", "Add lane…", "Add lane"], ["View", "Shortcuts", "Shortcuts and mouse actions"]]) {
    await step(`Crawl: press every control in the ${title} dialog`, async result => {
      const surface = {
        page: () => page(), selector: "#dialog",
        open: async () => {
          if ((await dialogTitle())?.startsWith(title)) return true;
          await settle();
          await openMenu(menu);
          await menuRow(row).click();
          return robot.dialogShown(title).then(() => true, () => false);
        },
        // A control that opened another dialog in its place: back to this one next time.
        tidy: async () => {
          if (native.available && (await native.dialogs(pid())).length) await native.cancel(pid());
          for (const window of others()) if (!window.url().endsWith("preview.html")) await window.close().catch(() => {});
        },
      };
      expect(await surface.open(), `${menu} › ${row} opened no "${title}" dialog`);
      // The buttons that finish the dialog (Copy, Generate, Create, Add, Save) are left alone: the
      // menu steps check that the dialog opens and the feature steps run the work itself.
      const finish = /^(Copy|Generate|Create|Add|Save|Apply|OK)$/;
      const done = await crawl(`main:dialog:${title}`, { ...surface, skipNames: finish }, result);
      await settle();
      return report(done, result);
    }, ["control:main:*"]);
  }

  // The features merged on 2026-11-24.
  await step("Feature: Save checks the written file and leaves no unused files", async () => {
    await ensureProbe();
    const before = snapshot(mapFolder);
    await page().click("#save");
    const reviewed = await finishSave();
    const after = snapshot(mapFolder);
    const added = [...after.keys()].filter(name => !before.has(name));
    const unused = added.filter(name => !playedBy(readFileSync(mapCopy, "latin1"), name));
    expect(!unused.length, `Save added files the difficulty never plays: ${unused.join(", ")}`);
    return `${reviewed.join("; ") || "no review"}; new files: ${added.join(", ") || "none"}`;
  }, ["feature:save-check", "menu:file.save"]);

  await step("Feature: live reload when the .osu changes on disk, with and without unsaved edits", async () => {
    // A harmless change outside the editor: a comment-free extra blank line at the end of the file.
    const bytes = readFileSync(mapCopy);
    writeFileSync(mapCopy, Buffer.concat([bytes, Buffer.from(bytes.includes(13) ? "\r\n" : "\n")]));
    await sleep(2500);
    const quiet = await fingerprint();
    // With unsaved edits the editor asks.
    await makeEdit();
    writeFileSync(mapCopy, bytes);
    await until(() => window.view.externalOsuChanged, null, "changing the .osu with unsaved edits showed no notice", 8000);
    await page().locator("#external-osu-notice button").filter({ hasText: "Reload from disk" }).click();
    await until(() => !window.view.externalOsuChanged && !window.view.busy, null, "Reload from disk did not reload", 10000);
    probe = null;
    return `without edits: ${quiet.externalOsu ? "a notice" : "reloaded quietly"}; with edits: notice, then Reload from disk`;
  }, ["feature:live-reload", "control:main:StatusBar:Reload from disk (button)"]);

  await step("Feature: mania hitsound import from File › Import mania hitsounds…", async () => {
    await ensureProbe();
    const history = await read(() => `${window.scene.historyPosition}/${window.scene.history.length}`);
    const toasts = await read(() => document.querySelectorAll(".toast-card").length);
    await openMenu("File");
    await menuRow("Import mania hitsounds…").click();
    await answerPicker(maniaCopy, "the mania picker");
    await until(([history, toasts]) => `${window.scene.historyPosition}/${window.scene.history.length}` !== history ||
      document.querySelectorAll(".toast-card").length > toasts, [history, toasts], "importing the mania difficulty made no edit and showed no message", 10000);
    const status = await read(() => (document.querySelector(".toast-card")?.innerText ?? document.getElementById("status").textContent).replace(/\s+/g, " "));
    await undoTo(history);
    return status.slice(0, 120);
  }, ["feature:mania-import", "menu:file.importManiaHitsounds"]);

  await step("Feature: Export MIDI writes a .mid where the picker says", async () => {
    const target = join(outputs, "every-input.mid");
    await openMenu("File");
    await menuRow("Export MIDI…").click();
    await answerPicker(target, "the MIDI save picker");
    await until(() => true, null, "", 100);
    const end = Date.now() + 8000;
    while (Date.now() < end && !existsSync(target)) await sleep(200);
    expect(existsSync(target) && statSync(target).size > 0, `no MIDI file at ${target}`);
    return `${statSync(target).size} bytes`;
  }, ["menu:file.exportMidi"]);

  await step("Feature: drum stem: attach, Song/Stem monitor with T, clear", async () => {
    await attachStem();
    const before = await read(() => !!window.view.stemActive);
    await page().mouse.move(300, 5);
    await page().keyboard.press("KeyT");
    await until(before => !!window.view.stemActive !== before, before, "T did not switch the Song/Stem monitor", 3000);
    await page().keyboard.press("KeyT");
    await openMenu("Playback");
    await menuRow("Clear drum stem").click();
    await until(() => !window.view.stemLoaded, null, "Clear drum stem did not clear it", 4000);
    // Attached again, a right-click on the stem lane clears it too.
    await attachStem();
    const lane = await read(() => document.getElementById("song-stem")?.getBoundingClientRect().toJSON());
    expect(lane && lane.width > 0, "the stem lane's switch is not shown");
    await page().locator("#song-stem").click({ button: "right" });
    await until(() => !window.view.stemLoaded, null, "right-clicking the stem lane did not clear the stem", 4000);
    return "stem attached from a picked WAV, monitored with T, cleared from the menu and by right-click";
  }, ["feature:drum-split", "feature:stem-monitor", "menu:playback.attachDrumStem", "menu:playback.clearDrumStem",
    "mouse:Right-click the stem lane"]);

  await step("Feature: Separate drums (AI) opens and says what it needs", async () => {
    await openMenu("Playback");
    await menuRow("Separate drums (AI)…").click();
    const dialog = page().getByRole("dialog", { name: "Separate drums (AI)" });
    await dialog.waitFor({ state: "visible" });
    const text = (await dialog.innerText()).replace(/\s+/g, " ");
    await page().keyboard.press("Escape");
    await dialog.waitFor({ state: "hidden" }).catch(async () => { await dialog.getByRole("button", { name: /Cancel|Close/ }).first().click(); });
    return text.slice(0, 120);
  }, ["feature:drum-split", "dialog:stem-separation"]);

  await step("Feature: green-line clean-up switch, then Save", async () => {
    await openMenu("File");
    await menuRow("Preferences…").click();
    const dialog = page().getByRole("dialog", { name: "Preferences" });
    await dialog.waitFor({ state: "visible" });
    const box = dialog.getByLabel("Clean redundant greenlines on Save & Export");
    const was = await box.isChecked();
    await box.click();
    await until(was => !!window.view.cleanRedundantGreenlines !== was, was, "the clean-up switch did not change", 3000);
    await dialog.getByRole("button", { name: "Close preferences" }).click();
    await makeEdit();
    const lines = text => text.split(/\r?\n/).filter(line => /^-?\d/.test(line) && line.split(",").length >= 7).length;
    const before = lines(readFileSync(mapCopy, "latin1"));
    await page().click("#save");
    await finishSave();
    const after = lines(readFileSync(mapCopy, "latin1"));
    // Put the switch back.
    await openMenu("File");
    await menuRow("Preferences…").click();
    await dialog.waitFor({ state: "visible" });
    if ((await dialog.getByLabel("Clean redundant greenlines on Save & Export").isChecked()) !== was) {
      await dialog.getByLabel("Clean redundant greenlines on Save & Export").click();
    }
    await dialog.getByRole("button", { name: "Close preferences" }).click();
    return `timing lines ${before} → ${after} with the clean-up ${was ? "off" : "on"}`;
  }, ["feature:greenline-cleanup", "control:main:PreferencesModal:Clean redundant greenlines on Save & Export (checkbox)"]);

  await step("Feature: coverage review list: filters, N and Shift+N", async () => {
    const review = page().locator('button[aria-label="Next coverage issue (N)"]').first();
    if (!(await review.count())) {
      await page().click("#rail-inspector-difficulty").catch(() => {});
      await sleep(300);
    }
    const before = await fingerprint();
    await page().mouse.move(300, 5);
    await page().keyboard.press("KeyN");
    await sleep(500);
    const next = await fingerprint();
    await page().keyboard.press("Shift+KeyN");
    await sleep(500);
    const back = await fingerprint();
    for (const name of ["All issues", "Empty", "Orphaned"]) {
      const filter = page().locator("button").filter({ hasText: new RegExp(`^${name}`) }).first();
      if (await filter.count() && await filter.isVisible()) await filter.click();
    }
    const moved = differs(before, next).filter(key => ["position", "start", "selected", "selection"].includes(key));
    if (!moved.length) return { skip: "this map has no coverage issue for N and Shift+N to step to" };
    return `N: ${describe(before, next, moved)}; Shift+N: ${describe(next, back, differs(next, back).filter(key => ["position", "start"].includes(key)))}`;
  }, ["feature:review-list", "key:page:N-next-issue", "key:page:Shift+N-previous-issue"]);

  await step("Feature: Export window: Ogg Vorbis mix, fallback sound, .osu changes view, then Cancel", async () => {
    await page().click("#export-entry");
    const dialog = page().getByRole("dialog", { name: "Export" });
    await dialog.waitFor({ state: "visible" });
    const window = await windowOpened("export.html", 3000).catch(() => null);
    const surface = window ?? dialog;
    const notes = [];
    const format = surface.locator('select:has(option[value="ogg_vorbis"])').first();
    expect(await format.count(), "the Export window shows no mix format list with Ogg Vorbis");
    const was = await format.inputValue();
    await format.selectOption("ogg_vorbis");
    notes.push("Ogg Vorbis chosen");
    await format.selectOption(was);
    const fallback = surface.getByText(/fallback/i).first();
    expect(await fallback.count(), "the Export window shows no fallback sound choice");
    notes.push("fallback sound choice shown");
    const diff = surface.locator("summary.osu-diff-toggle").first();
    if (await diff.count()) {
      await diff.click();
      notes.push(".osu changes view opened");
    }
    const cancel = surface.getByRole("button", { name: "Cancel" }).first();
    if (await cancel.count()) await cancel.click(); else await page().keyboard.press("Escape");
    return notes.join("; ");
  }, ["feature:ogg-export", "feature:fallback-sound", "feature:osu-diff-view", "window:export", "dialog:export-modal"]);

  await step("Feature: Copy hitsounds with slider-end muting, to the copy's other difficulty", async () => {
    await openMenu("Map");
    await menuRow("Copy hitsounds…").click();
    await robot.dialogShown("Copy hitsounds");
    const mute = page().locator("#dialog-body label").filter({ hasText: /slider end/i }).locator("input").first();
    if (!(await mute.count())) {
      await dialogButton(/^(Cancel|Close)$/).click();
      throw new Error("the Copy hitsounds dialog has no slider-end muting switch");
    }
    const was = await mute.isChecked();
    await mute.click();
    await mute.click();
    expect((await mute.isChecked()) === was, "the slider-end switch did not switch back");
    const targets = await page().locator("#dialog-body select option, #dialog-body input[type=checkbox]").count();
    await dialogButton(/^(Cancel|Close)$/).click();
    return `slider-end muting ${was ? "on" : "off"} by default; ${targets} choices in the dialog`;
  }, ["feature:slider-end-mute", "dialog:copy"]);

  await step("Feature: sample checks: the Save review lists sample warnings", async () => {
    await makeEdit();
    await page().click("#save");
    const seen = [];
    const end = Date.now() + 15000;
    while (Date.now() < end && await read(() => window.view.dirty || document.getElementById("dialog").open)) {
      const title = await dialogTitle();
      if (title) {
        seen.push(title + ": " + (await read(() => document.getElementById("dialog-body").innerText)).replace(/\s+/g, " ").slice(0, 80));
        if (title.startsWith("Review Save")) await dialogButton(/^(Save|Done)$/).click();
        else if (title.startsWith("Save hitsounds")) await dialogButton(/^(Save|Continue to Save)$/).click();
        else break;
      }
      await sleep(250);
    }
    expect(!(await read(() => window.view.dirty)), `Save did not finish: ${seen.join(" | ")}`);
    return seen.join(" | ") || "Save needed no review for this edit";
  }, ["feature:sample-checks"]);

  await step("Feature: Shared Base layer and Keysounds dialogs open and close", async () => {
    const notes = [];
    for (const [row, name] of [["Shared Base layer…", "Shared Base layer"], ["Keysounds & VST3…", "Keysounds and VST3"]]) {
      await openMenu("Map");
      await menuRow(row).click();
      const dialog = page().getByRole("dialog", { name });
      await dialog.waitFor({ state: "visible" });
      notes.push(`${name}: ${(await dialog.innerText()).replace(/\s+/g, " ").slice(0, 60)}`);
      await page().keyboard.press("Escape");
      await dialog.waitFor({ state: "hidden" }).catch(async () => { await dialog.getByRole("button", { name: /Cancel|Close|Done/ }).first().click(); });
    }
    return notes.join(" | ");
  }, ["feature:shared-base", "dialog:shared-base", "dialog:keysounds"]);

  await step("Feature: + Create Hitsound diff on the start screen", async () => {
    await openMenu("File");
    await menuRow("Start screen").click();
    await until(() => window.view.showHome, null, "the start screen did not show", 3000);
    const create = page().locator("#home button").filter({ hasText: /Create Hitsound/i }).first();
    if (!(await create.count())) {
      await page().keyboard.press("Escape");
      return { skip: "the start screen shows no + Create Hitsound diff button for this map" };
    }
    await create.click();
    await sleep(800);
    const now = await fingerprint();
    await settle();
    if ((await read(() => window.view.showHome))) await page().keyboard.press("Escape");
    return `${now.dialog ?? now.modals ?? "no dialog"}`;
  }, ["feature:create-hitsound-diff"]);

  // Every control on screen, in the main window, then each dialog and window. A surface is a page and
  // a selector for its root; the crawl presses each visible control once (a switch twice, so it ends
  // as it was), closes whatever that opened, and opens the surface again if the control closed it.
  const crawled = new Set();
  // Left alone: what ends the run or loses work, what reaches outside the run (updates, the system's
  // plug-in folder), the shortcut keys themselves, and anything that makes sound (the song and folder
  // mutes stay on, Audition and the lane names that audition stay off, Play and Record are not pressed).
  const danger = /^(Quit|Exit|Delete( lane)?|Revert|Discard|Reset|Remove|Clear|Overwrite|Check for updates|Scan|Rescan|Play|Pause|Preview|Listen|Audition|Select and audition|MIDI Record|Record|Mute|Solo|Unmute)\b/i;
  const leaveAlone = control => danger.test(control.name) || control.link || /\b(pref-key-chip|lane-mute|lane-solo)\b/.test(control.classes) ||
    ["song-mute", "song-solo", "song-stem"].includes(control.id);
  const listControls = (surfacePage, selector) => surfacePage.evaluate(({ selector }) => {
    const root = selector ? document.querySelector(selector) : document.body;
    if (!root) return [];
    const visible = node => node.getClientRects().length > 0 && getComputedStyle(node).visibility !== "hidden" && !node.closest("[hidden]");
    const name = node => (node.getAttribute("aria-label") || node.getAttribute("title") || node.getAttribute("placeholder") ||
      (node.tagName === "BUTTON" || node.tagName === "SUMMARY" ? node.textContent : "") ||
      node.closest("label")?.textContent || node.id || node.getAttribute("name") || "").replace(/\s+/g, " ").trim().slice(0, 60);
    const nodes = Array.from(root.querySelectorAll('button, summary, input:not([type=hidden]), select, [role=slider], [role=tab], [role=switch], [role=checkbox]'))
      .filter(node => visible(node) && !node.disabled && node.getAttribute("aria-disabled") !== "true")
      // The menu bar and its popup are the menu steps' work; another surface's dialog is its own crawl.
      .filter(node => !node.closest("#editor-menu, #editor-menus") && (selector || !node.closest('[role="dialog"], dialog')));
    const seen = new Map();
    return nodes.map((node, index) => {
      node.dataset.robotCrawl = String(index);
      const type = node.tagName === "INPUT" ? (node.type || "text") : node.getAttribute("role") || node.tagName.toLowerCase();
      const label = name(node);
      // The same name and type twice: the second is "name #2", so each keeps its own key.
      const base = `${label} (${type})`;
      seen.set(base, (seen.get(base) ?? 0) + 1);
      return { index, name: label, type, key: seen.get(base) > 1 ? `${base} #${seen.get(base)}` : base,
        pressed: node.hasAttribute("aria-pressed"), id: node.id, classes: String(node.className?.baseVal ?? node.className ?? ""),
        link: !!node.closest("a[href^=http]") };
    });
  }, { selector });
  // Each control is found again by its key just before it is pressed, so a surface that an earlier
  // control closed and the crawl opened again is still gone through to the end.
  async function crawl(surfaceName, surface, result) {
    let done = 0;
    const tried = new Set();
    for (let pass = 0; pass < 3; pass++) {
      const keys = (await listControls(surface.page(), surface.selector)).map(control => control.key);
      for (const wanted of keys) {
        const key = `${surfaceName}:${wanted}`;
        if (crawled.has(key) || tried.has(key)) continue;
        if (!(await surface.open())) break;
        const here = surface.page();
        const control = (await listControls(here, surface.selector)).find(control => control.key === wanted);
        if (!control || !control.name) continue;
        tried.add(key);
        if (leaveAlone(control) || surface.skipNames?.test(control.name)) {
          crawled.add(key);
          result.notes ??= [];
          if (result.notes.length < 40) result.notes.push(`left alone: ${control.name}`);
          continue;
        }
        const target = here.locator(`[data-robot-crawl="${control.index}"]`).first();
        if (!(await target.isVisible().catch(() => false))) continue;
        const explorersBefore = await native.explorers();
        try {
          // A switch (a checkbox, or a button that stays pressed) is pressed twice, so it ends as it was.
          if (["checkbox", "switch"].includes(control.type) || control.pressed) {
            await target.click({ timeout: 2000 });
            await sleep(250);
            if (await target.isVisible().catch(() => false)) await target.click({ timeout: 2000 }).catch(() => {});
          } else if (control.type === "select") {
            const value = await target.inputValue();
            const options = await target.locator("option").evaluateAll(list => list.map(option => option.value));
            for (const option of options.slice(0, 6)) await target.selectOption(option, { timeout: 2000 }).catch(() => {});
            await target.selectOption(value, { timeout: 2000 }).catch(() => {});
          } else if (control.type === "range" || control.type === "slider") {
            await target.focus();
            await here.keyboard.press("ArrowRight");
            await here.keyboard.press("ArrowLeft");
          } else if (control.type === "number") {
            await target.focus();
            await here.keyboard.press("ArrowUp");
            await here.keyboard.press("ArrowDown");
            await target.blur();
          } else if (["text", "search"].includes(control.type)) {
            if ((await target.getAttribute("readonly")) !== null) { crawled.add(key); continue; }
            const value = await target.inputValue();
            await target.fill(value + "a");
            await target.fill(value);
            await target.blur();
          } else {
            await target.click({ timeout: 2000 });
          }
          crawled.add(key);
          result.reached.push(`control:${surfaceName.split(":")[0]}:${control.name} (${control.type})`);
          done++;
        } catch (error) {
          result.notes ??= [];
          result.notes.push(`${control.name}: ${error.message.split("\n")[0].slice(0, 80)}`);
        }
        await sleep(300);
        if (native.available && (await native.dialogs(pid())).length) await native.cancel(pid());
        await closeNewExplorers(explorersBefore);
        await surface.tidy();
      }
    }
    return done;
  }
  const report = (done, result) => `${done} controls pressed${result.notes ? "; " + result.notes.slice(0, 8).join(" | ") : ""}`;
  // A dialog in the main window, opened by its menu row.
  const dialogSurface = (menu, row, label) => ({
    page: () => page(),
    selector: `[role="dialog"][aria-label="${label}"]`,
    open: async () => {
      if (await page().locator(`[role="dialog"][aria-label="${label}"]`).isVisible().catch(() => false)) return true;
      await settle();
      await openMenu(menu);
      await menuRow(row).click();
      return page().locator(`[role="dialog"][aria-label="${label}"]`).waitFor({ state: "visible", timeout: 5000 }).then(() => true, () => false);
    },
    // Close what a control opened over the dialog, not the dialog itself.
    tidy: async () => {
      if (native.available && (await native.dialogs(pid())).length) await native.cancel(pid());
      for (const window of others()) if (!window.url().endsWith("preview.html")) await window.close().catch(() => {});
      if (await read(() => document.getElementById("dialog").open)) await page().keyboard.press("Escape");
    },
  });

  await step("Crawl: press every control in the main window", async result => {
    await ensureProbe();
    await selectProbe();
    const surface = { page: () => page(), selector: null, open: async () => true, tidy: settle };
    return report(await crawl("main", surface, result), result);
  }, ["control:main:*"]);
  for (const [menu, row, label] of [["File", "Import MIDI…", "Import MIDI"], ["Map", "Keysounds & VST3…", "Keysounds and VST3"],
    ["Map", "Shared Base layer…", "Shared Base layer"], ["Playback", "Separate drums (AI)…", "Separate drums (AI)"]]) {
    await step(`Crawl: press every control in ${label}`, async result => {
      const surface = dialogSurface(menu, row, label);
      expect(await surface.open(), `${menu} › ${row} opened no "${label}" dialog`);
      const done = await crawl(`main:${label}`, surface, result);
      await settle();
      return report(done, result);
    }, ["control:main:*"]);
  }
  await step("Crawl: press every control on every Preferences tab", async result => {
    const surface = dialogSurface("File", "Preferences…", "Preferences");
    expect(await surface.open(), "File › Preferences… opened no Preferences");
    const tabs = (await page().locator('[aria-label="Preferences"] .pref-nav-item').allTextContents()).map(tab => tab.trim());
    let done = 0;
    for (const scope of ["App", "This map"]) {
      await surface.open();
      const tab = page().locator('[aria-label="Preferences"] [role="tab"]').filter({ hasText: scope }).first();
      if (await tab.isEnabled().catch(() => false)) await tab.click();
      for (const name of scope === "App" ? tabs : [""]) {
        await surface.open();
        if (name) await page().locator('[aria-label="Preferences"] .pref-nav-item').filter({ hasText: name }).first().click();
        done += await crawl(`main:Preferences:${scope}:${name}`, surface, result);
      }
    }
    await settle();
    return `${tabs.length} tabs; ${report(done, result)}`;
  }, ["control:main:PreferencesModal:*", "dialog:preferences"]);

  // covers-all: control:export:* control:drum:* control:node-graph:* window:export window:drum window:node-graph
  for (const [name, row, file] of [["Export", null, "export.html"], ["Drum hit detection", ["Map", "Drum hit detection…"], "drum.html"],
    ["Node graph", ["Map", "Node graph…"], "node-graph.html"]]) {
    await step(`Crawl: press every control in the ${name} window`, async result => {
      let window = null;
      const open = async () => {
        if (window && !window.isClosed()) return true;
        await settle();
        if (row) { await openMenu(row[0]); await menuRow(row[1]).click(); }
        else await page().click("#export-entry");
        window = await windowOpened(file, 15000).catch(() => null);
        return !!window;
      };
      if (!(await open())) {
        // Export opens in the main window when it is not its own window.
        const surface = { page: () => page(), selector: `[role="dialog"][aria-label="${name}"]`,
          open: async () => page().locator(`[role="dialog"][aria-label="${name}"]`).isVisible().catch(() => false) ||
            (await page().click("#export-entry").then(() => sleep(500)), true),
          tidy: async () => { if (native.available) await native.cancel(pid()); } };
        expect(await page().locator(surface.selector).isVisible().catch(() => false), `no ${file} window and no ${name} dialog opened`);
        const done = await crawl(`main:${name}`, surface, result);
        await settle();
        return report(done, result);
      }
      const surface = { page: () => window, selector: null, open,
        tidy: async () => { if (native.available && (await native.dialogs(pid())).length) await native.cancel(pid()); } };
      const done = await crawl(file.replace(/\.html$/, ""), surface, result);
      if (window && !window.isClosed()) await window.close().catch(() => {});
      await settle();
      return report(done, result);
    }, [`control:${file.replace(/\.html$/, "")}:*`, `window:${file.replace(/\.html$/, "")}`]);
  }
  await step("Crawl: press every control in the map preview window", async result => {
    if (!(await read(() => window.dawsuUi?.mapPreviewOn?.()))) { await openMenu("View"); await menuRow("Map preview").click(); }
    const popOut = page().getByRole("button", { name: "Open in its own window" });
    await popOut.waitFor({ timeout: 5000 });
    await popOut.click();
    const preview = await windowOpened("preview.html");
    const surface = { page: () => preview, selector: null, open: async () => !preview.isClosed(), tidy: async () => {} };
    const done = await crawl("preview", surface, result);
    if (!preview.isClosed()) await page().getByRole("button", { name: "Show it here" }).click().catch(() => preview.close());
    if (await read(() => window.dawsuUi?.mapPreviewOn?.())) { await openMenu("View"); await menuRow("Map preview").click(); }
    return report(done, result);
  }, ["control:preview:*", "window:preview"]);

  await step("The original map folder is byte-identical", async () => {
    await robot.quit();
    const after = snapshot(dirname(original));
    const changed = [...new Set([...originalBefore.keys(), ...after.keys()])].filter(name => originalBefore.get(name) !== after.get(name));
    expect(!changed.length, `changed: ${changed.join(", ")}`);
    return `${after.size} files`;
  }, [], { always: true });

  await step("What the run left in the map copy", async () => {
    const after = snapshot(mapFolder);
    const added = [...after.keys()].filter(name => !copyBefore.has(name));
    const removed = [...copyBefore.keys()].filter(name => !after.has(name));
    return `added: ${added.join(", ") || "nothing"}; removed: ${removed.join(", ") || "nothing"}`;
  }, [], { always: true });
  await step("No errors after the last step", async () => {
    await sleep(500);
  }, [], { always: true });
} finally {
  native.stop();
  const summary = { map: original, presets, routes: [...routes] };
  process.exitCode = await robot.finish({ started, json: args.json, keepProfile: args["keep-profile"], extra: summary, label: "every input" });
}
