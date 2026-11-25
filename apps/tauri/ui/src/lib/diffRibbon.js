// Pure 28 px Live 4-Diff Mini-Ribbon layout and hit-test helpers for <canvas id="diff-ribbon">.

export const RIBBON_HEIGHT_PX = 28;
export const MAX_RIBBON_ROWS = 4;

/**
 * Computes per-row pixel rectangles for up to 4 sibling diffs inside the 28 px mini-ribbon.
 */
export function ribbonRowRects(rowCount, totalHeight = RIBBON_HEIGHT_PX) {
  const count = Math.min(MAX_RIBBON_ROWS, Math.max(0, Number(rowCount) || 0));
  if (count === 0) return [];
  const gap = 1;
  const rowHeight = Math.max(4, Math.floor((totalHeight - gap * (count - 1)) / count));
  return Array.from({ length: count }, (_, index) => ({
    index,
    y: index * (rowHeight + gap),
    height: rowHeight,
  }));
}

/**
 * Returns the row index under pointer Y inside the mini-ribbon, or -1 if none.
 */
export function ribbonRowAtY(y, rowCount, totalHeight = RIBBON_HEIGHT_PX) {
  const rects = ribbonRowRects(rowCount, totalHeight);
  for (const rect of rects) {
    if (y >= rect.y && y <= rect.y + rect.height) return rect.index;
  }
  return -1;
}

/**
 * Summarizes a sibling ribbon row for accessible tooltips.
 */
export function ribbonRowSummary(row) {
  if (!row) return "";
  if (row.status?.kind === "unsupported_mode") {
    return `${row.version}: non-standard mode (preview unavailable)`;
  }
  if (row.status?.kind === "unreadable") {
    return `${row.version}: could not read file`;
  }
  let matched = 0;
  let unmatched = 0;
  for (const b of row.buckets ?? []) {
    matched += b.matched || 0;
    unmatched += b.unmatched || 0;
  }
  return `${row.version}: ${matched} matched, ${unmatched} unmatched`;
}
