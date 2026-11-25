<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import DrumInstrumentBar from "./DrumInstrumentBar.svelte";
  import DrumMapEditorModal from "./DrumMapEditorModal.svelte";
  import Vst3BounceModal from "./Vst3BounceModal.svelte";

  let barOpen = $state(false);
  let mapEditorOpen = $state(false);
  let bounceOpen = $state(false);
  let busy = $state(false);
  let plugins = $state([]);
  let bounceSamples = $state([]);
  let baseBytes = $state(0);
  let bouncedBytes = $state(0);
  let bounceFormat = $state("ogg_vorbis");
  let bounceMode = $state("tiered");
  let bounceAutoTrim = $state(true);
  let bounceNormalize = $state(true);

  const drumView = $derived(bridge.view?.drumView ?? null);
  const drumVst = $derived(bridge.view?.drumVst ?? null);
  const activeInstrumentId = $derived(drumVst?.loaded ? (drumVst.pluginUid || drumVst.pluginPath || "vst3") : "builtin:stock");
  const presets = $derived((drumView?.presets ?? []).map(name => ({ id: name, name })));
  const rows = $derived(drumView?.rows ?? []);

  // Pause timeline shortcuts while a Drum Studio modal is open.
  $effect(() => {
    if (!mapEditorOpen && !bounceOpen) return;
    globalThis.dawsuModalOpen = true;
    return () => {
      if (!mapEditorOpen && !bounceOpen) globalThis.dawsuModalOpen = false;
    };
  });

  function reportError(title, err) {
    globalThis.showToast?.({ tone: "error", title, body: String(err) });
  }

  async function scanPlugins() {
    try {
      const res = await invoke("drum_vst_action", { payload: { kind: "list_installed" } });
      plugins = (res?.plugins ?? []).map(p => ({
        id: p.uid || p.path || p.name,
        name: p.name,
        vendor: p.vendor || "VST3",
        path: p.path,
        uid: p.uid ?? null,
      }));
    } catch (err) {
      plugins = [];
      reportError("Scan VST3", err);
    }
  }

  async function selectInstrument(choice) {
    if (!choice) return;
    if (choice.kind === "builtin") {
      try {
        await invoke("drum_vst_action", { payload: { kind: "toggle", enabled: false } });
        globalThis.act?.(null, true);
      } catch (err) {
        reportError("Drum Instrument", err);
      }
      return;
    }
    if (!choice.path) return;
    busy = true;
    try {
      await invoke("drum_vst_action", {
        payload: { kind: "load", pluginPath: choice.path, pluginUid: choice.uid ?? null },
      });
      globalThis.act?.(null, true);
    } catch (err) {
      reportError("Load Drum VST3", err);
    } finally {
      busy = false;
    }
  }

  async function selectPreset(name) {
    const map = bridge.scene?.path;
    if (!map || !name) return;
    try {
      await invoke("drum_map_action", { map, action: { kind: "load_builtin", name } });
      globalThis.act?.(null, true);
    } catch (err) {
      reportError("Drum Map Preset", err);
    }
  }

  async function testRow(index) {
    const note = rows[index]?.midiNote ?? 36;
    await invoke("drum_vst_action", { payload: { kind: "preview_note", note, velocity: 110 } }).catch(() => {});
  }

  async function setChoke(index, group) {
    try {
      await invoke("drum_vst_action", {
        payload: { kind: "set_choke_group", rowIndex: index, chokeGroup: group },
      });
      globalThis.act?.(null, true);
    } catch (err) {
      reportError("Choke Group", err);
    }
  }

  async function importDrumMap() {
    try {
      const res = await invoke("drum_vst_action", { payload: { kind: "import_dawsudrum" } });
      if (res?.imported) globalThis.act?.(null, true);
    } catch (err) {
      reportError("Import Drum Map", err);
    }
  }

  async function exportDrumMap() {
    try {
      const res = await invoke("drum_vst_action", { payload: { kind: "export_dawsudrum" } });
      if (res?.exported) {
        globalThis.showToast?.({ tone: "success", title: "Saved drum map", body: res.path });
      }
    } catch (err) {
      reportError("Save Drum Map", err);
    }
  }

  async function previewBounce({ mode, format }) {
    busy = true;
    try {
      const plan = await invoke("drum_vst_action", {
        payload: { kind: "plan_render", exact: mode === "exact" },
      });
      const jobs = plan?.jobs ?? [];
      const ext = format === "wav16" ? "wav" : "ogg";
      const avgBytes = format === "wav16" ? 96000 : 18000;
      bounceSamples = jobs.map(j => ({
        filename: String(j.filename ?? "drum.wav").replace(/\.(wav|ogg)$/i, `.${ext}`),
        durationMs: 250,
        peakDbfs: format === "wav16" ? -1.0 : -1.5,
        usageCount: 1,
        silent: false,
        pitch: j.pitch,
        velocity: j.midiVelocity,
      }));
      const realBase = Number(plan?.baseBytes) || 0;
      const estBounced = bounceSamples.length * avgBytes;
      const budget = await invoke("bounce_osz_budget", { baseBytes: realBase, bouncedBytes: estBounced }).catch(() => null);
      baseBytes = budget?.baseBytes ?? realBase;
      bouncedBytes = budget?.bouncedBytes ?? estBounced;
    } catch (err) {
      bounceSamples = [];
      reportError("Bounce Preview", err);
    } finally {
      busy = false;
    }
  }

  async function commitBounce() {
    busy = true;
    try {
      const res = await invoke("drum_vst_action", {
        payload: {
          kind: "render_wavs",
          exact: bounceMode === "exact",
          format: bounceFormat,
          normalize: bounceNormalize,
          autoTrim: bounceAutoTrim,
        },
      });
      bounceOpen = false;
      globalThis.showToast?.({
        tone: "success",
        title: "Bounced VST3 drums",
        body: `Staged ${res?.renderedCount ?? 0} samples across ${res?.hitCount ?? 0} hits.`,
      });
      globalThis.act?.(null, true);
    } catch (err) {
      reportError("Bounce VST3 Kit", err);
    } finally {
      busy = false;
    }
  }

  function openStudio() {
    if (!bridge.scene) return;
    barOpen = true;
    if (!plugins.length) scanPlugins();
  }

  function openMapEditor() {
    if (!bridge.scene) return;
    mapEditorOpen = true;
  }

  function openBounce() {
    if (!bridge.scene) return;
    bounceOpen = true;
    previewBounce({ mode: bounceMode, format: bounceFormat });
  }

  onMount(() => {
    globalThis.openDrumStudio = openStudio;
    globalThis.openDrumMapEditor = openMapEditor;
    globalThis.openVst3Bounce = openBounce;
  });
</script>

{#if bridge.scene && barOpen}
  <div class="ds-bar-dock">
    <DrumInstrumentBar
      open={barOpen}
      {plugins}
      {activeInstrumentId}
      sidecarAvailable={true}
      onselect={selectInstrument}
      onscan={scanPlugins}
      onopenmap={openMapEditor}
      onadvanced={() => globalThis.openDrumVstModal?.()}
      onopenbounce={openBounce}
      onclose={() => { barOpen = false; }}
    />
  </div>
{/if}

<DrumMapEditorModal
  open={mapEditorOpen}
  {presets}
  {rows}
  onselectpreset={selectPreset}
  ontest={testRow}
  onsetchoke={setChoke}
  onimport={importDrumMap}
  onexport={exportDrumMap}
  onclose={() => { mapEditorOpen = false; }}
/>

<Vst3BounceModal
  open={bounceOpen}
  {busy}
  {baseBytes}
  {bouncedBytes}
  samples={bounceSamples}
  bind:format={bounceFormat}
  bind:mode={bounceMode}
  bind:autoTrim={bounceAutoTrim}
  bind:normalize={bounceNormalize}
  onrender={previewBounce}
  onaudition={sample => invoke("drum_vst_action", { payload: { kind: "preview_note", note: sample.pitch ?? 36, velocity: sample.velocity ?? 100 } }).catch(() => {})}
  oncommit={commitBounce}
  onclose={() => { bounceOpen = false; }}
/>

<style>
  .ds-bar-dock {
    position: fixed;
    left: 12px;
    right: 12px;
    bottom: 34px;
    z-index: 32;
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    overflow: hidden;
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
  }
</style>
