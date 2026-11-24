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
  let altHeld = $state(false);
  const actualTool = $derived(bridge.view?.tool ?? null);
  const tool = $derived(altHeld && actualTool === "Select" ? "Draw" : actualTool);
  const disabled = $derived(bridge.disabled);
  const pills = $derived(toolPills(tool));
  const buttons = {};
  let well;
  let hadFocus = false;

  function pickTool(name) {
    if (bridge.view && bridge.view.tool !== name) {
      bridge.view = { ...bridge.view, tool: name };
    }
    act(toolAction(name));
  }

  $effect(() => {
    const syncAlt = (event) => {
      altHeld = !!(event.altKey && !event.ctrlKey && !event.metaKey && !event.getModifierState?.("AltGraph"));
    };
    const clearAlt = () => { altHeld = false; };
    window.addEventListener("keydown", syncAlt, true);
    window.addEventListener("keyup", syncAlt, true);
    window.addEventListener("blur", clearAlt);
    return () => {
      window.removeEventListener("keydown", syncAlt, true);
      window.removeEventListener("keyup", syncAlt, true);
      window.removeEventListener("blur", clearAlt);
    };
  });

  // The fourth pill shows an off-toolbar tool by its icon only; pressing it opens the right-click tool picker.
  function openPicker(event) {
    const box = event.currentTarget.getBoundingClientRect();
    window.openEditorMenu?.("tool", false, {
      x: box.left, y: box.bottom, trigger: event.currentTarget, label: "Timeline tools",
    });
  }

  // A pick from the picker gives focus back to the fourth pill, which the new tool then replaces:
  // keep the focus in the toolbar on the new tool's pill.
  $effect.pre(() => {
    actualTool;
    hadFocus = !!well?.contains(document.activeElement);
  });
  $effect(() => {
    const current = actualTool;
    if (hadFocus && current && !well.contains(document.activeElement)) buttons[current]?.focus();
  });
</script>

<div class="ui-well" role="toolbar" aria-label="Editing tools" bind:this={well}>
  {#each pills as name, index (name)}
    {#if index < 3}
      <button type="button" class="ui-pill" aria-pressed={name === tool} title={`${name} (${index + 1})`} {disabled}
        bind:this={buttons[name]} onclick={() => pickTool(name)}>{@html icons[name]}{name}</button>
    {:else}
      <button data-more-tools type="button" class="ui-pill ui-pill-icon current" aria-haspopup="menu" aria-label={name} title={`${name}, More tools (4)`}
        {disabled} bind:this={buttons[name]} onclick={openPicker}>{@html icons[name]}<span aria-hidden="true">▾</span></button>
    {/if}
  {/each}
  {#if pills.length === 3}
    <button data-more-tools type="button" class="ui-pill ui-pill-icon" aria-haspopup="menu" aria-label="More tools" title="More tools (4)"
      {disabled} onclick={openPicker}><span aria-hidden="true">▾</span></button>
  {/if}
</div>
