<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { getCachedBackground, requestMapsetBackground } from "../lib/mapsetBackground.js";
  import { isHitsoundVersion } from "../lib/startScreen.js";

  const MODE_LABELS = { 1: "taiko", 2: "catch", 3: "mania" };

  let {
    mapset = undefined,
    busy = undefined,
    inline = false,
    onopen = null,
    oncreatehs = null,
    onclose = null,
  } = $props();

  const activeMapset = $derived(mapset !== undefined ? mapset : bridge.intakeMapset);
  const isBusy = $derived(busy !== undefined ? !!busy : !!bridge.view?.busy);

  const diffs = $derived(Array.isArray(activeMapset?.difficulties) ? activeMapset.difficulties : []);
  const isStandard = diff => diff?.mode == null || diff.mode === 0;
  const isHs = diff => Boolean(diff?.isHs) || isHitsoundVersion(diff?.version);
  const hsDiff = $derived(diffs.find(d => isHs(d)) ?? null);
  const standardNonHs = $derived(diffs.filter(d => !isHs(d) && isStandard(d)));
  const standardDiffs = $derived(diffs.filter(d => isStandard(d)));
  let selectedRefPath = $state("");
  let fetchedBgByKey = $state({});

  const bgKey = $derived(activeMapset?.folder || diffs[0]?.path || "");
  const bgUrl = $derived(
    activeMapset?.backgroundUrl ||
    fetchedBgByKey[bgKey] ||
    getCachedBackground(bgKey) ||
    ""
  );

  $effect(() => {
    const key = bgKey;
    const firstNonHs = diffs.find(d => !isHs(d)) ?? diffs[0];
    const osuPath = firstNonHs?.path || "";
    if (!key || !osuPath || activeMapset?.backgroundUrl) return;
    const cached = requestMapsetBackground(key, osuPath, (url) => {
      if (url && fetchedBgByKey[key] !== url) fetchedBgByKey[key] = url;
    });
    if (cached && fetchedBgByKey[key] !== cached) fetchedBgByKey[key] = cached;
  });

  $effect(() => {
    const candidates = standardNonHs.length ? standardNonHs : standardDiffs;
    const first = candidates[0]?.path ?? "";
    if (!selectedRefPath || !candidates.some(d => d.path === selectedRefPath)) {
      selectedRefPath = first;
    }
  });

  const heading = $derived.by(() => {
    if (!activeMapset) return "";
    const artist = activeMapset.artist?.trim();
    const title = activeMapset.title?.trim();
    if (artist && title) return `${artist} - ${title}`;
    return title || artist || activeMapset.folder || "Mapset folder";
  });

  const folderError = $derived.by(() => {
    if (!activeMapset) return "";
    if (activeMapset.corrupt || activeMapset.error) {
      return activeMapset.error || "This mapset folder could not be read.";
    }
    if (diffs.length === 0) return "No readable .osu difficulties found in this folder.";
    return "";
  });

  function handleClose() {
    if (onclose) onclose();
    else bridge.intakeMapset = null;
  }

  function handleOpen(path) {
    if (!path || isBusy) return;
    if (onopen) onopen(path);
    else {
      bridge.intakeMapset = null;
      globalThis.startScreenAction?.("openPath", path);
    }
  }

  function handleCreateHs(refPath, starterMode) {
    if (!refPath || isBusy) return;
    if (oncreatehs) oncreatehs(refPath, starterMode);
    else {
      bridge.intakeMapset = null;
      globalThis.startScreenAction?.("createHsDiff", refPath, starterMode);
    }
  }

  function onKeydown(event) {
    if (!inline && event.key === "Escape" && activeMapset) {
      event.stopPropagation();
      handleClose();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet cardBody()}
  <header class="intake-head" class:has-bg={Boolean(bgUrl)}>
    {#if bgUrl}
      <div class="intake-hero-backdrop" aria-hidden="true">
        <img class="intake-hero-bg" src={bgUrl} alt="" decoding="async">
        <div class="intake-hero-scrim"></div>
      </div>
    {/if}
    <div class="intake-head-copy">
      <p class="intake-eyebrow">{activeMapset.creator ? `Mapped by ${activeMapset.creator}` : "Mapset"}</p>
      <h2 class="intake-title">{heading}</h2>
    </div>
    {#if !inline}
      <button type="button" class="intake-close" aria-label="Close" onclick={handleClose}>✕</button>
    {/if}
  </header>

  <div class="intake-body">
  {#if folderError}
    <p class="intake-error" role="alert">{folderError}</p>
  {:else}
    {#if hsDiff}
      <div class="intake-continue">
        <button type="button" class="primary intake-continue-btn" disabled={isBusy}
          onclick={() => handleOpen(hsDiff.path)}>
          Continue [{hsDiff.version || hsDiff.fileName}] →
        </button>
        <details class="intake-more">
          <summary>Create another hitsound diff…</summary>
          <div class="intake-paths">
            {#if standardDiffs.length === 0}
              <p class="intake-note">No osu!standard difficulty found to copy timing from.</p>
            {:else}
              {#if standardNonHs.length > 1}
                <label class="intake-ref">
                  <span>Timing reference</span>
                  <select bind:value={selectedRefPath} disabled={isBusy}>
                    {#each standardNonHs as diff (diff.path)}
                      <option value={diff.path}>{diff.version || diff.fileName}</option>
                    {/each}
                  </select>
                </label>
              {/if}
              <button type="button" class="intake-path" disabled={isBusy || !selectedRefPath}
                onclick={() => handleCreateHs(selectedRefPath, "custom_only")}>
                <strong>Custom samples only</strong>
                <small>Blank [Hitsounds] diff with 0 lanes. Add or drop your own .wav files.</small>
              </button>
              <button type="button" class="intake-path" disabled={isBusy || !selectedRefPath}
                onclick={() => handleCreateHs(selectedRefPath, "default_12")}>
                <strong>12 osu! default lanes</strong>
                <small>Blank [Hitsounds] diff with osu!'s 12 skin sounds and Drum Map ready.</small>
              </button>
            {/if}
          </div>
        </details>
      </div>
    {:else}
      <section class="intake-section" aria-label="New hitsound diff">
        <p class="intake-section-label">Start a dedicated [Hitsounds] diff</p>
        {#if standardDiffs.length === 0}
          <p class="intake-note">No osu!standard difficulty found to copy timing from.</p>
        {:else}
          {#if standardNonHs.length > 1}
            <label class="intake-ref">
              <span>Timing reference</span>
              <select bind:value={selectedRefPath} disabled={isBusy}>
                {#each standardNonHs as diff (diff.path)}
                  <option value={diff.path}>{diff.version || diff.fileName}</option>
                {/each}
              </select>
            </label>
          {/if}
          <div class="intake-paths">
            <button type="button" class="intake-path intake-path-primary" disabled={isBusy || !selectedRefPath}
              onclick={() => handleCreateHs(selectedRefPath, "custom_only")}>
              <strong>Custom samples only</strong>
              <small>Starts with 0 lanes. Add your own .wav files or drop them onto the timeline.</small>
            </button>
            <button type="button" class="intake-path" disabled={isBusy || !selectedRefPath}
              onclick={() => handleCreateHs(selectedRefPath, "default_12")}>
              <strong>12 osu! default lanes</strong>
              <small>Starts with osu!'s 12 default skin sounds in Drum Map order.</small>
            </button>
          </div>
        {/if}
      </section>
    {/if}

    <section class="intake-section" aria-label="Edit an existing difficulty directly">
      <p class="intake-section-label">Or edit a difficulty directly</p>
      <div class="intake-diffs" role="list" aria-label="Difficulties">
        {#each diffs as diff (diff.path)}
          {@const supported = isStandard(diff)}
          <button
            type="button"
            class="intake-diff"
            class:unsupported={!supported}
            disabled={isBusy || !supported}
            title={supported ? diff.path : `${MODE_LABELS[diff.mode] || "Unsupported"} mode is not supported`}
            onclick={() => handleOpen(diff.path)}
          >
            <span class="intake-diff-name">{diff.version || diff.fileName}</span>
            {#if isHs(diff)}
              <span class="intake-badge-hs">HS</span>
            {/if}
            {#if !supported && MODE_LABELS[diff.mode]}
              <span class="intake-badge-mode">{MODE_LABELS[diff.mode]}</span>
            {/if}
          </button>
        {/each}
      </div>
    </section>

    <p class="intake-shield">Auto-backup stays on: your original mapset is snapshotted before any write.</p>
  {/if}
  </div>
{/snippet}

{#if activeMapset}
  {#if inline}
    <aside class="intake-card intake-card-inline" aria-label="Selected mapset">
      {@render cardBody()}
    </aside>
  {:else}
    <div class="intake-backdrop" role="presentation"
      onclick={e => { if (e.target === e.currentTarget) handleClose(); }}>
      <div class="intake-card" role="dialog" aria-modal="true" aria-label="Open mapset">
        {@render cardBody()}
      </div>
    </div>
  {/if}
{/if}

<style>
  .intake-backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.65);
    padding: 16px;
  }
  .intake-card {
    width: min(500px, 100%);
    padding: 0;
    border: 1px solid var(--editor-border);
    border-radius: 12px;
    background: var(--editor-panel);
    color: var(--editor-text);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    text-align: left;
  }
  .intake-card-inline {
    width: 100%;
    min-width: 0;
    max-width: 100%;
    position: sticky;
    top: 16px;
  }
  .intake-head {
    position: relative;
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 12px;
    padding: 18px 20px 14px;
    min-height: 76px;
    overflow: hidden;
  }
  .intake-head.has-bg {
    min-height: 132px;
    padding: 20px 20px 16px;
  }
  .intake-hero-backdrop {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .intake-hero-bg {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 35%;
    display: block;
  }
  .intake-hero-scrim {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      180deg,
      rgba(12, 12, 15, 0.14) 0%,
      rgba(21, 21, 23, 0.58) 54%,
      var(--editor-panel) 100%
    );
  }
  .intake-head-copy {
    position: relative;
    z-index: 1;
    min-width: 0;
  }
  .intake-eyebrow {
    margin: 0 0 2px;
    font-size: 12px;
    color: rgba(235, 235, 240, 0.82);
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.85);
  }
  .intake-title {
    margin: 0;
    font-size: 18px;
    font-weight: 700;
    letter-spacing: -0.02em;
    overflow-wrap: anywhere;
    color: #ffffff;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.9);
  }
  .intake-close {
    position: relative;
    z-index: 1;
    align-self: flex-start;
    flex: none;
    min-width: 28px;
    min-height: 28px;
    border: 0;
    border-radius: 6px;
    background: rgba(16, 16, 20, 0.65);
    color: var(--editor-text);
    cursor: pointer;
  }
  .intake-close:hover {
    background: var(--editor-hover);
    color: var(--editor-text);
  }
  .intake-body {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 0 20px 18px;
    min-width: 0;
  }
  .intake-error {
    margin: 0;
    padding: 10px 12px;
    border-radius: 8px;
    background: rgba(239, 68, 68, 0.14);
    color: #fca5a5;
    font-size: 12px;
  }
  .intake-note {
    margin: 0;
    font-size: 12px;
    color: var(--editor-muted);
  }
  .intake-continue {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .intake-continue-btn {
    width: 100%;
    white-space: normal;
  }
  .intake-more summary {
    cursor: pointer;
    font-size: 12px;
    color: var(--editor-secondary);
    user-select: none;
  }
  .intake-more[open] .intake-paths {
    margin-top: 8px;
  }
  .intake-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .intake-section-label {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--editor-text);
  }
  .intake-paths {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .intake-path {
    display: flex;
    flex-direction: column;
    gap: 3px;
    width: 100%;
    min-width: 0;
    padding: 10px 12px;
    border: 1px solid var(--editor-border);
    border-radius: 8px;
    background: var(--editor-chrome);
    color: var(--editor-text);
    text-align: left;
    white-space: normal;
    line-height: 1.35;
    cursor: pointer;
  }
  .intake-path-primary {
    border-color: var(--editor-accent);
    background: rgba(56, 189, 248, 0.1);
  }
  .intake-path:hover:not(:disabled) {
    border-color: var(--editor-accent);
    background: var(--editor-hover);
  }
  .intake-path strong {
    font-size: 13px;
    font-weight: 600;
  }
  .intake-path small {
    font-size: 11px;
    color: var(--editor-secondary);
  }
  .intake-diffs {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--editor-border);
  }
  .intake-diff {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 8px 6px;
    border: 0;
    border-bottom: 1px solid rgba(43, 43, 48, 0.7);
    border-radius: 0;
    background: transparent;
    color: var(--editor-text);
    font-size: 13px;
    font-weight: 500;
    text-align: left;
    cursor: pointer;
    transition: background 0.12s ease, color 0.12s ease;
  }
  .intake-diff:hover:not(:disabled) {
    background: var(--editor-hover);
    color: var(--editor-accent);
  }
  .intake-diff.unsupported {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .intake-diff-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .intake-badge-hs {
    color: var(--editor-accent);
    font-size: 11px;
    font-weight: 600;
    flex: none;
  }
  .intake-badge-mode {
    color: var(--editor-muted);
    font-size: 11px;
    font-weight: 500;
    flex: none;
  }
  .intake-ref {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 12px;
    color: var(--editor-secondary);
  }
  .intake-ref select {
    flex: 1 1 auto;
    min-height: 28px;
    padding: 0 8px;
    border-radius: 6px;
  }
  .intake-shield {
    margin: 2px 0 0;
    padding: 10px 0 0;
    border-top: 1px solid var(--editor-border);
    color: var(--editor-muted);
    font-size: 12px;
  }
</style>
