<script>
  import {
    baseName, difficultyName, fileName, targetByteIdentical, targetCounts,
    targetCoverageBins, targetCoveragePct, targetDiff, targetLineEndingsOnly, targetVerification,
  } from "../../lib/export.js";
  import OsuDiffDrawer from "./OsuDiffDrawer.svelte";

  let {
    view,
    choices = $bindable(),
    status,
    baseText = "",
    baseOsuDiff = null,
    baseVerification = null,
    baseByteIdentical = false,
    baseLineEndingsOnly = false,
    listeningTarget = null,
    dryRunFiles = null,
    dryRunBusy = false,
    ontoggle,
    ontogglelisten,
    onrundryrun,
  } = $props();

  let openDiffs = $state({});

  function toggleDiff(key) {
    openDiffs = { ...openDiffs, [key]: !openDiffs[key] };
  }

  function setDiffOpen(key, next) {
    if (!!openDiffs[key] !== next) {
      openDiffs = { ...openDiffs, [key]: next };
    }
  }

  function failureText(report) {
    if (!report) return "";
    const failed = report.checks?.find(check => check.passed === false);
    const detail = report.failureMessage
      || (failed ? (failed.detail ? `${failed.label}: ${failed.detail}` : failed.label) : "Read-back check failed.");
    const ms = Number.isFinite(report.failureTimeMs) ? Math.round(report.failureTimeMs) : null;
    const hasTime = ms !== null && new RegExp(`\\b${ms}\\s*ms\\b`, "i").test(detail);
    return ms !== null && !hasTime ? `${ms} ms — ${detail}` : detail;
  }
</script>

<ul class="targets">
  <li class="target" class:off={!choices.base}>
    <span class="port" aria-hidden="true"></span>
    <div class="card">
      <div class="line">
        <span class="pill">{baseName}</span>
        <span class="role">Base difficulty</span>
        <span class="spacer"></span>
        {#if choices.base}
          {#if baseOsuDiff}
            <button type="button" class="diff-btn"
              aria-label="Toggle .osu diff for base difficulty"
              aria-expanded={!!openDiffs.__base}
              aria-controls="osu-diff-base"
              onclick={() => toggleDiff("__base")}>
              Diff <span class="osu-diff-add">+{baseOsuDiff.added}</span> / <span class="osu-diff-rem">-{baseOsuDiff.removed}</span>
            </button>
          {:else if baseByteIdentical}
            <span class="unchanged-pill">No .osu changes</span>
          {:else if baseLineEndingsOnly}
            <span class="unchanged-pill">Line endings only</span>
          {/if}
          {#if baseVerification}
            <span class="verify-badge" class:ok={baseVerification.passed} class:fail={!baseVerification.passed}>
              <span aria-hidden="true">{baseVerification.passed ? "✓" : "✕"}</span>
              {baseVerification.passed ? "Verified" : "Check failed"}
            </span>
          {/if}
        {/if}
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
      {#if choices.base && baseVerification && !baseVerification.passed}
        <div class="verify-alert" role="status">{failureText(baseVerification)}</div>
      {/if}
      {#if choices.base && baseOsuDiff}
        <OsuDiffDrawer
          diff={baseOsuDiff}
          open={!!openDiffs.__base}
          onopenchange={next => setDiffOpen("__base", next)}
          id="osu-diff-base"
        />
      {/if}
    </div>
  </li>
  {#each view.targets ?? [] as path, idx (path)}
    {@const on = !choices.leftOut.includes(fileName(path))}
    {@const counts = on ? targetCounts(status.preview, path) : null}
    {@const coveragePct = on ? targetCoveragePct(status.preview, path) : null}
    {@const coverageBins = on ? targetCoverageBins(status.preview, path) : []}
    {@const diff = on ? targetDiff(status.preview, path) : null}
    {@const verification = on ? targetVerification(status.preview, path) : null}
    {@const byteIdentical = on ? targetByteIdentical(status.preview, path) : false}
    {@const lineEndingsOnly = on ? targetLineEndingsOnly(status.preview, path) : false}
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
              onclick={() => ontogglelisten(path)}>
              {listeningTarget === path ? "■ Stop" : "▶ Listen"}
            </button>
            {#if diff}
              <button type="button" class="diff-btn"
                aria-label={`Toggle .osu diff for ${difficultyName(path)}`}
                aria-expanded={!!openDiffs[path]}
                aria-controls={`osu-diff-${idx}`}
                onclick={() => toggleDiff(path)}>
                Diff <span class="osu-diff-add">+{diff.added}</span> / <span class="osu-diff-rem">-{diff.removed}</span>
              </button>
            {:else if byteIdentical}
              <span class="unchanged-pill">No .osu changes</span>
            {:else if lineEndingsOnly}
              <span class="unchanged-pill">Line endings only</span>
            {/if}
            {#if verification}
              <span class="verify-badge" class:ok={verification.passed} class:fail={!verification.passed}>
                <span aria-hidden="true">{verification.passed ? "✓" : "✕"}</span>
                {verification.passed ? "Verified" : "Check failed"}
              </span>
            {/if}
          {/if}
          {#if counts}<span class="changed">{counts.changed} changed</span>{/if}
          <button type="button" class="switch" role="switch" aria-checked={on}
            aria-label={"Include " + difficultyName(path)} onclick={() => ontoggle(path)}></button>
        </div>
        <div class="counts">
          {#if !on}
            Left out · keeps its own hitsounds
          {:else if counts}
            {#each counts.items as item (item.tone)}
              <span class="count {item.tone}">{item.text}</span>
            {/each}
            {#if coveragePct !== null}
              <span class="coverage-pct">{coveragePct}% covered</span>
            {/if}
          {/if}
        </div>
        {#if on && coverageBins.length}
          <div class="coverage-bins" aria-hidden="true">
            {#each coverageBins as tone, bi (bi)}
              <span class="coverage-bin {tone}"></span>
            {/each}
          </div>
        {:else if on && coveragePct !== null}
          <div class="coverage-bar" aria-hidden="true">
            <span class="coverage-fill" style:width={`${coveragePct}%`}></span>
          </div>
        {/if}
        {#if on && verification && !verification.passed}
          <div class="verify-alert" role="status">{failureText(verification)}</div>
        {/if}
        {#if diff}
          <OsuDiffDrawer
            {diff}
            open={!!openDiffs[path]}
            onopenchange={next => setDiffOpen(path, next)}
            id={`osu-diff-${idx}`}
          />
        {/if}
      </div>
    </li>
  {/each}
</ul>
{#if !view.targets?.length}
  <p class="empty">This mapset has no other difficulties to copy into.</p>
{/if}
<details class="dry-run-box">
  <summary onclick={() => { if (dryRunFiles === null) onrundryrun(); }}>
    Unused files dry-run
    {#if dryRunFiles !== null}
      ({dryRunFiles.length} removable)
    {/if}
  </summary>
  <div class="dry-run-body">
    <button type="button" class="listen-btn" disabled={dryRunBusy} onclick={onrundryrun}>
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

<style>
  .empty { color: var(--editor-muted); }
  .role { color: var(--editor-muted); }

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
    margin: 14px 0 0;
    padding: 0;
    display: grid;
    gap: 10px;
  }
  .target { position: relative; }
  .port { display: none; }
  .card {
    padding: 14px 18px;
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    box-shadow: var(--shadow-card);
  }
  .off .card { opacity: .52; }
  .line { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 10px; }
  .spacer { flex: 1; min-width: 8px; }
  .pill {
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  .osu-diff-add { color: var(--editor-play); font-variant-numeric: tabular-nums; }
  .osu-diff-rem { color: var(--editor-error); font-variant-numeric: tabular-nums; }
  .diff-btn,
  .listen-btn {
    height: 26px;
    padding: 0 11px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-control);
    color: var(--editor-text);
    font: inherit;
    font-size: 11px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    white-space: nowrap;
  }
  .diff-btn:hover:not(:disabled),
  .listen-btn:hover:not(:disabled) {
    border-color: var(--editor-accent);
    background: var(--editor-hover);
  }
  .diff-btn[aria-expanded="true"],
  .listen-btn[aria-pressed="true"] {
    border-color: var(--editor-accent);
    background: var(--editor-accent);
    color: var(--editor-accentink);
  }
  .diff-btn[aria-expanded="true"] .osu-diff-add,
  .diff-btn[aria-expanded="true"] .osu-diff-rem {
    color: inherit;
  }
  .unchanged-pill,
  .verify-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }
  .unchanged-pill {
    background: var(--editor-chrome);
    border: 1px solid var(--editor-border);
    color: var(--editor-muted);
  }
  .verify-badge.ok {
    background: rgba(52, 199, 89, .14);
    border: 1px solid rgba(52, 199, 89, .38);
    color: var(--editor-play);
  }
  .verify-badge.fail {
    background: rgba(255, 69, 58, .16);
    border: 1px solid rgba(255, 69, 58, .45);
    color: var(--editor-error);
  }
  .verify-alert {
    margin-top: 10px;
    padding: 8px 10px;
    border-radius: 6px;
    background: rgba(255, 69, 58, .12);
    border: 1px solid rgba(255, 69, 58, .4);
    color: var(--editor-error);
    font-size: 12px;
    line-height: 1.4;
  }
  .coverage-pct {
    margin-left: auto;
    font-size: 11px;
    font-weight: 600;
    color: var(--editor-secondary);
  }
  .coverage-bins {
    display: grid;
    grid-template-columns: repeat(36, 1fr);
    gap: 2px;
    height: 10px;
    margin-top: 10px;
    align-items: end;
  }
  .coverage-bin {
    height: 100%;
    border-radius: 2px;
    background: rgba(52, 199, 89, 0.72);
  }
  .coverage-bin.warn { background: #e8a427; }
  .coverage-bin.miss { background: var(--editor-error); }
  .coverage-bar {
    height: 4px;
    margin-top: 8px;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
    overflow: hidden;
  }
  .coverage-fill {
    display: block;
    height: 100%;
    border-radius: 999px;
    background: var(--editor-play);
    transition: width 0.16s ease;
  }
  .dry-run-box {
    margin: 14px 0 0;
    padding: 10px 14px;
    border-radius: 8px;
    background: var(--editor-panel);
    box-shadow: inset 0 0 0 1px var(--editor-border);
  }
  .dry-run-box summary {
    cursor: pointer;
    font-size: 12px;
    font-weight: 600;
    color: var(--editor-secondary);
    user-select: none;
  }
  .dry-run-body {
    margin-top: 10px;
    display: grid;
    gap: 8px;
  }
  .dry-run-empty {
    margin: 0;
    font-size: 12px;
    color: var(--editor-muted);
  }
  .dry-run-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 4px;
    max-height: 160px;
    overflow: auto;
  }
  .dry-run-list li {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    font-size: 12px;
  }
  .dry-run-list small {
    color: var(--editor-muted);
    font-size: 11px;
  }
  button:focus-visible { outline: 2px solid var(--editor-focus); outline-offset: 2px; }
</style>
