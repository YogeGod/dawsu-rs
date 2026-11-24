// Pure helpers for Map › Shared Base layer… (multi-diff hitsounding base layer).

export const DEFAULT_LENIENCY_MS = 5;
export const MAX_LENIENCY_MS = 50;

const plural = (count, word) => `${count} ${word}${count === 1 ? "" : "s"}`;

export function clampLeniency(value) {
  const num = Number(value);
  if (!Number.isFinite(num)) return DEFAULT_LENIENCY_MS;
  return Math.min(MAX_LENIENCY_MS, Math.max(0, num));
}

export function applyArgs({ leniencyMs = DEFAULT_LENIENCY_MS, overwriteOverrides = false } = {}) {
  return {
    leniencyMs: clampLeniency(leniencyMs),
    overwriteOverrides: Boolean(overwriteOverrides),
  };
}

export function formatStatusLine(status) {
  if (!status?.hasBase) {
    return "No Shared Base captured for this mapset yet.";
  }
  const source = status.sourceDifficulty || "Unknown difficulty";
  return `Base from ${source} · ${plural(status.baseMomentCount ?? 0, "moment")}`;
}

export function formatApplySummary(result) {
  const applied = result?.appliedMoments ?? 0;
  const skipped = result?.skippedOverrides ?? 0;
  const unmatched = result?.unmatchedBaseMoments ?? 0;
  const parts = [`Applied to ${plural(applied, "moment")}`];
  if (skipped > 0) parts.push(`${plural(skipped, "override")} kept`);
  if (unmatched > 0) parts.push(`${unmatched} unmatched`);
  return `${parts.join(" · ")}. Press Ctrl+Z to undo in one step.`;
}

export function formatApplyAllSummary(result) {
  const updated = result?.updatedDiffs ?? 0;
  const moments = result?.totalAppliedMoments ?? 0;
  const skipped = result?.totalSkippedOverrides ?? 0;
  const failed = Array.isArray(result?.failedDiffs) ? result.failedDiffs.length : 0;
  const parts = [`Updated ${plural(updated, "diff")} (${plural(moments, "moment")})`];
  if (skipped > 0) parts.push(`${plural(skipped, "override")} kept`);
  if (failed > 0) parts.push(`${plural(failed, "diff")} skipped with issues`);
  return `${parts.join(" · ")}.`;
}
