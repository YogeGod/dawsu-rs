<script>
  import { PX_PER_MS, pitchName, velocityAtY } from "../../lib/midi.js";
  import { sampleVelocityRamp } from "../../lib/pianoRoll.js";

  const VEL_PX = 64;

  let {
    velScroll = $bindable(null),
    spanPx = 0,
    notes = [],
    selected = $bindable(null),
    loading = false,
    onsetvelocity,
    onvelocityramp = null,
  } = $props();

  // Single velocity bar drag: { index, velocity, top, height }.
  let velDrag = $state(null);
  // Shift+drag velocity ramp: { startMs, endMs, startVel, endVel, curve, left, top, height }.
  let rampDrag = $state(null);

  const rampPoints = $derived.by(() => {
    if (!rampDrag) return "";
    const steps = 24;
    const pts = [];
    for (let i = 0; i <= steps; i++) {
      const f = i / steps;
      const ms = rampDrag.startMs + (rampDrag.endMs - rampDrag.startMs) * f;
      const v = sampleVelocityRamp(ms, rampDrag.startMs, rampDrag.endMs, rampDrag.startVel, rampDrag.endVel, rampDrag.curve)
        ?? rampDrag.startVel;
      const x = Math.max(0, ms * PX_PER_MS);
      const y = VEL_PX * (1 - v / 127);
      pts.push(`${x.toFixed(1)},${y.toFixed(1)}`);
    }
    return pts.join(" ");
  });

  function velocityOf(note) {
    if (rampDrag) {
      const previewVel = sampleVelocityRamp(
        note.startMs,
        rampDrag.startMs,
        rampDrag.endMs,
        rampDrag.startVel,
        rampDrag.endVel,
        rampDrag.curve,
      );
      if (previewVel != null) return previewVel;
    }
    return velDrag?.index === note.index ? velDrag.velocity : note.velocity;
  }

  function onVelDown(event, note) {
    if (event.button !== 0 || loading) return;
    if (event.shiftKey && onvelocityramp) return;
    event.stopPropagation();
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

  function onLaneDown(event) {
    if (event.button !== 0 || !event.shiftKey || loading || !onvelocityramp) return;
    const box = event.currentTarget.getBoundingClientRect();
    const startMs = Math.max(0, (event.clientX - box.left) / PX_PER_MS);
    const startVel = velocityAtY(event.clientY - box.top, box.height);
    const curve = event.altKey ? 1 : (event.ctrlKey ? -1 : 0);
    rampDrag = {
      startMs,
      endMs: startMs,
      startVel,
      endVel: startVel,
      curve,
      left: box.left,
      top: box.top,
      height: box.height,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onLaneMove(event) {
    if (!rampDrag) return;
    const endMs = Math.max(0, (event.clientX - rampDrag.left) / PX_PER_MS);
    const endVel = velocityAtY(event.clientY - rampDrag.top, rampDrag.height);
    const curve = event.altKey ? 1 : (event.ctrlKey ? -1 : rampDrag.curve);
    rampDrag = { ...rampDrag, endMs, endVel, curve };
  }

  function onLaneWheel(event) {
    if (!rampDrag) return;
    event.preventDefault();
    const nextCurve = Math.max(-2, Math.min(2, rampDrag.curve + (event.deltaY > 0 ? 0.25 : -0.25)));
    rampDrag = { ...rampDrag, curve: nextCurve };
  }

  function onLaneUp() {
    if (!rampDrag) return;
    const { startMs, endMs, startVel, endVel, curve } = rampDrag;
    rampDrag = null;
    const t0 = Math.min(startMs, endMs);
    const t1 = Math.max(startMs, endMs);
    const touchesAny = notes.some(n => n.startMs >= t0 && n.startMs <= t1);
    if (touchesAny || Math.abs(endMs - startMs) >= 1) {
      onvelocityramp?.(startMs, endMs, startVel, endVel, curve);
    }
  }
</script>

<div class="ks-vel-wrap">
  <span class="ks-vel-label">Velocity</span>
  <div class="ks-vel" bind:this={velScroll}>
    <div class="ks-vel-lane" role="application" tabindex="-1" aria-label="Velocity lane"
      style:--ks-span={spanPx + "px"} style:--ks-vel-h={VEL_PX + "px"}
      onpointerdown={onLaneDown} onpointermove={onLaneMove} onpointerup={onLaneUp} onwheel={onLaneWheel}>
      {#if rampDrag && rampPoints}
        <svg class="ks-vel-ramp-svg" aria-hidden="true" viewBox={`0 0 ${Math.max(1, spanPx)} ${VEL_PX}`} preserveAspectRatio="none">
          <polyline points={rampPoints} />
        </svg>
      {/if}
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
    touch-action: none;
  }
  .ks-vel-ramp-svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
    z-index: 2;
  }
  .ks-vel-ramp-svg polyline {
    fill: none;
    stroke: #efc16a;
    stroke-width: 1.5;
    stroke-dasharray: 4 3;
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
