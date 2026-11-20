<script>
  import Plus from "../icons/Plus.svg?raw";
  import { bridge } from "../lib/bridge.svelte.js";
  import { bookmarkRows, emptyInspector, inspectorHandlers } from "../lib/inspector.js";
  import InspectorHead from "./InspectorHead.svelte";

  const on = inspectorHandlers((...args) => globalThis.inspectorAction(...args));
  const model = $derived(bridge.inspector ?? emptyInspector);
  const rows = $derived(bookmarkRows(model.bookmarks, model.bookmark));
  // Go to, Rename and Remove act on the picked row, as they did on the old list's choice.
  const picked = $derived(!model.busy && rows.some(row => row.current));
  let open = $state(true);
</script>

<div class="insp">
  <div class="insp-titled">
    <InspectorHead title="Bookmarks" body="inspector-bookmarks-body" bind:open />
    <button type="button" class="insp-add" aria-label="Add bookmark" title="Add bookmark at the playhead"
      disabled={!model.doc} onclick={() => on.bookmark("add")}>{@html Plus}</button>
  </div>
  <div id="inspector-bookmarks-body" hidden={!open}>
    {#if rows.length}
      <div class="insp-marks" role="group" aria-label="Bookmarks">
        {#each rows as row (row.time)}
          <button type="button" class="insp-mark" aria-current={row.current ? "true" : undefined}
            title={"Go to " + row.clock} disabled={model.busy} onclick={() => on.pickBookmark(row.time)}
            ><span class="insp-dot" aria-hidden="true"></span><span class="insp-mark-label" title={row.label}>{row.label}</span
            ><time class="insp-mark-time">{row.clock}</time></button>
        {/each}
      </div>
    {:else}
      <p class="insp-summary">No bookmarks. Add one at the playhead with +.</p>
    {/if}
    <div class="insp-actions">
      <button type="button" disabled={!picked} onclick={() => on.bookmark("goTo")}>Go to</button>
      <button type="button" disabled={!picked} onclick={() => on.bookmark("rename")}>Rename…</button>
      <button type="button" disabled={!picked} onclick={() => on.bookmark("remove")}>Remove</button>
    </div>
    <div class="insp-actions">
      <button type="button" disabled={!model.doc} onclick={() => on.command("previousBookmark")}>Previous</button>
      <button type="button" disabled={!model.doc} onclick={() => on.command("nextBookmark")}>Next</button>
    </div>
  </div>
</div>
