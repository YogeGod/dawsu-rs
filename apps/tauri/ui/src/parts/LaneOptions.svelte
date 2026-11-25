<script>
  import { bridge } from "../lib/bridge.svelte.js";

  const scene = $derived(bridge.scene);
  const disabled = $derived(bridge.disabled);
  const rowHeight = $derived(bridge.rowHeight ?? scene?.rowHeight ?? 30);
  const send = (...args) => globalThis.popoverAction?.(...args);
</script>

<div class="row" role="group" aria-label="Lane zoom">
  <button type="button" id="lane-height-out" aria-label="Shorter lanes" {disabled}
    onclick={() => send("laneHeightStep", 1 / 1.15)}>−</button>
  <label class="toggle">Lane height
    <input type="range" min="20" max="65" step="1" id="lane-height" value={rowHeight} {disabled}
      oninput={event => send("laneHeight", Number(event.currentTarget.value), true)}
      onchange={event => send("laneHeight", Number(event.currentTarget.value), false)}>
  </label>
  <button type="button" id="lane-height-in" aria-label="Taller lanes" {disabled}
    onclick={() => send("laneHeightStep", 1.15)}>+</button>
</div>
<button type="button" id="collapse-layers" {disabled} onclick={() => send("collapseLayers")}>Collapse sample layers</button>
<label class="toggle" title="Show only the lanes that have hits when switched on">
  <input id="used-only" type="checkbox" checked={bridge.usedOnly} {disabled}
    onchange={event => send("usedOnly", event.currentTarget.checked)}>
  Used only
</label>
<label class="toggle" title="Group standard lanes under Normal, Soft, and Drum folders">
  <input id="group-by-sample-set" type="checkbox" checked={bridge.groupBySampleSet} {disabled}
    onchange={event => send("groupBySampleSet", event.currentTarget.checked)}>
  Group by sample set
</label>
<input id="lane-filter" type="search" aria-label="Filter lanes" placeholder="Filter lanes" size="13"
  value={bridge.laneFilter} {disabled} oninput={event => send("laneFilter", event.currentTarget.value)}>
