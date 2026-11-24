<script>
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "../lib/ipc.js";
  import { forwardSpace } from "../lib/space.js";
  import {
    adoptChoices, afterPoll, baseName, baseStatus, blockDrops, choicesFrom, difficultyName, exportLabel, exportState, fileName,
    laneSwatches, matchWithinValid, openWarnings, optionsKey, ownWriteError, pollDelay, previewRequest, statusLine, targetCounts,
    warningsLabel,
  } from "../lib/export.js";
  import { reviewMixSections } from "../lib/review.js";

  // The editor's answer to export_view, replaced whole on every poll. A test may pass one in.
  // When embedded inside the editor via ExportModal, `onclose` closes the modal instead of an OS window.
  let { initialView = null, onclose = null } = $props();
  let view = $state.raw(initialView);
  let choices = $state(choicesFrom(initialView));
  // The last preview this window asked for: { id, key }. null from the moment an option changes.
  let sent = $state.raw(null);
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
  const baseText = $derived(baseStatus(status.preview));
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
    if (!view?.source || !matchValid) return;
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

  async function close() {
    closed = true;
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
      </section>

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
          </div>
        </li>
          {#each view.targets ?? [] as path (path)}
            {@const on = !choices.leftOut.includes(fileName(path))}
            {@const counts = on ? targetCounts(status.preview, path) : null}
            <li class="target" class:off={!on}>
              <span class="port" aria-hidden="true"></span>
              <div class="card">
                <div class="line">
                  <span class="pill" title={fileName(path)}>{difficultyName(path)}</span>
                  <span class="role">Copy target</span>
                  <span class="spacer"></span>
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
              </div>
            </li>
          {/each}
      </ul>
      {#if !view.targets?.length}
        <p class="empty">This mapset has no other difficulties to copy into.</p>
      {/if}
    {/if}
  </div>

  <aside class="panel">
    <h2>Write to</h2>
    <div class="write-card">
      <strong>Into the mapset</strong>
      <p>Updates every included difficulty in place. Each changed file is backed up first.</p>
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
  button:focus-visible { outline: 2px solid var(--editor-focus); outline-offset: 2px; }
</style>
