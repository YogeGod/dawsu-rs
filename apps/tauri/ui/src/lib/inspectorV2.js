// Pure helpers for Tactile Inspector v2 (S|N|D & W|F|C pills, live gradient volume slider,
// 4-tab switcher) and consolidated bottom Objects overview diagnostics.

export const INSPECTOR_TABS = [
  { id: "selection", label: "Selection" },
  { id: "lane", label: "Lane" },
  { id: "bookmarks", label: "Bookmarks" },
  { id: "history", label: "History" },
];

export const SAMPLE_SET_PILLS = [
  { id: "soft", label: "Soft" },
  { id: "normal", label: "Normal" },
  { id: "drum", label: "Drum" },
];

export const ADDITION_PILLS = [
  { id: "whistle", label: "W", title: "Whistle" },
  { id: "finish", label: "F", title: "Finish" },
  { id: "clap", label: "C", title: "Clap" },
];

/**
 * Parses a formatted selection volume readout ("80%" or "60–80%") into a numeric slider value
 * and mixed-range flag.
 */
export function parseVolumeDisplay(text) {
  if (!text) return { value: 100, mixed: false };
  const range = String(text).match(/(\d+)\s*[–-]\s*(\d+)/);
  if (range) {
    const low = Number(range[1]);
    const high = Number(range[2]);
    return { value: Math.round((low + high) / 2), mixed: true };
  }
  const single = String(text).match(/(\d+)/);
  return { value: single ? Math.max(5, Math.min(100, Number(single[1]))) : 100, mixed: false };
}

/**
 * Returns the CSS gradient fill for the live volume slider (blue -> purple -> red).
 */
export function volumeSliderPercent(value) {
  const clamped = Math.max(5, Math.min(100, Math.round(Number(value) || 0)));
  return Math.round(((clamped - 5) / 95) * 100);
}

/**
 * Computes S|N|D pill states for the current selection. Custom-file-only selections disable
 * standard sample-set pills with an explanatory tooltip (Open Assumption #4).
 */
export function sampleSetPills(selection) {
  const customOnly = Boolean(selection?.hasCustomFile && !selection?.sampleSet);
  return SAMPLE_SET_PILLS.map(pill => ({
    ...pill,
    active: selection?.sampleSet === pill.id,
    mixed: selection?.sampleSet === "mixed",
    disabled: !selection || customOnly,
    title: customOnly
      ? "Custom sample files keep their own filenames"
      : `Set sample set to ${pill.label}`,
  }));
}

/**
 * Computes W|F|C addition pill states ("on" | "off" | "mixed") for the current selection.
 */
export function additionPills(selection) {
  const additions = selection?.additions ?? null;
  return ADDITION_PILLS.map(pill => {
    const state = additions?.[pill.id] ?? "off";
    return {
      ...pill,
      state,
      active: state === "on",
      mixed: state === "mixed",
      disabled: !selection,
    };
  });
}

/**
 * Consolidates save problems, orphaned hits, and empty objects into one summary for the bottom
 * Objects overview diagnostics bar.
 */
export function consolidateDiagnostics(problemStrip, coverage) {
  const problems = Array.isArray(problemStrip?.problems) ? problemStrip.problems : [];
  const orphaned = Array.isArray(problemStrip?.orphaned) ? problemStrip.orphaned : [];
  const issues = Array.isArray(coverage?.issues) ? coverage.issues : [];

  const points = [];
  for (const p of problems) {
    if (Number.isFinite(p?.time)) {
      points.push({ time: p.time, kind: "problem", label: p.message || "Save conflict", fixable: false });
    }
  }
  for (const t of orphaned) {
    if (Number.isFinite(t)) {
      points.push({ time: t, kind: "orphaned", label: "Hit with no object", fixable: true });
    }
  }
  for (const issue of issues) {
    if (issue?.kind === "empty" && Number.isFinite(issue?.time)) {
      points.push({ time: issue.time, kind: "empty", label: "Empty object", fixable: true });
    }
  }
  points.sort((a, b) => a.time - b.time);

  return {
    total: points.length,
    problemCount: problems.length,
    orphanedCount: orphaned.length,
    emptyCount: issues.filter(i => i?.kind === "empty").length,
    points,
  };
}

/**
 * Steps forward (+1) or backward (-1) through sorted diagnostic points from `playheadMs`.
 */
export function stepDiagnostic(points, playheadMs, direction = 1) {
  if (!points?.length) return null;
  const now = Number.isFinite(playheadMs) ? playheadMs : 0;
  if (direction >= 0) {
    return points.find(p => p.time > now + 1) ?? points[0];
  }
  for (let i = points.length - 1; i >= 0; i--) {
    if (points[i].time < now - 1) return points[i];
  }
  return points[points.length - 1];
}
