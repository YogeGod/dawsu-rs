import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import {
  DEFAULT_LENIENCY_MS,
  MAX_LENIENCY_MS,
  CLEAR_BASE_CONFIRM,
  RESET_OVERRIDES_CONFIRM,
  applyAllConfirm,
  applyArgs,
  applyToast,
  askFirst,
  baseCountLabel,
  captureConfirm,
  clampLeniency,
  confirmCaptureFirst,
  formatApplyAllSummary,
  formatApplySummary,
  formatStatusLine,
} from "../src/lib/sharedBase.js";
import SharedBaseModal from "../src/parts/SharedBaseModal.svelte";

test("clampLeniency keeps values inside 0..=50 ms and falls back to 5 ms", () => {
  assert.equal(DEFAULT_LENIENCY_MS, 5);
  assert.equal(MAX_LENIENCY_MS, 50);
  assert.equal(clampLeniency(5), 5);
  assert.equal(clampLeniency("12"), 12);
  assert.equal(clampLeniency(-4), 0);
  assert.equal(clampLeniency(120), 50);
  assert.equal(clampLeniency("abc"), 5);
  assert.equal(clampLeniency(undefined), 5);
});

test("applyArgs normalises leniencyMs and overwriteOverrides", () => {
  assert.deepEqual(applyArgs({ leniencyMs: "8", overwriteOverrides: 1 }), {
    leniencyMs: 8,
    overwriteOverrides: true,
  });
  assert.deepEqual(applyArgs(), {
    leniencyMs: 5,
    overwriteOverrides: false,
  });
});

test("formatStatusLine and formatApplySummary produce plain mapper copy", () => {
  assert.equal(
    formatStatusLine({ hasBase: false }),
    "No Shared Base captured for this mapset yet.",
  );
  assert.equal(
    formatStatusLine({ hasBase: true, sourceDifficulty: "Insane", baseMomentCount: 42 }),
    "Base from Insane · 42 moments",
  );
  assert.equal(
    formatStatusLine({ hasBase: true, sourceDifficulty: "Hard", baseMomentCount: 1 }),
    "Base from Hard · 1 moment",
  );
  assert.equal(
    formatApplySummary({ appliedMoments: 12, skippedOverrides: 2, unmatchedBaseMoments: 3 }),
    "Applied to 12 moments · 2 overrides kept · 3 unmatched. Press Ctrl+Z to undo in one step.",
  );
  assert.equal(
    formatApplySummary({ appliedMoments: 1, skippedOverrides: 0, unmatchedBaseMoments: 0 }),
    "Applied to 1 moment. Press Ctrl+Z to undo in one step.",
  );
});

test("SharedBaseModal renders cleanly and exposes openSharedBase", () => {
  const rendered = render(SharedBaseModal);
  assert.equal(typeof rendered.body, "string");
  const source = readFileSync(new URL("../src/parts/SharedBaseModal.svelte", import.meta.url), "utf8");
  assert.match(source, /bridge\.registerModal\("openSharedBase", openSharedBase\)/);
  assert.match(source, /Use Shared Base for this mapset/);
  assert.match(source, /Capture current difficulty as Base/);
  assert.match(source, /Apply Base to this difficulty/);
  assert.match(source, /Reset overrides/);
  assert.match(source, /Clear Shared Base/);
  assert.match(source, /Live diff tabs/);
  assert.match(source, /Lane rules/);
  assert.match(source, /Volume curve/);
});

test("applyToast names every failed difficulty in a sticky error toast", () => {
  const result = {
    updatedDiffs: 1, totalAppliedMoments: 4, totalSkippedOverrides: 0,
    failedDiffs: ["Hard: could not read (denied)", "Insane: verification failed (line 12)"],
  };
  const toast = applyToast(true, result);
  assert.equal(toast.tone, "error");
  assert.ok(toast.body.startsWith(formatApplyAllSummary(result)), toast.body);
  assert.match(toast.body, /Hard: could not read \(denied\)/);
  assert.match(toast.body, /Insane: verification failed \(line 12\)/);
});

test("applyToast is a success with the apply summaries when nothing failed", () => {
  const all = { updatedDiffs: 3, totalAppliedMoments: 12, totalSkippedOverrides: 0, failedDiffs: [] };
  assert.deepEqual(applyToast(true, all), {
    tone: "success", title: "Applied Base to all difficulties", body: "Updated 3 diffs (12 moments).",
  });
  const one = { appliedMoments: 2, skippedOverrides: 0, unmatchedBaseMoments: 0 };
  assert.deepEqual(applyToast(false, one), {
    tone: "success", title: "Applied Base to this difficulty",
    body: "Applied to 2 moments. Press Ctrl+Z to undo in one step.",
  });
});

test("baseCountLabel reads baseMomentCount", () => {
  assert.equal(baseCountLabel({ baseMomentCount: 7 }), "7");
  assert.equal(baseCountLabel({ momentCount: 7 }), "0");
  assert.equal(baseCountLabel(null), "0");
});

test("Apply all asks with the disk, undo and backup warning", () => {
  const request = applyAllConfirm(3);
  assert.equal(request.title, "Apply Shared Base to all difficulties?");
  assert.equal(request.body,
    "Writes the Shared Base into 3 other difficulty files on disk within 5 ms. Ctrl+Z does not undo this. " +
    "dawsu! keeps backups: File › Revert mapset to original…");
  assert.equal(request.danger, true);
  assert.match(applyAllConfirm(1).body, /into 1 other difficulty file on disk/);
  assert.match(RESET_OVERRIDES_CONFIRM.title, /Reset overrides/);
  assert.match(CLEAR_BASE_CONFIRM.title, /Clear Shared Base/);
});

test("Capture asks only when a Base already exists", () => {
  assert.equal(captureConfirm(null), null);
  assert.equal(captureConfirm({ hasBase: false, enabled: true }), null);
  const request = captureConfirm({ hasBase: true, enabled: false, sourceDifficulty: "Insane", baseMomentCount: 42 });
  assert.equal(request.title, "Replace the Shared Base?");
  assert.match(request.body, /42 moments from Insane/);
});

test("askFirst proceeds only on a true answer", async () => {
  const asked = [];
  const confirm = answer => request => { asked.push(request); return Promise.resolve(answer); };
  assert.equal(await askFirst(confirm(false), null), true, "nothing to ask");
  assert.equal(asked.length, 0);
  assert.equal(await askFirst(confirm(true), { title: "x" }), true);
  assert.equal(await askFirst(confirm(false), { title: "x" }), false);
  assert.equal(await askFirst(() => undefined, { title: "x" }), false, "no confirm registered fails closed");
});

test("confirmCaptureFirst queries the status when none is cached", async () => {
  const calls = [];
  const invoke = async (name, args) => { calls.push([name, args]); return { hasBase: true, baseMomentCount: 5 }; };
  const asked = [];
  const confirm = async request => { asked.push(request); return false; };
  assert.equal(await confirmCaptureFirst({ invoke, confirm }), false);
  assert.deepEqual(calls, [["shared_base_action", { action: { kind: "status" } }]]);
  assert.equal(asked.length, 1);

  calls.length = 0;
  assert.equal(await confirmCaptureFirst({ invoke, confirm, status: { hasBase: false } }), true);
  assert.equal(calls.length, 0, "a cached status is not queried again");
  assert.equal(asked.length, 1, "no Base yet: no question");
});

test("bridge.sharedBaseRev counts Shared Base changes made outside the strip", async () => {
  const { bridge } = await import("../src/lib/bridge.svelte.js");
  const { callAction } = await import("../src/lib/registry.js");
  const start = bridge.sharedBaseRev;
  bridge.bumpSharedBaseRev();
  assert.equal(bridge.sharedBaseRev, start + 1);
  callAction("bumpSharedBaseRev");
  assert.equal(bridge.sharedBaseRev, start + 2, "menus.js and boot.js reach it through the registry");
});

test("the strip reloads on every Shared Base change, reads baseMomentCount and asks first", () => {
  const strip = readFileSync(new URL("../src/parts/BaseDiffStrip.svelte", import.meta.url), "utf8");
  assert.match(strip, /const rev = bridge\.sharedBaseRev;/);
  assert.match(strip, /const key = `\$\{p\}\|\$\{rev\}`;[^]*lastLoadedKey = key;/);
  assert.match(strip, /seq !== refreshSeq/);
  assert.doesNotMatch(strip, /lastLoadedPath/);
  assert.match(strip, /applyToast\(all, res\)/);
  assert.match(strip, /baseCountLabel\(status\)/);
  assert.match(strip, /confirmCaptureFirst\(/);
  assert.match(strip, /applyAllConfirm\(/);
});

test("no Shared Base payload is read through momentCount or summary", () => {
  for (const part of ["parts/BaseDiffStrip.svelte", "parts/SharedBaseModal.svelte", "editor/menus.js", "editor/boot.js"]) {
    const source = readFileSync(new URL(`../src/${part}`, import.meta.url), "utf8");
    assert.doesNotMatch(source, /\.momentCount\b/, part);
    assert.doesNotMatch(source, /res\?*\.summary\b|result\?*\.summary\b/, part);
  }
});

test("SharedBaseModal toasts its apply results, asks first and tells the strip", () => {
  const source = readFileSync(new URL("../src/parts/SharedBaseModal.svelte", import.meta.url), "utf8");
  assert.equal(source.match(/bridge\.showToast\(applyToast\((false|true), result\)\)/g)?.length, 2);
  for (const ask of ["applyAllConfirm(", "RESET_OVERRIDES_CONFIRM", "CLEAR_BASE_CONFIRM", "confirmCaptureFirst("]) {
    assert.ok(source.includes(ask), ask);
  }
  assert.ok((source.match(/bridge\.bumpSharedBaseRev\(\)/g)?.length ?? 0) >= 10, "every mutation and close bumps");
});
