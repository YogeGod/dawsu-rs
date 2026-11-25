<script>
  import { isBlackKey } from "../lib/pianoRoll.js";

  const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

  let { lowPitch = 60, highPitch = 84, rowHeight = 18, onpreview = null } = $props();

  const keys = $derived.by(() => {
    const out = [];
    for (let p = highPitch; p >= lowPitch; p--) {
      const name = NAMES[p % 12];
      const oct = Math.floor(p / 12) - 1;
      out.push({ pitch: p, black: isBlackKey(p), label: name === "C" ? `C${oct}` : "" });
    }
    return out;
  });
</script>

<div class="pk-col" role="group" aria-label="Piano keyboard">
  {#each keys as key (key.pitch)}
    <button type="button" class="pk-key" class:black={key.black}
      style:height="{rowHeight}px"
      aria-label={`Audition MIDI ${key.pitch}`}
      onclick={() => onpreview?.(key.pitch)}>
      {#if key.label}<span class="pk-label">{key.label}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .pk-col {
    display: flex;
    flex-direction: column;
    width: 100%;
    background: var(--editor-ink);
  }
  .pk-key {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding: 0 6px;
    border: 0;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
    background: #232327;
    color: var(--editor-secondary);
    font-size: 10px;
    cursor: pointer;
  }
  .pk-key.black {
    background: #121214;
    color: var(--editor-muted);
  }
  .pk-key:hover {
    background: var(--editor-selected);
    color: var(--editor-accent);
  }
</style>
