<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyInspector, historyRows, inspectorHandlers } from "../lib/inspector.js";
  import InspectorHead from "./InspectorHead.svelte";

  const on = inspectorHandlers((...args) => globalThis.inspectorAction(...args));
  const model = $derived(bridge.inspector ?? emptyInspector);
  const steps = $derived(historyRows(model.history, model.historyPosition));
  let open = $state(true);
</script>

<div class="insp">
  <InspectorHead title="History" body="inspector-history-body" bind:open />
  <div id="inspector-history-body" hidden={!open}>
    <!-- docking.js keeps this list's scroll place by its id when the docks change. -->
    <div id="history" class="insp-steps">
      {#each steps as step (step.position)}
        <button type="button" class="insp-step" class:insp-undone={step.undone}
          aria-current={step.current ? "step" : undefined} title={step.label}
          onclick={() => on.history(step.position)}>{step.label}</button>
      {/each}
    </div>
  </div>
</div>
