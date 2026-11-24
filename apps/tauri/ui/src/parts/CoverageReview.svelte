<script>
  import ChevronDown from "../icons/ChevronDown.svg?raw";
  import { bridge } from "../lib/bridge.svelte.js";
  import {
    MAX_VISIBLE_ROWS,
    activeCoverageIssueId,
    coverageIssueLabel,
    filterCoverageIssues,
    normalizeCoverage,
    stepCoverageIssue,
    visibleCoverageIssues,
  } from "../lib/coverageReview.js";
  import "./coverage-review.css";

  let open = $state(false);

  const moments = $derived(bridge.scene?.moments ?? []);
  const summary = $derived(normalizeCoverage(bridge.scene?.coverage, moments));
  const filter = $derived(bridge.coverageFilter ?? "all");
  const filtered = $derived(filterCoverageIssues(summary.issues, filter));
  const visible = $derived(visibleCoverageIssues(filtered, MAX_VISIBLE_ROWS));
  const currentId = $derived(activeCoverageIssueId(visible.rows, bridge.coverageFocusTime ?? bridge.view?.position));
  const disabled = $derived(bridge.disabled);
  const issueCount = $derived(summary.issues.length);

  function setFilter(next) {
    bridge.coverageFilter = next;
  }

  function jumpTo(time) {
    if (disabled || !Number.isFinite(time)) return;
    bridge.coverageFocusTime = time;
    globalThis.inspectorAction?.("jump", time);
  }

  function step(direction) {
    if (disabled || !filtered.length) return;
    const playheadMs = bridge.latestView?.position ?? bridge.view?.position ?? 0;
    const target = stepCoverageIssue(filtered, playheadMs, direction);
    if (target) jumpTo(target.time);
  }

  const emptyMessage = $derived(
    !bridge.scene
      ? "Open a difficulty to review coverage."
      : filter === "empty"
        ? "No empty objects."
        : filter === "orphaned"
          ? "No orphaned hits."
          : "All objects covered and no orphaned hits."
  );
</script>

<div class="cov">
  <button type="button" class="cov-head" aria-expanded={open} aria-controls="inspector-coverage-body"
    onclick={() => (open = !open)}>
    <span class="cov-chevron" class:cov-folded={!open} aria-hidden="true">{@html ChevronDown}</span>
    <span class="cov-title">Coverage</span>
    {#if bridge.scene}
      <span class="cov-badge" class:cov-badge-warn={issueCount > 0} class:cov-badge-ok={issueCount === 0}
        title="{summary.matched} covered, {summary.empty} empty, {summary.orphaned} orphaned">
        {issueCount === 0 ? "All clear" : `${issueCount} ${issueCount === 1 ? "issue" : "issues"}`}
      </span>
    {/if}
  </button>
  <div id="inspector-coverage-body" class="cov-body" hidden={!open}>
    <div class="cov-stats" aria-label="Coverage counts">
      <div class="cov-stat cov-stat-matched">
        <span class="cov-stat-label">Covered</span>
        <span class="cov-stat-value">{summary.matched}</span>
      </div>
      <div class="cov-stat cov-stat-empty">
        <span class="cov-stat-label">Empty{summary.fallbackActive ? " (fallback)" : ""}</span>
        <span class="cov-stat-value">{summary.empty}</span>
      </div>
      <div class="cov-stat cov-stat-orphaned">
        <span class="cov-stat-label">Orphaned</span>
        <span class="cov-stat-value">{summary.orphaned}</span>
      </div>
    </div>

    <div class="cov-filters" role="group" aria-label="Filter coverage issues">
      <button type="button" class="cov-chip" aria-pressed={filter === "all"}
        onclick={() => setFilter("all")}>All issues ({summary.issues.length})</button>
      <button type="button" class="cov-chip" aria-pressed={filter === "empty"}
        onclick={() => setFilter("empty")}>Empty ({summary.empty})</button>
      <button type="button" class="cov-chip" aria-pressed={filter === "orphaned"}
        onclick={() => setFilter("orphaned")}>Orphaned ({summary.orphaned})</button>
    </div>

    <div class="cov-nav">
      <button type="button" disabled={disabled || !filtered.length} title="Previous coverage issue (Shift+N)"
        onclick={() => step(-1)}>Prev</button>
      <button type="button" disabled={disabled || !filtered.length} title="Next coverage issue (N)"
        onclick={() => step(1)}>Next</button>
    </div>

    {#if visible.rows.length}
      <div class="cov-list" role="group" aria-label="Coverage issues">
        {#each visible.rows as row (row.id)}
          <button type="button" class="cov-row" aria-current={row.id === currentId ? "true" : undefined}
            {disabled} onclick={() => jumpTo(row.time)}>
            <span class="cov-dot cov-dot-{row.kind}" aria-hidden="true"></span>
            <span class="cov-row-label">{coverageIssueLabel(row)}</span>
            <time class="cov-row-time">{row.clock}</time>
          </button>
        {/each}
      </div>
      {#if visible.hiddenCount > 0}
        <p class="cov-more">Showing first {visible.rows.length} of {visible.total}. Use N / Shift+N to step all.</p>
      {/if}
    {:else}
      <p class="cov-empty">{emptyMessage}</p>
    {/if}
  </div>
</div>
