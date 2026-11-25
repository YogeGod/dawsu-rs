<script>
  let {
    scales,
    scale,
    preset,
    view,
    busy,
    onapplyscale,
    onpickpreset,
    onpicksongs,
    onpicklibrary,
    onsendaction,
  } = $props();
</script>

<div class="pref-group">
  <div class="pref-row-head">
    <span class="pref-label">Interface scale</span>
    <span class="pref-hint">Ctrl + = / Ctrl + -, Ctrl + 0 resets</span>
  </div>
  <div class="pref-seg pref-seg-wide">
    {#each scales as pct (pct)}
      <button type="button" class:active={scale === pct} onclick={() => onapplyscale(pct)}>{pct}%</button>
    {/each}
  </div>
</div>

<hr class="pref-divider" />

<div class="pref-group">
  <span class="pref-label">Controls</span>
  <div class="pref-seg">
    <button type="button" class:active={preset === "osu!"} onclick={() => onpickpreset("osu!")}>osu! editor controls</button>
    <button type="button" class:active={preset !== "osu!"} onclick={() => onpickpreset("DAW")}>DAW-like keys</button>
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
    <button type="button" disabled={busy} onclick={onpicksongs}>Change…</button>
  </div>
</div>

<div class="pref-group">
  <span class="pref-label">Sample library folder</span>
  <div class="pref-path-row">
    <input type="text" readonly value={view.hitsoundLibrary || "No library folder selected"} />
    <button type="button" disabled={busy} onclick={onpicklibrary}>Add folder…</button>
  </div>
</div>

<hr class="pref-divider" />

<div class="pref-group">
  <span class="pref-label">Save & Export</span>
  <label class="pref-check">
    <input type="checkbox" checked={Boolean(view.cleanRedundantGreenlines)}
      onchange={(e) => onsendaction({ kind: "clean_redundant_greenlines", on: e.currentTarget.checked })} />
    <span>Clean redundant greenlines on Save & Export</span>
  </label>
  <p class="pref-caption">Removes extra green timing lines that don't change sample set, volume, slider velocity, or kiai. Off by default so untouched .osu lines stay identical.</p>
  <label class="pref-check">
    <input type="checkbox" checked={Boolean(view.reloadOsuAfterSave)}
      onchange={(e) => onsendaction({ kind: "reload_osu_after_save", on: e.currentTarget.checked })} />
    <span>Reload osu! editor after Save</span>
  </label>
  <p class="pref-caption">On Windows, brings osu!stable forward and sends Ctrl+L when the open difficulty matches. For osu!lazer external edits, reminds you to click Finish.</p>
</div>
