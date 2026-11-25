<script>
  import { PX_PER_MS, pitchName, velocityAtY } from "../../lib/midi.js";

  const VEL_PX = 64;

  let {
    velScroll = $bindable(null),
    spanPx = 0,
    notes = [],
    selected = $bindable(null),
    loading = false,
    onsetvelocity,
  } = $props();

  // The velocity bar being dragged: { index, velocity }.
  let velDrag = $state(null);

  function velocityOf(note) {
    return velDrag?.index === note.index ? velDrag.velocity : note.velocity;
  }

  function onVelDown(event, note) {
    if (event.button !== 0 || loading) return;
    selected = note.index;
    const box = event.currentTarget.parentElement.getBoundingClientRect();
    velDrag = { index: note.index, velocity: velocityAtY(event.clientY - box.top, box.height), top: box.top, height: box.height };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onVelMove(event) {
    if (velDrag) velDrag.velocity = velocityAtY(event.clientY - velDrag.top, velDrag.height);
  }

  function onVelUp() {
    if (!velDrag) return;
    const { index, velocity } = velDrag;
    velDrag = null;
    const note = notes.find(n => n.index === index);
    if (note && note.velocity !== velocity) onsetvelocity(index, velocity);
  }
</script>

<div class="ks-vel-wrap">
  <span class="ks-vel-label">Velocity</span>
  <div class="ks-vel" bind:this={velScroll}>
    <div class="ks-vel-lane" style:--ks-span={spanPx + "px"} style:--ks-vel-h={VEL_PX + "px"}>
      {#each notes as note (note.index)}
        <div class="ks-vel-hit" role="slider" tabindex="-1" aria-label={`Velocity of ${pitchName(note.pitch)}`}
          aria-valuemin="1" aria-valuemax="127" aria-valuenow={velocityOf(note)}
          style:--ks-left={note.startMs * PX_PER_MS - 4 + "px"}
          onpointerdown={(e) => onVelDown(e, note)} onpointermove={onVelMove} onpointerup={onVelUp}>
          <div class="ks-vel-bar" class:selected={selected === note.index}
            style:--ks-bar-h={velocityOf(note) / 127 * VEL_PX + "px"}></div>
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .ks-vel-wrap {
    display: flex;
    border-top: 1px solid var(--editor-border);
    background: #151517;
  }
  .ks-vel-label {
    flex: 0 0 56px;
    padding: 6px;
    box-sizing: border-box;
    border-right: 1px solid var(--editor-border);
    font-size: 10px;
    color: var(--editor-muted);
  }
  .ks-vel {
    flex: 1;
    overflow: hidden;
  }
  .ks-vel-lane {
    position: relative;
    width: var(--ks-span);
    height: var(--ks-vel-h);
  }
  .ks-vel-hit {
    position: absolute;
    top: 0;
    bottom: 0;
    left: var(--ks-left);
    width: 9px;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    cursor: ns-resize;
    touch-action: none;
  }
  .ks-vel-bar {
    width: 4px;
    height: var(--ks-bar-h);
    border-radius: 2px 2px 0 0;
    background: #0a84ff;
    pointer-events: none;
  }
  .ks-vel-bar.selected {
    background: #efc16a;
  }
</style>
