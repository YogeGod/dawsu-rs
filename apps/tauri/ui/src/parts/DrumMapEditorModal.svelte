<script>
  import { onDestroy } from "svelte";
  import { CHOKE_GROUP_OPTIONS, formatChokeGroup, formatMidiNoteLabel, groupDrumPresets } from "../lib/drumMapEditor.js";

  let {
    open = false,
    presets = [],
    rows = [],
    onselectpreset = null,
    ontest = null,
    onmidilearn = null,
    oncancellearn = null,
    onsetchoke = null,
    onimport = null,
    onexport = null,
    onclose = null,
  } = $props();

  let learningRow = $state(null);
  const grouped = $derived(groupDrumPresets(presets));

  function cancelLearn() {
    if (learningRow !== null) {
      learningRow = null;
      oncancellearn?.();
    }
  }

  function startLearn(index) {
    if (learningRow === index) {
      cancelLearn();
      return;
    }
    learningRow = index;
    onmidilearn?.(index);
  }

  function closeModal() {
    cancelLearn();
    onclose?.();
  }

  function onKeydown(event) {
    if (event.key === "Escape" && open) {
      if (learningRow !== null) cancelLearn();
      else closeModal();
    }
  }

  onDestroy(() => cancelLearn());
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div class="dme-backdrop" role="dialog" aria-modal="true" aria-label="Drum Map Editor">
    <div class="dme-modal">
      <header class="dme-head">
        <h2>Drum Map Editor</h2>
        <select aria-label="Drum map preset" onchange={e => onselectpreset?.(e.currentTarget.value)}>
          {#if grouped.free.length}
            <optgroup label="Free &amp; Standard">
              {#each grouped.free as preset (preset.id ?? preset.name)}
                <option value={preset.id ?? preset.name}>{preset.name}</option>
              {/each}
            </optgroup>
          {/if}
          {#if grouped.pro.length}
            <optgroup label="GGD &amp; Pro">
              {#each grouped.pro as preset (preset.id ?? preset.name)}
                <option value={preset.id ?? preset.name}>{preset.name}</option>
              {/each}
            </optgroup>
          {/if}
        </select>
        <button type="button" onclick={() => onimport?.()}>Import…</button>
        <button type="button" onclick={() => onexport?.()}>Save .dawsudrum…</button>
        <button type="button" aria-label="Close" onclick={closeModal}>✕</button>
      </header>
      <div class="dme-table">
        {#each rows as row, index (row.id ?? index)}
          <div class="dme-row">
            <span class="dme-name">{row.name}</span>
            <span class="dme-note">{formatMidiNoteLabel(row.midiNote)}</span>
            <button type="button" onclick={() => ontest?.(index)}>▶ Test</button>
            <button type="button" class:listening={learningRow === index} onclick={() => startLearn(index)}>
              {learningRow === index ? "Listening…" : "MIDI Learn"}
            </button>
            <select aria-label={"Choke group for " + row.name} value={row.chokeGroup ?? ""}
              onchange={e => onsetchoke?.(index, e.currentTarget.value ? Number(e.currentTarget.value) : null)}>
              {#each CHOKE_GROUP_OPTIONS as opt (opt.label)}
                <option value={opt.id ?? ""}>{formatChokeGroup(opt.id)}</option>
              {/each}
            </select>
          </div>
        {/each}
      </div>
    </div>
  </div>
{/if}

<style>
  .dme-backdrop {
    position: fixed;
    inset: 0;
    z-index: 45;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.65);
  }
  .dme-modal {
    width: min(720px, 94vw);
    max-height: 82vh;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    border: 1px solid var(--editor-border);
    border-radius: 12px;
    background: var(--editor-panel);
    color: var(--editor-text);
  }
  .dme-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dme-head h2 {
    margin: 0 auto 0 0;
    font-size: 15px;
  }
  .dme-table {
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dme-row {
    display: grid;
    grid-template-columns: 1.4fr 1fr auto auto 72px;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-radius: 6px;
    background: var(--editor-chrome);
    font-size: 12px;
  }
  .dme-note {
    color: var(--editor-secondary);
    font-variant-numeric: tabular-nums;
  }
  .listening {
    border-color: var(--editor-accent);
    color: var(--editor-accent);
  }
</style>
