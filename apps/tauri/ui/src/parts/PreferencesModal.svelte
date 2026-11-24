<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import { keyInput, playbackStartChoices, shortcutName } from "../editor/helpers.js";
  import { readPort, scanSummary, writePort } from "../lib/midi.js";

  const SCALES = [90, 100, 110, 125, 150, 175, 200];
  const TABS = [
    { id: "general", label: "General" },
    { id: "folders", label: "Folders" },
    { id: "audio", label: "Audio output" },
    { id: "view", label: "Editor view" },
    { id: "shortcuts", label: "Shortcuts" },
    { id: "midi", label: "MIDI Input & VST3" },
    { id: "updates", label: "Updates" },
  ];

  let open = $state(false);
  let scope = $state("app");
  let tab = $state("general");
  let scale = $state(readSavedScale());

  // Shortcuts state
  let shortcuts = $state([]);
  let preset = $state("DAW");
  let shortcutQuery = $state("");
  let capturingId = $state(null);

  // Audio state
  let audioConfig = $state(null);
  let selectedBackend = $state("");
  let selectedDevice = $state("");
  let bufferMs = $state("");
  let audioError = $state("");

  // MIDI Input & VST3 state: the port MIDI Record arms is kept in localStorage (lib/midi.js).
  let midiPorts = $state([]);
  let midiPort = $state("");
  let midiError = $state("");
  let vst3Scanning = $state(false);
  let vst3Report = $state(null);
  let vst3Error = $state("");

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
        globalThis.document.documentElement.dataset.uiScale = String(next);
      }
    } catch {}
    // The canvases size their backing store by the scale: lay out again and repaint at once,
    // so no stretched frame or empty strip shows until the next resize.
    globalThis.requestAnimationFrame?.(() => {
      try { globalThis.dispatchEvent?.(new Event("resize")); } catch {}
      globalThis.paintSoon?.();
    });
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

  const storage = () => { try { return globalThis.localStorage; } catch { return null; } };

  async function loadMidiPorts() {
    try {
      midiPorts = (await invoke("midi_input_ports")) ?? [];
      midiError = "";
      const saved = readPort(storage());
      midiPort = saved != null && saved < midiPorts.length ? String(saved) : "";
    } catch (err) {
      midiPorts = [];
      midiError = String(err);
    }
  }

  function pickMidiPort(value) {
    midiPort = value;
    writePort(storage(), value === "" ? null : Number(value));
  }

  async function scanVst3() {
    if (vst3Scanning) return;
    vst3Scanning = true;
    vst3Error = "";
    try {
      vst3Report = await invoke("vst3_scan");
    } catch (err) {
      vst3Error = String(err);
    } finally {
      vst3Scanning = false;
    }
  }

  async function openModal(initialTab = "general") {
    tab = TABS.some(t => t.id === initialTab) ? initialTab : "general";
    scope = "app";
    capturingId = null;
    open = true;
    await Promise.all([loadShortcuts(), loadAudio(null), loadMidiPorts()]);
  }

  // Every choice here applies the moment it is made, so there is nothing to apply or cancel: Close only closes.
  function closeModal() {
    capturingId = null;
    open = false;
  }

  // The editor's timeline shortcuts stay off while this is open (boot.js reads the flag).
  $effect(() => {
    globalThis.dawsuPreferencesOpen = open;
    return () => { globalThis.dawsuPreferencesOpen = false; };
  });

  // The tabs with something to put back: General's interface scale and the Shortcuts tab's keys.
  const resetLabel = $derived(scope !== "app" ? null
    : tab === "general" ? "Reset scale to 100%" : tab === "shortcuts" ? "Reset all shortcuts" : null);

  function resetTab() {
    if (tab === "general") applyScale(100);
    else if (tab === "shortcuts") resetAllShortcuts();
  }

  async function pickPreset(nextPreset) {
    try {
      const next = await invoke("editor_view", { scene: false, action: { kind: "key_preset", name: nextPreset } });
      applyReturnedView(next);
      if (next?.error) {
        globalThis.showToast?.({ tone: "error", title: "Controls", body: next.error });
        return;
      }
      await loadShortcuts();
      globalThis.showToast?.({
        tone: "success",
        title: "Controls updated",
        body: nextPreset === "osu!" ? "Switched to osu! editor controls." : "Switched to DAW-like keys.",
      });
    } catch (err) {
      globalThis.showToast?.({ tone: "error", title: "Controls", body: String(err) });
    }
  }

  async function applyAudioOutput() {
    if (audioLocked || !audioConfig || audioError) return;
    const ms = bufferMs === "" ? null : Number(bufferMs);
    if (ms !== null && (!Number.isInteger(ms) || ms < 10 || ms > 500)) {
      audioError = "Buffer must be between 10 and 500 ms.";
      return;
    }
    try {
      const next = await invoke("editor_view", {
        scene: false,
        action: { kind: "output_settings", backend: selectedBackend, device_id: selectedDevice || null, ms },
      });
      applyReturnedView(next);
      if (next?.error) {
        audioError = next.error;
        globalThis.showToast?.({ tone: "error", title: "Audio output", body: next.error });
        return;
      }
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

  // Rust sends each key as its formatted name ("Ctrl+O"); an older shape may still be a chord object.
  function keyLabel(key) {
    if (typeof key === "string") return key;
    if (key && typeof key.code === "string") return shortcutName(key) || key.key || "";
    return "";
  }

  function keysLabel(keys) {
    const names = (keys ?? []).map(keyLabel).filter(Boolean);
    return names.length ? names.join(", ") : "Unbound";
  }

  function onShortcutKeydown(event, entry) {
    if (capturingId !== entry.id) return;
    if (["Control", "Shift", "Alt", "Meta"].includes(event.key) || event.repeat) return;
    event.preventDefault();
    event.stopPropagation();
    const chord = keyInput(event);
    if (chord.key === "Escape" && !chord.ctrl && !chord.shift && !chord.alt) {
      capturingId = null;
      return;
    }
    capturingId = null;
    invoke("editor_view", { scene: false, action: { kind: "change_shortcut", id: entry.id, chord } })
      .then(async next => {
        applyReturnedView(next);
        if (next?.error) {
          globalThis.showToast?.({ tone: "error", title: "Shortcut", body: next.error });
          return;
        }
        await loadShortcuts();
      })
      .catch(err => {
        globalThis.showToast?.({ tone: "error", title: "Shortcut", body: String(err) });
      });
  }

  async function resetShortcut(entry) {
    try {
      const next = await invoke("editor_view", { scene: false, action: { kind: "change_shortcut", id: entry.id, chord: null } });
      applyReturnedView(next);
      if (next?.error) {
        globalThis.showToast?.({ tone: "error", title: "Shortcut", body: next.error });
        return;
      }
      await loadShortcuts();
    } catch (err) {
      globalThis.showToast?.({ tone: "error", title: "Shortcut", body: String(err) });
    }
  }

  async function resetAllShortcuts() {
    try {
      const next = await invoke("editor_view", { scene: false, action: { kind: "reset_shortcuts" } });
      applyReturnedView(next);
      if (next?.error) {
        globalThis.showToast?.({ tone: "error", title: "Shortcuts", body: next.error });
        return;
      }
      await loadShortcuts();
    } catch (err) {
      globalThis.showToast?.({ tone: "error", title: "Shortcuts", body: String(err) });
    }
  }

  const filteredShortcuts = $derived.by(() => {
    const q = shortcutQuery.trim().toLowerCase();
    if (!q) return shortcuts;
    return shortcuts.filter(s =>
      (s.label || s.id || "").toLowerCase().includes(q) ||
      (s.keys || []).map(keyLabel).join(" ").toLowerCase().includes(q)
    );
  });

  function applyReturnedView(next) {
    if (!next) return;
    if (typeof globalThis.show === "function") globalThis.show(next);
    else bridge.set(next, next.scene ?? bridge.scene);
  }

  function sendAction(action) {
    invoke("editor_view", { scene: false, action }).then(applyReturnedView).catch(() => {});
  }

  function pickSongsFolder() {
    invoke("pick_songs_folder").then(applyReturnedView).catch(() => {});
  }

  function pickLibraryFolder() {
    invoke("pick_hitsound_library").then(applyReturnedView).catch(() => {});
  }

  function onGlobalKey(event) {
    if (open && event.key === "Escape" && !capturingId) {
      event.preventDefault();
      closeModal();
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
    // dialogs-settings.js's openPreferences (the menus' Preferences…) calls this; it is never the
    // global openPreferences itself, so the two cannot call each other forever.
    globalThis.openPreferencesModal = openModal;
    globalThis.addEventListener("keydown", onGlobalKey);
    return () => {
      if (globalThis.openPreferencesModal === openModal) delete globalThis.openPreferencesModal;
      globalThis.removeEventListener("keydown", onGlobalKey);
    };
  });
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="pref-backdrop" onclick={(e) => { if (e.target === e.currentTarget) closeModal(); }}>
    <div class="pref-modal" role="dialog" aria-modal="true" aria-label="Preferences">
      <header class="pref-head">
        <span class="pref-title">Preferences</span>
        <div class="pref-seg" role="tablist" aria-label="Preference scope">
          <button type="button" role="tab" aria-selected={scope === "app"} class:active={scope === "app"} onclick={() => (scope = "app")}>App</button>
          <button type="button" role="tab" aria-selected={scope === "map"} class:active={scope === "map"} disabled={!scene} onclick={() => (scope = "map")}>This map</button>
        </div>
        <button type="button" class="pref-close" aria-label="Close preferences" onclick={closeModal}>×</button>
      </header>

      <div class="pref-body">
        {#if scope === "map" && scene}
          <section class="pref-pane">
            <div class="pref-group">
              <span class="pref-label">This map</span>
              <p class="pref-caption">{scene.path || "Active difficulty"}</p>
              <div class="pref-grid">
                <label class="pref-field">
                  <span>Song offset (ms)</span>
                  <input type="number" min="-60000" max="60000" value={view.offset ?? 0}
                    onchange={(e) => sendAction({ kind: "offset", ms: Number(e.currentTarget.value) || 0 })} />
                </label>
              </div>
              <label class="pref-check">
                <input type="checkbox" checked={Boolean(view.hsMode)}
                  onchange={(e) => sendAction({ kind: "hs_mode", on: e.currentTarget.checked })} />
                <span>Hitsounding mode (show drawn hits as live circles)</span>
              </label>
            </div>
            <hr class="pref-divider" />
            <div class="pref-group">
              <span class="pref-label">Mapset folders</span>
              <div class="pref-actions-inline">
                <button type="button" onclick={() => invoke("open_editor_folder", { folder: "map" }).catch(() => {})}>Open map folder</button>
                <button type="button" onclick={() => invoke("open_editor_folder", { folder: "project" }).catch(() => {})}>Open project folder</button>
              </div>
            </div>
          </section>
        {:else}
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
                <button type="button" class:active={preset === "osu!"} onclick={() => pickPreset("osu!")}>osu! editor controls</button>
                <button type="button" class:active={preset !== "osu!"} onclick={() => pickPreset("DAW")}>DAW-like keys</button>
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
                <button type="button" disabled={busy} onclick={pickSongsFolder}>Change…</button>
              </div>
            </div>

            <div class="pref-group">
              <span class="pref-label">Sample library folder</span>
              <div class="pref-path-row">
                <input type="text" readonly value={view.hitsoundLibrary || "No library folder selected"} />
                <button type="button" disabled={busy} onclick={pickLibraryFolder}>Add folder…</button>
              </div>
            </div>

          {:else if tab === "folders"}
            <div class="pref-group">
              <span class="pref-label">Songs folder</span>
              <div class="pref-path-row">
                <input type="text" readonly value={view.songsFolder || "Not configured"} />
                <button type="button" disabled={busy} onclick={pickSongsFolder}>Choose…</button>
              </div>
            </div>
            <div class="pref-group">
              <span class="pref-label">Hitsound library</span>
              <div class="pref-path-row">
                <input type="text" readonly value={view.hitsoundLibrary || "Not configured"} />
                <button type="button" disabled={busy} onclick={pickLibraryFolder}>Choose…</button>
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
                  <button type="button" class:active={preset === "osu!"} onclick={() => pickPreset("osu!")}>osu!</button>
                  <button type="button" class:active={preset === "DAW"} onclick={() => pickPreset("DAW")}>DAW-like</button>
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
                      {capturingId === entry.id ? "Press keys… · Esc cancels" : keysLabel(entry.keys)}
                    </button>
                    <button type="button" class="pref-reset-btn" title="Clear shortcut" onclick={() => resetShortcut(entry)}>Clear</button>
                  </div>
                {/each}
              </div>
            </div>

          {:else if tab === "midi"}
            <div class="pref-group">
              <div class="pref-row-head">
                <span class="pref-label">MIDI input</span>
                <button type="button" onclick={loadMidiPorts}>Refresh</button>
              </div>
              <p class="pref-caption">MIDI Record in the transport bar listens on this port.</p>
              {#if midiPorts.length}
                <label class="pref-field">
                  <span>Input port</span>
                  <select value={midiPort} onchange={(e) => pickMidiPort(e.currentTarget.value)}>
                    <option value="">First available port</option>
                    {#each midiPorts as name, index (index)}
                      <option value={String(index)}>{name}</option>
                    {/each}
                  </select>
                </label>
              {:else}
                <p class="pref-caption">No MIDI input ports found. Connect a MIDI keyboard or drum pad, then press Refresh.</p>
              {/if}
              {#if midiError}
                <p class="pref-error">{midiError}</p>
              {/if}
            </div>

            <hr class="pref-divider" />

            <div class="pref-group">
              <div class="pref-row-head">
                <span class="pref-label">VST3 plug-ins</span>
                <button type="button" disabled={vst3Scanning} onclick={scanVst3}>{vst3Scanning ? "Scanning…" : "Scan VST3 plug-ins"}</button>
              </div>
              <p class="pref-caption">Scans the system VST3 folder. Keysound tracks can use the instruments found (Map › Keysounds & VST3…).</p>
              {#if vst3Error}
                <p class="pref-error">{vst3Error}</p>
              {:else if vst3Report}
                <p class="pref-caption">{scanSummary(vst3Report)}</p>
                {#if vst3Report.plugins?.length}
                  <ul class="pref-plugin-list">
                    {#each vst3Report.plugins as plugin (plugin.uid ?? plugin.name)}
                      <li><span>{plugin.name}</span><span class="pref-hint">{plugin.vendor}{plugin.is_instrument ? " · Instrument" : plugin.is_effect ? " · Effect" : ""}</span></li>
                    {/each}
                  </ul>
                {/if}
              {/if}
            </div>

          {:else if tab === "updates"}
            <div class="pref-group">
              <span class="pref-label">dawsu! Desktop</span>
              <p class="pref-caption">Cross-platform Rust + Svelte 5 studio build. Check GitHub Releases for newer builds.</p>
            </div>
          {/if}
        </section>
        {/if}
      </div>

      <footer class="pref-foot">
        {#if resetLabel}
          <button type="button" class="pref-reset-tab" onclick={resetTab}>{resetLabel}</button>
        {/if}
        <button type="button" class="pref-primary" onclick={closeModal}>Close</button>
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
    background: var(--editor-chrome);
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
  .pref-reset-tab {
    margin-right: auto;
  }
  .pref-primary {
    background: var(--editor-accent);
    color: var(--editor-accentink);
    font-weight: 600;
  }
  .pref-primary:hover {
    background: var(--editor-focus);
  }
  .pref-plugin-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    max-height: 220px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }
  .pref-plugin-list li {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 6px 10px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.025);
    font-size: 12px;
  }
</style>
