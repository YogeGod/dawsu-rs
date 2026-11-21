<script>
  import { tick } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { emptyInspector, problemMarkHandlers, problemStrip } from "../lib/inspector.js";
  import "./problem-map.css";

  const on = problemMarkHandlers((...args) => globalThis.inspectorAction(...args));
  // app.js keeps the data until it changes, so the strip doesn't rebuild on every update.
  const data = $derived((bridge.inspector ?? emptyInspector).problemStrip);
  let width = $state(0);
  const strip = $derived(problemStrip(data, width || 200));
  const disabled = $derived(bridge.disabled);
  // One Tab stop: the mark last moved to, or the first.
  let current = $state(null);
  const focusIndex = $derived(Math.max(0, strip.marks.findIndex(mark => mark.key === current)));
  let buttons = $state([]);

  async function move(event, index) {
    const next = on.key(event, index, strip.marks.length);
    if (next === null) return;
    current = strip.marks[next].key;
    await tick();
    buttons[next]?.focus();
  }
</script>

<div class="pm">
  <div class="pm-head">
    <span class="pm-name" title={strip.name}>{strip.name}</span>
    <!-- The editor has no check behind these counts yet, so they show greyed out and empty. -->
    <span class="pm-sum pm-off" title="Not counted yet." aria-label="Sound right: not counted yet">– of – sound right</span>
  </div>
  <div class="pm-strip">
    <div class="pm-track" role="group" aria-label="Problems across the song" bind:clientWidth={width}>
      {#each strip.ticks as left}
        <span class="pm-tick" style:--at={left}></span>
      {/each}
      {#if strip.frame}
        <span class="pm-frame" style:--at={strip.frame.left} style:--span={strip.frame.width}></span>
      {/if}
      {#each strip.marks as mark, index (mark.key)}
        <button type="button" class="pm-mark pm-{mark.kind}" style:--at={mark.left} title={mark.label} aria-label={mark.label}
          tabindex={index === focusIndex ? 0 : -1} {disabled} bind:this={buttons[index]}
          onfocus={() => (current = mark.key)} onkeydown={event => move(event, index)} onclick={() => on.click(mark)}></button>
      {/each}
    </div>
  </div>
  <div class="pm-legend">
    <span class="pm-count pm-off" title="Not counted yet." aria-label="Silent: not counted yet"><i></i>– silent</span>
    <span class="pm-count pm-off" title="Not counted yet." aria-label="Fallback: not counted yet"><i></i>– fallback</span>
    <span class="pm-count pm-orphaned" title="Moments with hits and no object"><i></i>{strip.orphaned === 1
      ? "1 hit with no object" : strip.orphaned + " hits with no object"}</span>
    <!-- Save problems are known only after a Save check of this version, so no zero is shown. -->
    {#if strip.unsaved}
      <span class="pm-count pm-unsaved" title="Moments Save can't write"><i></i>{strip.unsaved} can't be saved</span>
    {/if}
  </div>
</div>
