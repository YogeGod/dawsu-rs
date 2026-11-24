<script>
  import { dbText, faderKey, faderPosition, faderReach, faderSender, positionGain, releaseFader, scaleMarks } from "../lib/mixer.js";

  // v3 §6 fader block: slot, dB scale and meter. A gain of null means the editor has no level for
  // this channel yet: the slot shows greyed out with no cap. The meter stays empty (no levels yet).
  // The cap and the scale share one 138 px travel; the part the gain can't reach is dimmed.
  let { gain = null, min = 0, max = 1, label, disabled = false, onchange } = $props();

  let slot = $state();
  // The value under the pointer while dragging. After a release the cap keeps it until the last
  // action is answered, so it never jumps back.
  let held = $state(null);
  let dragging = false;
  const shown = $derived(held ?? gain);
  const reach = $derived(faderReach(min, max));
  const send = faderSender(value => onchange?.(value), () => {
    if (!dragging) held = null;
  });

  const uiZoom = () => {
    try {
      const z = Number.parseFloat(getComputedStyle(document.documentElement).zoom);
      return Number.isFinite(z) && z > 0 ? z : 1;
    } catch {
      return 1;
    }
  };
  // The cap's centre is 6 px below the slot's top at the top of the travel.
  const gainAt = event => {
    const dy = (event.clientY - slot.getBoundingClientRect().top) / uiZoom();
    return positionGain((dy - 6) / 138, min, max);
  };

  function down(event) {
    if (disabled || event.button !== 0) return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    dragging = true;
    held = gainAt(event);
    send(held);
  }
  function move(event) {
    if (!dragging) return;
    const next = gainAt(event);
    if (Math.abs(next - held) < 0.0005) return;
    held = next;
    send(held);
  }
  function up(event) {
    if (!dragging) return;
    dragging = false;
    releaseFader(event.currentTarget);
    send(held);
  }

  function key(event) {
    if (disabled) return;
    const next = faderKey(event, shown, min, max);
    if (next === null) return;
    // The document's arrow keys would otherwise move the playhead.
    event.preventDefault();
    event.stopPropagation();
    if (next !== shown) send(next);
  }
</script>

<div class="fader">
  {#if shown === null || shown === undefined}
    <div class="slot off" aria-hidden="true"><div class="track"></div></div>
  {:else}
    <div class="slot" bind:this={slot} role="slider" tabindex={disabled ? -1 : 0} aria-label={label}
      aria-orientation="vertical" aria-valuemin={Math.round(min * 100)} aria-valuemax={Math.round(max * 100)}
      aria-valuenow={Math.round(shown * 100)} aria-valuetext={dbText(shown) + " dB"} aria-disabled={disabled}
      onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} onkeydown={key}>
      {#if reach.top > 0}<div class="reach top" style:--fader-at={reach.top}></div>{/if}
      {#if reach.bottom < 1}<div class="reach bottom" style:--fader-at={reach.bottom}></div>{/if}
      <div class="track"></div>
      <div class="cap" style:--fader-at={faderPosition(shown)}><span></span></div>
    </div>
  {/if}
  <div class="scale" aria-hidden="true">
    {#each scaleMarks as [mark, at] (mark)}<span style:--fader-at={at}>{mark}</span>{/each}
  </div>
  <div class="meter off" aria-hidden="true"><div></div><div></div></div>
</div>

<style>
  .fader {
    height: 150px;
    display: flex;
    gap: 3px;
    flex: none;
  }
  .slot {
    position: relative;
    width: 30px;
    flex: none;
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.4);
    touch-action: none;
    cursor: ns-resize;
  }
  .slot.off {
    cursor: default;
    opacity: 0.45;
  }
  .slot[aria-disabled="true"] {
    cursor: default;
  }
  .slot:focus-visible {
    outline: 2px solid var(--editor-focus);
    outline-offset: 1px;
  }
  .track {
    position: absolute;
    left: 14px;
    top: 6px;
    bottom: 6px;
    width: 2px;
    border-radius: 1px;
    background: #0a0a0a;
  }
  /* The travel the gain can't reach, dimmed, with a line at the stop. */
  .reach {
    position: absolute;
    left: 2px;
    right: 2px;
    background: rgba(0, 0, 0, 0.45);
  }
  .reach.top {
    top: 6px;
    height: calc(var(--fader-at) * 138px);
    border-bottom: 1px solid var(--editor-muted);
  }
  .reach.bottom {
    top: calc(6px + var(--fader-at) * 138px);
    bottom: 6px;
    border-top: 1px solid var(--editor-muted);
  }
  .cap {
    position: absolute;
    left: 4px;
    top: calc(var(--fader-at) * 138px);
    width: 22px;
    height: 12px;
    border-radius: 2px;
    background: #cdcdcd;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
  }
  .cap span {
    width: 100%;
    height: 1px;
    background: #3c3c3c;
  }
  .scale {
    position: relative;
    flex: 1;
    min-width: 14px;
  }
  .scale span {
    position: absolute;
    right: 1px;
    top: calc(6px + var(--fader-at) * 138px);
    margin-top: -4px;
    font-size: 9px;
    line-height: 9px;
    color: var(--editor-muted);
    font-variant-numeric: tabular-nums;
  }
  .meter {
    width: 11px;
    flex: none;
    border-radius: 2px;
    background: #0a0a0a;
    display: flex;
    gap: 1px;
    padding: 1px;
    box-sizing: border-box;
  }
  .meter div {
    flex: 1;
  }
  .meter.off {
    opacity: 0.45;
  }
</style>
