<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyInspector, inspectorHandlers, syncChecked } from "../lib/inspector.js";
  import InspectorHead from "./InspectorHead.svelte";

  const on = inspectorHandlers((...args) => globalThis.inspectorAction(...args));
  const model = $derived(bridge.inspector ?? emptyInspector);
  let open = $state(true);
  // Re-runs on every update, so a switch the editor refused flips back.
  const showChecked = value => box => syncChecked(box, value());
</script>

<div class="insp">
  <InspectorHead title="Difficulty" body="inspector-difficulty-body" bind:open />
  <!-- Folded, the buttons stay in the page: the Map menu and the New difficulty shortcut click them by id. -->
  <div id="inspector-difficulty-body" hidden={!open}>
    <div class="insp-rows">
      <label class="insp-row">Hitsounding mode
        <input type="checkbox" aria-label="Hitsounding mode" checked={model.hs}
          {@attach showChecked(() => model.hs)} onchange={event => on.toggle("hs_mode", event)}>
      </label>
    </div>
    <div class="insp-actions">
      <button id="generate" type="button" disabled={!model.doc} onclick={() => on.open("generate")}>Generate…</button>
      <button id="new" type="button" onclick={() => on.open("new")}>New…</button>
      <button id="copy" type="button" onclick={() => on.open("copy")}>Copy…</button>
      <button id="merge-settings" type="button" aria-haspopup="dialog" disabled={!model.doc}
        onclick={() => on.open("mergeSettings")}>Merge settings…</button>
    </div>
  </div>
</div>
