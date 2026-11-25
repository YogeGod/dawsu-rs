<script>
  import { CHORD_STRATEGIES, formatChordPitches } from "../lib/chordResolver.js";

  let {
    open = false,
    conflict = null,
    totalConflicts = 0,
    policy = $bindable("flatten_polyphonic"),
    onaudition = null,
    onapplyone = null,
    onapplyall = null,
    onclose = null,
  } = $props();
</script>

{#if open && conflict}
  <div class="cr-pop" role="dialog" aria-label="Chord resolver">
    <header class="cr-head">
      <strong>Chord: {formatChordPitches(conflict.pitches)}</strong>
      <button type="button" aria-label="Close" onclick={() => onclose?.()}>✕</button>
    </header>
    <p class="cr-note">
      osu!standard plays one sample per hit circle (storyboard samples are not used so notes never go silent when storyboards are off).
    </p>
    <div class="cr-options">
      {#each CHORD_STRATEGIES as strat (strat.id)}
        <label class="cr-card" class:selected={policy === strat.id}>
          <input type="radio" name="chord-policy" value={strat.id} bind:group={policy}>
          <span><strong>{strat.label}</strong><small>{strat.description}</small></span>
        </label>
      {/each}
    </div>
    <footer class="cr-foot">
      <button type="button" onclick={() => onaudition?.(conflict)}>▶ Audition</button>
      <button type="button" onclick={() => onapplyone?.(conflict, policy)}>Apply to This Chord</button>
      <button type="button" class="primary" onclick={() => onapplyall?.(policy)}>
        Apply to All {totalConflicts} {totalConflicts === 1 ? "Chord" : "Chords"}
      </button>
    </footer>
  </div>
{/if}

<style>
  .cr-pop {
    width: 340px;
    padding: 14px;
    border: 1px solid var(--editor-border);
    border-radius: 10px;
    background: var(--editor-panel);
    color: var(--editor-text);
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: 12px;
  }
  .cr-head, .cr-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .cr-note {
    margin: 0;
    color: var(--editor-secondary);
    font-size: 11px;
  }
  .cr-options {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cr-card {
    display: flex;
    gap: 8px;
    padding: 8px;
    border: 1px solid var(--editor-border);
    border-radius: 8px;
    background: var(--editor-chrome);
    cursor: pointer;
  }
  .cr-card.selected {
    border-color: var(--editor-accent);
  }
  .cr-card span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .cr-card small {
    color: var(--editor-muted);
    font-size: 10px;
  }
</style>
