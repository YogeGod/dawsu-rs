<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyBrowser } from "../lib/browser.js";
  import Folder from "../icons/Folder.svg?raw";
  import FolderOpen from "../icons/FolderOpen.svg?raw";
  import Volume1 from "../icons/Volume1.svg?raw";

  const model = $derived(bridge.browser ?? emptyBrowser);
  // Rows sit one step in under the tree's top row (Mapset samples or the library folder).
  const nest = $derived(model.root ? 1 : 0);
  const send = (...args) => globalThis.sampleBrowserAction?.(...args);
</script>

{#if model.root}
  <div class="browser-row browser-root" title={model.rootTitle}>
    <span class="browser-icon">{@html Folder}</span><span class="browser-name">{model.root}</span>
  </div>
{/if}
{#each model.rows as row (row.key)}
  {#if row.kind === "note"}
    <p class="browser-note" style:--depth={row.depth + nest}>{row.label}</p>
  {:else if row.kind === "folder"}
    <button type="button" class="browser-row" style:--depth={row.depth + nest} title={row.title}
      aria-expanded={model.classic ? undefined : row.open} onclick={() => send("toggle", row.key)}>
      <span class="browser-icon">{@html row.open ? FolderOpen : Folder}</span><span class="browser-name">{row.label}</span>
    </button>
  {:else}
    <button type="button" class="browser-row browser-sample" class:browser-selected={row.selected}
      style:--depth={row.depth + nest} title={row.title} aria-pressed={row.selected} disabled={model.busy}
      onclick={() => send("pick", row.key)} ondblclick={() => send("apply", row.key)}>
      <span class="browser-icon">{@html Volume1}</span><span class="browser-name">{row.label}</span>
    </button>
  {/if}
{/each}
