<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { auditionAction } from "../lib/tools.js";
  import icon from "../icons/Audition.svg?raw";

  const on = $derived(!!bridge.view?.audition);
  const disabled = $derived(bridge.disabled);
</script>

<!-- Below 1780 px style.css hides .toolbar-toggle-label, as it does for Follow and Return. -->
<div class="ui-well">
  <button type="button" class="ui-pill ui-pill-toggle" aria-pressed={on} aria-label="Audition"
    title="Audition when placing hits" {disabled} onclick={() => act(auditionAction(bridge.view))}
  >{@html icon}<span class="toolbar-toggle-label">Audition</span></button>
</div>
