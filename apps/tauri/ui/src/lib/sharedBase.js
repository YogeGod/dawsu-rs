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

export function uniqueLaneOptions(lanes) {
  return [...new Set((lanes ?? []).map((l) => l?.name).filter(Boolean))];
}

export function baseCountLabel(status) {
  return String(status?.baseMomentCount ?? 0);
}

// Apply results as a toast: any difficulty Apply all could not write makes it a sticky error naming each one.
export function applyToast(all, result) {
  const body = all ? formatApplyAllSummary(result) : formatApplySummary(result);
  const failed = Array.isArray(result?.failedDiffs) ? result.failedDiffs : [];
  if (failed.length > 0) {
    return { tone: "error", title: "Some difficulties were not updated", body: `${body} Not updated: ${failed.join("; ")}` };
  }
  return { tone: "success", title: all ? "Applied Base to all difficulties" : "Applied Base to this difficulty", body };
}

// Where the backups are; File holds Revert mapset to original… (lib/commands.js).
export const BACKUP_HINT = "dawsu! keeps backups: File \u203a Revert mapset to original\u2026";

export function applyAllConfirm(otherDiffs) {
  const count = Math.max(0, Number(otherDiffs) || 0);
  return {
    title: "Apply Shared Base to all difficulties?",
    body: `Writes the Shared Base into ${count} other difficulty ${count === 1 ? "file" : "files"} on disk within ${DEFAULT_LENIENCY_MS} ms. Ctrl+Z does not undo this. ${BACKUP_HINT}`,
    confirmLabel: "Apply to all",
    danger: true,
  };
}

export const RESET_OVERRIDES_CONFIRM = {
  title: "Reset overrides for this difficulty?",
  body: "This difficulty's own changes to the Shared Base are forgotten, so the next Apply writes the Base over them.",
  confirmLabel: "Reset overrides",
  danger: true,
};

export const CLEAR_BASE_CONFIRM = {
  title: "Clear Shared Base for this mapset?",
  body: "The captured Base, its overrides, lane rules and volume curve are removed. The .osu files are not changed.",
  confirmLabel: "Clear Shared Base",
  danger: true,
};

// Capturing over an existing Base replaces it (decision A6); a first Capture asks nothing.
export function captureConfirm(status) {
  if (!status?.hasBase) return null;
  const source = status.sourceDifficulty || "another difficulty";
  return {
    title: "Replace the Shared Base?",
    body: `The current Base (${plural(status.baseMomentCount ?? 0, "moment")} from ${source}) is replaced by this difficulty's hitsounds.`,
    confirmLabel: "Capture",
    danger: true,
  };
}

// True when there is nothing to ask or the mapper said yes; a missing confirm answers no.
export async function askFirst(confirm, request) {
  if (!request) return true;
  return (await confirm?.(request)) === true;
}

export async function confirmCaptureFirst({ invoke, confirm, status = null }) {
  const current = status ?? (await invoke("shared_base_action", { action: { kind: "status" } }));
  return askFirst(confirm, captureConfirm(current));
}
