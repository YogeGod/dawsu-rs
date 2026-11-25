// Pure helpers for shared Bounce & .osz 10 MB Ranking Budget views (D4 & K4).

export const OSZ_LIMIT_BYTES = 10 * 1024 * 1024;

export function formatMegabytes(bytes) {
  const mb = Math.max(0, Number(bytes) || 0) / (1024 * 1024);
  return `${mb.toFixed(2)} MB`;
}

export function budgetTone(totalBytes, limitBytes = OSZ_LIMIT_BYTES) {
  const ratio = (Number(totalBytes) || 0) / Math.max(1, Number(limitBytes) || OSZ_LIMIT_BYTES);
  if (ratio > 1) return "over";
  if (ratio >= 0.85) return "caution";
  return "safe";
}

export function dedupPercent(sourceNotes, uniqueSamples) {
  const total = Math.max(0, Number(sourceNotes) || 0);
  const unique = Math.max(0, Number(uniqueSamples) || 0);
  if (total === 0) return 0;
  return Math.max(0, Math.round(((total - unique) / total) * 100));
}
