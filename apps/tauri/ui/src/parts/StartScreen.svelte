<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { recentItems, recoveryState } from "../lib/startScreen.js";

  const view = $derived(bridge.view);
  const scene = $derived(bridge.scene);
  const busy = $derived(!!view?.busy);
  const recent = $derived(recentItems(view));
  const recovery = $derived(recoveryState(view, scene));
  const send = (...args) => globalThis.startScreenAction?.(...args);
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
    <button id="home-open-folder" type="button" class="home-action" disabled={busy} onclick={() => send("openFolder")}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 6.5h6l2 2h10V19H3z"/></svg>
      <span><strong>Open folder…</strong><small>A mapset folder from Songs</small></span>
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
  <section id="home-recent" class="home-recent" aria-labelledby="home-recent-title" hidden={!recent.length}>
    <h2 id="home-recent-title">Recent</h2>
    <div id="recent">
      {#each recent as item (item.path)}
        <button type="button" class="home-recent-map" title={item.path}
          onclick={() => send("openRecent", item.path)}>
          {#if item.backgroundUrl}
            <img class="home-recent-bg" src={item.backgroundUrl} alt="" loading="lazy">
          {/if}
          <strong>{item.name}</strong>
          <span>{item.difficulty}</span>
          {#if item.isHs}<span class="home-recent-hs">HS</span>{/if}
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
</style>
