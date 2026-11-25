<script>
  let {
    open = false,
    tracks = [],
    activeTrackId = null,
    chordCount = 0,
    canUndo = false,
    canRedo = false,
    dirty = false,
    onselecttrack = null,
    onimportmidi = null,
    onundo = null,
    onredo = null,
    onsave = null,
    onopenchords = null,
    onopenbounce = null,
  } = $props();
</script>

{#if open}
  <section class="ks-setup-bar" aria-label="Keysounds setup">
    <div class="ks-bar-row">
      <strong>Keysounds</strong>
      <select aria-label="Active keysound track" value={activeTrackId ?? ""}
        onchange={e => onselecttrack?.(Number(e.currentTarget.value))}>
        {#each tracks as track (track.id)}
          <option value={track.id}>{track.name} ({track.noteCount} notes)</option>
        {/each}
      </select>
      <button type="button" onclick={() => onimportmidi?.()}>Import .mid…</button>
      <button type="button" disabled={!canUndo} onclick={() => onundo?.()}>Undo</button>
      <button type="button" disabled={!canRedo} onclick={() => onredo?.()}>Redo</button>
      <button type="button" onclick={() => onsave?.()}>{dirty ? "Save .dawsuks*" : "Save .dawsuks"}</button>
      {#if chordCount > 0}
        <button type="button" class="ks-chord-badge" onclick={() => onopenchords?.()}>
          {chordCount} {chordCount === 1 ? "chord" : "chords"} to resolve
        </button>
      {/if}
      <button type="button" class="primary" onclick={() => onopenbounce?.()}>Bounce &amp; Deduplicate…</button>
    </div>
  </section>
{/if}

<style>
  .ks-setup-bar {
    padding: 8px 12px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-panel);
    color: var(--editor-text);
    font-size: 12px;
  }
  .ks-bar-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .ks-chord-badge {
    border-color: rgba(245, 158, 11, 0.5);
    color: #fbbf24;
  }
</style>
