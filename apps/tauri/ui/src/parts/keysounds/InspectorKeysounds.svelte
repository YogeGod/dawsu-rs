<script>
  import { bridge } from "../../lib/bridge.svelte.js";
  import { generateFooter, trackCaption } from "../../lib/keysounds.js";
  import { SCALE_DEFS, SCALE_ROOTS } from "../../lib/midi.js";
  import InspectorHead from "../InspectorHead.svelte";

  // The Inspector in Keysounds mode: the active track, its instrument, the note tools, the scale,
  // and Generate. It shows in the Selection section's place; KeysoundsModal.svelte owns the state
  // and hands this part a plain model (bridge.keysounds), as app.js does for the other sections.
  const on = (name, ...args) => bridge.callAction("keysoundsUi", name, ...args);
  const model = $derived(bridge.keysounds);
  const track = $derived(model?.track ?? null);
  const tracks = $derived(model?.tracks ?? []);
  const busy = $derived(!model || model.loading);
  const noNotes = $derived(busy || !model.notesCount);
  const snapOn = $derived(bridge.scene?.snap !== false);
  const chromatic = $derived((model?.scaleKind ?? "chromatic") === "chromatic");
  const footer = $derived(generateFooter(model));
  let open = $state(true);
  let generateMenu = $state(false);

  // A sticky box cannot leave its section, so while the footer is there the Selection section and
  // its mount are marked to give up their own boxes (inspector.css): the Inspector itself then lays
  // the footer out, last, at its bottom edge.
  function pinToInspector(foot) {
    const boxes = [foot.parentElement, foot.parentElement?.parentElement]
      .filter(box => box?.matches?.("[data-ui-mount], .inspector-part"));
    for (const box of boxes) box.setAttribute("data-keysounds-foot", "");
    return () => { for (const box of boxes) box.removeAttribute("data-keysounds-foot"); };
  }

  function closeGenerateMenu(event) {
    if (generateMenu && !event.target?.closest?.(".insp-ks-generate")) generateMenu = false;
  }
</script>

<svelte:window onpointerdown={closeGenerateMenu} />

<div class="insp insp-ks">
  <InspectorHead title="Keysounds" body="inspector-keysounds-body" bind:open />
  <div id="inspector-keysounds-body" hidden={!open}>
    <div class="insp-rows">
      <div class="insp-row">
        <span class="insp-label">Track</span>
        {#if tracks.length}
          <select class="insp-ks-wide" aria-label="Active keysound track" value={model.trackId ?? ""} disabled={busy}
            onchange={event => on("selectTrack", Number(event.currentTarget.value))}>
            {#each tracks as item, index (item.id)}
              <option value={item.id}>{index + 1} · {item.name}</option>
            {/each}
          </select>
        {:else}
          <span class="insp-value">No tracks
            <button type="button" class="insp-small" disabled={busy} onclick={() => on("addTrack")}>New track</button>
          </span>
        {/if}
      </div>
    </div>
    {#if track}
      <p class="insp-summary">{trackCaption(track)}</p>
    {/if}

    <p class="insp-caption insp-ks-caption">Track</p>
    <div class="insp-rows">
      <div class="insp-row">
        <span class="insp-label">Instrument</span>
        <select class="insp-ks-wide" aria-label="VST3 instrument" value={model?.pluginId ?? ""}
          disabled={busy || !model.instruments.length} onchange={event => on("setPlugin", event.currentTarget.value)}>
          <option value="">No instrument</option>
          {#each model?.instruments ?? [] as choice (choice.id)}
            <option value={choice.id}>{choice.name}{choice.vendor ? ` · ${choice.vendor}` : ""}</option>
          {/each}
        </select>
      </div>
      {#if track?.isSoundfont && track.soundfontPresets?.length > 1}
        <div class="insp-row">
          <span class="insp-label">Preset</span>
          <select class="insp-ks-wide" aria-label="SoundFont preset" value={track.soundfontPreset ?? 0} disabled={busy}
            onchange={event => on("selectPreset", Number(event.currentTarget.value))}>
            {#each track.soundfontPresets as presetName, index (index)}
              <option value={index}>{presetName}</option>
            {/each}
          </select>
        </div>
      {/if}
    </div>
    <div class="insp-actions">
      <button type="button" disabled={busy || !track || !model.pluginId || !model.sidecarAvailable}
        onclick={() => on("loadVst3")}>Load VST3</button>
      <button type="button" disabled={busy} onclick={() => on("scan")}>Scan VST3</button>
      <button type="button" disabled={busy || !track} onclick={() => on("loadSoundfont")}>Load SFZ / SF2…</button>
      <button type="button" class="insp-ks-delete" disabled={busy || !track}
        aria-label={track ? `Delete ${track.name}` : "Delete track"}
        onclick={() => on("deleteTrack", track.id)}>Delete track</button>
    </div>
    {#if model?.status?.text}
      <p class="insp-caption insp-ks-status" class:ready={model.status.tone === "ready"}
        class:warning={model.status.tone === "warning"}>{model.status.text}</p>
    {/if}

    <p class="insp-caption insp-ks-caption">Notes</p>
    <div class="insp-actions">
      <button type="button" disabled={noNotes || !snapOn}
        title={snapOn ? "Snap every note start to the editor's snap" : "Turn Snap on to snap notes to the grid"}
        onclick={() => on("snapToGrid")}>Snap to grid</button>
      <button type="button" disabled={noNotes} onclick={() => on("trim")}>Trim overlaps</button>
      <button type="button" disabled={busy || model.notesCount < 2}
        title="Strum simultaneous chord notes (+20 ms per voice)" onclick={() => on("strum")}>Strum</button>
      <button type="button" disabled={noNotes}
        title="Subtly humanise timing (±10 ms) and velocity (±12)" onclick={() => on("humanise")}>Humanise</button>
      <button type="button" disabled={noNotes} onclick={() => on("syncToObjects")}>Sync to objects</button>
    </div>
    <div class="insp-rows">
      <div class="insp-row">
        <span class="insp-label">Transpose</span>
        <span class="insp-value insp-ks-transpose">
          <button type="button" class="insp-small" aria-label="Transpose down an octave" disabled={noNotes}
            onclick={() => on("transpose", -12)}>−12</button>
          <button type="button" class="insp-small" aria-label="Transpose down a semitone" disabled={noNotes}
            onclick={() => on("transpose", -1)}>−1</button>
          <button type="button" class="insp-small" aria-label="Transpose up a semitone" disabled={noNotes}
            onclick={() => on("transpose", 1)}>+1</button>
          <button type="button" class="insp-small" aria-label="Transpose up an octave" disabled={noNotes}
            onclick={() => on("transpose", 12)}>+12</button>
        </span>
      </div>
    </div>

    <p class="insp-caption insp-ks-caption">Scale</p>
    <div class="insp-rows">
      <div class="insp-row">
        <span class="insp-label">Root</span>
        <select aria-label="Scale root" value={model?.scaleRoot ?? 0} disabled={busy}
          onchange={event => on("setScaleRoot", Number(event.currentTarget.value))}>
          {#each SCALE_ROOTS as rootName, index (index)}
            <option value={index}>{rootName}</option>
          {/each}
        </select>
      </div>
      <div class="insp-row">
        <span class="insp-label">Scale</span>
        <select class="insp-ks-wide" aria-label="Scale" value={model?.scaleKind ?? "chromatic"} disabled={busy}
          onchange={event => on("setScaleKind", event.currentTarget.value)}>
          {#each SCALE_DEFS as def (def.id)}
            <option value={def.id}>{def.label}</option>
          {/each}
        </select>
      </div>
      <label class="insp-row">Fold rows to scale
        <input type="checkbox" aria-label="Fold rows to scale" checked={!!model?.foldToScale && !chromatic}
          disabled={busy || chromatic} onchange={() => on("toggleFold")}>
      </label>
    </div>
  </div>
</div>

<!-- Pinned to the Inspector's bottom edge (inspector.css), so Generate stays in reach while the rows scroll. -->
<div class="insp-ks-foot" {@attach pinToInspector}>
  {#if footer.chords}
    <button type="button" class="insp-ks-chords" onclick={() => on("openChords")}>
      <span>⚠ {footer.chords}</span><span aria-hidden="true">›</span>
    </button>
  {/if}
  <div class="insp-ks-generate">
    <button type="button" class="insp-ks-go" disabled={footer.disabled} onclick={() => on("generate")}>Generate [Keysounds]</button>
    <button type="button" class="insp-ks-more" aria-haspopup="menu" aria-expanded={generateMenu}
      aria-label="More Generate options" disabled={footer.disabled} onclick={() => (generateMenu = !generateMenu)}>▾</button>
    {#if generateMenu}
      <div class="insp-ks-menu" role="menu" aria-label="Generate options">
        <button type="button" role="menuitem" onclick={() => { generateMenu = false; on("generate"); }}>Generate [Keysounds]</button>
        <button type="button" role="menuitem" onclick={() => { generateMenu = false; on("openBounce"); }}>Bounce &amp; Deduplicate…</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .insp-ks-caption {
    margin: 10px 0 4px;
  }
  .insp-ks :global(select.insp-ks-wide) {
    max-width: 128px;
  }
  .insp-ks-transpose {
    gap: 3px;
  }
  .insp-ks-delete:not(:disabled) {
    color: var(--editor-error);
  }
  .insp-ks-status {
    margin: 2px 0 0;
    padding: 0 6px;
  }
  .insp-ks-status.ready {
    color: var(--editor-accent);
  }
  .insp-ks-status.warning {
    color: var(--editor-warning);
  }
  .insp-ks-foot {
    position: sticky;
    bottom: 0;
    z-index: 2;
    order: 99;
    margin-top: auto;
    padding: 8px 12px 10px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-panel);
    box-shadow: 0 -8px 16px rgba(0, 0, 0, 0.5);
    font-size: 12px;
  }
  .insp-ks-chords {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    min-height: 30px;
    margin: 0 0 6px;
    padding: 3px 10px;
    border: 1px solid rgba(255, 214, 10, 0.45);
    border-radius: 4px;
    background: rgba(255, 214, 10, 0.06);
    color: var(--editor-warning);
    font: inherit;
    cursor: pointer;
  }
  .insp-ks-generate {
    position: relative;
    display: flex;
    height: 32px;
  }
  .insp-ks-go,
  .insp-ks-more {
    min-height: 0;
    border: 0;
    background: #efc16a;
    color: var(--editor-ink);
    font: 650 12px var(--font-ui);
    cursor: pointer;
  }
  .insp-ks-go {
    flex: 1;
    border-radius: 5px 0 0 5px;
  }
  .insp-ks-more {
    width: 30px;
    padding: 0;
    border-left: 1px solid rgba(0, 0, 0, 0.35);
    border-radius: 0 5px 5px 0;
    filter: brightness(0.9);
  }
  .insp-ks-go:disabled,
  .insp-ks-more:disabled {
    background: var(--editor-selected);
    color: var(--editor-disabled);
    cursor: default;
    filter: none;
  }
  .insp-ks-menu {
    position: absolute;
    right: 0;
    bottom: 38px;
    z-index: 3;
    display: flex;
    flex-direction: column;
    width: 200px;
    padding: 4px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-chrome);
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.55);
  }
  .insp-ks-menu button {
    min-height: 26px;
    padding: 0 10px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--editor-text);
    font: inherit;
    font-size: 12.5px;
    text-align: left;
    cursor: pointer;
  }
  .insp-ks-menu button:hover,
  .insp-ks-menu button:focus-visible {
    background: var(--editor-hover);
  }
</style>
