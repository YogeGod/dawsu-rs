import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import {
  adoptChoices, afterPoll, baseStatus, ownWriteError, blockDrops, choicesFrom, runOutcome, statusLine, copyOptions, defaultChoices, difficultyName, exportLabel, exportState, included,
  laneSwatches, matchWithinValid, optionsKey, pollDelay, previewRequest, targetCounts,
} from "../src/lib/export.js";
import ExportWindow from "../src/parts/ExportWindow.svelte";

const insane = "C:\\Songs\\set\\Artist - Song (Mapper) [Insane].osu";
const hard = "C:\\Songs\\set\\Artist - Song (Mapper) [Hard].osu";
const view = {
  source: { path: "C:\\Songs\\set\\Artist - Song (Mapper) [HS 1].osu", difficulty: "HS 1", version: 4, hits: 196,
    lanes: [{ name: "Kick", colourKey: "kick", hitCount: 120 }, { name: "Clap", colourKey: "clap", hitCount: 76 }],
    laneColours: { kick: "#f1ad67", clap: "red" } },
  targets: [insane, hard],
  choices: null, busy: false, previewing: false, writing: false, written: null, writeError: null,
  error: null, problems: [], preview: null,
};

test("a map without stored choices starts with Copy's defaults and every difficulty included", () => {
  const choices = choicesFrom(view);
  assert.deepEqual(choices, { leftOut: [], matchWithin: 5, copyVolumes: true, base: false, sliderAudio: false });
  assert.notEqual(choices.leftOut, defaultChoices.leftOut, "choices never share the frozen defaults' list");
  assert.deepEqual(choicesFrom({ choices: { leftOut: ["a.osu"], matchWithin: 3, copyVolumes: false } }),
    { leftOut: ["a.osu"], matchWithin: 3, copyVolumes: false, base: false, sliderAudio: false },
    "choices stored before the base difficulty read it as off");
  assert.deepEqual(choicesFrom({ choices: { leftOut: [], matchWithin: 3, copyVolumes: true, base: true, sliderAudio: true } }),
    { leftOut: [], matchWithin: 3, copyVolumes: true, base: true, sliderAudio: true });
  assert.deepEqual(included(view.targets, { leftOut: ["Artist - Song (Mapper) [Hard].osu"] }), [insane]);
});

test("the preview asks for the shown map, the included difficulties and Copy's options", () => {
  const choices = { leftOut: ["Artist - Song (Mapper) [Hard].osu"], matchWithin: 3, copyVolumes: false };
  const request = previewRequest(view, choices);
  assert.equal(request.options.source, view.source.path);
  assert.deepEqual(request.options.targets, [insane]);
  assert.equal(request.options.options.leniency, 3);
  assert.equal(request.options.options.copy_volumes, false);
  assert.equal(request.options.base, false);
  assert.equal(request.options.sliderAudio, false);
  const withBase = previewRequest(view, { ...choices, base: true, sliderAudio: true });
  assert.equal(withBase.options.base, true);
  assert.equal(withBase.options.sliderAudio, true);
  assert.deepEqual(withBase.options.options, request.options.options, "Copy's options do not change with the base");
  assert.deepEqual(copyOptions(defaultChoices), { ...copyOptions({ matchWithin: 5, copyVolumes: true }) });
});

test("difficulties read by their osu! name, and the label counts what is included", () => {
  assert.equal(difficultyName(insane), "Insane");
  assert.equal(difficultyName("/maps/plain.osu"), "plain");
  assert.equal(exportLabel(3), "Export 3 difficulties");
  assert.equal(exportLabel(1), "Export 1 difficulty");
  assert.equal(exportLabel(0), "Export");
  assert.ok(matchWithinValid(0) && matchWithinValid(1000));
  assert.ok(!matchWithinValid(-1) && !matchWithinValid(1001) && !matchWithinValid(NaN));
});

test("Export needs this window's own preview of the map as it is now", () => {
  const choices = choicesFrom(view);
  const key = optionsKey(view, choices);
  const preview = { id: 7, stale: false, source: view.source.path, version: 4, targets: [] };
  const ready = { ...view, preview };
  assert.equal(exportState(ready, { id: 7, key }, choices).canExport, true);
  assert.equal(exportState(ready, null, choices).canExport, false, "an option changed, nothing asked yet");
  assert.equal(exportState(ready, { id: 6, key }, choices).canExport, false, "an older request");
  assert.equal(exportState({ ...ready, preview: { ...preview, id: 3 } }, { id: 7, key }, choices).canExport, false,
    "a preview another page asked for");
  const other = { ...choices, matchWithin: 6 };
  assert.equal(exportState(ready, { id: 7, key }, other).canExport, false, "options changed since the request");
  for (const changed of [{ ...ready, busy: true }, { ...ready, writing: true }]) {
    assert.equal(exportState(changed, { id: 7, key }, choices).canExport, false);
  }
  const edited = { ...ready, source: { ...view.source, version: 5 } };
  assert.deepEqual(exportState(edited, { id: 7, key }, choices), { canExport: false, stale: true, count: 2, preview: null });
  assert.equal(exportState({ ...ready, preview: { ...preview, stale: true } }, { id: 7, key }, choices).stale, true);
  const none = { ...choices, leftOut: [insane, hard].map(path => path.split("\\").pop()) };
  assert.equal(exportState(ready, { id: 7, key: optionsKey(view, none) }, none).canExport, false);
  assert.equal(exportState(ready, { id: 7, key }, { ...choices, matchWithin: -2 }).canExport, false);
  for (const changed of [{ ...choices, base: true }, { ...choices, sliderAudio: true }]) {
    assert.notEqual(optionsKey(view, changed), key, "the base choices are part of the preview");
    assert.equal(exportState(ready, { id: 7, key }, changed).canExport, false);
  }
});

test("the base difficulty counts as one more difficulty and can be exported alone", () => {
  const preview = { id: 7, stale: false, source: view.source.path, version: 4, targets: [] };
  const base = { ...choicesFrom(view), base: true };
  assert.equal(exportState({ ...view, preview }, { id: 7, key: optionsKey(view, base) }, base).count, 3);
  const alone = { ...view, targets: [], preview };
  const state = exportState(alone, { id: 7, key: optionsKey(alone, base) }, base);
  assert.deepEqual([state.count, state.canExport], [1, true]);
  assert.equal(exportLabel(state.count), "Export 1 difficulty");
});

test("the base row says whether it writes a new difficulty or replaces the earlier one", () => {
  const path = "C:\Songs\set\Artist - Song (Mapper) [Hitsounds].osu";
  assert.equal(baseStatus({ base: { path, replaces: false } }), "New difficulty");
  assert.equal(baseStatus({ base: { path, replaces: true } }), "Replaces earlier, backed up");
  assert.equal(baseStatus({ base: null }), null);
  assert.equal(baseStatus(null), null);
});

test("each target shows Copy's counts; zero counts that need nothing stay hidden", () => {
  const preview = { targets: [{ path: insane, matched: 470, unmatched: 6, changed: 488, collisions: 0, keptFromSilent: 2, muted: 0 }] };
  assert.deepEqual(targetCounts(preview, insane), {
    changed: 488,
    items: [
      { tone: "matched", text: "470 matched" },
      { tone: "unmatched", text: "6 unmatched" },
      { tone: "kept", text: "2 kept to avoid silence" },
    ],
  });
  assert.equal(targetCounts(preview, hard), null);
  assert.equal(targetCounts(null, insane), null);
});

test("polling speeds up while anything runs", () => {
  assert.equal(pollDelay(view), 250);
  assert.equal(pollDelay(null), 250);
  for (const field of ["busy", "previewing", "writing"]) assert.equal(pollDelay({ [field]: true }), 100);
});

test("only colours the mapper picked show on the source card", () => {
  assert.deepEqual(laneSwatches(view.source), ["#f1ad67"]);
  assert.deepEqual(laneSwatches(null), []);
});

test("a file dropped on the window is never opened in place of the page", () => {
  const listeners = new Map();
  const target = {
    addEventListener: (name, handler) => listeners.set(name, handler),
    removeEventListener: name => listeners.delete(name),
  };
  const release = blockDrops(target);
  for (const name of ["dragover", "drop"]) {
    let prevented = false;
    listeners.get(name)({ preventDefault: () => { prevented = true; } });
    assert.ok(prevented, name);
  }
  release();
  assert.equal(listeners.size, 0);
});

test("before the first answer the window shows no rows and a disabled Export", () => {
  const { body } = render(ExportWindow);
  assert.match(body, /<button[^>]*class="primary[^"]*"[^>]*disabled[^>]*>Export<\/button>/);
  assert.match(body, />Cancel<\/button>/);
  assert.match(body, />Show in editor<\/button>/);
  assert.doesNotMatch(body, /role="switch"/);
  assert.doesNotMatch(body, /ROUTING|Routing/, "no eyebrow label");
});

test("each difficulty is a row with its include switch; a left-out one keeps its own hitsounds", () => {
  const stored = { ...view, choices: { leftOut: ["Artist - Song (Mapper) [Hard].osu"], matchWithin: 5, copyVolumes: true } };
  const { body } = render(ExportWindow, { props: { initialView: stored } });
  assert.match(body, /<h1[^>]*>Copy HS 1 into the mapset<\/h1>/);
  assert.match(body, /2 lanes · 196 hits/);
  assert.match(body, /role="switch" aria-checked="true" aria-label="Include Insane"/);
  assert.match(body, /role="switch" aria-checked="false" aria-label="Include Hard"/);
  assert.match(body, /Left out · keeps its own hitsounds/);
  assert.match(body, /role="switch" aria-checked="true" aria-label="Copy volumes"/);
  assert.match(body, /value="5"/);
  // No preview yet: Export counts the included difficulty but stays disabled.
  assert.match(body, /<button[^>]*class="primary[^"]*"[^>]*disabled[^>]*>Export 1 difficulty<\/button>/);
});

test("with no other difficulties the window says so and still offers the base difficulty", () => {
  const { body } = render(ExportWindow, { props: { initialView: { ...view, targets: [] } } });
  assert.match(body, /This mapset has no other difficulties to copy into\./);
  assert.doesNotMatch(body, /Include (Insane|Hard)/);
  assert.match(body, /aria-label="Include the base hitsound difficulty"/);
});

test("the first row is the base difficulty, off until chosen, with its slider audio choice", () => {
  const { body } = render(ExportWindow, { props: { initialView: view } });
  const base = body.indexOf("Include the base hitsound difficulty");
  assert.ok(base > 0 && base < body.indexOf("Include Insane"), "the base row comes first");
  assert.match(body, /role="switch" aria-checked="false" aria-label="Include the base hitsound difficulty"/);
  assert.match(body, /<input type="checkbox"[^>]*disabled[^>]*>\s*Slider slide, whistle and tick audio/);
  assert.match(body, />Hitsounds</);
  assert.doesNotMatch(body.slice(0, body.indexOf("Include Insane")), /keeps its own hitsounds/);

  const chosen = { ...view, choices: { leftOut: [], matchWithin: 5, copyVolumes: true, base: true, sliderAudio: true } };
  const on = render(ExportWindow, { props: { initialView: chosen } }).body;
  assert.match(on, /role="switch" aria-checked="true" aria-label="Include the base hitsound difficulty"/);
  assert.match(on, /<input type="checkbox"[^>]*checked[^>]*>\s*Slider slide, whistle and tick audio/);
  assert.doesNotMatch(on, /<input type="checkbox"[^>]*disabled/);
  assert.match(on, /<button[^>]*class="primary[^"]*"[^>]*disabled[^>]*>Export 3 difficulties<\/button>/);
});

test("a write's result belongs to the run that started it, never to an earlier one", () => {
  const old = { writing: false, writeId: 1, writeError: "Hard.osu changed since this review.", written: null };
  assert.equal(runOutcome({ id: null }, old), "pending", "export_run has not answered yet");
  assert.equal(runOutcome({ id: 2 }, old), "pending", "the earlier run's error is not this run's");
  assert.equal(runOutcome({ id: 2 }, { writing: true, writeId: 2, written: null, writeError: null }), "pending");
  assert.equal(runOutcome({ id: 2 }, { writing: false, writeId: 2, written: 3, writeError: null }), "written");
  assert.equal(runOutcome({ id: 2 }, { ...old, writeId: 2 }), "failed");
  assert.equal(runOutcome(null, old), null);
});

test("after each poll the window closes on its own write, and previews again after a failed one", () => {
  const run = { id: 2 };
  assert.deepEqual(afterPoll(run, { writing: true, writeId: 2 }), { run, close: false, preview: false });
  assert.deepEqual(afterPoll(run, { writing: false, writeId: 2, written: 1, writeError: null }),
    { run: null, close: true, preview: false });
  assert.deepEqual(afterPoll(run, { writing: false, writeId: 2, written: null, writeError: "x" }),
    { run: null, close: false, preview: true });
  assert.deepEqual(afterPoll(null, { writing: false, writeId: 2, written: null, writeError: "x" }),
    { run: null, close: false, preview: false }, "an old error alone starts nothing");
});

test("stored choices are taken once per map, so a poll never undoes a click", () => {
  const first = adoptChoices(undefined, view);
  assert.equal(first.adoptedFor, view.source.path);
  assert.deepEqual(first.choices, choicesFrom(view));
  assert.equal(adoptChoices(view.source.path, { ...view, choices: { leftOut: ["x.osu"], matchWithin: 1, copyVolumes: false } }).choices, null);
  const other = adoptChoices(view.source.path, { ...view, source: { ...view.source, path: "other.osu" } });
  assert.equal(other.adoptedFor, "other.osu");
  assert.ok(other.choices);
});

test("the status line puts a lost editor first and offers Preview again where the message says so", () => {
  const quiet = { view: { ...view }, pollError: null, actionError: null, stale: false, running: false, problems: [] };
  assert.equal(statusLine(quiet), null);
  assert.deepEqual(statusLine({ ...quiet, pollError: "The editor worker did not respond", actionError: "x" }),
    { tone: "error", text: "The editor worker did not respond", again: false });
  assert.deepEqual(statusLine({ ...quiet, actionError: "Another difficulty was opened. Preview again." }),
    { tone: "error", text: "Another difficulty was opened. Preview again.", again: true });
  assert.deepEqual(statusLine({ ...quiet, view: { ...view, error: "Hard.osu: 2 sample-mix or placement fixes needed before Copy" } }),
    { tone: "error", text: "Hard.osu: 2 sample-mix or placement fixes needed before Copy", again: true });
  assert.equal(statusLine({ ...quiet, view: { ...view, writeError: "old" } }), null, "only this window's own write error shows");
  assert.deepEqual(statusLine({ ...quiet, writeError: "Hard.osu changed since this review. Nothing was written." }),
    { tone: "error", text: "Hard.osu changed since this review. Nothing was written.", again: false });
  assert.deepEqual(statusLine({ ...quiet, stale: true }), { tone: "stale", text: "Map changed since preview", again: true });
  const settings = { ...view, preview: { stale: true, staleReason: "mergeSettings" } };
  assert.deepEqual(statusLine({ ...quiet, view: settings, stale: true }),
    { tone: "stale", text: "Merge settings changed since preview", again: true });
  assert.deepEqual(statusLine({ ...quiet, running: true }), { tone: "busy", text: "Writing…", again: false });
  assert.deepEqual(statusLine({ ...quiet, view: { ...view, previewing: true, progress: { done: 1, total: 3 } } }),
    { tone: "busy", text: "Previewing 1/3", again: false });
  assert.deepEqual(statusLine({ ...quiet, problems: [{ message: "4 empty objects" }, { message: "b" }] }),
    { tone: "warning", text: "4 empty objects", count: 2, again: false });
});

test("a write error shows only for this window's own last run", () => {
  const failed = { writing: false, writeId: 2, writeError: "x" };
  assert.equal(ownWriteError(2, failed), "x");
  assert.equal(ownWriteError(1, failed), null);
  assert.equal(ownWriteError(null, failed), null, "an error from before the window opened");
  assert.equal(ownWriteError(2, { ...failed, writing: true }), null);
});
