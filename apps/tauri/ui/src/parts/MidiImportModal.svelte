<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";

  let open = $state(false);
  let loading = $state(false);
  let error = $state("");
  let info = $state(null);
  let selectedTracks = $state([]);
  let useBeatmapTempo = $state(true);
  let offsetMs = $state(0);
  let snap = $state(true);
  let followVolume = $state(false);

  const hasDoc = $derived(Boolean(bridge.scene));

  async function openMidiImport() {
    if (!hasDoc) return;
    error = "";
    try {
      const path = await invoke("pick_midi_file");
      if (!path) return;
      loading = true;
      open = true;
      info = null;
      const result = await invoke("inspect_midi_file", { path });
      info = result;
      selectedTracks = (result?.tracks ?? []).map(t => t.index);
      useBeatmapTempo = true;
      offsetMs = 0;
      snap = true;
      followVolume = false;
    } catch (err) {
      error = String(err);
      open = true;
    } finally {
      loading = false;
    }
  }

  function toggleTrack(index) {
    if (selectedTracks.includes(index)) {
      selectedTracks = selectedTracks.filter(i => i !== index);
    } else {
      selectedTracks = [...selectedTracks, index];
    }
  }

  async function runImport() {
    if (!info || !selectedTracks.length || loading) return;
    loading = true;
    error = "";
    try {
      const outcome = await invoke("apply_midi_import", {
        path: info.path,
        tracks: selectedTracks,
        useBeatmapTempo,
        offsetMs: Number(offsetMs) || 0,
        snap,
        followVolume,
      });
      open = false;
      globalThis.showToast?.({
        tone: "success",
        title: "MIDI imported",
        body: `Added ${outcome?.added ?? 0} hits (${outcome?.skipped ?? 0} skipped). Press Ctrl+Z to undo.`,
      });
      await invoke("editor_view", { scene: true, action: null }).catch(() => {});
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  function closeModal() {
    if (loading) return;
    open = false;
  }

  onMount(() => {
    globalThis.openMidiImport = openMidiImport;
  });
</script>

{#if open}
  <div class="midi-backdrop" role="presentation" onclick={(e) => { if (e.target === e.currentTarget) closeModal(); }}>
    <div class="midi-modal" role="dialog" aria-modal="true" aria-label="Import MIDI">
      <header class="midi-head">
        <span class="midi-title">Import MIDI</span>
        {#if info?.name}
          <span class="midi-file">{info.name}</span>
        {/if}
        <button type="button" class="midi-close" aria-label="Close" onclick={closeModal}>×</button>
      </header>

      <div class="midi-body">
        {#if loading && !info}
          <p class="midi-caption">Reading MIDI tracks…</p>
        {:else if error}
          <p class="midi-error">{error}</p>
        {:else if info}
          <div class="midi-summary">
            <span>{info.noteCount ?? 0} notes</span>
            <span>{Math.round(info.bpm || 120)} BPM in file</span>
            <span>{info.mappedCount ?? 0} GM/drum-map matched</span>
          </div>

          <div class="midi-section">
            <span class="midi-label">Tracks</span>
            <div class="midi-tracks">
              {#each info.tracks ?? [] as tr (`${tr.index}-${tr.channel}`)}
                <label class="midi-track-row">
                  <input type="checkbox" checked={selectedTracks.includes(tr.index)} onchange={() => toggleTrack(tr.index)} />
                  <span class="midi-track-name">{tr.name || `Track ${tr.index + 1}`}</span>
                  {#if tr.isDrum}
                    <span class="midi-tag">Drums (Ch 10)</span>
                  {:else}
                    <span class="midi-meta">Ch {tr.channel + 1}</span>
                  {/if}
                  <span class="midi-count">{tr.noteCount} notes</span>
                </label>
              {/each}
            </div>
          </div>

          <div class="midi-section midi-options">
            <label class="midi-check">
              <input type="checkbox" bind:checked={useBeatmapTempo} />
              <span>Follow beatmap red lines (otherwise use MIDI file tempo)</span>
            </label>
            <label class="midi-check">
              <input type="checkbox" bind:checked={snap} />
              <span>Snap imported hits to the active grid and nearby objects</span>
            </label>
            <label class="midi-check">
              <input type="checkbox" bind:checked={followVolume} />
              <span>Use map volume instead of MIDI note velocity</span>
            </label>
            <label class="midi-offset">
              <span>Start offset (ms)</span>
              <input type="number" step="1" bind:value={offsetMs} />
            </label>
          </div>
        {/if}
      </div>

      <footer class="midi-foot">
        <button type="button" disabled={loading} onclick={closeModal}>Cancel</button>
        <button type="button" class="midi-primary" disabled={loading || !info || !selectedTracks.length} onclick={runImport}>
          {loading ? "Importing…" : "Import"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .midi-backdrop {
    position: fixed;
    inset: 0;
    z-index: 62;
    display: grid;
    place-items: center;
    background: rgba(6, 8, 13, 0.68);
    backdrop-filter: blur(4px);
  }
  .midi-modal {
    display: flex;
    flex-direction: column;
    width: min(560px, calc(100vw - 40px));
    max-height: min(580px, calc(100vh - 48px));
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    color: var(--editor-text);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.62);
    overflow: hidden;
  }
  .midi-head {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 50px;
    padding: 0 18px;
    border-bottom: 1px solid var(--editor-border);
    background: linear-gradient(180deg, #252c3b 0%, #1d2230 100%);
  }
  .midi-title {
    font-size: 14px;
    font-weight: 600;
  }
  .midi-file {
    font-size: 12px;
    color: var(--editor-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .midi-close {
    margin-left: auto;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--editor-muted);
    font-size: 18px;
  }
  .midi-body {
    flex: 1;
    padding: 18px 20px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .midi-summary {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    padding: 10px 12px;
    border-radius: 7px;
    background: var(--editor-canvas);
    border: 1px solid var(--editor-border);
    font-size: 12px;
    color: var(--editor-secondary);
    font-variant-numeric: tabular-nums;
  }
  .midi-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .midi-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--editor-text);
  }
  .midi-tracks {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 200px;
    overflow-y: auto;
  }
  .midi-track-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.025);
    font-size: 12px;
  }
  .midi-track-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .midi-tag {
    padding: 2px 6px;
    border-radius: 4px;
    background: rgba(79, 168, 255, 0.16);
    color: var(--editor-accent);
    font-size: 11px;
    font-weight: 600;
  }
  .midi-meta,
  .midi-count {
    font-size: 11px;
    color: var(--editor-muted);
    font-variant-numeric: tabular-nums;
  }
  .midi-options {
    padding-top: 8px;
    border-top: 1px solid var(--editor-border);
  }
  .midi-check {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
  }
  .midi-offset {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    font-size: 12px;
    color: var(--editor-secondary);
  }
  .midi-offset input {
    width: 110px;
  }
  .midi-caption {
    margin: 0;
    font-size: 12px;
    color: var(--editor-muted);
  }
  .midi-error {
    margin: 0;
    font-size: 12px;
    color: var(--editor-error);
  }
  .midi-foot {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    height: 52px;
    padding: 0 18px;
    align-items: center;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .midi-primary {
    background: var(--editor-accent);
    color: var(--editor-accentink);
    font-weight: 600;
  }
</style>
