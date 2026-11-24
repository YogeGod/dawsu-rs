export function formatBytes(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
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
  if (job.state === "downloadingModel") {
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
      return Math.max(0, Math.min(100, Math.round((chunk / total) * 100)));
    }
    return null;
  }
  if (job.state === "done") return 100;
  return null;
}

export function formatJobStatus(job) {
  if (!job || job.state === "idle") return "";
  switch (job.state) {
    case "downloadingRuntime":
      return "Downloading ONNX Runtime…";
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
      return "Drums ready — press T to flip between Song and Drums.";
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
    return "Not downloaded yet (~166 MB)";
  }
  if (Number.isFinite(status.modelSizeBytes) && status.modelSizeBytes > 0) {
    return `Ready on this computer (${formatBytes(status.modelSizeBytes)})`;
  }
  return "Ready on this computer";
}

export function primaryButtonLabel(status) {
  return status?.cachedStemPath ? "Re-separate drums" : "Separate drums";
}
