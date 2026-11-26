<script>
  import {
    PX_PER_MS, ROW_PX, isBlackKey, msAtX, noteDragPayload, pitchAtY, pitchName, pitchRow,
  } from "../../lib/midi.js";
  import { sustainSpanAt } from "../../lib/pianoRoll.js";

  let {
    rollScroll = $bindable(null),
    pitches = [],
    spanPx = 0,
    beats = [],
    sustainSpans = [],
    notes = [],
    ghostNotes = [],
    objectTimes = [],
    selected = $bindable(null),
    loading = false,
    trackId = null,
    onsyncscroll,
    preview,
    onaddnote,
    ondragnote,
    deleteNote,
  } = $props();

  // The note being dragged: { note, mode, x0, y0, dx, dy } (a local ghost until pointerup).
  let drag = $state(null);

  // A press on an empty cell places a note there and plays it.
  async function onGridDown(event) {
    if (event.button !== 0 || event.target !== event.currentTarget || loading || trackId == null) return;
    const box = event.currentTarget.getBoundingClientRect();
    const startMs = msAtX(event.clientX - box.left);
    const pitch = pitchAtY(event.clientY - box.top);
    await onaddnote(startMs, pitch);
  }

  function onNoteDown(event, note, mode) {
    event.stopPropagation();
    if (event.button !== 0 || loading) return;
    selected = note.index;
    drag = { note, mode, x0: event.clientX, y0: event.clientY, dx: 0, dy: 0 };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }

  function onNoteMove(event) {
    if (!drag) return;
    drag.dx = event.clientX - drag.x0;
    drag.dy = event.clientY - drag.y0;
  }

  async function onNoteUp() {
    if (!drag) return;
    const { note, mode, dx, dy } = drag;
    drag = null;
    if (Math.abs(dx) < 2 && Math.abs(dy) < 2) return;
    const payload = noteDragPayload(trackId, note, mode, dx, dy);
    if (!payload) return;
    await ondragnote(note, mode, payload);
  }

  // The ghost's box while dragging, else the note's own.
  function noteBox(note) {
    let start = note.startMs, end = note.endMs, pitch = note.pitch;
    if (drag && drag.note.index === note.index) {
      const payload = noteDragPayload(trackId, note, drag.mode, drag.dx, drag.dy);
      if (payload?.kind === "move_note") { end = payload.startMs + (end - start); start = payload.startMs; pitch = payload.pitch; }
      if (payload?.kind === "resize_note") end = payload.endMs;
    }
    return { left: start * PX_PER_MS + "px", top: pitchRow(pitch) + "px", width: Math.max(4, (end - start) * PX_PER_MS) + "px" };
  }
</script>

<div class="ks-roll" bind:this={rollScroll} onscroll={onsyncscroll} style:--ks-row={ROW_PX + "px"}
  style:--ks-roll-h={pitches.length * ROW_PX + "px"} style:--ks-span={spanPx + "px"}>
  <div class="ks-keys">
    {#each pitches as pitch (pitch)}
      <button type="button" class="ks-key" class:black={isBlackKey(pitch)} class:c={pitch % 12 === 0}
        aria-label={`Play ${pitchName(pitch)}`}
        onpointerdown={() => preview(pitch)}>{pitch % 12 === 0 ? pitchName(pitch) : ""}</button>
    {/each}
  </div>
  <div class="ks-grid" role="grid" tabindex="-1" aria-label="Piano roll"
    onpointerdown={onGridDown} oncontextmenu={(e) => e.preventDefault()}>
    {#each pitches as pitch (pitch)}
      <div class="ks-lane" class:black={isBlackKey(pitch)} class:c={pitch % 12 === 0}
        style:--ks-top={pitchRow(pitch) + "px"}></div>
    {/each}
    {#each beats as beat (beat)}
      <div class="ks-beat" class:bar={beat % 2000 === 0} class:sustained={Boolean(sustainSpanAt(sustainSpans, beat))}
        style:--ks-left={beat * PX_PER_MS + "px"}></div>
    {/each}
    {#each objectTimes as timeMs, idx (`obj-${idx}-${timeMs}`)}
      <div class="ks-object-line" aria-hidden="true" style:--ks-left={timeMs * PX_PER_MS + "px"}></div>
    {/each}
    {#each ghostNotes as ghost, idx (`ghost-${ghost.trackId}-${idx}`)}
      <div class="ks-ghost-note" aria-hidden="true"
        style:--ks-left={ghost.startMs * PX_PER_MS + "px"}
        style:--ks-top={pitchRow(ghost.pitch) + "px"}
        style:--ks-width={Math.max(4, (ghost.endMs - ghost.startMs) * PX_PER_MS) + "px"}></div>
    {/each}
    {#each notes as note (note.index)}
      <div class="ks-note" class:selected={selected === note.index} class:muted={note.muted}
        role="gridcell" tabindex="-1" aria-label={`${pitchName(note.pitch)} at ${Math.round(note.startMs)} ms`}
        style:--ks-left={noteBox(note).left} style:--ks-top={noteBox(note).top}
        style:--ks-width={noteBox(note).width} style:--ks-alpha={0.45 + 0.55 * note.velocity / 127}
        onpointerdown={(e) => onNoteDown(e, note, "move")} onpointermove={onNoteMove} onpointerup={onNoteUp}
        oncontextmenu={(e) => { e.preventDefault(); e.stopPropagation(); deleteNote(note.index); }}>
        <span class="ks-note-name">{pitchName(note.pitch)}</span>
        <span class="ks-edge" role="presentation"
          onpointerdown={(e) => onNoteDown(e, note, "resize")} onpointermove={onNoteMove} onpointerup={onNoteUp}></span>
      </div>
    {/each}
  </div>
</div>

<style>
  .ks-roll {
    flex: 1;
    display: flex;
    min-height: 0;
    overflow: auto;
    background: var(--editor-panel);
  }
  .ks-keys {
    position: sticky;
    left: 0;
    z-index: 3;
    flex: 0 0 56px;
    display: flex;
    flex-direction: column;
    height: var(--ks-roll-h);
    border-right: 1px solid var(--editor-border);
    background: #1c1c1e;
  }
  .ks-key {
    flex: 0 0 var(--ks-row);
    height: var(--ks-row);
    padding: 0 6px 0 0;
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
  .ks-key:active {
    background: #0a84ff;
  }
  .ks-grid {
    position: relative;
    flex: 0 0 auto;
    width: var(--ks-span);
    height: var(--ks-roll-h);
    cursor: crosshair;
  }
  .ks-lane {
    position: absolute;
    top: var(--ks-top);
    left: 0;
    right: 0;
    height: var(--ks-row);
    box-sizing: border-box;
    pointer-events: none;
    background: #1c1c1e;
    border-bottom: 1px solid rgba(255, 255, 255, 0.03);
  }
  .ks-lane.black {
    background: #151517;
  }
  .ks-lane.c {
    border-bottom-color: rgba(255, 255, 255, 0.1);
  }
  .ks-beat {
    position: absolute;
    top: 0;
    bottom: 0;
    left: var(--ks-left);
    width: 1px;
    pointer-events: none;
    background: rgba(255, 255, 255, 0.05);
  }
  .ks-beat.bar {
    background: rgba(255, 255, 255, 0.14);
  }
  .ks-object-line {
    position: absolute;
    top: 0;
    bottom: 0;
    left: var(--ks-left);
    width: 1px;
    pointer-events: none;
    background: rgba(239, 193, 106, 0.28);
  }
  .ks-ghost-note {
    position: absolute;
    z-index: 0;
    left: var(--ks-left);
    top: var(--ks-top);
    width: var(--ks-width);
    height: calc(var(--ks-row) - 1px);
    box-sizing: border-box;
    border: 1px solid rgba(148, 163, 184, 0.22);
    border-radius: 3px;
    background: rgba(148, 163, 184, 0.22);
    pointer-events: none;
  }
  .ks-note {
    position: absolute;
    z-index: 1;
    left: var(--ks-left);
    top: var(--ks-top);
    width: var(--ks-width);
    height: calc(var(--ks-row) - 1px);
    opacity: var(--ks-alpha);
    box-sizing: border-box;
    border: 1px solid rgba(0, 0, 0, 0.45);
    border-radius: 3px;
    background: #0a84ff;
    cursor: grab;
    overflow: hidden;
    touch-action: none;
  }
  .ks-note.selected {
    z-index: 2;
    background: #efc16a;
    border-color: #fff3d6;
  }
  .ks-note.muted {
    filter: grayscale(1);
  }
  .ks-note-name {
    position: absolute;
    left: 3px;
    top: 0;
    font-size: 9px;
    line-height: 12px;
    color: #08080a;
    pointer-events: none;
  }
  .ks-edge {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 6px;
    cursor: ew-resize;
  }
</style>
