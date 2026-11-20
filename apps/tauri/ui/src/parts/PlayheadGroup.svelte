<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { playheadActions } from "../lib/playhead.js";
  import LocateFixed from "../icons/LocateFixed.svg?raw";
  import Crosshair from "../icons/Crosshair.svg?raw";
  import UndoDot from "../icons/UndoDot.svg?raw";

  const follow = $derived(!!bridge.view?.follow);
  const returns = $derived(!!bridge.view?.returnOnStop);
  const disabled = $derived(bridge.disabled);
  // Light updates, as the old Follow and Return sent them.
  const send = name => globalThis.act(playheadActions(bridge.view)[name], false);

  function centre() {
    bridge.centre = !bridge.centre;
    globalThis.followPlayhead?.();
  }
</script>

<div class="ui-well playhead">
  <span class="label"><span class="glyph" aria-hidden="true"></span>Playhead</span>
  <button type="button" class="toggle" aria-label="Follow" aria-pressed={follow} {disabled}
    title="Scroll the timeline to keep the playhead visible" onclick={() => send("follow")}>{@html LocateFixed}</button>
  <button type="button" class="toggle" aria-label="Centre" aria-pressed={bridge.centre} disabled={disabled || !follow}
    title="Keep the playhead in the middle while following" onclick={centre}>{@html Crosshair}</button>
  <button type="button" class="toggle" aria-label="Return" aria-pressed={returns} {disabled}
    title="Return to the playback start when stopping" onclick={() => send("return")}>{@html UndoDot}</button>
</div>

<style>
  .playhead {
    padding: 0 3px 0 10px;
  }
  .label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-right: 4px;
    color: var(--editor-secondary);
    font-size: 13px;
    font-weight: 500;
    white-space: nowrap;
  }
  /* v3: a 1 px line with a 9 x 5 accent triangle on top. */
  .glyph {
    position: relative;
    width: 9px;
    height: 16px;
    flex: none;
  }
  .glyph::before {
    content: "";
    position: absolute;
    left: 4px;
    top: 0;
    bottom: 0;
    width: 1px;
    background: var(--editor-focus);
  }
  .glyph::after {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    border-style: solid;
    border-width: 5px 4.5px 0;
    border-color: var(--editor-accent) transparent transparent;
  }
  .toggle {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    min-height: 26px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--editor-secondary);
    cursor: pointer;
  }
  .toggle:hover:not(:disabled) {
    background: var(--editor-border);
  }
  .toggle[aria-pressed="true"],
  .toggle[aria-pressed="true"]:hover:not(:disabled) {
    border: 0;
    background: var(--editor-accent);
    color: var(--editor-ink);
  }
  .toggle:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .toggle:focus-visible {
    outline: 2px solid var(--editor-focus);
    outline-offset: 0;
  }
  .toggle :global(svg) {
    width: 15px;
    height: 15px;
  }
  /* style.css hides the toolbar's text labels from here, as it did Follow's and Return's. */
  @media (max-width: 1779px) {
    .label {
      font-size: 0;
      gap: 0;
    }
  }
  /* From 1399 px down the toolbar is tight: the group keeps only its three toggles. */
  @media (max-width: 1399px) {
    .playhead {
      padding: 0 2px;
    }
    .label {
      display: none;
    }
    .toggle {
      width: 24px;
    }
  }
  @media (forced-colors: active) {
    .toggle[aria-pressed="true"] {
      outline: 2px solid Highlight;
      outline-offset: -2px;
    }
  }
</style>
