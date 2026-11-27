<script>
  import { trackCaption } from "../../lib/keysounds.js";
  import Plus from "../../icons/Plus.svg?raw";

  // The keysound tracks, in the lane column of Keysounds mode: one row per track, then the two rows
  // that add one. A click makes a track the active one, the track the roll shows and edits.
  let {
    tracks = [],
    activeTrackId = null,
    loading = false,
    onselect = null,
    onmute = null,
    onsolo = null,
    onrename = null,
    ondelete = null,
    onadd = null,
    onimport = null,
  } = $props();

  // The track whose name is being typed, and the text so far.
  let renaming = $state(null);
  let draft = $state("");
  // The right-click menu: { track, x, y }.
  let menu = $state(null);

  function startRename(track) {
    if (loading) return;
    menu = null;
    renaming = track.id;
    draft = track.name;
  }

  function commitRename() {
    if (renaming == null) return;
    const id = renaming;
    const name = draft.trim();
    renaming = null;
    if (name && name !== tracks.find(track => track.id === id)?.name) onrename?.(id, name);
  }

  function onNameKey(event) {
    event.stopPropagation();
    if (event.key === "Enter") commitRename();
    else if (event.key === "Escape") renaming = null;
  }

  function onRowKey(event, track) {
    if (event.key === "F2") {
      event.preventDefault();
      startRename(track);
    }
  }

  function openMenu(event, track) {
    event.preventDefault();
    if (loading) return;
    menu = { track, x: event.clientX, y: event.clientY };
  }

  function closeMenuOutside(event) {
    if (menu && !event.target?.closest?.(".ks-track-menu")) menu = null;
  }

  function onWindowKey(event) {
    if (menu && event.key === "Escape") {
      event.stopPropagation();
      menu = null;
    }
  }

  const focusAll = node => {
    node.focus();
    node.select();
  };
  const focusFirst = node => {
    node.querySelector("button")?.focus();
  };
</script>

<svelte:window onpointerdown={closeMenuOutside} onkeydown={onWindowKey} />

<div class="ks-tracks" role="group" aria-label="Keysound tracks">
  {#if !tracks.length}
    <div class="ks-tracks-empty" role="status">
      <p>No keysound tracks yet. Add a track or import a .mid below.</p>
    </div>
  {/if}
  {#each tracks as track, index (track.id)}
    {@const active = track.id === activeTrackId}
    <div class="ks-track" class:active oncontextmenu={event => openMenu(event, track)} role="presentation">
      {#if renaming === track.id}
        <span class="ks-track-num" aria-hidden="true">{index + 1}</span>
        <input class="ks-track-rename" type="text" aria-label={`Rename ${track.name}`} bind:value={draft}
          {@attach focusAll} onkeydown={onNameKey} onblur={commitRename} />
      {:else}
        <button type="button" class="ks-track-main" aria-current={active ? "true" : undefined}
          aria-label={`Track ${index + 1}: ${track.name}, ${trackCaption(track)}`} title={track.name}
          onclick={() => onselect?.(track.id)} ondblclick={() => startRename(track)}
          onkeydown={event => onRowKey(event, track)}>
          <span class="ks-track-num" aria-hidden="true">{index + 1}</span>
          <span class="ks-track-text">
            <span class="ks-track-name">{track.name}</span>
            <span class="ks-track-caption">{trackCaption(track)}</span>
          </span>
        </button>
      {/if}
      <span class="ks-track-ms">
        <button type="button" class="ks-ms ks-mute" aria-label={`Mute ${track.name}`} aria-pressed={Boolean(track.muted)}
          disabled={loading} onclick={() => onmute?.(track.id)}>M</button>
        <button type="button" class="ks-ms ks-solo" aria-label={`Solo ${track.name}`} aria-pressed={Boolean(track.solo)}
          disabled={loading} onclick={() => onsolo?.(track.id)}>S</button>
      </span>
    </div>
  {/each}
  <button type="button" class="ks-track-add" aria-label="Add track" title="Add track" disabled={loading}
    onclick={() => onadd?.()}>{@html Plus}<span>Add track</span></button>
  <button type="button" class="ks-track-add" aria-label="Import .mid…" title="Import .mid…" disabled={loading}
    onclick={() => onimport?.()}>{@html Plus}<span>Import .mid…</span></button>
</div>

{#if menu}
  <div class="ks-track-menu" role="menu" aria-label={`Track ${menu.track.name}`}
    style:left={menu.x + "px"} style:top={menu.y + "px"} {@attach focusFirst}>
    <button type="button" role="menuitem" onclick={() => startRename(menu.track)}>Rename<kbd>F2</kbd></button>
    <button type="button" role="menuitem" class="ks-track-menu-delete"
      onclick={() => { ondelete?.(menu.track.id); menu = null; }}>Delete track</button>
  </div>
{/if}

<style>
  /* Fills the lane column left of the 44 px key column; its rows read their width from here. */
  .ks-tracks {
    container-type: inline-size;
    flex: 1 1 0;
    min-width: 0;
    overflow-x: hidden;
    overflow-y: auto;
    background: var(--editor-panel);
  }
  .ks-tracks-empty {
    margin: 10px 8px;
    padding: 12px;
    border: 1px dashed var(--editor-border);
    border-radius: 8px;
    background: rgba(56, 189, 248, 0.05);
    color: var(--editor-secondary);
    font-size: 12px;
    line-height: 1.45;
  }
  .ks-tracks-empty p {
    margin: 0;
  }
  .ks-track {
    display: flex;
    align-items: stretch;
    height: 46px;
    box-sizing: border-box;
    border-bottom: 1px solid var(--editor-canvas);
    background: var(--editor-panel);
  }
  .ks-track.active {
    background: rgba(90, 200, 250, 0.14);
    box-shadow: inset 0 0 0 1px var(--editor-accent);
  }
  .ks-track-main {
    display: flex;
    align-items: stretch;
    gap: 8px;
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: var(--editor-text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .ks-track-num {
    display: grid;
    place-items: center;
    flex: none;
    width: 24px;
    background: var(--editor-selected);
    color: var(--editor-secondary);
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .ks-track.active .ks-track-num {
    background: var(--editor-accent);
    color: var(--editor-ink);
  }
  .ks-track-text {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    min-width: 0;
  }
  .ks-track-name,
  .ks-track-caption {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ks-track-name {
    font-size: 13px;
    font-weight: 500;
  }
  .ks-track-caption {
    color: var(--editor-muted);
    font-size: 11px;
  }
  .ks-track-rename {
    flex: 1 1 auto;
    align-self: center;
    min-width: 0;
    height: 26px;
    margin: 0 6px 0 8px;
    padding: 0 6px;
  }
  .ks-track-ms {
    display: flex;
    align-items: center;
    gap: 3px;
    flex: none;
    padding: 0 6px 0 4px;
  }
  .ks-ms {
    width: 24px;
    height: 25px;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: var(--editor-control);
    box-shadow: inset 0 0 0 1px var(--editor-border), var(--shadow-raised);
    color: var(--editor-secondary);
    font: 600 12px var(--font-ui);
    cursor: pointer;
  }
  .ks-ms:hover:not(:disabled) {
    color: var(--editor-text);
  }
  .ks-mute[aria-pressed="true"] {
    background: var(--mute-on);
    color: var(--editor-ink);
  }
  .ks-solo[aria-pressed="true"] {
    background: var(--solo-on);
    color: var(--editor-ink);
  }
  .ks-track-add {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 34px;
    min-height: 0;
    padding: 0 16px;
    border: 0;
    border-bottom: 1px solid var(--editor-canvas);
    border-radius: 0;
    background: var(--editor-panel);
    color: var(--editor-secondary);
    font: 500 13px var(--font-ui);
    cursor: pointer;
  }
  .ks-track-add :global(svg) {
    flex: none;
    width: 14px;
    height: 14px;
  }
  .ks-track-add span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ks-track-add:hover:not(:disabled) {
    color: var(--editor-text);
  }
  .ks-track-add:disabled {
    color: var(--editor-disabled);
    cursor: default;
  }
  /* A narrow lane column (99 px leaves 55 px here): the number, with M over S; the name is in the
     row's tooltip, and the Inspector's Track picker names every track. */
  @container (max-width: 130px) {
    .ks-track-text,
    .ks-track-add span,
    .ks-tracks-empty {
      display: none;
    }
    .ks-track-main {
      flex: none;
    }
    .ks-track-ms {
      flex: 1 1 auto;
      flex-direction: column;
      justify-content: center;
      gap: 2px;
      padding: 0;
    }
    .ks-ms {
      width: 22px;
      height: 19px;
      font-size: 11px;
    }
    .ks-track-add {
      justify-content: center;
      padding: 0;
    }
  }
  .ks-track-menu {
    position: fixed;
    z-index: 70;
    display: flex;
    flex-direction: column;
    min-width: 168px;
    padding: 4px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-chrome);
    box-shadow: 0 14px 36px rgba(0, 0, 0, 0.55);
  }
  .ks-track-menu button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
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
  .ks-track-menu button:hover,
  .ks-track-menu button:focus-visible {
    background: var(--editor-hover);
  }
  .ks-track-menu kbd {
    color: var(--editor-muted);
    font: inherit;
    font-size: 11px;
  }
  .ks-track-menu-delete {
    color: var(--editor-error);
  }
</style>
