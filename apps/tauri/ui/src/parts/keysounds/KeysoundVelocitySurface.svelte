<script>
  import { bridge } from "../../lib/bridge.svelte.js";
  import { velocityAtY } from "../../lib/midi.js";
  import { visibleNotes } from "../../lib/rollPaint.js";

  // Keysounds mode's velocity lane, in the bottom panel where the hit volume lane is. The bars are
  // painted by the editor's paint() onto the volume canvas (#controller); this part sits over that
  // canvas, so in Keysounds mode no pointer event reaches the hit volume handlers.
  // A press on a bar drags that note's velocity; Shift+drag draws a ramp (Alt or Ctrl bends it).
  let velocityDrag = null; // { index, velocity }
  let ramp = null; // { startMs, endMs, startVel, endVel, curve }
  const BAR_REACH_PX = 6;

  const uiScale = () => (Number(document.documentElement.dataset.uiScale) || 100) / 100;
  const view = () => ({ start: bridge.scene?.viewStart ?? 0, zoom: bridge.scene?.zoom ?? 0.16 });

  function local(event) {
    const rect = event.currentTarget.getBoundingClientRect();
    return {
      x: (event.clientX - rect.left) / uiScale(),
      y: (event.clientY - rect.top) / uiScale(),
      height: rect.height / uiScale(),
    };
  }

  // The drag in progress, shown by the next paint.
  function show() {
    const model = bridge.keysoundRoll;
    if (model) {
      model.velocityDrag = velocityDrag;
      model.ramp = ramp;
    }
    bridge.callAction("paintSoon");
  }

  // The note whose bar is nearest the pointer, within reach.
  function barAt(x) {
    const model = bridge.keysoundRoll;
    if (!model) return null;
    const { start, zoom } = view();
    const reach = BAR_REACH_PX / zoom;
    const ms = start + x / zoom;
    let best = null;
    // Every bar is at a note's start, so no note that starts before the reach can match.
    for (const note of visibleNotes(model.notes, ms - reach, ms + reach, { ...model.index, longest: 0 })) {
      const distance = Math.abs(note.startMs - ms);
      if (distance <= reach && (!best || distance < Math.abs(best.startMs - ms))) best = note;
    }
    return best;
  }

  const bend = (event, fallback = 0) => (event.altKey ? 1 : event.ctrlKey ? -1 : fallback);

  function onDown(event) {
    if (event.button !== 0 || !bridge.keysoundRoll) return;
    const { x, y, height } = local(event);
    if (event.shiftKey) {
      const ms = Math.max(0, view().start + x / view().zoom);
      const velocity = velocityAtY(y, height);
      ramp = { startMs: ms, endMs: ms, startVel: velocity, endVel: velocity, curve: bend(event) };
    } else {
      const note = barAt(x);
      if (!note) return;
      bridge.callAction("keysoundsRoll", "select", note.index);
      velocityDrag = { index: note.index, velocity: velocityAtY(y, height) };
    }
    event.currentTarget.setPointerCapture?.(event.pointerId);
    show();
  }

  function onMove(event) {
    if (!velocityDrag && !ramp) return;
    const { x, y, height } = local(event);
    if (velocityDrag) velocityDrag = { ...velocityDrag, velocity: velocityAtY(y, height) };
    if (ramp) {
      ramp = {
        ...ramp,
        endMs: Math.max(0, view().start + x / view().zoom),
        endVel: velocityAtY(y, height),
        curve: bend(event, ramp.curve),
      };
    }
    show();
  }

  function onUp() {
    const done = velocityDrag;
    const drawn = ramp;
    velocityDrag = null;
    ramp = null;
    show();
    if (done) bridge.callAction("keysoundsRoll", "velocity", done.index, done.velocity);
    if (drawn && drawn.endMs !== drawn.startMs) bridge.callAction("keysoundsRoll", "ramp", drawn);
  }

  // The wheel bends a ramp while it is drawn; otherwise it goes on to the editor.
  function onWheel(event) {
    if (!ramp) return;
    event.preventDefault();
    event.stopPropagation();
    ramp = { ...ramp, curve: Math.max(-2, Math.min(2, ramp.curve + (event.deltaY > 0 ? 0.25 : -0.25))) };
    show();
  }
</script>

{#if bridge.mode === "keysounds"}
  <div class="ks-velocity" role="application" tabindex="-1"
    aria-label="Note velocity: drag a bar to change it, Shift+drag to draw a ramp"
    onpointerdown={onDown} onpointermove={onMove} onpointerup={onUp} onpointercancel={onUp} onwheel={onWheel}
    oncontextmenu={(event) => event.preventDefault()}></div>
{/if}

<style>
  .ks-velocity {
    width: 100%;
    height: 100%;
    cursor: ns-resize;
    touch-action: none;
    outline: none;
  }
</style>
