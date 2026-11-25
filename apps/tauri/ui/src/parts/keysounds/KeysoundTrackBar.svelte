<script>
  import {
    QUANTIZE_MS,
    keysoundsQuantize,
    keysoundsScan,
    keysoundsTranspose,
  } from "../../lib/midi.js";

  let {
    track = null,
    loading = false,
    name = $bindable(""),
    pluginId = $bindable(""),
    instruments = [],
    sidecarAvailable = false,
    drumVst3,
    notesCount = 0,
    ondeletetrack,
    onaddtrack,
    onpluginchange,
    onloadvst3,
    onsend,
    ontoggledrumvst3,
    ontrackaction,
  } = $props();
</script>

<div class="ks-bar">
  <div class="ks-group">
    <button type="button" aria-label={track ? `Delete ${track.name}` : "Delete track"} disabled={loading || !track}
      onclick={ondeletetrack}>Delete track</button>
    <input type="text" placeholder="Keysound Track" aria-label="Track name" bind:value={name}
      onkeydown={(e) => { if (e.key === "Enter") onaddtrack(); }} />
    <button type="button" disabled={loading} onclick={onaddtrack}>Add track</button>
  </div>
  <div class="ks-group">
    <select aria-label="VST3 instrument" bind:value={pluginId} disabled={!instruments.length}
      onchange={onpluginchange}>
      <option value="">No instrument</option>
      {#each instruments as choice (choice.id)}
        <option value={choice.id}>{choice.name}{choice.vendor ? ` · ${choice.vendor}` : ""}</option>
      {/each}
    </select>
    <button type="button" disabled={loading || !track || !pluginId || !sidecarAvailable}
      onclick={onloadvst3}>Load VST3</button>
    <button type="button" disabled={loading} onclick={() => onsend(keysoundsScan())}>Scan VST3</button>
    <label class="ks-drum-vst3">
      <input type="checkbox" checked={drumVst3.enabled} disabled={loading}
        onchange={(e) => ontoggledrumvst3(e.currentTarget.checked)} />
      <span>Use VST3 sounds for drum map</span>
    </label>
  </div>
  <div class="ks-group">
    <button type="button" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsQuantize(id, QUANTIZE_MS))}>Quantize 125 ms</button>
    <button type="button" aria-label="Transpose down an octave" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTranspose(id, -12))}>−12</button>
    <button type="button" aria-label="Transpose down a semitone" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTranspose(id, -1))}>−1</button>
    <button type="button" aria-label="Transpose up a semitone" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTranspose(id, 1))}>+1</button>
    <button type="button" aria-label="Transpose up an octave" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTranspose(id, 12))}>+12</button>
  </div>
</div>

<style>
  .ks-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 16px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .ks-group {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ks-group input {
    width: 130px;
  }
  .ks-drum-vst3 {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--editor-text);
    cursor: pointer;
    white-space: nowrap;
  }
  .ks-drum-vst3 input {
    width: auto;
    margin: 0;
    accent-color: var(--editor-accent);
  }
</style>
