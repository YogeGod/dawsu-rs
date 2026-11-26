<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import {
    getCachedBackground,
    observeMapsetBg,
    requestMapsetBackground,
  } from "../lib/mapsetBackground.js";
  import MapsetIntakeCard from "./MapsetIntakeCard.svelte";
  import {
    dismissSetupChecklist,
    isHitsoundVersion,
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
  const send = (...args) => bridge.callAction("startScreenAction", ...args);

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
  let selectedMapsetFolder = $state("");
  let bgUrls = $state({});
  let searchSeq = 0;
  let debounceTimer = null;

  function primaryDiffPath(set) {
    const diffs = set?.diffs ?? [];
    const nonHs = diffs.find(d => !isHitsoundVersion(d.version));
    return (nonHs ?? diffs[0])?.path || "";
  }

  function recordBgUrl(key, url) {
    if (key && url && bgUrls[key] !== url) {
      bgUrls[key] = url;
    }
  }

  const selectedMapset = $derived.by(() => {
    if (!songsMatches.length) return null;
    const found = selectedMapsetFolder
      ? songsMatches.find(s => s.folder === selectedMapsetFolder)
      : null;
    const set = found ?? songsMatches[0];
    if (!set) return null;
    const backgroundUrl = bgUrls[set.folder] || getCachedBackground(set.folder) || "";
    return { ...set, difficulties: set.diffs ?? [], backgroundUrl };
  });

  $effect(() => {
    const set = selectedMapset;
    if (!set?.folder) return;
    const osuPath = primaryDiffPath(set);
    if (!osuPath) return;
    const cached = requestMapsetBackground(set.folder, osuPath, (url) => {
      recordBgUrl(set.folder, url);
    });
    if (cached) recordBgUrl(set.folder, cached);
  });

  function openIntakeForSet(set) {
    selectedMapsetFolder = set.folder;
    const payload = {
      ...set,
      difficulties: set.diffs ?? [],
      backgroundUrl: bgUrls[set.folder] || getCachedBackground(set.folder) || "",
    };
    if (typeof window !== "undefined" && window.innerWidth < 1020) {
      send("openMapsetIntake", payload);
    }
  }

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

  function coverTone(folder) {
    const text = String(folder ?? "");
    let hash = 0;
    for (let i = 0; i < text.length; i++) hash = ((hash * 31) + text.charCodeAt(i)) | 0;
    return `home-art-${Math.abs(hash) % 6}`;
  }

  async function relinkRecent(missingPath, pickFolder = false) {
    if (busy) return;
    try {
      const res = await invoke("relink_moved_mapset", { missingPath, pickFolder });
      if (res?.newPath) {
        bridge.showToast({
          tone: "success",
          title: "Mapset relinked",
          body: res.migratedBase
            ? "Moved folder found and Shared Base migrated."
            : "Moved folder found.",
        });
        send("openPath", res.newPath);
      }
    } catch (err) {
      if (!pickFolder) {
        // Auto-locate in Songs folder did not find it; fall back to manual folder picker.
        return relinkRecent(missingPath, true);
      }
      const msg = String(err || "");
      if (!msg.toLowerCase().includes("cancelled")) {
        bridge.showToast({ tone: "error", title: "Could not relink mapset", body: msg });
      }
    }
  }
</script>

<div class="home-panel" class:has-right-intake={Boolean(selectedMapset)}>
  <div class="home-main">
  <div class="home-hero">
    <div class="home-hero-copy">
      <img class="home-logo" src="studio-assets/dawsu.png" alt="dawsu!">
      <div>
        <h1>Open a map to start hitsounding</h1>
        <p>Every sample in the mapset becomes a lane you draw hits on, over the song.</p>
      </div>
    </div>
    <button id="home-resume" type="button" class="primary" hidden={!view?.showHome || !view?.hasDocument}
      disabled={busy} onclick={() => send("resume")}>Resume editing</button>
  </div>
  <div class="home-actions" role="group" aria-label="Open a map">
    <button id="home-open" type="button" class="home-action" disabled={busy} onclick={() => send("open")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6.5h6l2 2h10V19H3z"/></svg>
      <span><strong>Open map…</strong><small>An .osu difficulty or .osz mapset</small></span>
    </button>
    <button id="home-create-hs" type="button" class="home-action" disabled={busy} onclick={() => send("createHsDiff")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5v14M5 12h14"/></svg>
      <span><strong>Create Hitsound diff…</strong><small>Start a blank [Hitsounds] diff for a mapset</small></span>
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
    Or drop an .osu, .osz, or mapset folder anywhere in this window
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
          {@const bgUrl = bgUrls[set.folder] || getCachedBackground(set.folder) || ""}
          <div
            class="home-song-set"
            class:selected={selectedMapset?.folder === set.folder}
            use:observeMapsetBg={{ key: set.folder, osuPath: primaryDiffPath(set), onLoad: recordBgUrl }}
          >
            <div class="home-song-backdrop {coverTone(set.folder)}" aria-hidden="true">
              {#if bgUrl}
                <img class="home-song-bg" src={bgUrl} alt="" decoding="async">
              {/if}
              <div class="home-song-scrim"></div>
            </div>
            <div class="home-song-body">
              <div class="home-song-meta">
                <button type="button" class="home-song-heading" disabled={busy}
                  onclick={() => openIntakeForSet(set)}>
                  <strong>{mapsetHeading(set)}</strong>
                  {#if set.creator}
                    <span class="home-song-creator">by {set.creator}</span>
                  {/if}
                </button>
              </div>
              <div class="home-song-diffs">
                {#each set.diffs as diff (diff.path)}
                  <button type="button" class="home-song-diff" title={diff.path} disabled={busy}
                    onclick={() => send("openPath", diff.path)}>
                    <span>{diff.version}</span>
                    {#if isHitsoundVersion(diff.version)}
                      <span class="home-song-hs">HS</span>
                    {/if}
                    {#if MODE_LABELS[diff.mode]}
                      <small>{MODE_LABELS[diff.mode]}</small>
                    {/if}
                  </button>
                {/each}
                {#if set.diffs?.length}
                  <button type="button" class="home-song-diff home-song-create-hs"
                    title="Choose how to start hitsounding this mapset"
                    disabled={busy}
                    onclick={() => send("openMapsetIntake", { ...set, difficulties: set.diffs ?? [], backgroundUrl: bgUrl })}>
                    + Hitsounds
                  </button>
                {/if}
              </div>
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
        <div class="home-recent-row">
          <button type="button" class="home-recent-map" title={item.path}
            onclick={() => send("openRecent", item.path)}>
            {#if item.backgroundUrl}
              <img class="home-recent-bg" src={item.backgroundUrl} alt="" loading="lazy">
            {/if}
            <strong>{item.name}</strong><span>{item.difficulty}</span>
            {#if item.isHs}<span class="home-recent-hs">HS</span>{/if}
          </button>
          <button type="button" class="home-recent-relink" disabled={busy}
            title="Relink if this mapset folder moved (keeps Shared Base and project settings)"
            aria-label={"Relink moved folder for " + item.name}
            onclick={() => relinkRecent(item.path, false)}>Locate…</button>
        </div>
      {/each}
    </div>
  </section>
  </div>
  {#if selectedMapset}
    <div class="home-right-intake">
      <MapsetIntakeCard
        mapset={selectedMapset}
        {busy}
        inline={true}
        onopen={path => send("openPath", path)}
        oncreatehs={(refPath, starterMode) => send("createHsDiff", refPath, starterMode)}
      />
    </div>
  {/if}
</div>

<style>
  :global(#home) .home-panel {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    width: min(1040px, 100%);
    margin: 0 auto;
    padding: 8px 0 24px;
    text-align: left;
    gap: 16px;
  }
  :global(#home) .home-panel.has-right-intake {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 340px;
    justify-items: stretch;
    gap: 20px;
    align-items: start;
    width: min(1380px, 100%);
    max-width: 1380px;
  }
  .home-main {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
    width: 100%;
    min-width: 0;
  }
  .home-hero {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding-bottom: 4px;
  }
  .home-hero-copy {
    display: flex;
    align-items: center;
    gap: 14px;
    min-width: 0;
  }
  .home-hero-copy .home-logo {
    width: 40px;
    height: 40px;
    margin: 0;
    flex: none;
  }
  .home-hero-copy h1 {
    margin: 0;
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    color: var(--editor-text);
  }
  .home-hero-copy p {
    margin: 2px 0 0;
    font-size: 13px;
    color: var(--editor-muted);
  }
  :global(#home .home-main .home-actions) {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(152px, 1fr));
    gap: 8px;
    width: 100%;
  }
  :global(#home .home-panel:not(.has-right-intake) .home-main .home-actions) {
    grid-template-columns: repeat(5, minmax(0, 1fr));
  }
  @media (max-width: 1120px) {
    :global(#home .home-panel:not(.has-right-intake) .home-main .home-actions) {
      grid-template-columns: repeat(auto-fit, minmax(175px, 1fr));
    }
  }
  :global(#home .home-main .home-action) {
    gap: 10px;
    min-height: 64px;
    padding: 10px 11px;
    border-radius: var(--radius-8);
    border: 1px solid var(--editor-border);
    background: var(--editor-panel);
    transition: border-color 0.14s ease, background 0.14s ease;
  }
  :global(#home .home-main .home-action svg) {
    width: 20px;
    height: 20px;
  }
  :global(#home .home-main .home-action:hover:not(:disabled)) {
    border-color: var(--editor-accent);
    background: var(--editor-chrome);
  }
  :global(#home .home-main .home-drop) {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    width: 100%;
    padding: 12px 16px;
    border-radius: var(--radius-8);
    border: 1px dashed var(--editor-border);
    background: var(--editor-panel);
    color: var(--editor-muted);
    font-size: 12px;
  }
  :global(#home .home-main .home-drop svg) {
    width: 16px;
    height: 16px;
  }
  .home-right-intake {
    width: 100%;
    min-width: 0;
    position: sticky;
    top: 16px;
  }
  .home-song-set.selected {
    border-color: var(--editor-accent);
    box-shadow: 0 0 0 1px var(--editor-accent);
    background: var(--editor-panel);
  }
  @media (max-width: 1019px) {
    :global(#home) .home-panel.has-right-intake {
      grid-template-columns: minmax(0, 1fr);
    }
    .home-right-intake {
      display: none;
    }
  }
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
    min-width: 0;
    text-align: left;
    display: flex;
    flex-direction: column;
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
    width: 100%;
    min-width: 0;
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
    flex: none;
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
    min-width: 0;
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
    grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
    grid-auto-rows: max-content;
    align-content: start;
    gap: 10px;
    width: 100%;
    min-width: 0;
    max-height: 440px;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 2px 4px 2px 2px;
  }
  .home-songs-empty {
    margin: 4px 2px;
    font-size: 12px;
    color: var(--editor-muted);
    grid-column: 1 / -1;
  }
  .home-song-set {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 112px;
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-chrome);
    transition: border-color 0.14s ease, box-shadow 0.14s ease;
  }
  .home-song-set:hover {
    border-color: var(--editor-muted);
  }
  .home-song-backdrop {
    position: absolute;
    inset: 0;
    border-radius: 9px;
    overflow: hidden;
    pointer-events: none;
  }
  .home-song-bg {
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 35%;
    display: block;
    transition: transform 0.28s ease;
  }
  .home-song-set:hover .home-song-bg {
    transform: scale(1.04);
  }
  .home-song-scrim {
    position: absolute;
    inset: 0;
    background: linear-gradient(
      180deg,
      rgba(10, 10, 13, 0.22) 0%,
      rgba(10, 10, 13, 0.62) 48%,
      rgba(10, 10, 13, 0.90) 100%
    );
  }
  .home-art-0 { background: linear-gradient(135deg, #112332 0%, #1d445c 65%, #111c24 100%); }
  .home-art-1 { background: linear-gradient(135deg, #241932 0%, #4a2a54 65%, #191420 100%); }
  .home-art-2 { background: linear-gradient(135deg, #2a1d18 0%, #583524 65%, #1c1513 100%); }
  .home-art-3 { background: linear-gradient(135deg, #132527 0%, #224b4d 65%, #111c1e 100%); }
  .home-art-4 { background: linear-gradient(135deg, #181c30 0%, #2e3762 65%, #141621 100%); }
  .home-art-5 { background: linear-gradient(135deg, #18231c 0%, #2d4935 65%, #131a16 100%); }
  .home-song-body {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 8px;
    padding: 18px 12px 11px;
    flex: 1;
    min-width: 0;
  }
  .home-song-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    justify-content: flex-end;
    min-width: 0;
    overflow: hidden;
  }
  .home-song-heading {
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 2px;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    padding: 0;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
    color: inherit;
    font: inherit;
  }
  .home-song-heading:hover:not(:disabled) {
    background: transparent;
    border-color: transparent;
  }
  .home-song-heading:hover:not(:disabled) strong {
    color: var(--editor-accent);
  }
  .home-song-meta strong {
    display: block;
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    font-weight: 650;
    color: #ffffff;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.9);
  }
  .home-song-creator {
    display: block;
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 11.5px;
    color: rgba(235, 235, 240, 0.84);
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.9);
  }
  .home-song-diffs {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .home-song-diff {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 100%;
    min-height: 23px;
    padding: 2px 8px;
    border: 1px solid rgba(255, 255, 255, 0.14);
    border-radius: 999px;
    background: rgba(16, 16, 20, 0.78);
    backdrop-filter: blur(6px);
    color: var(--editor-text);
    font-size: 11.5px;
    overflow: hidden;
  }
  .home-song-diff > span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .home-song-diff:hover:not(:disabled) {
    border-color: var(--editor-accent);
    background: rgba(28, 28, 34, 0.92);
  }
  .home-song-diff small {
    font-size: 10px;
    color: var(--editor-muted);
    text-transform: uppercase;
  }
  .home-song-hs {
    display: inline-flex;
    align-items: center;
    height: 15px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--editor-accent);
    color: #111111;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .home-song-create-hs {
    border-style: dashed;
    border-color: rgba(90, 200, 250, 0.45);
    color: var(--editor-accent);
  }
  .home-recent {
    width: 100%;
    text-align: left;
  }
  .home-recent h2 {
    margin: 0 0 10px;
    font-size: 14px;
    font-weight: 600;
    color: var(--editor-text);
  }
  .home-recent :global(#recent) {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 8px;
  }
  .home-recent-row {
    display: flex;
    align-items: stretch;
    gap: 4px;
  }
  :global(#home .home-recent-row .home-recent-map) {
    position: relative;
    overflow: hidden;
    flex: 1 1 auto;
    min-width: 0;
    border: 1px solid var(--editor-border);
    background: var(--editor-panel);
  }
  :global(#home .home-recent-row .home-recent-map:hover:not(:disabled)) {
    border-color: var(--editor-muted);
    background: var(--editor-chrome);
  }
  :global(#home .home-recent-row .home-recent-map::before),
  :global(#home .home-recent-row .home-recent-map > :not(.home-recent-bg)) {
    position: relative;
    z-index: 1;
  }
  :global(#home .home-recent-row .home-recent-map strong) {
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.85);
  }
  :global(#home .home-recent-row .home-recent-bg) {
    position: absolute;
    inset: 0;
    z-index: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: center 35%;
    opacity: 0.34;
    pointer-events: none;
  }
  .home-recent-hs {
    flex: none;
    display: inline-flex;
    align-items: center;
    height: 16px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--editor-accent);
    color: #111111;
    font-size: 10px;
    font-weight: 700;
  }
  .home-recent-relink {
    flex: 0 0 auto;
    padding: 0 8px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-control);
    color: var(--editor-muted);
    font-size: 11px;
    cursor: pointer;
  }
  .home-recent-relink:hover:not(:disabled) {
    color: var(--editor-text);
    border-color: var(--editor-accent);
  }
</style>
