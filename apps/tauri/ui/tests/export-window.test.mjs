import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import {
  blockDrops, choicesFrom, copyOptions, defaultChoices, difficultyName, exportLabel, exportState, included,
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
  assert.deepEqual(choices, { leftOut: [], matchWithin: 5, copyVolumes: true });
  assert.notEqual(choices.leftOut, defaultChoices.leftOut, "choices never share the frozen defaults' list");
  assert.deepEqual(choicesFrom({ choices: { leftOut: ["a.osu"], matchWithin: 3, copyVolumes: false } }),
    { leftOut: ["a.osu"], matchWithin: 3, copyVolumes: false });
  assert.deepEqual(included(view.targets, { leftOut: ["Artist - Song (Mapper) [Hard].osu"] }), [insane]);
});

test("the preview asks for the shown map, the included difficulties and Copy's options", () => {
  const choices = { leftOut: ["Artist - Song (Mapper) [Hard].osu"], matchWithin: 3, copyVolumes: false };
  const request = previewRequest(view, choices);
  assert.equal(request.options.source, view.source.path);
  assert.deepEqual(request.options.targets, [insane]);
  assert.equal(request.options.options.leniency, 3);
  assert.equal(request.options.options.copy_volumes, false);
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

test("with no other difficulties the window says so", () => {
  const { body } = render(ExportWindow, { props: { initialView: { ...view, targets: [] } } });
  assert.match(body, /This mapset has no other difficulties to copy into\./);
  assert.doesNotMatch(body, /Include /);
});
