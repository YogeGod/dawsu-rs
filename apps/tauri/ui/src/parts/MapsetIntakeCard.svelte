<script>
  import { isHitsoundVersion } from "../lib/startScreen.js";

  let {
    mapset = null,
    busy = false,
    onopen = null,
    oncreatehs = null,
    onclose = null,
  } = $props();

  const diffs = $derived(Array.isArray(mapset?.difficulties) ? mapset.difficulties : []);
  const hsDiff = $derived(diffs.find(d => isHitsoundVersion(d?.version)) ?? null);
  const nonHsDiffs = $derived(diffs.filter(d => !isHitsoundVersion(d?.version)));
  let selectedRefPath = $state("");

  $effect(() => {
    const first = nonHsDiffs[0]?.path ?? diffs[0]?.path ?? "";
    if (!selectedRefPath || !diffs.some(d => d.path === selectedRefPath)) {
      selectedRefPath = first;
    }
  });

  const heading = $derived.by(() => {
    if (!mapset) return "";
    const artist = mapset.artist?.trim();
    const title = mapset.title?.trim();
    if (artist && title) return `${artist} - ${title}`;
    return title || artist || mapset.folder || "Mapset folder";
  });

  const folderError = $derived.by(() => {
    if (!mapset) return "";
    if (mapset.corrupt || mapset.error) return mapset.error || "This mapset folder could not be read.";
    if (diffs.length === 0) return "No readable .osu difficulties found in this folder.";
    return "";
  });

  function onKeydown(event) {
    if (event.key === "Escape" && mapset) {
      event.stopPropagation();
      onclose?.();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if mapset}
  <div class="intake-backdrop" role="presentation"
    onclick={e => { if (e.target === e.currentTarget) onclose?.(); }}>
    <div class="intake-card" role="dialog" aria-modal="true" aria-label="Open mapset">
      <header class="intake-head">
        <div>
          <p class="intake-eyebrow">Mapset</p>
          <h2 class="intake-title">{heading}</h2>
        </div>
        <button type="button" class="intake-close" aria-label="Close" onclick={() => onclose?.()}>✕</button>
      </header>

      {#if folderError}
        <p class="intake-error" role="alert">{folderError}</p>
      {:else}
        <div class="intake-diffs" role="list" aria-label="Difficulties">
          {#each diffs as diff (diff.path)}
            <button type="button" class="intake-diff" disabled={busy}
              onclick={() => onopen?.(diff.path)}>
              <span class="intake-diff-name">{diff.version || diff.fileName}</span>
              {#if isHitsoundVersion(diff.version)}
                <span class="intake-badge-hs">HS</span>
              {/if}
            </button>
          {/each}
        </div>

        <div class="intake-actions">
          {#if hsDiff}
            <button type="button" class="primary" disabled={busy}
              onclick={() => onopen?.(hsDiff.path)}>
              Open [{hsDiff.version}] →
            </button>
          {:else}
            <label class="intake-ref">
              <span>Reference timing from</span>
              <select bind:value={selectedRefPath} disabled={busy}>
                {#each nonHsDiffs as diff (diff.path)}
                  <option value={diff.path}>{diff.version || diff.fileName}</option>
                {/each}
              </select>
            </label>
            <button type="button" class="primary" disabled={busy || !selectedRefPath}
              onclick={() => oncreatehs?.(selectedRefPath)}>
              Create [Hitsounds] &amp; Start Editing →
            </button>
          {/if}
        </div>

        <p class="intake-shield">Auto-backup stays on: your original mapset is snapshotted before any write.</p>
      {/if}
    </div>
  </div>
{/if}

<style>
  .intake-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.65);
    padding: 16px;
  }
  .intake-card {
    width: min(460px, 100%);
    padding: 18px 20px;
    border: 1px solid var(--editor-border);
    border-radius: 12px;
    background: var(--editor-panel);
    color: var(--editor-text);
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .intake-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .intake-eyebrow {
    margin: 0 0 2px;
    font-size: 11px;
    color: var(--editor-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .intake-title {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  .intake-close {
    min-width: 28px;
    min-height: 28px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--editor-secondary);
    cursor: pointer;
  }
  .intake-close:hover {
    background: var(--editor-hover);
    color: var(--editor-text);
  }
  .intake-error {
    margin: 0;
    padding: 10px 12px;
    border-radius: 8px;
    background: rgba(239, 68, 68, 0.14);
    color: #fca5a5;
    font-size: 12px;
  }
  .intake-diffs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .intake-diff {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 5px 10px;
    border: 1px solid var(--editor-border);
    border-radius: 999px;
    background: var(--editor-chrome);
    color: var(--editor-text);
    font-size: 12px;
    cursor: pointer;
  }
  .intake-diff:hover:not(:disabled) {
    border-color: var(--editor-accent);
  }
  .intake-badge-hs {
    padding: 1px 5px;
    border-radius: 999px;
    background: rgba(52, 199, 89, 0.18);
    color: #34c759;
    font-size: 10px;
    font-weight: 600;
  }
  .intake-actions {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .intake-ref {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 12px;
    color: var(--editor-secondary);
  }
  .intake-ref select {
    flex: 1 1 auto;
    min-height: 28px;
    padding: 0 8px;
    border-radius: 6px;
  }
  .intake-shield {
    margin: 0;
    padding: 8px 10px;
    border-radius: 8px;
    background: rgba(52, 199, 89, 0.1);
    color: #86efac;
    font-size: 11px;
  }
</style>
