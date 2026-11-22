<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    NO_PLUGINS, instrumentChoices, keysoundsAdd, keysoundsDelete, keysoundsScan, keysoundsView,
  } from "../lib/midi.js";

  // Map › Keysounds & VST3…: the keysound tracks Rust keeps for this map, each optionally played
  // by a scanned VST3 instrument. menus.js calls globalThis.openKeysounds.
  let open = $state(false);
  let loading = $state(false);
  let error = $state("");
  let summary = $state(null);
  let name = $state("");
  let plugin = $state("");

  const tracks = $derived(summary?.tracks ?? []);
  const instruments = $derived(instrumentChoices(summary?.vst3Plugins));

  async function send(payload) {
    loading = true;
    error = "";
    try {
      summary = await invoke("keysounds_action", { payload });
      if (plugin && !instrumentChoices(summary?.vst3Plugins).some(choice => choice.name === plugin)) plugin = "";
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  async function openKeysounds() {
    if (!bridge.scene) return;
    open = true;
    name = "";
    await send(keysoundsView());
  }

  async function addTrack() {
    if (loading) return;
    await send(keysoundsAdd(name, plugin || null));
    if (!error) name = "";
  }

  function closeModal() {
    if (loading) return;
    open = false;
  }

  function onKeydown(event) {
    if (open && event.key === "Escape") {
      event.stopPropagation();
      closeModal();
    }
  }

  // The editor's timeline shortcuts stay off while this is open (boot.js reads the flag).
  $effect(() => {
    globalThis.dawsuModalOpen = open;
    return () => { globalThis.dawsuModalOpen = false; };
  });

  onMount(() => {
    globalThis.openKeysounds = openKeysounds;
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <div class="ks-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal(); }}>
    <div class="ks-modal" role="dialog" aria-modal="true" aria-label="Keysounds and VST3">
      <header class="ks-head">
        <span class="ks-title">Keysounds & VST3</span>
        {#if summary?.targetDifficulty}
          <span class="ks-sub">{summary.targetDifficulty}</span>
        {/if}
        <button type="button" class="ks-close" aria-label="Close" onclick={closeModal}>×</button>
      </header>

      <div class="ks-body">
        {#if error}
          <p class="ks-error" role="alert">{error}</p>
        {/if}

        <section class="ks-section">
          <span class="ks-label">Keysound tracks</span>
          {#if !summary && loading}
            <p class="ks-caption">Loading…</p>
          {:else if tracks.length}
            <ul class="ks-list">
              {#each tracks as track (track.id)}
                <li class="ks-row">
                  <span class="ks-name">{track.name}</span>
                  <span class="ks-meta">{track.instrument || "No instrument"}</span>
                  <span class="ks-meta">{track.noteCount} notes</span>
                  <button type="button" aria-label={`Delete ${track.name}`} disabled={loading}
                    onclick={() => send(keysoundsDelete(track.id))}>Delete</button>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="ks-caption">No keysound tracks yet. Add one below; MIDI Record fills the first track.</p>
          {/if}
        </section>

        <section class="ks-section ks-add">
          <span class="ks-label">Add a track</span>
          <div class="ks-add-row">
            <input type="text" placeholder="Keysound Track" aria-label="Track name" bind:value={name}
              onkeydown={(e) => { if (e.key === "Enter") addTrack(); }} />
            <select aria-label="VST3 instrument" bind:value={plugin} disabled={!instruments.length}>
              <option value="">No instrument</option>
              {#each instruments as choice (choice.id)}
                <option value={choice.name}>{choice.name}{choice.vendor ? ` · ${choice.vendor}` : ""}</option>
              {/each}
            </select>
            <button type="button" class="ks-primary" disabled={loading} onclick={addTrack}>Add track</button>
          </div>
        </section>

        <section class="ks-section">
          <div class="ks-row-head">
            <span class="ks-label">VST3 instruments</span>
            <button type="button" disabled={loading} onclick={() => send(keysoundsScan())}>{loading ? "Working…" : "Scan VST3 plug-ins"}</button>
          </div>
          {#if summary && !instruments.length}
            <p class="ks-caption ks-empty">{NO_PLUGINS}</p>
          {:else if instruments.length}
            <p class="ks-caption">{instruments.length} instrument{instruments.length === 1 ? "" : "s"} ready to assign.</p>
          {/if}
        </section>
      </div>

      <footer class="ks-foot">
        <button type="button" disabled={loading} onclick={closeModal}>Close</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .ks-backdrop {
    position: fixed;
    inset: 0;
    z-index: 62;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.68);
  }
  .ks-modal {
    display: flex;
    flex-direction: column;
    width: min(600px, calc(100vw - 40px));
    max-height: min(620px, calc(100vh - 48px));
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    color: var(--editor-text);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.62);
    overflow: hidden;
  }
  .ks-head {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 50px;
    padding: 0 18px;
    border-bottom: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .ks-title {
    font-size: 14px;
    font-weight: 600;
  }
  .ks-sub {
    font-size: 12px;
    color: var(--editor-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ks-close {
    margin-left: auto;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--editor-muted);
    font-size: 18px;
  }
  .ks-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 18px 20px;
    overflow-y: auto;
  }
  .ks-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .ks-add {
    padding-top: 12px;
    border-top: 1px solid var(--editor-border);
  }
  .ks-label {
    font-size: 12px;
    font-weight: 600;
  }
  .ks-row-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  .ks-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 220px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }
  .ks-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 10px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.025);
    font-size: 12px;
  }
  .ks-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ks-meta {
    font-size: 11px;
    color: var(--editor-muted);
    font-variant-numeric: tabular-nums;
  }
  .ks-add-row {
    display: flex;
    gap: 8px;
  }
  .ks-add-row input {
    flex: 1;
    min-width: 0;
  }
  .ks-caption {
    margin: 0;
    font-size: 12px;
    color: var(--editor-muted);
  }
  .ks-empty {
    color: var(--editor-warning);
  }
  .ks-error {
    margin: 0;
    font-size: 12px;
    color: var(--editor-error);
  }
  .ks-foot {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    height: 52px;
    padding: 0 18px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .ks-primary {
    background: var(--editor-accent);
    color: var(--editor-accentink);
    font-weight: 600;
  }
</style>
