import assert from "node:assert/strict";
import { test } from "node:test";
import { render } from "svelte/server";
import { bridge } from "../src/lib/bridge.svelte.js";
import {
  MAX_VISIBLE_ROWS,
  activeCoverageIssueId,
  analyzeCoverage,
  coverageIssueLabel,
  filterCoverageIssues,
  nextCoverageIssue,
  prevCoverageIssue,
  visibleCoverageIssues,
} from "../src/lib/coverageReview.js";
import CoverageReview from "../src/parts/CoverageReview.svelte";

test("analyzeCoverage classifies matched, empty, and orphaned moments and sorts issues by time", () => {
  const moments = [
    [2500, 80, true, []], // empty object
    [1000, 80, true, [[0, false]]], // matched object
    [1500, 80, false, [[1, false], [2, false]]], // orphaned (2 hits)
    [1800, 80, false, []], // empty non-object placeholder -> ignored
    [3000, 80, true, [[0, false], [3, true]]], // matched object
    [500, 80, false, [[0, false]]], // orphaned (1 hit)
  ];

  const summary = analyzeCoverage(moments);
  assert.equal(summary.matched, 2);
  assert.equal(summary.empty, 1);
  assert.equal(summary.orphaned, 2);
  assert.equal(summary.totalObjects, 3);
  assert.deepEqual(
    summary.issues.map(item => [item.kind, item.time, item.clock, item.hitCount]),
    [
      ["orphaned", 500, "00:00.500", 1],
      ["orphaned", 1500, "00:01.500", 2],
      ["empty", 2500, "00:02.500", 0],
    ],
  );
  assert.equal(coverageIssueLabel(summary.issues[0]), "Orphaned hit");
  assert.equal(coverageIssueLabel(summary.issues[1]), "Orphaned · 2 hits");
  assert.equal(coverageIssueLabel(summary.issues[2]), "Empty object");
});

test("filterCoverageIssues and visibleCoverageIssues filter cleanly and cap rows", () => {
  const moments = [];
  for (let i = 0; i < 210; i++) {
    moments.push([i * 100, 80, true, []]);
  }
  moments.push([99999, 80, false, [[0, false]]]);

  const { issues } = analyzeCoverage(moments);
  assert.equal(filterCoverageIssues(issues, "all").length, 211);
  assert.equal(filterCoverageIssues(issues, "empty").length, 210);
  assert.equal(filterCoverageIssues(issues, "orphaned").length, 1);

  const visible = visibleCoverageIssues(issues, MAX_VISIBLE_ROWS);
  assert.equal(visible.rows.length, MAX_VISIBLE_ROWS);
  assert.equal(visible.hiddenCount, 11);
  assert.equal(visible.total, 211);
});

test("nextCoverageIssue and prevCoverageIssue step and wrap around from any playhead time", () => {
  const { issues } = analyzeCoverage([
    [1000, 80, true, []],
    [2000, 80, false, [[0, false]]],
    [4000, 80, true, []],
  ]);

  assert.equal(nextCoverageIssue(issues, 0)?.time, 1000);
  assert.equal(nextCoverageIssue(issues, 1000)?.time, 2000, "moves past the current issue");
  assert.equal(nextCoverageIssue(issues, 1000.2)?.time, 2000, "respects epsilon tolerance");
  assert.equal(nextCoverageIssue(issues, 2500)?.time, 4000);
  assert.equal(nextCoverageIssue(issues, 4000)?.time, 1000, "wraps to the first issue");
  assert.equal(nextCoverageIssue(issues, 9000)?.time, 1000, "wraps after the last issue");
  assert.equal(nextCoverageIssue(issues, NaN)?.time, 1000);

  assert.equal(prevCoverageIssue(issues, 5000)?.time, 4000);
  assert.equal(prevCoverageIssue(issues, 4000)?.time, 2000, "moves before the current issue");
  assert.equal(prevCoverageIssue(issues, 2000.2)?.time, 1000, "respects epsilon tolerance");
  assert.equal(prevCoverageIssue(issues, 1000)?.time, 4000, "wraps to the last issue");
  assert.equal(prevCoverageIssue(issues, 0)?.time, 4000, "wraps before the first issue");
  assert.equal(prevCoverageIssue(issues, NaN)?.time, 4000);

  assert.equal(nextCoverageIssue([], 1000), null);
  assert.equal(prevCoverageIssue([], 1000), null);
});

test("activeCoverageIssueId finds the issue currently under the playhead", () => {
  const { issues } = analyzeCoverage([
    [1000, 80, true, []],
    [2500, 80, false, [[0, false]]],
  ]);
  assert.equal(activeCoverageIssueId(issues, 1000), issues[0].id);
  assert.equal(activeCoverageIssueId(issues, 2500.3), issues[1].id);
  assert.equal(activeCoverageIssueId(issues, 1750), null);
});

test("CoverageReview renders counts, filter chips, issue rows, and stepping controls via SSR", () => {
  const prevScene = bridge.scene;
  const prevView = bridge.view;
  const prevFilter = bridge.coverageFilter;
  const prevFocus = bridge.coverageFocusTime;

  try {
    bridge.scene = {
      path: "song.osu",
      moments: [
        [1000, 80, true, [[0, false]]],
        [2000, 80, true, []],
        [3500, 80, false, [[1, false], [2, false]]],
      ],
    };
    bridge.view = { busy: false, fault: null, position: 2000 };
    bridge.coverageFilter = "all";
    bridge.coverageFocusTime = null;

    const html = render(CoverageReview).body;
    assert.match(html, /aria-controls="inspector-coverage-body"/);
    assert.match(html, /Coverage/);
    assert.match(html, /2 issues/);
    assert.match(html, /Covered[\s\S]*?>1</);
    assert.match(html, /Empty[\s\S]*?>1</);
    assert.match(html, /Orphaned[\s\S]*?>1</);
    assert.match(html, /All issues \(2\)/);
    assert.match(html, /Empty \(1\)/);
    assert.match(html, /Orphaned \(1\)/);
    assert.match(html, /title="Previous coverage issue \(Shift\+N\)"/);
    assert.match(html, /title="Next coverage issue \(N\)"/);
    assert.match(html, /aria-current="true"[\s\S]*?Empty object[\s\S]*?00:02\.000/);
    assert.match(html, /Orphaned · 2 hits[\s\S]*?00:03\.500/);
    assert.doesNotMatch(html, /\sstyle="/);
    assert.doesNotMatch(html, /data-(action|command|volume|doc|map)\b/);

    // Filter to orphaned only.
    bridge.coverageFilter = "orphaned";
    const orphanedHtml = render(CoverageReview).body;
    assert.doesNotMatch(orphanedHtml, /Empty object/);
    assert.match(orphanedHtml, /Orphaned · 2 hits/);

    // Clean difficulty shows "All clear".
    bridge.scene = { path: "clean.osu", moments: [[1000, 80, true, [[0, false]]]] };
    bridge.coverageFilter = "all";
    const cleanHtml = render(CoverageReview).body;
    assert.match(cleanHtml, /All clear/);
    assert.match(cleanHtml, /All objects covered and no orphaned hits\./);
  } finally {
    bridge.scene = prevScene;
    bridge.view = prevView;
    bridge.coverageFilter = prevFilter;
    bridge.coverageFocusTime = prevFocus;
  }
});
