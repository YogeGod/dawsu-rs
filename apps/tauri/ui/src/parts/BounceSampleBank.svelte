<script>
  let { samples = [], onaudition = null } = $props();
</script>

<div class="bsb-list" role="list" aria-label="Bounced samples">
  {#each samples as sample (sample.filename)}
    <div class="bsb-row" role="listitem">
      <button type="button" class="bsb-play" aria-label={"Audition " + sample.filename}
        onclick={() => onaudition?.(sample)}>▶</button>
      <div class="bsb-info">
        <strong>{sample.filename}</strong>
        <small>
          {sample.durationMs ?? 0} ms · {sample.peakDbfs != null ? `${sample.peakDbfs.toFixed(1)} dBFS` : "—"}
          {#if sample.silent}<span class="bsb-silent">Silent render</span>{/if}
        </small>
      </div>
      {#if sample.usageCount != null}
        <span class="bsb-use">{sample.usageCount}×</span>
      {/if}
    </div>
  {/each}
</div>

<style>
  .bsb-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 220px;
    overflow-y: auto;
  }
  .bsb-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 6px;
    background: var(--editor-chrome);
    font-size: 12px;
  }
  .bsb-play {
    min-width: 24px;
    min-height: 24px;
    border: 1px solid var(--editor-border);
    border-radius: 999px;
    background: var(--editor-panel);
    color: var(--editor-text);
    cursor: pointer;
  }
  .bsb-info {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1 1 auto;
  }
  .bsb-info small {
    color: var(--editor-muted);
    font-size: 10px;
  }
  .bsb-silent {
    margin-left: 6px;
    color: #fbbf24;
    font-weight: 600;
  }
  .bsb-use {
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--editor-panel);
    color: var(--editor-secondary);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
  }
</style>
