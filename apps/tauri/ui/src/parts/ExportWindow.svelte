<script>
  import { onDestroy, onMount } from "svelte";
  import { invoke } from "../lib/ipc.js";
  import { onlyModalOpen } from "../lib/registry.js";
  import { exportModalOnTop, forwardSpace } from "../lib/space.js";
  import {
    adoptChoices, afterPoll, baseByteIdentical, baseDiff, baseLineEndingsOnly, baseStatus,
    baseVerification, blockDrops, choicesFrom, clampFallbackVolume, closeExport, difficultyName,
    divisorPresetFrom, exportDiffSummary, exportLabel, exportState, exportSummary,
    exportVerificationError, fileName, laneSwatches, matchWithinValid, minBeatsValid,
    openWarnings, optionsKey, ownWriteError, pollDelay, previewRequest, statusLine,
    verificationSummary, warningsLabel,
  } from "../lib/export.js";
  import { reviewMixSections, reviewSampleWarnings } from "../lib/review.js";
  import ExportOptionsForm from "./export/ExportOptionsForm.svelte";
  import ExportTargetsList from "./export/ExportTargetsList.svelte";
  import ExportReviewPane from "./export/ExportReviewPane.svelte";

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
  const baseVerify = $derived(choices.base ? baseVerification(status.preview) : null);
  const baseIdentical = $derived(choices.base ? baseByteIdentical(status.preview) : false);
  const baseLineEndings = $derived(choices.base ? baseLineEndingsOnly(status.preview) : false);
  // Which .wav files the export creates or reuses, and how volumes are written.
  const mixInfo = $derived(reviewMixSections(status.preview));
  const sampleWarnings = $derived(reviewSampleWarnings(status.preview));
  const summary = $derived(exportSummary(status.preview, view?.targets, choices));
  const diffSummary = $derived(exportDiffSummary(status.preview, view?.targets, choices));
  const verification = $derived(verificationSummary(status.preview));
  const verificationError = $derived(exportVerificationError(status.preview, view?.targets, choices));
  const line = $derived(statusLine({
    view, pollError, actionError, writeError: ownWriteError(lastRun, view),
    verificationError, stale: status.stale, running: !!run, problems,
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
      if (after.close) return close(true);
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

  // Cancel, Escape and the Export modal's backdrop all end here, once. wrote: this window's own write finished.
  export async function close(wrote = false) {
    if (closed) return;
    closed = true;
    stopListen();
    clearTimeout(pollTimer);
    try {
      await closeExport({
        flush: () => (previewTimer !== null ? preview() : undefined),
        invoke,
        onclose: onclose && (() => onclose({ wroteHitsoundOnly: wrote && choices.writeMode === "hitsound_only" })),
      });
    } catch (error) {
      closed = false;
      actionError = String(error);
      schedulePoll();
    }
  }

  // Embedded in the editor, Space goes to the timeline only while the Export modal is on top.
  function onWindowKeydown(event) {
    if (!onclose || exportModalOnTop(onlyModalOpen)) forwardSpace(event);
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

<svelte:window onkeydown={onWindowKeydown} />

<div class="export">
  <div class="routing">
    {#if !view}
      <p class="empty">Loading…</p>
    {:else if !source}
      <p class="empty">Open a difficulty in the editor first.</p>
    {:else}
      <h1>Copy {source.difficulty || difficultyName(source.path)} into the mapset</h1>

      <ExportOptionsForm
        {source}
        {swatches}
        bind:choices
        {matchValid}
        {muteBeatsValid}
        {divisorPreset}
        bind:showAdvanced
        onsetfallbackset={setFallbackSet}
        onsetfallbackvolume={setFallbackVolume}
        onpickdir={pickHitsoundOnlyDir}
      />

      <ExportTargetsList
        {view}
        bind:choices
        {status}
        {baseText}
        {baseOsuDiff}
        baseVerification={baseVerify}
        baseByteIdentical={baseIdentical}
        baseLineEndingsOnly={baseLineEndings}
        {listeningTarget}
        {dryRunFiles}
        {dryRunBusy}
        ontoggle={toggle}
        ontogglelisten={toggleListenTarget}
        onrundryrun={runUnusedDryRun}
      />
    {/if}
  </div>

  <ExportReviewPane {choices} {mixInfo} {sampleWarnings} {summary} {diffSummary} {verification} />

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
      {#if !onclose}
        <button type="button" class="link" onclick={showInEditor}>Show in editor</button>
      {/if}
    </div>
    {#if warnings.length}
      <ul class="warning-list" id="export-warnings" aria-label="Warnings">
        {#each warnings as message, index (index)}<li>{message}</li>{/each}
      </ul>
    {/if}
    <button type="button" class="secondary" onclick={() => close()}>Cancel</button>
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
    grid-template-columns: minmax(0, 1fr) 390px;
    grid-template-rows: minmax(0, 1fr) 64px;
    width: 100%;
    height: 100%;
    flex: 1 1 auto;
    min-height: 0;
  }
  :global(body.export-page) .export {
    height: 100vh;
  }
  .routing { padding: 24px 36px; overflow: auto; min-width: 0; }
  h1 { margin: 0 0 16px; font-size: 22px; font-weight: 700; letter-spacing: -0.02em; }
  .empty { color: var(--editor-muted); }

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
