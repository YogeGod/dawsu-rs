<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import { keyInput } from "../editor/helpers.js";
  import { readPort, writePort } from "../lib/midi.js";
  import {
    applyColorConfig, defaultColorConfig, readColorConfig, writeColorConfig,
  } from "../lib/theme.js";
  import "./preferences/preferences.css";
  import GeneralTab from "./preferences/GeneralTab.svelte";
  import AudioTab from "./preferences/AudioTab.svelte";
  import ColoursTab from "./preferences/ColoursTab.svelte";
  import ShortcutsTab from "./preferences/ShortcutsTab.svelte";
  import MidiVstTab from "./preferences/MidiVstTab.svelte";
  import UpdatesTab from "./preferences/UpdatesTab.svelte";

  const SCALES = [90, 100, 110, 125, 150, 175, 200];
  const TABS = [
    { id: "general", label: "General" },
    { id: "folders", label: "Folders" },
    { id: "audio", label: "Audio output" },
    { id: "view", label: "Editor view" },
    { id: "colours", label: "Colours" },
    { id: "shortcuts", label: "Shortcuts" },
    { id: "midi", label: "MIDI Input & VST3" },
    { id: "updates", label: "Updates" },
  ];

  let colorConfig = $state(readColorConfig());
  let expandedColorRow = $state(null);

  function updateColorConfig(next) {
    colorConfig = next;
    writeColorConfig(next);
    applyColorConfig(next);
  }

  function resetColours() {
    expandedColorRow = null;
    updateColorConfig(defaultColorConfig());
  }

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
  let soundfontSummary = $state(null);
  let soundfontError = $state("");
  let soundfontLoading = $state(false);

  // Updates state: checked only when the user clicks "Check for updates".
  let updateChecking = $state(false);
  let updateDownloading = $state(false);
  let updateStatus = $state("");
  let updateResult = $state(null);
  let updateError = $state("");
  let updateCopied = $state(false);

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
      bridge.callAction("paintSoon");
    });
  }

  function stepScale(delta) {
    const idx = SCALES.indexOf(scale);
    const next = SCALES[Math.max(0, Math.min(SCALES.length - 1, (idx < 0 ? 1 : idx) + delta))];
    applyScale(next);
    bridge.showToast({
      tone: "info",
      title: `Interface scale ${next}%`,
      body: "Press Ctrl + 0 to reset to 100%.",
    });
  }

  async function loadShortcuts() {
    try {
      // Through the editor, so tool keys elsewhere follow a rebind, reset or preset switch.
      const res = await (bridge.callAction("refreshShortcuts") ?? invoke("editor_shortcuts"));
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
      midiPorts = (await invoke("midi_action", { action: { kind: "input_ports" } })) ?? [];
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

  async function retryBlockedVst3() {
    if (vst3Scanning) return;
    vst3Scanning = true;
    vst3Error = "";
    try {
      vst3Report = await invoke("vst3_scan", { retryBlocked: true });
    } catch (err) {
      vst3Error = String(err);
    } finally {
      vst3Scanning = false;
    }
  }

  async function pickSoundfont() {
    if (soundfontLoading) return;
    soundfontLoading = true;
    soundfontError = "";
    try {
      const path = await invoke("pick_soundfont_file");
      if (path) {
        soundfontSummary = await invoke("midi_action", { action: { kind: "inspect_soundfont", path } });
      }
    } catch (err) {
      soundfontError = String(err);
      soundfontSummary = null;
    } finally {
      soundfontLoading = false;
    }
  }

  async function auditionSoundfont(path) {
    if (!path) return;
    soundfontError = "";
    try {
      await invoke("midi_action", {
        action: { kind: "audition_soundfont", path, presetIndex: 0, pitch: 60, velocity: 100, durationMs: 300 },
      });
    } catch (err) {
      soundfontError = String(err);
    }
  }

  async function checkForUpdates() {
    if (updateChecking || updateDownloading) return;
    updateChecking = true;
    updateError = "";
    updateStatus = "";
    updateCopied = false;
    try {
      updateResult = await invoke("check_for_updates");
    } catch (err) {
      updateResult = null;
      updateError = String(err);
    } finally {
      updateChecking = false;
    }
  }

  async function downloadAndInstallUpdate() {
    if (updateChecking || updateDownloading || !(updateResult?.installerUrl || updateResult?.portableUrl) || !updateResult?.sha256sumsUrl) return;
    updateDownloading = true;
    updateError = "";
    updateStatus = `Downloading and verifying dawsu! v${updateResult.latestVersion}…`;
    try {
      const downloaded = await invoke("download_update");
      updateStatus = `v${downloaded.version} verified (SHA-256). Closing dawsu! to install…`;
      await invoke("install_update_on_close");
    } catch (err) {
      updateError = String(err);
      updateStatus = "";
    } finally {
      updateDownloading = false;
    }
  }

  async function copyReleaseUrl() {
    if (!updateResult?.releaseUrl) return;
    try {
      await globalThis.navigator?.clipboard?.writeText(updateResult.releaseUrl);
      updateCopied = true;
    } catch {
      updateCopied = false;
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
    bridge.setModalOpen("preferences", open);
    return () => { bridge.setModalOpen("preferences", false); };
  });

  // Command Palette stays out of the way while capturing a custom shortcut binding.
  $effect(() => {
    bridge.setShortcutCapturing(Boolean(open && capturingId));
    return () => { bridge.setShortcutCapturing(false); };
  });

  // The tabs with something to put back: General's interface scale, Colours, and Shortcuts.
  const resetLabel = $derived(scope !== "app" ? null
    : tab === "general" ? "Reset scale to 100%"
    : tab === "colours" ? "Defaults"
    : tab === "shortcuts" ? "Reset all shortcuts" : null);

  function resetTab() {
    if (tab === "general") applyScale(100);
    else if (tab === "colours") resetColours();
    else if (tab === "shortcuts") resetAllShortcuts();
  }

  async function pickPreset(nextPreset) {
    try {
      const next = await invoke("editor_view", { scene: false, action: { kind: "key_preset", name: nextPreset } });
      applyReturnedView(next);
      if (next?.error) {
        bridge.showToast({ tone: "error", title: "Controls", body: next.error });
        return;
      }
      await loadShortcuts();
      bridge.showToast({
        tone: "success",
        title: "Controls updated",
        body: nextPreset === "osu!" ? "Switched to osu! editor controls." : "Switched to DAW-like keys.",
      });
    } catch (err) {
      bridge.showToast({ tone: "error", title: "Controls", body: String(err) });
    }
  }

  async function applyAudioOutput() {
    if (audioLocked || !audioConfig || audioError) return;
    const minMs = selectedBackend === "mme" || selectedBackend === "direct_sound" ? 10 : 2;
    const ms = bufferMs === "" ? null : Number(bufferMs);
    if (ms !== null && (!Number.isInteger(ms) || ms < minMs || ms > 500)) {
      audioError = `Buffer must be between ${minMs} and 500 ms.`;
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
        bridge.showToast({ tone: "error", title: "Audio output", body: next.error });
        return;
      }
      await loadAudio(selectedBackend);
      bridge.showToast({
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
          bridge.showToast({ tone: "error", title: "Shortcut", body: next.error });
          return;
        }
        await loadShortcuts();
      })
      .catch(err => {
        bridge.showToast({ tone: "error", title: "Shortcut", body: String(err) });
      });
  }

  async function resetShortcut(entry) {
    try {
      const next = await invoke("editor_view", { scene: false, action: { kind: "change_shortcut", id: entry.id, chord: null } });
      applyReturnedView(next);
      if (next?.error) {
        bridge.showToast({ tone: "error", title: "Shortcut", body: next.error });
        return;
      }
      await loadShortcuts();
    } catch (err) {
      bridge.showToast({ tone: "error", title: "Shortcut", body: String(err) });
    }
  }

  async function resetAllShortcuts() {
    try {
      const next = await invoke("editor_view", { scene: false, action: { kind: "reset_shortcuts" } });
      applyReturnedView(next);
      if (next?.error) {
        bridge.showToast({ tone: "error", title: "Shortcuts", body: next.error });
        return;
      }
      await loadShortcuts();
    } catch (err) {
      bridge.showToast({ tone: "error", title: "Shortcuts", body: String(err) });
    }
  }

  function applyReturnedView(next) {
    if (!next) return;
    if (typeof bridge.actions.show === "function") bridge.callAction("show", next);
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
    const disposeModal = bridge.registerModal("openPreferencesModal", openModal);
    globalThis.addEventListener("keydown", onGlobalKey);
    let startupTimer = null;
    const isDesktop = typeof window !== "undefined" && Boolean(window.__TAURI__?.core?.invoke) && !globalThis.navigator?.webdriver;
    if (isDesktop) {
      startupTimer = setTimeout(async () => {
        if (bridge.view?.autoCheckUpdates === false || updateChecking || updateDownloading) return;
        try {
          const res = await invoke("check_for_updates");
          if (res?.updateAvailable) {
            updateResult = res;
            bridge.showToast({
              tone: "info",
              title: `dawsu! v${res.latestVersion} is available`,
              body: "Open Preferences › Updates to download and install.",
            });
          }
        } catch {
          // Quiet startup check: stay silent when offline or rate-limited.
        }
      }, 2500);
    }
    return () => {
      if (startupTimer !== null) clearTimeout(startupTimer);
      disposeModal();
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
            <GeneralTab
              scales={SCALES}
              {scale}
              {preset}
              {view}
              {busy}
              onapplyscale={applyScale}
              onpickpreset={pickPreset}
              onpicksongs={pickSongsFolder}
              onpicklibrary={pickLibraryFolder}
              onsendaction={sendAction}
            />
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
            <AudioTab
              {audioConfig}
              bind:selectedBackend
              bind:selectedDevice
              bind:bufferMs
              {audioError}
              {audioLocked}
              {view}
              onloadaudio={loadAudio}
              onapplyaudio={applyAudioOutput}
              onsendaction={sendAction}
            />
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
              <label class="pref-check">
                <input type="checkbox" checked={Boolean(view.stepPadsMode)}
                  onchange={(e) => sendAction({ kind: "step_pads_mode", on: e.currentTarget.checked })} />
                <span>Show beat step-sequencer pads on lane rows</span>
              </label>
            </div>
          {:else if tab === "colours"}
            <ColoursTab
              {colorConfig}
              bind:expandedColorRow
              onupdate={updateColorConfig}
            />
          {:else if tab === "shortcuts"}
            <ShortcutsTab
              {shortcuts}
              {preset}
              bind:shortcutQuery
              bind:capturingId
              onpickpreset={pickPreset}
              onresetall={resetAllShortcuts}
              onshortcutkeydown={onShortcutKeydown}
              onresetshortcut={resetShortcut}
            />
          {:else if tab === "midi"}
            <MidiVstTab
              {midiPorts}
              {midiPort}
              {midiError}
              {vst3Scanning}
              {vst3Report}
              {vst3Error}
              {soundfontSummary}
              {soundfontError}
              {soundfontLoading}
              onloadmidi={loadMidiPorts}
              onpickmidi={pickMidiPort}
              onscanvst3={scanVst3}
              onretryblocked={retryBlockedVst3}
              onpicksoundfont={pickSoundfont}
              onauditionsoundfont={auditionSoundfont}
            />
          {:else if tab === "updates"}
            <UpdatesTab
              {view}
              {updateChecking}
              {updateDownloading}
              {updateStatus}
              {updateResult}
              {updateError}
              {updateCopied}
              oncheck={checkForUpdates}
              ondownload={downloadAndInstallUpdate}
              oncopyurl={copyReleaseUrl}
              onsendaction={sendAction}
            />
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
