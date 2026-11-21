<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { auditionAction } from "../lib/tools.js";
  import icon from "../icons/Audition.svg?raw";

  const on = $derived(!!bridge.view?.audition);
  const objectLinesOn = $derived(!!bridge.view?.objectLines);
  const disabled = $derived(bridge.disabled);
</script>

<!-- Below 1120 px style.css hides .toolbar-toggle-label; the Playhead label hides below 1400 px. -->
<div class="ui-well">
  <button type="button" class="ui-pill ui-pill-toggle" aria-pressed={on} aria-label="Audition"
    title="Audition when placing hits" {disabled} onclick={() => act(auditionAction(bridge.view))}
  >{@html icon}<span class="toolbar-toggle-label">Audition</span></button>
  <button type="button" class="ui-pill ui-pill-toggle" aria-pressed={objectLinesOn} aria-label="Object lines"
    title="Show vertical lines at gameplay objects" {disabled} onclick={() => act({ kind: "object_lines", on: !bridge.view?.objectLines }, false)}
  ><svg viewBox="0 0 24 24" aria-hidden="true" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><line x1="6" y1="3" x2="6" y2="21"/><line x1="18" y1="3" x2="18" y2="21"/><rect x="10" y="7" width="4" height="10" rx="1"/></svg><span class="toolbar-toggle-label">Object lines</span></button>
</div>
