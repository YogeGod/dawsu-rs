<script>
  import { bridge } from "../lib/bridge.svelte.js";
  import { invoke } from "../lib/ipc.js";
  import InspectorHead from "./InspectorHead.svelte";
  import PreviewCanvas from "./PreviewCanvas.svelte";

  // The map preview's Inspector panel, shown from View › Map preview (on or off is kept in this
  // window only). It can open the preview in its own window; while that is open the panel draws
  // nothing and offers to bring it back.
  let open = $state(true);
  const on = $derived(bridge.mapPreview);

  const popOut = () => invoke("preview_window_open").catch(reason => console.error(reason));
  const bringBack = () => invoke("close_preview_window").catch(reason => console.error(reason));
</script>

{#if on}
  <div class="insp">
    <InspectorHead title="Map preview" body="inspector-preview-body" bind:open />
    <div id="inspector-preview-body" class="preview" hidden={!open}>
      {#if bridge.previewWindow}
        <p class="insp-summary">The map preview is in its own window.</p>
        <div class="insp-actions"><button type="button" onclick={bringBack}>Show it here</button></div>
      {:else}
        <PreviewCanvas view={bridge.view} path={bridge.scene?.path ?? null} previewKey={bridge.scene?.previewKey ?? null}
          active={open} />
        <div class="insp-actions"><button type="button" onclick={popOut}>Open in its own window</button></div>
      {/if}
    </div>
  </div>
{/if}
