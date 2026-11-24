<script>
  import { bridge } from "../lib/bridge.svelte.js";
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
  const path = $derived(scene?.path ?? "");
  const fileParts = $derived(splitFileName(path));
  const counts = $derived(hitLaneCounts(scene));
  const zoomValue = $derived(zoomSliderValue(bridge.zoom ?? scene?.zoom));
  const audio = $derived(audioLabel(view?.outputDevice));
  const send = (...args) => globalThis.statusBarAction?.(...args);
</script>

<div id="playback-rates" class="playback-rates" role="group" aria-label="Playback speed"
  title="Playback speed with pitch preserved" hidden={!hasScene}>
  {#each PLAYBACK_SPEEDS as speed (speed)}
    <button type="button" aria-label={"Playback speed " + Math.round(speed * 100) + "%"}
      aria-pressed={view?.speed === speed} disabled={docDisabled}
      onclick={() => send("speed", speed)}>{Math.round(speed * 100)}%</button>
  {/each}
</div>
<span id="play-state" class="play-state" data-playing={String(!!view?.playing)} hidden={!hasScene}>
  {view?.playing ? "Playing" : "Stopped"}
</span>
<span id="selection-count" class="selection-count" hidden={!selected}>{selectionLabel(selected)}</span>
<span id="status" role="status" title={status}>{status}</span>
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
