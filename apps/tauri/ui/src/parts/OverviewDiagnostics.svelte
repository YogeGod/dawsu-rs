<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { normalizeCoverage } from "../lib/coverageReview.js";
  import { emptyInspector } from "../lib/inspector.js";
  import { consolidateDiagnostics, stepDiagnostic } from "../lib/inspectorV2.js";

  const problemStrip = $derived((bridge.inspector ?? emptyInspector).problemStrip);
  const coverage = $derived(normalizeCoverage(bridge.scene?.coverage, bridge.scene?.moments ?? []));
  const diag = $derived(consolidateDiagnostics(problemStrip, coverage));
  const disabled = $derived(bridge.disabled);
  let activePoint = $state(null);

  function step(direction) {
    if (disabled || !diag.points.length) return;
    const now = bridge.latestView?.position ?? bridge.view?.position ?? 0;
    const next = stepDiagnostic(diag.points, now, direction);
    if (!next) return;
    activePoint = next;
    globalThis.inspectorAction?.("jump", next.time);
  }

  function autoFix() {
    if (disabled) return;
    const target = activePoint?.fixable ? activePoint : diag.points.find(p => p.fixable);
    if (!target) return;
    act({ kind: "auto_fix_problem", time_ms: target.time });
  }

  const hasFixable = $derived(diag.points.some(p => p.fixable));
</script>

<div class="ov-diag">
  <span class="ov-title">Objects overview</span>
  {#if bridge.scene}
    <span class="ov-badge" class:ov-badge-warn={diag.total > 0} class:ov-badge-ok={diag.total === 0}
      title="{diag.problemCount} conflicts, {diag.orphanedCount} orphaned, {diag.emptyCount} empty">
      {diag.total === 0 ? "All clear" : `${diag.total} ${diag.total === 1 ? "issue" : "issues"}`}
    </span>
    {#if diag.total > 0}
      <button type="button" class="ov-btn" aria-label="Previous issue" title="Previous issue (Shift+F8)"
        {disabled} onclick={() => step(-1)}>◀</button>
      <button type="button" class="ov-btn" aria-label="Next issue" title="Next issue (F8)"
        {disabled} onclick={() => step(1)}>▶</button>
      {#if hasFixable}
        <button type="button" class="ov-fix" title="Fix selected or next orphaned/empty issue"
          {disabled} onclick={autoFix}>Fix</button>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .ov-diag {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    min-width: 0;
    font-size: 11px;
  }
  .ov-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ov-badge {
    margin-left: auto;
    padding: 1px 6px;
    border-radius: 999px;
    font-size: 10px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .ov-badge-ok {
    background: rgba(52, 199, 89, 0.14);
    color: #34c759;
  }
  .ov-badge-warn {
    background: rgba(245, 158, 11, 0.18);
    color: #fbbf24;
  }
  .ov-btn,
  .ov-fix {
    min-height: 18px;
    padding: 0 5px;
    border: 1px solid var(--editor-border);
    border-radius: 4px;
    background: var(--editor-panel);
    color: var(--editor-text);
    font-size: 10px;
    line-height: 1;
    cursor: pointer;
  }
  .ov-fix {
    border-color: rgba(245, 158, 11, 0.45);
    color: #fbbf24;
  }
  .ov-btn:hover:not(:disabled),
  .ov-fix:hover:not(:disabled) {
    background: var(--editor-hover);
  }
</style>
