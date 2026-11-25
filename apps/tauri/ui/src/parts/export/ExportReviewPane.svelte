<script>
  import { fileName } from "../../lib/export.js";

  let {
    choices,
    mixInfo,
    summary = null,
  } = $props();
</script>

<aside class="panel">
  {#if summary}
    <div class="review-summary" aria-label="Export summary">
      <div class="summary-row">
        <span class="summary-label">Included</span>
        <strong>{summary.count} {summary.count === 1 ? "difficulty" : "difficulties"}</strong>
      </div>
      {#if summary.coveragePct !== null}
        <div class="summary-row">
          <span class="summary-label">Match coverage</span>
          <strong>{summary.coveragePct}%</strong>
        </div>
        <div class="summary-bar" aria-hidden="true">
          <span class="summary-fill" style:width={`${summary.coveragePct}%`}></span>
        </div>
        <div class="summary-stats">
          <span class="stat-ok">{summary.matched} matched</span>
          <span class="stat-miss">{summary.unmatched} unmatched</span>
          <span>{summary.changed} changed</span>
        </div>
      {/if}
    </div>
  {/if}
  <h2>Write to</h2>
  <div class="write-card">
    {#if choices.writeMode === "hitsound_only"}
      <strong>Hitsound-only folder</strong>
      <p>Writes a separate folder ({choices.hitsoundOnlyDir || "dawsu! exports folder"}) with the song, samples, and hitsound <code>.osu</code> files. Your mapset folder stays untouched.</p>
    {:else}
      <strong>Into the mapset</strong>
      <p>Updates every included difficulty in place. Each changed file is backed up first.</p>
    {/if}
  </div>
  {#if mixInfo.sections.length || mixInfo.volumes.length}
    <div class="write-details" aria-label="Sample files and volumes">
      {#each mixInfo.sections as section (section.title)}
        <strong>{section.title}</strong>
        <ul>{#each section.rows as row (row.path)}<li title={row.path}>{fileName(row.path)}</li>{/each}</ul>
      {/each}
      {#if mixInfo.volumes.length}
        <strong>Volumes</strong>
        {#each mixInfo.volumes as message, index (index)}<p>{message}</p>{/each}
      {/if}
    </div>
  {/if}
</aside>

<style>
  .panel {
    padding: 28px 24px;
    border-left: 1px solid var(--editor-border);
    background: var(--editor-panel);
    overflow: auto;
  }
  h2 { margin: 0 0 12px; font-size: 13px; font-weight: 600; color: var(--editor-secondary); }
  .write-card {
    padding: 14px 16px;
    border-radius: 8px;
    background: var(--editor-control);
    box-shadow: inset 0 0 0 1px var(--editor-accent);
  }
  .write-card p { margin: 4px 0 0; color: var(--editor-muted); }
  .write-details { margin-top: 12px; padding: 0 4px; }
  .write-details strong { display: block; margin-top: 10px; }
  .write-details ul { margin: 4px 0 0; padding-left: 18px; color: var(--editor-muted); }
  .write-details p { margin: 4px 0 0; color: var(--editor-muted); }
  .review-summary {
    margin-bottom: 16px;
    padding: 12px 14px;
    border-radius: 8px;
    background: var(--editor-chrome);
    box-shadow: inset 0 0 0 1px var(--editor-border);
    display: grid;
    gap: 6px;
  }
  .summary-row {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    font-variant-numeric: tabular-nums;
  }
  .summary-label {
    color: var(--editor-muted);
    font-size: 12px;
  }
  .summary-bar {
    height: 5px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
    overflow: hidden;
  }
  .summary-fill {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--editor-play);
  }
  .summary-stats {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 12px;
    font-size: 11px;
    color: var(--editor-muted);
    font-variant-numeric: tabular-nums;
  }
  .stat-ok { color: var(--editor-play); }
  .stat-miss { color: var(--editor-error); }
</style>
