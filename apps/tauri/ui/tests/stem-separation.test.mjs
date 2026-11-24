import assert from "node:assert/strict";
import { test } from "node:test";
import {
  formatBytes,
  formatJobStatus,
  formatModelStatus,
  isJobRunning,
  primaryButtonLabel,
  progressPercent,
} from "../src/lib/stemSeparation.js";

test("formatBytes formats bytes, KB, MB, and GB cleanly", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(512), "512 B");
  assert.equal(formatBytes(2048), "2.0 KB");
  assert.equal(formatBytes(166 * 1024 * 1024), "166 MB");
  assert.equal(formatBytes(1.5 * 1024 * 1024 * 1024), "1.5 GB");
});

test("progressPercent computes download and chunk separation percentages", () => {
  assert.equal(progressPercent({ state: "idle" }), null);
  assert.equal(progressPercent({ state: "downloadingRuntime" }), null);
  assert.equal(
    progressPercent({ state: "downloadingModel", downloadedBytes: 42, totalBytes: 100 }),
    42,
  );
  assert.equal(
    progressPercent({ state: "downloadingModel", downloadedBytes: 500, totalBytes: null }),
    null,
  );
  assert.equal(
    progressPercent({ state: "separating", chunk: 4, totalChunks: 16 }),
    25,
  );
  assert.equal(progressPercent({ state: "done", stemPath: "x.wav" }), 100);
});

test("formatJobStatus produces plain mapper-facing status lines", () => {
  assert.equal(formatJobStatus({ state: "idle" }), "");
  assert.equal(formatJobStatus({ state: "downloadingRuntime" }), "Downloading ONNX Runtime…");
  assert.match(
    formatJobStatus({
      state: "downloadingModel",
      downloadedBytes: 83 * 1024 * 1024,
      totalBytes: 166 * 1024 * 1024,
    }),
    /Downloading model… 50%/,
  );
  assert.equal(
    formatJobStatus({ state: "separating", chunk: 4, totalChunks: 18 }),
    "Separating chunk 4 of 18…",
  );
  assert.match(formatJobStatus({ state: "done", stemPath: "x.wav" }), /press T/i);
  assert.equal(formatJobStatus({ state: "cancelled" }), "Separation cancelled.");
  assert.equal(
    formatJobStatus({ state: "failed", message: "No internet" }),
    "Separation failed: No internet",
  );
});

test("model status, running detection, and primary button label", () => {
  assert.equal(formatModelStatus({ modelReady: false }), "Not downloaded yet (~166 MB)");
  assert.equal(
    formatModelStatus({ modelReady: true, modelSizeBytes: 166 * 1024 * 1024 }),
    "Ready on this computer (166 MB)",
  );
  assert.equal(isJobRunning({ state: "downloadingModel" }), true);
  assert.equal(isJobRunning({ state: "separating" }), true);
  assert.equal(isJobRunning({ state: "done" }), false);
  assert.equal(primaryButtonLabel({ cachedStemPath: null }), "Separate drums");
  assert.equal(primaryButtonLabel({ cachedStemPath: "stems/abc-drums.wav" }), "Re-separate drums");
});
