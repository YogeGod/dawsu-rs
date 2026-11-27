<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyInspector, inspectorHandlers, laneTint } from "../lib/inspector.js";
  import InspectorHead from "./InspectorHead.svelte";

  const on = inspectorHandlers((...args) => bridge.callAction("inspectorAction", ...args));
  const model = $derived(bridge.inspector ?? emptyInspector);
  const lane = $derived(model.lane);
  const tint = $derived(laneTint(lane));
  let open = $state(true);
  // No lanes show in Keysounds mode: the section stays in the page, hidden, for the ids menus click.
  const keysounds = $derived(bridge.mode === "keysounds");
</script>

<div class="insp" hidden={keysounds}>
  <InspectorHead title="Lane" body="inspector-lane-body" bind:open />
  <!-- Folded, the buttons stay in the page: the lane menu clicks Lane colour and Replace sample by id. -->
  <div id="inspector-lane-body" hidden={!open}>
    {#if lane}
      <p class="insp-lane-name"><strong title={lane.title}>{lane.title}</strong>
        {#if lane.sample}<span class="insp-file" title={lane.sample}>{lane.sample}</span>{/if}</p>
    {/if}
    <div class="insp-rows">
      <div class="insp-row">
        <span class="insp-label">Lane colour</span>
        <button id="lane-colour" type="button" class="insp-colour" aria-haspopup="dialog"
          aria-label={lane ? "Lane colour: " + tint.name : "Lane colour"}
          disabled={!model.doc || !lane?.canColour} onclick={() => on.open("laneColour")}>
          {#if lane}<span class="insp-swatch" style:--swatch={tint.swatch} aria-hidden="true"></span>{tint.name}{/if}
        </button>
      </div>
    </div>
    <div class="insp-actions">
      <button id="replace-sample" type="button" disabled={!model.doc || !lane?.canReplace}
        onclick={() => on.open("replaceSample")}>Replace sample…</button>
    </div>
  </div>
</div>
