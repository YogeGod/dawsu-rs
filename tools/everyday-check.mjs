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
import { cpSync, existsSync, readFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkProvenance, createRobot, parseArgs, playwrightKey, silentWav, sleep, snapshot } from "./robot/core.mjs";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = parseArgs();
if (!args.exe) throw new Error("--exe is required: the built dawsu-tauri.exe");
const exe = resolve(args.exe);
if (!existsSync(exe)) throw new Error(`no editor at ${exe}`);
checkProvenance(repo, exe, Boolean(args["allow-stale"]), "everyday-check");

for (const key of ["map", "shots", "json"]) if (args[key] === true) throw new Error(`--${key} needs a value`);
const original = resolve(args.map ?? join(repo, "tests/fixtures/save-pack/synthetic/lf-no-final-newline.osu"));
if (!existsSync(original)) throw new Error(`no map at ${original}`);
const started = Date.now();

// Playwright is found before any folder is made, so a missing Playwright leaves nothing behind.
// The profile, map copy and shots stay next to the build (target/ is ignored and on the build drive).
const robot = createRobot({ repo, exe, name: "everyday-check", shots: args.shots,
  opensMap: ["Open the map copy and see lanes and a drawn timeline"] });
const { profile, launch, quit, step, page, read, until, expect, hits, namedHits, at, menuRow, openMenu, fromMenu,
  pickTool, dialogTitle, dialogShown, dialogButton, dialogClosed, windowOpened, windowClosed, menuKeys } = robot;
const mapFolder = join(profile, "map");
const mapCopy = join(mapFolder, basename(original));
const originalBefore = snapshot(dirname(original));
// dereference: a link in the map's folder is copied as the file it points at, so the editor can
// never write through the copy into the original.
cpSync(dirname(original), mapFolder, { recursive: true, dereference: true });
// No fixture folder holds a spare sample, so Add lane would have nothing to offer. The Add lane step
// puts this short silent WAV in the copy first, as a mapper drops a new sample into the open mapset.
// The original folder is never written.
const laneSample = "everyday-check-lane.wav";
const writeLaneSample = () => silentWav(join(mapFolder, laneSample));
const copyBefore = snapshot(mapFolder);
const osuBefore = readFileSync(mapCopy);
const scene = fn => read(fn);

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
  }, { covers: ["dialog:key-preset"] });
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
  }, { covers: ["control:main:AuditionPill:Audition (button)"] });

  await step("Pick each of the seven tools", async () => {
    for (const tool of ["Draw", "Erase", "Mute", "Grab", "Range", "Ramp", "Select"]) await pickTool(tool);
  }, { covers: ["menu:tool.toolMute", "menu:tool.toolGrab", "menu:tool.toolRange", "menu:tool.toolRamp"] });

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
  }, { covers: ["mouse:Click / drag (Draw)"] });

  await step("Select the hit", async () => {
    if (!silent) return needsSilence();
    await pickTool("Select");
    await page().mouse.click(400, 10);
    const point = await at(placed[0], placed[1]);
    await page().mouse.click(point.x, point.y);
    await until(([time, row]) => window.scene.selection?.count === 1 && window.scene.moments.some(([t, , , list]) =>
      t === time && list.some(([r, selected]) => r === row && selected)), placed, "the hit was not selected");
  }, { covers: ["mouse:Click / drag (Select)"] });

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
  }, { covers: ["key:delete@DAW"] });

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
  }, { covers: ["key:undo@DAW"] });

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
  }, { covers: ["key:redo@DAW"] });

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
  }, { covers: ["dialog:add-lane"] });

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
  }, { covers: ["menu:view.viewSettings", "dialog:editor-view"] });

  await step("Open and close Folders…", async () => {
    await fromMenu("File", "Folders…");
    await dialogShown("Folders");
    await page().keyboard.press("Escape");
    await dialogClosed();
  }, { covers: ["menu:file.folders", "dialog:folders"] });

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
  }, { covers: ["menu:view.shortcuts", "dialog:help", "dialog:shortcut-editor"] });

  await step("Open and close the Export window", async () => {
    await page().click("#export-entry");
    const dialog = page().getByRole("dialog", { name: "Export" });
    await dialog.waitFor({ state: "visible" });
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await dialog.waitFor({ state: "hidden" });
  }, { covers: ["dialog:export-modal"] });

  await step("Open and close the drum window", async () => {
    await fromMenu("Map", "Drum hit detection…");
    const window = await windowOpened("drum.html");
    await window.getByRole("button", { name: "Cancel" }).click();
    await windowClosed(window, "drum");
  }, { covers: ["menu:map.drumDetection", "window:drum"] });

  await step("Open and close the map preview window", async () => {
    // The preview sits in the Inspector: show it first if a step before folded it.
    if (await read(() => document.getElementById("view-inspector").getAttribute("aria-pressed") !== "true")) await page().click("#view-inspector");
    if (await read(() => !window.dawsuUi?.mapPreviewOn?.())) await fromMenu("View", "Map preview");
    const popOut = page().getByRole("button", { name: "Open in its own window" });
    await popOut.waitFor({ timeout: 5000 });
    await popOut.click();
    const window = await windowOpened("preview.html");
    await page().getByRole("button", { name: "Show it here" }).click();
    await windowClosed(window, "map preview");
    await fromMenu("View", "Map preview");
    expect(await read(() => typeof window.dawsuUi?.mapPreviewOn === "function"), "the page no longer has dawsuUi.mapPreviewOn to ask");
    await until(() => !window.dawsuUi.mapPreviewOn(), null, "View › Map preview did not switch the preview off", 4000);
  }, { covers: ["menu:view.mapPreview", "window:preview"] });

  await step("Open and close the node graph window", async () => {
    await fromMenu("Map", "Node graph…");
    const window = await windowOpened("node-graph.html");
    await window.locator("button", { hasText: /^Close$/ }).first().click();
    await windowClosed(window, "node graph");
  }, { covers: ["menu:map.nodeGraph", "window:node-graph"] });

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
  }, { covers: ["mouse:Ctrl+wheel (DAW-like) / Alt+wheel (osu!)", "mouse:Shift+wheel", "mouse:Wheel without modifiers"] });

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
    }, { covers: ["mouse:Click / drag (Draw)"] });
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
    return reviewed.join("; ");
  }, { covers: ["dialog:save-review", "feature:save-check"] });

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
  // Save writes a sound the circle's line cannot hold as a mix of new sample files, and the editor
  // project keeps no record of the lanes it was drawn on: reopened, the circle plays those files
  // (as crates/dawsu-application/src/lib.rs hs_save_mix_reopens_without_duplicate_project_hits expects).
  await step("Reopening shows the mixed circle as the sample files Save wrote", async () => {
    expect(reopened, "the reopen did not finish, so there is nothing to compare");
    const now = reopened.filter(mixed).map(hit => hit.slice("3500 ms ".length));
    expect(now.length, "the circle at 3500 ms has no hits after reopening");
    const written = mixFiles.map(name => name.replace(/\.(wav|ogg|mp3)$/i, ""));
    const other = now.filter(name => !written.includes(name));
    expect(written.length && !other.length, `at 3500 ms: ${now.join(", ")}; Save wrote: ${mixFiles.join(", ") || "nothing"}`);
    return now.join(", ");
  });
  await step("No errors after the last step", async () => {
    await sleep(500);
  }, true);
} finally {
  process.exitCode = await robot.finish({ started, json: args.json, keepProfile: args["keep-profile"],
    extra: { map: original }, label: "everyday check" });
}
