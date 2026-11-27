<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { typingReadout } from "../lib/keysounds.js";
  import { auditionAction } from "../lib/tools.js";
  import icon from "../icons/Audition.svg?raw";

  const on = $derived(!!bridge.view?.audition);
  const objectLinesOn = $derived(!!bridge.view?.objectLines);
  const disabled = $derived(bridge.disabled);
  // Keysounds mode swaps this well for the roll's own view toggles (KeysoundsModal.svelte keeps them).
  const keysounds = $derived(bridge.mode === "keysounds");
  const roll = $derived(bridge.keysounds);
  const typing = $derived(roll?.typing ?? null);
  const rollOff = $derived(disabled || !roll);
  const toggle = name => bridge.callAction("keysoundsUi", name);
</script>

<!-- Below 1120 px style.css hides .toolbar-toggle-label; the Playhead label hides below 1400 px. -->
{#if keysounds}
  <div class="ui-well" role="group" aria-label="Piano roll view">
    <button type="button" class="ui-pill ui-pill-toggle" aria-pressed={!!roll?.showGhosts} aria-label="Ghosts"
      title="Show the other tracks' notes behind this one" disabled={rollOff} onclick={() => toggle("toggleGhosts")}
    ><svg viewBox="0 0 24 24" aria-hidden="true" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="6" width="10" height="5" rx="1" stroke-dasharray="3 2"/><rect x="9" y="14" width="12" height="5" rx="1"/></svg><span class="toolbar-toggle-label">Ghosts</span></button>
    <button type="button" class="ui-pill ui-pill-toggle" aria-pressed={!!roll?.showObjectLines} aria-label="Object lines"
      title="Show vertical lines at gameplay objects in the roll" disabled={rollOff} onclick={() => toggle("toggleObjectLines")}
    ><svg viewBox="0 0 24 24" aria-hidden="true" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><line x1="6" y1="3" x2="6" y2="21"/><line x1="18" y1="3" x2="18" y2="21"/><rect x="10" y="7" width="4" height="10" rx="1"/></svg><span class="toolbar-toggle-label">Object lines</span></button>
    <button type="button" class="ui-pill ui-pill-toggle" aria-pressed={!!typing?.enabled} aria-label="Typing"
      title="Play notes with your computer keyboard (Alt+K). Z/X octave, C/V velocity." disabled={rollOff}
      onclick={() => toggle("toggleTyping")}
    ><svg viewBox="0 0 24 24" aria-hidden="true" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><rect x="2" y="6" width="20" height="12" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10"/></svg><span class="toolbar-toggle-label">Typing</span>{#if typing?.enabled}<small class="ks-typing-readout">{typingReadout(typing)}</small>{/if}</button>
  </div>
{:else}
  <div class="ui-well">
    <button type="button" class="ui-pill ui-pill-toggle" aria-pressed={on} aria-label="Audition"
      title="Audition when placing hits" {disabled} onclick={() => act(auditionAction(bridge.view))}
    >{@html icon}<span class="toolbar-toggle-label">Audition</span></button>
    <button type="button" class="ui-pill ui-pill-toggle" aria-pressed={objectLinesOn} aria-label="Object lines"
      title="Show vertical lines at gameplay objects" {disabled} onclick={() => act({ kind: "object_lines", on: !bridge.view?.objectLines }, false)}
    ><svg viewBox="0 0 24 24" aria-hidden="true" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><line x1="6" y1="3" x2="6" y2="21"/><line x1="18" y1="3" x2="18" y2="21"/><rect x="10" y="7" width="4" height="10" rx="1"/></svg><span class="toolbar-toggle-label">Object lines</span></button>
  </div>
{/if}

<style>
  /* The octave and velocity show only while Typing is on, and stay when the label hides. */
  .ks-typing-readout {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    opacity: 0.85;
    white-space: nowrap;
  }
</style>
