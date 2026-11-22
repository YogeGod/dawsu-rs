import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  STEPS, STEP_COUNT, STORAGE_KEY, backStep, clampStep, isDone, isLastStep, markDone, nextStep, shouldAutoOpen,
} from "../src/lib/quickStart.js";
import QuickStartTour from "../src/parts/QuickStartTour.svelte";

const fakeStorage = (entries = {}) => {
  const data = new Map(Object.entries(entries));
  return { getItem: key => data.has(key) ? data.get(key) : null, setItem: (key, value) => data.set(key, String(value)), data };
};
const mapOpen = { hasDocument: true, showHome: false, firstLaunch: false };

test("the tour has five steps; Next and Back walk 1..5 and stop at the ends", () => {
  assert.equal(STEP_COUNT, 5);
  assert.equal(STEPS.length, 5);
  const walked = [1];
  while (!isLastStep(walked.at(-1))) walked.push(nextStep(walked.at(-1)));
  assert.deepEqual(walked, [1, 2, 3, 4, 5]);
  assert.equal(nextStep(5), 5);
  assert.equal(backStep(5), 4);
  assert.equal(backStep(2), 1);
  assert.equal(backStep(1), 1);
  assert.equal(clampStep(0), 1);
  assert.equal(clampStep(9), 5);
  assert.equal(clampStep("x"), 1);
});

test("step 2 keeps the design's f14 copy", () => {
  assert.equal(STEPS[1].title, "Draw a hit on a circle");
  assert.equal(STEPS[1].body,
    "Each row plays one sound. With Draw (3), click a row under a circle and the hit snaps onto it. Press Space to hear it with the song.");
});

test("done is kept in localStorage as dawsu.quickStartDone = 1", () => {
  const storage = fakeStorage();
  assert.equal(STORAGE_KEY, "dawsu.quickStartDone");
  assert.equal(isDone(storage), false);
  markDone(storage);
  assert.equal(storage.data.get("dawsu.quickStartDone"), "1");
  assert.equal(isDone(storage), true);
  // No storage, or storage that throws, never breaks the editor.
  assert.equal(isDone(null), false);
  assert.doesNotThrow(() => markDone({ setItem() { throw new Error("full"); } }));
  assert.equal(isDone({ getItem() { throw new Error("denied"); } }), false);
});

test("opens by itself only with a map open, not done, and no automation", () => {
  const storage = fakeStorage();
  assert.equal(shouldAutoOpen({ view: mapOpen, storage, navigator: {} }), true);
  assert.equal(shouldAutoOpen({ view: mapOpen, storage, navigator: { webdriver: true } }), false);
  assert.equal(shouldAutoOpen({ view: { ...mapOpen, hasDocument: false }, storage, navigator: {} }), false);
  assert.equal(shouldAutoOpen({ view: { ...mapOpen, showHome: true }, storage, navigator: {} }), false);
  assert.equal(shouldAutoOpen({ view: { ...mapOpen, firstLaunch: true }, storage, navigator: {} }), false);
  assert.equal(shouldAutoOpen({ view: mapOpen, storage: fakeStorage({ [STORAGE_KEY]: "1" }), navigator: {} }), false);
  assert.equal(shouldAutoOpen(), false);
});

test("closed, the tour draws nothing", () => {
  bridge.quickStart = 0;
  assert.equal(render(QuickStartTour).body.includes("quick-start"), false);
});

test("the card shows its step, five pills, Back, Next or Done, and Skip tour", () => {
  for (const step of [1, 2, 5]) {
    bridge.quickStart = step;
    const { body } = render(QuickStartTour);
    assert.match(body, new RegExp(`QUICK START · ${step} OF 5`));
    const pills = [...body.matchAll(/class="([^"]*)"/g)].map(match => match[1].split(" ")).filter(names => names.includes("qs-pill"));
    assert.equal(pills.length, 5);
    assert.equal(pills.filter(names => names.includes("on")).length, step);
    assert.match(body, />Back</);
    assert.match(body, />Skip tour</);
    assert.match(body, step === 5 ? />Done</ : />Next</);
    assert.ok(body.includes(STEPS[step - 1].title));
    // A floating card, not a modal over the editor.
    assert.match(body, /aria-modal="false"/);
  }
  bridge.quickStart = 1;
  assert.match(render(QuickStartTour).body, /class="qs-back[^"]*"[^>]*disabled/);
  bridge.quickStart = 2;
  assert.ok(render(QuickStartTour).body.includes("Press Space to hear it with the song."));
  bridge.quickStart = 0;
});
