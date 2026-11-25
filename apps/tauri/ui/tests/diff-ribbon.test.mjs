import assert from "node:assert/strict";
import { test } from "node:test";
import { ribbonRowAtY, ribbonRowRects, ribbonRowSummary } from "../src/lib/diffRibbon.js";

test("ribbonRowRects caps at 4 rows and hit-tests Y cleanly", () => {
  const rects = ribbonRowRects(6, 28);
  assert.equal(rects.length, 4);
  assert.equal(ribbonRowAtY(rects[0].y + 1, 4, 28), 0);
  assert.equal(ribbonRowAtY(rects[3].y + 1, 4, 28), 3);
  assert.equal(ribbonRowAtY(100, 4, 28), -1);
});

test("ribbonRowSummary formats Ready, UnsupportedMode, and Unreadable rows", () => {
  assert.equal(
    ribbonRowSummary({ version: "Hard", status: { kind: "ready" }, buckets: [{ matched: 4, unmatched: 1 }] }),
    "Hard: 4 matched, 1 unmatched",
  );
  assert.match(
    ribbonRowSummary({ version: "4K", status: { kind: "unsupported_mode", mode: 3 }, buckets: [] }),
    /non-standard mode/,
  );
  assert.match(
    ribbonRowSummary({ version: "Bad", status: { kind: "unreadable", message: "io" }, buckets: [] }),
    /could not read file/,
  );
});
