<script>
  import BounceSampleBank from "./BounceSampleBank.svelte";
  import OszBudgetGauge from "./OszBudgetGauge.svelte";

  let {
    open = false,
    busy = false,
    baseBytes = 0,
    bouncedBytes = 0,
    samples = [],
    format = $bindable("ogg_vorbis"),
    mode = $bindable("tiered"),
    autoTrim = $bindable(true),
    normalize = $bindable(true),
    onrender = null,
    oncancel = null,
    onaudition = null,
    oncommit = null,
    onclose = null,
  } = $props();

  const targetDbfs = $derived(format === "wav16" ? "-1.0 dBFS" : "-1.5 dBFS");
</script>

{#if open}
  <div class="vb-backdrop" role="dialog" aria-modal="true" aria-label="Bounce VST3 Drum Kit">
    <div class="vb-modal">
      <header class="vb-head">
        <h2>Bounce VST3 Drum Kit</h2>
        <button type="button" aria-label="Close" onclick={() => onclose?.()}>✕</button>
      </header>
      <div class="vb-controls">
        <label>
          Velocity mode
          <select bind:value={mode} disabled={busy}>
            <option value="tiered">Smart 3-Tier (Ghost / Mid / Hard)</option>
            <option value="exact">Exact per volume</option>
          </select>
        </label>
        <label>
          Format
          <select bind:value={format} disabled={busy}>
            <option value="ogg_vorbis">Ogg Vorbis (.ogg · -1.5 dBFS)</option>
            <option value="wav16">16-bit WAV (.wav · -1.0 dBFS)</option>
          </select>
        </label>
        <label><input type="checkbox" bind:checked={autoTrim} disabled={busy}> Auto-trim (-60 dBFS)</label>
        <label><input type="checkbox" bind:checked={normalize} disabled={busy}> Uniform kit peak normalize ({targetDbfs})</label>
      </div>
      <OszBudgetGauge {baseBytes} {bouncedBytes} />
      <BounceSampleBank {samples} {onaudition} />
      <footer class="vb-foot">
        {#if busy}
          <button type="button" onclick={() => oncancel?.()}>Cancel render</button>
        {:else}
          <button type="button" onclick={() => onrender?.({ mode, format, autoTrim, normalize })}>Render preview</button>
          <button type="button" class="primary" disabled={!samples.length} onclick={() => oncommit?.()}>
            Commit {samples.length} {samples.length === 1 ? "Sample" : "Samples"} to [Hitsounds] →
          </button>
        {/if}
      </footer>
    </div>
  </div>
{/if}

<style>
  .vb-backdrop {
    position: fixed;
    inset: 0;
    z-index: 45;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.65);
  }
  .vb-modal {
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
  .vb-head, .vb-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .vb-head h2 {
    margin: 0;
    font-size: 15px;
  }
  .vb-controls {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
    font-size: 12px;
  }
</style>
