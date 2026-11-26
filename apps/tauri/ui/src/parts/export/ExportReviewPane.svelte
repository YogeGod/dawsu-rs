<script>
  import { fileName } from "../../lib/export.js";

  let {
    choices,
    mixInfo,
    summary = null,
    diffSummary = null,
    verification = null,
  } = $props();
</script>

<aside class="panel">
  {#if summary}
    <div class="review-summary" aria-label="Export summary">
      <div class="summary-row">
        <span class="summary-label">Included</span>
        <strong>{summary.count} {summary.count === 1 ? "difficulty" : "difficulties"}</strong>
      </div>
      {#if diffSummary && diffSummary.totalFiles > 0}
        <div class="summary-row">
          <span class="summary-label">.osu diff</span>
          {#if diffSummary.added === 0 && diffSummary.removed === 0}
            <strong>{diffSummary.anyByteChanged ? "Line endings only" : "No .osu changes"}</strong>
          {:else}
            <strong class="diff-totals">
              <span class="stat-ok">+{diffSummary.added}</span> / <span class="stat-miss">-{diffSummary.removed}</span> lines
            </strong>
          {/if}
        </div>
      {/if}
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
  {#if verification}
    <section
      class="verify-card"
      class:ok={verification.passed}
      class:fail={!verification.passed}
      aria-label="Pre-write read-back check"
      role={verification.passed ? null : "alert"}
    >
      <strong class="verify-title">
        {verification.passed ? "Read-back check passed" : "Read-back check failed — Export blocked"}
      </strong>
      <p class="verify-meta">
        {#if verification.passed}
          {verification.checkedFiles} {verification.checkedFiles === 1 ? "difficulty" : "difficulties"} verified · {verification.momentsChecked} moments · {verification.bodiesChecked} slider bodies
        {:else}
          {verification.failedFiles} of {verification.checkedFiles} {verification.checkedFiles === 1 ? "difficulty" : "difficulties"} failed
        {/if}
      </p>
      {#if (verification.files?.length ?? 0) > 0}
        <details class="verify-details" open={!verification.passed}>
          <summary>Check details</summary>
          <ul class="verify-files">
            {#each verification.files ?? [] as file (file.path)}
              <li class="verify-file">
                <span class="verify-file-name" title={file.path}>{file.fileName || fileName(file.path)}</span>
                <ul class="verify-checks">
                  {#each file.checks ?? [] as check (check.id)}
                    <li class:ok={check.passed} class:fail={!check.passed}>
                      <span class="verify-icon" aria-hidden="true">{check.passed ? "✓" : "✕"}</span>
                      <span class="verify-check-label">{check.label}</span>
                      {#if check.detail}
                        <span class="verify-check-detail">{check.detail}</span>
                      {/if}
                    </li>
                  {/each}
                </ul>
              </li>
            {/each}
          </ul>
        </details>
      {:else if !verification.passed && verification.firstError}
        <p class="verify-error-summary">{verification.firstError}</p>
      {/if}
    </section>
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
  .write-details { margin-top: 14px; display: grid; gap: 6px; }
  .write-details strong { display: block; margin-top: 8px; font-size: 12px; color: var(--editor-secondary); }
  .write-details ul {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: grid;
    gap: 5px;
  }
  .write-details li {
    padding: 7px 10px;
    border-radius: 7px;
    background: var(--editor-chrome);
    border: 1px solid var(--editor-border);
    color: var(--editor-text);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .write-details p { margin: 4px 0 0; color: var(--editor-muted); font-size: 12px; }
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
  .diff-totals { font-variant-numeric: tabular-nums; }
  .verify-card {
    margin-bottom: 16px;
    padding: 12px 14px;
    border-radius: 8px;
    background: var(--editor-chrome);
    display: grid;
    gap: 6px;
  }
  .verify-card.ok {
    box-shadow: inset 0 0 0 1px rgba(52, 199, 89, .38);
  }
  .verify-card.fail {
    background: rgba(255, 69, 58, .08);
    box-shadow: inset 0 0 0 1px rgba(255, 69, 58, .5);
  }
  .verify-title { font-size: 12px; }
  .verify-card.ok .verify-title { color: var(--editor-play); }
  .verify-card.fail .verify-title { color: var(--editor-error); }
  .verify-meta {
    margin: 0;
    font-size: 11px;
    color: var(--editor-muted);
    font-variant-numeric: tabular-nums;
  }
  .verify-details summary {
    cursor: pointer;
    font-size: 11px;
    color: var(--editor-secondary);
    user-select: none;
  }
  .verify-files,
  .verify-checks {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: grid;
    gap: 6px;
  }
  .verify-file {
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--editor-panel);
    border: 1px solid var(--editor-border);
  }
  .verify-file-name {
    display: block;
    font-size: 11px;
    font-weight: 600;
    color: var(--editor-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .verify-checks li {
    display: grid;
    grid-template-columns: 12px auto 1fr;
    gap: 4px 6px;
    align-items: baseline;
    font-size: 11px;
    color: var(--editor-muted);
  }
  .verify-checks li.ok .verify-icon { color: var(--editor-play); }
  .verify-checks li.fail .verify-icon,
  .verify-checks li.fail .verify-check-label,
  .verify-checks li.fail .verify-check-detail { color: var(--editor-error); }
  .verify-check-label { color: var(--editor-secondary); font-weight: 600; }
  .verify-check-detail { min-width: 0; overflow-wrap: anywhere; }
  .verify-error-summary { margin: 0; font-size: 11px; color: var(--editor-error); overflow-wrap: anywhere; }
</style>
