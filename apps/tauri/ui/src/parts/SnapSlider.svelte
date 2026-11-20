<script>
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { nearestStop, onStop, sliderKeys, snapAction, STOPS, stepStop, stopAt } from "../lib/snap.js";

  let track;
  // The stop the mapper is choosing, shown until the editor has answered.
  let local = $state(null);
  let dragging = false;
  let sending = Promise.resolve();

  const scene = $derived(bridge.scene);
  const disabled = $derived(!scene || !!bridge.view?.busy);
  const index = $derived(local ?? (scene ? nearestStop(scene.beats, scene.divisor) : 3));
  const exact = $derived(local !== null || !scene || onStop(scene.beats, scene.divisor));
  const value = $derived(local !== null ? "1/" + STOPS[local] : scene ? scene.beats + "/" + scene.divisor : "–");

  function current() {
    return local !== null ? [1, STOPS[local]] : [scene.beats, scene.divisor];
  }

  function choose(target) {
    const [beats, divisor] = current();
    local = target;
    if (divisor !== beats * STOPS[target]) sending = act(snapAction(scene, target));
  }

  function release() {
    const sent = sending;
    sent.finally(() => {
      if (!dragging && sending === sent) local = null;
    });
  }

  function pointAt(event) {
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
    release();
  }

  function key(event) {
    if (!sliderKeys.includes(event.key) || event.ctrlKey || event.altKey || event.metaKey) return;
    // The editor's own keydown would seek or move the selection.
    event.preventDefault();
    if (disabled) return;
    choose(stepStop(...current(), event.key));
    release();
  }
</script>

<div class="snap" class:off={scene && !scene.snap} class:disabled>
  <span class="label">Snap</span>
  <div
    class="track"
    class:between={!exact}
    bind:this={track}
    role="slider"
    tabindex="0"
    aria-label="Snap"
    aria-valuemin={STOPS[0]}
    aria-valuemax={STOPS.at(-1)}
    aria-valuenow={STOPS[index]}
    aria-valuetext={value + (scene && !scene.snap ? ", snap off" : "")}
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
      <span class="tick" class:lit={k <= index}>{#if k === index}<span class="thumb"></span>{/if}</span>
    {/each}
  </div>
  <span class="chip">{value}</span>
</div>

<style>
  .snap {
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
    background: #555;
  }
  .lit {
    background: var(--editor-accent);
  }
  .thumb {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #f1f1f1;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.35);
    transform: translate(-50%, -50%);
  }
  /* The real snap has no stop of its own (4/1, 1/5, 1/64): the thumb shows the nearest one, hollow. */
  .between .thumb {
    background: var(--editor-ink);
    box-shadow: inset 0 0 0 2px #f1f1f1, 0 0 0 1px rgba(0, 0, 0, 0.35);
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
    box-shadow: var(--shadow-raised);
    color: var(--editor-accent);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
</style>
