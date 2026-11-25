<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyInspector, inspectorHandlers } from "../lib/inspector.js";
  import CoverageReview from "./CoverageReview.svelte";
  import InspectorHead from "./InspectorHead.svelte";

  const on = inspectorHandlers((...args) => globalThis.inspectorAction(...args));
  const model = $derived(bridge.inspector ?? emptyInspector);
  // Hitsounding mode lives in Preferences › This map, which keeps this section to the map's actions.
  let open = $state(true);
</script>

<div class="insp">
  <InspectorHead title="Difficulty" body="inspector-difficulty-body" bind:open />
  <!-- Folded, the buttons stay in the page for backwards compatibility; menus use direct menuCalls. -->
  <div id="inspector-difficulty-body" hidden={!open}>
    <CoverageReview />
    <div class="insp-actions">
      <button id="generate" type="button" disabled={!model.doc} onclick={() => on.open("generate")}>Generate…</button>
      <button id="new" type="button" onclick={() => on.open("new")}>New…</button>
      <button id="copy" type="button" onclick={() => on.open("copy")}>Copy…</button>
      <button id="merge-settings" type="button" aria-haspopup="dialog" disabled={!model.doc}
        onclick={() => on.open("mergeSettings")}>Merge settings…</button>
    </div>
  </div>
</div>
