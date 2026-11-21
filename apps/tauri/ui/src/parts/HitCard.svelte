<script>
  import { untrack } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { tooltipDelay } from "../lib/hitCard.js";

  // app.js hands over the hovered hit (hitCardAt) only when it changes; null hides the card.
  const card = $derived(bridge.hitCard);
  let shown = $state(null);
  let lastHidden = null;

  $effect(() => {
    const next = card;
    const showing = untrack(() => shown);
    if (!next) {
      if (showing) lastHidden = performance.now();
      shown = null;
      return;
    }
    const wait = showing ? 0 : tooltipDelay(performance.now(), lastHidden);
    if (!wait) {
      shown = next;
      return;
    }
    const timer = setTimeout(() => { shown = next; }, wait);
    return () => clearTimeout(timer);
  });

  // Server rendering (the tests) has no effects: show the handed-over card as it is.
  const visible = $derived(typeof window === "undefined" ? card : shown);
</script>

{#if visible}
  <div class="hit-card" role="tooltip" style:--hit-x={visible.x + "px"} style:--hit-y={visible.y + "px"}>
    <div class="head"><span class="title">{visible.title}</span>
      <span class="time">{visible.time}</span></div>
    <div class="detail">{visible.detail}</div>
  </div>
{/if}

<style>
  /* states.md: the hit tooltip sits above the hit. */
  .hit-card {
    position: fixed;
    left: var(--hit-x);
    top: var(--hit-y);
    z-index: 50;
    max-width: 320px;
    padding: 8px 12px;
    border: 1px solid var(--editor-border);
    border-radius: 6px;
    background: var(--editor-chrome);
    box-shadow: var(--shadow-tip);
    color: var(--editor-text);
    font: 12px var(--font-ui);
    white-space: nowrap;
    pointer-events: none;
    transform: translate(-50%, calc(-100% - 14px));
  }
  .head {
    display: flex;
    gap: 10px;
    justify-content: space-between;
  }
  .title {
    font-weight: 500;
  }
  .time {
    color: var(--editor-secondary);
    font-variant-numeric: tabular-nums;
  }
  .detail {
    margin-top: 4px;
    color: var(--editor-secondary);
  }
</style>
