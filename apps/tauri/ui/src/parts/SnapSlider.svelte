<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import {
    changesSnap, COMMON, currentSnap, divisorColour, keepsFocus, nearestStop, onStop, ownsKey, snapAction, snapColour, STOPS,
    stepStop, stopAt,
  } from "../lib/snap.js";

  let track;
  // The stop the mapper is choosing, shown until the editor has answered.
  let local = $state(null);
  let dragging = false;
  let sending = Promise.resolve();

  const scene = $derived(bridge.scene);
  const disabled = $derived(bridge.disabled);
  const index = $derived(local ?? (scene ? nearestStop(scene.beats, scene.divisor) : 3));
  const exact = $derived(local !== null || !scene || onStop(scene.beats, scene.divisor));
  const group = $derived(bridge.snapGroup ?? COMMON);
  const colour = $derived(local !== null ? divisorColour(STOPS[local]) : scene ? snapColour(scene.beats, scene.divisor) : undefined);
  const value = $derived(local !== null ? "1/" + STOPS[local] : scene ? scene.beats + "/" + scene.divisor : "–");

  function choose(target) {
    const snap = currentSnap(local, scene);
    local = target;
    if (changesSnap(snap, target)) sending = act(snapAction(scene, target));
  }

  function release(input) {
    const sent = sending;
    sent.finally(() => {
      if (!dragging && sending === sent) local = null;
    });
    if (!keepsFocus(input)) track.blur();
  }

  function pointAt(event) {
    if (disabled) return;
    const box = track.getBoundingClientRect();
    choose(stopAt((event.clientX - box.left) / box.width));
  }

  function press(event) {
    if (disabled || event.button !== 0) return;
    dragging = true;
    track.setPointerCapture(event.pointerId);
    pointAt(event);
  }

  function letGo() {
    if (!dragging) return;
    dragging = false;
    release("pointer");
  }

  function key(event) {
    if (!ownsKey(event)) return;
    // The editor's own keydown would seek or move the selection.
    event.preventDefault();
    if (disabled) return;
    choose(stepStop(...currentSnap(local, scene), event.key));
    release("keyboard");
  }
</script>

<div class="snap" class:off={scene && !scene.snap} class:disabled style:--snap={colour}>
  <span class="label">Snap</span>
  <div
    class="track"
    class:between={!exact}
    bind:this={track}
    role="slider"
    tabindex="0"
    aria-label="Snap"
    aria-valuemin={STOPS[0]}
    aria-valuemax={STOPS[STOPS.length - 1]}
    aria-valuenow={scene ? STOPS[index] : undefined}
    aria-valuetext={scene ? value + (scene.snap ? "" : ", snap off") : undefined}
    aria-disabled={disabled}
    onpointerdown={press}
    onpointermove={event => dragging && pointAt(event)}
    onpointerup={letGo}
    onpointercancel={letGo}
    onlostpointercapture={letGo}
    onkeydown={key}
  >
    {#each STOPS as stop, k (stop)}
      {#if k > 0}<span class="segment" class:lit={k <= index}></span>{/if}
      <span class="tick" class:lit={k <= index} class:dim={!group.presets.includes(stop)} style:--tick={divisorColour(stop)}>{#if k === index}<span class="thumb"></span>{/if}</span>
    {/each}
  </div>
  <span class="chip">{value}</span>
</div>

<style>
  .snap {
    /* v3 §3.6: unlit ticks are #555; the tokens have no such grey. */
    --snap-tick-idle: #555;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 32px;
    padding: 0 6px 0 12px;
    border-radius: 6px;
    background: var(--editor-ink);
    box-shadow: var(--shadow-well);
    font-size: 13px;
    /* A drag that selects text would start a text drag on the next press and end the slider drag. */
    user-select: none;
  }
  .off {
    opacity: 0.5;
  }
  .disabled {
    opacity: 0.45;
  }
  .label {
    color: var(--editor-secondary);
    font-weight: 500;
  }
  .track {
    display: flex;
    align-items: center;
    width: 168px;
    height: 20px;
    cursor: pointer;
    touch-action: none;
    border-radius: 4px;
  }
  .disabled .track {
    cursor: default;
  }
  .track:focus-visible {
    outline: 2px solid var(--editor-focus);
    outline-offset: 2px;
  }
  .segment {
    flex: 1;
    height: 2px;
    background: var(--editor-border);
  }
  .tick {
    position: relative;
    flex: none;
    width: 2px;
    height: 10px;
    border-radius: 1px;
    /* Each stop wears its lazer colour; the stops outside the snap group the wheel steps through stay grey. */
    background: var(--tick);
  }
  .segment.lit {
    background: var(--editor-accent);
  }
  .tick.dim {
    background: var(--snap-tick-idle);
  }
  .thumb {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--snap, var(--editor-text));
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.35);
    transform: translate(-50%, -50%);
  }
  /* The real snap has no stop of its own (4/1, 1/5, 1/64): the thumb shows the nearest one, hollow. */
  .between .thumb {
    background: var(--editor-ink);
    box-shadow: inset 0 0 0 2px var(--snap, var(--editor-text)), 0 0 0 1px rgba(0, 0, 0, 0.35);
  }
  .chip {
    display: grid;
    place-items: center;
    box-sizing: border-box;
    min-width: 38px;
    height: 22px;
    padding: 0 6px;
    border-radius: 4px;
    background: var(--editor-selected);
    /* The number stays readable; the snap colour (dark at 1/12 and 1/16) underlines it. */
    box-shadow: inset 0 -2px 0 var(--snap, transparent), var(--shadow-raised);
    color: var(--editor-text);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
</style>
