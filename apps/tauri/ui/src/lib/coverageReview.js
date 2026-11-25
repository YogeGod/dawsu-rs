// Per-difficulty hitsound coverage analysis and issue stepping.
// Pure functions over scene.moments ([time, volume, onObject, hits]) so updates stay instant with zero IPC.
import { clock } from "./transport.js";

export const COVERAGE_FILTERS = Object.freeze(["all", "empty", "orphaned"]);
export const MAX_VISIBLE_ROWS = 200;
export const STEP_EPSILON_MS = 0.5;

const formatTime = ms => (ms < 0 ? "−" : "") + clock(Math.abs(ms));

/**
 * Classify every moment in the open difficulty into:
 * - matched: gameplay object edge with at least one hitsound
 * - empty: gameplay object edge with no hitsounds
 * - orphaned: hitsound(s) placed at a timestamp with no gameplay object
 */
export function analyzeCoverage(moments) {
  let matched = 0;
  let empty = 0;
  let orphaned = 0;
  const issues = [];
  const list = Array.isArray(moments) ? moments : [];

  for (let index = 0; index < list.length; index++) {
    const moment = list[index];
    if (!Array.isArray(moment)) continue;
    const time = Number(moment[0]);
    if (!Number.isFinite(time)) continue;
    const onObject = Boolean(moment[2]);
    const hits = Array.isArray(moment[3]) ? moment[3] : [];
    const hitCount = hits.length;

    if (onObject) {
      if (hitCount > 0) {
        matched++;
      } else {
        empty++;
        issues.push({
          id: `empty:${index}`,
          kind: "empty",
          time,
          clock: formatTime(time),
          hitCount: 0,
        });
      }
    } else if (hitCount > 0) {
      orphaned++;
      issues.push({
        id: `orphaned:${index}`,
        kind: "orphaned",
        time,
        clock: formatTime(time),
        hitCount,
      });
    }
  }

  issues.sort((a, b) => a.time - b.time);

  return {
    matched,
    empty,
    orphaned,
    totalObjects: matched + empty,
    issues,
  };
}

/**
 * Combine Rust's lightweight `scene.coverage` counts (including `fallbackActive` / `fallbackCovered`)
 * with the locally derived issue jump list from `scene.moments` (avoiding large IPC arrays on edits).
 */
export function normalizeCoverage(rustCoverage, moments) {
  const local = analyzeCoverage(moments);
  if (rustCoverage && typeof rustCoverage === "object") {
    const fallbackActive = Boolean(rustCoverage.fallbackActive);
    const issues = fallbackActive
      ? local.issues.filter(item => item.kind !== "empty")
      : local.issues;
    return {
      matched: Number(rustCoverage.matched) || 0,
      empty: Number(rustCoverage.empty) || 0,
      fallbackCovered: Number(rustCoverage.fallbackCovered) || 0,
      orphaned: Number(rustCoverage.orphaned) || 0,
      totalObjects: Number(rustCoverage.totalObjects) || 0,
      fallbackActive,
      issues,
    };
  }
  return { ...local, fallbackCovered: 0, fallbackActive: false };
}

/**
 * Filter coverage issues by kind ("all", "empty", or "orphaned").
 */
export function filterCoverageIssues(issues, filter = "all") {
  const list = Array.isArray(issues) ? issues : [];
  if (filter === "empty") return list.filter(item => item.kind === "empty");
  if (filter === "orphaned") return list.filter(item => item.kind === "orphaned");
  return list;
}

/**
 * Cap the visible jump-list rows so large difficulties stay snappy.
 */
export function visibleCoverageIssues(issues, limit = MAX_VISIBLE_ROWS) {
  const list = Array.isArray(issues) ? issues : [];
  const cap = Number.isInteger(limit) && limit > 0 ? limit : MAX_VISIBLE_ROWS;
  const rows = list.slice(0, cap);
  return {
    rows,
    hiddenCount: Math.max(0, list.length - rows.length),
    total: list.length,
  };
}

/**
 * Step forward (direction >= 0) or backward (direction < 0) through sorted issues from playheadMs,
 * wrapping around at either end.
 */
export function stepCoverageIssue(issues, playheadMs, direction = 1, epsilon = STEP_EPSILON_MS) {
  const list = Array.isArray(issues) ? issues : [];
  if (!list.length) return null;
  const now = Number(playheadMs);
  if (!Number.isFinite(now)) return direction >= 0 ? list[0] : list[list.length - 1];

  if (direction >= 0) {
    for (let i = 0; i < list.length; i++) {
      if (list[i].time > now + epsilon) return list[i];
    }
    return list[0];
  }

  for (let i = list.length - 1; i >= 0; i--) {
    if (list[i].time < now - epsilon) return list[i];
  }
  return list[list.length - 1];
}

export const nextCoverageIssue = (issues, playheadMs, epsilon = STEP_EPSILON_MS) =>
  stepCoverageIssue(issues, playheadMs, 1, epsilon);

export const prevCoverageIssue = (issues, playheadMs, epsilon = STEP_EPSILON_MS) =>
  stepCoverageIssue(issues, playheadMs, -1, epsilon);

/**
 * Return the id of the issue currently under the playhead (within epsilon), or null.
 */
export function activeCoverageIssueId(issues, playheadMs, epsilon = STEP_EPSILON_MS) {
  const list = Array.isArray(issues) ? issues : [];
  const now = Number(playheadMs);
  if (!list.length || !Number.isFinite(now)) return null;
  let best = null;
  let bestDist = epsilon;
  for (const item of list) {
    const dist = Math.abs(item.time - now);
    if (dist <= bestDist) {
      bestDist = dist;
      best = item.id;
    }
  }
  return best;
}

/**
 * Human-readable row label for a coverage issue.
 */
export function coverageIssueLabel(issue) {
  if (!issue) return "";
  if (issue.kind === "empty") return "Empty object";
  return issue.hitCount > 1 ? `Orphaned · ${issue.hitCount} hits` : "Orphaned hit";
}
