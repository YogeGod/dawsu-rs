<script>
  import { tick } from "svelte";
  import { act, bridge } from "../lib/bridge.svelte.js";
  import { parseVolumeInput, VOLUME_MAX, VOLUME_MIN } from "../lib/volumeBubble.js";

  const bubble = $derived(bridge.volumeBubble);
  let draft = $state("");
  let inputEl = $state(null);

  $effect(() => {
    const current = bubble;
    const editing = Boolean(current?.editing);
    bridge.setModalOpen("volume-bubble", editing);
    if (editing) {
      draft = String(current.volume);
      tick().then(() => {
        inputEl?.focus();
        inputEl?.select();
      });
    }
    return () => {
      bridge.setModalOpen("volume-bubble", false);
    };
  });

  function closeBubble() {
    bridge.setModalOpen("volume-bubble", false);
    bridge.volumeBubble = null;
  }

  function commit() {
    const current = bubble;
    if (!current) return;
    const parsed = parseVolumeInput(draft);
    closeBubble();
    if (parsed != null && parsed !== current.volume) {
      act({ kind: "moment_volume", ms: current.ms, value: parsed });
    }
  }

  function onKeydown(event) {
    event.stopPropagation();
    if (event.key === "Enter") {
      event.preventDefault();
      commit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      closeBubble();
    }
  }
</script>

{#if bubble}
  <div
    class="vol-bubble"
    class:below={bubble.below}
    class:editing={bubble.editing}
    role={bubble.editing ? "dialog" : "tooltip"}
    aria-label={bubble.editing ? "Set hit volume percentage" : undefined}
    style:--vb-x={bubble.x + "px"}
    style:--vb-y={bubble.y + "px"}
  >
    {#if bubble.editing}
      <form class="vb-form" onsubmit={(e) => { e.preventDefault(); commit(); }}>
        <input
          bind:this={inputEl}
          class="vb-input"
          type="number"
          min={VOLUME_MIN}
          max={VOLUME_MAX}
          step="1"
          aria-label="Hit volume percent"
          bind:value={draft}
          onkeydown={onKeydown}
          onblur={commit}
        >
        <span class="vb-unit">%</span>
      </form>
    {:else}
      <span class="vb-val">{bubble.volume}%</span>
      <span class="vb-time">{bubble.timeLabel}</span>
    {/if}
  </div>
{/if}

<style>
  .vol-bubble {
    position: fixed;
    left: var(--vb-x);
    top: var(--vb-y);
    z-index: 55;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    border: 1px solid var(--editor-accent, #58a6ff);
    border-radius: 5px;
    background: var(--editor-chrome, #1c1f24);
    box-shadow: var(--shadow-tip, 0 6px 18px rgba(0, 0, 0, 0.45));
    color: var(--editor-text, #e6edf3);
    font: 12px/1.2 var(--font-ui, sans-serif);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    pointer-events: none;
    transform: translate(-50%, -100%);
  }
  .vol-bubble.below {
    transform: translate(-50%, 0);
  }
  .vol-bubble.editing {
    pointer-events: auto;
    padding: 3px 6px;
  }
  .vb-val {
    font-weight: 600;
  }
  .vb-time {
    color: var(--editor-secondary, #8b949e);
    font-size: 11px;
  }
  .vb-form {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin: 0;
  }
  .vb-input {
    width: 48px;
    padding: 1px 4px;
    border: 1px solid var(--editor-border, #30363d);
    border-radius: 3px;
    background: var(--editor-panel, #14171c);
    color: var(--editor-text, #e6edf3);
    font: 600 12px/1.2 var(--font-ui, sans-serif);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .vb-input:focus {
    outline: 1px solid var(--editor-accent, #58a6ff);
  }
  .vb-unit {
    color: var(--editor-secondary, #8b949e);
    font-size: 11px;
  }
</style>
