<script>
  import { onMount } from "svelte";
  import { bridge } from "../lib/bridge.svelte.js";
  import { exportModalOnTop } from "../lib/space.js";
  import ExportWindow from "./ExportWindow.svelte";

  // In-editor Export modal: embeds ExportWindow.svelte inside the main editor window instead of
  // opening a separate OS window.
  let open = $state(false);
  let exportWindow = $state(null);

  function show() {
    open = true;
  }

  // The button, Escape and the backdrop all close through ExportWindow, which sends a waiting
  // choice and tells the editor first, then calls closed().
  function requestClose() {
    exportWindow?.close();
  }

  // Closed on its own hitsound-only write: the editor asks for the view that tells it.
  function closed({ wroteHitsoundOnly = false } = {}) {
    open = false;
    bridge.callAction("exportClosed", wroteHitsoundOnly);
  }

  function onKeydown(event) {
    if (open && event.key === "Escape" && exportModalOnTop(bridge.onlyModalOpen)) {
      event.preventDefault();
      requestClose();
    }
  }

  $effect(() => {
    bridge.setModalOpen("export", open);
    return () => { bridge.setModalOpen("export", false); };
  });

  onMount(() => {
    return bridge.registerModal("openExportModal", show);
  });
</script>

<svelte:window onkeydown={onKeydown} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="export-backdrop" onclick={(e) => { if (e.target === e.currentTarget) requestClose(); }}>
    <div class="export-dialog" role="dialog" aria-modal="true" aria-label="Export">
      <ExportWindow bind:this={exportWindow} onclose={closed} />
    </div>
  </div>
{/if}

<style>
  .export-backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    background: rgba(0, 0, 0, 0.58);
    display: grid;
    place-items: center;
    padding: 24px;
    box-sizing: border-box;
  }
  .export-dialog {
    width: min(1140px, 94vw);
    height: min(760px, 90vh);
    border-radius: 12px;
    border: 1px solid var(--editor-border);
    background: var(--editor-canvas);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
</style>
