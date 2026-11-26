<script>
  import {
    QUANTIZE_MS,
    SCALE_DEFS,
    SCALE_ROOTS,
    keysoundsQuantize,
    keysoundsScan,
    keysoundsToggleMute,
    keysoundsToggleSolo,
    keysoundsTranspose,
    keysoundsTrimOverlaps,
  } from "../../lib/midi.js";
  import { octaveLabel } from "../../lib/musicalTyping.js";

  let {
    track = null,
    loading = false,
    name = $bindable(""),
    pluginId = $bindable(""),
    scaleRoot = $bindable(0),
    scaleKind = $bindable("chromatic"),
    foldToScale = false,
    instruments = [],
    sidecarAvailable = false,
    drumVst3,
    notesCount = 0,
    showGhosts = true,
    showObjectLines = true,
    typingState = { enabled: false, basePitch: 60, velocity: 100 },
    ondeletetrack,
    onaddtrack,
    onpluginchange,
    onloadvst3,
    onloadsoundfont = null,
    onselectpreset = null,
    onsend,
    ontoggledrumvst3,
    ontrackaction,
    onsynctoobjects = null,
    ontoggleghosts = null,
    ontoggleobjectlines = null,
    ontoggletyping = null,
    ontogglefold = null,
    onscalechange = null,
    onstrum = null,
    onhumanise = null,
  } = $props();
</script>

<div class="ks-bar">
  <div class="ks-group">
    <button type="button" class="ks-ms" class:active={Boolean(track?.muted)} aria-label="Mute track"
      aria-pressed={Boolean(track?.muted)} disabled={loading || !track}
      onclick={() => ontrackaction(id => keysoundsToggleMute(id))}>M</button>
    <button type="button" class="ks-ms ks-solo" class:active={Boolean(track?.solo)} aria-label="Solo track"
      aria-pressed={Boolean(track?.solo)} disabled={loading || !track}
      onclick={() => ontrackaction(id => keysoundsToggleSolo(id))}>S</button>
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
    {#if onloadsoundfont}
      <button type="button" disabled={loading || !track} onclick={onloadsoundfont}>Load SFZ / SF2…</button>
    {/if}
    {#if track?.isSoundfont && track.soundfontPresets?.length > 1 && onselectpreset}
      <select aria-label="SoundFont preset" value={track.soundfontPreset ?? 0} disabled={loading}
        onchange={(e) => onselectpreset(Number(e.currentTarget.value))}>
        {#each track.soundfontPresets as presetName, idx (idx)}
          <option value={idx}>{presetName}</option>
        {/each}
      </select>
    {/if}
    <label class="ks-drum-vst3">
      <input type="checkbox" checked={drumVst3.enabled} disabled={loading}
        onchange={(e) => ontoggledrumvst3(e.currentTarget.checked)} />
      <span>Use VST3 sounds for drum map</span>
    </label>
  </div>
  <div class="ks-group">
    <button type="button" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsQuantize(id, QUANTIZE_MS))}>Quantize 125 ms</button>
    <button type="button" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTrimOverlaps(id))}>Trim overlaps</button>
    {#if onstrum}
      <button type="button" disabled={loading || notesCount < 2}
        title="Strum simultaneous chord notes (+20 ms per voice)" onclick={onstrum}>Strum</button>
    {/if}
    {#if onhumanise}
      <button type="button" disabled={loading || !notesCount}
        title="Subtly humanise timing (±10 ms) and velocity (±12)" onclick={onhumanise}>Humanise</button>
    {/if}
    {#if onsynctoobjects}
      <button type="button" disabled={loading || !notesCount} onclick={onsynctoobjects}>Sync to objects</button>
    {/if}
    <button type="button" aria-label="Transpose down an octave" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTranspose(id, -12))}>−12</button>
    <button type="button" aria-label="Transpose down a semitone" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTranspose(id, -1))}>−1</button>
    <button type="button" aria-label="Transpose up a semitone" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTranspose(id, 1))}>+1</button>
    <button type="button" aria-label="Transpose up an octave" disabled={loading || !notesCount} onclick={() => ontrackaction(id => keysoundsTranspose(id, 12))}>+12</button>
  </div>
  <div class="ks-group">
    <select aria-label="Scale root" bind:value={scaleRoot} disabled={loading}>
      {#each SCALE_ROOTS as rootName, idx (idx)}
        <option value={idx}>{rootName}</option>
      {/each}
    </select>
    <select aria-label="Scale" bind:value={scaleKind} disabled={loading}
      onchange={() => onscalechange?.(scaleKind)}>
      {#each SCALE_DEFS as def (def.id)}
        <option value={def.id}>{def.label}</option>
      {/each}
    </select>
    {#if ontogglefold}
      <button type="button" class="ks-toggle" class:active={foldToScale && scaleKind !== "chromatic"}
        aria-pressed={foldToScale && scaleKind !== "chromatic"}
        disabled={loading || scaleKind === "chromatic"}
        title="Fold rows to the selected scale"
        onclick={ontogglefold}>Fold</button>
    {/if}
    {#if ontoggleghosts}
      <button type="button" class="ks-toggle" class:active={showGhosts} aria-pressed={showGhosts}
        onclick={ontoggleghosts}>Ghosts</button>
    {/if}
    {#if ontoggleobjectlines}
      <button type="button" class="ks-toggle" class:active={showObjectLines} aria-pressed={showObjectLines}
        onclick={ontoggleobjectlines}>Object lines</button>
    {/if}
    {#if ontoggletyping}
      <button type="button" class="ks-toggle" class:active={typingState.enabled} aria-pressed={typingState.enabled}
        title="Play notes with your computer keyboard (Alt+K). Z/X octave, C/V velocity."
        onclick={ontoggletyping}>
        {typingState.enabled ? `Typing · ${octaveLabel(typingState.basePitch)} · v${typingState.velocity}` : "Typing (Alt+K)"}
      </button>
    {/if}
  </div>
</div>

<style>
  .ks-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 8px 16px;
    padding: 8px 14px;
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
  .ks-ms {
    min-width: 24px;
    padding: 2px 6px;
    font-weight: 600;
  }
  .ks-ms.active {
    background: rgba(239, 68, 68, 0.24);
    border-color: #ef4444;
    color: #fecaca;
  }
  .ks-ms.ks-solo.active {
    background: rgba(239, 193, 106, 0.24);
    border-color: #efc16a;
    color: #fde68a;
  }
  .ks-toggle.active {
    background: rgba(56, 189, 248, 0.22);
    border-color: var(--editor-accent);
    color: var(--editor-text);
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
