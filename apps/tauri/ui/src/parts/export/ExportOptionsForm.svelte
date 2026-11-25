<script>
  import {
    difficultyName, divisorPresets, divisorsFromPreset,
  } from "../../lib/export.js";

  let {
    source,
    swatches = [],
    choices = $bindable(),
    matchValid = true,
    muteBeatsValid = true,
    divisorPreset,
    showAdvanced = $bindable(false),
    onsetfallbackset,
    onsetfallbackvolume,
    onpickdir,
  } = $props();
</script>

<section class="source" aria-label="Source">
  <div class="who">
    <strong>{source.difficulty || difficultyName(source.path)}</strong> <span class="role">Source</span>
    <div class="meta">{source.lanes.length} {source.lanes.length === 1 ? "lane" : "lanes"} · {source.hits} {source.hits === 1 ? "hit" : "hits"}</div>
  </div>
  {#if swatches.length}
    <div class="palette" aria-hidden="true">
      {#each swatches as colour, index (index)}<span style:--swatch={colour}></span>{/each}
    </div>
  {/if}
  <label class="match">
    Match within
    <span class="field" class:invalid={!matchValid}>
      <input type="number" min="0" max="1000" step="any" bind:value={choices.matchWithin}
        aria-invalid={!matchValid}>
      ms
    </span>
  </label>
  <span class="divider" aria-hidden="true"></span>
  <label class="switch-label">
    Copy volumes
    <button type="button" class="switch" role="switch" aria-checked={choices.copyVolumes}
      aria-label="Copy volumes"
      onclick={() => (choices.copyVolumes = !choices.copyVolumes)}></button>
  </label>
  <span class="divider" aria-hidden="true"></span>
  <label class="switch-label" title="Off: combine all source hits in the match window. On: the later moment replaces earlier ones.">
    Later moment wins
    <button type="button" class="switch" role="switch" aria-checked={choices.laterWins}
      aria-label="Later moment wins"
      onclick={() => (choices.laterWins = !choices.laterWins)}></button>
  </label>
  <span class="divider" aria-hidden="true"></span>
  <label class="format">
    Sample format
    <select class="field select" aria-label="Sample format" bind:value={choices.mixFormat}>
      <option value="wav16">16-bit WAV</option>
      <option value="wav32f">32-bit float WAV</option>
      <option value="ogg_vorbis">Ogg Vorbis</option>
    </select>
  </label>
  <span class="divider" aria-hidden="true"></span>
  <label class="switch-label" title="Trim leading silence and pad samples shorter than 25 ms to meet osu! Ranking Criteria.">
    Auto-fix samples
    <button type="button" class="switch" role="switch" aria-checked={choices.autoFixStagedMixes}
      aria-label="Auto-fix ranking criteria sample issues"
      onclick={() => (choices.autoFixStagedMixes = !choices.autoFixStagedMixes)}></button>
  </label>
  <span class="divider" aria-hidden="true"></span>
  <label class="format" title="Write straight into the mapset (with backups) or into a separate hitsound-only folder.">
    Write to
    <select class="field select" aria-label="Export destination" bind:value={choices.writeMode}>
      <option value="direct">Mapset (with backups)</option>
      <option value="hitsound_only">Hitsound-only folder</option>
    </select>
  </label>
  {#if choices.writeMode === "hitsound_only"}
    <button type="button" class="adv-btn" onclick={onpickdir}
      title={choices.hitsoundOnlyDir || "Default: dawsu! exports folder"}>
      {choices.hitsoundOnlyDir ? "Change folder…" : "Choose folder…"}
    </button>
  {/if}
  <span class="divider" aria-hidden="true"></span>
  <label class="switch-label" title="Set un-hit slider ends to 5% volume. Ends with a matched hitsound are never muted.">
    Mute un-hit slider ends
    <button type="button" class="switch" role="switch" aria-checked={choices.sliderEndMute.enabled}
      aria-label="Mute un-hit slider ends"
      onclick={() => (choices.sliderEndMute.enabled = !choices.sliderEndMute.enabled)}></button>
  </label>
  <span class="divider" aria-hidden="true"></span>
  <label class="format" title="Optional hitsound for target objects that did not match any source hit.">
    Fallback sound
    <select
      class="field select"
      aria-label="Fallback sound for un-hit objects"
      value={choices.fallbackSound?.set ?? "off"}
      onchange={(event) => onsetfallbackset(event.currentTarget.value)}
    >
      <option value="off">Off</option>
      <option value="normal">Normal</option>
      <option value="soft">Soft</option>
      <option value="drum">Drum</option>
    </select>
  </label>
  {#if choices.fallbackSound}
    <label class="format">
      <span class="field">
        <input
          type="number"
          min="5"
          max="100"
          step="1"
          aria-label="Fallback sound volume"
          value={choices.fallbackSound.volume}
          oninput={(event) => onsetfallbackvolume(event.currentTarget.valueAsNumber)}
        >
        %
      </span>
    </label>
  {/if}
  <span class="divider" aria-hidden="true"></span>
  <button type="button" class="adv-btn" aria-expanded={showAdvanced}
    onclick={() => (showAdvanced = !showAdvanced)}>
    {showAdvanced ? "Hide advanced" : "Advanced…"}
  </button>
</section>

{#if showAdvanced}
  <section class="mute-options" aria-label="Advanced export rules">
    <label class="mute-field" title="How unchanged custom sample files are copied alongside mixed samples.">
      Single samples
      <select class="select" aria-label="Single sample format" bind:value={choices.singleFormat}>
        <option value="source">Keep original file</option>
        <option value="match_mix_format">Match mix format</option>
      </select>
    </label>
    <label class="mute-field" title="Which hitnormal plays when a moment only has whistle, finish, or clap.">
      Addition-only hitnormal
      <select class="select" aria-label="Addition-only hitnormal policy" bind:value={choices.additionOnlyNormal}>
        <option value="difficulty">Difficulty set</option>
        <option value="additions_set">Addition set</option>
        <option value="skin_normal">Skin Normal</option>
        <option value="skin_soft">Skin Soft</option>
        <option value="skin_drum">Skin Drum</option>
        <option value="silent">Silent (44 B WAV)</option>
      </select>
    </label>
    <label class="mute-field" title="Write short green lines with custom slidertick indices on matched slider ticks.">
      Slider ticks
      <select class="select" aria-label="Slider tick rendering" bind:value={choices.tickPolicy}>
        <option value="ignore">Ignore</option>
        <option value="render">Render green lines (+5 ms revert)</option>
      </select>
    </label>
    <label class="check" title="Apply the mapset's or difficulty's green-line-only master volume curve on export.">
      <input type="checkbox" bind:checked={choices.applyVolumeEnvelope}>
      Apply volume curve
    </label>
    <label class="check" title="Allow allocating new custom sample indices when new combinations appear.">
      <input type="checkbox" bind:checked={choices.allowSchemaGrowth}>
      Allow schema growth
    </label>
    <label class="check" title="Automatically archive stale dawsu!-created sample mixes on repeat direct exports.">
      <input type="checkbox" bind:checked={choices.autoCleanupStale}>
      Auto-clean stale mixes
    </label>
    {#if choices.autoCleanupStale && choices.writeMode === "direct"}
      <label class="check" title="Also archive existing unused samples in the mapset (backed up first).">
        <input type="checkbox" bind:checked={choices.takeOverSamples}>
        Take over existing samples
      </label>
    {/if}
  </section>
{/if}

{#if choices.sliderEndMute.enabled}
  <section class="mute-options" aria-label="Slider-end muting options">
    <label class="mute-field">
      Min length
      <span class="field" class:invalid={!muteBeatsValid}>
        <input type="number" min="0" max="64" step="0.25" bind:value={choices.sliderEndMute.minBeats}
          aria-label="Minimum slider length in beats"
          aria-invalid={!muteBeatsValid}>
        beats
      </span>
    </label>
    <label class="mute-field">
      Beat snap
      <select class="select" aria-label="Mute on beat divisor" value={divisorPreset}
        onchange={event => (choices.sliderEndMute.divisors = divisorsFromPreset(event.currentTarget.value))}>
        {#each divisorPresets as preset (preset.id)}
          <option value={preset.id}>{preset.label}</option>
        {/each}
      </select>
    </label>
    <label class="check">
      <input type="checkbox" bind:checked={choices.sliderEndMute.muteSpinnerEnds}>
      Also mute spinner ends
    </label>
    <span class="mute-note">5% volume · keeps hitsounded ends</span>
  </section>
{/if}

<style>
  .role, .meta { color: var(--editor-muted); }
  .meta { margin-top: 2px; font-variant-numeric: tabular-nums; }

  .source {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px 18px;
    min-height: 76px;
    padding: 14px 18px;
    border-radius: 10px;
    background: rgba(90, 200, 250, 0.06);
    box-shadow: inset 0 0 0 1px rgba(90, 200, 250, 0.28), var(--shadow-card);
  }
  .source strong { font-size: 14px; font-weight: 600; }
  .who { margin-right: 8px; }
  .palette { display: flex; gap: 2px; margin-right: auto; }
  .palette span { width: 8px; height: 24px; border-radius: 2px; background: var(--swatch); }
  .match { margin-left: auto; display: flex; align-items: center; gap: 8px; white-space: nowrap; }
  .palette + .match { margin-left: 0; }
  .field {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 32px;
    padding: 0 10px;
    border-radius: 6px;
    background: var(--editor-ink);
    box-shadow: var(--shadow-well);
    color: var(--editor-muted);
  }
  .field.invalid { box-shadow: inset 0 0 0 1px var(--editor-error); }
  .field input {
    width: 44px;
    border: 0;
    background: transparent;
    color: var(--editor-text);
    font: inherit;
    font-variant-numeric: tabular-nums;
  }
  .field input:focus { outline: none; }
  .field:focus-within { box-shadow: inset 0 0 0 1px var(--editor-focus); }
  .divider { display: none; }
  .switch-label { display: flex; align-items: center; gap: 8px; white-space: nowrap; }
  .format { display: flex; align-items: center; gap: 8px; white-space: nowrap; }
  select.field {
    border: 0;
    color: var(--editor-text);
    font: inherit;
    padding: 0 8px;
    cursor: pointer;
  }
  select.field:focus { outline: none; box-shadow: inset 0 0 0 1px var(--editor-focus); }

  .mute-options {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 16px;
    margin-top: 8px;
    padding: 10px 18px;
    border-radius: 8px;
    background: var(--editor-panel);
    box-shadow: inset 0 0 0 1px var(--editor-border);
  }
  .mute-field { display: flex; align-items: center; gap: 8px; white-space: nowrap; color: var(--editor-secondary); }
  .select {
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: 6px;
    background: var(--editor-ink);
    box-shadow: var(--shadow-well);
    color: var(--editor-text);
    font: inherit;
  }
  .select:focus-visible { outline: 2px solid var(--editor-focus); outline-offset: 1px; }
  .mute-note { margin-left: auto; color: var(--editor-muted); font-size: 12px; white-space: nowrap; }
  .check { display: flex; align-items: center; gap: 8px; color: var(--editor-secondary); }
  .check input { margin: 0; accent-color: var(--editor-accent); }

  .switch {
    position: relative;
    width: 38px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 11px;
    background: var(--editor-selected);
    cursor: pointer;
  }
  .switch::after {
    content: "";
    position: absolute;
    top: 3px;
    left: 3px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--editor-muted);
    transition: transform .12s;
  }
  .switch[aria-checked="true"] { background: var(--editor-accent); }
  .switch[aria-checked="true"]::after { transform: translateX(16px); background: var(--editor-white); }
  .switch:focus-visible { outline: 2px solid var(--editor-focus); outline-offset: 2px; }

  .adv-btn {
    height: 26px;
    padding: 0 10px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-control);
    color: var(--editor-text);
    font-size: 12px;
    cursor: pointer;
  }
  .adv-btn:hover { border-color: var(--editor-accent); }
  button:focus-visible { outline: 2px solid var(--editor-focus); outline-offset: 2px; }
</style>
