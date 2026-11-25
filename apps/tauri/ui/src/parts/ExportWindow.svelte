<script>
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "../lib/ipc.js";
  import { forwardSpace } from "../lib/space.js";
  import {
    adoptChoices, afterPoll, baseDiff, baseName, baseStatus, blockDrops, choicesFrom, clampFallbackVolume, difficultyName,
    divisorPresetFrom, divisorPresets, divisorsFromPreset, exportLabel, exportState, fileName,
    laneSwatches, matchWithinValid, minBeatsValid, openWarnings, optionsKey, ownWriteError, pollDelay,
    previewRequest, statusLine, targetCounts, targetDiff, warningsLabel,
  } from "../lib/export.js";
  import { reviewMixSections } from "../lib/review.js";

  // The editor's answer to export_view, replaced whole on every poll. A test may pass one in.
  // When embedded inside the editor via ExportModal, `onclose` closes the modal instead of an OS window.
  let { initialView = null, initialSent = null, onclose = null } = $props();
  let view = $state.raw(initialView);
  let choices = $state(choicesFrom(initialView));
  // The last preview this window asked for: { id, key }. null from the moment an option changes.
  let sent = $state.raw(initialSent);
  // The last export_view failed: cleared by the next answer.
  let pollError = $state(null);
  // A preview, Export, close or Show in editor the editor refused: cleared by the next one that works.
  let actionError = $state(null);
  // This window's Export press while its write runs: { id }, id null until export_run answers.
  let run = $state.raw(null);
  // The id of this window's last write, so only its own result shows.
  let lastRun = $state(null);
  // The footer's "N warnings", opened into the list of them.
  let showWarnings = $state(false);
  let showAdvanced = $state(false);
  let dryRunFiles = $state.raw(null);
  let dryRunBusy = $state(false);
  let listeningTarget = $state(null);
  let listenSawPlaying = $state(false);
  let adoptedFor = initialView?.source?.path;
  let pollTimer;
  let previewTimer = null;
  let closed = false;

  const status = $derived(exportState(view, sent, choices));
  const key = $derived(optionsKey(view, choices));
  const source = $derived(view?.source ?? null);
  const swatches = $derived(laneSwatches(source));
  const problems = $derived(status.preview ? view?.problems ?? [] : []);
  const matchValid = $derived(matchWithinValid(choices.matchWithin));
  const muteBeatsValid = $derived(!choices.sliderEndMute.enabled || minBeatsValid(choices.sliderEndMute.minBeats));
  const divisorPreset = $derived(divisorPresetFrom(choices.sliderEndMute.divisors));
  const baseText = $derived(baseStatus(status.preview));
  const baseOsuDiff = $derived(choices.base ? baseDiff(status.preview) : null);
  // Which .wav files the export creates or reuses, and how volumes are written.
  const mixInfo = $derived(reviewMixSections(status.preview));
  const line = $derived(statusLine({
    view, pollError, actionError, writeError: ownWriteError(lastRun, view),
    stale: status.stale, running: !!run, problems,
  }));
  const warnings = $derived(openWarnings(showWarnings, line, problems));

  function schedulePoll() {
    clearTimeout(pollTimer);
    if (!closed) pollTimer = setTimeout(poll, pollDelay(view));
  }

  async function poll() {
    if (closed) return;
    try {
      const next = await invoke("export_view");
      view = next;
      pollError = null;
      const adopted = adoptChoices(adoptedFor, next);
      adoptedFor = adopted.adoptedFor;
      if (adopted.choices) choices = adopted.choices;
      const after = afterPoll(run, next);
      run = after.run;
      if (after.close) return close();
      if (after.preview) preview();
    } catch (error) {
      pollError = String(error);
    }
    schedulePoll();
  }

  // Sends the choices, which the editor keeps per map, and asks for their preview. With every
  // difficulty left out the editor only keeps the choice.
  async function preview() {
    clearTimeout(previewTimer);
    previewTimer = null;
    if (!view?.source || !matchValid || !muteBeatsValid) return;
    const asked = key;
    try {
      const id = await invoke("export_preview", previewRequest(view, choices));
      if (asked === key) sent = { id, key: asked };
      actionError = null;
    } catch (error) {
      actionError = String(error);
    }
  }

  // Every change of map, difficulties or options asks for a new preview a moment later.
  $effect(() => {
    key;
    sent = null;
    clearTimeout(previewTimer);
    previewTimer = setTimeout(preview, 300);
  });

  function toggle(path) {
    const name = fileName(path);
    choices.leftOut = choices.leftOut.includes(name)
      ? choices.leftOut.filter(item => item !== name)
      : [...choices.leftOut, name];
  }

  function setFallbackSet(value) {
    if (value === "off") {
      choices.fallbackSound = null;
      return;
    }
    const volume = choices.fallbackSound?.volume ?? 80;
    choices.fallbackSound = { set: value, sound: "normal", volume };
  }

  function setFallbackVolume(value) {
    if (!choices.fallbackSound || !Number.isFinite(value)) return;
    choices.fallbackSound = { ...choices.fallbackSound, volume: clampFallbackVolume(value) };
  }

  async function exportNow() {
    if (!status.canExport || run) return;
    run = { id: null };
    try {
      const id = await invoke("export_run", { previewId: sent.id });
      run = { id };
      lastRun = id;
      actionError = null;
    } catch (error) {
      run = null;
      actionError = String(error);
    }
  }

  async function pickHitsoundOnlyDir() {
    try {
      const picked = await invoke("export_pick_hitsound_only_dir");
      if (picked) choices.hitsoundOnlyDir = picked;
    } catch (error) {
      actionError = String(error);
    }
  }

  async function runUnusedDryRun() {
    if (dryRunBusy) return;
    dryRunBusy = true;
    try {
      dryRunFiles = await invoke("export_unused_dry_run");
      actionError = null;
    } catch (error) {
      actionError = String(error);
    } finally {
      dryRunBusy = false;
    }
  }

  function stopListen() {
    if (!listeningTarget) return;
    listeningTarget = null;
    listenSawPlaying = false;
    invoke("export_stop_listen").catch(() => {});
  }

  async function toggleListenTarget(path) {
    if (listeningTarget === path) {
      stopListen();
      return;
    }
    listeningTarget = path;
    listenSawPlaying = false;
    try {
      await invoke("export_listen_target", { targetPath: path });
    } catch (error) {
      listeningTarget = null;
      listenSawPlaying = false;
      actionError = String(error);
    }
  }

  $effect(() => {
    // If the user untoggles the currently listening target, stop preview playback cleanly.
    if (listeningTarget && choices.leftOut.includes(fileName(listeningTarget))) {
      stopListen();
    }
  });

  $effect(() => {
    if (!listeningTarget || !view) return;
    if (view.playing) {
      listenSawPlaying = true;
    } else if (listenSawPlaying) {
      listeningTarget = null;
      listenSawPlaying = false;
    }
  });

  async function close() {
    closed = true;
    stopListen();
    clearTimeout(pollTimer);
    // A choice still waiting for its preview is sent first, so the editor keeps it with the map.
    if (previewTimer !== null) await preview();
    if (onclose) {
      onclose();
      return;
    }
    try {
      await invoke("close_export_window");
    } catch (error) {
      closed = false;
      actionError = String(error);
      schedulePoll();
    }
  }

  function showInEditor() {
    invoke("show_in_editor").then(() => (actionError = null), error => (actionError = String(error)));
  }

  let release;
  onMount(() => {
    release = blockDrops(window);
    poll();
  });
  onDestroy(() => {
    closed = true;
    stopListen();
    clearTimeout(pollTimer);
    clearTimeout(previewTimer);
    release?.();
  });
</script>

<svelte:window onkeydown={forwardSpace} />

<div class="export">
  <div class="routing">
    {#if !view}
      <p class="empty">Loading…</p>
    {:else if !source}
      <p class="empty">Open a difficulty in the editor first.</p>
    {:else}
      <h1>Copy {source.difficulty || difficultyName(source.path)} into the mapset</h1>

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
          <button type="button" class="adv-btn" onclick={pickHitsoundOnlyDir}
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
            onchange={(event) => setFallbackSet(event.currentTarget.value)}
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
                oninput={(event) => setFallbackVolume(event.currentTarget.valueAsNumber)}
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

      <ul class="targets">
        <li class="target" class:off={!choices.base}>
          <span class="port" aria-hidden="true"></span>
          <div class="card">
            <div class="line">
              <span class="pill">{baseName}</span>
              <span class="role">Base difficulty</span>
              <span class="spacer"></span>
              <button type="button" class="switch" role="switch" aria-checked={choices.base}
                aria-label="Include the base hitsound difficulty"
                onclick={() => (choices.base = !choices.base)}></button>
            </div>
            <div class="counts">
              {#if !choices.base}
                Left out
              {:else if baseText}
                <span>{baseText}</span>
              {/if}
              <span class="spacer"></span>
              <label class="check" class:off={!choices.base}>
                <input type="checkbox" bind:checked={choices.sliderAudio} disabled={!choices.base}>
                Slider slide, whistle and tick audio
              </label>
            </div>
            {#if baseOsuDiff}
              <details class="osu-diff">
                <summary class="osu-diff-toggle">
                  Compare <code>.osu</code>
                  (<span class="osu-diff-add">+{baseOsuDiff.added}</span> / <span class="osu-diff-rem">-{baseOsuDiff.removed}</span> lines)
                </summary>
                <div class="osu-diff-body">
                  {#each baseOsuDiff.groups as group, gi (gi)}
                    {#if group.section}<div class="osu-diff-section">{group.section}</div>{/if}
                    {#each group.entries as entry, ei (ei)}
                      <div class="osu-diff-line {entry.kind}">
                        <span class="osu-diff-ln">{entry.line}</span>
                        <span class="osu-diff-sign">{entry.kind === "added" ? "+" : "-"}</span>
                        <span class="osu-diff-text">{entry.text}</span>
                      </div>
                    {/each}
                  {/each}
                  {#if baseOsuDiff.truncated > 0}
                    <div class="osu-diff-more">… {baseOsuDiff.truncated} more changed {baseOsuDiff.truncated === 1 ? "line" : "lines"}</div>
                  {/if}
                </div>
              </details>
            {/if}
          </div>
        </li>
          {#each view.targets ?? [] as path (path)}
            {@const on = !choices.leftOut.includes(fileName(path))}
            {@const counts = on ? targetCounts(status.preview, path) : null}
            {@const diff = on ? targetDiff(status.preview, path) : null}
            <li class="target" class:off={!on}>
              <span class="port" aria-hidden="true"></span>
              <div class="card">
                <div class="line">
                  <span class="pill" title={fileName(path)}>{difficultyName(path)}</span>
                  <span class="role">Copy target</span>
                  <span class="spacer"></span>
                  {#if on}
                    <button type="button" class="listen-btn" aria-pressed={listeningTarget === path}
                      title={listeningTarget === path ? "Stop preview" : "Preview this target difficulty"}
                      onclick={() => toggleListenTarget(path)}>
                      {listeningTarget === path ? "■ Stop" : "▶ Listen"}
                    </button>
                  {/if}
                  {#if counts}<span class="changed">{counts.changed} changed</span>{/if}
                  <button type="button" class="switch" role="switch" aria-checked={on}
                    aria-label={"Include " + difficultyName(path)} onclick={() => toggle(path)}></button>
                </div>
                <div class="counts">
                  {#if !on}
                    Left out · keeps its own hitsounds
                  {:else if counts}
                    {#each counts.items as item (item.tone)}
                      <span class="count {item.tone}">{item.text}</span>
                    {/each}
                  {/if}
                </div>
                {#if diff}
                  <details class="osu-diff">
                    <summary class="osu-diff-toggle">
                      Compare <code>.osu</code>
                      (<span class="osu-diff-add">+{diff.added}</span> / <span class="osu-diff-rem">-{diff.removed}</span> lines)
                    </summary>
                    <div class="osu-diff-body">
                      {#each diff.groups as group, gi (gi)}
                        {#if group.section}<div class="osu-diff-section">{group.section}</div>{/if}
                        {#each group.entries as entry, ei (ei)}
                          <div class="osu-diff-line {entry.kind}">
                            <span class="osu-diff-ln">{entry.line}</span>
                            <span class="osu-diff-sign">{entry.kind === "added" ? "+" : "-"}</span>
                            <span class="osu-diff-text">{entry.text}</span>
                          </div>
                        {/each}
                      {/each}
                      {#if diff.truncated > 0}
                        <div class="osu-diff-more">… {diff.truncated} more changed {diff.truncated === 1 ? "line" : "lines"}</div>
                      {/if}
                    </div>
                  </details>
                {/if}
              </div>
            </li>
          {/each}
      </ul>
      {#if !view.targets?.length}
        <p class="empty">This mapset has no other difficulties to copy into.</p>
      {/if}
      <details class="dry-run-box">
        <summary onclick={() => { if (dryRunFiles === null) runUnusedDryRun(); }}>
          Unused files dry-run
          {#if dryRunFiles !== null}
            ({dryRunFiles.length} removable)
          {/if}
        </summary>
        <div class="dry-run-body">
          <button type="button" class="listen-btn" disabled={dryRunBusy} onclick={runUnusedDryRun}>
            {dryRunBusy ? "Checking…" : "Refresh dry-run"}
          </button>
          {#if dryRunFiles && dryRunFiles.length === 0}
            <p class="dry-run-empty">No unused dawsu! sample files found.</p>
          {:else if dryRunFiles}
            <ul class="dry-run-list">
              {#each dryRunFiles as item (item.name)}
                <li>
                  <span>{item.name}</span>
                  <small>{Math.max(1, Math.round(item.sizeBytes / 1024))} KB · {item.reason}</small>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      </details>
    {/if}
  </div>

  <aside class="panel">
    <h2>Write to</h2>
    <div class="write-card">
      {#if choices.writeMode === "hitsound_only"}
        <strong>Hitsound-only folder</strong>
        <p>Writes a separate folder ({choices.hitsoundOnlyDir || "dawsu! exports folder"}) with the song, samples, and hitsound <code>.osu</code> files. Your mapset folder stays untouched.</p>
      {:else}
        <strong>Into the mapset</strong>
        <p>Updates every included difficulty in place. Each changed file is backed up first.</p>
      {/if}
    </div>
    {#if mixInfo.sections.length || mixInfo.volumes.length}
      <div class="write-details" aria-label="Sample files and volumes">
        {#each mixInfo.sections as section (section.title)}
          <strong>{section.title}</strong>
          <ul>{#each section.rows as row (row.path)}<li title={row.path}>{fileName(row.path)}</li>{/each}</ul>
        {/each}
        {#if mixInfo.volumes.length}
          <strong>Volumes</strong>
          {#each mixInfo.volumes as message, index (index)}<p>{message}</p>{/each}
        {/if}
      </div>
    {/if}
  </aside>

  <footer class="actions">
    <div class="status" role="status">
      {#if line}
        {#if line.count}
          <button type="button" class="warnings" aria-expanded={showWarnings} aria-controls="export-warnings"
            onclick={() => (showWarnings = !showWarnings)}>{warningsLabel(line.count)}</button>
        {/if}
        <span class="first" class:error={line.tone === "error"} role={line.tone === "error" ? "alert" : null}>{line.text}</span>
        {#if line.again}
          <button type="button" class="link" onclick={preview}>{line.tone === "stale" ? "Refresh" : "Preview again"}</button>
        {/if}
      {/if}
      <button type="button" class="link" onclick={showInEditor}>Show in editor</button>
    </div>
    {#if warnings.length}
      <ul class="warning-list" id="export-warnings" aria-label="Warnings">
        {#each warnings as message, index (index)}<li>{message}</li>{/each}
      </ul>
    {/if}
    <button type="button" class="secondary" onclick={close}>Cancel</button>
    <button type="button" class="primary" disabled={!status.canExport || !!run} onclick={exportNow}>{exportLabel(status.count)}</button>
  </footer>
</div>

<style>
  /* export.html has no style.css: the page's own frame lives here, scoped to that body. */
  :global(body.export-page) {
    margin: 0;
    height: 100vh;
    overflow: hidden;
    background: var(--editor-canvas);
    color: var(--editor-text);
    font: 13px/1.4 var(--font-ui);
    color-scheme: dark;
  }
  :global(body.export-page *) { box-sizing: border-box; }

  .export {
    display: grid;
    grid-template-columns: 1fr 360px;
    grid-template-rows: 1fr 64px;
    height: 100vh;
  }
  .routing { padding: 28px 40px; overflow: auto; min-width: 0; }
  h1 { margin: 0 0 20px; font-size: 22px; font-weight: 600; }
  .empty { color: var(--editor-muted); }
  .role, .meta { color: var(--editor-muted); }
  .meta { margin-top: 2px; font-variant-numeric: tabular-nums; }

  .source {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 16px;
    min-height: 84px;
    padding: 12px 20px;
    border-radius: 10px;
    background: var(--editor-tealtint);
    box-shadow: inset 0 0 0 1px rgba(98, 214, 207, .35), var(--shadow-card);
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
  .divider { width: 1px; height: 24px; background: var(--editor-border); }
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

  .targets {
    list-style: none;
    margin: 0;
    padding: 10px 0 0 56px;
    position: relative;
  }
  .targets::before {
    content: "";
    position: absolute;
    left: 26px;
    top: 0;
    bottom: 47px;
    width: 2px;
    background: var(--editor-border);
  }
  .target { position: relative; margin-bottom: 10px; }
  .target::before {
    content: "";
    position: absolute;
    left: -30px;
    top: 50%;
    width: 30px;
    border-top: 2px solid var(--editor-accent);
  }
  .target.off::before { border-top: 2px dashed var(--editor-disabled); }
  .port {
    position: absolute;
    left: -35px;
    top: calc(50% - 4px);
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--editor-accent);
  }
  .off .port { background: var(--editor-disabled); }
  .card {
    padding: 12px 18px;
    border-radius: 10px;
    background: var(--editor-panel);
    box-shadow: var(--shadow-card);
  }
  .off .card { opacity: .55; }
  .line { display: flex; align-items: center; gap: 10px; }
  .spacer { flex: 1; }
  .pill {
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--editor-selected);
    color: var(--editor-text);
    font-weight: 600;
  }
  .changed { color: var(--editor-muted); font-variant-numeric: tabular-nums; }
  .counts {
    display: flex;
    gap: 16px;
    min-height: 20px;
    margin-top: 10px;
    color: var(--editor-muted);
    font-variant-numeric: tabular-nums;
  }
  .count { color: var(--editor-secondary); }
  .check { display: flex; align-items: center; gap: 8px; color: var(--editor-secondary); }
  .check input { margin: 0; accent-color: var(--editor-accent); }
  .check.off { color: var(--editor-disabled); }
  .count::before {
    content: "";
    display: inline-block;
    width: 6px;
    height: 6px;
    margin-right: 6px;
    border-radius: 50%;
    vertical-align: middle;
    background: var(--editor-disabled);
  }
  .count.matched::before { background: var(--editor-play); }
  .count.unmatched::before { background: var(--editor-error); }
  .count.replaced::before, .count.kept::before, .count.muted::before { background: var(--editor-warning); }

  .osu-diff {
    margin-top: 10px;
    border-top: 1px solid var(--editor-border);
    padding-top: 8px;
  }
  .osu-diff-toggle {
    cursor: pointer;
    color: var(--editor-secondary);
    font-size: 12px;
    user-select: none;
    list-style-position: outside;
  }
  .osu-diff-toggle code {
    font-family: var(--font-mono);
    font-size: 11px;
  }
  .osu-diff-add { color: var(--editor-play); font-variant-numeric: tabular-nums; }
  .osu-diff-rem { color: var(--editor-error); font-variant-numeric: tabular-nums; }
  .osu-diff-body {
    margin-top: 8px;
    max-height: 220px;
    overflow: auto;
    border-radius: 6px;
    background: var(--editor-ink);
    box-shadow: var(--shadow-well);
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.45;
  }
  .osu-diff-section {
    padding: 4px 10px 2px;
    color: var(--editor-accent);
    font-weight: 600;
    background: rgba(255, 255, 255, .03);
    border-top: 1px solid var(--editor-border);
  }
  .osu-diff-section:first-child { border-top: 0; }
  .osu-diff-line {
    display: grid;
    grid-template-columns: 44px 16px 1fr;
    padding: 1px 10px 1px 6px;
    white-space: pre;
  }
  .osu-diff-line.added {
    background: rgba(52, 199, 89, .10);
    color: var(--editor-text);
  }
  .osu-diff-line.removed {
    background: rgba(255, 69, 58, .10);
    color: var(--editor-text);
  }
  .osu-diff-ln {
    text-align: right;
    padding-right: 8px;
    color: var(--editor-disabled);
    font-variant-numeric: tabular-nums;
    user-select: none;
  }
  .osu-diff-sign {
    text-align: center;
    font-weight: 600;
    user-select: none;
  }
  .osu-diff-line.added .osu-diff-sign { color: var(--editor-play); }
  .osu-diff-line.removed .osu-diff-sign { color: var(--editor-error); }
  .osu-diff-text { min-width: 0; }
  .osu-diff-more {
    padding: 5px 10px;
    color: var(--editor-muted);
    border-top: 1px solid var(--editor-border);
    font-style: italic;
  }

  .panel {
    padding: 28px 24px;
    border-left: 1px solid var(--editor-border);
    background: var(--editor-panel);
    overflow: auto;
  }
  h2 { margin: 0 0 12px; font-size: 13px; font-weight: 600; color: var(--editor-secondary); }
  .write-card {
    padding: 14px 16px;
    border-radius: 8px;
    background: var(--editor-control);
    box-shadow: inset 0 0 0 1px var(--editor-accent);
  }
  .write-card p { margin: 4px 0 0; color: var(--editor-muted); }
  .write-details { margin-top: 12px; padding: 0 4px; }
  .write-details strong { display: block; margin-top: 10px; }
  .write-details ul { margin: 4px 0 0; padding-left: 18px; color: var(--editor-muted); }
  .write-details p { margin: 4px 0 0; color: var(--editor-muted); }

  .actions {
    position: relative;
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 24px 0 40px;
    border-top: 1px solid var(--editor-border);
    background: var(--editor-chrome);
  }
  .status { flex: 1; display: flex; align-items: center; gap: 12px; min-width: 0; color: var(--editor-secondary); }
  .status span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .status .first { min-width: 0; }
  .warnings {
    padding: 4px 10px;
    border: 0;
    border-radius: 6px;
    background: rgba(239, 193, 106, .12);
    color: var(--editor-warning);
    font: inherit;
    white-space: nowrap;
    cursor: pointer;
  }
  .warning-list {
    position: absolute;
    left: 40px;
    bottom: calc(100% + 8px);
    width: min(560px, calc(100% - 80px));
    max-height: 240px;
    margin: 0;
    padding: 8px 12px 8px 28px;
    overflow: auto;
    border-radius: 8px;
    background: var(--editor-panel);
    box-shadow: inset 0 0 0 1px var(--editor-border), var(--shadow-card);
    color: var(--editor-text);
  }
  .warning-list li + li { margin-top: 4px; }
  .error { color: var(--editor-error); }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--editor-accent);
    font: inherit;
    cursor: pointer;
    white-space: nowrap;
  }
  .secondary, .primary {
    height: 40px;
    padding: 0 18px;
    border: 0;
    border-radius: 8px;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    box-shadow: var(--shadow-raised);
  }
  .secondary { background: var(--editor-control); color: var(--editor-text); }
  .primary { background: var(--editor-accent); color: var(--editor-accentink); }
  .primary:disabled { background: var(--editor-selected); color: var(--editor-disabled); cursor: default; }
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
