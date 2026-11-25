<script>
  import {
    baseName, difficultyName, fileName, targetCounts, targetCoverageBins, targetCoveragePct, targetDiff,
  } from "../../lib/export.js";

  let {
    view,
    choices = $bindable(),
    status,
    baseText = "",
    baseOsuDiff = null,
    listeningTarget = null,
    dryRunFiles = null,
    dryRunBusy = false,
    ontoggle,
    ontogglelisten,
    onrundryrun,
  } = $props();
</script>

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
    {@const coveragePct = on ? targetCoveragePct(status.preview, path) : null}
    {@const coverageBins = on ? targetCoverageBins(status.preview, path) : []}
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
              onclick={() => ontogglelisten(path)}>
              {listeningTarget === path ? "■ Stop" : "▶ Listen"}
            </button>
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
    cursor: pointer;
    white-space: nowrap;
  }
  .listen-btn:hover:not(:disabled) {
    border-color: var(--editor-accent);
    background: var(--editor-hover);
  }
  .listen-btn[aria-pressed="true"] {
    border-color: var(--editor-accent);
    background: var(--editor-accent);
    color: var(--editor-accentink);
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
