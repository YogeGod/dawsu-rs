import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { render } from "svelte/server";
import {
  DEFAULT_LENIENCY_MS,
  MAX_LENIENCY_MS,
  applyArgs,
  clampLeniency,
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
