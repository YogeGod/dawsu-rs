// Decimal units (1 MB = 1,000,000 bytes), as the Rust sizes and the download pages count.
export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const decimals = unit === 0 ? 0 : value >= 100 ? 0 : 1;
  return `${value.toFixed(decimals)} ${units[unit]}`;
}

export function isJobRunning(job) {
  const state = job?.state;
  return (
    state === "downloadingRuntime" ||
    state === "downloadingModel" ||
    state === "separating"
  );
}

export function progressPercent(job) {
  if (!job) return null;
  if (job.state === "downloadingRuntime" || job.state === "downloadingModel") {
    const total = Number(job.totalBytes);
    const done = Number(job.downloadedBytes);
    if (Number.isFinite(total) && total > 0 && Number.isFinite(done)) {
      return Math.max(0, Math.min(100, Math.round((done / total) * 100)));
    }
    return null;
  }
  if (job.state === "separating") {
    const total = Number(job.totalChunks);
    const chunk = Number(job.chunk);
    if (Number.isFinite(total) && total > 0 && Number.isFinite(chunk)) {
      // Chunk N is the one running now, so N - 1 are finished; only done shows 100%.
      return Math.max(0, Math.min(99, Math.round(((chunk - 1) / total) * 100)));
    }
    return null;
  }
  if (job.state === "done") return 100;
  return null;
}

export function formatJobStatus(job, jobMatchesOpenSong = true) {
  if (!job || job.state === "idle") return "";
  switch (job.state) {
    case "downloadingRuntime": {
      const pct = progressPercent(job);
      if (pct != null) {
        return `Downloading ONNX Runtime… ${pct}% (${formatBytes(job.downloadedBytes)} / ${formatBytes(job.totalBytes)})`;
      }
      return "Downloading ONNX Runtime…";
    }
    case "downloadingModel": {
      const pct = progressPercent(job);
      if (pct != null) {
        return `Downloading model… ${pct}% (${formatBytes(job.downloadedBytes)} / ${formatBytes(job.totalBytes)})`;
      }
      if (job.downloadedBytes > 0) {
        return `Downloading model… ${formatBytes(job.downloadedBytes)}`;
      }
      return "Downloading model…";
    }
    case "separating": {
      const total = Number(job.totalChunks);
      const chunk = Number(job.chunk);
      if (Number.isFinite(total) && total > 0 && Number.isFinite(chunk) && chunk > 0) {
        return `Separating chunk ${chunk} of ${total}…`;
      }
      return "Preparing audio for separation…";
    }
    case "done":
      return jobMatchesOpenSong
        ? "Drums ready — press T to flip between Song and Drums."
        : "Drum stem cached for another song — open that song's map to use it.";
    case "cancelled":
      return "Separation cancelled.";
    case "failed":
      return job.message ? `Separation failed: ${job.message}` : "Separation failed.";
    default:
      return "";
  }
}

export function formatModelStatus(status) {
  if (!status?.modelReady) {
    return `Not downloaded yet (${formatBytes(MODEL_SIZE_BYTES)})`;
  }
  if (Number.isFinite(status.modelSizeBytes) && status.modelSizeBytes > 0) {
    return `Ready on this computer (${formatBytes(status.modelSizeBytes)})`;
  }
  return "Ready on this computer";
}

// The pinned HT-Demucs model's size (DEFAULT_MODEL_SIZE in crates/dawsu-application/src/stem_separation.rs).
const MODEL_SIZE_BYTES = 165_612_636;

export function firstUseCopy(status) {
  const bytes = Number(status?.downloadBytesNeeded);
  return Number.isFinite(bytes) && bytes > 0 ? `Downloads about ${formatBytes(bytes)} once` : "";
}

// No cache: separate. Cached but not loaded: load it, or separate again. Loaded: separate again.
export function dialogButtons(status) {
  const again = { label: "Separate again", forceRerun: true };
  if (!status?.cachedStemPath) return { primary: { label: "Separate drums", forceRerun: false }, secondary: null };
  if (status.activeStemLoaded) return { primary: again, secondary: null };
  return { primary: { label: "Load cached drums", forceRerun: false }, secondary: again };
}

// Returns the new status, or null when the mapper said no (a missing confirm answers no).
export async function removeDownloadedModel({ invoke, confirm }) {
  const yes = (await confirm?.({
    title: "Remove the downloaded model?",
    body: `The HT-Demucs model (${formatBytes(MODEL_SIZE_BYTES)}) is deleted from this computer. Separating drums again downloads it again.`,
    confirmLabel: "Remove",
    danger: true,
  })) === true;
  if (!yes) return null;
  return invoke("stem_separation_action", { action: { kind: "delete_cache" } });
}

export function finishedJobToast(status) {
  const state = status?.job?.state;
  if (state === "done") {
    return status.jobMatchesOpenSong
      ? { tone: "success", title: "Drum stem ready", body: "Press T to flip between Song and Drums." }
      : { tone: "info", title: "Drum stem cached for another song", body: "Open that song's map to use it." };
  }
  if (state === "failed") {
    return { tone: "error", title: "Drum stem separation failed", body: status.job.message || "Stem separation failed.", sticky: true };
  }
  return null;
}

// One status timer at a time: starting a mode stops the timer before it.
export function createJobPoller({ setTimer = setInterval, clearTimer = clearInterval } = {}) {
  let timer = null;
  let mode = null;
  return {
    get mode() { return mode; },
    start(nextMode, ms, tick) {
      this.stop();
      mode = nextMode;
      timer = setTimer(tick, ms);
    },
    stop() {
      if (timer !== null) clearTimer(timer);
      timer = null;
      mode = null;
    },
  };
}

const BACKGROUND_POLL_MS = 2000;

// The dialog closed while a job runs: poll slowly, raise one toast when it ends, then stop.
export function watchInBackground(poller, { poll, toast, onFinish = () => {} }) {
  poller.start("slow", BACKGROUND_POLL_MS, async () => {
    let next;
    try {
      next = await poll();
    } catch (err) {
      if (poller.mode !== "slow") return;
      poller.stop();
      toast({ tone: "error", title: "Drum stem separation failed", body: String(err), sticky: true });
      return;
    }
    // Re-opened meanwhile (the dialog shows the result) or another tick got here first.
    if (poller.mode !== "slow" || !next || isJobRunning(next.job)) return;
    poller.stop();
    const finished = finishedJobToast(next);
    if (finished) toast(finished);
    onFinish(next);
  });
}
