<script>
  import { autoMatchDrumPreset, formatHostLatency, groupDrumInstruments } from "../lib/drumVst3.js";

  let {
    open = false,
    plugins = [],
    activeInstrumentId = "builtin:stock",
    sidecarAvailable = true,
    blockFrames = 512,
    sampleRate = 48000,
    onselect = null,
    onscan = null,
    onopenmap = null,
    onopenbounce = null,
    onclose = null,
  } = $props();

  const grouped = $derived(groupDrumInstruments(plugins));
  const latencyText = $derived(formatHostLatency(blockFrames, sampleRate));
</script>

{#if open}
  <section class="drum-inst-bar" aria-label="Drum instrument loader">
    <div class="dib-head">
      <strong>Drum Instrument</strong>
      <span class="dib-status">{sidecarAvailable ? `Host ready · ${latencyText}` : "Built-in mode"}</span>
      <button type="button" class="dib-btn" onclick={() => onscan?.()}>Scan VST3</button>
      <button type="button" class="dib-btn" onclick={() => onopenmap?.()}>Drum Map…</button>
      <button type="button" class="dib-btn primary" onclick={() => onopenbounce?.()}>Bounce VST3 Kit…</button>
      <button type="button" class="dib-close" aria-label="Hide drum instrument bar" onclick={() => onclose?.()}>✕</button>
    </div>
    <div class="dib-grid">
      {#each grouped.builtin as kit (kit.id)}
        <button type="button" class="dib-card" class:active={activeInstrumentId === kit.id}
          onclick={() => onselect?.({ ...kit, preset: autoMatchDrumPreset(kit.name) })}>
          <strong>{kit.name}</strong>
          <small>{kit.vendor}</small>
        </button>
      {/each}
      {#each grouped.vst3 as plugin (plugin.id)}
        <button type="button" class="dib-card" class:active={activeInstrumentId === plugin.id}
          onclick={() => onselect?.({ ...plugin, preset: autoMatchDrumPreset(plugin.name) })}>
          <strong>{plugin.name}</strong>
          <small>{plugin.vendor || "VST3"}</small>
        </button>
      {/each}
    </div>
  </section>
{/if}

<style>
  .drum-inst-bar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-panel);
    color: var(--editor-text);
    font-size: 12px;
  }
  .dib-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dib-status {
    margin-right: auto;
    color: var(--editor-muted);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .dib-btn, .dib-close {
    min-height: 24px;
    padding: 0 8px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-chrome);
    color: var(--editor-text);
    font-size: 11px;
    cursor: pointer;
  }
  .dib-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .dib-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 6px 10px;
    border: 1px solid var(--editor-border);
    border-radius: 8px;
    background: var(--editor-chrome);
    color: var(--editor-text);
    cursor: pointer;
  }
  .dib-card.active {
    border-color: var(--editor-accent);
    background: var(--editor-selected);
  }
  .dib-card small {
    color: var(--editor-muted);
    font-size: 10px;
  }
</style>
