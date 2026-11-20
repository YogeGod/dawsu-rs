<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { auditionAction } from "../lib/tools.js";
  import icon from "../icons/Audition.svg?raw";

  const on = $derived(!!bridge.view?.audition);
  const disabled = $derived(!bridge.scene || !!bridge.view?.busy);
</script>

<div class="ui-well">
  <button type="button" class="ui-pill" aria-pressed={on} title="Audition when placing hits" {disabled}
    onclick={() => act(auditionAction(bridge.view))}>{@html icon}Audition</button>
</div>
