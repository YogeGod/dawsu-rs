<script>
  import { playbackStartChoices } from "../../editor/helpers.js";

  let {
    audioConfig,
    selectedBackend = $bindable(""),
    selectedDevice = $bindable(""),
    bufferMs = $bindable(""),
    audioError = "",
    audioLocked = false,
    view,
    onloadaudio,
    onapplyaudio,
    onsendaction,
  } = $props();
</script>

<div class="pref-group">
  <span class="pref-label">Output device</span>
  {#if audioConfig?.current}
    <p class="pref-caption">Active: {audioConfig.current}{audioConfig.effectiveBufferMs != null ? ` (${audioConfig.effectiveBufferMs} ms)` : ""}</p>
  {/if}
  <div class="pref-grid">
    <label class="pref-field">
      <span>Backend</span>
      <select bind:value={selectedBackend} disabled={audioLocked} onchange={() => onloadaudio(selectedBackend)}>
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
    <button type="button" disabled={audioLocked || Boolean(audioError)} onclick={onapplyaudio}>Apply output</button>
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
      <select value={view.playbackStart ?? "cursor"} onchange={(e) => onsendaction({ kind: "playback_start", value: e.currentTarget.value })}>
        {#each playbackStartChoices as [val, lbl] (val)}
          <option value={val}>{lbl}</option>
        {/each}
      </select>
    </label>
    <label class="pref-field">
      <span>Song offset (ms)</span>
      <input type="number" min="-60000" max="60000" value={view.offset ?? 0}
        onchange={(e) => onsendaction({ kind: "offset", ms: Number(e.currentTarget.value) || 0 })} />
    </label>
  </div>
  <label class="pref-check">
    <input type="checkbox" checked={Boolean(view.returnOnStop)}
      onchange={(e) => onsendaction({ kind: "return_on_stop", on: e.currentTarget.checked })} />
    <span>Return playhead to start position on Stop</span>
  </label>
  <label class="pref-check">
    <input type="checkbox" checked={view.keepPitch !== false}
      onchange={(e) => onsendaction({ kind: "keep_pitch", on: e.currentTarget.checked })} />
    <span>Keep song pitch when playback speed is slowed or sped up</span>
  </label>
</div>
