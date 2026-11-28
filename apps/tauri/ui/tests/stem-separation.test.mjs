import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import {
  createJobPoller,
  dialogButtons,
  finishedJobToast,
  firstUseCopy,
  formatBytes,
  formatJobStatus,
  formatModelStatus,
  isJobRunning,
  progressPercent,
  removeDownloadedModel,
  watchInBackground,
} from "../src/lib/stemSeparation.js";

const modal = await readFile(new URL("../src/parts/StemSeparationModal.svelte", import.meta.url), "utf8");

test("formatBytes uses decimal units, as the Rust sizes and download pages do", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(2000), "2.0 KB");
  assert.equal(formatBytes(287015333), "287 MB");
  assert.equal(formatBytes(165612636), "166 MB");
  assert.equal(formatBytes(1.5e9), "1.5 GB");
});

test("progressPercent: downloads by bytes, separating by finished chunks, 100 only when done", () => {
  assert.equal(progressPercent({ state: "idle" }), null);
  assert.equal(progressPercent({ state: "downloadingRuntime", downloadedBytes: 0, totalBytes: null }), null);
  assert.equal(progressPercent({ state: "downloadingRuntime", downloadedBytes: 60701348, totalBytes: 121402697 }), 50);
  assert.equal(progressPercent({ state: "downloadingModel", downloadedBytes: 42, totalBytes: 100 }), 42);
  assert.equal(progressPercent({ state: "downloadingModel", downloadedBytes: 500, totalBytes: null }), null);
  assert.equal(progressPercent({ state: "separating", chunk: 0, totalChunks: 4 }), 0);
  assert.equal(progressPercent({ state: "separating", chunk: 1, totalChunks: 1 }), 0);
  assert.equal(progressPercent({ state: "separating", chunk: 5, totalChunks: 17 }), 24);
  assert.equal(progressPercent({ state: "separating", chunk: 10, totalChunks: 10 }), 90);
  assert.equal(progressPercent({ state: "separating", chunk: 205, totalChunks: 205 }), 99);
  assert.equal(progressPercent({ state: "done", stemPath: "x.wav" }), 100);
});

test("formatJobStatus produces plain mapper-facing status lines", () => {
  assert.equal(formatJobStatus({ state: "idle" }), "");
  assert.equal(formatJobStatus({ state: "downloadingRuntime", downloadedBytes: 0, totalBytes: null }), "Downloading ONNX Runtime…");
  assert.equal(
    formatJobStatus({ state: "downloadingRuntime", downloadedBytes: 60701348, totalBytes: 121402697 }),
    "Downloading ONNX Runtime… 50% (60.7 MB / 121 MB)",
  );
  assert.match(
    formatJobStatus({ state: "downloadingModel", downloadedBytes: 83e6, totalBytes: 166e6 }),
    /Downloading model… 50%/,
  );
  assert.equal(formatJobStatus({ state: "separating", chunk: 4, totalChunks: 18 }), "Separating chunk 4 of 18…");
  assert.match(formatJobStatus({ state: "done", stemPath: "x.wav" }), /press T/i);
  assert.match(formatJobStatus({ state: "done", stemPath: "x.wav" }, false), /another song/i);
  assert.doesNotMatch(formatJobStatus({ state: "done", stemPath: "x.wav" }, false), /press T/i);
  assert.equal(formatJobStatus({ state: "cancelled" }), "Separation cancelled.");
  assert.equal(formatJobStatus({ state: "failed", message: "No internet" }), "Separation failed: No internet");
});

test("model status and running detection", () => {
  assert.equal(formatModelStatus({ modelReady: false }), "Not downloaded yet (166 MB)");
  assert.equal(formatModelStatus({ modelReady: true, modelSizeBytes: 165612636 }), "Ready on this computer (166 MB)");
  assert.equal(isJobRunning({ state: "downloadingRuntime" }), true);
  assert.equal(isJobRunning({ state: "downloadingModel" }), true);
  assert.equal(isJobRunning({ state: "separating" }), true);
  assert.equal(isJobRunning({ state: "done" }), false);
});

test("the first-use line names what still has to download, and nothing once both parts are here", () => {
  assert.equal(firstUseCopy({ downloadBytesNeeded: 287015333 }), "Downloads about 287 MB once");
  assert.equal(firstUseCopy({ downloadBytesNeeded: 165612636 }), "Downloads about 166 MB once");
  assert.equal(firstUseCopy({ downloadBytesNeeded: 0 }), "");
  assert.equal(firstUseCopy({}), "");
  assert.match(modal, /firstUseCopy\(status\)/);
  assert.doesNotMatch(modal, /166 MB/);
});

test("the dialog has three button states: separate, load cached, separate again", () => {
  assert.deepEqual(dialogButtons({ cachedStemPath: null, activeStemLoaded: false }), {
    primary: { label: "Separate drums", forceRerun: false },
    secondary: null,
  });
  assert.deepEqual(dialogButtons({ cachedStemPath: "stems/abc-drums.wav", activeStemLoaded: false }), {
    primary: { label: "Load cached drums", forceRerun: false },
    secondary: { label: "Separate again", forceRerun: true },
  });
  assert.deepEqual(dialogButtons({ cachedStemPath: "stems/abc-drums.wav", activeStemLoaded: true }), {
    primary: { label: "Separate again", forceRerun: true },
    secondary: null,
  });
  // The component sends the button's own forceRerun, not one guessed from the cache.
  assert.match(modal, /dialogButtons\(status\)/);
  assert.doesNotMatch(modal, /Boolean\(status\.cachedStemPath\)/);
});

test("Remove downloaded model asks first and sends delete_cache only on yes", async () => {
  const sent = [];
  const invoke = async (name, args) => { sent.push([name, args]); return { modelReady: false }; };
  const asked = [];
  assert.equal(await removeDownloadedModel({ invoke, confirm: async request => { asked.push(request); return false; } }), null);
  assert.deepEqual(sent, []);
  assert.equal(asked.length, 1);
  assert.equal(asked[0].danger, true);
  assert.equal(await removeDownloadedModel({ invoke, confirm: undefined }), null, "no confirm answers no");
  assert.deepEqual(sent, []);
  assert.deepEqual(await removeDownloadedModel({ invoke, confirm: async () => true }), { modelReady: false });
  assert.deepEqual(sent, [["stem_separation_action", { action: { kind: "delete_cache" } }]]);
  assert.match(modal, /removeDownloadedModel\(\{\s*invoke,\s*confirm\s*\}\)/);
});

test("a job that ends with the dialog closed raises one toast that fits the result", () => {
  const done = { job: { state: "done", stemPath: "x.wav" } };
  assert.deepEqual(finishedJobToast({ ...done, jobMatchesOpenSong: true }), {
    tone: "success", title: "Drum stem ready", body: "Press T to flip between Song and Drums.",
  });
  assert.deepEqual(finishedJobToast({ ...done, jobMatchesOpenSong: false }), {
    tone: "info", title: "Drum stem cached for another song", body: "Open that song's map to use it.",
  });
  assert.deepEqual(finishedJobToast({ job: { state: "failed", message: "No internet" } }), {
    tone: "error", title: "Drum stem separation failed", body: "No internet", sticky: true,
  });
  assert.equal(finishedJobToast({ job: { state: "cancelled" } }), null);
});

function fakeTimers() {
  const live = new Map();
  const log = [];
  let next = 1;
  return {
    live, log,
    setTimer(fn, ms) { const id = next++; live.set(id, { fn, ms }); log.push(["set", id, ms]); return id; },
    clearTimer(id) { live.delete(id); log.push(["clear", id]); },
    async tick(id) { await live.get(id)?.fn(); },
  };
}

test("the background watch polls slowly, toasts once when the job ends, then stops", async () => {
  const timers = fakeTimers();
  const poller = createJobPoller(timers);
  const replies = [
    { job: { state: "separating", chunk: 2, totalChunks: 5 } },
    { job: { state: "done", stemPath: "x.wav" }, jobMatchesOpenSong: true },
  ];
  const toasts = [];
  let finished = 0;
  watchInBackground(poller, { poll: async () => replies.shift(), toast: t => toasts.push(t), onFinish: () => { finished++; } });
  assert.equal(poller.mode, "slow");
  const [[, id, ms]] = timers.log;
  assert.ok(ms > 400);
  await timers.tick(id);
  assert.deepEqual(toasts, []);
  await timers.tick(id);
  assert.deepEqual(toasts.map(t => t.title), ["Drum stem ready"]);
  assert.equal(finished, 1);
  assert.equal(poller.mode, null);
  assert.equal(timers.live.size, 0);
});

test("two slow ticks answering at once raise only one toast; a failed poll raises a sticky error", async () => {
  const timers = fakeTimers();
  const poller = createJobPoller(timers);
  const toasts = [];
  const doneReply = { job: { state: "done", stemPath: "x.wav" }, jobMatchesOpenSong: true };
  watchInBackground(poller, { poll: async () => doneReply, toast: t => toasts.push(t) });
  const fn = [...timers.live.values()][0].fn;
  await Promise.all([fn(), fn()]);
  assert.equal(toasts.length, 1);

  const errors = [];
  watchInBackground(poller, { poll: async () => { throw new Error("bridge gone"); }, toast: t => errors.push(t) });
  await [...timers.live.values()][0].fn();
  assert.equal(errors.length, 1);
  assert.equal(errors[0].tone, "error");
  assert.equal(errors[0].sticky, true);
  assert.equal(timers.live.size, 0);
});

test("re-opening the dialog stops the slow poll before the 400 ms poll starts, so one timer runs", async () => {
  const timers = fakeTimers();
  const poller = createJobPoller(timers);
  const toasts = [];
  watchInBackground(poller, { poll: async () => ({ job: { state: "done" }, jobMatchesOpenSong: true }), toast: t => toasts.push(t) });
  const slowTick = [...timers.live.values()][0].fn;
  poller.start("fast", 400, () => {});
  assert.deepEqual(timers.log.map(([kind, , ms]) => kind === "set" ? ["set", ms > 400 ? "slow" : ms] : [kind]),
    [["set", "slow"], ["clear"], ["set", 400]]);
  assert.equal(timers.live.size, 1);
  assert.equal(poller.mode, "fast");
  // A slow tick already on its way when the dialog re-opened raises nothing; the dialog shows it.
  await slowTick();
  assert.deepEqual(toasts, []);
  assert.equal(poller.mode, "fast");
  // The component closes into the background watch only while a job runs.
  assert.match(modal, /watchInBackground\(poller,/);
  assert.match(modal, /poller\.start\("fast", 400,/);
  assert.match(modal, /formatJobStatus\(status\.job,\s*status\.jobMatchesOpenSong\)/);
  assert.match(modal, /next\.jobMatchesOpenSong/);
  assert.match(modal, /catch\s*\([^)]*\)\s*\{\s*if\s*\(poller\.mode\s*!==\s*"fast"\)\s*return;/);
  assert.match(modal, /syncPollingFor\(next\)/);
});
