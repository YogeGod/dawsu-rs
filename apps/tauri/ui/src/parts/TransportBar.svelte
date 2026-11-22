<script>
  import { onDestroy } from "svelte";
  import { act, bridge } from "../lib/bridge.svelte.js";
  import {
    clickGate, copyText, readout, segmentsDisabled, selectionValue, transportActions,
  } from "../lib/transport.js";
  import { invoke } from "../lib/ipc.js";
  import { readPort, recordArgs, recordToast } from "../lib/midi.js";
  import SkipBack from "../icons/SkipBack.svg?raw";
  import SkipForward from "../icons/SkipForward.svg?raw";
  import Repeat2 from "../icons/Repeat2.svg?raw";
  import Square from "../icons/Square.svg?raw";
  import Play from "../icons/Play.svg?raw";
  import Circle from "../icons/Circle.svg?raw";
  import ChevronLeft from "../icons/ChevronLeft.svg?raw";
  import ChevronRight from "../icons/ChevronRight.svg?raw";
  import Copy from "../icons/Copy.svg?raw";
  import Check from "../icons/Check.svg?raw";

  // Frontend only, never saved: the bar starts with every button showing.
  let collapsed = $state(false);
  // "copied" or "failed" for 1.2 s after a click on SELECTION.
  let copyResult = $state(null);
  let copyTimer;
  const toggles = clickGate();
  // MIDI Record: armed listens on the port Preferences › MIDI Input & VST3 picked; disarming
  // commits the recorded notes as hits (Rust's midi_record_toggle).
  let armed = $state(false);
  let arming = $state(false);

  const view = $derived(bridge.view);
  const scene = $derived(bridge.scene);
  const playing = $derived(!!view?.playing);
  const cycle = $derived(!!view?.cycle);
  const disabled = $derived(segmentsDisabled(view, scene));
  const shown = $derived(readout(bridge.playhead, scene));
  const time = $derived(shown.time);
  const bpm = $derived(shown.bpm);
  const meter = $derived(shown.meter);
  const selection = $derived(selectionValue(scene));
  const send = name => act(transportActions(bridge.view)[name]);
  const toggle = name => toggles(bridge.latestView) && send(name);

  async function copy() {
    if (!selection) return;
    const result = await copyText(selection, navigator.clipboard);
    copyResult = result;
    clearTimeout(copyTimer);
    copyTimer = setTimeout(() => (copyResult = null), 1200);
  }
  onDestroy(() => clearTimeout(copyTimer));

  const storage = () => { try { return globalThis.localStorage; } catch { return null; } };

  async function toggleRecord() {
    if (arming) return;
    arming = true;
    try {
      const status = await invoke("midi_record_toggle", recordArgs(readPort(storage()), bridge.scene));
      armed = !!status?.armed;
      const toast = recordToast(status);
      if (toast) globalThis.showToast?.(toast);
      if (!armed && status?.committedNotes) await act(null);
    } catch (error) {
      globalThis.showToast?.({ tone: "error", title: "MIDI Record", body: String(error) });
    } finally {
      arming = false;
    }
  }
</script>

<div class="transport">
  <div class="bar" role="group" aria-label="Transport">
    <button type="button" class="seg tab" aria-label="Go to start and Go to end" title="Go to start and Go to end"
      aria-expanded={!collapsed}
      onclick={() => (collapsed = !collapsed)}>{@html collapsed ? ChevronRight : ChevronLeft}</button>
    {#if !collapsed}
      <button type="button" class="seg jump" aria-label="Go to start" title="Go to start" disabled={disabled.jump}
        onclick={() => send("start")}>{@html SkipBack}</button>
      <button type="button" class="seg jump" aria-label="Go to end" title="Go to end" disabled={disabled.end}
        onclick={() => send("end")}>{@html SkipForward}</button>
    {/if}
    <button type="button" class="seg cycle" aria-label="Cycle" title="Cycle between L and R" aria-pressed={cycle}
      disabled={disabled.playback} onclick={() => toggle("cycle")}>{@html Repeat2}</button>
    <button type="button" class="seg" class:on={!playing} aria-label="Stop" title="Stop"
      disabled={disabled.playback} onclick={() => send("stop")}>{@html Square}</button>
    <button type="button" class="seg" class:playing aria-label="Play" title="Play or pause" aria-pressed={playing}
      disabled={disabled.playback} onclick={() => toggle("play")}>{@html Play}</button>
    <button type="button" class="seg record" class:armed aria-label="MIDI Record"
      title={armed ? "Stop MIDI recording and add the notes" : "Arm MIDI recording"} aria-pressed={armed}
      disabled={disabled.playback || arming} onclick={toggleRecord}>{@html Circle}</button>
  </div>
  <div class="readout">
    <span class="time">{time}</span>
    <span class="divider music"></span>
    <span class="cell music" title="Tempo at the playhead"><span class="value">{bpm}</span> <small>BPM</small></span>
    <span class="divider music"></span>
    <span class="cell music" title="Time signature at the playhead"><span class="value">{meter}</span> <small>TIME</small></span>
    <span class="divider"></span>
    <button type="button" class="cell selection" aria-label="Copy selection time" title="Copy selection time"
      disabled={!selection} onclick={copy}>
      <span class="stack">
        <span class="value" class:empty={!selection}>{selection ?? "No selection"}</span>
        <small>SELECTION</small>
      </span>
      {#if copyResult === "failed"}<span class="copy failed" role="status">Could not copy</span>
      {:else if selection}<span class="copy" class:copied={copyResult === "copied"}>{@html copyResult === "copied" ? Check : Copy}</span>{/if}
    </button>
  </div>
</div>

<style>
  .transport {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 44px;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--editor-panel);
    font-family: Inter, system-ui, sans-serif;
    font-variant-numeric: tabular-nums;
    user-select: none;
  }
  .bar {
    display: flex;
    gap: 1px;
    height: 34px;
    padding: 1px;
    box-sizing: border-box;
    border-radius: 6px;
    background: var(--editor-ink);
    box-shadow: var(--shadow-well);
    overflow: hidden;
  }
  .seg {
    display: grid;
    place-items: center;
    flex: none;
    width: 38px;
    height: 32px;
    min-height: 0;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: var(--transport-seg);
    color: var(--editor-text);
    cursor: pointer;
  }
  .seg:first-child {
    border-radius: 5px 0 0 5px;
  }
  .seg:last-child {
    border-radius: 0 5px 5px 0;
  }
  .seg:hover:not(:disabled) {
    background: var(--editor-border);
  }
  .seg[aria-pressed="true"],
  .seg[aria-pressed="true"]:hover:not(:disabled),
  .seg.on,
  .seg.on:hover:not(:disabled) {
    border: 0;
    background: var(--transport-seg-on);
    color: var(--editor-text);
  }
  .seg.cycle[aria-pressed="true"],
  .seg.cycle[aria-pressed="true"]:hover:not(:disabled) {
    color: var(--editor-accent);
  }
  .seg.playing[aria-pressed="true"],
  .seg.playing[aria-pressed="true"]:hover:not(:disabled) {
    color: var(--editor-play);
  }
  .seg.record.armed,
  .seg.record.armed:hover:not(:disabled) {
    color: var(--editor-error);
  }
  .seg:disabled {
    cursor: default;
    opacity: 1;
    color: var(--editor-disabled);
  }
  .seg:focus-visible,
  .selection:focus-visible {
    outline: 2px solid var(--editor-focus);
    outline-offset: -2px;
  }
  .seg :global(svg) {
    width: 15px;
    height: 15px;
  }
  .tab {
    width: 18px;
    background: #1e1e1e;
    color: var(--editor-muted);
  }
  .tab :global(svg) {
    width: 12px;
    height: 12px;
  }
  .readout {
    display: flex;
    align-items: center;
    height: 40px;
    border-radius: 8px;
    background: var(--editor-ink);
    box-shadow: var(--shadow-readout);
    white-space: nowrap;
  }
  .time {
    padding: 0 16px 0 12px;
    font-size: 21px;
    color: var(--readout-value);
  }
  .divider {
    flex: none;
    width: 1px;
    height: 27px;
    background: var(--editor-border);
  }
  .cell {
    display: grid;
    justify-items: start;
    gap: 2px;
    padding: 0 12px;
    font-size: 13px;
    line-height: 15px;
  }
  .value {
    color: var(--readout-value);
  }
  small {
    font-size: 11px;
    color: var(--editor-muted);
  }
  .selection {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    min-height: 0;
    border: 0;
    border-radius: 0 8px 8px 0;
    background: transparent;
    font: inherit;
    font-size: 13px;
    line-height: 15px;
    text-align: left;
    cursor: pointer;
  }
  .selection:hover:not(:disabled) {
    background: var(--editor-panel);
  }
  .selection:disabled {
    cursor: default;
    opacity: 1;
  }
  .stack {
    display: grid;
    gap: 2px;
  }
  .empty {
    color: var(--editor-disabled);
  }
  .copy {
    display: grid;
    color: var(--editor-muted);
  }
  .copy.copied {
    color: var(--editor-play);
  }
  .copy.failed {
    color: var(--editor-error);
    font-size: 12px;
  }
  .copy :global(svg) {
    width: 14px;
    height: 14px;
  }
  @media (forced-colors: active) {
    .seg[aria-pressed="true"],
    .seg.on {
      outline: 2px solid Highlight;
      outline-offset: -2px;
    }
  }
  @media (max-width: 1599px) {
    .transport {
      gap: 8px;
    }
    .time {
      padding: 0 12px 0 10px;
      font-size: 19px;
    }
    .cell {
      padding: 0 9px;
    }
  }
  /* The window can be 1024 px wide. Below 1240 px the readout drops BPM and TIME; below 1120 px
     the bar drops the collapse tab, Go to start (Home and the Playback menu keep it) and Go to end. */
  @media (max-width: 1239px) {
    .music {
      display: none;
    }
  }
  @media (max-width: 1119px) {
    .tab,
    .jump {
      display: none;
    }
    .cycle {
      border-radius: 5px 0 0 5px;
    }
    .time {
      font-size: 17px;
    }
  }
</style>
