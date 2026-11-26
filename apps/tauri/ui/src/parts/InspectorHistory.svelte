<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyInspector, historyRows, inspectorHandlers, scrollNearest } from "../lib/inspector.js";
  import InspectorHead from "./InspectorHead.svelte";

  const on = inspectorHandlers((...args) => bridge.callAction("inspectorAction", ...args));
  const model = $derived(bridge.inspector ?? emptyInspector);
  // app.js keeps the list the same array while it does not change, so polls rebuild no rows.
  const list = $derived(model.history);
  const position = $derived(model.historyPosition);
  const steps = $derived(historyRows(list, position));
  let open = $state(false);
  let box = $state();
  // When the position changes (an undo, a redo, a new step), the current step comes into view.
  $effect(() => {
    if (position >= 0 && box) scrollNearest(box, box.querySelector("[aria-current]"));
  });
</script>

<div class="insp">
  <InspectorHead title="History" body="inspector-history-body" bind:open />
  <div id="inspector-history-body" hidden={!open}>
    <!-- docking.js keeps this list's scroll place by its id when the docks change. -->
    <div id="history" class="insp-steps" bind:this={box}>
      {#each steps as step (step.position)}
        <button type="button" class="insp-step" class:insp-undone={step.undone}
          aria-current={step.current ? "step" : undefined} title={step.label}
          onclick={() => on.history(step.position)}>{step.label}</button>
      {/each}
    </div>
  </div>
</div>
