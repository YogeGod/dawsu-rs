<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    dismissSetupChecklist,
    isSetupChecklistDismissed,
    recentItems,
    recoveryState,
    setupChecklistState,
  } from "../lib/startScreen.js";

  const MODE_LABELS = { 1: "taiko", 2: "catch", 3: "mania" };
  const storage = () => { try { return globalThis.localStorage; } catch { return null; } };

  const view = $derived(bridge.view);
  const scene = $derived(bridge.scene);
  const busy = $derived(!!view?.busy);
  const recent = $derived(recentItems(view));
  const recovery = $derived(recoveryState(view, scene));
  const songsFolder = $derived(view?.songsFolder || "");
  const hasSongsFolder = $derived(Boolean(songsFolder));
  const send = (...args) => globalThis.startScreenAction?.(...args);

  let checklistDismissed = $state(isSetupChecklistDismissed(storage()));
  const checklist = $derived(setupChecklistState(view, checklistDismissed));

  function dismissChecklist() {
    dismissSetupChecklist(storage());
    checklistDismissed = true;
  }

  let songsQuery = $state("");
  let songsMatches = $state.raw([]);
  let songsTotal = $state(0);
  let songsLoading = $state(false);
  let songsError = $state("");
  let searchSeq = 0;
  let debounceTimer = null;

  async function runSongsSearch(query, refresh = false) {
    if (typeof window === "undefined" || !window.__TAURI__?.core?.invoke) return;
    if (!songsFolder) {
      songsMatches = [];
      songsTotal = 0;
      songsError = "";
      return;
    }
    const seq = ++searchSeq;
    songsLoading = true;
    songsError = "";
    try {
      const res = await invoke("songs_index_search", { query, refresh });
      if (seq !== searchSeq) return;
      songsMatches = res?.matches ?? [];
      songsTotal = res?.totalMapsets ?? 0;
    } catch (err) {
      if (seq !== searchSeq) return;
      songsError = String(err || "Could not read Songs folder.");
      songsMatches = [];
    } finally {
      if (seq === searchSeq) songsLoading = false;
    }
  }

  $effect(() => {
    const folder = songsFolder;
    const query = songsQuery;
    if (!folder) {
      songsMatches = [];
      songsTotal = 0;
      songsError = "";
      return;
    }
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      runSongsSearch(query, false);
    }, query ? 120 : 0);
    return () => clearTimeout(debounceTimer);
  });

  function mapsetHeading(set) {
    const artist = set.artist?.trim();
    const title = set.title?.trim();
    if (artist && title) return `${artist} - ${title}`;
    return title || artist || set.folder;
  }
</script>

<div class="home-panel">
  <img class="home-logo" src="studio-assets/dawsu.png" alt="dawsu!">
  <h1>Open a map to start hitsounding</h1>
  <p>Every sample in the mapset becomes a lane you draw hits on, over the song.</p>
  <button id="home-resume" type="button" class="primary" hidden={!view?.showHome || !view?.hasDocument}
    disabled={busy} onclick={() => send("resume")}>Resume editing</button>
  <div class="home-actions" role="group" aria-label="Open a map">
    <button id="home-open" type="button" class="home-action" disabled={busy} onclick={() => send("open")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6.5h6l2 2h10V19H3z"/></svg>
      <span><strong>Open map…</strong><small>An .osu difficulty or .osz mapset</small></span>
    </button>
    <button id="home-open-osu" type="button" class="home-action" disabled={busy} onclick={() => send("openFromOsu")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 17H7A5 5 0 0 1 7 7h2m6 0h2a5 5 0 1 1 0 10h-2m-7-5h8"/></svg>
      <span><strong>Open from osu!</strong><small>Open the map active in osu! editor</small></span>
    </button>
    <button id="home-import" type="button" class="home-action" disabled={busy} onclick={() => send("importOsz")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3.5 4.5h17v4h-17zM5 8.5V19h14V8.5M12 11v5m-2.5-2.5L12 16l2.5-2.5"/></svg>
      <span><strong>Import .osz…</strong><small>Unpack a mapset and open it</small></span>
    </button>
    <button id="home-folders" type="button" class="home-action" disabled={busy} onclick={() => send("folders")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 4.5h5l1.5 1.5H21v9H6zM3 8v11h14"/></svg>
      <span><strong>Folders…</strong><small>Songs folder and hitsound library</small></span>
    </button>
  </div>
  <section id="home-setup" class="home-setup" aria-labelledby="home-setup-title" hidden={!checklist.visible}>
    <div class="home-setup-head">
      <h2 id="home-setup-title">Get set up</h2>
      <span class="home-setup-progress">{checklist.completedCount} of {checklist.totalCount} done</span>
      <button id="home-setup-dismiss" type="button" class="home-setup-dismiss" onclick={dismissChecklist}>Dismiss</button>
    </div>
    <ul class="home-setup-list">
      {#each checklist.steps as step (step.id)}
        <li class="home-setup-item" class:done={step.done} data-step={step.id}>
          <span class="home-setup-check" aria-hidden="true">
            {#if step.done}
              <svg viewBox="0 0 16 16"><path d="M3.5 8.5 6.5 11.5 12.5 4.5"/></svg>
            {/if}
          </span>
          <span class="home-setup-copy">
            <strong>{step.label}</strong>
            <small>{step.detail}</small>
          </span>
          {#if step.done}
            <span class="home-setup-done">Done</span>
          {:else}
            <button type="button" class="home-setup-action" disabled={busy}
              onclick={() => send(step.action)}>{step.actionLabel}</button>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
  <section id="recovery" aria-labelledby="recovery-title" hidden={!recovery.visible}>
    <h2 id="recovery-title">Recovery copies</h2>
    <div id="recovery-entries" class="recovery-entries">
      {#if recovery.error}
        <p>{recovery.error}</p>
      {/if}
      {#each recovery.offers as offer (offer.id)}
        <p>{offer.name}<br><time>{offer.createdText}</time></p>
        <button type="button" aria-label={"Restore " + offer.name} disabled={busy}
          onclick={() => send("restoreRecovery", offer.id)}>Restore</button>
      {/each}
    </div>
  </section>
  <div class="home-drop">
    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 3h8l4 4v14H6zM14 3v4h4M12 10v7m-3-3 3 3 3-3"/></svg>
    Or drop an .osu or .osz file anywhere in this window
  </div>
  <section id="home-songs" class="home-songs" aria-labelledby="home-songs-title" hidden={!hasSongsFolder}>
    <div class="home-songs-head">
      <h2 id="home-songs-title">Songs folder</h2>
      {#if songsTotal > 0}
        <span class="home-songs-count">{songsTotal.toLocaleString()} mapsets</span>
      {/if}
      <button id="home-songs-refresh" type="button" class="home-songs-refresh"
        disabled={busy || songsLoading} onclick={() => runSongsSearch(songsQuery, true)}>
        {songsLoading ? "Indexing…" : "Refresh"}
      </button>
    </div>
    <input
      id="home-songs-search"
      class="home-songs-search"
      type="search"
      placeholder="Search artist, title, mapper, diff, tags, or BeatmapSetID…"
      aria-label="Search Songs folder"
      disabled={busy}
      bind:value={songsQuery}
    />
    <div id="home-songs-list" class="home-songs-list">
      {#if songsError}
        <p class="home-songs-empty">{songsError}</p>
      {:else if songsLoading && !songsMatches.length}
        <p class="home-songs-empty">Indexing Songs folder…</p>
      {:else if !songsMatches.length}
        <p class="home-songs-empty">{songsQuery.trim() ? "No matching mapsets" : "No mapsets found in Songs folder"}</p>
      {:else}
        {#each songsMatches as set (set.folder)}
          <div class="home-song-set">
            <div class="home-song-meta">
              <strong>{mapsetHeading(set)}</strong>
              {#if set.creator}
                <span class="home-song-creator">by {set.creator}</span>
              {/if}
            </div>
            <div class="home-song-diffs">
              {#each set.diffs as diff (diff.path)}
                <button type="button" class="home-song-diff" title={diff.path} disabled={busy}
                  onclick={() => send("openPath", diff.path)}>
                  <span>{diff.version}</span>
                  {#if MODE_LABELS[diff.mode]}
                    <small>{MODE_LABELS[diff.mode]}</small>
                  {/if}
                </button>
              {/each}
            </div>
          </div>
        {/each}
      {/if}
    </div>
  </section>
  <section id="home-recent" class="home-recent" aria-labelledby="home-recent-title" hidden={!recent.length}>
    <h2 id="home-recent-title">Recent</h2>
    <div id="recent">
      {#each recent as item (item.path)}
        <button type="button" class="home-recent-map" title={item.path}
          onclick={() => send("openRecent", item.path)}>
          <strong>{item.name}</strong><span>{item.difficulty}</span>
        </button>
      {/each}
    </div>
  </section>
</div>

<style>
  .recovery-entries {
    max-height: 200px;
    overflow-y: auto;
  }
  .home-setup {
    width: 100%;
    text-align: left;
    display: grid;
    gap: 10px;
    padding: 12px 14px;
    border: 1px solid var(--editor-border);
    border-radius: var(--radius-8);
    background: var(--editor-panel);
  }
  .home-setup-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .home-setup-head h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--editor-text);
  }
  .home-setup-progress {
    font-size: 12px;
    color: var(--editor-muted);
  }
  .home-setup-dismiss {
    margin-left: auto;
    min-height: 24px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--radius-6);
    background: transparent;
    color: var(--editor-muted);
    font-size: 12px;
    cursor: pointer;
  }
  .home-setup-dismiss:hover {
    background: var(--editor-hover);
    color: var(--editor-text);
  }
  .home-setup-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 6px;
  }
  .home-setup-item {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-6);
    background: var(--editor-chrome);
  }
  .home-setup-check {
    flex: none;
    width: 18px;
    height: 18px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1.5px solid var(--editor-border);
    border-radius: 999px;
    color: var(--editor-text);
  }
  .home-setup-item.done .home-setup-check {
    border-color: var(--editor-accent);
    background: var(--editor-accent);
    color: #111111;
  }
  .home-setup-check svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .home-setup-copy {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
  }
  .home-setup-copy strong {
    font-size: 13px;
    font-weight: 600;
    color: var(--editor-text);
  }
  .home-setup-item.done .home-setup-copy strong {
    color: var(--editor-muted);
  }
  .home-setup-copy small {
    font-size: 12px;
    color: var(--editor-muted);
  }
  .home-setup-action {
    flex: none;
    min-height: 26px;
    padding: 0 10px;
    border: 1px solid var(--editor-border);
    border-radius: var(--radius-6);
    background: var(--editor-control);
    color: var(--editor-text);
    font-size: 12px;
    cursor: pointer;
  }
  .home-setup-action:hover:not(:disabled) {
    border-color: var(--editor-accent);
    background: var(--editor-hover);
  }
  .home-setup-done {
    flex: none;
    font-size: 12px;
    font-weight: 500;
    color: var(--editor-muted);
  }
  .home-songs {
    width: 100%;
    text-align: left;
    display: grid;
    gap: 10px;
    padding: 12px 14px;
    border: 1px solid var(--editor-border);
    border-radius: var(--radius-8);
    background: var(--editor-panel);
  }
  .home-songs-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .home-songs-head h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--editor-text);
  }
  .home-songs-count {
    font-size: 12px;
    color: var(--editor-muted);
  }
  .home-songs-refresh {
    margin-left: auto;
    min-height: 26px;
    padding: 0 10px;
    border: 1px solid var(--editor-border);
    border-radius: var(--radius-6);
    background: var(--editor-control);
    color: var(--editor-secondary);
    font-size: 12px;
  }
  .home-songs-refresh:hover:not(:disabled) {
    background: var(--editor-hover);
    color: var(--editor-text);
  }
  .home-songs-search {
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--editor-border);
    border-radius: var(--radius-6);
    background: var(--editor-canvas);
    color: var(--editor-text);
    font-size: 13px;
  }
  .home-songs-list {
    display: grid;
    gap: 6px;
    max-height: 240px;
    overflow-y: auto;
  }
  .home-songs-empty {
    margin: 4px 2px;
    font-size: 12px;
    color: var(--editor-muted);
  }
  .home-song-set {
    display: grid;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-6);
    background: var(--editor-chrome);
  }
  .home-song-meta {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .home-song-meta strong {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 600;
    color: var(--editor-text);
  }
  .home-song-creator {
    flex: none;
    font-size: 12px;
    color: var(--editor-muted);
  }
  .home-song-diffs {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .home-song-diff {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-height: 24px;
    padding: 2px 9px;
    border: 1px solid var(--editor-border);
    border-radius: 999px;
    background: var(--editor-control);
    color: var(--editor-text);
    font-size: 12px;
  }
  .home-song-diff:hover:not(:disabled) {
    border-color: var(--editor-accent);
    background: var(--editor-hover);
  }
  .home-song-diff small {
    font-size: 10px;
    color: var(--editor-muted);
    text-transform: uppercase;
  }
</style>
