<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { inspectorHandlers, problemStrip } from "../lib/inspector.js";
  import "./problem-map.css";

  const on = inspectorHandlers((...args) => globalThis.inspectorAction(...args));
  const strip = $derived(problemStrip(bridge.view, bridge.scene));
  // The editor has no check behind these counts yet, so they show greyed out and empty.
  const notCounted = {
    right: "Not counted yet: the editor doesn't compare each object's saved sound with the timeline.",
    silent: "Not counted yet: osu! still plays the old sound on an object with no hits, so the editor can't tell which objects are silent.",
    fallback: "Not counted yet: the editor doesn't check which samples fall back to the skin.",
  };
</script>

<div class="pm">
  <div class="pm-head">
    <span class="pm-name" title={strip.name}>{strip.name}</span>
    <span class="pm-sum pm-off" title={notCounted.right}>– of – sound right</span>
  </div>
  <div class="pm-strip">
    {#each strip.marks as mark}
      <button type="button" class="pm-mark pm-{mark.kind}" style:--at={mark.left} title={mark.label} aria-label={mark.label}
        disabled={bridge.disabled} onclick={() => on.jump(mark.time)}></button>
    {/each}
  </div>
  <div class="pm-legend">
    <span class="pm-count pm-off" title={notCounted.silent}><i></i>– silent</span>
    <span class="pm-count pm-off" title={notCounted.fallback}><i></i>– fallback</span>
    <span class="pm-count pm-orphaned" title="Hits with no object"><i></i>{strip.orphaned} orphaned</span>
    <!-- Save problems are known only after a Save check of this version, so no zero is shown. -->
    {#if strip.unsaved}
      <span class="pm-count pm-unsaved" title="Moments Save can't write"><i></i>{strip.unsaved} can't be saved</span>
    {/if}
  </div>
</div>
