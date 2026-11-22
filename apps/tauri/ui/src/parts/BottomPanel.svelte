<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import ChevronRight from "../icons/ChevronRight.svg?raw";

  // The bottom panel's side column (v3 F1-F3): the Volume and Mixer tabs, or the folded strip.
  // Open or folded is the saved Volume and Mixer preference; the tab is kept in this window only.
  const folded = $derived(bridge.view?.panels?.controller === false);
  const tab = $derived(bridge.bottomTab);
  const disabled = $derived(bridge.disabled);

  function choose(name) {
    if (bridge.bottomTab === name) return;
    bridge.bottomTab = name;
    // app.js lays the panel out from the tab and draws the volume canvas only on the Volume tab.
    globalThis.renderPanelLayout?.();
    globalThis.paintSoon?.();
  }

  // A click or a drag up of 12 px opens the folded panel.
  const open = () => globalThis.togglePanel?.("controller");
  let from = null;
  let dragged = false;
  function down(event) {
    if (disabled || event.button !== 0) return;
    from = event.clientY;
    dragged = false;
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }
  function move(event) {
    if (from === null || from - event.clientY < 12) return;
    from = null;
    dragged = true;
    open();
  }
  function click() {
    if (!dragged) open();
  }
  // The click that ends a drag comes right after pointerup; the flag clears after it.
  function up() {
    from = null;
    setTimeout(() => (dragged = false));
  }
</script>

{#if folded}
  <button type="button" class="folded" aria-label="Volume and Mixer" aria-expanded="false" {disabled}
    onclick={click} onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}>
    <span class="chevron" aria-hidden="true">{@html ChevronRight}</span>
    <span class:on={tab === "volume"}>Volume</span>
    <span class:on={tab === "mixer"}>Mixer</span>
    <span class="hint">Drag up to open</span>
  </button>
{:else}
  <div class="seg" role="group" aria-label="Bottom panel">
    <button type="button" aria-pressed={tab === "volume"} onclick={() => choose("volume")}>Volume</button>
    <button type="button" aria-pressed={tab === "mixer"} onclick={() => choose("mixer")}>Mixer</button>
  </div>
{/if}

<style>
  .seg {
    height: 30px;
    flex: none;
    display: flex;
    gap: 2px;
    padding: 2px;
    box-sizing: border-box;
    border-radius: 6px;
    background: var(--editor-chrome);
  }
  .seg button {
    flex: 1;
    min-width: 0;
    min-height: 0;
    padding: 0 4px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: var(--editor-secondary);
    font: 500 13px var(--font-ui);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
  }
  .seg button[aria-pressed="true"] {
    background: var(--editor-selected);
    color: var(--editor-accent);
  }
  .seg button:disabled {
    color: var(--editor-disabled);
    cursor: default;
  }
  .folded {
    flex: 1;
    min-height: 0;
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px;
    border: 0;
    border-radius: 0;
    background: var(--editor-panel);
    color: var(--editor-disabled);
    font: 13px var(--font-ui);
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    touch-action: none;
    cursor: ns-resize;
  }
  .folded:disabled {
    cursor: default;
  }
  .folded .on {
    color: var(--editor-text);
    font-weight: 500;
  }
  .chevron {
    display: inline-flex;
    width: 14px;
    height: 14px;
    flex: none;
    color: var(--editor-secondary);
  }
  .chevron :global(svg) {
    width: 14px;
    height: 14px;
  }
  .hint {
    margin-left: 8px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
