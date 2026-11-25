<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { normalizeCoverage } from "../lib/coverageReview.js";
  import { ribbonRowRects, ribbonRowSummary } from "../lib/diffRibbon.js";
  import { emptyInspector } from "../lib/inspector.js";
  import { consolidateDiagnostics, stepDiagnostic } from "../lib/inspectorV2.js";
  import { invoke } from "../lib/ipc.js";

  const problemStrip = $derived((bridge.inspector ?? emptyInspector).problemStrip);
  const coverage = $derived(normalizeCoverage(bridge.scene?.coverage, bridge.scene?.moments ?? []));
  const diag = $derived(consolidateDiagnostics(problemStrip, coverage));
  const disabled = $derived(bridge.disabled);
  const scenePath = $derived(bridge.scene?.path ?? null);
  let activeSelection = $state(null);
  let ribbonOpen = $state(false);
  let ribbonRows = $state.raw([]);

  // Clear stale selection whenever difficulty changes or the issue no longer exists.
  const validActivePoint = $derived(
    activeSelection && activeSelection.path === scenePath
      ? diag.points.find(p => p.time === activeSelection.time && p.kind === activeSelection.kind) ?? null
      : null,
  );
  const fixTarget = $derived(validActivePoint?.fixable ? validActivePoint : diag.points.find(p => p.fixable) ?? null);
  const siblings = $derived((bridge.scene?.difficulties ?? []).filter(p => p !== scenePath).slice(0, 4));
  const ribbonRects = $derived(ribbonRowRects(ribbonRows.length));

  $effect(() => {
    void scenePath;
    ribbonRows = [];
    ribbonOpen = false;
  });

  function step(direction) {
    if (disabled || !diag.points.length) return;
    const now = bridge.latestView?.position ?? bridge.view?.position ?? 0;
    const next = stepDiagnostic(diag.points, now, direction);
    if (!next) return;
    activeSelection = { path: scenePath, time: next.time, kind: next.kind };
    globalThis.inspectorAction?.("jump", next.time);
  }

  function autoFix() {
    if (disabled || !fixTarget) return;
    const time = fixTarget.time;
    activeSelection = null;
    act({ kind: "auto_fix_problem", time_ms: time });
  }

  async function toggleRibbon() {
    if (disabled || !siblings.length) return;
    ribbonOpen = !ribbonOpen;
    if (!ribbonOpen) return;
    try {
      const hitTimes = (bridge.scene?.moments ?? [])
        .filter(([, , , hits]) => Array.isArray(hits) && hits.length > 0)
        .map(([time]) => time);
      const durationMs = bridge.latestView?.duration ?? bridge.view?.duration ?? 180000;
      ribbonRows = await invoke("preview_diff_ribbon", {
        siblings,
        sourceHitTimesMs: hitTimes,
        durationMs,
        toleranceMs: 5.0,
        bins: 64,
      });
    } catch {
      ribbonRows = [];
    }
  }

  const fixLabel = $derived(fixTarget?.kind === "empty" ? "Add normal" : "Fix orphan");
  const fixTitle = $derived(
    fixTarget?.kind === "empty"
      ? "Add a default hitnormal to this empty object"
      : "Snap orphaned hit to nearest object within 50 ms, or remove if un-snappable (Undoable)",
  );
</script>

<div class="ov-diag">
  <span class="ov-title">Objects overview</span>
  {#if bridge.scene}
    <span class="ov-badge" class:ov-badge-warn={diag.total > 0} class:ov-badge-ok={diag.total === 0}
      title="{diag.problemCount} conflicts, {diag.orphanedCount} orphaned, {diag.emptyCount} empty">
      {diag.total === 0 ? "All clear" : `${diag.total} ${diag.total === 1 ? "issue" : "issues"}`}
    </span>
    {#if siblings.length > 0}
      <button type="button" class="ov-btn" aria-pressed={ribbonOpen} title="Preview coverage across up to 4 sibling difficulties"
        {disabled} onclick={toggleRibbon}>Diffs ({siblings.length})</button>
    {/if}
    {#if diag.total > 0}
      <button type="button" class="ov-btn" aria-label="Previous issue" title="Previous issue (Shift+F8)"
        {disabled} onclick={() => step(-1)}>◀</button>
      <button type="button" class="ov-btn" aria-label="Next issue" title="Next issue (F8)"
        {disabled} onclick={() => step(1)}>▶</button>
      {#if fixTarget}
        <button type="button" class="ov-fix" title={fixTitle}
          {disabled} onclick={autoFix}>{fixLabel}</button>
      {/if}
    {/if}
  {/if}
</div>
{#if ribbonOpen && ribbonRows.length > 0}
  <div class="ov-ribbon" role="list" aria-label="Sibling difficulty coverage">
    {#each ribbonRows as row, idx (row.path)}
      <div class="ov-ribbon-row" role="listitem" title={ribbonRowSummary(row)}
        style:min-height="{ribbonRects[idx]?.height ?? 6}px">
        <span class="ov-ribbon-name">{row.version}</span>
        <span class="ov-ribbon-meta">{ribbonRowSummary(row)}</span>
      </div>
    {/each}
  </div>
{/if}

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
  .ov-ribbon {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 4px;
    padding: 4px 6px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-chrome);
    font-size: 10px;
  }
  .ov-ribbon-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .ov-ribbon-name {
    font-weight: 600;
    color: var(--editor-text);
  }
  .ov-ribbon-meta {
    color: var(--editor-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
