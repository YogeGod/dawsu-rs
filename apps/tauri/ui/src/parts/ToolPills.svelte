<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { toolAction, toolPills } from "../lib/tools.js";
  import Select from "../icons/MousePointer2.svg?raw";
  import Draw from "../icons/Drumstick.svg?raw";
  import Erase from "../icons/Eraser.svg?raw";
  import Mute from "../icons/VolumeX.svg?raw";
  import Grab from "../icons/Hand.svg?raw";
  import Range from "../icons/Scan.svg?raw";
  import Ramp from "../icons/TrendingUp.svg?raw";

  const icons = { Select, Draw, Erase, Mute, Grab, Range, Ramp };
  const tool = $derived(bridge.view?.tool ?? null);
  const disabled = $derived(!bridge.scene || !!bridge.view?.busy);
  const pills = $derived(toolPills(tool));

  // The fourth pill shows an off-toolbar tool; pressing it opens the right-click tool picker.
  function openPicker(event) {
    const box = event.currentTarget.getBoundingClientRect();
    window.openEditorMenu?.("tool", false, {
      x: box.left, y: box.bottom, trigger: event.currentTarget, label: "Timeline tools",
    });
  }
</script>

<div class="ui-well" role="toolbar" aria-label="Editing tools">
  {#each pills as name, index (name)}
    {#if index < 3}
      <button type="button" class="ui-pill" aria-pressed={name === tool} title={name} {disabled}
        onclick={() => act(toolAction(name))}>{@html icons[name]}{name}</button>
    {:else}
      <button type="button" class="ui-pill" aria-pressed="true" aria-haspopup="menu" title={name} {disabled}
        onclick={openPicker}>{@html icons[name]}{name}</button>
    {/if}
  {/each}
</div>
