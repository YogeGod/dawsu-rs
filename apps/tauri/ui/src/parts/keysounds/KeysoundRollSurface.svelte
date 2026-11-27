<script>
  import { onDestroy } from "svelte";
  import { bridge } from "../../lib/bridge.svelte.js";
  import { QUANTIZE_MS, isBlackKey, noteDragPayload, pitchName } from "../../lib/midi.js";
  import {
    ROLL_ROW_MAX, ROLL_ROW_MIN, hitTest, maxScrollY, noteIndex, pointAt, rollWheelAction, visibleRows,
  } from "../../lib/rollPaint.js";

  // The piano roll's key column and its pointer surface. The roll itself is painted by the editor's
  // own paint() onto the lanes' canvas (#grid), on the editor's time axis, from the model this part
  // hands to the bridge; this part sits over that canvas and takes every pointer event on it.
  let {
    pitches = [],
    scaleMask = 0xfff,
    foldToScale = false,
    sustainSpans = [],
    notes = [],
    ghostNotes = [],
    objectTimes = [],
    selected = $bindable(null),
    loading = false,
    trackId = null,
    preview,
    onaddnote,
    ondragnote,
    deleteNote,
  } = $props();

  let surface = $state(null);
  let rowHeight = $state(14);
  // Rows scrolled off the top, in px; null until the surface is measured and centred once.
  let scrollY = $state(null);
  // Where the surface sits on the lanes' canvas, in the canvas's own px.
  let box = $state({ top: 0, height: 0 });
  // The note being dragged: { note, mode, x0, y0, payload }.
  let drag = $state(null);
  let cursor = $state("crosshair");

  // Once per summary, so no paint or pointer move has to read every note.
  const index = $derived(noteIndex(notes));
  const roll = $derived({ top: box.top, height: box.height, rowHeight, scrollY: scrollY ?? 0, pitches });
  const rows = $derived.by(() => {
    if (!box.height || !pitches.length) return [];
    const [first, last] = visibleRows(roll);
    return Array.from({ length: Math.max(0, last - first + 1) }, (_, i) => first + i);
  });

  const gridCanvas = () => document.getElementById("grid");
  const uiScale = () => (Number(document.documentElement.dataset.uiScale) || 100) / 100;
  // The editor's time axis, read when needed: its view changes in place, with no rune to follow.
  const view = () => ({ start: bridge.scene?.viewStart ?? 0, zoom: bridge.scene?.zoom ?? 0.16 });

  function local(event) {
    const rect = gridCanvas()?.getBoundingClientRect() ?? { left: 0, top: 0 };
    return { x: (event.clientX - rect.left) / uiScale(), y: (event.clientY - rect.top) / uiScale() };
  }

  function measure() {
    const canvas = gridCanvas();
    if (!surface || !canvas) return;
    const own = surface.getBoundingClientRect();
    const rect = canvas.getBoundingClientRect();
    const next = { top: (own.top - rect.top) / uiScale(), height: own.height / uiScale() };
    if (next.top !== box.top || next.height !== box.height) box = next;
    // First sight of the roll: put the octave around middle C in the middle.
    if (scrollY === null && next.height > 0 && pitches.length) {
      const middle = Math.max(0, pitches.findIndex(pitch => pitch <= 66));
      scrollY = clampScroll(middle * rowHeight - next.height / 2, next.height);
    }
  }

  function clampScroll(value, height = box.height) {
    return Math.min(maxScrollY({ pitches, rowHeight, height }), Math.max(0, Math.round(value)));
  }

  $effect(() => {
    if (!surface || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(surface);
    measure();
    return () => observer.disconnect();
  });

  // Folding to a scale or a new row height changes how far the roll can scroll.
  $effect(() => {
    if (scrollY !== null && scrollY > maxScrollY(roll)) scrollY = clampScroll(scrollY);
  });

  // Hand paint() the roll as it is now, and ask for a frame.
  $effect(() => {
    bridge.keysoundRoll = {
      ...roll,
      notes, index, ghosts: ghostNotes, objectTimes, sustainSpans, selected, scaleMask, foldToScale,
      drag: drag?.payload ? { index: drag.note.index, ...drag.payload } : null,
    };
    bridge.callAction("paintSoon");
  });
  onDestroy(() => {
    bridge.keysoundRoll = null;
    bridge.callAction("paintSoon");
  });

  // Plain wheel scrolls pitch and Ctrl+Shift+wheel sets the row height; both stop here. Every other
  // wheel gesture goes on to the editor's own handler on #scroll: time zoom, snap step and pan.
  function onWheel(event) {
    const action = rollWheelAction(event);
    if (!action) return;
    event.preventDefault();
    event.stopPropagation();
    if (action === "rows") {
      rowHeight = Math.min(ROLL_ROW_MAX, Math.max(ROLL_ROW_MIN, rowHeight + (event.deltaY < 0 ? 2 : -2)));
    } else {
      scrollY = clampScroll((scrollY ?? 0) + event.deltaY);
    }
  }

  function dragPayload(note, mode, dx, dy) {
    const payload = noteDragPayload(trackId, note, mode, dx, dy, QUANTIZE_MS, pitches, view().zoom, rowHeight);
    if (!payload) return null;
    return payload.kind === "resize_note"
      ? { startMs: note.startMs, endMs: payload.endMs, pitch: note.pitch, action: payload }
      : { startMs: payload.startMs, endMs: payload.startMs + (note.endMs - note.startMs), pitch: payload.pitch, action: payload };
  }

  async function onDown(event) {
    if (event.button !== 0 || loading || trackId == null) return;
    const { x, y } = local(event);
    const hit = hitTest(notes, x, y, view(), roll, index);
    if (hit) {
      selected = hit.note.index;
      drag = { note: hit.note, mode: hit.edge ? "resize" : "move", x0: x, y0: y, payload: null };
      event.currentTarget.setPointerCapture?.(event.pointerId);
      return;
    }
    // A press on an empty cell places a note there, its start snapped down to the step.
    const { ms, pitch } = pointAt(x, y, view(), roll);
    await onaddnote(Math.max(0, Math.floor(ms / QUANTIZE_MS) * QUANTIZE_MS), pitch);
  }

  function onMove(event) {
    const { x, y } = local(event);
    if (!drag) {
      const hit = hitTest(notes, x, y, view(), roll, index);
      cursor = hit ? (hit.edge ? "ew-resize" : "grab") : "crosshair";
      return;
    }
    const dx = x - drag.x0;
    const dy = y - drag.y0;
    drag.payload = Math.abs(dx) < 2 && Math.abs(dy) < 2 ? null : dragPayload(drag.note, drag.mode, dx, dy);
  }

  async function onUp() {
    if (!drag) return;
    const { note, mode, payload } = drag;
    drag = null;
    if (payload) await ondragnote(note, mode, payload.action);
  }

  function onContextMenu(event) {
    event.preventDefault();
    if (loading) return;
    const { x, y } = local(event);
    const hit = hitTest(notes, x, y, view(), roll, index);
    if (hit) deleteNote(hit.note.index);
  }

  function inScale(pitch) {
    return ((scaleMask >> (((pitch % 12) + 12) % 12)) & 1) === 1;
  }
</script>

<div class="ks-roll">
  <div class="ks-side">
    <div class="ks-keys" style:--ks-row={rowHeight + "px"}>
      {#each rows as row (pitches[row])}
        {@const pitch = pitches[row]}
        <button type="button" class="ks-key" class:black={isBlackKey(pitch)}
          class:out-of-scale={!foldToScale && !inScale(pitch)}
          style:top={row * rowHeight - (scrollY ?? 0) + "px"}
          aria-label={`Play ${pitchName(pitch)}`}
          onpointerdown={() => preview(pitch)}>{pitch % 12 === 0 ? pitchName(pitch) : ""}</button>
      {/each}
    </div>
  </div>
  <div class="ks-surface" bind:this={surface} role="application" tabindex="-1"
    aria-label="Piano roll: click to place a note, drag to move, drag the right edge to resize, right-click to delete"
    style:cursor={cursor}
    onwheel={onWheel} onpointerdown={onDown} onpointermove={onMove} onpointerup={onUp} onpointercancel={onUp}
    oncontextmenu={onContextMenu}></div>
</div>

<style>
  .ks-roll {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  /* As wide as the lane headers it covers, so the surface starts where the lanes' canvas does. */
  .ks-side {
    flex: 0 0 var(--lanes);
    display: flex;
    justify-content: flex-end;
    min-width: 0;
    background: var(--editor-panel);
    border-right: 1px solid var(--editor-border);
    box-sizing: border-box;
  }
  .ks-keys {
    position: relative;
    flex: 0 0 44px;
    overflow: hidden;
    border-left: 1px solid var(--editor-border);
    background: #1c1c1e;
  }
  .ks-key {
    position: absolute;
    left: 0;
    right: 0;
    height: var(--ks-row);
    min-height: 0;
    padding: 0 5px 0 0;
    border: 0;
    border-bottom: 1px solid rgba(0, 0, 0, 0.5);
    border-radius: 0;
    background: #d8d8dc;
    color: #151517;
    font-size: 9px;
    line-height: 1;
    text-align: right;
  }
  .ks-key.black {
    background: #151517;
    color: var(--editor-muted);
  }
  .ks-key.out-of-scale {
    opacity: 0.45;
  }
  .ks-key:active {
    background: #0a84ff;
  }
  /* See-through: the roll is painted on the canvas underneath. */
  .ks-surface {
    flex: 1;
    min-width: 0;
    touch-action: none;
    outline: none;
  }
</style>
