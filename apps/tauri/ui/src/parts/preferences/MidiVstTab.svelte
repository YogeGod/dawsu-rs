<script>
  import { scanSummary } from "../../lib/midi.js";
  import { readDrumVst3, writeDrumVst3 } from "../../lib/drumVst3.js";

  let {
    midiPorts = [],
    midiPort = "",
    midiError = "",
    vst3Scanning = false,
    vst3Report = null,
    vst3Error = "",
    soundfontSummary = null,
    soundfontError = "",
    soundfontLoading = false,
    onloadmidi,
    onpickmidi,
    onscanvst3,
    onretryblocked = null,
    onpicksoundfont = null,
    onauditionsoundfont = null,
  } = $props();

  // "Use VST3 sounds for drum map": the same saved setting Drum Map Setup and the drum detection
  // window switch (lib/drumVst3.js), so one set there shows here and the other way round.
  const storage = () => { try { return globalThis.localStorage; } catch { return null; } };
  let drumVst3 = $state(readDrumVst3(storage()));
  function toggleDrumVst3(checked) {
    drumVst3 = writeDrumVst3(storage(), { ...readDrumVst3(storage()), enabled: checked });
  }
</script>

<div class="pref-group">
  <div class="pref-row-head">
    <span class="pref-label">MIDI input</span>
    <button type="button" onclick={onloadmidi}>Refresh</button>
  </div>
  <p class="pref-caption">MIDI Record in the transport bar listens on this port.</p>
  {#if midiPorts.length}
    <label class="pref-field">
      <span>Input port</span>
      <select value={midiPort} onchange={(e) => onpickmidi(e.currentTarget.value)}>
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
    <button type="button" disabled={vst3Scanning} onclick={onscanvst3}>{vst3Scanning ? "Scanning…" : "Scan VST3 plug-ins"}</button>
  </div>
  <p class="pref-caption">Scans the system VST3 folder. Keysound tracks can use the instruments found (Map › Keysounds & VST3…).</p>
  {#if vst3Error}
    <p class="pref-error">{vst3Error}</p>
  {:else if vst3Report}
    <p class="pref-caption">{scanSummary(vst3Report)}</p>
    {#if vst3Report.blocked?.length}
      <p class="pref-caption">
        {vst3Report.blocked.length} {vst3Report.blocked.length === 1 ? "plug-in" : "plug-ins"} skipped after an earlier scan crash or timeout.
        {#if onretryblocked}
          <button type="button" disabled={vst3Scanning} onclick={onretryblocked}>Retry blocked plug-ins</button>
        {/if}
      </p>
    {/if}
    {#if vst3Report.plugins?.length}
      <ul class="pref-plugin-list">
        {#each vst3Report.plugins as plugin (plugin.uid ?? plugin.name)}
          <li><span>{plugin.name}</span><span class="pref-hint">{plugin.vendor}{plugin.is_instrument ? " · Instrument" : plugin.is_effect ? " · Effect" : ""}</span></li>
        {/each}
      </ul>
    {/if}
  {/if}
  <label class="pref-check">
    <input type="checkbox" checked={drumVst3.enabled} onchange={event => toggleDrumVst3(event.currentTarget.checked)} />
    <span>Use VST3 sounds for drum map</span>
  </label>
  <p class="pref-caption">
    Drum map rows play through {drumVst3.pluginName || "the instrument picked in Keysounds mode or Drum Map Setup"}.
  </p>
</div>

{#if onpicksoundfont}
  <hr class="pref-divider" />

  <div class="pref-group">
    <div class="pref-row-head">
      <span class="pref-label">SoundFont / SFZ sampler</span>
      <button type="button" disabled={soundfontLoading} onclick={onpicksoundfont}>
        {soundfontLoading ? "Loading…" : "Test .sf2 / .sfz…"}
      </button>
    </div>
    <p class="pref-caption">Plays .sf2 and .sfz instruments directly without a VST3 plug-in. Attach one to any track in Map › Keysounds & VST3….</p>
    {#if soundfontError}
      <p class="pref-error">{soundfontError}</p>
    {:else if soundfontSummary}
      <p class="pref-caption">
        {soundfontSummary.name} ({soundfontSummary.format.toUpperCase()}) · {soundfontSummary.regionCount} region{soundfontSummary.regionCount === 1 ? "" : "s"}
        {#if soundfontSummary.presets?.length > 1} · {soundfontSummary.presets.length} presets{/if}
      </p>
      {#if onauditionsoundfont}
        <button type="button" onclick={() => onauditionsoundfont(soundfontSummary.path)}>Preview C4</button>
      {/if}
    {/if}
  </div>
{/if}
