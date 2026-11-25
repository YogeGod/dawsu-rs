<script>
  import { dedupPercent } from "../lib/bounce.js";
  import BounceSampleBank from "./BounceSampleBank.svelte";
  import OszBudgetGauge from "./OszBudgetGauge.svelte";

  let {
    open = false,
    busy = false,
    sourceNotes = 0,
    baseBytes = 0,
    bouncedBytes = 0,
    samples = [],
    velocityBuckets = $bindable(4),
    durationBucketMs = $bindable(50),
    flattenChords = $bindable(true),
    format = $bindable("ogg_vorbis"),
    onrender = null,
    oncancel = null,
    onaudition = null,
    oncommit = null,
    onclose = null,
  } = $props();

  const savedPct = $derived(dedupPercent(sourceNotes, samples.length));
  const targetDbfs = $derived(format === "wav16" ? "-1.0 dBFS" : "-1.5 dBFS");
</script>

{#if open}
  <div class="kb-backdrop" role="dialog" aria-modal="true" aria-label="Bounce and Deduplicate Keysounds">
    <div class="kb-modal">
      <header class="kb-head">
        <h2>Bounce &amp; Deduplicate Keysounds</h2>
        <button type="button" aria-label="Close" onclick={() => onclose?.()}>✕</button>
      </header>
      <p class="kb-stats">
        {sourceNotes} MIDI notes → <strong>{samples.length}</strong> unique samples ({savedPct}% deduplicated)
      </p>
      <div class="kb-controls">
        <label>
          Velocity buckets
          <input type="number" min="1" max="16" bind:value={velocityBuckets} disabled={busy}>
        </label>
        <label>
          Duration step (ms)
          <input type="number" min="25" max="500" step="25" bind:value={durationBucketMs} disabled={busy}>
        </label>
        <label>
          Format
          <select bind:value={format} disabled={busy}>
            <option value="ogg_vorbis">Ogg Vorbis (.ogg · -1.5 dBFS)</option>
            <option value="wav16">16-bit WAV (.wav · -1.0 dBFS)</option>
          </select>
        </label>
        <label><input type="checkbox" bind:checked={flattenChords} disabled={busy}> Flatten chords (uniform gain {targetDbfs})</label>
      </div>
      <OszBudgetGauge {baseBytes} {bouncedBytes} />
      <BounceSampleBank {samples} {onaudition} />
      <footer class="kb-foot">
        {#if busy}
          <button type="button" onclick={() => oncancel?.()}>Cancel render</button>
        {:else}
          <button type="button" onclick={() => onrender?.({ velocityBuckets, durationBucketMs, flattenChords, format })}>Render preview</button>
          <button type="button" class="primary" disabled={!samples.length} onclick={() => oncommit?.()}>
            Commit {samples.length} Keysounds to [Hitsounds] →
          </button>
        {/if}
      </footer>
    </div>
  </div>
{/if}

<style>
  .kb-backdrop {
    position: fixed;
    inset: 0;
    z-index: 45;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.65);
  }
  .kb-modal {
    width: min(620px, 94vw);
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    border: 1px solid var(--editor-border);
    border-radius: 12px;
    background: var(--editor-panel);
    color: var(--editor-text);
  }
  .kb-head, .kb-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .kb-head h2 {
    margin: 0;
    font-size: 15px;
  }
  .kb-stats {
    margin: 0;
    font-size: 12px;
    color: var(--editor-secondary);
  }
  .kb-controls {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    font-size: 12px;
  }
</style>
