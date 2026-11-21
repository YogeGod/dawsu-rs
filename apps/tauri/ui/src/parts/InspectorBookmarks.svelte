<script>
  import Plus from "../icons/Plus.svg?raw";
  import { bridge } from "../lib/bridge.svelte.js";
  import { bookmarkRows, cutOffTitle, emptyInspector, inspectorHandlers } from "../lib/inspector.js";
  import InspectorHead from "./InspectorHead.svelte";

  const on = inspectorHandlers((...args) => globalThis.inspectorAction(...args));
  const model = $derived(bridge.inspector ?? emptyInspector);
  // app.js keeps the list the same array while it does not change, so polls rebuild no rows.
  const marks = $derived(model.bookmarks);
  const pick = $derived(model.bookmark);
  const rows = $derived(bookmarkRows(marks, pick));
  // Go to, Rename and Remove act on the picked row, as they did on the old list's choice.
  const picked = $derived(!model.busy && rows.some(row => row.current));
  // Add, Previous and Next need a difficulty and wait for a running job, as the old markup made them.
  const ready = $derived(model.doc && !model.busy);
  let open = $state(true);
</script>

<div class="insp">
  <div class="insp-titled">
    <InspectorHead title="Bookmarks" body="inspector-bookmarks-body" bind:open />
    <button type="button" class="insp-add" aria-label="Add bookmark" title="Add bookmark at the playhead"
      disabled={!ready} onclick={() => on.bookmark("add")}>{@html Plus}</button>
  </div>
  <div id="inspector-bookmarks-body" hidden={!open}>
    {#if rows.length}
      <div class="insp-marks" role="group" aria-label="Bookmarks">
        {#each rows as row (row.time)}
          <button type="button" class="insp-mark" aria-current={row.current ? "true" : undefined}
            disabled={model.busy} onclick={() => on.pickBookmark(row.time)}
            onpointerenter={event => cutOffTitle(event.currentTarget, row.label)}
            ><span class="insp-dot" aria-hidden="true"></span>{#if row.label}<span class="insp-mark-label">{row.label}</span
            >{/if}<time class="insp-mark-time">{row.clock}</time></button>
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
      <button type="button" disabled={!ready} onclick={() => on.command("previousBookmark")}>Previous</button>
      <button type="button" disabled={!ready} onclick={() => on.command("nextBookmark")}>Next</button>
    </div>
  </div>
</div>
