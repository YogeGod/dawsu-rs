<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { BROWSER_TAGS, emptyBrowser } from "../lib/browser.js";
  import { nameOf } from "../editor/helpers.js";
  import { invoke } from "../lib/ipc.js";
  import Search from "../icons/Search.svg?raw";
  import Folder from "../icons/Folder.svg?raw";
  import FolderOpen from "../icons/FolderOpen.svg?raw";

  const model = $derived(bridge.browser ?? emptyBrowser);
  // No map open, or nothing handed over yet: the head shows but takes no input.
  const off = $derived(!bridge.scene || !bridge.browser);
  const send = (...args) => bridge.callAction("sampleBrowserAction", ...args);
  // One click to the mapset's or the sample library's folder in the file manager (Rust resolves it).
  const openFolder = folder => invoke("open_editor_folder", { folder }).catch(() => {});
</script>

<div class="browser-head">
  <div class="browser-search-row">
    <label class="browser-search">
      <span class="browser-search-icon">{@html Search}</span>
      <input type="search" id="sample-browser-filter" placeholder="Search samples" aria-label="Search samples"
        value={model.filter} disabled={off} oninput={event => send("filter", event.currentTarget.value)}>
    </label>
    <button type="button" class="browser-open-folder" id="sample-browser-open-map" aria-label="Open mapset folder"
      title="Open mapset folder" disabled={off} onclick={() => openFolder("map")}>{@html FolderOpen}</button>
    <button type="button" class="browser-open-folder" id="sample-browser-open-library" aria-label="Open sample library folder"
      title="Open sample library folder" disabled={off} onclick={() => openFolder("library")}>{@html Folder}</button>
  </div>
  <div class="browser-tabs" role="group" aria-label="Sample source">
    <button type="button" id="sample-browser-mapset" aria-pressed={model.source === "mapset"}
      aria-controls="sample-browser-entries" disabled={off} onclick={() => send("source", "mapset")}>Mapset</button>
    <button type="button" id="sample-browser-library" aria-pressed={model.source === "library"}
      aria-controls="sample-browser-entries" disabled={off} onclick={() => send("source", "library")}>Library</button>
  </div>
  <div class="browser-tags" role="group" aria-label="Filter by instrument tag">
    {#each BROWSER_TAGS as tag (tag)}
      <button type="button" class="browser-tag-chip" aria-pressed={model.activeTag === tag}
        disabled={off} onclick={() => send("tag", tag)}>{tag}</button>
    {/each}
    {#if model.similarTo}
      <button type="button" class="browser-tag-chip active-sim" aria-pressed="true"
        disabled={off} onclick={() => send("clearSimilar")}>Similar: {nameOf(model.similarTo)} ×</button>
    {/if}
  </div>
  {#if model.folder && !model.activeTag && !model.similarTo}
    <div class="browser-folder">
      <button type="button" disabled={off || !model.folder.canUp} onclick={() => send("up")}>Up</button>
      <output aria-label="Current sample folder" title={model.folder.title}>{model.folder.label}</output>
    </div>
  {/if}
</div>
