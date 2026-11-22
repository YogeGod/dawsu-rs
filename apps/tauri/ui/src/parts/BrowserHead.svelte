<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyBrowser } from "../lib/browser.js";
  import Search from "../icons/Search.svg?raw";

  const model = $derived(bridge.browser ?? emptyBrowser);
  // No map open, or nothing handed over yet: the head shows but takes no input.
  const off = $derived(!bridge.scene || !bridge.browser);
  const send = (...args) => globalThis.sampleBrowserAction?.(...args);
</script>

<div class="browser-head">
  <label class="browser-search">
    <span class="browser-search-icon">{@html Search}</span>
    <input type="search" id="sample-browser-filter" placeholder="Search samples" aria-label="Search samples"
      value={model.filter} disabled={off} oninput={event => send("filter", event.currentTarget.value)}>
  </label>
  <div class="browser-tabs" role="group" aria-label="Sample source">
    <button type="button" id="sample-browser-mapset" aria-pressed={model.source === "mapset"}
      aria-controls="sample-browser-entries" disabled={off} onclick={() => send("source", "mapset")}>Mapset</button>
    <button type="button" id="sample-browser-library" aria-pressed={model.source === "library"}
      aria-controls="sample-browser-entries" disabled={off} onclick={() => send("source", "library")}>Library</button>
  </div>
  <div class="browser-folder-shortcuts" role="group" aria-label="Folder shortcuts">
    <button type="button" id="browser-open-mapset-folder" disabled={off} title="Open Mapset Folder"
      onclick={() => send("openMapsetFolder")}>Open Mapset Folder</button>
    <button type="button" id="browser-open-library-folder" disabled={off} title="Open Sample Library Folder"
      onclick={() => send("openLibraryFolder")}>Open Library Folder</button>
  </div>
  {#if model.folder}
    <div class="browser-folder">
      <button type="button" disabled={off || !model.folder.canUp} onclick={() => send("up")}>Up</button>
      <output aria-label="Current sample folder" title={model.folder.title}>{model.folder.label}</output>
    </div>
  {/if}
</div>
