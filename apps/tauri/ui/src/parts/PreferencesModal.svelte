<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import { playbackStartChoices, shortcutName } from "../editor/helpers.js";

  const SCALES = [90, 100, 110, 125, 150, 175, 200];
  const TABS = [
    { id: "general", label: "General" },
    { id: "folders", label: "Folders" },
    { id: "audio", label: "Audio output" },
    { id: "view", label: "Editor view" },
    { id: "shortcuts", label: "Shortcuts" },
    { id: "updates", label: "Updates" },
  ];

  let open = $state(false);
  let scope = $state("app");
  let tab = $state("general");
  let scale = $state(readSavedScale());
  let appliedScale = $state(100);

  // Shortcuts state
  let shortcuts = $state([]);
  let preset = $state("DAW");
  let shortcutQuery = $state("");
  let capturingId = $state(null);

  // Audio & MIDI state
  let audioConfig = $state(null);
  let selectedBackend = $state("");
  let selectedDevice = $state("");
  let bufferMs = $state("");
  let audioError = $state("");
  let midiPorts = $state([]);
  let selectedMidiPort = $state(readSavedMidiPort());

  const view = $derived(bridge.view ?? {});
  const scene = $derived(bridge.scene);
  const busy = $derived(Boolean(view.busy));
  const audioLocked = $derived(Boolean(view.playing || view.busy));

  function readSavedScale() {
    try {
      const raw = Number(globalThis.localStorage?.getItem("dawsu.uiScale") ?? 100);
      return SCALES.includes(raw) ? raw : 100;
    } catch {
      return 100;
    }
  }

  function applyScale(next) {
    scale = next;
    try {
      globalThis.localStorage?.setItem("dawsu.uiScale", String(next));
      if (globalThis.document?.documentElement) {
        globalThis.document.documentElement.style.zoom = next === 100 ? "" : `${next}%`;
      }
    } catch {}
  }

  function stepScale(delta) {
    const idx = SCALES.indexOf(scale);
    const next = SCALES[Math.max(0, Math.min(SCALES.length - 1, (idx < 0 ? 1 : idx) + delta))];
    applyScale(next);
    globalThis.showToast?.({
      tone: "info",
      title: `Interface scale ${next}%`,
      body: "Press Ctrl + 0 to reset to 100%.",
    });
  }

  async function loadShortcuts() {
    try {
      const res = await invoke("editor_shortcuts");
      shortcuts = (res?.entries ?? []).map(([id, label, keys]) => ({ id, label, keys: keys ?? [] }));
      preset = res?.preset ?? "DAW";
    } catch {}
  }

  async function loadAudio(backend = null) {
    try {
      const cfg = await invoke("audio_outputs", { backend });
      audioConfig = cfg;
      selectedBackend = cfg?.backend ?? "";
      selectedDevice = cfg?.device ?? "";
      bufferMs = cfg?.bufferMs == null ? "" : String(cfg.bufferMs);
      audioError = cfg?.error ?? "";
    } catch (err) {
      audioError = String(err);
    }
  }

  function readSavedMidiPort() {
    try {
      return globalThis.localStorage?.getItem("dawsu.midiPort") ?? "__all__";
    } catch {
      return "__all__";
    }
  }

  function saveMidiPort(next) {
    selectedMidiPort = next;
    try {
      globalThis.localStorage?.setItem("dawsu.midiPort", next);
    } catch {}
  }

  async function loadMidiPorts() {
    try {
      const ports = await invoke("midi_input_ports");
      midiPorts = Array.isArray(ports) ? ports : [];
    } catch {
      midiPorts = [];
    }
  }

  async function openModal(initialTab = "general") {
    tab = TABS.some(t => t.id === initialTab) ? initialTab : "general";
    scope = "app";
    appliedScale = scale;
    capturingId = null;
    open = true;
    await Promise.all([loadShortcuts(), loadAudio(null), loadMidiPorts()]);
  }

  function cancelModal() {
    applyScale(appliedScale);
    capturingId = null;
    open = false;
  }

  function commitModal() {
    appliedScale = scale;
    capturingId = null;
    open = false;
  }

  async function pickPreset(nextPreset) {
    try {
      await invoke("editor_view", { scene: false, action: { kind: "key_preset", name: nextPreset } });
      await loadShortcuts();
      globalThis.showToast?.({
        tone: "success",
        title: "Controls updated",
        body: nextPreset === "osu!" ? "Switched to osu! editor controls." : "Switched to DAW-like keys.",
      });
    } catch {}
  }

  async function applyAudioOutput() {
    if (audioLocked || !audioConfig || audioError) return;
    const ms = bufferMs === "" ? null : Number(bufferMs);
    if (ms !== null && (!Number.isInteger(ms) || ms < 10 || ms > 500)) {
      audioError = "Buffer must be between 10 and 500 ms.";
      return;
    }
    try {
      await invoke("editor_view", {
        scene: false,
        action: { kind: "output_settings", backend: selectedBackend, device_id: selectedDevice || null, ms },
      });
      await loadAudio(selectedBackend);
      globalThis.showToast?.({
        tone: "success",
        title: "Audio output updated",
        body: audioConfig?.current || "Active device applied.",
      });
    } catch (err) {
      audioError = String(err);
    }
  }

  function onShortcutKeydown(event, entry) {
    if (capturingId !== entry.id) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      capturingId = null;
      return;
    }
    if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return;
    const parts = [];
    if (event.ctrlKey || event.metaKey) parts.push("Ctrl");
    if (event.altKey) parts.push("Alt");
    if (event.shiftKey) parts.push("Shift");
    parts.push(event.key.length === 1 ? event.key.toUpperCase() : event.key);
    const nextKey = parts.join("+");
    capturingId = null;
    invoke("editor_view", { scene: false, action: { kind: "change_shortcut", id: entry.id, chord: nextKey } })
      .then(loadShortcuts)
      .catch(() => {});
  }

  async function resetShortcut(entry) {
    try {
      await invoke("editor_view", { scene: false, action: { kind: "change_shortcut", id: entry.id, chord: null } });
      await loadShortcuts();
    } catch {}
  }

  async function resetAllShortcuts() {
    try {
      await invoke("editor_view", { scene: false, action: { kind: "reset_shortcuts" } });
      await loadShortcuts();
    } catch {}
  }

  const filteredShortcuts = $derived.by(() => {
    const q = shortcutQuery.trim().toLowerCase();
    if (!q) return shortcuts;
    return shortcuts.filter(s =>
      (s.label || s.id || "").toLowerCase().includes(q) ||
      (s.keys || []).join(" ").toLowerCase().includes(q)
    );
  });

  function sendAction(action) {
    invoke("editor_view", { scene: false, action }).catch(() => {});
  }

  function onGlobalKey(event) {
    if (open && event.key === "Escape" && !capturingId) {
      event.preventDefault();
      cancelModal();
      return;
    }
    if ((event.ctrlKey || event.metaKey) && !event.altKey) {
      if (event.key === ",") {
        event.preventDefault();
        openModal("general");
      } else if (event.key === "=" || event.key === "+") {
        event.preventDefault();
        stepScale(1);
      } else if (event.key === "-") {
        event.preventDefault();
        stepScale(-1);
      } else if (event.key === "0") {
        event.preventDefault();
        applyScale(100);
      }
    }
  }

  onMount(() => {
    applyScale(scale);
    globalThis.openPreferences = openModal;
    globalThis.addEventListener("keydown", onGlobalKey);
    return () => {
      globalThis.removeEventListener("keydown", onGlobalKey);
    };
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="pref-backdrop" onclick={(e) => { if (e.target === e.currentTarget) cancelModal(); }}>
    <div class="pref-modal" role="dialog" aria-modal="true" aria-label="Preferences">
      <header class="pref-head">
        <span class="pref-title">Preferences</span>
        <div class="pref-seg" role="tablist" aria-label="Preference scope">
          <button type="button" role="tab" aria-selected={scope === "app"} class:active={scope === "app"} onclick={() => (scope = "app")}>App</button>
          <button type="button" role="tab" aria-selected={scope === "map"} class:active={scope === "map"} disabled={!scene} onclick={() => (scope = "map")}>This map</button>
        </div>
        <button type="button" class="pref-close" aria-label="Close preferences" onclick={cancelModal}>×</button>
      </header>

      <div class="pref-body">
        <nav class="pref-nav" aria-label="Preferences sections">
          {#each TABS as item (item.id)}
            <button type="button" class="pref-nav-item" class:active={tab === item.id} onclick={() => (tab = item.id)}>
              {item.label}
            </button>
          {/each}
        </nav>

        <section class="pref-pane">
          {#if tab === "general"}
            <div class="pref-group">
              <div class="pref-row-head">
                <span class="pref-label">Interface scale</span>
                <span class="pref-hint">Ctrl + = / Ctrl + -, Ctrl + 0 resets</span>
              </div>
              <div class="pref-seg pref-seg-wide">
                {#each SCALES as pct (pct)}
                  <button type="button" class:active={scale === pct} onclick={() => applyScale(pct)}>{pct}%</button>
                {/each}
              </div>
            </div>

            <hr class="pref-divider" />

            <div class="pref-group">
              <span class="pref-label">Controls</span>
              <div class="pref-seg">
                <button type="button" class:active={preset !== "osu!"} onclick={() => pickPreset("DAW")}>DAW-like keys</button>
                <button type="button" class:active={preset === "osu!"} onclick={() => pickPreset("osu!")}>osu! editor controls</button>
              </div>
              <p class="pref-caption">
                {preset === "osu!"
                  ? "Wheel scrolls time, Alt+wheel zooms, osu! editor shortcuts active."
                  : "Wheel scrolls the lanes, Ctrl+wheel zooms in time, 1-4 pick tools."}
              </p>
            </div>

            <hr class="pref-divider" />

            <div class="pref-group">
              <span class="pref-label">osu! Songs folder</span>
              <p class="pref-caption">Used to browse mapsets and resolve default hitsounds.</p>
              <div class="pref-path-row">
                <input type="text" readonly value={view.songsFolder || "Not configured"} />
                <button type="button" disabled={busy} onclick={() => invoke("pick_songs_folder").catch(() => {})}>Change…</button>
              </div>
            </div>

            <div class="pref-group">
              <span class="pref-label">Sample library folder</span>
              <div class="pref-path-row">
                <input type="text" readonly value={view.hitsoundLibrary || "No library folder selected"} />
                <button type="button" disabled={busy} onclick={() => invoke("pick_hitsound_library").catch(() => {})}>Add folder…</button>
              </div>
            </div>

          {:else if tab === "folders"}
            <div class="pref-group">
              <span class="pref-label">Songs folder</span>
              <div class="pref-path-row">
                <input type="text" readonly value={view.songsFolder || "Not configured"} />
                <button type="button" disabled={busy} onclick={() => invoke("pick_songs_folder").catch(() => {})}>Choose…</button>
              </div>
            </div>
            <div class="pref-group">
              <span class="pref-label">Hitsound library</span>
              <div class="pref-path-row">
                <input type="text" readonly value={view.hitsoundLibrary || "Not configured"} />
                <button type="button" disabled={busy} onclick={() => invoke("pick_hitsound_library").catch(() => {})}>Choose…</button>
                {#if view.hitsoundLibrary}
                  <button type="button" onclick={() => invoke("open_editor_folder", { folder: "library" }).catch(() => {})}>Open</button>
                {/if}
              </div>
            </div>
            {#if scene}
              <hr class="pref-divider" />
              <div class="pref-group">
                <span class="pref-label">Current mapset</span>
                <div class="pref-actions-inline">
                  <button type="button" onclick={() => invoke("open_editor_folder", { folder: "map" }).catch(() => {})}>Open map folder</button>
                  <button type="button" onclick={() => invoke("open_editor_folder", { folder: "project" }).catch(() => {})}>Open project folder</button>
                </div>
              </div>
            {/if}

          {:else if tab === "audio"}
            <div class="pref-group">
              <span class="pref-label">Output device</span>
              {#if audioConfig?.current}
                <p class="pref-caption">Active: {audioConfig.current}{audioConfig.effectiveBufferMs != null ? ` (${audioConfig.effectiveBufferMs} ms)` : ""}</p>
              {/if}
              <div class="pref-grid">
                <label class="pref-field">
                  <span>Backend</span>
                  <select bind:value={selectedBackend} disabled={audioLocked} onchange={() => loadAudio(selectedBackend)}>
                    {#each audioConfig?.available ?? [] as b (b.id)}
                      <option value={b.id}>{b.label}</option>
                    {/each}
                  </select>
                </label>
                <label class="pref-field">
                  <span>Device</span>
                  <select bind:value={selectedDevice} disabled={audioLocked || Boolean(audioError)}>
                    <option value="">{selectedBackend === "asio" ? "Choose an ASIO driver" : "Device default"}</option>
                    {#each audioConfig?.devices ?? [] as d (d.id)}
                      <option value={d.id}>{d.name}</option>
                    {/each}
                  </select>
                </label>
                <label class="pref-field">
                  <span>Buffer (ms)</span>
                  <input type="number" min="10" max="500" placeholder="Device default" bind:value={bufferMs} disabled={audioLocked} />
                </label>
              </div>
              {#if audioError}
                <p class="pref-error">{audioError}</p>
              {/if}
              <div class="pref-actions-inline">
                <button type="button" disabled={audioLocked || Boolean(audioError)} onclick={applyAudioOutput}>Apply output</button>
                {#if view.playing}
                  <span class="pref-hint">Stop playback to switch output device.</span>
                {/if}
              </div>
            </div>

            <hr class="pref-divider" />

            <div class="pref-group">
              <span class="pref-label">Playback</span>
              <div class="pref-grid">
                <label class="pref-field">
                  <span>Start playing from</span>
                  <select value={view.playbackStart ?? "cursor"} onchange={(e) => sendAction({ kind: "playback_start", value: e.currentTarget.value })}>
                    {#each playbackStartChoices as [val, lbl] (val)}
                      <option value={val}>{lbl}</option>
                    {/each}
                  </select>
                </label>
                <label class="pref-field">
                  <span>Song offset (ms)</span>
                  <input type="number" min="-60000" max="60000" value={view.offset ?? 0}
                    onchange={(e) => sendAction({ kind: "offset", ms: Number(e.currentTarget.value) || 0 })} />
                </label>
              </div>
              <label class="pref-check">
                <input type="checkbox" checked={Boolean(view.returnOnStop)}
                  onchange={(e) => sendAction({ kind: "return_on_stop", on: e.currentTarget.checked })} />
                <span>Return playhead to start position on Stop</span>
              </label>
            </div>

            <hr class="pref-divider" />

            <div class="pref-group">
              <span class="pref-label">MIDI input</span>
              <p class="pref-caption">Connect a MIDI keyboard, pad controller, or electronic drum kit for live audition and recording.</p>
              <div class="pref-path-row">
                <select value={selectedMidiPort} onchange={(e) => saveMidiPort(e.currentTarget.value)}>
                  <option value="__all__">All MIDI inputs ({midiPorts.length} detected)</option>
                  {#each midiPorts as port (port)}
                    <option value={port}>{port}</option>
                  {/each}
                  <option value="__none__">No MIDI input</option>
                </select>
                <button type="button" onclick={loadMidiPorts}>Rescan</button>
              </div>
            </div>

          {:else if tab === "view"}
            <div class="pref-group">
              <span class="pref-label">Panels and overlays</span>
              <label class="pref-check">
                <input type="checkbox" checked={view.panels?.laneNames !== false}
                  onchange={(e) => sendAction({ kind: "panel", panel: "lane_names", on: e.currentTarget.checked })} />
                <span>Show lane names</span>
              </label>
              <label class="pref-check">
                <input type="checkbox" checked={Boolean(view.classicBrowser)}
                  onchange={(e) => sendAction({ kind: "browser_style", classic: e.currentTarget.checked })} />
                <span>Classic Browser inside Inspector</span>
              </label>
            </div>

          {:else if tab === "shortcuts"}
            <div class="pref-group pref-shortcuts">
              <div class="pref-shortcut-bar">
                <div class="pref-seg">
                  <button type="button" class:active={preset === "DAW"} onclick={() => pickPreset("DAW")}>DAW-like</button>
                  <button type="button" class:active={preset === "osu!"} onclick={() => pickPreset("osu!")}>osu!</button>
                </div>
                <input type="search" class="pref-search" placeholder="Search commands or keys" bind:value={shortcutQuery} />
                <button type="button" onclick={resetAllShortcuts}>Reset all</button>
              </div>
              <div class="pref-shortcut-list">
                {#each filteredShortcuts as entry (entry.id)}
                  <div class="pref-shortcut-row" class:capturing={capturingId === entry.id}
                    role="group" onkeydown={(e) => onShortcutKeydown(e, entry)}>
                    <span class="pref-shortcut-name">{entry.label || entry.id}</span>
                    <button type="button" class="pref-key-chip" class:capturing={capturingId === entry.id}
                      onclick={() => (capturingId = capturingId === entry.id ? null : entry.id)}>
                      {capturingId === entry.id ? "Press keys…" : (entry.keys?.length ? entry.keys.map(shortcutName).join(", ") : "Unbound")}
                    </button>
                    <button type="button" class="pref-reset-btn" title="Clear shortcut" onclick={() => resetShortcut(entry)}>Clear</button>
                  </div>
                {/each}
              </div>
            </div>

          {:else if tab === "updates"}
            <div class="pref-group">
              <span class="pref-label">dawsu! Desktop</span>
              <p class="pref-caption">Cross-platform Rust + Svelte 5 studio build. Check GitHub Releases for newer builds.</p>
            </div>
          {/if}
        </section>
      </div>

      <footer class="pref-foot">
        <button type="button" onclick={cancelModal}>Cancel</button>
        <button type="button" class="pref-primary" onclick={commitModal}>Apply</button>
      </footer>
    </div>
  </div>
{/if}

<style>
  .pref-backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: grid;
    place-items: center;
    background: rgba(6, 8, 13, 0.68);
    backdrop-filter: blur(4px);
  }
  .pref-modal {
    display: flex;
    flex-direction: column;
    width: min(900px, calc(100vw - 48px));
    height: min(640px, calc(100vh - 48px));
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    color: var(--editor-text);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.62);
    overflow: hidden;
  }
  .pref-head {
    display: flex;
    align-items: center;
    gap: 16px;
    height: 56px;
    padding: 0 20px;
    border-bottom: 1px solid var(--editor-border);
    background: linear-gradient(180deg, #1e1e21 0%, #151517 100%);
  }
  .pref-title {
    font-size: 14px;
    font-weight: 600;
  }
  .pref-close {
    margin-left: auto;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--editor-muted);
    font-size: 18px;
    line-height: 1;
  }
  .pref-close:hover {
    color: var(--editor-white);
    background: var(--editor-hover);
  }
  .pref-seg {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 30px;
    padding: 3px;
    border-radius: 7px;
    background: var(--editor-canvas);
    border: 1px solid var(--editor-border);
  }
  .pref-seg button {
    height: 24px;
    min-height: 24px;
    padding: 0 12px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--editor-secondary);
    font-size: 12px;
  }
  .pref-seg button.active {
    background: var(--editor-selected);
    color: var(--editor-accent);
    font-weight: 600;
  }
  .pref-body {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .pref-nav {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 184px;
    padding: 14px 10px;
    border-right: 1px solid var(--editor-border);
    background: rgba(11, 14, 20, 0.45);
  }
  .pref-nav-item {
    position: relative;
    height: 32px;
    padding: 0 12px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--editor-secondary);
    text-align: left;
    font-size: 12px;
  }
  .pref-nav-item:hover {
    background: var(--editor-hover);
    color: var(--editor-white);
  }
  .pref-nav-item.active {
    background: var(--editor-selected);
    color: var(--editor-white);
    font-weight: 600;
  }
  .pref-nav-item.active::before {
    content: "";
    position: absolute;
    left: 2px;
    top: 6px;
    bottom: 6px;
    width: 3px;
    border-radius: 2px;
    background: var(--editor-accent);
  }
  .pref-pane {
    flex: 1;
    padding: 22px 28px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .pref-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pref-row-head {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  .pref-label {
    font-size: 13px;
    font-weight: 600;
    color: var(--editor-text);
  }
  .pref-hint,
  .pref-caption {
    margin: 0;
    font-size: 12px;
    color: var(--editor-muted);
  }
  .pref-divider {
    border: 0;
    border-top: 1px solid var(--editor-border);
    margin: 2px 0;
  }
  .pref-path-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .pref-path-row input {
    flex: 1;
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-canvas);
    color: var(--editor-readout);
    font-size: 12px;
  }
  .pref-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 12px;
  }
  .pref-field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--editor-secondary);
  }
  .pref-check {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--editor-text);
  }
  .pref-actions-inline {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .pref-error {
    margin: 0;
    font-size: 12px;
    color: var(--editor-error);
  }
  .pref-shortcuts {
    flex: 1;
    min-height: 0;
  }
  .pref-shortcut-bar {
    display: flex;
    gap: 10px;
    align-items: center;
  }
  .pref-search {
    flex: 1;
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-canvas);
    color: var(--editor-text);
    font-size: 12px;
  }
  .pref-shortcut-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 360px;
    overflow-y: auto;
    padding-right: 4px;
  }
  .pref-shortcut-row {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 36px;
    padding: 0 10px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.02);
  }
  .pref-shortcut-row.capturing {
    background: var(--editor-selected);
  }
  .pref-shortcut-name {
    flex: 1;
    font-size: 12px;
  }
  .pref-key-chip {
    min-width: 72px;
    height: 24px;
    min-height: 24px;
    padding: 0 10px;
    border: 1px solid var(--editor-border);
    border-radius: 5px;
    background: var(--editor-canvas);
    color: var(--editor-readout);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .pref-key-chip.capturing {
    border-color: var(--editor-accent);
    color: var(--editor-accent);
  }
  .pref-reset-btn {
    height: 24px;
    min-height: 24px;
    padding: 0 8px;
    font-size: 11px;
    color: var(--editor-muted);
    background: transparent;
  }
  .pref-foot {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 10px;
    height: 54px;
    padding: 0 20px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .pref-primary {
    background: var(--editor-accent);
    color: var(--editor-accentink);
    font-weight: 600;
  }
  .pref-primary:hover {
    background: var(--editor-focus);
  }
</style>
