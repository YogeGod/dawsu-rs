<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { hasPriorityStatus, hintsFor } from "../lib/hints.js";
  import {
    audioLabel, cannotSaveLabel, hitLaneCounts, PLAYBACK_SPEEDS, problemTooltip, selectionLabel, splitFileName,
    statusText, ZOOM_MAX, ZOOM_MIN, ZOOM_STEP, zoomSliderValue,
  } from "../lib/statusbar.js";

  const view = $derived(bridge.view);
  const scene = $derived(bridge.scene);
  const hasScene = $derived(!!scene);
  const docDisabled = $derived(bridge.disabled);
  const selected = $derived(scene?.selection?.count ?? 0);
  const problems = $derived(view?.outputProblems ?? []);
  const cannotSave = $derived(cannotSaveLabel(problems.length));
  const problemTip = $derived(problemTooltip(problems));
  const status = $derived(statusText(bridge.statusNotice, view));
  const hints = $derived(hasScene && !hasPriorityStatus(bridge.statusNotice, view)
    ? hintsFor(bridge.hint, view?.tool, view?.keyPreset)
    : []);
  const showHints = $derived(hints.length > 0);
  const path = $derived(scene?.path ?? "");
  const fileParts = $derived(splitFileName(path));
  const counts = $derived(hitLaneCounts(scene));
  const zoomValue = $derived(zoomSliderValue(bridge.zoom ?? scene?.zoom));
  const audio = $derived(audioLabel(view?.outputDevice));
  const send = (...args) => bridge.callAction("statusBarAction", ...args);
</script>

<div id="playback-rates" class="playback-rates" role="group" aria-label="Playback speed"
  title={view?.keepPitch !== false ? "Playback speed (pitch preserved)" : "Playback speed (pitch follows speed)"} hidden={!hasScene}>
  {#each PLAYBACK_SPEEDS as speed (speed)}
    <button type="button" aria-label={"Playback speed " + Math.round(speed * 100) + "%"}
      aria-pressed={view?.speed === speed} disabled={docDisabled}
      onclick={() => send("speed", speed)}>{Math.round(speed * 100)}%</button>
  {/each}
  <button id="keep-pitch-toggle" type="button" class="pitch-toggle"
    aria-label="Keep song pitch when playback speed changes"
    aria-pressed={view?.keepPitch !== false}
    title={view?.keepPitch !== false ? "Keep pitch: On (click to pitch-shift with speed)" : "Keep pitch: Off (click to preserve song pitch)"}
    disabled={docDisabled}
    onclick={() => send("keepPitch", view?.keepPitch === false)}>Pitch</button>
</div>
<span id="play-state" class="play-state" data-playing={String(!!view?.playing)} hidden={!hasScene}>
  {view?.playing ? "Playing" : "Stopped"}
</span>
<span id="selection-count" class="selection-count" hidden={!selected}>{selectionLabel(selected)}</span>
<span id="status" role="status" aria-live="polite" title={status} hidden={showHints}>{status}</span>
<span id="a11y-live" class="sr-only" role="status" aria-live="polite" aria-atomic="true">
  {status}{selected ? ` · ${selectionLabel(selected)}` : ""}
</span>
{#if showHints}
  <span id="hint-bar" class="status-hints" aria-hidden="true">
    {#each hints as item (item.keys + ":" + item.action)}
      <span class="status-hint"><kbd>{item.keys}</kbd><span>{item.action}</span></span>
    {/each}
  </span>
{/if}
<span id="external-osu-notice" class="external-osu-notice" role="alert" hidden={!view?.externalOsuChanged}>
  <span class="external-osu-label">.osu changed on disk</span>
  <button id="reload-from-disk" type="button" disabled={!!view?.busy}
    onclick={() => send("reloadFromDisk")}>Reload from disk</button>
  <button id="keep-my-edits" type="button"
    onclick={() => send("dismissExternalOsuChange")}>Keep my edits</button>
</span>
<span id="problem-summary" class="problem-summary" title={problemTip} hidden={!problems.length}>
  <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 2.5 14.5 13.5h-13zM8 6.5v3M8 11.5v.01"/></svg>
  <span id="problem-summary-text">{cannotSave}</span>
</span>
<button id="next-output-problem" type="button" hidden={!problems.length} disabled={!!view?.busy}
  onclick={() => send("nextOutputProblem")}>Next problem</button>
<button id="warnings" type="button" hidden={!(view?.warnings?.length)}
  onclick={() => send("warnings")}>Audio warnings</button>
<button id="cancel-job" type="button" hidden={!view?.busy}
  onclick={() => send("cancelJob")}>Cancel job</button>
<div class="status-right">
  <span id="file-name" class="status-file" title={path} hidden>
    <span id="file-name-head">{fileParts.head}</span><span id="file-name-tail">{fileParts.tail}</span>
  </span>
  <span id="hit-lane-counts" class="status-counts" hidden={!hasScene}>{counts}</span>
  <div id="status-view" class="status-view" role="group" aria-label="Timeline zoom" hidden={!hasScene}>
    <button id="step-pads-toggle" type="button" aria-pressed={Boolean(view?.stepPadsMode)}
      title="Show compact beat step pads on lane rows"
      disabled={docDisabled}
      onclick={() => send("stepPadsMode", !view?.stepPadsMode)}>Pads</button>
    <button id="fit" type="button" title="Fit the difficulty in the timeline" disabled={docDisabled}
      onclick={() => send("fit")}>Fit</button>
    <input type="range" min={ZOOM_MIN} max={ZOOM_MAX} step={ZOOM_STEP} id="zoom" aria-label="Timeline zoom"
      title="Timeline zoom" disabled={!hasScene} value={zoomValue}
      oninput={event => send("zoom", Number(event.currentTarget.value), true)}
      onchange={event => send("zoom", Number(event.currentTarget.value), false)}>
  </div>
  <button id="audio" type="button" class="status-device" aria-label={audio.ariaLabel} title={audio.ariaLabel}
    onclick={() => send("audio")}>{audio.text}</button>
</div>

<style>
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  .pitch-toggle {
    margin-left: 2px;
  }
</style>
